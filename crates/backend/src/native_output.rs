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

use parson_output::player::{InputFormat, PcmOpener, PcmSource, Player, PlayerStatus};
use parson_output::{
    Backend, DepthChange, DeviceInfo, Driver, FormatSupport, OutputPlan, OutputSettings, PlanError,
    SampleFormat, SourceFormat, StreamFormat, format_support, plan_output,
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
    pub signal_path: ServerSignalPath,
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
    song_id: String,
    user_id: i32,
    duration_ms: u64,
    plan: OutputPlan,
    leveling: Option<LevelingRequest>,
    signal_path: ServerSignalPath,
    player: Player,
}

impl Session {
    fn view(&self) -> SessionView {
        SessionView {
            device_id: self.device_id.clone(),
            device_name: self.device_name.clone(),
            song_id: self.song_id.clone(),
            user_id: self.user_id,
            duration_ms: self.duration_ms,
            status: self.player.status(),
            plan: self.plan.clone(),
            leveling: self.leveling.clone(),
            signal_path: self.signal_path.clone(),
        }
    }
}

#[derive(Default)]
struct State {
    devices: Vec<DeviceInfo>,
    probed: bool,
    settings: HashMap<String, OutputSettings>,
    settings_loaded: bool,
}

/// How decoded PCM is produced for a track; ffmpeg in production.
pub type DecoderFactory = Arc<
    dyn Fn(&Path, &OutputPlan, &SourceFormat, Option<f64>, u64) -> io::Result<PcmSource>
        + Send
        + Sync,
>;

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
    resampler: &'static str,
    state: Mutex<State>,
    session: Mutex<Option<Session>>,
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
            resampler,
            state: Mutex::new(State::default()),
            session: Mutex::new(None),
        }
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
    }

    fn save_settings(&self, settings: &HashMap<String, OutputSettings>) -> io::Result<()> {
        let Some(path) = &self.settings_path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, serde_json::to_vec_pretty(settings)?)?;
        std::fs::rename(temp, path)
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
        Ok(view(&device, settings, busy))
    }

    fn device(
        &self,
        device_id: &str,
    ) -> Result<(DeviceInfo, OutputSettings, Arc<dyn Backend>), NativeError> {
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
        Ok((device, settings, backend))
    }

    /// How `song` would play on `device_id`, without playing it.
    pub fn describe(
        &self,
        device_id: &str,
        song: &Song,
    ) -> Result<(OutputPlan, ServerSignalPath), NativeError> {
        let (device, settings, _) = self.device(device_id)?;
        self.plan_and_path(&device, &settings, song, None)
    }

    fn plan_and_path(
        &self,
        device: &DeviceInfo,
        settings: &OutputSettings,
        song: &Song,
        leveling: Option<&LevelingRequest>,
    ) -> Result<(OutputPlan, ServerSignalPath), NativeError> {
        let source = source_format(song.audio.as_ref()).ok_or(NativeError::UnknownFormat)?;
        let mut plan =
            plan_output(source, device.caps.as_ref(), settings).map_err(NativeError::Plan)?;
        // Processing the listener asked for changes the samples on purpose.
        if leveling.is_some() {
            plan.bit_perfect = false;
        }
        let extension = Path::new(&song.path)
            .extension()
            .and_then(|extension| extension.to_str());
        let mode_label = device.driver.mode_label(settings.exclusive);
        let path = describe_native_leg(
            &song.id,
            song.audio.as_ref(),
            extension,
            &plan,
            &NativeOutputTarget {
                device_name: &device.name,
                mode_label,
                simulated: device.driver == Driver::Simulated,
                resampler: self.resampler,
                leveling: leveling.map(|leveling| NativeLeveling {
                    gain_db: leveling.gain_db,
                    source: &leveling.source,
                }),
            },
        );
        Ok((plan, path))
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
        let (device, settings, backend) = self.device(device_id)?;
        let (plan, signal_path) =
            self.plan_and_path(&device, &settings, song, leveling.as_ref())?;
        let source = source_format(song.audio.as_ref()).ok_or(NativeError::UnknownFormat)?;
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
        let decoder = Arc::clone(&self.decoder);
        let song_path = PathBuf::from(&song.path);
        let decode_plan = plan.clone();
        let gain_db = leveling.as_ref().map(|leveling| leveling.gain_db);
        let opener: PcmOpener =
            Box::new(move |position| decoder(&song_path, &decode_plan, &source, gain_db, position));
        let player = Player::start(
            Box::new(move || backend.open(&open_id, stream, exclusive)),
            opener,
            stream,
            InputFormat::for_container(plan.sample_format),
            position_ms,
        )
        .map_err(|error| NativeError::Device(format!("{}: {error}", device.name)))?;
        let started = Session {
            device_id: device.id.clone(),
            device_name: device.name.clone(),
            song_id: song.id.clone(),
            user_id,
            duration_ms: (song.duration.max(0.0) * 1000.0) as u64,
            plan,
            leveling,
            signal_path,
            player,
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

fn view(device: &DeviceInfo, settings: OutputSettings, in_use: bool) -> DeviceView {
    DeviceView {
        id: device.id.clone(),
        name: device.name.clone(),
        detail: device.detail.clone(),
        driver: device.driver,
        mode_label: device.driver.mode_label(settings.exclusive).to_string(),
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
        settings,
        in_use,
    }
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
        InputFormat::S32 => ("pcm_s32le", "s32le"),
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
    let mut child = Command::new("ffmpeg")
        .args(decode_args(
            path,
            plan,
            gain_db,
            start_ms,
            ffmpeg_has_soxr(),
        ))
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
}
