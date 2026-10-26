//! Audio devices of the machine running the server. In the desktop app the
//! server is the app's own sidecar, so every USB DAC, built-in output or
//! HDMI port of the computer becomes a zone.
//!
//! Playback is bit-perfect when it can be: the file is decoded at its own
//! sample rate and bit depth, the device is opened exclusively at exactly
//! that format, and volume uses the device's own mixer (or is fixed).
//! Anything that changes samples (a rate the device cannot take, software
//! volume, leveling, DSP) is shown in the zone's signal path.

pub mod engine;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;

pub use parson_output::simulated;
pub use parson_output::{Backend, DeviceInfo, HardwareVolume, SampleFormat, Sink, StreamFormat};

use super::renderer::{
    Renderer, RendererInfo, RendererSettings, RendererState, TrackRequest, to_device_scale,
};
use super::{FormatLimits, SourceFormat};
use crate::zones::{DeviceVolumeRange, VolumeMode, ZoneKind};

/// What a device plays untouched: any decodable lossless file at a rate
/// and depth it takes.
pub fn device_limits(device: &DeviceInfo) -> FormatLimits {
    FormatLimits {
        max_sample_rate: device.max_sample_rate(),
        max_bit_depth: device.max_bits().min(32),
        flac: true,
        wav: true,
        alac: true,
        mp3: true,
        aac: true,
    }
}

/// The stream format for `source` on `device`, and whether it is exact
/// (no resampling, no lost bits, same channels).
pub fn choose_format(
    device: &DeviceInfo,
    source: &SourceFormat,
    max: FormatLimits,
) -> (StreamFormat, bool) {
    let allowed_rates: Vec<u32> = device
        .sample_rates()
        .iter()
        .copied()
        .filter(|rate| *rate <= max.max_sample_rate)
        .collect();
    let rates = if allowed_rates.is_empty() {
        device.sample_rates().to_vec()
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
    let pcm: Vec<SampleFormat> = device
        .sample_formats()
        .iter()
        .copied()
        .filter(|format| !format.is_dsd())
        .collect();
    let usable: Vec<SampleFormat> = pcm
        .iter()
        .copied()
        .filter(|format| format.bits() <= max.max_bit_depth.max(16))
        .collect();
    let formats = if usable.is_empty() { pcm } else { usable };
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
    let channels = source_channels.min(device.max_channels());
    let exact = sample_rate == source_rate
        && format.bits() >= source_bits.min(32)
        && channels == source_channels
        && lossless;
    (
        StreamFormat {
            sample_rate,
            sample_format: format,
            channels,
        },
        exact,
    )
}

/// Where a manager's devices are: this machine, a Parson Bridge, an
/// AirPlay receiver.
#[derive(Clone, Debug)]
pub struct Placement {
    /// Zone id prefix, e.g. `device:` or `bridge:<bridge id>:`.
    pub prefix: String,
    pub kind: ZoneKind,
    /// Shown before the device description, e.g. `kitchen-pi · Parson Bridge`.
    pub host: Option<String>,
    /// Added to the signal path's output, e.g. `on kitchen-pi via Parson Bridge`.
    pub route: Option<String>,
    /// A step every stream takes (sync corrections of a group).
    pub step: Option<String>,
    /// Facts the zone shows (what was tested, what the protocol allows).
    pub notes: Vec<String>,
}

impl Placement {
    pub fn local() -> Self {
        Self {
            prefix: "device:".to_string(),
            kind: ZoneKind::Device,
            host: None,
            route: None,
            step: None,
            notes: Vec::new(),
        }
    }
}

/// What a device zone says about an audio API that has only been tested
/// against a protocol-level fake.
pub fn untested_note(backend: &str) -> Option<String> {
    match backend {
        "WASAPI" => Some(
            "WASAPI exclusive mode: the DAC runs at the file's own rate and depth with no Windows mixer in between. Tested against a protocol-level fake of Windows' audio stack, not yet on real hardware."
                .to_string(),
        ),
        "CoreAudio" => Some(
            "CoreAudio hog mode: Parson takes the DAC for itself and switches it to the file's own rate and bit depth (16 and 24-bit arrive unchanged). Tested against a protocol-level fake of the macOS audio HAL, not yet on real hardware."
                .to_string(),
        ),
        _ => None,
    }
}

/// One device zone.
pub struct DeviceRenderer {
    zone_id: String,
    device: Mutex<DeviceInfo>,
    backend: Arc<dyn Backend>,
    engine: engine::Engine,
    placement: Placement,
}

impl DeviceRenderer {
    /// A renderer outside any manager (a synced group of bridge devices).
    pub fn standalone(
        zone_id: String,
        device: DeviceInfo,
        backend: Arc<dyn Backend>,
        changed: tokio::sync::broadcast::Sender<()>,
        placement: Placement,
    ) -> Self {
        Self {
            engine: engine::Engine::start(
                backend.clone(),
                device.clone(),
                changed,
                engine::Route {
                    suffix: placement.route.clone(),
                    step: placement.step.clone(),
                    modeless: placement.kind == ZoneKind::Airplay,
                },
            ),
            zone_id,
            device: Mutex::new(device),
            backend,
            placement,
        }
    }

    /// Ends the playback thread.
    pub fn shutdown(&self) {
        self.engine.send(engine::Control::Shutdown);
    }

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
            .sample_rates()
            .iter()
            .map(|rate| super::format_rate(*rate).replace(" kHz", ""))
            .collect();
        let mut notes = vec![format!(
            "Takes {} kHz at {}.",
            rates.join(", "),
            device
                .sample_formats()
                .iter()
                .filter(|format| !format.is_dsd())
                .map(|format| format!("{}-bit", format.bits()))
                .collect::<Vec<_>>()
                .join(" / ")
        )];
        if device.busy {
            notes.push("Another app is using this device; Parson cannot open it exclusively until it lets go.".to_string());
        }
        notes.extend(self.placement.notes.iter().cloned());
        RendererInfo {
            id: self.zone_id.clone(),
            kind: self.placement.kind,
            name: device.name.clone(),
            device: match &self.placement.host {
                Some(host) => format!("{host} · {}", device.detail),
                None => device.detail.clone(),
            },
            limits: device_limits(&device),
            volume_range: self.volume_range(&device),
            volume_modes,
            gapless: true,
            exclusive_capable: self.placement.kind != ZoneKind::Airplay,
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
                            .map_err(|error| error.to_string())
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
    backend: Option<Arc<dyn Backend>>,
    players: Mutex<HashMap<String, Arc<DeviceRenderer>>>,
    changed: tokio::sync::broadcast::Sender<()>,
    note: Option<String>,
    placement: Placement,
}

impl DeviceManager {
    /// `PARSON_LOCAL_AUDIO`: `on` uses the system's audio API (ALSA on
    /// Linux, WASAPI exclusive mode on Windows, CoreAudio hog mode on
    /// macOS), `simulated` two simulated DACs (tests and demos), anything
    /// else (or unset) keeps device zones off.
    pub fn from_env() -> Option<Self> {
        let value = std::env::var("PARSON_LOCAL_AUDIO").ok()?;
        match value.trim().to_ascii_lowercase().as_str() {
            "simulated" | "simulate" => Some(Self::new(
                Arc::new(simulated::SimulatedBackend::demo()),
                None,
            )),
            "1" | "on" | "true" | "yes" | "native" => Some(Self::native()),
            _ => None,
        }
    }

    /// This machine's audio API: ALSA on Linux, WASAPI exclusive mode on
    /// Windows, CoreAudio hog mode on macOS.
    fn native() -> Self {
        match parson_output::platform_backend() {
            Ok(backend) => {
                let backend: Arc<dyn Backend> = Arc::from(backend);
                let notes = untested_note(backend.name()).into_iter().collect();
                Self::placed(
                    backend,
                    None,
                    Placement {
                        notes,
                        ..Placement::local()
                    },
                    tokio::sync::broadcast::channel(64).0,
                )
            }
            Err(error) => Self {
                backend: None,
                players: Mutex::new(HashMap::new()),
                changed: tokio::sync::broadcast::channel(16).0,
                note: Some(format!("Audio devices are unavailable: {error}.")),
                placement: Placement::local(),
            },
        }
    }

    pub fn new(backend: Arc<dyn Backend>, note: Option<String>) -> Self {
        Self::placed(
            backend,
            note,
            Placement::local(),
            tokio::sync::broadcast::channel(64).0,
        )
    }

    /// Devices somewhere else (a bridge, an AirPlay receiver), reporting
    /// changes on `changed`.
    pub fn placed(
        backend: Arc<dyn Backend>,
        note: Option<String>,
        placement: Placement,
        changed: tokio::sync::broadcast::Sender<()>,
    ) -> Self {
        let manager = Self {
            backend: Some(backend),
            players: Mutex::new(HashMap::new()),
            changed,
            note,
            placement,
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
                let zone_id = format!("{}{}", self.placement.prefix, device.id);
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
                                engine::Route {
                                    suffix: self.placement.route.clone(),
                                    step: self.placement.step.clone(),
                                    modeless: self.placement.kind == ZoneKind::Airplay,
                                },
                            ),
                            placement: self.placement.clone(),
                        };
                        players.insert(zone_id, Arc::new(renderer));
                        changed = true;
                    }
                }
            }
            players.retain(|zone_id, player| {
                let present = devices
                    .iter()
                    .any(|device| format!("{}{}", self.placement.prefix, device.id) == *zone_id);
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

impl DeviceManager {
    /// Stops every engine (the devices went away).
    pub fn shutdown(&self) {
        for player in self.lock().drain().map(|(_, player)| player) {
            player.engine.send(engine::Control::Shutdown);
        }
        let _ = self.changed.send(());
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
    use parson_output::DeviceCaps;
    use parson_output::simulated::simulated_device;

    use super::{DeviceInfo, HardwareVolume, SampleFormat, choose_format, device_limits};
    use crate::outputs::{FormatLimits, SourceFormat};

    fn caps(rates: &[u32], formats: &[SampleFormat]) -> DeviceCaps {
        DeviceCaps {
            sample_rates: rates.to_vec(),
            sample_formats: formats.to_vec(),
            min_channels: 1,
            max_channels: 2,
            dsd_formats: Vec::new(),
            dsd_rates: Vec::new(),
        }
    }

    fn dac() -> DeviceInfo {
        simulated_device(
            "dac",
            "DAC",
            "USB",
            caps(
                &[44_100, 48_000, 88_200, 96_000, 176_400, 192_000],
                &[
                    SampleFormat::S16,
                    SampleFormat::S24Packed,
                    SampleFormat::S32,
                ],
            ),
            Some(HardwareVolume {
                min_db: -127.5,
                max_db: 0.0,
                step_db: 0.5,
            }),
        )
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
    fn device_zones_on_windows_and_mac_say_what_is_untested() {
        for backend in ["WASAPI", "CoreAudio"] {
            let note = super::untested_note(backend).expect("note");
            assert!(note.contains("not yet on real hardware"), "{note}");
        }
        assert!(super::untested_note("ALSA").is_none());
    }

    #[test]
    fn picks_the_exact_format_when_the_device_takes_it() {
        let device = dac();
        let limits = device_limits(&device);
        let (format, exact) = choose_format(&device, &flac(96_000, 24), limits);
        assert!(exact);
        assert_eq!(format.sample_rate, 96_000);
        assert_eq!(format.sample_format, SampleFormat::S24Packed);
        let (format, exact) = choose_format(&device, &flac(44_100, 16), limits);
        assert!(exact);
        assert_eq!(format.sample_format, SampleFormat::S16);
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
            caps: Some(caps(&[48_000], &[SampleFormat::S16])),
            hardware_volume: None,
            ..dac()
        };
        let (format, exact) = choose_format(&device, &flac(44_100, 24), device_limits(&device));
        assert!(!exact);
        assert_eq!(
            (format.sample_rate, format.sample_format),
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
