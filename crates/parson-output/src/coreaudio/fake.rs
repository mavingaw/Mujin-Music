//! A protocol-level fake of the macOS audio HAL for tests and demos on
//! machines that are not Macs. It follows the HAL's documented behaviour:
//!
//! - `kAudioDevicePropertyHogMode` is a toggle: setting it takes a free
//!   device for the calling process and releases one the process owns; a
//!   device another process hogs stays theirs, and its rate, format and IO
//!   are refused with `kAudioDevicePermissionsError`;
//! - a nominal sample rate change lands asynchronously, some milliseconds
//!   after it is asked for (callers must wait for it), and only rates in
//!   `kAudioDevicePropertyAvailableNominalSampleRates` are accepted;
//! - the physical format must be one the stream lists;
//! - the IOProc pulls float buffers at the device's real rate, and the HAL
//!   converts them to the physical integer format (scale by 2^(bits-1),
//!   round, clip), which is what the DAC receives.
//!
//! It keeps the integer samples each device received so tests can prove
//! bit-perfection.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{
    BAD_DEVICE, HalApi, HalDevice, ILLEGAL_OPERATION, OsStatus, PERMISSIONS, PhysicalFormat,
    RenderCallback, UNKNOWN_PROPERTY, UNSUPPORTED_FORMAT,
};

/// The pid the fake reports for "this process".
pub const THIS_PROCESS: i32 = 4_242;
/// Another process (Music, a browser) for tests.
pub const OTHER_PROCESS: i32 = 777;

#[derive(Clone, Debug)]
pub struct FakeHalDevice {
    pub device: HalDevice,
    pub rates: Vec<(f64, f64)>,
    pub physical: Vec<PhysicalFormat>,
    pub volume_db: Option<(f32, f32)>,
    /// How long a nominal rate change takes to land.
    pub rate_switch_delay: Duration,
    /// Frames per IOProc cycle.
    pub io_frames: u32,
    pub latency_frames: u32,
}

impl FakeHalDevice {
    /// A USB DAC: discrete rates 44.1 to 384 kHz, 16/24/32-bit integer
    /// physical formats, a hardware volume of -96 to 0 dB.
    pub fn usb_dac(object_id: u32, uid: &str, name: &str) -> Self {
        let rates = [
            44_100.0, 48_000.0, 88_200.0, 96_000.0, 176_400.0, 192_000.0, 352_800.0, 384_000.0,
        ];
        let mut physical = Vec::new();
        for rate in rates {
            for bits in [16, 24, 32] {
                physical.push(PhysicalFormat {
                    rate_min: rate,
                    rate_max: rate,
                    bits,
                    float: false,
                    channels: 2,
                });
            }
        }
        Self {
            device: HalDevice {
                object_id,
                uid: uid.to_string(),
                name: name.to_string(),
                manufacturer: "Chord Electronics Ltd".to_string(),
                transport: u32::from_be_bytes(*b"usb "),
                output_channels: 2,
            },
            rates: rates.iter().map(|rate| (*rate, *rate)).collect(),
            physical,
            volume_db: Some((-96.0, 0.0)),
            rate_switch_delay: Duration::from_millis(40),
            io_frames: 512,
            latency_frames: 64,
        }
    }

    /// Built-in speakers: 44.1/48/96 kHz as one range, float physical only,
    /// software-free hardware volume.
    pub fn built_in(object_id: u32) -> Self {
        Self {
            device: HalDevice {
                object_id,
                uid: "BuiltInSpeakerDevice".to_string(),
                name: "MacBook Pro Speakers".to_string(),
                manufacturer: "Apple Inc.".to_string(),
                transport: u32::from_be_bytes(*b"bltn"),
                output_channels: 2,
            },
            rates: vec![(44_100.0, 96_000.0)],
            physical: vec![PhysicalFormat {
                rate_min: 44_100.0,
                rate_max: 96_000.0,
                bits: 32,
                float: true,
                channels: 2,
            }],
            volume_db: Some((-63.5, 0.0)),
            rate_switch_delay: Duration::from_millis(10),
            io_frames: 512,
            latency_frames: 400,
        }
    }
}

/// What one device went through.
#[derive(Clone, Debug, Default)]
pub struct FakeHalCapture {
    /// Integer samples as the DAC received them (right-aligned).
    pub samples: Vec<i32>,
    /// Physical bits and nominal rate the IO ran at, per start.
    pub runs: Vec<(u32, f64)>,
    /// Every nominal rate asked for.
    pub rate_requests: Vec<f64>,
    pub volume_db: Option<f32>,
    pub muted: bool,
}

struct DeviceState {
    config: FakeHalDevice,
    nominal: f64,
    pending: Option<(f64, Instant)>,
    physical: PhysicalFormat,
    hog: i32,
    capture: FakeHalCapture,
}

impl DeviceState {
    fn settle(&mut self) {
        if let Some((rate, at)) = self.pending
            && Instant::now() >= at
        {
            self.nominal = rate;
            self.physical.rate_min = rate;
            self.physical.rate_max = rate;
            self.pending = None;
        }
    }

    fn other_owner(&self) -> bool {
        self.hog != -1 && self.hog != THIS_PROCESS
    }
}

struct Io {
    device: u32,
    running: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

pub struct FakeHal {
    devices: Arc<Mutex<HashMap<u32, DeviceState>>>,
    order: Mutex<Vec<u32>>,
    ios: Mutex<HashMap<u64, Io>>,
    speed: f64,
}

static NEXT_IO: AtomicU64 = AtomicU64::new(1);

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The HAL's float-to-integer step.
fn to_integer(sample: f32, bits: u32) -> i32 {
    let scale = f64::from(1_u32 << (bits.clamp(2, 32) - 1).min(31));
    let maximum = scale - 1.0;
    (f64::from(sample) * scale).round().clamp(-scale, maximum) as i32
}

impl FakeHal {
    pub fn new(devices: Vec<FakeHalDevice>, speed: f64) -> Self {
        let order = devices
            .iter()
            .map(|device| device.device.object_id)
            .collect();
        let devices = devices
            .into_iter()
            .map(|config| {
                let physical = config.physical[0];
                let nominal = config.rates[0].0;
                (
                    config.device.object_id,
                    DeviceState {
                        nominal,
                        pending: None,
                        physical: PhysicalFormat {
                            rate_min: nominal,
                            rate_max: nominal,
                            ..physical
                        },
                        hog: -1,
                        capture: FakeHalCapture::default(),
                        config,
                    },
                )
            })
            .collect();
        Self {
            devices: Arc::new(Mutex::new(devices)),
            order: Mutex::new(order),
            ios: Mutex::new(HashMap::new()),
            speed: speed.max(0.01),
        }
    }

    pub fn capture(&self, object_id: u32) -> FakeHalCapture {
        lock(&self.devices)
            .get(&object_id)
            .map(|device| device.capture.clone())
            .unwrap_or_default()
    }

    /// Another process takes (or releases, with -1) the device.
    pub fn set_hog(&self, object_id: u32, pid: i32) {
        if let Some(device) = lock(&self.devices).get_mut(&object_id) {
            device.hog = pid;
        }
    }

    pub fn nominal(&self, object_id: u32) -> f64 {
        let mut devices = lock(&self.devices);
        devices
            .get_mut(&object_id)
            .map(|device| {
                device.settle();
                device.nominal
            })
            .unwrap_or(0.0)
    }

    pub fn physical(&self, object_id: u32) -> Option<PhysicalFormat> {
        lock(&self.devices)
            .get(&object_id)
            .map(|device| device.physical)
    }

    pub fn unplug(&self, object_id: u32) {
        lock(&self.order).retain(|id| *id != object_id);
        lock(&self.devices).remove(&object_id);
    }

    fn with<T>(
        &self,
        object_id: u32,
        action: impl FnOnce(&mut DeviceState) -> Result<T, OsStatus>,
    ) -> Result<T, OsStatus> {
        let mut devices = lock(&self.devices);
        let device = devices.get_mut(&object_id).ok_or(BAD_DEVICE)?;
        device.settle();
        action(device)
    }
}

impl HalApi for FakeHal {
    fn output_devices(&self) -> Result<Vec<HalDevice>, OsStatus> {
        let order = lock(&self.order).clone();
        let devices = lock(&self.devices);
        Ok(order
            .iter()
            .filter_map(|id| devices.get(id))
            .map(|device| device.config.device.clone())
            .collect())
    }

    fn available_rates(&self, device: u32) -> Result<Vec<(f64, f64)>, OsStatus> {
        self.with(device, |device| Ok(device.config.rates.clone()))
    }

    fn nominal_rate(&self, device: u32) -> Result<f64, OsStatus> {
        self.with(device, |device| Ok(device.nominal))
    }

    fn set_nominal_rate(&self, device: u32, rate: f64) -> Result<(), OsStatus> {
        self.with(device, |device| {
            if device.other_owner() {
                return Err(PERMISSIONS);
            }
            if !device
                .config
                .rates
                .iter()
                .any(|(low, high)| (*low..=*high).contains(&rate))
            {
                return Err(ILLEGAL_OPERATION);
            }
            device.capture.rate_requests.push(rate);
            device.pending = Some((rate, Instant::now() + device.config.rate_switch_delay));
            Ok(())
        })
    }

    fn physical_formats(&self, device: u32) -> Result<Vec<PhysicalFormat>, OsStatus> {
        self.with(device, |device| Ok(device.config.physical.clone()))
    }

    fn set_physical_format(&self, device: u32, format: &PhysicalFormat) -> Result<(), OsStatus> {
        self.with(device, |device| {
            if device.other_owner() {
                return Err(PERMISSIONS);
            }
            let listed = device.config.physical.iter().any(|physical| {
                physical.bits == format.bits
                    && physical.float == format.float
                    && physical.channels == format.channels
                    && physical.rate_min <= format.rate_min
                    && format.rate_max <= physical.rate_max
            });
            if !listed {
                return Err(UNSUPPORTED_FORMAT);
            }
            // Setting the physical format also sets the device's rate.
            device.physical = *format;
            device.nominal = format.rate_min;
            device.pending = None;
            Ok(())
        })
    }

    fn hog_owner(&self, device: u32) -> Result<i32, OsStatus> {
        self.with(device, |device| Ok(device.hog))
    }

    fn toggle_hog(&self, device: u32) -> Result<i32, OsStatus> {
        self.with(device, |device| {
            device.hog = match device.hog {
                -1 => THIS_PROCESS,
                THIS_PROCESS => -1,
                other => other,
            };
            Ok(device.hog)
        })
    }

    fn volume_range_db(&self, device: u32) -> Option<(f32, f32)> {
        self.with(device, |device| Ok(device.config.volume_db))
            .ok()
            .flatten()
    }

    fn set_volume_db(&self, device: u32, db: f32) -> Result<(), OsStatus> {
        self.with(device, |device| {
            let (min, max) = device.config.volume_db.ok_or(UNKNOWN_PROPERTY)?;
            device.capture.volume_db = Some(db.clamp(min, max));
            Ok(())
        })
    }

    fn set_mute(&self, device: u32, muted: bool) -> Result<(), OsStatus> {
        self.with(device, |device| {
            device.capture.muted = muted;
            Ok(())
        })
    }

    fn latency_frames(&self, device: u32) -> u32 {
        self.with(device, |device| {
            Ok(device.config.latency_frames + device.config.io_frames)
        })
        .unwrap_or(0)
    }

    fn start(&self, device: u32, mut render: RenderCallback) -> Result<u64, OsStatus> {
        let (channels, io_frames) = self.with(device, |state| {
            if state.other_owner() {
                return Err(PERMISSIONS);
            }
            let run = (state.physical.bits, state.nominal);
            state.capture.runs.push(run);
            Ok((
                state.config.device.output_channels as usize,
                state.config.io_frames as usize,
            ))
        })?;
        let running = Arc::new(AtomicBool::new(true));
        let thread_running = running.clone();
        let devices = self.devices.clone();
        let speed = self.speed;
        let thread = std::thread::Builder::new()
            .name("fake-hal-io".into())
            .spawn(move || {
                let mut buffer = vec![0.0_f32; io_frames * channels];
                let started = Instant::now();
                let mut cycles: u64 = 0;
                while thread_running.load(Ordering::Relaxed) {
                    let (rate, bits, float) = {
                        let devices = lock(&devices);
                        let Some(state) = devices.get(&device) else {
                            return;
                        };
                        (state.nominal, state.physical.bits, state.physical.float)
                    };
                    // Pull at the device's own pace.
                    let due = started
                        + Duration::from_secs_f64(
                            cycles as f64 * io_frames as f64 / rate.max(1.0) / speed,
                        );
                    if let Some(wait) = due.checked_duration_since(Instant::now()) {
                        std::thread::sleep(wait);
                    }
                    cycles += 1;
                    render(&mut buffer, channels);
                    let mut devices = lock(&devices);
                    let Some(state) = devices.get_mut(&device) else {
                        return;
                    };
                    let converted = buffer.iter().map(|sample| {
                        if float {
                            sample.to_bits() as i32
                        } else {
                            to_integer(*sample, bits)
                        }
                    });
                    state.capture.samples.extend(converted);
                }
            })
            .map_err(|_| ILLEGAL_OPERATION)?;
        let handle = NEXT_IO.fetch_add(1, Ordering::Relaxed);
        lock(&self.ios).insert(
            handle,
            Io {
                device,
                running,
                thread: Some(thread),
            },
        );
        Ok(handle)
    }

    fn stop(&self, device: u32, handle: u64) {
        let io = lock(&self.ios).remove(&handle);
        if let Some(mut io) = io
            && io.device == device
        {
            io.running.store(false, Ordering::Relaxed);
            if let Some(thread) = io.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn process_id(&self) -> i32 {
        THIS_PROCESS
    }
}
