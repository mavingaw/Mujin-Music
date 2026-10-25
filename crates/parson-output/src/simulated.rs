//! A software output device with no sound card behind it.
//!
//! It accepts the formats of a typical USB DAC (44.1 to 192 kHz, 16 to 32
//! bit, stereo) and consumes audio in real time, so the native output path,
//! its settings and its signal path can be exercised on machines without
//! audio hardware (CI, containers, demos). The server only lists it when
//! `PARSON_NATIVE_OUTPUT_SIMULATE=1` is set, and it is labelled as simulated
//! everywhere it appears.

use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{Backend, DeviceCaps, DeviceInfo, Driver, SampleFormat, Sink, StreamFormat};

pub const SIMULATED_DEVICE_ID: &str = "simulated:usb-dac";

pub fn simulated_caps() -> DeviceCaps {
    DeviceCaps {
        sample_rates: vec![44_100, 48_000, 88_200, 96_000, 176_400, 192_000],
        sample_formats: vec![
            SampleFormat::S16,
            SampleFormat::S24Packed,
            SampleFormat::S32,
        ],
        min_channels: 2,
        max_channels: 2,
    }
}

pub struct SimulatedBackend;

impl Backend for SimulatedBackend {
    fn driver(&self) -> Driver {
        Driver::Simulated
    }

    fn devices(&self) -> Vec<DeviceInfo> {
        vec![DeviceInfo {
            id: SIMULATED_DEVICE_ID.into(),
            name: "Simulated USB DAC".into(),
            detail: "Software device, no audio hardware".into(),
            driver: Driver::Simulated,
            exclusive_capable: true,
            caps: Some(simulated_caps()),
            probe_error: None,
            is_default: false,
        }]
    }

    fn open(
        &self,
        device_id: &str,
        format: StreamFormat,
        _exclusive: bool,
    ) -> io::Result<Box<dyn Sink>> {
        if device_id != SIMULATED_DEVICE_ID {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no such simulated device",
            ));
        }
        let caps = simulated_caps();
        if !caps.sample_rates.contains(&format.sample_rate)
            || !caps.sample_formats.contains(&format.sample_format)
            || format.channels != 2
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "the simulated device does not accept this format",
            ));
        }
        Ok(Box::new(RealtimeSink::new(format)))
    }
}

/// Discards audio at the stream's real-time rate.
struct RealtimeSink {
    format: StreamFormat,
    started: Option<Instant>,
    frames: u64,
}

impl RealtimeSink {
    fn new(format: StreamFormat) -> Self {
        Self {
            format,
            started: None,
            frames: 0,
        }
    }

    fn played_frames(&self) -> u64 {
        self.started.map_or(0, |started| {
            (started.elapsed().as_secs_f64() * f64::from(self.format.sample_rate)) as u64
        })
    }
}

/// Audio the simulated device keeps buffered, like a small hardware FIFO.
const BUFFER_SECONDS: f64 = 0.1;

impl Sink for RealtimeSink {
    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        let started = *self.started.get_or_insert_with(Instant::now);
        self.frames += (data.len() / self.format.frame_bytes().max(1)) as u64;
        let ahead = self.frames as f64 / f64::from(self.format.sample_rate) - BUFFER_SECONDS;
        let elapsed = started.elapsed().as_secs_f64();
        if ahead > elapsed {
            std::thread::sleep(Duration::from_secs_f64(ahead - elapsed));
        }
        Ok(())
    }

    fn delay_frames(&mut self) -> u64 {
        self.frames.saturating_sub(self.played_frames())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.started = None;
        self.frames = 0;
        Ok(())
    }

    fn drain(&mut self) -> io::Result<()> {
        let remaining = self.delay_frames();
        std::thread::sleep(Duration::from_secs_f64(
            remaining as f64 / f64::from(self.format.sample_rate),
        ));
        self.flush()
    }
}

/// Bytes a [`CaptureSink`] received, shared with the test that made it.
#[derive(Clone, Default)]
pub struct CaptureLog(Arc<Mutex<Vec<u8>>>);

impl CaptureLog {
    pub fn bytes(&self) -> Vec<u8> {
        self.0.lock().map(|bytes| bytes.clone()).unwrap_or_default()
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
        } else if let Ok(mut bytes) = self.log.0.lock() {
            bytes.extend_from_slice(data);
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
    fn the_simulated_device_rejects_formats_it_does_not_list() {
        let backend = SimulatedBackend;
        let device = &backend.devices()[0];
        assert_eq!(device.driver, Driver::Simulated);
        let good = StreamFormat {
            sample_rate: 96_000,
            sample_format: SampleFormat::S24Packed,
            channels: 2,
        };
        assert!(backend.open(SIMULATED_DEVICE_ID, good, true).is_ok());
        let bad = StreamFormat {
            sample_rate: 384_000,
            ..good
        };
        assert!(backend.open(SIMULATED_DEVICE_ID, bad, true).is_err());
        assert!(backend.open("other", good, true).is_err());
    }

    #[test]
    fn the_realtime_sink_paces_writes_and_reports_its_buffer() {
        let format = StreamFormat {
            sample_rate: 48_000,
            sample_format: SampleFormat::S16,
            channels: 2,
        };
        let mut sink = RealtimeSink::new(format);
        let started = Instant::now();
        // 0.2 s of audio: the first 0.1 s fills the buffer, the rest waits.
        sink.write(&vec![0; 9_600 * 4]).unwrap();
        assert!(started.elapsed() >= Duration::from_millis(90));
        assert!(sink.delay_frames() <= 9_600);
        sink.flush().unwrap();
        assert_eq!(sink.delay_frames(), 0);
    }
}
