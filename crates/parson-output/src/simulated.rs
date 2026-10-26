//! Simulated audio devices: they consume audio in (optionally sped-up)
//! real time like hardware, enforce their formats like a `hw:` device, and
//! keep what they were sent so tests can prove playback is bit-perfect.
//!
//! The server lists them when `PARSON_LOCAL_AUDIO=simulated` (or the older
//! `PARSON_NATIVE_OUTPUT_SIMULATE=1`) asks for them, for demos and CI on
//! machines without a DAC, and labels them as simulated everywhere.

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{
    Backend, DeviceCaps, DeviceInfo, Driver, HardwareVolume, SampleFormat, Sink, StreamFormat,
};

/// The simulated hi-res USB DAC.
pub const SIMULATED_DEVICE_ID: &str = "sim-usb-dac";
/// The simulated 48 kHz/16-bit HDMI output without a mixer.
pub const SIMULATED_HDMI_ID: &str = "sim-hdmi";

/// Keeps at most this much of what a device was sent.
const MAX_CAPTURE_BYTES: usize = 64 * 1024 * 1024;

/// What the simulated USB DAC takes: PCM up to 384 kHz/32-bit and native
/// DSD64 and DSD128 in `DSD_U32_BE`.
pub fn simulated_caps() -> DeviceCaps {
    DeviceCaps {
        sample_rates: vec![
            44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000,
        ],
        sample_formats: vec![
            SampleFormat::S16,
            SampleFormat::S24Packed,
            SampleFormat::S32,
        ],
        min_channels: 2,
        max_channels: 2,
        dsd_formats: vec![SampleFormat::DsdU32Be],
        dsd_rates: vec![2_822_400, 5_644_800],
    }
}

/// A simulated device with `caps`.
pub fn simulated_device(
    id: &str,
    name: &str,
    detail: &str,
    caps: DeviceCaps,
    hardware_volume: Option<HardwareVolume>,
) -> DeviceInfo {
    DeviceInfo {
        id: id.to_string(),
        name: name.to_string(),
        detail: detail.to_string(),
        driver: Driver::Simulated,
        exclusive_capable: true,
        caps: Some(caps),
        probe_error: None,
        is_default: false,
        hardware: None,
        address: None,
        network: None,
        hardware_volume,
        busy: false,
    }
}

/// What a simulated device received.
#[derive(Clone, Debug, Default)]
pub struct Capture {
    /// Every format the device was opened with, and whether exclusively.
    pub opened: Vec<(StreamFormat, bool)>,
    pub bytes: Vec<u8>,
    pub volume_db: Option<f32>,
    pub muted: bool,
}

pub struct SimulatedBackend {
    devices: Mutex<Vec<DeviceInfo>>,
    captures: Arc<Mutex<HashMap<String, Capture>>>,
    /// Playback speed: 1.0 is real time.
    speed: f64,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl SimulatedBackend {
    pub fn new(devices: Vec<DeviceInfo>, speed: f64) -> Self {
        Self {
            devices: Mutex::new(devices),
            captures: Arc::new(Mutex::new(HashMap::new())),
            speed: speed.max(0.01),
        }
    }

    /// A hi-res USB DAC with a hardware mixer and native DSD, and a
    /// 48 kHz/16-bit HDMI output without a mixer.
    pub fn with_default_devices(speed: f64) -> Self {
        Self::new(
            vec![
                simulated_device(
                    SIMULATED_DEVICE_ID,
                    "USB DAC (simulated)",
                    "Simulated USB Audio 2.0 DAC",
                    simulated_caps(),
                    Some(HardwareVolume {
                        min_db: -127.5,
                        max_db: 0.0,
                        step_db: 0.5,
                    }),
                ),
                simulated_device(
                    SIMULATED_HDMI_ID,
                    "HDMI output (simulated)",
                    "Simulated HDMI audio",
                    DeviceCaps {
                        sample_rates: vec![48_000],
                        sample_formats: vec![SampleFormat::S16],
                        min_channels: 2,
                        max_channels: 2,
                        dsd_formats: Vec::new(),
                        dsd_rates: Vec::new(),
                    },
                    None,
                ),
            ],
            speed,
        )
    }

    /// The demo devices at real time.
    pub fn demo() -> Self {
        Self::with_default_devices(1.0)
    }

    /// One USB DAC that stops at 192 kHz (and takes DSD64 and DSD128
    /// natively), so higher rates have to be converted.
    pub fn dac_up_to_192k() -> Self {
        let mut caps = simulated_caps();
        caps.sample_rates.retain(|rate| *rate <= 192_000);
        Self::new(
            vec![simulated_device(
                SIMULATED_DEVICE_ID,
                "Simulated USB DAC",
                "Software device, no audio hardware",
                caps,
                None,
            )],
            1.0,
        )
    }

    pub fn capture(&self, device_id: &str) -> Capture {
        lock(&self.captures)
            .get(device_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Simulates plugging devices in or out.
    pub fn set_devices(&self, devices: Vec<DeviceInfo>) {
        *lock(&self.devices) = devices;
    }
}

/// Whether `caps` take `format` exactly, like a `hw:` device.
fn accepts(caps: &DeviceCaps, format: StreamFormat) -> bool {
    let layout = if format.sample_format.is_dsd() {
        caps.dsd_formats.contains(&format.sample_format)
            && caps
                .dsd_rates
                .iter()
                .any(|rate| format.sample_format.dsd_frame_rate(*rate) == format.sample_rate)
    } else {
        caps.sample_rates.contains(&format.sample_rate)
            && caps.sample_formats.contains(&format.sample_format)
    };
    layout && (1..=caps.max_channels).contains(&format.channels)
}

impl Backend for SimulatedBackend {
    fn driver(&self) -> Driver {
        Driver::Simulated
    }

    fn devices(&self) -> Vec<DeviceInfo> {
        lock(&self.devices).clone()
    }

    fn open(
        &self,
        device_id: &str,
        format: StreamFormat,
        exclusive: bool,
    ) -> io::Result<Box<dyn Sink>> {
        let device = self
            .devices()
            .into_iter()
            .find(|device| device.id == device_id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "device unplugged"))?;
        if !device
            .caps
            .as_ref()
            .is_some_and(|caps| accepts(caps, format))
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "{} does not take {} Hz {}",
                    device.name,
                    format.sample_rate,
                    format.sample_format.alsa_name()
                ),
            ));
        }
        lock(&self.captures)
            .entry(device_id.to_string())
            .or_default()
            .opened
            .push((format, exclusive));
        Ok(Box::new(SimulatedSink {
            device_id: device_id.to_string(),
            format,
            captures: self.captures.clone(),
            speed: self.speed,
            played_before: 0.0,
            started: None,
            written: 0,
            capacity: u64::from(format.sample_rate) / 10,
        }))
    }

    fn set_hardware_volume(&self, device_id: &str, db: f32) -> io::Result<()> {
        lock(&self.captures)
            .entry(device_id.to_string())
            .or_default()
            .volume_db = Some(db);
        Ok(())
    }

    fn set_hardware_mute(&self, device_id: &str, muted: bool) -> io::Result<()> {
        lock(&self.captures)
            .entry(device_id.to_string())
            .or_default()
            .muted = muted;
        Ok(())
    }
}

/// A simulated device stream: a ~100 ms buffer drained in real time.
struct SimulatedSink {
    device_id: String,
    format: StreamFormat,
    captures: Arc<Mutex<HashMap<String, Capture>>>,
    speed: f64,
    /// Frames played before `started`.
    played_before: f64,
    started: Option<Instant>,
    written: u64,
    capacity: u64,
}

impl SimulatedSink {
    fn played(&self) -> u64 {
        let running = self.started.map_or(0.0, |started| {
            started.elapsed().as_secs_f64() * f64::from(self.format.sample_rate) * self.speed
        });
        ((self.played_before + running) as u64).min(self.written)
    }
}

impl Sink for SimulatedSink {
    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        if !data.len().is_multiple_of(self.format.frame_bytes()) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "partial frame"));
        }
        if self.started.is_none() {
            self.played_before = self.played_before.min(self.written as f64);
            self.started = Some(Instant::now());
        }
        // Block like hardware while the buffer is full.
        while self.written.saturating_sub(self.played()) > self.capacity {
            std::thread::sleep(Duration::from_millis(2));
        }
        {
            let mut captures = lock(&self.captures);
            let capture = captures.entry(self.device_id.clone()).or_default();
            if capture.bytes.len() + data.len() <= MAX_CAPTURE_BYTES {
                capture.bytes.extend_from_slice(data);
            }
        }
        self.written += (data.len() / self.format.frame_bytes()) as u64;
        Ok(())
    }

    fn delay_frames(&mut self) -> u64 {
        self.written.saturating_sub(self.played())
    }

    fn pause(&mut self, paused: bool) -> io::Result<()> {
        if paused {
            self.played_before = self.played() as f64;
            self.started = None;
        } else if self.started.is_none() {
            self.started = Some(Instant::now());
        }
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.written = self.played();
        self.played_before = self.written as f64;
        self.started = None;
        Ok(())
    }

    fn drain(&mut self) -> io::Result<()> {
        while self.started.is_some() && self.played() < self.written {
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    }
}

/// Bytes a [`CaptureSink`] received, shared with the test that made it.
#[derive(Clone, Default)]
pub struct CaptureLog(Arc<Mutex<Vec<u8>>>);

impl CaptureLog {
    pub fn bytes(&self) -> Vec<u8> {
        lock(&self.0).clone()
    }
}

/// Records what is written, for tests. `paced` sleeps a little per write
/// so a player keeps running long enough to be controlled.
pub struct CaptureSink {
    log: CaptureLog,
    paced: bool,
}

impl CaptureSink {
    pub fn new(log: CaptureLog, paced: bool) -> Self {
        Self { log, paced }
    }
}

impl Sink for CaptureSink {
    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        if self.paced {
            std::thread::sleep(Duration::from_millis(2));
        } else {
            lock(&self.log.0).extend_from_slice(data);
        }
        Ok(())
    }

    fn delay_frames(&mut self) -> u64 {
        0
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn drain(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulated_devices_refuse_formats_they_do_not_list() {
        let backend = SimulatedBackend::demo();
        let device = &backend.devices()[0];
        assert_eq!(device.driver, Driver::Simulated);
        let good = StreamFormat {
            sample_rate: 96_000,
            sample_format: SampleFormat::S24Packed,
            channels: 2,
        };
        assert!(backend.open(SIMULATED_DEVICE_ID, good, true).is_ok());
        let dsd64 = StreamFormat {
            sample_rate: 88_200,
            sample_format: SampleFormat::DsdU32Be,
            channels: 2,
        };
        assert!(backend.open(SIMULATED_DEVICE_ID, dsd64, true).is_ok());
        let bad = StreamFormat {
            sample_rate: 768_000,
            ..good
        };
        assert!(backend.open(SIMULATED_DEVICE_ID, bad, true).is_err());
        assert!(
            backend.open(SIMULATED_HDMI_ID, good, true).is_err(),
            "HDMI takes 48 kHz/16-bit only"
        );
        assert!(backend.open("other", good, true).is_err());
    }

    #[test]
    fn a_simulated_device_paces_writes_and_reports_its_buffer() {
        let backend = SimulatedBackend::demo();
        let format = StreamFormat {
            sample_rate: 48_000,
            sample_format: SampleFormat::S16,
            channels: 2,
        };
        let mut sink = backend.open(SIMULATED_HDMI_ID, format, true).unwrap();
        let started = Instant::now();
        // 0.2 s of audio in 20 ms writes: the first 0.1 s fills the
        // buffer, the rest waits for room like hardware.
        for _ in 0..10 {
            sink.write(&[0; 960 * 4]).unwrap();
        }
        assert!(started.elapsed() >= Duration::from_millis(80));
        assert!(sink.delay_frames() <= 4_800 + 960);
        sink.pause(true).unwrap();
        let held = sink.delay_frames();
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(sink.delay_frames(), held, "nothing plays while paused");
        sink.flush().unwrap();
        assert_eq!(sink.delay_frames(), 0);
        assert_eq!(backend.capture(SIMULATED_HDMI_ID).bytes.len(), 9_600 * 4);
    }
}
