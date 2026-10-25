//! Audio devices of the machine running the server. In the desktop app the
//! server is the app's own sidecar, so every USB DAC, built-in output or
//! HDMI port of the computer becomes a zone.
//!
//! Playback is bit-perfect when it can be: the file is decoded at its own
//! sample rate and bit depth, the device is opened exclusively at exactly
//! that format, and volume uses the device's own mixer (or is fixed).
//! Anything that changes samples (a rate the device cannot take, software
//! volume, leveling, DSP) is shown in the zone's signal path.

#[cfg(target_os = "linux")]
pub mod alsa;
pub mod engine;
pub mod simulated;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;

use super::renderer::{
    Renderer, RendererInfo, RendererSettings, RendererState, TrackRequest, to_device_scale,
};
use super::{FormatLimits, SourceFormat};
use crate::zones::{DeviceVolumeRange, VolumeMode, ZoneKind};

/// Sample formats Parson writes to devices (interleaved, little endian).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HardwareVolume {
    pub min_db: f32,
    pub max_db: f32,
    pub step_db: f32,
}

/// One output as the backend reports it.
#[derive(Clone, Debug, PartialEq)]
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

    /// What the device plays untouched: any decodable lossless file at a
    /// rate and depth it takes.
    pub fn limits(&self) -> FormatLimits {
        FormatLimits {
            max_sample_rate: self.max_sample_rate(),
            max_bit_depth: self.max_bits().min(32),
            flac: true,
            wav: true,
            alac: true,
            mp3: true,
            aac: true,
        }
    }
}

/// An open playback stream (used from the playback thread only).
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

/// An audio API: ALSA on Linux, or simulated devices for tests and demos.
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

/// The stream format for `source` on `device`, and whether it is exact
/// (no resampling, no lost bits, same channels).
pub fn choose_format(
    device: &DeviceInfo,
    source: &SourceFormat,
    max: FormatLimits,
) -> (StreamFormat, bool) {
    let allowed_rates: Vec<u32> = device
        .sample_rates
        .iter()
        .copied()
        .filter(|rate| *rate <= max.max_sample_rate)
        .collect();
    let rates = if allowed_rates.is_empty() {
        device.sample_rates.clone()
    } else {
        allowed_rates
    };
    let source_rate = source.sample_rate.unwrap_or(44_100);
    let family = |rate: u32| rate.is_multiple_of(11_025);
    let sample_rate = if rates.contains(&source_rate) {
        source_rate
    } else {
        // Prefer the same rate family (no 44.1 -> 48 conversion), highest
        // rate at or below the source, then anything at or below it.
        rates
            .iter()
            .copied()
            .filter(|rate| family(*rate) == family(source_rate) && *rate <= source_rate)
            .max()
            .or_else(|| {
                rates
                    .iter()
                    .copied()
                    .filter(|rate| *rate <= source_rate)
                    .max()
            })
            .or_else(|| rates.iter().copied().min())
            .unwrap_or(48_000)
    };
    let lossless = source.lossless();
    let source_bits = if lossless {
        source.bit_depth.unwrap_or(16)
    } else {
        // Decoded lossy audio has more resolution than 16 bits.
        32
    };
    let usable: Vec<SampleFormat> = device
        .formats
        .iter()
        .copied()
        .filter(|format| format.bits() <= max.max_bit_depth.max(16))
        .collect();
    let formats = if usable.is_empty() {
        device.formats.clone()
    } else {
        usable
    };
    // The smallest container that holds every source bit, else the largest.
    let mut by_size = formats.clone();
    by_size.sort_by_key(|format| format.bits());
    let format = by_size
        .iter()
        .copied()
        .find(|format| format.bits() >= source_bits)
        .or_else(|| by_size.last().copied())
        .unwrap_or(SampleFormat::S16);
    let source_channels = source.channels.unwrap_or(2).max(1);
    let channels = source_channels.min(device.max_channels.max(1));
    let exact = sample_rate == source_rate
        && format.bits() >= source_bits.min(32)
        && channels == source_channels
        && lossless;
    (
        StreamFormat {
            sample_rate,
            format,
            channels,
        },
        exact,
    )
}

/// One device zone.
pub struct DeviceRenderer {
    zone_id: String,
    device: Mutex<DeviceInfo>,
    backend: Arc<dyn DeviceBackend>,
    engine: engine::Engine,
}

impl DeviceRenderer {
    fn device(&self) -> DeviceInfo {
        self.device
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn volume_range(&self, device: &DeviceInfo) -> Option<DeviceVolumeRange> {
        device.hardware_volume.map(|volume| DeviceVolumeRange {
            min: volume.min_db,
            max: volume.max_db,
            step: volume.step_db,
            unit: "dB",
            hardware: true,
            current: self.engine.state().device_volume,
        })
    }
}

fn ready<T: Send + 'static>(value: T) -> BoxFuture<'static, T> {
    Box::pin(std::future::ready(value))
}

impl Renderer for DeviceRenderer {
    fn info(&self) -> RendererInfo {
        let device = self.device();
        let mut volume_modes = Vec::new();
        if device.hardware_volume.is_some() {
            volume_modes.push(VolumeMode::Hardware);
        }
        volume_modes.push(VolumeMode::Software);
        volume_modes.push(VolumeMode::Fixed);
        let rates: Vec<String> = device
            .sample_rates
            .iter()
            .map(|rate| super::format_rate(*rate).replace(" kHz", ""))
            .collect();
        let mut notes = vec![format!(
            "Takes {} kHz at {}.",
            rates.join(", "),
            device
                .formats
                .iter()
                .map(|format| format!("{}-bit", format.bits()))
                .collect::<Vec<_>>()
                .join(" / ")
        )];
        if device.busy {
            notes.push("Another app is using this device; Parson cannot open it exclusively until it lets go.".to_string());
        }
        RendererInfo {
            id: self.zone_id.clone(),
            kind: ZoneKind::Device,
            name: device.name.clone(),
            device: device.description.clone(),
            limits: device.limits(),
            volume_range: self.volume_range(&device),
            volume_modes,
            gapless: true,
            exclusive_capable: true,
            network: false,
            notes,
        }
    }

    fn state(&self) -> RendererState {
        self.engine.state()
    }

    fn configure(&self, settings: RendererSettings) {
        self.engine.send(engine::Control::Configure(settings));
    }

    fn load(
        &self,
        track: TrackRequest,
        next: Option<TrackRequest>,
        position_ms: i64,
        play: bool,
    ) -> BoxFuture<'_, Result<(), String>> {
        self.engine.send(engine::Control::Load {
            track: Box::new(track),
            next: next.map(Box::new),
            position_ms,
            play,
        });
        ready(Ok(()))
    }

    fn set_next(&self, next: Option<TrackRequest>) -> BoxFuture<'_, Result<(), String>> {
        self.engine
            .send(engine::Control::SetNext(next.map(Box::new)));
        ready(Ok(()))
    }

    fn play(&self) -> BoxFuture<'_, Result<(), String>> {
        self.engine.send(engine::Control::Play);
        ready(Ok(()))
    }

    fn pause(&self) -> BoxFuture<'_, Result<(), String>> {
        self.engine.send(engine::Control::Pause);
        ready(Ok(()))
    }

    fn stop(&self) -> BoxFuture<'_, Result<(), String>> {
        self.engine.send(engine::Control::Stop);
        ready(Ok(()))
    }

    fn seek(&self, position_ms: i64) -> BoxFuture<'_, Result<(), String>> {
        self.engine.send(engine::Control::Seek(position_ms));
        ready(Ok(()))
    }

    fn set_volume(&self, level: u8) -> BoxFuture<'_, Result<(), String>> {
        let settings = self.engine.settings();
        let result = match settings.volume_mode {
            VolumeMode::Fixed => Err("fixed_volume".to_string()),
            VolumeMode::Hardware => {
                let device = self.device();
                match self.volume_range(&device) {
                    Some(range) => {
                        let db = to_device_scale(level, &range);
                        self.backend
                            .set_hardware_volume(&device.id, db)
                            .map(|()| self.engine.hardware_volume_set(level, db))
                    }
                    None => Err("no_hardware_volume".to_string()),
                }
            }
            VolumeMode::Software => {
                self.engine.send(engine::Control::Volume(level));
                Ok(())
            }
        };
        ready(result)
    }

    fn set_mute(&self, muted: bool) -> BoxFuture<'_, Result<(), String>> {
        let settings = self.engine.settings();
        let device = self.device();
        let hardware = settings.volume_mode == VolumeMode::Hardware
            && device.hardware_volume.is_some()
            && self.backend.set_hardware_mute(&device.id, muted).is_ok();
        self.engine.send(engine::Control::Mute { muted, hardware });
        ready(Ok(()))
    }

    fn poll(&self) -> BoxFuture<'_, ()> {
        ready(())
    }
}

/// Every local device, kept in step with what the backend reports.
pub struct DeviceManager {
    backend: Option<Arc<dyn DeviceBackend>>,
    players: Mutex<HashMap<String, Arc<DeviceRenderer>>>,
    changed: tokio::sync::broadcast::Sender<()>,
    note: Option<String>,
}

impl DeviceManager {
    /// `PARSON_LOCAL_AUDIO`: `on` uses the system's audio API (ALSA on
    /// Linux), `simulated` two simulated DACs (tests and demos), anything
    /// else (or unset) keeps device zones off.
    pub fn from_env() -> Option<Self> {
        let value = std::env::var("PARSON_LOCAL_AUDIO").ok()?;
        match value.trim().to_ascii_lowercase().as_str() {
            "simulated" | "simulate" => Some(Self::new(
                Arc::new(simulated::SimulatedBackend::with_default_devices(1.0)),
                None,
            )),
            "1" | "on" | "true" | "yes" | "native" => Some(Self::native()),
            _ => None,
        }
    }

    #[cfg(target_os = "linux")]
    fn native() -> Self {
        match alsa::AlsaBackend::load() {
            Ok(backend) => Self::new(Arc::new(backend), None),
            Err(error) => Self {
                backend: None,
                players: Mutex::new(HashMap::new()),
                changed: tokio::sync::broadcast::channel(16).0,
                note: Some(format!("Audio devices are unavailable: {error}.")),
            },
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn native() -> Self {
        Self {
            backend: None,
            players: Mutex::new(HashMap::new()),
            changed: tokio::sync::broadcast::channel(16).0,
            note: Some(
                "Audio-device zones use ALSA (Linux) in this build. WASAPI exclusive mode (Windows) and CoreAudio hog mode (macOS) are not implemented yet."
                    .to_string(),
            ),
        }
    }

    pub fn new(backend: Arc<dyn DeviceBackend>, note: Option<String>) -> Self {
        let manager = Self {
            backend: Some(backend),
            players: Mutex::new(HashMap::new()),
            changed: tokio::sync::broadcast::channel(64).0,
            note,
        };
        manager.refresh();
        manager
    }

    pub fn backend_name(&self) -> Option<&'static str> {
        self.backend.as_ref().map(|backend| backend.name())
    }

    pub fn note(&self) -> Option<String> {
        self.note.clone()
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.changed.subscribe()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<DeviceRenderer>>> {
        self.players
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Picks up plugged-in devices and drops unplugged idle ones.
    pub fn refresh(&self) {
        let Some(backend) = &self.backend else {
            return;
        };
        let devices = backend.devices();
        let mut changed = false;
        {
            let mut players = self.lock();
            for device in &devices {
                let zone_id = format!("device:{}", device.id);
                match players.get(&zone_id) {
                    Some(player) => {
                        let mut current = player
                            .device
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        if *current != *device {
                            *current = device.clone();
                            changed = true;
                        }
                    }
                    None => {
                        let renderer = DeviceRenderer {
                            zone_id: zone_id.clone(),
                            device: Mutex::new(device.clone()),
                            backend: backend.clone(),
                            engine: engine::Engine::start(
                                backend.clone(),
                                device.clone(),
                                self.changed.clone(),
                            ),
                        };
                        players.insert(zone_id, Arc::new(renderer));
                        changed = true;
                    }
                }
            }
            players.retain(|zone_id, player| {
                let present = devices
                    .iter()
                    .any(|device| format!("device:{}", device.id) == *zone_id);
                if !present {
                    changed = true;
                    player.engine.send(engine::Control::Shutdown);
                }
                present
            });
        }
        if changed {
            let _ = self.changed.send(());
        }
    }

    pub fn renderers(&self) -> Vec<Arc<dyn Renderer>> {
        let mut players: Vec<Arc<DeviceRenderer>> = self.lock().values().cloned().collect();
        players.sort_by(|left, right| left.zone_id.cmp(&right.zone_id));
        players
            .into_iter()
            .map(|player| player as Arc<dyn Renderer>)
            .collect()
    }

    pub fn renderer(&self, zone_id: &str) -> Option<Arc<dyn Renderer>> {
        self.lock()
            .get(zone_id)
            .cloned()
            .map(|player| player as Arc<dyn Renderer>)
    }
}

impl Drop for DeviceManager {
    fn drop(&mut self) {
        for player in self.lock().values() {
            player.engine.send(engine::Control::Shutdown);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DeviceInfo, HardwareVolume, SampleFormat, choose_format};
    use crate::outputs::{FormatLimits, SourceFormat};

    fn dac() -> DeviceInfo {
        DeviceInfo {
            id: "dac".into(),
            name: "DAC".into(),
            description: "USB".into(),
            sample_rates: vec![44_100, 48_000, 88_200, 96_000, 176_400, 192_000],
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
        }
    }

    fn flac(rate: u32, bits: u8) -> SourceFormat {
        SourceFormat {
            codec: Some("flac".into()),
            sample_rate: Some(rate),
            bit_depth: Some(bits),
            channels: Some(2),
        }
    }

    #[test]
    fn picks_the_exact_format_when_the_device_takes_it() {
        let device = dac();
        let limits = device.limits();
        let (format, exact) = choose_format(&device, &flac(96_000, 24), limits);
        assert!(exact);
        assert_eq!(format.sample_rate, 96_000);
        assert_eq!(format.format, SampleFormat::S24Packed);
        let (format, exact) = choose_format(&device, &flac(44_100, 16), limits);
        assert!(exact);
        assert_eq!(format.format, SampleFormat::S16);
        // 352.8 kHz goes down within its family, not to 192 kHz.
        let (format, exact) = choose_format(&device, &flac(352_800, 24), limits);
        assert!(!exact);
        assert_eq!(format.sample_rate, 176_400);
        // A user maximum of 96 kHz applies.
        let (format, _) = choose_format(
            &device,
            &flac(192_000, 24),
            limits.capped(Some(96_000), None),
        );
        assert_eq!(format.sample_rate, 96_000);
    }

    #[test]
    fn hdmi_style_outputs_convert_and_say_so() {
        let device = DeviceInfo {
            sample_rates: vec![48_000],
            formats: vec![SampleFormat::S16],
            hardware_volume: None,
            ..dac()
        };
        let (format, exact) = choose_format(&device, &flac(44_100, 24), device.limits());
        assert!(!exact);
        assert_eq!(
            (format.sample_rate, format.format),
            (48_000, SampleFormat::S16)
        );
        let (format, exact) = choose_format(
            &device,
            &SourceFormat {
                codec: Some("mp3".into()),
                sample_rate: Some(48_000),
                bit_depth: None,
                channels: Some(2),
            },
            FormatLimits::SONOS,
        );
        assert!(!exact, "lossy sources are never called bit-perfect");
        assert_eq!(format.sample_rate, 48_000);
    }
}
