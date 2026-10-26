//! The playback thread of one audio device.
//!
//! ffmpeg decodes each track to raw PCM at the format chosen for the
//! device (the file's own rate and depth when the device takes it, so
//! nothing is resampled or requantised). The thread writes frames to the
//! device, starts the next track on the same open stream for gapless
//! playback, and reports position, format and signal path.

use std::io::Read;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{DeviceBackend, DeviceInfo, OutputStream, SampleFormat, StreamFormat, choose_format};
use crate::outputs::renderer::{PlayState, RendererSettings, RendererState, TrackRequest};
use crate::outputs::{describe_source, format_rate};
use crate::zones::{VolumeMode, ZoneStreamInfo};

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
    /// After the device name, e.g. `on kitchen-pi via Parson Bridge`.
    pub suffix: Option<String>,
    /// A step every stream takes.
    pub step: Option<String>,
    /// The output has no exclusive/shared mode (AirPlay).
    pub modeless: bool,
}

struct Shared {
    state: RendererState,
    settings: RendererSettings,
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
        backend: Arc<dyn DeviceBackend>,
        device: DeviceInfo,
        changed: tokio::sync::broadcast::Sender<()>,
        route: Route,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        let settings = RendererSettings {
            exclusive: true,
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
        }));
        let thread_shared = shared.clone();
        let name = format!("parson-audio-{}", device.id);
        let spawned = std::thread::Builder::new().name(name).spawn(move || {
            Player {
                backend,
                device,
                route,
                receiver,
                shared: thread_shared,
                changed,
                current: None,
                next: None,
                stream: None,
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

    pub fn state(&self) -> RendererState {
        lock(&self.shared).state.clone()
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

struct Decoder {
    child: Child,
    stdout: ChildStdout,
    /// Bytes of a frame ffmpeg writes (s16 or s32 per channel).
    frame_bytes: usize,
    pending: Vec<u8>,
}

impl Decoder {
    /// Starts ffmpeg for `track` at `position_ms`, producing `format`.
    fn start(
        track: &TrackRequest,
        position_ms: i64,
        format: StreamFormat,
        exact: bool,
    ) -> Result<Self, String> {
        let wide = format.format != SampleFormat::S16;
        let mut command = Command::new("ffmpeg");
        command.args(["-nostdin", "-hide_banner", "-v", "error"]);
        if position_ms > 0 {
            command
                .arg("-ss")
                .arg(format!("{:.3}", position_ms as f64 / 1_000.0));
        }
        command.arg("-i").arg(&track.path);
        command.args(["-map", "0:a:0", "-vn"]);
        let mut filters = Vec::new();
        if let Some(gain) = track.gain_db.filter(|gain| gain.abs() >= 0.05) {
            filters.push(format!("volume={gain:.2}dB"));
        }
        if let Some(dsp) = track.dsp.ffmpeg_filter() {
            filters.push(dsp);
        }
        if !filters.is_empty() {
            command.arg("-af").arg(filters.join(","));
        }
        if !exact || !filters.is_empty() {
            command
                .arg("-ar")
                .arg(format.sample_rate.to_string())
                .arg("-ac")
                .arg(format.channels.to_string());
        }
        if wide {
            command.args(["-c:a", "pcm_s32le", "-f", "s32le"]);
        } else {
            command.args(["-c:a", "pcm_s16le", "-f", "s16le"]);
        }
        command
            .arg("pipe:1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command
            .spawn()
            .map_err(|error| format!("could not start the decoder: {error}"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "decoder has no output".to_string())?;
        Ok(Self {
            child,
            stdout,
            frame_bytes: (if wide { 4 } else { 2 }) * usize::from(format.channels),
            pending: Vec::new(),
        })
    }

    /// Reads up to `frames` whole frames; an empty result is the end.
    fn read(&mut self, frames: usize) -> Vec<u8> {
        let wanted = frames * self.frame_bytes;
        let mut buffer = std::mem::take(&mut self.pending);
        let mut chunk = vec![0_u8; wanted.saturating_sub(buffer.len()).max(self.frame_bytes)];
        while buffer.len() < wanted {
            match self.stdout.read(&mut chunk) {
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

impl Drop for Decoder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Active {
    track: TrackRequest,
    decoder: Decoder,
    format: StreamFormat,
    exact: bool,
    /// Where the decoder started, in the track.
    start_ms: i64,
    frames_written: u64,
    /// The stream was dropped on pause: restart here on play.
    resume_at: Option<i64>,
}

struct Player {
    backend: Arc<dyn DeviceBackend>,
    device: DeviceInfo,
    route: Route,
    receiver: Receiver<Control>,
    shared: Arc<Mutex<Shared>>,
    changed: tokio::sync::broadcast::Sender<()>,
    current: Option<Active>,
    next: Option<TrackRequest>,
    stream: Option<Box<dyn OutputStream>>,
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
                    stream.drop_buffer();
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
                            stream.drop_buffer();
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
                drop(shared);
                self.notify();
            }
            Control::Seek(position_ms) => {
                if let Some(active) = self.current.as_ref() {
                    let track = active.track.clone();
                    if let Some(stream) = self.stream.as_mut() {
                        stream.drop_buffer();
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
                let reopen = settings.exclusive != self.settings().exclusive;
                lock(&self.shared).settings = settings;
                if settings.volume_mode != VolumeMode::Software {
                    self.software_volume = 100;
                }
                if reopen {
                    // Reopen in the new mode at the heard position.
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

    /// Starts decoding `track` at `position_ms`.
    fn begin(&mut self, track: TrackRequest, position_ms: i64) {
        let settings = self.settings();
        let (format, exact) = choose_format(&self.device, &track.source, settings.limits);
        match Decoder::start(&track, position_ms, format, exact) {
            Ok(decoder) => {
                self.current = Some(Active {
                    track: track.clone(),
                    decoder,
                    format,
                    exact,
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
                }
                self.refresh_stream_info();
                self.notify();
            }
            Err(error) => {
                tracing::warn!(%error, device = self.device.id, "could not decode track");
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
        active.start_ms + (heard as i64 * 1_000) / i64::from(active.format.sample_rate.max(1))
    }

    fn set_state(&self, state: PlayState) {
        lock(&self.shared).state.state = state;
        self.notify();
    }

    fn close_stream(&mut self, drain: bool) {
        if let Some(mut stream) = self.stream.take() {
            if drain {
                stream.drain();
            } else {
                stream.drop_buffer();
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
        drop(shared);
        self.notify();
    }

    fn refresh_stream_info(&self) {
        let info = self.current.as_ref().map(|active| {
            let settings = self.settings();
            let mut steps = Vec::new();
            let source_rate = active
                .track
                .source
                .sample_rate
                .unwrap_or(active.format.sample_rate);
            if source_rate != active.format.sample_rate {
                steps.push(format!(
                    "Resampled {} to {} (the device does not take {})",
                    format_rate(source_rate),
                    format_rate(active.format.sample_rate),
                    format_rate(source_rate)
                ));
            }
            if let Some(bits) = active.track.source.bit_depth
                && active.track.source.lossless()
                && bits > active.format.format.bits()
            {
                steps.push(format!(
                    "Reduced to {}-bit (the device takes at most {}-bit)",
                    active.format.format.bits(),
                    active.format.format.bits()
                ));
            }
            if !active.track.source.lossless() {
                steps.push("Decoded from a lossy file".to_string());
            }
            if let Some(gain) = active.track.gain_db.filter(|gain| gain.abs() >= 0.05) {
                steps.push(format!("Volume leveling {gain:+.1} dB"));
            }
            if active.track.dsp.is_active() {
                steps.push(format!("DSP: {}", active.track.dsp.summary()));
            }
            if let Some(step) = &self.route.step {
                steps.push(step.clone());
            }
            let software = settings.volume_mode == VolumeMode::Software
                && (self.software_volume < 100 || self.muted);
            if software {
                steps.push("Software volume (Parson scales the samples)".to_string());
            }
            let bit_perfect =
                active.exact && steps.is_empty() && settings.volume_mode != VolumeMode::Software;
            let reduced = active
                .track
                .source
                .bit_depth
                .is_some_and(|bits| bits > active.format.format.bits());
            let quality = crate::zones::StreamQuality::rate(
                active.track.source.lossless(),
                source_rate != active.format.sample_rate || reduced || software,
                active.track.gain_db.is_some_and(|gain| gain.abs() >= 0.05)
                    || active.track.dsp.is_active(),
            );
            ZoneStreamInfo {
                source: describe_source(&active.track.source),
                output: format!(
                    "{}/{}-bit ({}) to {}{}{}",
                    format_rate(active.format.sample_rate),
                    active.format.format.bits(),
                    active.format.format.name(),
                    self.device.name,
                    self.route
                        .suffix
                        .as_ref()
                        .map(|suffix| format!(" {suffix}"))
                        .unwrap_or_default(),
                    match (self.route.modeless, settings.exclusive) {
                        (true, _) => "",
                        (false, true) => ", exclusive",
                        (false, false) => ", shared",
                    }
                ),
                bit_perfect,
                steps,
                quality,
            }
        });
        lock(&self.shared).state.stream = info;
    }

    /// Decodes and writes one chunk, moving to the next track at the end.
    fn pump(&mut self) {
        let Some(active) = self.current.as_mut() else {
            return;
        };
        let pcm = active.decoder.read(CHUNK_FRAMES);
        let frame_bytes = active.decoder.frame_bytes;
        let format = active.format;
        if pcm.is_empty() {
            self.track_ended();
            return;
        }
        if self.stream.as_ref().map(|stream| stream.format()) != Some(format) {
            // New format (or first track): drain the old stream, reopen.
            self.close_stream(true);
            match self
                .backend
                .open(&self.device.id, format, self.settings().exclusive)
            {
                Ok(stream) => self.stream = Some(stream),
                Err(error) => {
                    tracing::warn!(%error, device = self.device.id, "could not open audio device");
                    // Stop rather than skip through the queue.
                    self.current = None;
                    self.close_stream(false);
                    let mut shared = lock(&self.shared);
                    shared.state.reachable = false;
                    shared.state.state = PlayState::Stopped;
                    shared.state.stream = None;
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
        let data = convert(&pcm, format.format, gain);
        let Some(stream) = self.stream.as_mut() else {
            return;
        };
        if let Err(error) = stream.write(&data) {
            tracing::warn!(%error, device = self.device.id, "audio device write failed");
            self.close_stream(false);
            let mut shared = lock(&self.shared);
            if error.contains(parson_bridge::endpoint::REPLACED) {
                // Another zone (or a synced group) now plays this DAC: stop
                // instead of taking it back, so the two never fight.
                self.current = None;
                shared.state.state = PlayState::Stopped;
                shared.state.position_ms = 0;
                shared.state.stream = None;
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

/// Turns decoder output (s16 or s32) into the device format, applying a
/// software gain when set. With no gain the samples pass unchanged.
fn convert(pcm: &[u8], format: SampleFormat, gain: Option<f64>) -> Vec<u8> {
    match format {
        SampleFormat::S16 => match gain {
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
        SampleFormat::S32 | SampleFormat::S24Packed => {
            let mut out = Vec::with_capacity(pcm.len() / 4 * format.bytes());
            for sample in pcm.as_chunks::<4>().0 {
                let mut value = i32::from_le_bytes(*sample);
                if let Some(gain) = gain {
                    value = (f64::from(value) * gain)
                        .round()
                        .clamp(f64::from(i32::MIN), f64::from(i32::MAX))
                        as i32;
                }
                let bytes = value.to_le_bytes();
                if format == SampleFormat::S24Packed {
                    // The top 24 bits; 24-bit sources leave the low byte 0.
                    out.extend_from_slice(&bytes[1..4]);
                } else {
                    out.extend_from_slice(&bytes);
                }
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::convert;
    use crate::outputs::device::SampleFormat;

    #[test]
    fn conversion_keeps_samples_unless_a_gain_is_set() {
        let s32: Vec<u8> = [0x0012_3400_i32, -0x0100_0000]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        assert_eq!(convert(&s32, SampleFormat::S32, None), s32);
        assert_eq!(
            convert(&s32, SampleFormat::S24Packed, None),
            vec![0x34, 0x12, 0x00, 0x00, 0x00, 0xff]
        );
        let s16: Vec<u8> = [1000_i16, -1000]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(convert(&s16, SampleFormat::S16, None), s16);
        let halved = convert(&s16, SampleFormat::S16, Some(0.5));
        assert_eq!(i16::from_le_bytes([halved[0], halved[1]]), 500);
        assert_eq!(i16::from_le_bytes([halved[2], halved[3]]), -500);
    }
}
