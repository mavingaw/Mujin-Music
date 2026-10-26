//! The playback thread of one audio device.
//!
//! Each track goes through the server's one pipeline
//! ([`crate::outputs::pipeline`]): planned against the device's formats
//! and the zone's device setup, run through the zone's DSP chain when it
//! has one, and decoded (or, for DSD, read) to exactly the format the
//! device is opened with. The thread writes frames to the device, starts
//! the next track on the same open stream for gapless playback, and
//! reports position and the signal path the pipeline described.

use std::io::Read;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use parson_output::dsp::DspStats;
use parson_output::player::{InputFormat, PcmSource};

use super::{Backend, DeviceInfo, SampleFormat, Sink, StreamFormat};
use crate::outputs::pipeline::{DevicePlace, Pipeline, PlannedTrack, TrackSource};
use crate::outputs::renderer::{PlayState, RendererSettings, RendererState, TrackRequest};
use crate::outputs::{describe_source, format_rate};
use crate::signal_path::{
    ServerSignalPath, SignalQuality, SignalStage, StageKind, add_hardware_report,
    carry_lossy_generation, overall,
};
use crate::zones::{StreamQuality, VolumeMode, ZoneStreamInfo};

/// Frames per write: about 20 ms at 192 kHz, so controls stay responsive.
const CHUNK_FRAMES: usize = 4_096;

pub enum Control {
    Load {
        track: Box<TrackRequest>,
        next: Option<Box<TrackRequest>>,
        position_ms: i64,
        play: bool,
    },
    SetNext(Option<Box<TrackRequest>>),
    Play,
    Pause,
    Stop,
    Seek(i64),
    Configure(RendererSettings),
    /// Software volume, 0..=100.
    Volume(u8),
    Mute {
        muted: bool,
        /// The device muted itself; only the state changes here.
        hardware: bool,
    },
    Shutdown,
}

/// Where the device is, for the signal path.
#[derive(Clone, Debug, Default)]
pub struct Route {
    /// The machine the device is attached to; this server when `None`.
    pub host: Option<String>,
    /// After the device name, e.g. `on kitchen-pi via Parson Bridge`.
    pub suffix: Option<String>,
    /// A step every stream takes (sync corrections of a group).
    pub step: Option<String>,
    /// The output has no exclusive/shared mode (AirPlay).
    pub modeless: bool,
}

struct Shared {
    state: RendererState,
    settings: RendererSettings,
    /// Live measurements of the playing track's DSP chain.
    stats: Option<Arc<DspStats>>,
}

/// Handle to a device's playback thread.
pub struct Engine {
    sender: Mutex<Sender<Control>>,
    shared: Arc<Mutex<Shared>>,
}

fn lock(shared: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Engine {
    pub fn start(
        backend: Arc<dyn Backend>,
        device: DeviceInfo,
        changed: tokio::sync::broadcast::Sender<()>,
        route: Route,
        pipeline: Arc<Pipeline>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        let settings = RendererSettings {
            output: parson_output::OutputSettings::default(),
            volume_mode: if device.hardware_volume.is_some() {
                VolumeMode::Hardware
            } else {
                VolumeMode::Fixed
            },
            limits: super::device_limits(&device),
        };
        let shared = Arc::new(Mutex::new(Shared {
            state: RendererState {
                volume: 100,
                ..RendererState::default()
            },
            settings,
            stats: None,
        }));
        let thread_shared = shared.clone();
        let name = format!("parson-audio-{}", device.id);
        let spawned = std::thread::Builder::new().name(name).spawn(move || {
            Player {
                backend,
                device,
                route,
                pipeline,
                receiver,
                shared: thread_shared,
                changed,
                current: None,
                next: None,
                stream: None,
                stream_format: None,
                paused: false,
                software_volume: 100,
                muted: false,
            }
            .run();
        });
        if let Err(error) = spawned {
            tracing::error!(%error, "could not start the audio device thread");
        }
        Self {
            sender: Mutex::new(sender),
            shared,
        }
    }

    pub fn send(&self, control: Control) {
        let _ = self
            .sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .send(control);
    }

    /// The device's state, with the DSP chain's live measurements.
    pub fn state(&self) -> RendererState {
        let shared = lock(&self.shared);
        let mut state = shared.state.clone();
        if let (Some(stats), Some(processing)) = (
            shared.stats.as_ref(),
            state
                .stream
                .as_mut()
                .and_then(|stream| stream.signal_path.as_mut())
                .and_then(|path| path.processing.as_mut()),
        ) {
            processing.status = Some(stats.snapshot());
        }
        state
    }

    pub fn settings(&self) -> RendererSettings {
        lock(&self.shared).settings
    }

    /// Records a hardware volume change made through the backend.
    pub fn hardware_volume_set(&self, level: u8, db: f32) {
        let mut shared = lock(&self.shared);
        shared.state.volume = level;
        shared.state.device_volume = Some(db);
    }
}

/// Bytes of one decoded frame as the pipeline writes it.
fn input_frame_bytes(input: InputFormat, stream: &StreamFormat) -> usize {
    let sample = match input {
        InputFormat::S16 => 2,
        InputFormat::S32 => 4,
        InputFormat::Raw => stream.sample_format.bytes(),
    };
    sample * usize::from(stream.channels.max(1))
}

struct Active {
    track: TrackRequest,
    planned: PlannedTrack,
    pcm: PcmSource,
    /// Bytes of a decoded frame.
    frame_bytes: usize,
    /// A partial frame left over from the last read.
    pending: Vec<u8>,
    /// Where the decoder started, in the track.
    start_ms: i64,
    frames_written: u64,
    /// The stream was dropped on pause: restart here on play.
    resume_at: Option<i64>,
}

impl Active {
    /// Reads up to `frames` whole frames; an empty result is the end.
    fn read(&mut self, frames: usize) -> Vec<u8> {
        let wanted = frames * self.frame_bytes;
        let mut buffer = std::mem::take(&mut self.pending);
        let mut chunk = vec![0_u8; wanted.saturating_sub(buffer.len()).max(self.frame_bytes)];
        while buffer.len() < wanted {
            match self.pcm.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => buffer.extend_from_slice(&chunk[..read]),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
            if buffer.len() >= self.frame_bytes * 256 {
                // Enough for a write; don't wait for a full chunk.
                break;
            }
        }
        let whole = buffer.len() - buffer.len() % self.frame_bytes;
        self.pending = buffer.split_off(whole);
        buffer
    }
}

struct Player {
    backend: Arc<dyn Backend>,
    device: DeviceInfo,
    route: Route,
    pipeline: Arc<Pipeline>,
    receiver: Receiver<Control>,
    shared: Arc<Mutex<Shared>>,
    changed: tokio::sync::broadcast::Sender<()>,
    current: Option<Active>,
    next: Option<TrackRequest>,
    stream: Option<Box<dyn Sink>>,
    /// The format and mode `stream` was opened with.
    stream_format: Option<(StreamFormat, bool)>,
    paused: bool,
    software_volume: u8,
    muted: bool,
}

impl Player {
    fn settings(&self) -> RendererSettings {
        lock(&self.shared).settings
    }

    fn notify(&self) {
        let _ = self.changed.send(());
    }

    fn run(mut self) {
        loop {
            let control = if self.current.is_some() && !self.paused {
                match self.receiver.try_recv() {
                    Ok(control) => Some(control),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => return,
                }
            } else {
                match self.receiver.recv_timeout(Duration::from_millis(500)) {
                    Ok(control) => Some(control),
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            };
            if let Some(control) = control {
                if !self.handle(control) {
                    return;
                }
                continue;
            }
            self.pump();
        }
    }

    /// Applies one control; `false` ends the thread.
    fn handle(&mut self, control: Control) -> bool {
        match control {
            Control::Shutdown => {
                self.close_stream(false);
                return false;
            }
            Control::Load {
                track,
                next,
                position_ms,
                play,
            } => {
                self.next = next.map(|next| *next);
                self.paused = !play;
                if let Some(stream) = self.stream.as_mut() {
                    let _ = stream.flush();
                }
                self.begin(*track, position_ms.max(0));
            }
            Control::SetNext(next) => self.next = next.map(|next| *next),
            Control::Play => {
                if self.paused {
                    self.paused = false;
                    if let Some(active) = self.current.as_mut()
                        && let Some(position) = active.resume_at.take()
                    {
                        let track = active.track.clone();
                        self.begin(track, position);
                    } else if let Some(stream) = self.stream.as_mut() {
                        let _ = stream.pause(false);
                    }
                    self.set_state(PlayState::Playing);
                }
            }
            Control::Pause => {
                if self.current.is_some() && !self.paused {
                    self.paused = true;
                    let position = self.position_ms();
                    let paused_in_place = self
                        .stream
                        .as_mut()
                        .map(|stream| stream.pause(true).is_ok())
                        .unwrap_or(false);
                    if !paused_in_place {
                        if let Some(stream) = self.stream.as_mut() {
                            let _ = stream.flush();
                        }
                        if let Some(active) = self.current.as_mut() {
                            active.resume_at = Some(position);
                        }
                    }
                    let mut shared = lock(&self.shared);
                    shared.state.position_ms = position;
                    shared.state.polled_at_ms = crate::zones::service::now_ms();
                    shared.state.state = PlayState::Paused;
                    drop(shared);
                    self.notify();
                }
            }
            Control::Stop => {
                self.current = None;
                self.paused = false;
                self.close_stream(false);
                let mut shared = lock(&self.shared);
                shared.state.state = PlayState::Stopped;
                shared.state.position_ms = 0;
                shared.state.stream = None;
                shared.stats = None;
                drop(shared);
                self.notify();
            }
            Control::Seek(position_ms) => {
                if let Some(active) = self.current.as_ref() {
                    let track = active.track.clone();
                    if let Some(stream) = self.stream.as_mut() {
                        let _ = stream.flush();
                    }
                    self.begin(track, position_ms.max(0));
                    if self.paused
                        && let Some(active) = self.current.as_mut()
                    {
                        active.resume_at = Some(position_ms.max(0));
                    }
                }
            }
            Control::Configure(settings) => {
                let replan = settings.output != self.settings().output;
                lock(&self.shared).settings = settings;
                if settings.volume_mode != VolumeMode::Software {
                    self.software_volume = 100;
                }
                if replan {
                    // Replan in the new setup at the heard position.
                    if let Some(active) = self.current.as_ref() {
                        let track = active.track.clone();
                        let position = self.position_ms();
                        self.close_stream(false);
                        self.begin(track, position);
                    } else {
                        self.close_stream(false);
                    }
                }
                self.refresh_stream_info();
            }
            Control::Volume(level) => {
                self.software_volume = level.min(100);
                lock(&self.shared).state.volume = self.software_volume;
                self.refresh_stream_info();
                self.notify();
            }
            Control::Mute { muted, hardware } => {
                self.muted = muted && !hardware;
                lock(&self.shared).state.muted = muted;
                self.notify();
            }
        }
        true
    }

    /// Plans `track` and starts decoding it at `position_ms`.
    fn plan(&self, track: &TrackRequest) -> Result<PlannedTrack, String> {
        let settings = self.settings();
        let host = self
            .route
            .host
            .as_deref()
            .unwrap_or_else(|| crate::outputs::pipeline::host_name());
        self.pipeline
            .plan(
                &self.device,
                &settings.output,
                &track.dsp,
                TrackSource {
                    song_id: &track.song_id,
                    path: &track.path,
                    audio: track.audio.as_ref(),
                },
                track.leveling.as_ref(),
                DevicePlace { host },
            )
            .map_err(|error| error.to_string())
    }

    /// Starts decoding `track` at `position_ms`.
    fn begin(&mut self, track: TrackRequest, position_ms: i64) {
        let started = self.plan(&track).and_then(|planned| {
            let stats = Arc::new(DspStats::default());
            let mut opener = self.pipeline.opener(
                PathBuf::from(&track.path),
                &planned.samples,
                Arc::clone(&stats),
            );
            let pcm = opener(position_ms.max(0) as u64)
                .map_err(|error| format!("could not start the decoder: {error}"))?;
            Ok((planned, pcm, stats))
        });
        match started {
            Ok((planned, pcm, stats)) => {
                self.current = Some(Active {
                    frame_bytes: input_frame_bytes(planned.input, &planned.stream),
                    track: track.clone(),
                    planned,
                    pcm,
                    pending: Vec::new(),
                    start_ms: position_ms,
                    frames_written: 0,
                    resume_at: None,
                });
                {
                    let mut shared = lock(&self.shared);
                    shared.state.song_id = Some(track.song_id.clone());
                    shared.state.serial = Some(track.serial);
                    shared.state.position_ms = position_ms;
                    shared.state.duration_ms = track.duration_ms;
                    shared.state.polled_at_ms = crate::zones::service::now_ms();
                    shared.state.state = if self.paused {
                        PlayState::Paused
                    } else {
                        PlayState::Playing
                    };
                    shared.state.reachable = true;
                    shared.stats = Some(stats);
                }
                self.refresh_stream_info();
                self.notify();
            }
            Err(error) => {
                tracing::warn!(%error, device = self.device.id, "could not play track");
                self.current = None;
                lock(&self.shared).state.serial = Some(track.serial);
                self.finish();
            }
        }
    }

    fn position_ms(&mut self) -> i64 {
        let Some(active) = self.current.as_ref() else {
            return 0;
        };
        if let Some(position) = active.resume_at {
            return position;
        }
        let delay = self
            .stream
            .as_mut()
            .map(|stream| stream.delay_frames())
            .unwrap_or(0);
        let Some(active) = self.current.as_ref() else {
            return 0;
        };
        let heard = active.frames_written.saturating_sub(delay);
        active.start_ms
            + (heard as i64 * 1_000) / i64::from(active.planned.stream.sample_rate.max(1))
    }

    fn set_state(&self, state: PlayState) {
        lock(&self.shared).state.state = state;
        self.notify();
    }

    fn close_stream(&mut self, drain: bool) {
        self.stream_format = None;
        if let Some(mut stream) = self.stream.take() {
            if drain {
                let _ = stream.drain();
            } else {
                let _ = stream.flush();
            }
        }
    }

    /// The queue ran out: release the device (exclusive mode would keep
    /// other apps from it) and report the end.
    fn finish(&mut self) {
        self.close_stream(true);
        let ended = self.current.take().map(|active| active.track.serial);
        let mut shared = lock(&self.shared);
        shared.state.state = PlayState::Stopped;
        shared.state.position_ms = 0;
        shared.state.stream = None;
        shared.state.finished_serial = ended.or(shared.state.serial);
        shared.stats = None;
        drop(shared);
        self.notify();
    }

    /// Software volume is scaling the samples.
    fn software_gain_active(&self) -> bool {
        self.settings().volume_mode == VolumeMode::Software
            && (self.software_volume < 100 || self.muted)
    }

    fn refresh_stream_info(&mut self) {
        let hardware = match (self.current.as_ref(), self.stream_format) {
            (Some(active), Some((format, _))) if format == active.planned.stream => self
                .backend
                .hardware_status(&self.device.id, &active.planned.stream),
            _ => None,
        };
        let info = self.current.as_ref().map(|active| {
            stream_info(
                active,
                &self.device,
                &self.route,
                self.settings(),
                self.software_gain_active(),
                hardware.as_ref(),
            )
        });
        lock(&self.shared).state.stream = info;
    }

    /// Decodes and writes one chunk, moving to the next track at the end.
    fn pump(&mut self) {
        let Some(active) = self.current.as_mut() else {
            return;
        };
        let pcm = active.read(CHUNK_FRAMES);
        let frame_bytes = active.frame_bytes;
        let format = active.planned.stream;
        let exclusive = active.planned.exclusive;
        let input = active.planned.input;
        if pcm.is_empty() {
            self.track_ended();
            return;
        }
        if self.stream.is_none() || self.stream_format != Some((format, exclusive)) {
            // New format (or first track): drain the old stream, reopen.
            self.close_stream(true);
            match self.backend.open(&self.device.id, format, exclusive) {
                Ok(stream) => {
                    self.stream = Some(stream);
                    self.stream_format = Some((format, exclusive));
                    self.refresh_stream_info();
                }
                Err(error) => {
                    tracing::warn!(%error, device = self.device.id, "could not open audio device");
                    // Stop rather than skip through the queue.
                    self.current = None;
                    self.close_stream(false);
                    let mut shared = lock(&self.shared);
                    shared.state.reachable = false;
                    shared.state.state = PlayState::Stopped;
                    shared.state.stream = None;
                    shared.stats = None;
                    drop(shared);
                    self.notify();
                    return;
                }
            }
        }
        let gain = if self.muted {
            Some(0.0)
        } else if self.settings().volume_mode == VolumeMode::Software && self.software_volume < 100
        {
            // Same curve as hardware dB volume: 0 mutes, 100 is unity.
            let db = -60.0 * (1.0 - f64::from(self.software_volume) / 100.0);
            Some(if self.software_volume == 0 {
                0.0
            } else {
                10_f64.powf(db / 20.0)
            })
        } else {
            None
        };
        let frames = pcm.len() / frame_bytes;
        let data = convert(&pcm, input, format.sample_format, gain);
        let Some(stream) = self.stream.as_mut() else {
            return;
        };
        if let Err(error) = stream.write(&data) {
            tracing::warn!(%error, device = self.device.id, "audio device write failed");
            self.close_stream(false);
            let mut shared = lock(&self.shared);
            if error
                .to_string()
                .contains(parson_bridge::endpoint::REPLACED)
            {
                // Another zone (or a synced group) now plays this DAC: stop
                // instead of taking it back, so the two never fight.
                self.current = None;
                shared.state.state = PlayState::Stopped;
                shared.state.position_ms = 0;
                shared.state.stream = None;
                shared.stats = None;
                drop(shared);
                self.notify();
                return;
            }
            shared.state.reachable = false;
            return;
        }
        if let Some(active) = self.current.as_mut() {
            active.frames_written += frames as u64;
        }
        let position = self.position_ms();
        let mut shared = lock(&self.shared);
        shared.state.position_ms = position;
        shared.state.polled_at_ms = crate::zones::service::now_ms();
        shared.state.reachable = true;
    }

    fn track_ended(&mut self) {
        match self.next.take() {
            Some(next) => {
                // Same open stream when the format matches: gapless.
                self.begin(next, 0);
            }
            None => self.finish(),
        }
    }
}

/// The zone's view of what plays: the pipeline's signal path, with what
/// the engine itself adds (software volume, a group's sync) and what the
/// driver reports.
fn stream_info(
    active: &Active,
    device: &DeviceInfo,
    route: &Route,
    settings: RendererSettings,
    software_gain: bool,
    hardware: Option<&parson_output::procfs::HardwareStatus>,
) -> ZoneStreamInfo {
    let planned = &active.planned;
    let mut path: ServerSignalPath = planned.signal_path.clone();
    let mut added = Vec::new();
    if let Some(step) = &route.step {
        let detail = step.strip_prefix("Sync: ").unwrap_or(step);
        added.push(SignalStage {
            id: "sync",
            kind: StageKind::Transport,
            title: "Sync".into(),
            detail: detail.to_string(),
            quality: Some(SignalQuality::HighQuality),
            explanation: format!(
                "{detail}. A repeated or skipped frame is inaudible but changes the samples, so the path is not bit-perfect."
            ),
        });
    }
    if software_gain {
        added.push(SignalStage {
            id: "software-volume",
            kind: StageKind::Dsp,
            title: "Software volume".into(),
            detail: "Parson scales the samples".into(),
            quality: Some(SignalQuality::HighQuality),
            explanation: "The zone's volume is set to Software, so Parson lowers the level by scaling every sample before the device. Set it to Hardware (the device's own control) or Fixed for a bit-perfect path.".into(),
        });
    }
    if !added.is_empty() {
        // Before the transport and output stages at the end.
        let at = path
            .stages
            .iter()
            .position(|stage| stage.id == "native-transport")
            .unwrap_or(path.stages.len());
        path.stages.splice(at..at, added);
    }
    if let Some(hardware) = hardware {
        add_hardware_report(&mut path, &device.name, hardware, &planned.stream);
    }
    carry_lossy_generation(&mut path.stages);
    path.quality = overall(&path.stages);
    let steps = path
        .stages
        .iter()
        .filter(|stage| {
            !matches!(
                stage.id,
                "source" | "native-decode" | "native-transport" | "native-output"
            ) && stage.kind != StageKind::Source
        })
        .map(|stage| format!("{}: {}", stage.title, stage.detail))
        .collect();
    let quality = match path.quality {
        SignalQuality::Lossless => StreamQuality::Lossless,
        SignalQuality::Enhanced => StreamQuality::Enhanced,
        SignalQuality::HighQuality => StreamQuality::High,
        SignalQuality::LowQuality => StreamQuality::Low,
    };
    // The samples reach the output unchanged (an AirPlay receiver's own
    // processing after that still rates the path High quality).
    let bit_perfect = planned.samples.plan.bit_perfect
        && planned.samples.processing.is_none()
        && settings.volume_mode != VolumeMode::Software
        && route.step.is_none()
        && path.hardware_confirmed != Some(false);
    let format = &planned.stream;
    let sent = match planned.samples.plan.dsd.as_ref() {
        Some(dsd) if dsd.mode == parson_output::DsdMode::Native => format!(
            "{} ({}, {} frames)",
            dsd.dsd_label,
            format.sample_format.alsa_name(),
            format_rate(format.sample_rate)
        ),
        Some(dsd) if dsd.mode == parson_output::DsdMode::Dop => format!(
            "{} as DoP in {}/{}-bit ({})",
            dsd.dsd_label,
            format_rate(format.sample_rate),
            format.sample_format.bits(),
            format.sample_format.alsa_name()
        ),
        _ => format!(
            "{}/{}-bit ({})",
            format_rate(format.sample_rate),
            format.sample_format.bits(),
            format.sample_format.alsa_name()
        ),
    };
    ZoneStreamInfo {
        source: describe_source(&active.track.source),
        output: format!(
            "{sent} to {}{}{}",
            device.name,
            route
                .suffix
                .as_ref()
                .map(|suffix| format!(" {suffix}"))
                .unwrap_or_default(),
            match (route.modeless, planned.exclusive) {
                (true, _) => "",
                (false, true) => ", exclusive",
                (false, false) => ", shared",
            }
        ),
        bit_perfect,
        steps,
        quality,
        signal_path: Some(path),
    }
}

/// Turns decoded PCM into the device's container, applying a software
/// gain when set. With no gain the samples pass unchanged; DSD (`Raw`) is
/// never scaled.
fn convert(pcm: &[u8], input: InputFormat, container: SampleFormat, gain: Option<f64>) -> Vec<u8> {
    match input {
        InputFormat::Raw => pcm.to_vec(),
        InputFormat::S16 => match gain {
            None => pcm.to_vec(),
            Some(gain) => pcm
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|sample| {
                    let value = f64::from(i16::from_le_bytes(*sample)) * gain;
                    (value.round().clamp(-32_768.0, 32_767.0) as i16).to_le_bytes()
                })
                .collect(),
        },
        InputFormat::S32 => {
            let scaled;
            let words = match gain {
                None => pcm,
                Some(gain) => {
                    scaled = pcm
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .flat_map(|sample| {
                            let value = f64::from(i32::from_le_bytes(*sample)) * gain;
                            (value
                                .round()
                                .clamp(f64::from(i32::MIN), f64::from(i32::MAX))
                                as i32)
                                .to_le_bytes()
                        })
                        .collect::<Vec<u8>>();
                    &scaled
                }
            };
            let mut out = Vec::with_capacity(words.len());
            parson_output::pack_samples(words, container, &mut out);
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::convert;
    use crate::outputs::device::SampleFormat;
    use parson_output::player::InputFormat;

    #[test]
    fn conversion_keeps_samples_unless_a_gain_is_set() {
        let s32: Vec<u8> = [0x0012_3400_i32, -0x0100_0000]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        assert_eq!(
            convert(&s32, InputFormat::S32, SampleFormat::S32, None),
            s32
        );
        assert_eq!(
            convert(&s32, InputFormat::S32, SampleFormat::S24Packed, None),
            vec![0x34, 0x12, 0x00, 0x00, 0x00, 0xff]
        );
        let s16: Vec<u8> = [1000_i16, -1000]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(
            convert(&s16, InputFormat::S16, SampleFormat::S16, None),
            s16
        );
        let halved = convert(&s16, InputFormat::S16, SampleFormat::S16, Some(0.5));
        assert_eq!(i16::from_le_bytes([halved[0], halved[1]]), 500);
        assert_eq!(i16::from_le_bytes([halved[2], halved[3]]), -500);
        let dsd = vec![0x69_u8; 8];
        assert_eq!(
            convert(&dsd, InputFormat::Raw, SampleFormat::DsdU32Be, Some(0.5)),
            dsd,
            "DSD is never scaled"
        );
    }
}
