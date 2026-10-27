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
#[cfg(test)]
mod engine_tests;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;

pub use parson_output::simulated;
pub use parson_output::{Backend, DeviceInfo, HardwareVolume, SampleFormat, Sink, StreamFormat};

use super::FormatLimits;
use super::renderer::{
    Renderer, RendererInfo, RendererSettings, RendererState, TrackRequest, to_device_scale,
};
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
        lpcm: false,
    }
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
    /// The machine the devices are attached to, for the signal path; this
    /// server when `None`.
    pub machine: Option<String>,
    /// Added to the signal path's output, e.g. `on kitchen-pi via Parson Bridge`.
    pub route: Option<String>,
    /// A step every stream takes (sync corrections of a group).
    pub step: Option<String>,
    /// Facts the zone shows (what was tested, what the protocol allows).
    pub notes: Vec<String>,
    /// Only devices that can be played become zones (a receiver that needs
    /// pairing is listed in the status, not as a zone).
    pub playable_only: bool,
}

impl Placement {
    fn engine_route(&self) -> engine::Route {
        engine::Route {
            host: self.machine.clone(),
            suffix: self.route.clone(),
            step: self.step.clone(),
            modeless: self.kind == ZoneKind::Airplay,
        }
    }

    pub fn local() -> Self {
        Self {
            prefix: "device:".to_string(),
            kind: ZoneKind::Device,
            host: None,
            machine: None,
            route: None,
            step: None,
            notes: Vec::new(),
            playable_only: false,
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
            "CoreAudio hog mode: Mujin Music takes the DAC for itself and switches it to the file's own rate and bit depth (16 and 24-bit arrive unchanged). Tested against a protocol-level fake of the macOS audio HAL, not yet on real hardware."
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
                placement.engine_route(),
                crate::outputs::pipeline::global(),
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
            notes.push("Another app is using this device; Mujin Music cannot open it exclusively until it lets go.".to_string());
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
            endpoint: device.network.clone(),
            notes,
        }
    }

    fn endpoints(&self, output: &parson_output::OutputSettings) -> Vec<crate::zones::ZoneEndpoint> {
        let device = self.device();
        let mut endpoint = crate::zones::ZoneEndpoint::for_device(&self.zone_id, &device, output);
        if let Some(host) = &self.placement.host {
            endpoint.detail = format!("{host} · {}", device.detail);
        }
        vec![endpoint]
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
    /// Decoders and filters its devices play with.
    pipeline: Arc<crate::outputs::pipeline::Pipeline>,
}

impl DeviceManager {
    /// `PARSON_LOCAL_AUDIO`: `off` keeps device zones off, `simulated`
    /// lists two simulated DACs (tests and demos; the older
    /// `PARSON_NATIVE_OUTPUT_SIMULATE=1` too), anything else (or unset) uses
    /// the system's audio API (ALSA on Linux, WASAPI exclusive mode on
    /// Windows, CoreAudio hog mode on macOS).
    pub fn from_env() -> Option<Self> {
        // The environment wins; a background service the desktop app
        // installed reads it from <data>/server.env instead.
        let value = crate::settings::host_setting("PARSON_LOCAL_AUDIO")
            .or_else(|| {
                std::env::var("PARSON_NATIVE_OUTPUT_SIMULATE")
                    .is_ok_and(|value| value == "1")
                    .then(|| "simulated".to_string())
            })
            .unwrap_or_else(|| "on".to_string());
        match value.trim().to_ascii_lowercase().as_str() {
            "simulated" | "simulate" => Some(Self::new(
                Arc::new(simulated::SimulatedBackend::demo()),
                None,
            )),
            "0" | "off" | "false" | "no" | "disabled" => None,
            _ => Some(Self::native()),
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
                pipeline: crate::outputs::pipeline::global(),
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
        Self::with_pipeline(
            backend,
            note,
            placement,
            changed,
            crate::outputs::pipeline::global(),
        )
    }

    /// [`DeviceManager::placed`] with its own decoders and filters.
    pub fn with_pipeline(
        backend: Arc<dyn Backend>,
        note: Option<String>,
        placement: Placement,
        changed: tokio::sync::broadcast::Sender<()>,
        pipeline: Arc<crate::outputs::pipeline::Pipeline>,
    ) -> Self {
        let manager = Self {
            backend: Some(backend),
            players: Mutex::new(HashMap::new()),
            changed,
            note,
            placement,
            pipeline,
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
        let mut devices = backend.devices();
        if self.placement.playable_only {
            devices.retain(|device| device.caps.is_some());
        }
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
                                self.placement.engine_route(),
                                self.pipeline.clone(),
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
    #[test]
    fn device_zones_on_windows_and_mac_say_what_is_untested() {
        for backend in ["WASAPI", "CoreAudio"] {
            let note = super::untested_note(backend).expect("note");
            assert!(note.contains("not yet on real hardware"), "{note}");
        }
        assert!(super::untested_note("ALSA").is_none());
    }
}
