//! Audio devices of one machine: the types and traits shared by the Parson
//! server (its own outputs) and Parson Bridge (outputs of another machine),
//! plus the backends: ALSA on Linux (incl. Raspberry Pi) and simulated
//! devices for tests and demos.
//!
//! Playback is bit-perfect when it can be: a stream is opened at exactly the
//! file's own rate and depth, exclusively, and samples pass unchanged.

#[cfg(target_os = "linux")]
pub mod alsa;
pub mod simulated;

use serde::{Deserialize, Serialize};

/// Sample formats Parson writes to devices (interleaved, little endian).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleFormat {
    S16,
    /// 24 bits in 3 bytes (`S24_3LE`), common on USB DACs.
    S24Packed,
    /// 32-bit container; 24-bit sources fill the top 24 bits.
    S32,
}

impl SampleFormat {
    pub fn bytes(self) -> usize {
        match self {
            Self::S16 => 2,
            Self::S24Packed => 3,
            Self::S32 => 4,
        }
    }

    /// Significant bits the format carries.
    pub fn bits(self) -> u8 {
        match self {
            Self::S16 => 16,
            Self::S24Packed => 24,
            Self::S32 => 32,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::S16 => "S16_LE",
            Self::S24Packed => "S24_3LE",
            Self::S32 => "S32_LE",
        }
    }
}

/// What a stream is opened with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamFormat {
    pub sample_rate: u32,
    pub format: SampleFormat,
    pub channels: u8,
}

impl StreamFormat {
    pub fn frame_bytes(&self) -> usize {
        self.format.bytes() * usize::from(self.channels)
    }
}

/// A hardware volume control in dB.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HardwareVolume {
    pub min_db: f32,
    pub max_db: f32,
    pub step_db: f32,
}

/// One output as the backend reports it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Stable id, part of the zone id.
    pub id: String,
    /// e.g. `Topping E30 II`.
    pub name: String,
    /// e.g. `USB Audio · hw:2,0`.
    pub description: String,
    /// Sample rates the device accepts natively, ascending.
    pub sample_rates: Vec<u32>,
    pub formats: Vec<SampleFormat>,
    pub max_channels: u8,
    pub hardware_volume: Option<HardwareVolume>,
    /// Another app holds the device right now.
    pub busy: bool,
}

impl DeviceInfo {
    pub fn max_sample_rate(&self) -> u32 {
        self.sample_rates.iter().copied().max().unwrap_or(48_000)
    }

    pub fn max_bits(&self) -> u8 {
        self.formats
            .iter()
            .map(|format| format.bits())
            .max()
            .unwrap_or(16)
    }
}

/// An open playback stream (used from one playback thread only).
pub trait OutputStream: Send {
    fn format(&self) -> StreamFormat;
    /// Writes whole interleaved frames, blocking until the device takes them.
    fn write(&mut self, data: &[u8]) -> Result<(), String>;
    /// Frames written but not yet heard.
    fn delay_frames(&mut self) -> u64;
    /// Pauses in place; `Err` when the device cannot (then the buffer is
    /// dropped and playback restarts from the heard position).
    fn pause(&mut self, paused: bool) -> Result<(), String>;
    /// Discards buffered audio at once (seek, stop).
    fn drop_buffer(&mut self);
    /// Plays out buffered audio.
    fn drain(&mut self);
}

/// An audio API: ALSA on Linux, a Parson Bridge on another machine, or
/// simulated devices for tests and demos.
pub trait DeviceBackend: Send + Sync {
    fn name(&self) -> &'static str;
    fn devices(&self) -> Vec<DeviceInfo>;
    /// Opens `device_id` at exactly `format`. `exclusive` asks for direct
    /// hardware access with no mixing or conversion in between.
    fn open(
        &self,
        device_id: &str,
        format: StreamFormat,
        exclusive: bool,
    ) -> Result<Box<dyn OutputStream>, String>;
    fn set_hardware_volume(&self, device_id: &str, db: f32) -> Result<(), String>;
    fn set_hardware_mute(&self, device_id: &str, muted: bool) -> Result<(), String>;
}

/// The audio API of this machine, or why there is none.
pub fn native_backend() -> Result<Box<dyn DeviceBackend>, String> {
    #[cfg(target_os = "linux")]
    {
        alsa::AlsaBackend::load().map(|backend| Box::new(backend) as Box<dyn DeviceBackend>)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(NATIVE_UNSUPPORTED.to_string())
    }
}

/// What a machine without a supported audio API reports.
pub const NATIVE_UNSUPPORTED: &str = "Bit-perfect output uses ALSA (Linux, including Raspberry Pi OS) in this build. WASAPI exclusive mode (Windows) and CoreAudio hog mode (macOS) are not implemented yet, so this machine lists no devices";
