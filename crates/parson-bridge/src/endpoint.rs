//! The bridge side: listens for the Parson server, lists this machine's
//! audio devices and plays the PCM the server sends on them, exactly as
//! sent, in exclusive mode at the stream's own rate and depth.
//!
//! A stream in a sample-synced group starts at a set time: the bridge
//! primes the device with silence so that the first music frame is heard
//! exactly then. The clock master then just plays; a follower compares
//! what it has heard with the master's timeline (sent by the server) and
//! repeats or skips single frames to stay within a quarter millisecond.

use std::collections::{HashMap, VecDeque};
use std::io::{BufReader, BufWriter, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::audio::{DeviceBackend, DeviceInfo, OutputStream, StreamFormat};
use crate::clock::{Correction, drift_correction, monotonic_us};
use crate::protocol::{
    BridgeMessage, Frame, PROTOCOL_VERSION, ServerMessage, SyncStart, read_frame, write_message,
};

/// Frames per device write.
const CHUNK_FRAMES: usize = 1_024;
const STATUS_INTERVAL: Duration = Duration::from_millis(50);
/// Why a stream ended when another one took its device.
pub const REPLACED: &str = "another stream took over this device";
const DEVICE_POLL: Duration = Duration::from_secs(3);

#[derive(Clone, Debug)]
pub struct EndpointConfig {
    pub bridge_id: String,
    pub name: String,
    pub host: String,
    /// Added to this bridge's clock (tests prove sync across clocks that
    /// disagree by seconds).
    pub clock_offset_us: i64,
    /// Why there are no devices (no supported audio API), shown in Parson.
    pub note: Option<String>,
}

impl EndpointConfig {
    pub fn now_us(&self) -> i64 {
        monotonic_us() + self.clock_offset_us
    }
}

type Writer = Arc<Mutex<BufWriter<TcpStream>>>;

/// The server this bridge plays for. One server at a time: another one is
/// refused while it is active, so two Parson servers on one network do not
/// take the bridge from each other; the same server reconnecting (after a
/// restart) replaces its old session.
struct Active {
    server_id: String,
    session: u64,
    socket: TcpStream,
    last_seen: Arc<Mutex<std::time::Instant>>,
}

/// A server silent for this long has gone.
const ACTIVE_TIMEOUT: Duration = Duration::from_secs(15);
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

fn send(writer: &Writer, message: &BridgeMessage) -> bool {
    let mut writer = writer
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    write_message(&mut *writer, message).is_ok() && writer.flush().is_ok()
}

/// A running bridge endpoint.
pub struct Endpoint {
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    sessions: Arc<AtomicU64>,
}

impl Endpoint {
    /// Serves `listener` on a background thread.
    pub fn start(
        listener: TcpListener,
        backend: Option<Arc<dyn DeviceBackend>>,
        config: EndpointConfig,
    ) -> std::io::Result<Self> {
        let address = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let sessions = Arc::new(AtomicU64::new(0));
        let thread_stop = stop.clone();
        let thread_sessions = sessions.clone();
        std::thread::Builder::new()
            .name("parson-bridge-accept".into())
            .spawn(move || {
                let current: Arc<Mutex<Option<Active>>> = Arc::new(Mutex::new(None));
                for connection in listener.incoming() {
                    if thread_stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let Ok(connection) = connection else {
                        continue;
                    };
                    let _ = connection.set_nodelay(true);
                    thread_sessions.fetch_add(1, Ordering::Relaxed);
                    let backend = backend.clone();
                    let config = config.clone();
                    let current = current.clone();
                    let _ = std::thread::Builder::new()
                        .name("parson-bridge-session".into())
                        .spawn(move || {
                            let peer = connection.peer_addr().ok();
                            if let Err(error) = Session::run(connection, backend, config, current) {
                                tracing::info!(?peer, %error, "server session ended");
                            }
                        });
                }
            })?;
        Ok(Self {
            address,
            stop,
            sessions,
        })
    }

    pub fn address(&self) -> std::net::SocketAddr {
        self.address
    }

    /// Server connections accepted so far.
    pub fn sessions(&self) -> u64 {
        self.sessions.load(Ordering::Relaxed)
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wake the accept loop.
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_millis(200));
    }
}

#[derive(Default)]
struct Pending {
    queue: VecDeque<u8>,
    paused: bool,
    flush: bool,
    close: bool,
    drain: bool,
    /// Latest master timeline: (frame, bridge_us, rate).
    sync_ref: Option<(u64, i64, f64)>,
    received: u64,
}

struct StreamShared {
    pending: Mutex<Pending>,
    wake: Condvar,
}

impl StreamShared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Pending> {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

struct Session {
    backend: Option<Arc<dyn DeviceBackend>>,
    config: EndpointConfig,
    writer: Writer,
    streams: HashMap<u32, (Arc<StreamShared>, StreamFormat, String)>,
    devices: Arc<Mutex<Vec<DeviceInfo>>>,
    alive: Arc<AtomicBool>,
}

impl Session {
    fn run(
        connection: TcpStream,
        backend: Option<Arc<dyn DeviceBackend>>,
        config: EndpointConfig,
        current: Arc<Mutex<Option<Active>>>,
    ) -> std::io::Result<()> {
        let writer: Writer = Arc::new(Mutex::new(BufWriter::new(connection.try_clone()?)));
        let mut reader = BufReader::with_capacity(256 * 1024, connection.try_clone()?);
        connection.set_read_timeout(Some(Duration::from_secs(10)))?;
        let (server_id, server_name) = match read_frame::<_, ServerMessage>(&mut reader)? {
            Frame::Message(ServerMessage::Hello {
                protocol,
                server_id,
                server_name,
            }) => {
                if protocol != PROTOCOL_VERSION {
                    send(
                        &writer,
                        &BridgeMessage::Refused {
                            reason: format!(
                                "this bridge speaks protocol {PROTOCOL_VERSION}, the server {protocol}; update the older one"
                            ),
                        },
                    );
                    return Err(std::io::Error::other("protocol mismatch"));
                }
                (server_id, server_name)
            }
            _ => return Err(std::io::Error::other("expected hello")),
        };
        let session_id = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
        let last_seen = Arc::new(Mutex::new(std::time::Instant::now()));
        {
            let mut active = current
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(existing) = active.as_ref()
                && existing.server_id != server_id
                && existing
                    .last_seen
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .elapsed()
                    < ACTIVE_TIMEOUT
            {
                send(
                    &writer,
                    &BridgeMessage::Refused {
                        reason: "this bridge is in use by another Parson server".into(),
                    },
                );
                return Err(std::io::Error::other("in use by another server"));
            }
            if let Some(old) = active.replace(Active {
                server_id,
                session: session_id,
                socket: connection.try_clone()?,
                last_seen: last_seen.clone(),
            }) {
                let _ = old.socket.shutdown(Shutdown::Both);
            }
        }
        tracing::info!(%server_name, "Parson server connected");
        // The server pings every second.
        connection.set_read_timeout(Some(Duration::from_secs(15)))?;
        let devices = backend
            .as_ref()
            .map(|backend| backend.devices())
            .unwrap_or_default();
        send(
            &writer,
            &BridgeMessage::Welcome {
                protocol: PROTOCOL_VERSION,
                bridge_id: config.bridge_id.clone(),
                name: config.name.clone(),
                host: config.host.clone(),
                platform: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
                version: env!("CARGO_PKG_VERSION").to_string(),
                backend: backend.as_ref().map(|backend| backend.name().to_string()),
                note: config.note.clone(),
                devices: devices.clone(),
            },
        );
        let mut session = Session {
            backend,
            config,
            writer,
            streams: HashMap::new(),
            devices: Arc::new(Mutex::new(devices)),
            alive: Arc::new(AtomicBool::new(true)),
        };
        session.watch_devices();
        let result = loop {
            let frame = read_frame::<_, ServerMessage>(&mut reader);
            *last_seen
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = std::time::Instant::now();
            match frame {
                Ok(Frame::Message(message)) => session.handle(message),
                Ok(Frame::Audio { stream, data }) => session.audio(stream, data),
                Err(error) => break Err(error),
            }
        };
        session.alive.store(false, Ordering::Relaxed);
        {
            let mut active = current
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if active
                .as_ref()
                .is_some_and(|active| active.session == session_id)
            {
                *active = None;
            }
        }
        for (shared, _, _) in session.streams.values() {
            shared.lock().close = true;
            shared.wake.notify_all();
        }
        let _ = connection.shutdown(Shutdown::Both);
        result
    }

    fn watch_devices(&self) {
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let devices = self.devices.clone();
        let alive = self.alive.clone();
        let writer = self.writer.clone();
        let _ = std::thread::Builder::new()
            .name("parson-bridge-devices".into())
            .spawn(move || {
                while alive.load(Ordering::Relaxed) {
                    std::thread::sleep(DEVICE_POLL);
                    let now = backend.devices();
                    let mut known = devices
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if *known != now {
                        *known = now.clone();
                        drop(known);
                        if !send(&writer, &BridgeMessage::Devices { devices: now }) {
                            return;
                        }
                    }
                }
            });
    }

    fn audio(&mut self, stream: u32, data: Vec<u8>) {
        let Some((shared, format, _)) = self.streams.get(&stream) else {
            return;
        };
        let frame_bytes = format.frame_bytes();
        let mut pending = shared.lock();
        pending.received += (data.len() / frame_bytes) as u64;
        pending.queue.extend(data);
        drop(pending);
        shared.wake.notify_all();
    }

    fn with_stream(&self, stream: u32, change: impl FnOnce(&mut Pending)) {
        if let Some((shared, _, _)) = self.streams.get(&stream) {
            change(&mut shared.lock());
            shared.wake.notify_all();
        }
    }

    fn handle(&mut self, message: ServerMessage) {
        match message {
            ServerMessage::Hello { .. } => {}
            ServerMessage::Ping { id, server_us } => {
                send(
                    &self.writer,
                    &BridgeMessage::Pong {
                        id,
                        server_us,
                        bridge_us: self.config.now_us(),
                    },
                );
            }
            ServerMessage::Open {
                stream,
                device,
                format,
                exclusive,
                sync,
            } => self.open(stream, device, format, exclusive, sync),
            ServerMessage::Pause { stream, paused } => {
                self.with_stream(stream, |pending| pending.paused = paused)
            }
            ServerMessage::Flush { stream } => self.with_stream(stream, |pending| {
                pending.flush = true;
                pending.queue.clear();
            }),
            ServerMessage::Drain { stream } => {
                self.with_stream(stream, |pending| pending.drain = true)
            }
            ServerMessage::Close { stream } => {
                self.with_stream(stream, |pending| pending.close = true);
                self.streams.remove(&stream);
            }
            ServerMessage::SyncRef {
                stream,
                frame,
                bridge_us,
                rate,
            } => self.with_stream(stream, |pending| {
                pending.sync_ref = Some((frame, bridge_us, rate));
            }),
            ServerMessage::Volume { device, db } => {
                if let Some(backend) = &self.backend
                    && let Err(error) = backend.set_hardware_volume(&device, db)
                {
                    send(&self.writer, &BridgeMessage::DeviceError { device, error });
                }
            }
            ServerMessage::Mute { device, muted } => {
                if let Some(backend) = &self.backend
                    && let Err(error) = backend.set_hardware_mute(&device, muted)
                {
                    send(&self.writer, &BridgeMessage::DeviceError { device, error });
                }
            }
        }
    }

    fn open(
        &mut self,
        stream: u32,
        device: String,
        format: StreamFormat,
        exclusive: bool,
        sync: Option<SyncStart>,
    ) {
        let Some(backend) = self.backend.clone() else {
            send(
                &self.writer,
                &BridgeMessage::Failed {
                    stream,
                    error: "this bridge has no audio API".into(),
                },
            );
            return;
        };
        // One stream per device: a new one replaces the old. The server is
        // told, so whatever still feeds the old stream fails at once
        // instead of waiting for audio that will never be played.
        let writer = self.writer.clone();
        self.streams.retain(|old, (shared, _, open_device)| {
            if *open_device == device {
                shared.lock().close = true;
                shared.wake.notify_all();
                send(
                    &writer,
                    &BridgeMessage::Failed {
                        stream: *old,
                        error: REPLACED.to_string(),
                    },
                );
                false
            } else {
                true
            }
        });
        let shared = Arc::new(StreamShared {
            pending: Mutex::new(Pending::default()),
            wake: Condvar::new(),
        });
        self.streams
            .insert(stream, (shared.clone(), format, device.clone()));
        let writer = self.writer.clone();
        let config = self.config.clone();
        let _ = std::thread::Builder::new()
            .name(format!("parson-bridge-play-{stream}"))
            .spawn(move || {
                let output = match backend.open(&device, format, exclusive) {
                    Ok(output) => output,
                    Err(error) => {
                        send(&writer, &BridgeMessage::Failed { stream, error });
                        return;
                    }
                };
                send(&writer, &BridgeMessage::Opened { stream });
                Player {
                    stream,
                    shared,
                    output,
                    format,
                    sync,
                    writer,
                    config,
                    started: false,
                    device_frames: 0,
                    silence_frames: 0,
                    corrections: 0,
                    heard_base: 0,
                    source_written: 0,
                    underruns: 0,
                    starved: false,
                    device_paused: false,
                }
                .run();
            });
    }
}

struct Player {
    stream: u32,
    shared: Arc<StreamShared>,
    output: Box<dyn OutputStream>,
    format: StreamFormat,
    sync: Option<SyncStart>,
    writer: Writer,
    config: EndpointConfig,
    started: bool,
    /// Frames written to the device, silence and corrections included.
    device_frames: u64,
    silence_frames: u64,
    /// Net frames inserted (+) or dropped (-) to follow the master.
    corrections: i64,
    /// Source frames counted as heard before the last flush.
    heard_base: u64,
    /// Source frames taken from the queue since the last flush.
    source_written: u64,
    underruns: u32,
    starved: bool,
    device_paused: bool,
}

impl Player {
    fn frame_bytes(&self) -> usize {
        self.format.frame_bytes()
    }

    /// Source frames heard so far.
    fn heard(&mut self) -> u64 {
        let delay = self.output.delay_frames();
        let audio = self
            .device_frames
            .saturating_sub(delay)
            .saturating_sub(self.silence_frames);
        // Inserted frames are not source frames; dropped ones count as
        // passed. Never past what was taken from the queue.
        let source = (audio as i64 - self.corrections).max(0) as u64;
        self.heard_base + source.min(self.source_written)
    }

    fn report(&mut self) -> bool {
        let heard = self.heard();
        let bridge_us = self.config.now_us();
        let received = self.shared.lock().received;
        send(
            &self.writer,
            &BridgeMessage::Status {
                stream: self.stream,
                received,
                heard,
                bridge_us,
                corrections: self.corrections,
                underruns: self.underruns,
                started: self.started,
            },
        )
    }

    fn reset_counters(&mut self, received: u64) {
        self.heard_base = received;
        self.device_frames = 0;
        self.silence_frames = 0;
        self.source_written = 0;
        self.corrections = 0;
    }

    fn run(mut self) {
        let mut last_report = std::time::Instant::now();
        loop {
            if last_report.elapsed() >= STATUS_INTERVAL {
                last_report = std::time::Instant::now();
                if !self.report() {
                    break;
                }
            }
            let frame_bytes = self.frame_bytes();
            let mut pending = self.shared.lock();
            if pending.close {
                drop(pending);
                self.output.drop_buffer();
                break;
            }
            if pending.flush {
                pending.flush = false;
                let received = pending.received;
                drop(pending);
                self.output.drop_buffer();
                self.reset_counters(received);
                self.report();
                continue;
            }
            if pending.paused != self.device_paused {
                self.device_paused = pending.paused;
                let paused = pending.paused;
                drop(pending);
                let _ = self.output.pause(paused);
                self.report();
                continue;
            }
            if pending.paused {
                drop(self.shared.wake.wait_timeout(pending, STATUS_INTERVAL));
                continue;
            }
            if pending.queue.len() < frame_bytes {
                if pending.drain {
                    pending.drain = false;
                    drop(pending);
                    self.output.drain();
                    self.report();
                    send(
                        &self.writer,
                        &BridgeMessage::Drained {
                            stream: self.stream,
                        },
                    );
                    continue;
                }
                if self.started && !self.starved {
                    self.starved = true;
                    self.underruns += 1;
                }
                drop(
                    self.shared
                        .wake
                        .wait_timeout(pending, Duration::from_millis(10)),
                );
                continue;
            }
            self.starved = false;
            if let Some(sync) = self.sync
                && !self.started
            {
                drop(pending);
                self.prime(sync);
                continue;
            }
            let available = pending.queue.len() / frame_bytes;
            let sync_ref = pending.sync_ref;
            drop(pending);
            let mut correction = Correction::None;
            if let (Some(sync), Some((frame, at_us, rate))) = (self.sync, sync_ref)
                && !sync.master
            {
                let now = self.config.now_us();
                let target = frame as f64 + (now - at_us) as f64 * rate / 1e6;
                let error = self.heard() as f64 - target;
                correction = drift_correction(error, self.format.sample_rate);
            }
            let skipped = match correction {
                Correction::Drop(frames) => {
                    (frames as usize).min(available.saturating_sub(1)) as u64
                }
                _ => 0,
            };
            let take = (available - skipped as usize).min(CHUNK_FRAMES);
            let mut chunk: Vec<u8> = {
                let mut pending = self.shared.lock();
                if pending.flush || pending.close {
                    continue;
                }
                let skip_bytes = (skipped as usize * frame_bytes).min(pending.queue.len());
                pending.queue.drain(..skip_bytes);
                let bytes = (take * frame_bytes).min(pending.queue.len());
                pending.queue.drain(..bytes).collect()
            };
            self.source_written += skipped + (chunk.len() / frame_bytes) as u64;
            if skipped > 0 {
                self.corrections -= skipped as i64;
            }
            if let Correction::Insert(frames) = correction {
                if frames == 1 && chunk.len() >= frame_bytes {
                    // Repeat the chunk's last frame.
                    let last = chunk[chunk.len() - frame_bytes..].to_vec();
                    chunk.extend_from_slice(&last);
                } else {
                    let mut silence = vec![0_u8; frames as usize * frame_bytes];
                    silence.extend_from_slice(&chunk);
                    chunk = silence;
                }
                self.corrections += frames as i64;
            }
            if chunk.is_empty() {
                continue;
            }
            if let Err(error) = self.output.write(&chunk) {
                tracing::warn!(%error, "device write failed");
                send(
                    &self.writer,
                    &BridgeMessage::Failed {
                        stream: self.stream,
                        error,
                    },
                );
                break;
            }
            self.device_frames += (chunk.len() / frame_bytes) as u64;
        }
    }

    /// Writes silence until the next frame written will be heard at the
    /// start time, then starts. Late (the start already passed): skips the
    /// frames that should have been heard by now.
    fn prime(&mut self, sync: SyncStart) {
        let rate = f64::from(self.format.sample_rate);
        let frame_bytes = self.frame_bytes();
        let now = self.config.now_us();
        let delay = self.output.delay_frames();
        let heard_at = now as f64 + delay as f64 * 1e6 / rate;
        let gap_frames = ((sync.start_bridge_us as f64 - heard_at) * rate / 1e6).round();
        if gap_frames >= 1.0 {
            // Up to 10 ms at a time, so the device keeps its own pace.
            let frames = (gap_frames as u64).min(self.format.sample_rate as u64 / 100 + 1);
            let silence = vec![0_u8; frames as usize * frame_bytes];
            if self.output.write(&silence).is_err() {
                self.started = true;
                return;
            }
            self.device_frames += frames;
            self.silence_frames += frames;
            return;
        }
        if gap_frames < 0.0 {
            let late = (-gap_frames) as u64;
            let mut pending = self.shared.lock();
            let skip = (late as usize * frame_bytes).min(pending.queue.len());
            let skip = skip - skip % frame_bytes;
            pending.queue.drain(..skip);
            drop(pending);
            self.source_written += (skip / frame_bytes) as u64;
            self.corrections -= (skip / frame_bytes) as i64;
        }
        self.started = true;
        self.report();
    }
}
