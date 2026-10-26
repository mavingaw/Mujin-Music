//! The server's own audio outputs.
//!
//! Every other Parson output hands audio to a browser or phone, which plays
//! it through the operating system's shared mixer. This module lets the
//! server play a track itself on a device attached to the machine it runs
//! on, opened exclusively through `parson-output` (ALSA `hw:`, WASAPI
//! Exclusive, CoreAudio hog mode). That is the only way Parson can promise
//! a bit-perfect path, and it is how the desktop app (whose sidecar is this
//! server) plays to a USB DAC.
//!
//! The server keeps one native playback session at a time. Clients start a
//! track on a device, then poll the session for position and state and
//! send transport commands; they advance the queue themselves when a track
//! ends.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};

use parson_output::dsd::{DsdPacking, DsdPcm, DsdReader, DsdSource, plan_dsd};
use parson_output::dsp::{
    DspChain, DspPcm, DspSettings, DspStats, ProcessingPlan, ResamplerKind, plan_processing,
    selectable_rates,
};
use parson_output::player::{InputFormat, PcmOpener, PcmSource, Player, PlayerState, PlayerStatus};
use parson_output::procfs::{HardwareInfo, HardwareStatus};
use parson_output::{
    Backend, DepthChange, DeviceInfo, Driver, DsdMode, FormatSupport, OutputPlan, OutputSettings,
    PlanError, SampleFormat, SourceFormat, StreamFormat, format_support, plan_output,
};
use serde::Serialize;

use crate::domain::{AudioProperties, Song};
use crate::signal_path::{
    NativeLeveling, NativeOutputTarget, ServerSignalPath, describe_native_leg,
};

pub const SIMULATE_ENV: &str = "PARSON_NATIVE_OUTPUT_SIMULATE";

/// A device as the API presents it.
#[derive(Serialize, Clone, Debug)]
pub struct DeviceView {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub driver: Driver,
    /// e.g. `WASAPI Exclusive Mode`, following the exclusive setting.
    pub mode_label: String,
    /// How the driver addresses it, e.g. `hw:1,0` or an ALSA PCM name.
    pub address: Option<String>,
    pub exclusive_capable: bool,
    pub is_default: bool,
    pub probe_error: Option<String>,
    pub settings: OutputSettings,
    /// Every standard rate and depth, supported or not; `None` when the
    /// device could not be probed.
    pub formats: Option<FormatSupport>,
    /// Sample containers the device accepts, e.g. `24-bit packed`.
    pub sample_formats: Vec<&'static str>,
    pub channels: Option<(u8, u8)>,
    /// The native session is playing on this device.
    pub in_use: bool,
    /// The device's DSP chain, run by the server before the device.
    pub dsp: DspSettings,
    /// The chain was saved for this device (rather than the defaults).
    pub dsp_customized: bool,
    pub dsp_support: DspSupport,
    /// Native DSD containers the driver accepts, e.g. `DSD_U32_BE`.
    pub dsd_formats: Vec<&'static str>,
    /// What the kernel reports about the hardware: USB product name and
    /// id, and the formats its descriptors declare.
    pub hardware: Option<HardwareInfo>,
}

/// What the server-side DSP chain can do on a device.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct DspSupport {
    /// The chain can run: the device's formats are known.
    pub available: bool,
    /// Why it cannot, when it cannot.
    pub reason: Option<String>,
    /// e.g. `Parson DSP, 64-bit float`.
    pub engine: &'static str,
    /// The device opens in stereo, which crossfeed needs.
    pub crossfeed: bool,
    /// Fixed DSP sample rates the device accepts as configured.
    pub sample_rates: Vec<u32>,
    /// Depth the chain dithers to.
    pub output_bits: Option<u8>,
}

#[derive(Serialize, Clone, Debug)]
pub struct DevicesView {
    /// The platform has a native backend (or the simulated one is on).
    pub supported: bool,
    /// The platform's driver, e.g. `wasapi`.
    pub driver: Option<Driver>,
    pub simulated: bool,
    pub devices: Vec<DeviceView>,
}

#[derive(Serialize, Clone, Debug)]
pub struct SessionView {
    pub device_id: String,
    pub device_name: String,
    pub song_id: String,
    pub user_id: i32,
    pub duration_ms: u64,
    #[serde(flatten)]
    pub status: PlayerStatus,
    pub plan: OutputPlan,
    /// Volume leveling the server applies, if any.
    pub leveling: Option<LevelingRequest>,
    /// The server-side DSP chain, when anything processes.
    pub processing: Option<ProcessingPlan>,
    pub signal_path: ServerSignalPath,
    /// What the operating system says the hardware is running at, read
    /// while it plays (Linux: the PCM's `hw_params`).
    pub hardware: Option<HardwareStatus>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum NativeError {
    UnknownDevice,
    Plan(PlanError),
    /// The track has no known sample rate (rescan the library).
    UnknownFormat,
    NoSession,
    Device(String),
}

impl std::fmt::Display for NativeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownDevice => f.write_str("That output device is not connected."),
            Self::Plan(error) => write!(f, "This track cannot be played on the device: {error}."),
            Self::UnknownFormat => f.write_str(
                "Parson does not know this track's sample rate yet. Rescan the library, then try again.",
            ),
            Self::NoSession => f.write_str("Nothing is playing on a server output."),
            Self::Device(message) => f.write_str(message),
        }
    }
}

impl NativeError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnknownDevice => "native_output_device_not_found",
            Self::Plan(_) => "native_output_format_unsupported",
            Self::UnknownFormat => "native_output_format_unknown",
            Self::NoSession => "native_output_no_session",
            Self::Device(_) => "native_output_device_error",
        }
    }
}

struct Session {
    device_id: String,
    device_name: String,
    song: Song,
    user_id: i32,
    duration_ms: u64,
    plan: OutputPlan,
    leveling: Option<LevelingRequest>,
    processing: Option<ProcessingPlan>,
    stats: Arc<DspStats>,
    signal_path: ServerSignalPath,
    player: Player,
    backend: Arc<dyn Backend>,
    stream: StreamFormat,
}

impl Session {
    fn view(&self) -> SessionView {
        let mut signal_path = self.signal_path.clone();
        if let Some(processing) = &mut signal_path.processing {
            processing.status = Some(self.stats.snapshot());
        }
        let status = self.player.status();
        let hardware = matches!(status.state, PlayerState::Playing | PlayerState::Starting)
            .then(|| self.backend.hardware_status(&self.device_id, &self.stream))
            .flatten();
        if let Some(hardware) = &hardware {
            crate::signal_path::add_hardware_report(
                &mut signal_path,
                &self.device_name,
                hardware,
                &self.stream,
            );
        }
        SessionView {
            device_id: self.device_id.clone(),
            device_name: self.device_name.clone(),
            song_id: self.song.id.clone(),
            user_id: self.user_id,
            duration_ms: self.duration_ms,
            status,
            plan: self.plan.clone(),
            leveling: self.leveling.clone(),
            processing: self.processing.clone(),
            signal_path,
            hardware,
        }
    }
}

#[derive(Default)]
struct State {
    devices: Vec<DeviceInfo>,
    probed: bool,
    settings: HashMap<String, OutputSettings>,
    settings_loaded: bool,
    dsp: HashMap<String, DspSettings>,
}

/// How decoded PCM is produced for a track; ffmpeg in production.
pub type DecoderFactory = Arc<
    dyn Fn(&Path, &OutputPlan, &SourceFormat, Option<f64>, u64) -> io::Result<PcmSource>
        + Send
        + Sync,
>;

/// What a float decoder produces: interleaved `f64le` at `sample_rate`
/// with `channels` channels. `resample_to` asks the decoder itself to
/// convert (for ratios the chain's polyphase table cannot hold).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FloatDecode {
    pub channels: u8,
    pub sample_rate: u32,
    pub resample_to: Option<u32>,
}

/// Decodes a track to 64-bit float for the DSP chain; ffmpeg in
/// production.
pub type FloatDecoderFactory =
    Arc<dyn Fn(&Path, &FloatDecode, u64) -> io::Result<PcmSource> + Send + Sync>;

/// Volume leveling a client asks the server to apply.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct LevelingRequest {
    pub gain_db: f64,
    /// `track` or `album`.
    pub source: String,
}

impl LevelingRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.gain_db.is_finite() || self.gain_db.abs() > 24.0 {
            return Err("Leveling gain must be between -24 and +24 dB.");
        }
        if !matches!(self.source.as_str(), "track" | "album") {
            return Err("Leveling source must be track or album.");
        }
        Ok(())
    }
}

pub struct NativeOutputs {
    backends: Vec<Arc<dyn Backend>>,
    simulated: bool,
    settings_path: Option<PathBuf>,
    decoder: DecoderFactory,
    float_decoder: FloatDecoderFactory,
    resampler: &'static str,
    state: Mutex<State>,
    session: Mutex<Option<Session>>,
}

/// This machine's host name, for the transport stage ("studio-pc ·
/// Parson audio transport"); `This server` when it cannot be read.
pub fn host_name() -> &'static str {
    static HOST: OnceLock<String> = OnceLock::new();
    HOST.get_or_init(|| {
        let mut buffer = [0u8; 256];
        // SAFETY: the buffer is writable for its whole length, and
        // gethostname NUL-terminates within it on success.
        #[cfg(target_os = "linux")]
        let name = (unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) } == 0)
            .then(|| {
                let end = buffer
                    .iter()
                    .position(|byte| *byte == 0)
                    .unwrap_or(buffer.len());
                String::from_utf8_lossy(&buffer[..end]).trim().to_string()
            });
        #[cfg(not(target_os = "linux"))]
        let name = {
            let _ = &mut buffer;
            std::env::var("COMPUTERNAME")
                .or_else(|_| std::env::var("HOSTNAME"))
                .ok()
        };
        name.filter(|name| !name.is_empty())
            .unwrap_or_else(|| "This server".to_string())
    })
}

/// The process-wide outputs, with the platform backend and, when
/// `PARSON_NATIVE_OUTPUT_SIMULATE=1`, the simulated device.
pub fn global() -> Arc<NativeOutputs> {
    static OUTPUTS: OnceLock<Arc<NativeOutputs>> = OnceLock::new();
    Arc::clone(OUTPUTS.get_or_init(|| {
        let simulated = std::env::var(SIMULATE_ENV).is_ok_and(|value| value == "1");
        let mut backends: Vec<Arc<dyn Backend>> = Vec::new();
        if let Some(backend) = parson_output::platform_backend() {
            backends.push(Arc::from(backend));
        }
        if simulated {
            backends.push(Arc::new(parson_output::simulated::SimulatedBackend));
        }
        Arc::new(NativeOutputs::new(
            backends,
            simulated,
            Some(crate::settings::data_path(&["native-outputs.json"])),
            Arc::new(ffmpeg_decoder),
            if ffmpeg_has_soxr() {
                "soxr, very high quality"
            } else {
                "ffmpeg resampler, 64-tap sinc"
            },
        ))
    }))
}

impl NativeOutputs {
    pub fn new(
        backends: Vec<Arc<dyn Backend>>,
        simulated: bool,
        settings_path: Option<PathBuf>,
        decoder: DecoderFactory,
        resampler: &'static str,
    ) -> Self {
        Self {
            backends,
            simulated,
            settings_path,
            decoder,
            float_decoder: Arc::new(ffmpeg_float_decoder),
            resampler,
            state: Mutex::new(State::default()),
            session: Mutex::new(None),
        }
    }

    /// Replaces the float decoder the DSP chain reads from.
    pub fn with_float_decoder(mut self, decoder: FloatDecoderFactory) -> Self {
        self.float_decoder = decoder;
        self
    }

    fn dsp_path(&self) -> Option<PathBuf> {
        self.settings_path
            .as_ref()
            .map(|path| path.with_file_name("native-dsp.json"))
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn session(&self) -> std::sync::MutexGuard<'_, Option<Session>> {
        self.session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn load_settings(&self, state: &mut State) {
        if state.settings_loaded {
            return;
        }
        state.settings_loaded = true;
        if let Some(path) = &self.settings_path
            && let Ok(text) = std::fs::read_to_string(path)
            && let Ok(settings) = serde_json::from_str::<HashMap<String, OutputSettings>>(&text)
        {
            state.settings = settings
                .into_iter()
                .filter(|(_, settings)| settings.validate().is_ok())
                .collect();
        }
        if let Some(path) = self.dsp_path()
            && let Ok(text) = std::fs::read_to_string(path)
            && let Ok(chains) = serde_json::from_str::<HashMap<String, DspSettings>>(&text)
        {
            state.dsp = chains
                .into_iter()
                .take(64)
                .map(|(id, settings)| (id, settings.normalized()))
                .collect();
        }
    }

    fn save_json<T: Serialize>(path: Option<PathBuf>, value: &T) -> io::Result<()> {
        let Some(path) = path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, serde_json::to_vec_pretty(value)?)?;
        std::fs::rename(temp, path)
    }

    fn save_settings(&self, settings: &HashMap<String, OutputSettings>) -> io::Result<()> {
        Self::save_json(self.settings_path.clone(), settings)
    }

    /// Lists devices, probing them the first time or when `refresh`. The
    /// device the session holds is not re-probed (it is busy); its last
    /// result is kept.
    pub fn devices(&self, refresh: bool) -> DevicesView {
        let busy = self
            .session()
            .as_ref()
            .map(|session| session.device_id.clone());
        let mut state = self.state();
        self.load_settings(&mut state);
        if refresh || !state.probed {
            let previous = std::mem::take(&mut state.devices);
            let mut devices = Vec::new();
            for backend in &self.backends {
                for device in backend.devices() {
                    if busy.as_deref() == Some(device.id.as_str())
                        && let Some(kept) = previous.iter().find(|old| old.id == device.id)
                    {
                        devices.push(kept.clone());
                    } else {
                        devices.push(device);
                    }
                }
            }
            state.devices = devices;
            state.probed = true;
        }
        let devices = state
            .devices
            .iter()
            .map(|device| {
                let settings = state.settings.get(&device.id).cloned().unwrap_or_default();
                view(
                    device,
                    settings,
                    state.dsp.get(&device.id),
                    busy.as_deref() == Some(device.id.as_str()),
                )
            })
            .collect();
        DevicesView {
            supported: !self.backends.is_empty(),
            driver: self
                .backends
                .iter()
                .map(|backend| backend.driver())
                .find(|driver| *driver != Driver::Simulated),
            simulated: self.simulated,
            devices,
        }
    }

    pub fn set_settings(
        &self,
        device_id: &str,
        settings: OutputSettings,
    ) -> Result<DeviceView, String> {
        settings.validate()?;
        self.devices(false);
        let busy = self
            .session()
            .as_ref()
            .is_some_and(|session| session.device_id == device_id);
        let mut state = self.state();
        let Some(device) = state
            .devices
            .iter()
            .find(|device| device.id == device_id)
            .cloned()
        else {
            return Err("That output device is not connected.".into());
        };
        let mut next = state.settings.clone();
        next.insert(device_id.to_string(), settings.clone());
        self.save_settings(&next)
            .map_err(|error| format!("Could not save the device settings: {error}"))?;
        state.settings = next;
        let dsp = state.dsp.get(device_id).cloned();
        drop(state);
        self.restart_on(device_id);
        Ok(view(&device, settings, dsp.as_ref(), busy))
    }

    /// Saves a device's DSP chain. A track playing on the device restarts
    /// at its position so the new chain is heard at once.
    pub fn set_dsp(&self, device_id: &str, settings: DspSettings) -> Result<DeviceView, String> {
        let settings = settings.normalized();
        self.devices(false);
        let busy = self
            .session()
            .as_ref()
            .is_some_and(|session| session.device_id == device_id);
        let mut state = self.state();
        let Some(device) = state
            .devices
            .iter()
            .find(|device| device.id == device_id)
            .cloned()
        else {
            return Err("That output device is not connected.".into());
        };
        let mut next = state.dsp.clone();
        next.insert(device_id.to_string(), settings.clone());
        Self::save_json(self.dsp_path(), &next)
            .map_err(|error| format!("Could not save the DSP settings: {error}"))?;
        state.dsp = next;
        let output = state.settings.get(device_id).cloned().unwrap_or_default();
        drop(state);
        self.restart_on(device_id);
        Ok(view(&device, output, Some(&settings), busy))
    }

    /// Replays the session's track at its position, keeping pause, so new
    /// settings apply to what is playing.
    fn restart_on(&self, device_id: &str) {
        let restart = self
            .session()
            .as_ref()
            .filter(|session| session.device_id == device_id)
            .map(|session| {
                (
                    session.user_id,
                    session.song.clone(),
                    session.player.status(),
                    session.leveling.clone(),
                )
            });
        let Some((user_id, song, status, leveling)) = restart else {
            return;
        };
        if !matches!(
            status.state,
            PlayerState::Starting | PlayerState::Playing | PlayerState::Paused
        ) {
            return;
        }
        if self
            .play(user_id, device_id, &song, status.position_ms, leveling)
            .is_ok()
            && status.state == PlayerState::Paused
        {
            let _ = self.pause();
        }
    }

    fn device(
        &self,
        device_id: &str,
    ) -> Result<(DeviceInfo, OutputSettings, DspSettings, Arc<dyn Backend>), NativeError> {
        self.devices(false);
        let state = self.state();
        let device = state
            .devices
            .iter()
            .find(|device| device.id == device_id)
            .cloned()
            .ok_or(NativeError::UnknownDevice)?;
        let backend = self
            .backends
            .iter()
            .find(|backend| backend.driver() == device.driver)
            .cloned()
            .ok_or(NativeError::UnknownDevice)?;
        let settings = state.settings.get(device_id).cloned().unwrap_or_default();
        let dsp = state.dsp.get(device_id).cloned().unwrap_or_default();
        Ok((device, settings, dsp, backend))
    }

    /// How `song` would play on `device_id`, without playing it.
    pub fn describe(
        &self,
        device_id: &str,
        song: &Song,
    ) -> Result<(OutputPlan, ServerSignalPath), NativeError> {
        let (device, settings, dsp, _) = self.device(device_id)?;
        self.plan_and_path(&device, &settings, &dsp, song, None)
            .map(|(plan, _, path, _)| (plan, path))
    }

    fn plan_and_path(
        &self,
        device: &DeviceInfo,
        settings: &OutputSettings,
        dsp: &DspSettings,
        song: &Song,
        leveling: Option<&LevelingRequest>,
    ) -> Result<
        (
            OutputPlan,
            Option<ProcessingPlan>,
            ServerSignalPath,
            SourceFormat,
        ),
        NativeError,
    > {
        let (mut plan, source) = match dsd_source(song.audio.as_ref()) {
            Some(dsd_source) => {
                // DSP and leveling run on PCM, so they convert DSD first.
                let force_pcm = (dsp.processes() || leveling.is_some())
                    .then_some("the DSP engine or volume leveling is on, and both work on PCM");
                let plan = plan_dsd(dsd_source, device.caps.as_ref(), settings, force_pcm)
                    .map_err(NativeError::Plan)?;
                (plan, parson_output::dsd::pcm_source(dsd_source))
            }
            None => {
                let source =
                    source_format(song.audio.as_ref()).ok_or(NativeError::UnknownFormat)?;
                (
                    plan_output(source, device.caps.as_ref(), settings)
                        .map_err(NativeError::Plan)?,
                    source,
                )
            }
        };
        let sent_as_dsd = plan
            .dsd
            .as_ref()
            .is_some_and(|dsd| dsd.mode != DsdMode::Pcm);
        // Processing the listener asked for changes the samples on purpose;
        // it then runs in the server's own chain, before the device.
        let processing = device
            .caps
            .as_ref()
            .filter(|_| !sent_as_dsd)
            .and_then(|caps| {
                plan_processing(
                    &mut plan,
                    &source,
                    dsp,
                    leveling.map(|leveling| leveling.gain_db),
                    caps,
                    settings,
                )
            });
        let extension = Path::new(&song.path)
            .extension()
            .and_then(|extension| extension.to_str());
        let mode_label = device.mode_label(settings.exclusive);
        let path = describe_native_leg(
            &song.id,
            song.audio.as_ref(),
            extension,
            &plan,
            &NativeOutputTarget {
                device_name: &device.name,
                mode_label: &mode_label,
                host: host_name(),
                config_pcm: device.is_alsa_config_pcm(),
                usb_id: device
                    .hardware
                    .as_ref()
                    .and_then(|hardware| hardware.usb_id.as_deref()),
                usb_speed: device
                    .hardware
                    .as_ref()
                    .and_then(|hardware| hardware.speed.as_deref()),
                simulated: device.driver == Driver::Simulated,
                kernel_emulator: device
                    .hardware
                    .as_ref()
                    .is_some_and(|hardware| hardware.kernel_emulator),
                resampler: self.resampler,
                leveling: leveling.map(|leveling| NativeLeveling {
                    gain_db: leveling.gain_db,
                    source: &leveling.source,
                }),
                processing: processing.as_ref(),
            },
        );
        Ok((plan, processing, path, source))
    }

    /// Plays `song` on `device_id` from `position_ms`, replacing any
    /// current session. Blocks while the device opens.
    pub fn play(
        &self,
        user_id: i32,
        device_id: &str,
        song: &Song,
        position_ms: u64,
        leveling: Option<LevelingRequest>,
    ) -> Result<SessionView, NativeError> {
        let (device, settings, dsp, backend) = self.device(device_id)?;
        let (plan, processing, signal_path, source) =
            self.plan_and_path(&device, &settings, &dsp, song, leveling.as_ref())?;
        let mut session = self.session();
        // Release the device (and any other) before opening it again.
        if let Some(previous) = session.take() {
            previous.player.stop();
        }
        let stream = StreamFormat {
            sample_rate: plan.sample_rate,
            sample_format: plan.sample_format,
            channels: plan.channels,
        };
        let exclusive = settings.exclusive;
        let open_id = device.id.clone();
        let song_path = PathBuf::from(&song.path);
        let stats = Arc::new(DspStats::default());
        let dsd_packing = plan.dsd.as_ref().and_then(|dsd| match dsd.mode {
            DsdMode::Native => Some(DsdPacking::Native(plan.sample_format)),
            DsdMode::Dop => Some(DsdPacking::Dop(plan.sample_format)),
            DsdMode::Pcm => None,
        });
        let opener: PcmOpener = match (dsd_packing, processing.clone()) {
            (Some(packing), _) => {
                let path = song_path.clone();
                Box::new(move |position| open_dsd(&path, packing, position))
            }
            (None, None) => {
                let decoder = Arc::clone(&self.decoder);
                let decode_plan = plan.clone();
                Box::new(move |position| decoder(&song_path, &decode_plan, &source, None, position))
            }
            (None, Some(processing)) => {
                let decoder = Arc::clone(&self.float_decoder);
                let resample_to = (processing.resampler == Some(ResamplerKind::Decoder))
                    .then_some(processing.output_rate);
                let spec = FloatDecode {
                    channels: plan.channels,
                    sample_rate: source.sample_rate,
                    resample_to,
                };
                let input_rate = resample_to.unwrap_or(source.sample_rate);
                let output = InputFormat::for_container(plan.sample_format);
                let chain_stats = Arc::clone(&stats);
                Box::new(move |position| {
                    let decoded = decoder(&song_path, &spec, position)?;
                    let chain =
                        DspChain::new(&processing, input_rate, output, Arc::clone(&chain_stats));
                    Ok(Box::new(DspPcm::new(decoded, chain)) as PcmSource)
                })
            }
        };
        let open_backend = Arc::clone(&backend);
        let player = Player::start(
            Box::new(move || open_backend.open(&open_id, stream, exclusive)),
            opener,
            stream,
            if dsd_packing.is_some() {
                InputFormat::Raw
            } else {
                InputFormat::for_container(plan.sample_format)
            },
            position_ms,
        )
        .map_err(|error| NativeError::Device(format!("{}: {error}", device.name)))?;
        let started = Session {
            device_id: device.id.clone(),
            device_name: device.name.clone(),
            song: song.clone(),
            user_id,
            duration_ms: (song.duration.max(0.0) * 1000.0) as u64,
            plan,
            leveling,
            processing,
            stats,
            signal_path,
            player,
            backend,
            stream,
        };
        let view = started.view();
        *session = Some(started);
        Ok(view)
    }

    pub fn current(&self) -> Option<SessionView> {
        self.session().as_ref().map(Session::view)
    }

    pub fn pause(&self) -> Result<SessionView, NativeError> {
        self.with_session(|session| session.player.pause())
    }

    pub fn resume(&self) -> Result<SessionView, NativeError> {
        self.with_session(|session| session.player.resume())
    }

    pub fn seek(&self, position_ms: u64) -> Result<SessionView, NativeError> {
        self.with_session(|session| session.player.seek(position_ms))
    }

    /// Stops playback and releases the device.
    pub fn stop(&self) -> Result<(), NativeError> {
        let session = self.session().take().ok_or(NativeError::NoSession)?;
        session.player.stop();
        Ok(())
    }

    fn with_session(&self, action: impl FnOnce(&Session)) -> Result<SessionView, NativeError> {
        let session = self.session();
        let session = session.as_ref().ok_or(NativeError::NoSession)?;
        action(session);
        Ok(session.view())
    }
}

fn dsp_support(device: &DeviceInfo, settings: &OutputSettings) -> DspSupport {
    let Some(caps) = device.caps.as_ref() else {
        return DspSupport {
            available: false,
            reason: Some(match &device.probe_error {
                Some(error) => format!(
                    "Parson could not read the formats {} accepts ({error}), so it cannot run the DSP engine for it.",
                    device.name
                ),
                None => format!(
                    "Parson could not read the formats {} accepts, so it cannot run the DSP engine for it.",
                    device.name
                ),
            }),
            engine: crate::signal_path::SERVER_DSP_ENGINE,
            crossfeed: false,
            sample_rates: Vec::new(),
            output_bits: None,
        };
    };
    DspSupport {
        available: true,
        reason: None,
        engine: crate::signal_path::SERVER_DSP_ENGINE,
        crossfeed: caps.min_channels <= 2 && caps.max_channels >= 2,
        sample_rates: selectable_rates(caps, settings),
        output_bits: caps
            .bit_depths()
            .into_iter()
            .filter(|bits| settings.max_bit_depth.is_none_or(|max| *bits <= max))
            .max(),
    }
}

fn view(
    device: &DeviceInfo,
    settings: OutputSettings,
    dsp: Option<&DspSettings>,
    in_use: bool,
) -> DeviceView {
    DeviceView {
        dsp: dsp.cloned().unwrap_or_default(),
        dsp_customized: dsp.is_some(),
        dsp_support: dsp_support(device, &settings),
        id: device.id.clone(),
        name: device.name.clone(),
        detail: device.detail.clone(),
        driver: device.driver,
        mode_label: device.mode_label(settings.exclusive),
        address: device.address.clone(),
        exclusive_capable: device.exclusive_capable,
        is_default: device.is_default,
        probe_error: device.probe_error.clone(),
        formats: device
            .caps
            .as_ref()
            .map(|caps| format_support(caps, &settings)),
        sample_formats: device
            .caps
            .as_ref()
            .map(|caps| {
                caps.sample_formats
                    .iter()
                    .map(|format| format.label())
                    .collect()
            })
            .unwrap_or_default(),
        channels: device
            .caps
            .as_ref()
            .map(|caps| (caps.min_channels, caps.max_channels)),
        dsd_formats: device
            .caps
            .as_ref()
            .map(|caps| {
                caps.dsd_formats
                    .iter()
                    .map(|format| format.label())
                    .collect()
            })
            .unwrap_or_default(),
        hardware: device.hardware.clone(),
        settings,
        in_use,
    }
}

/// A DSD track's rate and channels, from its indexed properties.
pub fn dsd_source(audio: Option<&AudioProperties>) -> Option<DsdSource> {
    let audio = audio?;
    if audio.codec.as_deref() != Some("dsd") {
        return None;
    }
    Some(DsdSource {
        rate: audio.sample_rate.filter(|rate| *rate > 0)?,
        channels: audio.channels.filter(|channels| *channels > 0).unwrap_or(2),
    })
}

/// Opens a DSD file at `position_ms`, packed for the device.
fn open_dsd(path: &Path, packing: DsdPacking, position_ms: u64) -> io::Result<PcmSource> {
    let mut file = io::BufReader::new(std::fs::File::open(path)?);
    let info = parson_output::dsd::read_info(&mut file)?;
    let per_frame = packing.bytes_per_frame() as u64;
    let start = info.byte_at_ms(position_ms) / per_frame * per_frame;
    Ok(Box::new(DsdPcm::new(
        DsdReader::new(file, info, start),
        info.channels,
        packing,
    )))
}

/// The decoder's view of a track; `None` without a known sample rate.
pub fn source_format(audio: Option<&AudioProperties>) -> Option<SourceFormat> {
    let audio = audio?;
    let lossless = audio.lossless == Some(true);
    Some(SourceFormat {
        sample_rate: audio.sample_rate.filter(|rate| *rate > 0)?,
        bit_depth: if lossless {
            audio.bit_depth.filter(|bits| (8..=32).contains(bits))
        } else {
            None
        },
        channels: audio.channels.filter(|channels| *channels > 0).unwrap_or(2),
        lossless,
    })
}

/// The `-af` filter for a plan, or `None` when the samples pass through
/// untouched (format widening is an exact shift ffmpeg does by itself).
pub fn decode_filter(plan: &OutputPlan, gain_db: Option<f64>, soxr: bool) -> Option<String> {
    let gain = gain_db.filter(|gain| gain.is_finite() && *gain != 0.0);
    let dither = plan.sample_format == SampleFormat::S16
        && (gain.is_some()
            || matches!(
                plan.depth,
                DepthChange::Reduce { .. } | DepthChange::Quantize { .. }
            ));
    if plan.resample.is_none() && !dither && gain.is_none() {
        return None;
    }
    let mut options = vec![format!("osr={}", plan.sample_rate)];
    if plan.resample.is_some() {
        if soxr {
            options.push("resampler=soxr".into());
            options.push("precision=28".into());
        } else {
            options.push("filter_size=64".into());
            options.push("phase_shift=10".into());
            options.push("cutoff=0.97".into());
        }
    }
    if dither || (plan.resample.is_some() && plan.sample_format == SampleFormat::S16) {
        options.push("dither_method=triangular".into());
    }
    let resample = format!("aresample={}", options.join(":"));
    Some(match gain {
        Some(gain) => format!("volume={gain:.2}dB:precision=double,{resample}"),
        None => resample,
    })
}

/// ffmpeg arguments that decode `path` from `start_ms` to raw PCM for
/// `plan` on stdout.
pub fn decode_args(
    path: &Path,
    plan: &OutputPlan,
    gain_db: Option<f64>,
    start_ms: u64,
    soxr: bool,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["-hide_banner", "-nostdin", "-v", "error"]
        .into_iter()
        .map(OsString::from)
        .collect();
    if start_ms > 0 {
        args.push("-ss".into());
        args.push(format!("{}.{:03}", start_ms / 1000, start_ms % 1000).into());
    }
    args.push("-i".into());
    args.push(path.as_os_str().to_owned());
    for arg in ["-map", "0:a:0", "-vn", "-sn", "-dn"] {
        args.push(arg.into());
    }
    if let Some(filter) = decode_filter(plan, gain_db, soxr) {
        args.push("-af".into());
        args.push(filter.into());
    }
    let (codec, format) = match InputFormat::for_container(plan.sample_format) {
        InputFormat::S16 => ("pcm_s16le", "s16le"),
        // DSD is read by the server itself, never through ffmpeg.
        InputFormat::S32 | InputFormat::Raw => ("pcm_s32le", "s32le"),
    };
    for arg in [
        "-ac".to_string(),
        plan.channels.to_string(),
        "-ar".to_string(),
        plan.sample_rate.to_string(),
        "-c:a".to_string(),
        codec.to_string(),
        "-f".to_string(),
        format.to_string(),
        "pipe:1".to_string(),
    ] {
        args.push(arg.into());
    }
    args
}

/// ffmpeg arguments that decode `path` from `start_ms` to interleaved
/// 64-bit float for the DSP chain.
pub fn float_decode_args(
    path: &Path,
    spec: &FloatDecode,
    start_ms: u64,
    soxr: bool,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["-hide_banner", "-nostdin", "-v", "error"]
        .into_iter()
        .map(OsString::from)
        .collect();
    if start_ms > 0 {
        args.push("-ss".into());
        args.push(format!("{}.{:03}", start_ms / 1000, start_ms % 1000).into());
    }
    args.push("-i".into());
    args.push(path.as_os_str().to_owned());
    for arg in ["-map", "0:a:0", "-vn", "-sn", "-dn"] {
        args.push(arg.into());
    }
    let rate = spec.resample_to.unwrap_or(spec.sample_rate);
    if let Some(to) = spec.resample_to {
        args.push("-af".into());
        args.push(
            if soxr {
                format!("aresample=osr={to}:resampler=soxr:precision=28")
            } else {
                format!("aresample=osr={to}:filter_size=64:phase_shift=10:cutoff=0.97")
            }
            .into(),
        );
    }
    for arg in [
        "-ac".to_string(),
        spec.channels.to_string(),
        "-ar".to_string(),
        rate.to_string(),
        "-c:a".to_string(),
        "pcm_f64le".to_string(),
        "-f".to_string(),
        "f64le".to_string(),
        "pipe:1".to_string(),
    ] {
        args.push(arg.into());
    }
    args
}

fn ffmpeg_float_decoder(path: &Path, spec: &FloatDecode, start_ms: u64) -> io::Result<PcmSource> {
    spawn_ffmpeg(float_decode_args(path, spec, start_ms, ffmpeg_has_soxr()))
}

fn spawn_ffmpeg(args: Vec<OsString>) -> io::Result<PcmSource> {
    let mut child = Command::new("ffmpeg")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| io::Error::other(format!("could not start ffmpeg: {error}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("ffmpeg has no output"))?;
    Ok(Box::new(ChildPcm { child, stdout }))
}

/// ffmpeg's stdout; the process is killed when this is dropped.
struct ChildPcm {
    child: Child,
    stdout: ChildStdout,
}

impl Read for ChildPcm {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.stdout.read(buffer)
    }
}

impl Drop for ChildPcm {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn ffmpeg_decoder(
    path: &Path,
    plan: &OutputPlan,
    _source: &SourceFormat,
    gain_db: Option<f64>,
    start_ms: u64,
) -> io::Result<PcmSource> {
    spawn_ffmpeg(decode_args(
        path,
        plan,
        gain_db,
        start_ms,
        ffmpeg_has_soxr(),
    ))
}

/// Whether ffmpeg was built with the SoX resampler.
fn ffmpeg_has_soxr() -> bool {
    static SOXR: OnceLock<bool> = OnceLock::new();
    *SOXR.get_or_init(|| {
        Command::new("ffmpeg")
            .args(["-hide_banner", "-buildconf"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .is_ok_and(|output| {
                String::from_utf8_lossy(&output.stdout).contains("--enable-libsoxr")
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use parson_output::simulated::{SIMULATED_DEVICE_ID, SimulatedBackend};
    use std::time::{Duration, Instant};

    fn song(rate: u32, bits: u8, lossless: bool) -> Song {
        Song {
            id: "song-1".into(),
            path: "/music/track.flac".into(),
            duration: 200.0,
            audio: Some(AudioProperties {
                codec: Some(if lossless { "flac" } else { "mp3" }.into()),
                container: Some(if lossless { "flac" } else { "mp3" }.into()),
                sample_rate: Some(rate),
                bit_depth: lossless.then_some(bits),
                channels: Some(2),
                bitrate_kbps: Some(if lossless { 2_000 } else { 320 }),
                lossless: Some(lossless),
            }),
            ..Song::default()
        }
    }

    fn outputs(settings_path: Option<PathBuf>) -> NativeOutputs {
        NativeOutputs::new(
            vec![Arc::new(SimulatedBackend)],
            true,
            settings_path,
            Arc::new(|_, _, _, _, _| Ok(Box::new(io::repeat(0)) as PcmSource)),
            "test resampler",
        )
        .with_float_decoder(Arc::new(|_, spec, _| {
            Ok(Box::new(SineDecoder::new(spec.channels, spec.sample_rate)) as PcmSource)
        }))
    }

    /// An endless full-scale-ish sine in `f64le`, standing in for ffmpeg.
    struct SineDecoder {
        channels: u8,
        rate: u32,
        frame: u64,
        pending: Vec<u8>,
    }

    impl SineDecoder {
        fn new(channels: u8, rate: u32) -> Self {
            Self {
                channels,
                rate,
                frame: 0,
                pending: Vec::new(),
            }
        }
    }

    impl Read for SineDecoder {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.pending.is_empty() {
                for _ in 0..1_024 {
                    let t = self.frame as f64 / f64::from(self.rate);
                    let value = 0.9 * (2.0 * std::f64::consts::PI * 1_000.0 * t).sin();
                    for _ in 0..self.channels {
                        self.pending.extend_from_slice(&value.to_le_bytes());
                    }
                    self.frame += 1;
                }
            }
            let count = buffer.len().min(self.pending.len());
            buffer[..count].copy_from_slice(&self.pending[..count]);
            self.pending.drain(..count);
            Ok(count)
        }
    }

    #[test]
    fn devices_report_format_support_and_settings() {
        let outputs = outputs(None);
        let view = outputs.devices(false);
        assert!(view.supported && view.simulated);
        let device = &view.devices[0];
        assert_eq!(device.id, SIMULATED_DEVICE_ID);
        assert_eq!(device.mode_label, "Simulated exclusive output");
        let formats = device.formats.as_ref().unwrap();
        let rate = |value| {
            formats
                .sample_rates
                .iter()
                .find(|entry| entry.value == value)
                .unwrap()
        };
        assert!(rate(192_000).supported);
        assert!(!rate(384_000).supported);
        assert!(device.settings.exclusive);
    }

    #[test]
    fn settings_persist_and_change_the_plan() {
        let dir = std::env::temp_dir().join(format!("parson-native-{}", uuid::Uuid::new_v4()));
        let path = dir.join("native-outputs.json");
        let outputs = outputs(Some(path.clone()));
        let (plan, signal) = outputs
            .describe(SIMULATED_DEVICE_ID, &song(96_000, 24, true))
            .unwrap();
        assert!(plan.bit_perfect);
        assert_eq!(signal.quality, crate::signal_path::SignalQuality::Lossless);
        let capped = OutputSettings {
            exclusive: true,
            max_sample_rate: Some(48_000),
            max_bit_depth: None,
            ..OutputSettings::default()
        };
        let updated = outputs
            .set_settings(SIMULATED_DEVICE_ID, capped.clone())
            .unwrap();
        assert!(!updated.formats.unwrap().sample_rates[2].allowed);
        let (plan, signal) = outputs
            .describe(SIMULATED_DEVICE_ID, &song(96_000, 24, true))
            .unwrap();
        assert_eq!(plan.resample, Some((96_000, 48_000)));
        assert_eq!(
            signal.quality,
            crate::signal_path::SignalQuality::HighQuality
        );
        // A fresh manager reads the saved settings back.
        let reloaded = outputs_with_path(path);
        assert_eq!(reloaded.devices(false).devices[0].settings, capped);
        assert!(
            outputs
                .set_settings(
                    SIMULATED_DEVICE_ID,
                    OutputSettings {
                        max_sample_rate: Some(12_345),
                        ..OutputSettings::default()
                    }
                )
                .is_err()
        );
        assert!(
            outputs
                .set_settings("missing", OutputSettings::default())
                .is_err()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    fn outputs_with_path(path: PathBuf) -> NativeOutputs {
        outputs(Some(path))
    }

    #[test]
    fn a_session_plays_pauses_seeks_and_releases_the_device() {
        let outputs = outputs(None);
        let started = outputs
            .play(7, SIMULATED_DEVICE_ID, &song(44_100, 16, true), 5_000, None)
            .unwrap();
        assert_eq!(started.user_id, 7);
        assert!(started.plan.bit_perfect);
        assert!(outputs.devices(false).devices[0].in_use);
        let deadline = Instant::now() + Duration::from_secs(3);
        while outputs.current().unwrap().status.position_ms < 5_050 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(outputs.current().unwrap().status.position_ms >= 5_050);
        outputs.pause().unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while outputs.current().unwrap().status.state != parson_output::player::PlayerState::Paused
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        outputs.seek(60_000).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(outputs.current().unwrap().status.position_ms, 60_000);
        outputs.stop().unwrap();
        assert!(outputs.current().is_none());
        assert_eq!(outputs.stop(), Err(NativeError::NoSession));
        assert!(!outputs.devices(false).devices[0].in_use);
    }

    fn dsd_song(dir: &Path) -> Song {
        let bytes: Vec<u8> = (0..352_800_u32).map(|n| (n * 37 % 251) as u8).collect();
        let path = dir.join("track.dsf");
        std::fs::write(&path, parson_output::dsd::write_dsf(2_822_400, 2, &bytes)).unwrap();
        Song {
            id: "dsd-1".into(),
            path: path.display().to_string(),
            duration: 0.5,
            audio: Some(AudioProperties {
                codec: Some("dsd".into()),
                container: Some("dsf".into()),
                sample_rate: Some(2_822_400),
                bit_depth: Some(1),
                channels: Some(2),
                bitrate_kbps: Some(5_644),
                lossless: Some(true),
            }),
            ..Song::default()
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("parson-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn stage_ids(path: &ServerSignalPath) -> Vec<&'static str> {
        path.stages.iter().map(|stage| stage.id).collect()
    }

    #[test]
    fn dsd_follows_the_device_strategy() {
        use crate::signal_path::SignalQuality;
        let dir = temp_dir("dsd-strategy");
        let song = dsd_song(&dir);
        let outputs = outputs(None);
        // The simulated DAC reports native DSD_U32_BE up to DSD128.
        let native = outputs
            .play(1, SIMULATED_DEVICE_ID, &song, 0, None)
            .unwrap();
        let dsd = native.plan.dsd.clone().unwrap();
        assert_eq!(dsd.mode, DsdMode::Native);
        assert_eq!(native.plan.sample_format, SampleFormat::DsdU32Be);
        assert_eq!(native.plan.sample_rate, 88_200);
        assert!(native.plan.bit_perfect);
        assert_eq!(native.signal_path.quality, SignalQuality::Lossless);
        assert!(stage_ids(&native.signal_path).contains(&"native-dsd-read"));
        let output = native.signal_path.stages.last().unwrap();
        assert!(
            output.detail.contains("native DSD64 (DSD_U32_BE"),
            "{}",
            output.detail
        );
        assert_eq!(native.signal_path.source.summary, "DSF DSD64 2.8MHz 2ch");
        assert_eq!(
            native.signal_path.delivered.summary,
            "Native DSD64 2.8MHz 2ch"
        );
        // The real player thread reads and plays the file to the end.
        let deadline = Instant::now() + Duration::from_secs(5);
        while outputs.current().unwrap().status.state != PlayerState::Ended
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        let finished = outputs.current().unwrap();
        assert_eq!(
            finished.status.state,
            PlayerState::Ended,
            "{:?}",
            finished.status
        );
        assert!(finished.status.position_ms >= 450);

        let dop = OutputSettings {
            dsd_strategy: parson_output::DsdStrategy::Dop,
            ..OutputSettings::default()
        };
        outputs.set_settings(SIMULATED_DEVICE_ID, dop).unwrap();
        let session = outputs
            .play(1, SIMULATED_DEVICE_ID, &song, 0, None)
            .unwrap();
        assert_eq!(session.plan.dsd.as_ref().unwrap().mode, DsdMode::Dop);
        assert_eq!(session.plan.sample_rate, 176_400);
        assert_eq!(session.plan.sample_format, SampleFormat::S24Packed);
        assert!(stage_ids(&session.signal_path).contains(&"native-dop"));
        assert_eq!(session.signal_path.quality, SignalQuality::Lossless);

        let pcm = OutputSettings {
            dsd_strategy: parson_output::DsdStrategy::Pcm,
            ..OutputSettings::default()
        };
        outputs.set_settings(SIMULATED_DEVICE_ID, pcm).unwrap();
        let (plan, path) = outputs.describe(SIMULATED_DEVICE_ID, &song).unwrap();
        assert_eq!(plan.dsd.as_ref().unwrap().mode, DsdMode::Pcm);
        // 352.8 kHz from the decoder, converted to the DAC's 176.4 kHz.
        assert_eq!(plan.resample, Some((352_800, 176_400)));
        assert!(!plan.bit_perfect);
        assert!(stage_ids(&path).contains(&"dsd-to-pcm"));
        assert_eq!(path.quality, SignalQuality::HighQuality);

        // DSP works on PCM, so it converts DSD even under Native.
        outputs
            .set_settings(SIMULATED_DEVICE_ID, OutputSettings::default())
            .unwrap();
        outputs
            .set_dsp(SIMULATED_DEVICE_ID, boosted(-3.0, Some(-3.0)))
            .unwrap();
        let (plan, path) = outputs.describe(SIMULATED_DEVICE_ID, &song).unwrap();
        let dsd = plan.dsd.as_ref().unwrap();
        assert_eq!(dsd.mode, DsdMode::Pcm);
        assert!(dsd.fallback_reason.as_deref().unwrap().contains("DSP"));
        assert!(path.processing.is_some());
        outputs.stop().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The simulated DAC with a driver that reports what the hardware runs.
    struct ReportingBackend {
        report: Option<&'static str>,
    }

    impl Backend for ReportingBackend {
        fn driver(&self) -> Driver {
            Driver::Simulated
        }

        fn devices(&self) -> Vec<DeviceInfo> {
            SimulatedBackend.devices()
        }

        fn open(
            &self,
            device_id: &str,
            format: StreamFormat,
            exclusive: bool,
        ) -> io::Result<Box<dyn parson_output::Sink>> {
            SimulatedBackend.open(device_id, format, exclusive)
        }

        fn hardware_status(
            &self,
            _device_id: &str,
            stream: &StreamFormat,
        ) -> Option<HardwareStatus> {
            let params = parson_output::procfs::parse_hw_params(self.report?)?;
            Some(HardwareStatus {
                source: "/proc/asound/card1/pcm0p/sub0/hw_params".into(),
                reporter: parson_output::procfs::HardwareReporter::Kernel,
                chain: None,
                matches_stream: parson_output::procfs::params_match(&params, stream),
                params,
                momentary_hz: Some(96_000),
                kernel_emulator: false,
            })
        }
    }

    #[test]
    fn the_driver_report_confirms_or_breaks_the_path() {
        use crate::signal_path::SignalQuality;
        let reporting = |report| {
            NativeOutputs::new(
                vec![Arc::new(ReportingBackend { report })],
                true,
                None,
                Arc::new(|_, _, _, _, _| Ok(Box::new(io::repeat(0)) as PcmSource)),
                "test resampler",
            )
        };
        let matching = reporting(Some(
            "access: RW_INTERLEAVED\nformat: S24_3LE\nsubformat: STD\nchannels: 2\nrate: 96000 (96000/1)\n",
        ));
        let view = matching
            .play(1, SIMULATED_DEVICE_ID, &song(96_000, 24, true), 0, None)
            .unwrap();
        let hardware = view.hardware.expect("the driver reports while playing");
        assert!(hardware.matches_stream);
        let stage = view.signal_path.stages.last().unwrap();
        assert_eq!(stage.id, "native-hardware");
        assert_eq!(stage.title, "Confirmed by the driver");
        assert_eq!(
            stage.detail,
            "hardware running at S24_3LE 96kHz 2ch, USB clock 96kHz"
        );
        assert_eq!(view.signal_path.quality, SignalQuality::Lossless);
        matching.stop().unwrap();

        // Something below Parson (a misconfigured plug, a driver quirk)
        // runs the device at another rate: the path says so.
        let converted = reporting(Some(
            "access: RW_INTERLEAVED\nformat: S16_LE\nsubformat: STD\nchannels: 2\nrate: 48000 (48000/1)\n",
        ));
        let view = converted
            .play(1, SIMULATED_DEVICE_ID, &song(96_000, 24, true), 0, None)
            .unwrap();
        assert!(!view.hardware.unwrap().matches_stream);
        assert_eq!(view.signal_path.quality, SignalQuality::HighQuality);
        converted.stop().unwrap();

        let silent = reporting(None);
        let view = silent
            .play(1, SIMULATED_DEVICE_ID, &song(96_000, 24, true), 0, None)
            .unwrap();
        assert!(view.hardware.is_none());
        assert!(!stage_ids(&view.signal_path).contains(&"native-hardware"));
        silent.stop().unwrap();
    }

    #[test]
    fn unknown_formats_and_devices_are_refused() {
        let outputs = outputs(None);
        let mut unknown = song(44_100, 16, true);
        unknown.audio = None;
        assert_eq!(
            outputs
                .play(1, SIMULATED_DEVICE_ID, &unknown, 0, None)
                .err(),
            Some(NativeError::UnknownFormat)
        );
        assert_eq!(
            outputs
                .play(1, "alsa:none:0", &song(44_100, 16, true), 0, None)
                .err(),
            Some(NativeError::UnknownDevice)
        );
    }

    #[test]
    fn decode_arguments_only_resample_or_dither_when_the_plan_says_so() {
        let source = SourceFormat {
            sample_rate: 96_000,
            bit_depth: Some(24),
            channels: 2,
            lossless: true,
        };
        let caps = parson_output::simulated::simulated_caps();
        let plan = plan_output(source, Some(&caps), &OutputSettings::default()).unwrap();
        assert_eq!(decode_filter(&plan, None, true), None);
        let args: Vec<String> = decode_args(Path::new("/m/a.flac"), &plan, None, 61_250, true)
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let joined = args.join(" ");
        assert!(joined.contains("-ss 61.250 -i /m/a.flac"));
        assert!(joined.ends_with("-ac 2 -ar 96000 -c:a pcm_s32le -f s32le pipe:1"));
        assert!(!joined.contains("-af"));

        let capped = OutputSettings {
            max_sample_rate: Some(48_000),
            max_bit_depth: Some(16),
            ..OutputSettings::default()
        };
        let plan = plan_output(source, Some(&caps), &capped).unwrap();
        assert_eq!(
            decode_filter(&plan, None, true).as_deref(),
            Some("aresample=osr=48000:resampler=soxr:precision=28:dither_method=triangular")
        );
        assert!(
            decode_filter(&plan, None, false)
                .unwrap()
                .contains("filter_size=64")
        );
        let joined = decode_args(Path::new("/m/a.flac"), &plan, None, 0, true)
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(!joined.contains("-ss"));
        assert!(joined.ends_with("-c:a pcm_s16le -f s16le pipe:1"));
    }

    #[test]
    fn leveling_is_applied_in_double_precision_and_is_not_bit_perfect() {
        let outputs = outputs(None);
        let leveling = LevelingRequest {
            gain_db: -6.4,
            source: "album".into(),
        };
        let started = outputs
            .play(
                1,
                SIMULATED_DEVICE_ID,
                &song(96_000, 24, true),
                0,
                Some(leveling.clone()),
            )
            .unwrap();
        assert!(!started.plan.bit_perfect);
        assert_eq!(started.leveling, Some(leveling));
        assert_eq!(
            started.signal_path.quality,
            crate::signal_path::SignalQuality::Enhanced
        );
        outputs.stop().unwrap();
        let caps = parson_output::simulated::simulated_caps();
        let source = SourceFormat {
            sample_rate: 96_000,
            bit_depth: Some(24),
            channels: 2,
            lossless: true,
        };
        let plan = plan_output(source, Some(&caps), &OutputSettings::default()).unwrap();
        assert_eq!(
            decode_filter(&plan, Some(-6.4), true).as_deref(),
            Some("volume=-6.40dB:precision=double,aresample=osr=96000")
        );
        assert_eq!(decode_filter(&plan, Some(0.0), true), None);
        assert!(
            LevelingRequest {
                gain_db: 40.0,
                source: "track".into()
            }
            .validate()
            .is_err()
        );
        assert!(
            LevelingRequest {
                gain_db: -3.0,
                source: "loud".into()
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn source_formats_come_from_indexed_properties() {
        let lossy = source_format(song(44_100, 16, false).audio.as_ref()).unwrap();
        assert_eq!(lossy.bit_depth, None);
        assert!(!lossy.lossless);
        let flac = source_format(song(88_200, 24, true).audio.as_ref()).unwrap();
        assert_eq!(flac.bit_depth, Some(24));
        assert_eq!(flac.sample_rate, 88_200);
    }

    fn boosted(gain_db: f64, headroom: Option<f64>) -> DspSettings {
        let mut dsp = DspSettings::default();
        dsp.eq.enabled = true;
        dsp.eq.preset = "custom".into();
        dsp.eq.bands[2].gain_db = gain_db;
        if let Some(headroom) = headroom {
            dsp.headroom.enabled = true;
            dsp.headroom.gain_db = headroom;
        }
        dsp
    }

    fn wait_for_status(
        outputs: &NativeOutputs,
        done: impl Fn(&parson_output::dsp::DspStatus) -> bool,
    ) -> parson_output::dsp::DspStatus {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let status = outputs
                .current()
                .and_then(|session| session.signal_path.processing)
                .and_then(|processing| processing.status)
                .expect("a processing session");
            if done(&status) || Instant::now() > deadline {
                return status;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn the_server_runs_the_dsp_chain_and_flags_clipping() {
        let dir = std::env::temp_dir().join(format!("parson-native-dsp-{}", uuid::Uuid::new_v4()));
        let outputs = outputs(Some(dir.join("native-outputs.json")));
        // +12 dB at 1 kHz on a 0.9 full-scale 1 kHz tone clips.
        let view = outputs
            .set_dsp(SIMULATED_DEVICE_ID, boosted(12.0, None))
            .unwrap();
        assert!(view.dsp_customized && view.dsp_support.available);
        let started = outputs
            .play(1, SIMULATED_DEVICE_ID, &song(44_100, 16, true), 0, None)
            .unwrap();
        assert!(!started.plan.bit_perfect);
        // The chain dithers to the device's widest word.
        assert_eq!(started.plan.bit_depth, 32);
        assert_eq!(
            started.signal_path.quality,
            crate::signal_path::SignalQuality::Enhanced
        );
        // Clipping can be seen before the first speed measurement.
        let status = wait_for_status(&outputs, |status| {
            status.clipping && status.realtime_factor.is_some()
        });
        assert!(status.clipping && status.clipped_samples > 0);
        assert!(status.realtime_factor.unwrap() > 1.0);

        // Saving a chain with headroom restarts the track with it.
        outputs
            .set_dsp(SIMULATED_DEVICE_ID, boosted(12.0, Some(-12.0)))
            .unwrap();
        let session = outputs.current().unwrap();
        assert!(session.processing.unwrap().headroom_db.is_some());
        let status = wait_for_status(&outputs, |status| {
            status.realtime_factor.is_some() && status.peak_dbfs.is_some()
        });
        assert!(!status.clipping, "{status:?}");

        // The chain is saved per device and read back.
        let reloaded = outputs_with_path(dir.join("native-outputs.json"));
        let device = reloaded.devices(false).devices.remove(0);
        assert_eq!(device.dsp.headroom.gain_db, -12.0);
        // Bypassing the whole chain gives the bit-perfect path back.
        let mut bypassed = boosted(12.0, Some(-12.0));
        bypassed.enabled = false;
        outputs.set_dsp(SIMULATED_DEVICE_ID, bypassed).unwrap();
        let session = outputs.current().unwrap();
        assert!(session.plan.bit_perfect && session.processing.is_none());
        assert!(session.signal_path.processing.is_none());
        outputs.stop().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn float_decode_arguments_feed_the_chain() {
        let spec = FloatDecode {
            channels: 2,
            sample_rate: 44_100,
            resample_to: None,
        };
        let joined = float_decode_args(Path::new("/m/a.flac"), &spec, 1_500, true)
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(joined.contains("-ss 1.500 -i /m/a.flac"));
        assert!(!joined.contains("-af"));
        assert!(joined.ends_with("-ac 2 -ar 44100 -c:a pcm_f64le -f f64le pipe:1"));
        let joined = float_decode_args(
            Path::new("/m/a.flac"),
            &FloatDecode {
                resample_to: Some(768_000),
                ..spec
            },
            0,
            true,
        )
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
        assert!(joined.contains("-af aresample=osr=768000:resampler=soxr"));
        assert!(joined.contains("-ar 768000"));
    }

    #[test]
    fn ffmpeg_decodes_a_real_flac_into_the_chain() {
        // Exercises the production decoder end to end when ffmpeg exists.
        let dir = std::env::temp_dir().join(format!("parson-flac-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let flac = dir.join("tone.flac");
        let made = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
            ])
            .arg("sine=frequency=1000:sample_rate=44100:duration=0.5")
            .args(["-ac", "2", "-sample_fmt", "s16", "-y"])
            .arg(&flac)
            .status()
            .is_ok_and(|status| status.success());
        if !made {
            let _ = std::fs::remove_dir_all(dir);
            return;
        }
        let spec = FloatDecode {
            channels: 2,
            sample_rate: 44_100,
            resample_to: None,
        };
        let processing = ProcessingPlan {
            leveling_db: None,
            headroom_db: Some(-3.0),
            bands: boosted(3.0, None).active_bands(),
            crossfeed: Some(parson_output::dsp::CrossfeedStrength::Medium),
            crossfeed_skipped: false,
            channels: 2,
            input_rate: 44_100,
            output_rate: 48_000,
            rate_requested: true,
            requested_rate_unavailable: None,
            resampler: Some(ResamplerKind::Parson),
            output_bits: 24,
        };
        let stats = Arc::new(DspStats::default());
        let decoded = ffmpeg_float_decoder(&flac, &spec, 0).unwrap();
        let mut pcm = DspPcm::new(
            decoded,
            DspChain::new(&processing, 44_100, InputFormat::S32, Arc::clone(&stats)),
        );
        let mut out = Vec::new();
        pcm.read_to_end(&mut out).unwrap();
        // 0.5 s at 48 kHz, stereo, 4-byte samples.
        assert_eq!(out.len(), 24_000 * 2 * 4);
        let status = stats.snapshot();
        assert!(!status.clipping);
        assert!(status.peak_dbfs.unwrap() < 0.0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
