//! CoreAudio output for macOS, in hog mode: Parson takes the DAC for
//! itself (no other app, no system sounds, no mixing), switches its nominal
//! sample rate to the file's own rate, and sets the stream's physical format
//! to integer PCM at the file's own bit depth. Samples then pass through the
//! HAL's float stage, which carries up to 24 bits exactly, so 16- and
//! 24-bit audio reaches the DAC bit-perfect. (32-bit integer sources are not
//! offered: the float stage keeps only their top 24 bits.)
//!
//! The logic here works on the [`HalApi`] trait: the real
//! `AudioObject*` / `AudioDeviceIOProc` API on macOS ([`hal`]), and
//! [`fake::FakeHal`] everywhere, which follows the HAL's documented rules
//! (hog mode is a toggle owned by one process, a rate change lands
//! asynchronously some time after it is asked for, the IOProc pulls float
//! buffers that the HAL converts to the physical integer format) so the
//! tests prove the logic on any machine.

pub mod fake;
#[cfg(target_os = "macos")]
pub mod hal;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::{DeviceBackend, DeviceInfo, HardwareVolume, OutputStream, SampleFormat, StreamFormat};

pub type OsStatus = i32;

const fn fourcc(code: &[u8; 4]) -> i32 {
    i32::from_be_bytes(*code)
}

pub const NO_ERR: OsStatus = 0;
/// `kAudioHardwareIllegalOperationError` ('nope').
pub const ILLEGAL_OPERATION: OsStatus = fourcc(b"nope");
/// `kAudioHardwareBadDeviceError` ('!dev').
pub const BAD_DEVICE: OsStatus = fourcc(b"!dev");
/// `kAudioDeviceUnsupportedFormatError` ('!dat').
pub const UNSUPPORTED_FORMAT: OsStatus = fourcc(b"!dat");
/// `kAudioDevicePermissionsError` ('!hog').
pub const PERMISSIONS: OsStatus = fourcc(b"!hog");
/// `kAudioHardwareUnknownPropertyError` ('who?').
pub const UNKNOWN_PROPERTY: OsStatus = fourcc(b"who?");

/// Rates listed from continuous ranges.
const PROBE_RATES: &[u32] = &[
    44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000, 705_600, 768_000,
];
/// How long a nominal rate change may take to land.
const RATE_SWITCH_TIMEOUT: Duration = Duration::from_secs(3);
/// Audio Parson keeps ahead of the device, in seconds.
const RING_SECONDS: f64 = 0.25;
/// The IOProc has not asked for audio for this long: the device stalled.
const STALL_TIMEOUT: Duration = Duration::from_secs(3);

/// What an OSStatus means, for people.
pub fn describe(status: OsStatus) -> String {
    let text = match status {
        PERMISSIONS => "another app has the device in hog mode",
        UNSUPPORTED_FORMAT => "the device does not take this format",
        BAD_DEVICE => "the device is gone",
        ILLEGAL_OPERATION => "the device refused the operation",
        UNKNOWN_PROPERTY => "the device does not have this control",
        _ => "",
    };
    let code = status.to_be_bytes();
    let readable = if code
        .iter()
        .all(|byte| byte.is_ascii_graphic() || *byte == b' ')
    {
        format!("'{}'", String::from_utf8_lossy(&code))
    } else {
        status.to_string()
    };
    if text.is_empty() {
        format!("CoreAudio error {readable}")
    } else {
        format!("{text} ({readable})")
    }
}

/// One output-capable `AudioDevice`.
#[derive(Clone, Debug, PartialEq)]
pub struct HalDevice {
    pub object_id: u32,
    /// `kAudioDevicePropertyDeviceUID`: stable across reboots.
    pub uid: String,
    /// `kAudioObjectPropertyName`.
    pub name: String,
    /// `kAudioObjectPropertyManufacturer`.
    pub manufacturer: String,
    /// `kAudioDevicePropertyTransportType` ('usb ', 'bltn', 'hdmi', ...).
    pub transport: u32,
    /// Output channels (`kAudioDevicePropertyStreamConfiguration`).
    pub output_channels: u32,
}

/// One `AudioStreamRangedDescription` of the first output stream.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysicalFormat {
    pub rate_min: f64,
    pub rate_max: f64,
    pub bits: u32,
    pub float: bool,
    pub channels: u32,
}

impl PhysicalFormat {
    fn covers(&self, rate: u32) -> bool {
        (self.rate_min..=self.rate_max).contains(&f64::from(rate))
    }
}

/// Fills one IOProc cycle: interleaved float samples, `channels` per frame.
pub type RenderCallback = Box<dyn FnMut(&mut [f32], usize) + Send>;

/// The parts of the HAL Parson uses.
pub trait HalApi: Send + Sync + 'static {
    fn output_devices(&self) -> Result<Vec<HalDevice>, OsStatus>;
    /// `kAudioDevicePropertyAvailableNominalSampleRates`.
    fn available_rates(&self, device: u32) -> Result<Vec<(f64, f64)>, OsStatus>;
    /// `kAudioDevicePropertyNominalSampleRate`.
    fn nominal_rate(&self, device: u32) -> Result<f64, OsStatus>;
    /// Asks for a new nominal rate; it lands asynchronously.
    fn set_nominal_rate(&self, device: u32, rate: f64) -> Result<(), OsStatus>;
    /// `kAudioStreamPropertyAvailablePhysicalFormats` of the first output
    /// stream.
    fn physical_formats(&self, device: u32) -> Result<Vec<PhysicalFormat>, OsStatus>;
    /// `kAudioStreamPropertyPhysicalFormat` (`rate_min == rate_max`).
    fn set_physical_format(&self, device: u32, format: &PhysicalFormat) -> Result<(), OsStatus>;
    /// `kAudioDevicePropertyHogMode`: the owning pid, or -1.
    fn hog_owner(&self, device: u32) -> Result<i32, OsStatus>;
    /// Sets `kAudioDevicePropertyHogMode`, which toggles: it takes a free
    /// device for this process and releases one this process owns. Returns
    /// the owner afterwards.
    fn toggle_hog(&self, device: u32) -> Result<i32, OsStatus>;
    /// `kAudioDevicePropertyVolumeRangeDecibels` on the output scope.
    fn volume_range_db(&self, device: u32) -> Option<(f32, f32)>;
    fn set_volume_db(&self, device: u32, db: f32) -> Result<(), OsStatus>;
    fn set_mute(&self, device: u32, muted: bool) -> Result<(), OsStatus>;
    /// Device latency + safety offset + IO buffer, in frames.
    fn latency_frames(&self, device: u32) -> u32;
    /// Creates an IOProc and starts the device; returns its handle.
    fn start(&self, device: u32, render: RenderCallback) -> Result<u64, OsStatus>;
    /// Stops the device and destroys the IOProc.
    fn stop(&self, device: u32, handle: u64);
    fn process_id(&self) -> i32;
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A short stable id from a device UID.
pub fn device_id(uid: &str) -> String {
    let cleaned: String = uid
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed: Vec<&str> = cleaned.split('-').filter(|part| !part.is_empty()).collect();
    format!("coreaudio-{}", trimmed.join("-"))
}

fn transport_name(transport: u32) -> &'static str {
    match &transport.to_be_bytes() {
        b"usb " => "USB",
        b"bltn" => "Built-in",
        b"hdmi" => "HDMI",
        b"dprt" => "DisplayPort",
        b"thun" => "Thunderbolt",
        b"1394" => "FireWire",
        b"pci " => "PCI",
        b"blue" | b"blea" => "Bluetooth",
        b"airp" => "AirPlay",
        b"virt" => "Virtual",
        b"grup" | b"agrp" => "Aggregate",
        _ => "Audio device",
    }
}

fn bits_of(format: SampleFormat) -> u32 {
    match format {
        SampleFormat::S16 => 16,
        SampleFormat::S24Packed | SampleFormat::S32 => 24,
    }
}

#[derive(Clone, Debug)]
struct Known {
    info: DeviceInfo,
    object_id: u32,
}

pub struct CoreAudioBackend<A: HalApi> {
    api: Arc<A>,
    known: Mutex<HashMap<String, Known>>,
}

impl<A: HalApi> CoreAudioBackend<A> {
    pub fn new(api: A) -> Self {
        Self {
            api: Arc::new(api),
            known: Mutex::new(HashMap::new()),
        }
    }

    pub fn api(&self) -> &A {
        &self.api
    }

    fn describe_device(&self, device: &HalDevice) -> Option<DeviceInfo> {
        let ranges = self.api.available_rates(device.object_id).ok()?;
        let mut rates: Vec<u32> = Vec::new();
        for (low, high) in ranges {
            if (high - low).abs() < 0.5 {
                let rate = low.round() as u32;
                if rate > 0 {
                    rates.push(rate);
                }
            } else {
                rates.extend(
                    PROBE_RATES
                        .iter()
                        .copied()
                        .filter(|rate| (low..=high).contains(&f64::from(*rate))),
                );
            }
        }
        rates.sort_unstable();
        rates.dedup();
        if rates.is_empty() {
            return None;
        }
        let physical = self
            .api
            .physical_formats(device.object_id)
            .unwrap_or_default();
        // Integer depths carry 16 and 24 bits exactly; a float-only device
        // (built-in outputs) takes 24 bits exactly too.
        let deepest = physical
            .iter()
            .map(|format| if format.float { 24 } else { format.bits })
            .max()
            .unwrap_or(16);
        let mut formats = vec![SampleFormat::S16];
        if deepest >= 24 {
            formats.push(SampleFormat::S24Packed);
        }
        let hardware_volume = self
            .api
            .volume_range_db(device.object_id)
            .filter(|(min, max)| max > min)
            .map(|(min_db, max_db)| HardwareVolume {
                min_db,
                max_db,
                step_db: 0.5,
            });
        let owner = self.api.hog_owner(device.object_id).unwrap_or(-1);
        let busy = owner != -1 && owner != self.api.process_id();
        let transport = transport_name(device.transport);
        let description = if device.manufacturer.is_empty() {
            format!("{transport} · CoreAudio hog mode")
        } else {
            format!("{} · {transport} · CoreAudio hog mode", device.manufacturer)
        };
        Some(DeviceInfo {
            id: device_id(&device.uid),
            name: device.name.clone(),
            description,
            sample_rates: rates,
            formats,
            max_channels: u8::try_from(device.output_channels).unwrap_or(u8::MAX),
            hardware_volume,
            busy,
        })
    }

    fn lookup(&self, device_id: &str) -> Option<Known> {
        if let Some(known) = lock(&self.known).get(device_id) {
            return Some(known.clone());
        }
        self.devices();
        lock(&self.known).get(device_id).cloned()
    }

    /// Asks for `rate` and waits until the device runs at it.
    fn switch_rate(&self, object_id: u32, rate: u32) -> Result<(), OsStatus> {
        let target = f64::from(rate);
        let current = self.api.nominal_rate(object_id)?;
        if (current - target).abs() < 0.5 {
            return Ok(());
        }
        self.api.set_nominal_rate(object_id, target)?;
        let deadline = Instant::now() + RATE_SWITCH_TIMEOUT;
        while Instant::now() < deadline {
            if (self.api.nominal_rate(object_id)? - target).abs() < 0.5 {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Err(ILLEGAL_OPERATION)
    }

    /// Integer PCM at the source's depth (or the shallowest deeper one) at
    /// `rate`, so the HAL's float-to-integer step is exact.
    fn choose_physical(&self, object_id: u32, format: StreamFormat) -> Option<PhysicalFormat> {
        let wanted = bits_of(format.format);
        let formats = self.api.physical_formats(object_id).ok()?;
        formats
            .iter()
            .filter(|physical| !physical.float && physical.covers(format.sample_rate))
            .filter(|physical| physical.bits >= wanted && physical.bits <= 32)
            .filter(|physical| physical.channels >= u32::from(format.channels))
            .min_by_key(|physical| (physical.bits, physical.channels))
            .map(|physical| PhysicalFormat {
                rate_min: f64::from(format.sample_rate),
                rate_max: f64::from(format.sample_rate),
                ..*physical
            })
    }
}

impl<A: HalApi> DeviceBackend for CoreAudioBackend<A> {
    fn name(&self) -> &'static str {
        "CoreAudio"
    }

    fn devices(&self) -> Vec<DeviceInfo> {
        let Ok(devices) = self.api.output_devices() else {
            return Vec::new();
        };
        let mut known = lock(&self.known);
        let mut listed = Vec::new();
        for device in devices {
            if device.output_channels == 0 {
                continue;
            }
            let Some(info) = self.describe_device(&device) else {
                continue;
            };
            known.insert(
                info.id.clone(),
                Known {
                    info: info.clone(),
                    object_id: device.object_id,
                },
            );
            listed.push(info);
        }
        listed
    }

    fn open(
        &self,
        device_id: &str,
        format: StreamFormat,
        exclusive: bool,
    ) -> Result<Box<dyn OutputStream>, String> {
        let known = self
            .lookup(device_id)
            .ok_or_else(|| "device unplugged".to_string())?;
        let name = known.info.name.clone();
        let object_id = known.object_id;
        if !known.info.formats.contains(&format.format) {
            return Err(format!(
                "{name} does not take {} bit-perfect through CoreAudio",
                format.format.name()
            ));
        }
        if !known.info.sample_rates.contains(&format.sample_rate) {
            return Err(format!("{name} does not take {} Hz", format.sample_rate));
        }
        if format.channels == 0 || u32::from(format.channels) > u32::from(known.info.max_channels) {
            return Err(format!(
                "{name} has fewer than {} channels",
                format.channels
            ));
        }
        let pid = self.api.process_id();
        let mut hogged = false;
        if exclusive {
            let owner = self
                .api
                .hog_owner(object_id)
                .map_err(|status| format!("{name}: {}", describe(status)))?;
            if owner != -1 && owner != pid {
                return Err(format!("{name}: {}", describe(PERMISSIONS)));
            }
            if owner == -1 {
                let now = self
                    .api
                    .toggle_hog(object_id)
                    .map_err(|status| format!("{name}: {}", describe(status)))?;
                if now != pid {
                    return Err(format!("{name}: {}", describe(PERMISSIONS)));
                }
            }
            hogged = true;
        }
        let release = |api: &A| {
            if hogged && api.hog_owner(object_id).unwrap_or(-1) == pid {
                let _ = api.toggle_hog(object_id);
            }
        };
        if let Err(status) = self.switch_rate(object_id, format.sample_rate) {
            release(&self.api);
            return Err(format!(
                "{name} did not switch to {} Hz: {}",
                format.sample_rate,
                describe(status)
            ));
        }
        let physical = self.choose_physical(object_id, format);
        if let Some(physical) = &physical
            && let Err(status) = self.api.set_physical_format(object_id, physical)
        {
            release(&self.api);
            return Err(format!("{name}: {}", describe(status)));
        }
        let channels = usize::from(format.channels);
        let ring = Arc::new(Ring {
            state: Mutex::new(RingState::default()),
            space: Condvar::new(),
            pulls: AtomicU64::new(0),
            running: AtomicBool::new(false),
        });
        let latency = self.api.latency_frames(object_id);
        Ok(Box::new(CoreAudioStream {
            api: self.api.clone(),
            object_id,
            format,
            channels,
            ring,
            capacity_frames: (f64::from(format.sample_rate) * RING_SECONDS) as usize,
            latency_frames: u64::from(latency),
            io: None,
            hogged,
            pid,
            name,
        }))
    }

    fn set_hardware_volume(&self, device_id: &str, db: f32) -> Result<(), String> {
        let known = self
            .lookup(device_id)
            .ok_or_else(|| "device unplugged".to_string())?;
        let range = known
            .info
            .hardware_volume
            .ok_or_else(|| format!("{} has no hardware volume", known.info.name))?;
        self.api
            .set_volume_db(known.object_id, db.clamp(range.min_db, range.max_db))
            .map_err(describe)
    }

    fn set_hardware_mute(&self, device_id: &str, muted: bool) -> Result<(), String> {
        let known = self
            .lookup(device_id)
            .ok_or_else(|| "device unplugged".to_string())?;
        self.api.set_mute(known.object_id, muted).map_err(describe)
    }
}

#[derive(Default)]
struct RingState {
    samples: VecDeque<f32>,
    paused: bool,
}

/// Audio between Parson's writer thread and the device's IO thread.
struct Ring {
    state: Mutex<RingState>,
    space: Condvar,
    /// IOProc cycles so far (to notice a stalled device).
    pulls: AtomicU64,
    running: AtomicBool,
}

impl Ring {
    /// The IOProc: fills `out` (interleaved, `device_channels` per frame)
    /// from the ring, silence where there is none.
    fn render(&self, out: &mut [f32], device_channels: usize, channels: usize) {
        self.pulls.fetch_add(1, Ordering::Relaxed);
        out.fill(0.0);
        let device_channels = device_channels.max(1);
        let frames = out.len() / device_channels;
        let mut state = lock(&self.state);
        if state.paused {
            return;
        }
        let available = state.samples.len() / channels.max(1);
        let take = frames.min(available);
        for frame in 0..take {
            for channel in 0..channels {
                let sample = state.samples.pop_front().unwrap_or(0.0);
                if channel < device_channels {
                    out[frame * device_channels + channel] = sample;
                }
            }
        }
        drop(state);
        self.space.notify_all();
    }
}

/// Integer PCM to the HAL's float, exactly (24 significant bits at most).
fn to_float(data: &[u8], format: SampleFormat, out: &mut VecDeque<f32>) {
    match format {
        SampleFormat::S16 => {
            for sample in data.as_chunks::<2>().0 {
                let value = i16::from_le_bytes(*sample);
                out.push_back(f32::from(value) / 32_768.0);
            }
        }
        SampleFormat::S24Packed => {
            for sample in data.as_chunks::<3>().0 {
                let value = i32::from_le_bytes([0, sample[0], sample[1], sample[2]]) >> 8;
                out.push_back(value as f32 / 8_388_608.0);
            }
        }
        SampleFormat::S32 => {
            for sample in data.as_chunks::<4>().0 {
                let value = i32::from_le_bytes(*sample) >> 8;
                out.push_back(value as f32 / 8_388_608.0);
            }
        }
    }
}

pub struct CoreAudioStream<A: HalApi> {
    api: Arc<A>,
    object_id: u32,
    format: StreamFormat,
    channels: usize,
    ring: Arc<Ring>,
    capacity_frames: usize,
    latency_frames: u64,
    io: Option<u64>,
    hogged: bool,
    pid: i32,
    name: String,
}

impl<A: HalApi> CoreAudioStream<A> {
    fn ensure_running(&mut self) -> Result<(), String> {
        if self.io.is_some() {
            return Ok(());
        }
        let ring = self.ring.clone();
        let channels = self.channels;
        let handle = self
            .api
            .start(
                self.object_id,
                Box::new(move |out, device_channels| ring.render(out, device_channels, channels)),
            )
            .map_err(|status| format!("{}: {}", self.name, describe(status)))?;
        self.ring.running.store(true, Ordering::Relaxed);
        self.io = Some(handle);
        Ok(())
    }

    fn stop_io(&mut self) {
        if let Some(handle) = self.io.take() {
            self.api.stop(self.object_id, handle);
            self.ring.running.store(false, Ordering::Relaxed);
        }
    }

    fn queued_frames(&self) -> u64 {
        (lock(&self.ring.state).samples.len() / self.channels.max(1)) as u64
    }
}

impl<A: HalApi> OutputStream for CoreAudioStream<A> {
    fn format(&self) -> StreamFormat {
        self.format
    }

    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        if !data.len().is_multiple_of(self.format.frame_bytes()) {
            return Err("partial frame".to_string());
        }
        let limit = self.capacity_frames * self.channels;
        let mut state = lock(&self.ring.state);
        // Room first, like a device buffer: block while the ring is full.
        let mut pulls = self.ring.pulls.load(Ordering::Relaxed);
        let mut since = Instant::now();
        while state.samples.len() >= limit && self.io.is_some() {
            let (next, _) = self
                .ring
                .space
                .wait_timeout(state, Duration::from_millis(20))
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state = next;
            let now = self.ring.pulls.load(Ordering::Relaxed);
            if now != pulls || state.paused {
                pulls = now;
                since = Instant::now();
            } else if since.elapsed() > STALL_TIMEOUT {
                return Err(format!("{} stopped asking for audio", self.name));
            }
        }
        to_float(data, self.format.format, &mut state.samples);
        let paused = state.paused;
        drop(state);
        if !paused {
            self.ensure_running()?;
        }
        Ok(())
    }

    fn delay_frames(&mut self) -> u64 {
        let queued = self.queued_frames();
        if self.io.is_some() {
            queued + self.latency_frames
        } else {
            queued
        }
    }

    fn pause(&mut self, paused: bool) -> Result<(), String> {
        // The device keeps running (and stays hogged), playing silence.
        lock(&self.ring.state).paused = paused;
        self.ring.space.notify_all();
        Ok(())
    }

    fn drop_buffer(&mut self) {
        lock(&self.ring.state).samples.clear();
        self.ring.space.notify_all();
    }

    fn drain(&mut self) {
        if self.queued_frames() > 0 && self.ensure_running().is_err() {
            return;
        }
        let deadline = Instant::now()
            + Duration::from_secs_f64(
                self.queued_frames() as f64 / f64::from(self.format.sample_rate.max(1)),
            )
            + STALL_TIMEOUT;
        while self.queued_frames() > 0 && Instant::now() < deadline {
            if lock(&self.ring.state).paused {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        // What the device already took plays out.
        std::thread::sleep(Duration::from_secs_f64(
            self.latency_frames as f64 / f64::from(self.format.sample_rate.max(1)),
        ));
    }
}

impl<A: HalApi> Drop for CoreAudioStream<A> {
    fn drop(&mut self) {
        self.stop_io();
        if self.hogged && self.api.hog_owner(self.object_id).unwrap_or(-1) == self.pid {
            let _ = self.api.toggle_hog(self.object_id);
        }
    }
}

#[cfg(test)]
mod tests;
