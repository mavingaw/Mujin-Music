//! Real-time playback of decoded PCM to a [`Sink`] on a dedicated thread.

use std::io::{self, Read};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde::Serialize;

use crate::{SampleFormat, Sink, StreamFormat, pack_samples};

/// Layout of the PCM the decoder hands the player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputFormat {
    /// Interleaved signed 16-bit little endian, copied as-is (the decoder
    /// already dithered if it reduced depth).
    S16,
    /// Interleaved left-justified signed 32-bit little endian, packed into
    /// the device's container.
    S32,
    /// Already in the device's container (DSD, native or DoP), copied
    /// as-is.
    Raw,
}

impl InputFormat {
    /// What the decoder should produce for a device container.
    pub fn for_container(format: SampleFormat) -> Self {
        if format == SampleFormat::S16 {
            Self::S16
        } else {
            Self::S32
        }
    }

    fn bytes(self, container: SampleFormat) -> usize {
        match self {
            Self::S16 => 2,
            Self::S32 => 4,
            Self::Raw => container.bytes(),
        }
    }
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlayerState {
    Starting,
    Playing,
    Paused,
    Ended,
    Stopped,
    Failed,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct PlayerStatus {
    pub state: PlayerState,
    /// What is being heard now, in milliseconds from the start of the track.
    pub position_ms: u64,
    pub error: Option<String>,
}

pub type PcmSource = Box<dyn Read + Send>;
/// Opens the decoded track at a position in milliseconds.
pub type PcmOpener = Box<dyn FnMut(u64) -> io::Result<PcmSource> + Send>;
/// Opens the device; runs on the playback thread (COM and CoreAudio state
/// stay on the thread that uses them).
pub type SinkOpener = Box<dyn FnOnce() -> io::Result<Box<dyn Sink>> + Send>;

enum Command {
    Pause,
    Resume,
    Seek(u64),
    Stop,
}

/// One track playing on one device. Dropping it stops playback and closes
/// the device.
pub struct Player {
    commands: Sender<Command>,
    status: Arc<Mutex<PlayerStatus>>,
    thread: Option<JoinHandle<()>>,
}

const BLOCK_FRAMES: usize = 4096;

impl Player {
    /// Opens the device and starts playing at `start_ms`. Returns once the
    /// device is open, so an open failure (busy, unsupported format) is
    /// reported here rather than later.
    pub fn start(
        open_sink: SinkOpener,
        mut open_pcm: PcmOpener,
        stream: StreamFormat,
        input: InputFormat,
        start_ms: u64,
    ) -> io::Result<Self> {
        let status = Arc::new(Mutex::new(PlayerStatus {
            state: PlayerState::Starting,
            position_ms: start_ms,
            error: None,
        }));
        let (commands, receiver) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel::<io::Result<()>>();
        let thread_status = Arc::clone(&status);
        let thread = std::thread::Builder::new()
            .name("parson-native-output".into())
            .spawn(move || {
                let opened = open_sink().and_then(|sink| open_pcm(start_ms).map(|pcm| (sink, pcm)));
                let (sink, pcm) = match opened {
                    Ok(opened) => {
                        let _ = ready_tx.send(Ok(()));
                        opened
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                let mut engine = Engine {
                    sink,
                    pcm,
                    open_pcm,
                    stream,
                    input,
                    base_ms: start_ms,
                    frames: 0,
                    status: thread_status,
                };
                engine.run(&receiver);
            })?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                commands,
                status,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = thread.join();
                Err(io::Error::other(
                    "the playback thread stopped while opening the device",
                ))
            }
        }
    }

    pub fn status(&self) -> PlayerStatus {
        self.status
            .lock()
            .map(|status| status.clone())
            .unwrap_or(PlayerStatus {
                state: PlayerState::Failed,
                position_ms: 0,
                error: Some("player state is unavailable".into()),
            })
    }

    pub fn pause(&self) {
        let _ = self.commands.send(Command::Pause);
    }

    pub fn resume(&self) {
        let _ = self.commands.send(Command::Resume);
    }

    pub fn seek(&self, position_ms: u64) {
        let _ = self.commands.send(Command::Seek(position_ms));
    }

    /// Stops playback and waits for the device to close.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        let _ = self.commands.send(Command::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct Engine {
    sink: Box<dyn Sink>,
    pcm: PcmSource,
    open_pcm: PcmOpener,
    stream: StreamFormat,
    input: InputFormat,
    base_ms: u64,
    frames: u64,
    status: Arc<Mutex<PlayerStatus>>,
}

enum Flow {
    Continue,
    Exit,
}

impl Engine {
    fn set(&self, state: PlayerState, position_ms: u64, error: Option<String>) {
        if let Ok(mut status) = self.status.lock() {
            status.state = state;
            status.position_ms = position_ms;
            status.error = error;
        }
    }

    fn heard_ms(&mut self) -> u64 {
        let delay = self.sink.delay_frames();
        let heard = self.frames.saturating_sub(delay);
        self.base_ms + heard * 1000 / u64::from(self.stream.sample_rate.max(1))
    }

    fn fail(&mut self, error: &io::Error) {
        let position = self.heard_ms();
        let _ = self.sink.flush();
        self.set(PlayerState::Failed, position, Some(error.to_string()));
    }

    /// Restarts decoding at `position_ms` after discarding buffered audio.
    fn reposition(&mut self, position_ms: u64) -> Flow {
        let _ = self.sink.flush();
        match (self.open_pcm)(position_ms) {
            Ok(pcm) => {
                self.pcm = pcm;
                self.base_ms = position_ms;
                self.frames = 0;
                Flow::Continue
            }
            Err(error) => {
                self.base_ms = position_ms;
                self.frames = 0;
                self.fail(&error);
                Flow::Exit
            }
        }
    }

    fn handle(&mut self, command: Command, receiver: &Receiver<Command>) -> Flow {
        match command {
            Command::Stop => {
                let position = self.heard_ms();
                let _ = self.sink.flush();
                self.set(PlayerState::Stopped, position, None);
                Flow::Exit
            }
            Command::Seek(position) => {
                let flow = self.reposition(position);
                self.set(PlayerState::Playing, position, None);
                flow
            }
            Command::Resume => Flow::Continue,
            Command::Pause => {
                let position = self.heard_ms();
                let _ = self.sink.flush();
                self.set(PlayerState::Paused, position, None);
                let mut resume_at = position;
                loop {
                    match receiver.recv() {
                        Ok(Command::Resume) => break,
                        Ok(Command::Seek(next)) => {
                            resume_at = next;
                            self.set(PlayerState::Paused, next, None);
                        }
                        Ok(Command::Pause) => {}
                        Ok(Command::Stop) | Err(_) => {
                            self.set(PlayerState::Stopped, resume_at, None);
                            return Flow::Exit;
                        }
                    }
                }
                let flow = self.reposition(resume_at);
                self.set(PlayerState::Playing, resume_at, None);
                flow
            }
        }
    }

    fn run(&mut self, receiver: &Receiver<Command>) {
        let channels = usize::from(self.stream.channels.max(1));
        let input_frame = self.input.bytes(self.stream.sample_format) * channels;
        let mut block = vec![0_u8; BLOCK_FRAMES * input_frame];
        let mut packed = Vec::with_capacity(BLOCK_FRAMES * self.stream.frame_bytes());
        self.set(PlayerState::Playing, self.base_ms, None);
        loop {
            match receiver.try_recv() {
                Ok(command) => {
                    if let Flow::Exit = self.handle(command, receiver) {
                        return;
                    }
                    continue;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    let _ = self.sink.flush();
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            let filled = match read_full(&mut self.pcm, &mut block) {
                Ok(filled) => filled,
                Err(error) => {
                    self.fail(&error);
                    return;
                }
            };
            let whole = filled / input_frame * input_frame;
            if whole > 0 {
                let data: &[u8] = match self.input {
                    InputFormat::S16 | InputFormat::Raw => &block[..whole],
                    InputFormat::S32 => {
                        pack_samples(&block[..whole], self.stream.sample_format, &mut packed);
                        &packed
                    }
                };
                if let Err(error) = self.sink.write(data) {
                    self.fail(&error);
                    return;
                }
                self.frames += (whole / input_frame) as u64;
                let position = self.heard_ms();
                self.set(PlayerState::Playing, position, None);
            }
            if filled < block.len() {
                // End of track: let the device play out, but stay responsive
                // to stop while it drains.
                let _ = self.sink.drain();
                let position = self.heard_ms();
                self.set(PlayerState::Ended, position, None);
                // Keep the thread alive briefly so a late seek still works.
                match receiver.recv_timeout(Duration::from_millis(50)) {
                    Ok(Command::Seek(position)) => {
                        if let Flow::Exit = self.reposition(position) {
                            return;
                        }
                        self.set(PlayerState::Playing, position, None);
                        continue;
                    }
                    Ok(_) | Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                        return;
                    }
                }
            }
        }
    }
}

fn read_full(reader: &mut PcmSource, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulated::{CaptureLog, CaptureSink};
    use std::io::Cursor;
    use std::time::Instant;

    fn stereo_s32(frames: usize) -> Vec<u8> {
        (0..frames * 2)
            .flat_map(|index| ((index as i32) << 8).to_le_bytes())
            .collect()
    }

    fn wait_for(player: &Player, wanted: PlayerState) -> PlayerStatus {
        let started = Instant::now();
        loop {
            let status = player.status();
            if status.state == wanted || started.elapsed() > Duration::from_secs(5) {
                return status;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn format() -> StreamFormat {
        StreamFormat {
            sample_rate: 48_000,
            sample_format: SampleFormat::S24Packed,
            channels: 2,
        }
    }

    #[test]
    fn plays_a_track_to_the_end_with_exact_samples() {
        let log = CaptureLog::default();
        let sink_log = log.clone();
        let pcm = stereo_s32(10_000);
        let source = pcm.clone();
        let player = Player::start(
            Box::new(move || Ok(Box::new(CaptureSink::new(sink_log, false)) as Box<dyn Sink>)),
            Box::new(move |_| Ok(Box::new(Cursor::new(source.clone())) as PcmSource)),
            format(),
            InputFormat::S32,
            0,
        )
        .unwrap();
        let status = wait_for(&player, PlayerState::Ended);
        assert_eq!(status.state, PlayerState::Ended);
        let mut expected = Vec::new();
        pack_samples(&pcm, SampleFormat::S24Packed, &mut expected);
        assert_eq!(log.bytes(), expected);
        assert_eq!(status.position_ms, 10_000 * 1000 / 48_000);
    }

    #[test]
    fn seek_and_pause_reopen_the_decoder_at_the_heard_position() {
        let log = CaptureLog::default();
        let sink_log = log.clone();
        let opened = Arc::new(Mutex::new(Vec::new()));
        let opened_log = Arc::clone(&opened);
        let player = Player::start(
            Box::new(move || Ok(Box::new(CaptureSink::new(sink_log, true)) as Box<dyn Sink>)),
            Box::new(move |position| {
                opened_log.lock().unwrap().push(position);
                // Endless silence, so the player keeps running.
                Ok(Box::new(io::repeat(0)) as PcmSource)
            }),
            format(),
            InputFormat::S32,
            1_000,
        )
        .unwrap();
        player.seek(30_000);
        std::thread::sleep(Duration::from_millis(30));
        player.pause();
        let paused = wait_for(&player, PlayerState::Paused);
        assert!(paused.position_ms >= 30_000, "{paused:?}");
        player.resume();
        let playing = wait_for(&player, PlayerState::Playing);
        assert!(playing.position_ms >= paused.position_ms);
        player.stop();
        let opened = opened.lock().unwrap();
        assert_eq!(opened[0], 1_000);
        assert_eq!(opened[1], 30_000);
        assert_eq!(opened[2], paused.position_ms);
    }

    #[test]
    fn open_failures_are_returned_from_start() {
        let result = Player::start(
            Box::new(|| Err(io::Error::other("device busy"))),
            Box::new(|_| Ok(Box::new(io::empty()) as PcmSource)),
            format(),
            InputFormat::S32,
            0,
        );
        assert_eq!(result.err().unwrap().to_string(), "device busy");
    }

    #[test]
    fn decoder_errors_fail_the_player() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("decoder crashed"))
            }
        }
        let log = CaptureLog::default();
        let player = Player::start(
            Box::new(move || Ok(Box::new(CaptureSink::new(log, false)) as Box<dyn Sink>)),
            Box::new(|_| Ok(Box::new(Broken) as PcmSource)),
            format(),
            InputFormat::S32,
            0,
        )
        .unwrap();
        let status = wait_for(&player, PlayerState::Failed);
        assert_eq!(status.error.as_deref(), Some("decoder crashed"));
    }
}
