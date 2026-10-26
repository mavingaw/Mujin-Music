//! Simulated audio devices: they consume audio in (optionally sped-up)
//! real time like hardware, enforce their formats like hardware, and keep
//! what they were sent so tests can prove playback is bit-perfect. Used by
//! the tests and by `PARSON_LOCAL_AUDIO=simulated` for demos on machines
//! without a DAC.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{DeviceBackend, DeviceInfo, HardwareVolume, OutputStream, SampleFormat, StreamFormat};

/// Keeps at most this much of what a device was sent.
const MAX_CAPTURE_BYTES: usize = 64 * 1024 * 1024;

/// What a simulated device received.
#[derive(Clone, Debug, Default)]
pub struct Capture {
    /// Every format the device was opened with, in order.
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

impl SimulatedBackend {
    pub fn new(devices: Vec<DeviceInfo>, speed: f64) -> Self {
        Self {
            devices: Mutex::new(devices),
            captures: Arc::new(Mutex::new(HashMap::new())),
            speed: speed.max(0.01),
        }
    }

    /// A hi-res USB DAC with a hardware mixer, and a 48 kHz/16-bit HDMI
    /// output without one.
    pub fn with_default_devices(speed: f64) -> Self {
        Self::new(
            vec![
                DeviceInfo {
                    id: "sim-usb-dac".into(),
                    name: "USB DAC (simulated)".into(),
                    description: "Simulated USB Audio 2.0 DAC".into(),
                    sample_rates: vec![
                        44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000,
                    ],
                    formats: vec![
                        SampleFormat::S16,
                        SampleFormat::S24Packed,
                        SampleFormat::S32,
                    ],
                    max_channels: 2,
                    hardware_volume: Some(HardwareVolume {
                        min_db: -127.5,
                        max_db: 0.0,
                        step_db: 0.5,
                    }),
                    busy: false,
                },
                DeviceInfo {
                    id: "sim-hdmi".into(),
                    name: "HDMI output (simulated)".into(),
                    description: "Simulated HDMI audio".into(),
                    sample_rates: vec![48_000],
                    formats: vec![SampleFormat::S16],
                    max_channels: 2,
                    hardware_volume: None,
                    busy: false,
                },
            ],
            speed,
        )
    }

    pub fn capture(&self, device_id: &str) -> Capture {
        self.captures
            .lock()
            .unwrap()
            .get(device_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Simulates plugging a device in (or out, with `None`).
    pub fn set_devices(&self, devices: Vec<DeviceInfo>) {
        *self.devices.lock().unwrap() = devices;
    }
}

struct SimulatedStream {
    device_id: String,
    format: StreamFormat,
    captures: Arc<Mutex<HashMap<String, Capture>>>,
    speed: f64,
    /// Frames the device will have played by now, counted from `started`.
    played_before: f64,
    started: Option<Instant>,
    written: u64,
    /// The device buffer: about 100 ms.
    capacity: u64,
}

impl SimulatedStream {
    fn played(&self) -> u64 {
        let running = self
            .started
            .map(|started| {
                started.elapsed().as_secs_f64() * f64::from(self.format.sample_rate) * self.speed
            })
            .unwrap_or(0.0);
        ((self.played_before + running) as u64).min(self.written)
    }
}

impl OutputStream for SimulatedStream {
    fn format(&self) -> StreamFormat {
        self.format
    }

    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        if !data.len().is_multiple_of(self.format.frame_bytes()) {
            return Err("partial frame".to_string());
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
            let mut captures = self.captures.lock().unwrap();
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

    fn pause(&mut self, paused: bool) -> Result<(), String> {
        if paused {
            self.played_before = self.played() as f64;
            self.started = None;
        } else if self.started.is_none() {
            self.started = Some(Instant::now());
        }
        Ok(())
    }

    fn drop_buffer(&mut self) {
        self.written = self.played();
        self.played_before = self.written as f64;
        self.started = None;
    }

    fn drain(&mut self) {
        while self.started.is_some() && self.played() < self.written {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

impl DeviceBackend for SimulatedBackend {
    fn name(&self) -> &'static str {
        "Simulated"
    }

    fn devices(&self) -> Vec<DeviceInfo> {
        self.devices.lock().unwrap().clone()
    }

    fn open(
        &self,
        device_id: &str,
        format: StreamFormat,
        exclusive: bool,
    ) -> Result<Box<dyn OutputStream>, String> {
        let device = self
            .devices()
            .into_iter()
            .find(|device| device.id == device_id)
            .ok_or_else(|| "device unplugged".to_string())?;
        // Like a hw: device, refuse anything not native.
        if !device.sample_rates.contains(&format.sample_rate)
            || !device.formats.contains(&format.format)
            || format.channels > device.max_channels
        {
            return Err(format!(
                "{} does not take {} Hz {}",
                device.name,
                format.sample_rate,
                format.format.name()
            ));
        }
        self.captures
            .lock()
            .unwrap()
            .entry(device_id.to_string())
            .or_default()
            .opened
            .push((format, exclusive));
        let capacity = u64::from(format.sample_rate) / 10;
        Ok(Box::new(SimulatedStream {
            device_id: device_id.to_string(),
            format,
            captures: self.captures.clone(),
            speed: self.speed,
            played_before: 0.0,
            started: None,
            written: 0,
            capacity,
        }))
    }

    fn set_hardware_volume(&self, device_id: &str, db: f32) -> Result<(), String> {
        self.captures
            .lock()
            .unwrap()
            .entry(device_id.to_string())
            .or_default()
            .volume_db = Some(db);
        Ok(())
    }

    fn set_hardware_mute(&self, device_id: &str, muted: bool) -> Result<(), String> {
        self.captures
            .lock()
            .unwrap()
            .entry(device_id.to_string())
            .or_default()
            .muted = muted;
        Ok(())
    }
}
