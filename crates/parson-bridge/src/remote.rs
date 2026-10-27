//! The server side of a bridge connection: [`RemoteBridge`] is a
//! [`Backend`] whose devices are another machine's, so the server's
//! playback engine (decoding at each file's own format, gapless, DSP,
//! leveling, volume modes, signal path) drives a remote DAC exactly as it
//! drives a local one. [`SyncGroup`] plays one stream on several bridges,
//! sample-synced to a clock master.

use std::collections::{HashMap, VecDeque};
use std::io::{self, BufReader, BufWriter, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use parson_output::{Backend, DeviceCaps, DeviceInfo, Driver, HardwareVolume, Sink, StreamFormat};

use crate::clock::{ClockEstimator, RateEstimator, monotonic_us};
use crate::protocol::{
    BridgeMessage, Frame, MAX_FRAME, PROTOCOL_VERSION, ServerMessage, SyncStart, WireDevice,
    read_frame, write_audio, write_message,
};

/// Audio the server keeps ahead of what a bridge has played.
const WINDOW: Duration = Duration::from_millis(750);
/// How far ahead a synced group starts, so every member has audio queued.
const SYNC_LEAD: Duration = Duration::from_millis(500);
const PING_INTERVAL: Duration = Duration::from_secs(1);
/// No message for this long: the bridge is gone.
const SILENCE_TIMEOUT: Duration = Duration::from_secs(6);
/// An open stream the bridge has not reported on for this long has ended
/// there (bridges report every 50 ms, paused or not): writes fail instead
/// of waiting forever for audio that will not be played.
const STREAM_STALL: Duration = Duration::from_secs(3);
/// How long a pause waits for the bridge to report its device stopped.
const PAUSE_ACK_TIMEOUT: Duration = Duration::from_secs(1);

/// What a bridge said about itself.
#[derive(Clone, Debug, PartialEq)]
pub struct BridgeInfo {
    pub bridge_id: String,
    pub name: String,
    pub host: String,
    pub platform: String,
    pub version: String,
    pub backend: Option<String>,
    pub note: Option<String>,
    pub address: SocketAddr,
}

#[derive(Clone, Debug, Default)]
struct StreamStatus {
    opened: Option<Result<(), String>>,
    /// When the bridge said it opened the device.
    opened_at: Option<Instant>,
    received: u64,
    heard: u64,
    /// Server time of the report.
    at_server_us: i64,
    at: Option<Instant>,
    corrections: i64,
    underruns: u32,
    started: bool,
    /// The bridge reported the device paused.
    paused: bool,
    drained: bool,
    rate: Option<RateEstimatorBox>,
}

#[derive(Clone, Debug)]
struct RateEstimatorBox(RateEstimator);

impl Default for RateEstimatorBox {
    fn default() -> Self {
        Self(RateEstimator::new(44_100))
    }
}

struct Link {
    writer: Mutex<BufWriter<TcpStream>>,
    socket: TcpStream,
    info: Mutex<BridgeInfo>,
    devices: Mutex<Vec<DeviceInfo>>,
    clock: Mutex<ClockEstimator>,
    streams: Mutex<HashMap<u32, StreamStatus>>,
    wake: Condvar,
    connected: AtomicBool,
    next_stream: AtomicU32,
    last_heard: Mutex<Instant>,
    errors: Mutex<VecDeque<String>>,
    changed: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    pings: AtomicU64,
}

impl Link {
    fn send(&self, message: &ServerMessage) -> Result<(), String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err("bridge disconnected".into());
        }
        let mut writer = self
            .writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        write_message(&mut *writer, message)
            .and_then(|()| writer.flush())
            .map_err(|error| self.fail(error.to_string()))
    }

    fn send_audio(&self, stream: u32, data: &[u8]) -> Result<(), String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err("bridge disconnected".into());
        }
        let mut writer = self
            .writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        write_audio(&mut *writer, stream, data)
            .and_then(|()| writer.flush())
            .map_err(|error| self.fail(error.to_string()))
    }

    fn fail(&self, error: String) -> String {
        if self.connected.swap(false, Ordering::Relaxed) {
            let _ = self.socket.shutdown(Shutdown::Both);
            self.wake.notify_all();
            self.notify();
        }
        error
    }

    fn notify(&self) {
        if let Some(changed) = self
            .changed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
        {
            changed();
        }
    }

    fn streams(&self) -> std::sync::MutexGuard<'_, HashMap<u32, StreamStatus>> {
        self.streams
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn clock(&self) -> ClockEstimator {
        self.clock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn receive(&self, message: BridgeMessage) {
        *self
            .last_heard
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Instant::now();
        match message {
            BridgeMessage::Pong {
                server_us,
                bridge_us,
                ..
            } => {
                self.clock
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .add(server_us, bridge_us, monotonic_us());
                self.pings.fetch_add(1, Ordering::Relaxed);
                self.wake.notify_all();
            }
            BridgeMessage::Devices { devices } => {
                *self
                    .devices
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = received_devices(devices);
                self.notify();
            }
            BridgeMessage::Opened { stream } => {
                let mut streams = self.streams();
                let status = streams.entry(stream).or_default();
                status.opened = Some(Ok(()));
                status.opened_at = Some(Instant::now());
                drop(streams);
                self.wake.notify_all();
            }
            BridgeMessage::Failed { stream, error } => {
                // The bridge is not authenticated: clean its text once here
                // so every log line and error shown to users stays one line.
                let error = clean_bridge_text(&error);
                let mut streams = self.streams();
                let replaced = error == crate::endpoint::REPLACED;
                if replaced && !streams.contains_key(&stream) {
                    // A stream this server already closed: nothing to do.
                    return;
                }
                let status = streams.entry(stream).or_default();
                status.opened = Some(Err(error.clone()));
                drop(streams);
                if !replaced {
                    self.push_error(error);
                }
                self.wake.notify_all();
            }
            BridgeMessage::Status {
                stream,
                received,
                heard,
                bridge_us,
                corrections,
                underruns,
                started,
                paused,
            } => {
                let at_server_us = self.clock().to_local(bridge_us);
                let mut streams = self.streams();
                let Some(status) = streams.get_mut(&stream) else {
                    return;
                };
                status.received = received;
                status.heard = heard;
                status.at_server_us = at_server_us;
                status.at = Some(Instant::now());
                status.corrections = corrections;
                status.underruns = underruns;
                status.started = started;
                status.paused = paused;
                if started && let Some(rate) = status.rate.as_mut() {
                    rate.0.add(at_server_us, heard);
                }
                drop(streams);
                self.wake.notify_all();
            }
            BridgeMessage::Drained { stream } => {
                if let Some(status) = self.streams().get_mut(&stream) {
                    status.drained = true;
                }
                self.wake.notify_all();
            }
            BridgeMessage::DeviceError { device, error } => {
                self.push_error(format!(
                    "{}: {}",
                    clean_bridge_text(&device),
                    clean_bridge_text(&error)
                ));
            }
            BridgeMessage::Welcome { .. } | BridgeMessage::Refused { .. } => {}
        }
    }

    fn push_error(&self, error: String) {
        let mut errors = self
            .errors
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if errors.len() == 8 {
            errors.pop_front();
        }
        errors.push_back(error);
    }
}

/// The most characters of bridge-supplied text the server keeps.
const MAX_BRIDGE_TEXT: usize = 300;

/// Bridge-supplied text with control characters (newlines, escapes)
/// removed and its length capped, so a rogue bridge on the LAN cannot forge
/// log lines or flood the errors shown to users.
pub(crate) fn clean_bridge_text(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_control())
        .take(MAX_BRIDGE_TEXT)
        .collect()
}

/// The devices a bridge lists, as the server keeps them. Names and
/// descriptions are cleaned like any bridge-supplied text. A device id is
/// sent back to the bridge to open the device, so it is never rewritten: a
/// device whose id is not already clean text is left out.
fn received_devices(devices: Vec<WireDevice>) -> Vec<DeviceInfo> {
    devices
        .into_iter()
        .filter(|device| clean_bridge_text(&device.id) == device.id)
        .map(|device| {
            let mut device = device.into_device();
            device.name = clean_bridge_text(&device.name);
            device.detail = clean_bridge_text(&device.detail);
            device
        })
        .collect()
}

/// A connected Parson Bridge.
#[derive(Clone)]
pub struct RemoteBridge {
    link: Arc<Link>,
}

impl RemoteBridge {
    /// Connects, says hello and waits for the bridge's welcome and a first
    /// clock estimate.
    pub fn connect(
        address: SocketAddr,
        server_id: &str,
        server_name: &str,
        timeout: Duration,
    ) -> Result<Self, String> {
        let socket = TcpStream::connect_timeout(&address, timeout)
            .map_err(|error| format!("could not reach {address}: {error}"))?;
        let _ = socket.set_nodelay(true);
        let clone = |socket: &TcpStream| socket.try_clone().map_err(|error| error.to_string());
        let mut writer = BufWriter::with_capacity(256 * 1024, clone(&socket)?);
        write_message(
            &mut writer,
            &ServerMessage::Hello {
                protocol: PROTOCOL_VERSION,
                server_id: server_id.to_string(),
                server_name: server_name.to_string(),
            },
        )
        .and_then(|()| writer.flush())
        .map_err(|error| error.to_string())?;
        let mut reader = BufReader::new(clone(&socket)?);
        socket
            .set_read_timeout(Some(timeout))
            .map_err(|error| error.to_string())?;
        let welcome = read_frame::<_, BridgeMessage>(&mut reader)
            .map_err(|error| format!("no answer from {address}: {error}"))?;
        if let Frame::Message(BridgeMessage::Refused { reason }) = &welcome {
            return Err(format!("{address}: {reason}"));
        }
        let Frame::Message(BridgeMessage::Welcome {
            protocol,
            bridge_id,
            name,
            host,
            platform,
            version,
            backend,
            note,
            devices,
        }) = welcome
        else {
            return Err(format!("{address} is not a Mujin Bridge"));
        };
        if protocol != PROTOCOL_VERSION {
            return Err(format!(
                "the bridge speaks protocol {protocol}; this server speaks {PROTOCOL_VERSION}. Update the older one."
            ));
        }
        socket
            .set_read_timeout(Some(SILENCE_TIMEOUT))
            .map_err(|error| error.to_string())?;
        let link = Arc::new(Link {
            writer: Mutex::new(writer),
            socket: clone(&socket)?,
            info: Mutex::new(BridgeInfo {
                bridge_id,
                name,
                host,
                platform,
                version,
                backend,
                note,
                address,
            }),
            devices: Mutex::new(received_devices(devices)),
            clock: Mutex::new(ClockEstimator::default()),
            streams: Mutex::new(HashMap::new()),
            wake: Condvar::new(),
            connected: AtomicBool::new(true),
            next_stream: AtomicU32::new(1),
            last_heard: Mutex::new(Instant::now()),
            errors: Mutex::new(VecDeque::new()),
            changed: Mutex::new(None),
            pings: AtomicU64::new(0),
        });
        let reader_link = link.clone();
        std::thread::Builder::new()
            .name("parson-bridge-link".into())
            .spawn(move || {
                loop {
                    match read_frame::<_, BridgeMessage>(&mut reader) {
                        Ok(Frame::Message(message)) => reader_link.receive(message),
                        Ok(Frame::Audio { .. }) => {}
                        Err(error) => {
                            reader_link.fail(error.to_string());
                            break;
                        }
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        let ping_link = Arc::downgrade(&link);
        std::thread::Builder::new()
            .name("parson-bridge-ping".into())
            .spawn(move || {
                let mut id = 0_u64;
                loop {
                    let Some(link) = ping_link.upgrade() else {
                        return;
                    };
                    if !link.connected.load(Ordering::Relaxed) {
                        return;
                    }
                    let silent = link
                        .last_heard
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .elapsed();
                    if silent > SILENCE_TIMEOUT {
                        link.fail("the bridge stopped answering".into());
                        return;
                    }
                    id += 1;
                    let _ = link.send(&ServerMessage::Ping {
                        id,
                        server_us: monotonic_us(),
                    });
                    drop(link);
                    // A burst at first for a quick clock estimate.
                    std::thread::sleep(if id < 10 {
                        Duration::from_millis(20)
                    } else {
                        PING_INTERVAL
                    });
                }
            })
            .map_err(|error| error.to_string())?;
        let bridge = Self { link };
        // Wait for a few round trips.
        let deadline = Instant::now() + timeout;
        while bridge.link.pings.load(Ordering::Relaxed) < 4 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(bridge)
    }

    pub fn info(&self) -> BridgeInfo {
        self.link
            .info
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn connected(&self) -> bool {
        self.link.connected.load(Ordering::Relaxed)
    }

    /// Calls `changed` when the device list changes or the link drops.
    pub fn on_change(&self, changed: impl Fn() + Send + Sync + 'static) {
        *self
            .link
            .changed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Box::new(changed));
    }

    /// Bridge clock minus server clock, and its uncertainty, in µs.
    pub fn clock_offset_us(&self) -> Option<(i64, i64)> {
        let clock = self.link.clock();
        Some((clock.offset_us()?, clock.uncertainty_us()?))
    }

    pub fn recent_errors(&self) -> Vec<String> {
        self.link
            .errors
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    pub fn disconnect(&self) {
        self.link.fail("disconnected by the server".into());
    }

    pub(crate) fn open_stream(
        &self,
        device_id: &str,
        format: StreamFormat,
        exclusive: bool,
        sync: Option<SyncStart>,
    ) -> Result<RemoteStream, String> {
        let stream = self.link.next_stream.fetch_add(1, Ordering::Relaxed);
        self.link.streams().insert(
            stream,
            StreamStatus {
                rate: Some(RateEstimatorBox(RateEstimator::new(format.sample_rate))),
                ..StreamStatus::default()
            },
        );
        self.link.send(&ServerMessage::Open {
            stream,
            device: device_id.to_string(),
            format: format.into(),
            exclusive,
            sync,
        })?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut streams = self.link.streams();
        loop {
            match streams
                .get(&stream)
                .and_then(|status| status.opened.clone())
            {
                Some(Ok(())) => break,
                Some(Err(error)) => {
                    streams.remove(&stream);
                    return Err(error);
                }
                None => {}
            }
            if !self.connected() {
                return Err("bridge disconnected".into());
            }
            let now = Instant::now();
            if now >= deadline {
                streams.remove(&stream);
                return Err("the bridge did not open the device in time".into());
            }
            streams = self
                .link
                .wake
                .wait_timeout(streams, deadline - now)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        drop(streams);
        Ok(RemoteStream {
            link: self.link.clone(),
            stream,
            format,
            sent: 0,
            floor: 0,
            paused: false,
            synced: sync.is_some(),
        })
    }
}

impl Backend for RemoteBridge {
    fn driver(&self) -> Driver {
        Driver::Bridge
    }

    fn devices(&self) -> Vec<DeviceInfo> {
        if !self.connected() {
            return Vec::new();
        }
        self.link
            .devices
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn open(
        &self,
        device_id: &str,
        format: StreamFormat,
        exclusive: bool,
    ) -> io::Result<Box<dyn Sink>> {
        self.open_stream(device_id, format, exclusive, None)
            .map(|stream| Box::new(stream) as Box<dyn Sink>)
            .map_err(io::Error::other)
    }

    fn set_hardware_volume(&self, device_id: &str, db: f32) -> io::Result<()> {
        self.link
            .send(&ServerMessage::Volume {
                device: device_id.to_string(),
                db,
            })
            .map_err(io::Error::other)
    }

    fn set_hardware_mute(&self, device_id: &str, muted: bool) -> io::Result<()> {
        self.link
            .send(&ServerMessage::Mute {
                device: device_id.to_string(),
                muted,
            })
            .map_err(io::Error::other)
    }
}

/// A stream on a bridge's device. Writes block to keep [`WINDOW`] of
/// audio ahead of what the bridge has played, like a device buffer.
pub struct RemoteStream {
    link: Arc<Link>,
    stream: u32,
    format: StreamFormat,
    sent: u64,
    /// Everything up to here was flushed.
    floor: u64,
    paused: bool,
    synced: bool,
}

/// Progress of a remote stream.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreamProgress {
    pub sent: u64,
    /// Source frames heard, extrapolated to now.
    pub heard: u64,
    pub corrections: i64,
    pub underruns: u32,
    pub started: bool,
    /// The bridge acknowledged a pause: its device has stopped and
    /// `heard` holds until playback resumes.
    pub paused: bool,
}

/// A stream's timeline: (server time, source frames heard, rate).
fn timeline(link: &Link, stream: u32) -> Option<(i64, u64, f64)> {
    let streams = link.streams();
    let status = streams.get(&stream)?;
    if !status.started {
        return None;
    }
    let rate = status.rate.as_ref()?;
    let (at, heard) = rate.0.latest()?;
    Some((at, heard, rate.0.rate()))
}

impl RemoteStream {
    fn heard_now(&self, status: &StreamStatus) -> u64 {
        let mut heard = status.heard;
        if status.started
            && !self.paused
            && let Some(at) = status.at
        {
            heard += (at.elapsed().as_secs_f64() * f64::from(self.format.sample_rate)) as u64;
        }
        heard
            .min(status.received.max(status.heard))
            .max(self.floor)
            .min(self.sent)
    }

    pub fn progress(&self) -> StreamProgress {
        let streams = self.link.streams();
        let status = streams.get(&self.stream).cloned().unwrap_or_default();
        drop(streams);
        StreamProgress {
            sent: self.sent,
            heard: self.heard_now(&status),
            corrections: status.corrections,
            underruns: status.underruns,
            started: status.started,
            paused: status.paused,
        }
    }

    fn window_frames(&self) -> u64 {
        (WINDOW.as_secs_f64() * f64::from(self.format.sample_rate)) as u64
    }
}

impl RemoteStream {
    fn put(&mut self, data: &[u8]) -> Result<(), String> {
        let frame_bytes = self.format.frame_bytes();
        if !data.len().is_multiple_of(frame_bytes) {
            return Err("partial frame".into());
        }
        let window = self.window_frames();
        let mut streams = self.link.streams();
        loop {
            if !self.link.connected.load(Ordering::Relaxed) {
                return Err("bridge disconnected".into());
            }
            let status = streams.get(&self.stream).cloned().unwrap_or_default();
            if let Some(Err(error)) = status.opened {
                return Err(error);
            }
            if self.sent.saturating_sub(self.heard_now(&status)) <= window {
                break;
            }
            if status
                .at
                .or(status.opened_at)
                .is_some_and(|last| last.elapsed() >= STREAM_STALL)
            {
                return Err("the bridge stopped playing this stream".into());
            }
            streams = self
                .link
                .wake
                .wait_timeout(streams, Duration::from_millis(10))
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        drop(streams);
        let max = (MAX_FRAME - 16) / frame_bytes * frame_bytes;
        for chunk in data.chunks(max) {
            self.link.send_audio(self.stream, chunk)?;
        }
        self.sent += (data.len() / frame_bytes) as u64;
        Ok(())
    }

    fn queued(&self) -> u64 {
        let streams = self.link.streams();
        let status = streams.get(&self.stream).cloned().unwrap_or_default();
        drop(streams);
        self.sent.saturating_sub(self.heard_now(&status))
    }

    fn hold(&mut self, paused: bool) -> Result<(), String> {
        if self.synced {
            return Err("synced streams restart on resume".into());
        }
        self.link.send(&ServerMessage::Pause {
            stream: self.stream,
            paused,
        })?;
        let mut streams = self.link.streams();
        if paused {
            // Frames keep playing until the pause reaches the device, so
            // wait for the bridge to report where it stopped: from then on
            // the position is exact. This holds for a stream that has not
            // started yet too: it may start before the pause arrives.
            let deadline = Instant::now() + PAUSE_ACK_TIMEOUT;
            while streams
                .get(&self.stream)
                .is_some_and(|status| !status.paused)
                && self.link.connected.load(Ordering::Relaxed)
            {
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                streams = self
                    .link
                    .wake
                    .wait_timeout(streams, deadline - now)
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .0;
            }
        } else if let Some(status) = streams.get_mut(&self.stream) {
            status.paused = false;
        }
        // Freeze the extrapolation at the last report.
        if let Some(status) = streams.get_mut(&self.stream)
            && paused
            && !status.paused
        {
            let heard = {
                let status = status.clone();
                self.heard_now(&status)
            };
            status.heard = heard;
            status.at = Some(Instant::now());
        }
        self.paused = paused;
        Ok(())
    }

    fn discard(&mut self) {
        let _ = self.link.send(&ServerMessage::Flush {
            stream: self.stream,
        });
        self.floor = self.sent;
        if let Some(status) = self.link.streams().get_mut(&self.stream) {
            status.heard = self.sent;
            status.at = Some(Instant::now());
        }
    }

    fn play_out(&mut self) {
        if self
            .link
            .send(&ServerMessage::Drain {
                stream: self.stream,
            })
            .is_err()
        {
            return;
        }
        let wait = Duration::from_secs_f64(
            self.queued() as f64 / f64::from(self.format.sample_rate.max(1)),
        ) + Duration::from_secs(3);
        let deadline = Instant::now() + wait;
        let mut streams = self.link.streams();
        while !streams
            .get(&self.stream)
            .is_some_and(|status| status.drained)
            && self.link.connected.load(Ordering::Relaxed)
        {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            streams = self
                .link
                .wake
                .wait_timeout(streams, deadline - now)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        if let Some(status) = streams.get_mut(&self.stream) {
            status.drained = false;
        }
    }
}

impl Sink for RemoteStream {
    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        self.put(data).map_err(io::Error::other)
    }

    fn delay_frames(&mut self) -> u64 {
        self.queued()
    }

    fn pause(&mut self, paused: bool) -> io::Result<()> {
        self.hold(paused).map_err(io::Error::other)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.discard();
        Ok(())
    }

    fn drain(&mut self) -> io::Result<()> {
        self.play_out();
        Ok(())
    }
}

impl Drop for RemoteStream {
    fn drop(&mut self) {
        let _ = self.link.send(&ServerMessage::Close {
            stream: self.stream,
        });
        self.link.streams().remove(&self.stream);
    }
}

/// One member of a synced group: a device on a bridge.
#[derive(Clone)]
pub struct GroupMember {
    pub bridge: RemoteBridge,
    pub device_id: String,
}

impl GroupMember {
    pub fn device(&self) -> Option<DeviceInfo> {
        self.bridge
            .devices()
            .into_iter()
            .find(|device| device.id == self.device_id)
    }
}

/// What every member takes: the rates and sample formats they share.
pub fn common_device(id: &str, name: &str, members: &[DeviceInfo]) -> Option<DeviceInfo> {
    let first = members.first()?;
    let mut rates = first.sample_rates().to_vec();
    let mut formats: Vec<_> = first
        .sample_formats()
        .iter()
        .copied()
        .filter(|format| !format.is_dsd())
        .collect();
    let mut channels = first.max_channels();
    for member in &members[1..] {
        rates.retain(|rate| member.sample_rates().contains(rate));
        formats.retain(|format| member.sample_formats().contains(format));
        channels = channels.min(member.max_channels());
    }
    if rates.is_empty() || formats.is_empty() {
        return None;
    }
    let hardware_volume = members
        .iter()
        .all(|member| member.hardware_volume.is_some())
        .then_some(HardwareVolume {
            min_db: -80.0,
            max_db: 0.0,
            step_db: 0.5,
        });
    let busy = members.iter().any(|member| member.busy);
    Some(DeviceInfo {
        id: id.to_string(),
        name: name.to_string(),
        detail: members
            .iter()
            .map(|member| member.name.clone())
            .collect::<Vec<_>>()
            .join(" + "),
        driver: Driver::Bridge,
        exclusive_capable: true,
        caps: Some(DeviceCaps {
            sample_rates: rates,
            sample_formats: formats,
            min_channels: 1,
            max_channels: channels,
            // Groups play PCM: native DSD cannot be sample-corrected.
            dsd_formats: Vec::new(),
            dsd_rates: Vec::new(),
        }),
        probe_error: busy.then(|| "a group member is in use by another app".to_string()),
        is_default: false,
        hardware: None,
        address: None,
        network: None,
        hardware_volume,
        busy,
    })
}

/// Several bridge devices playing one stream, sample-synced: the first
/// member is the clock master; the others follow its DAC clock.
pub struct SyncGroup {
    pub id: String,
    pub name: String,
    members: Vec<GroupMember>,
}

impl SyncGroup {
    pub fn new(id: &str, name: &str, members: Vec<GroupMember>) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            members,
        }
    }

    pub fn members(&self) -> &[GroupMember] {
        &self.members
    }

    pub fn device(&self) -> Option<DeviceInfo> {
        let devices: Option<Vec<DeviceInfo>> =
            self.members.iter().map(|member| member.device()).collect();
        let mut device = common_device(&self.id, &self.name, &devices?)?;
        // Which machines play, in order: the clock master first.
        device.detail = self
            .members
            .iter()
            .map(|member| member.bridge.info().name)
            .collect::<Vec<_>>()
            .join(" + ");
        Some(device)
    }
}

impl Backend for SyncGroup {
    fn driver(&self) -> Driver {
        Driver::Bridge
    }

    fn name(&self) -> &'static str {
        "Mujin Bridge group"
    }

    fn devices(&self) -> Vec<DeviceInfo> {
        self.device().into_iter().collect()
    }

    fn open(
        &self,
        _device_id: &str,
        format: StreamFormat,
        exclusive: bool,
    ) -> io::Result<Box<dyn Sink>> {
        Ok(Box::new(GroupStream {
            members: self.members.clone(),
            format,
            exclusive,
            streams: Vec::new(),
            stop: Arc::new(AtomicBool::new(false)),
            shared: Arc::new(Mutex::new(Vec::new())),
            offline: 0,
        }))
    }

    fn set_hardware_volume(&self, _device_id: &str, db: f32) -> io::Result<()> {
        // The same level on each member's own scale.
        let mut result = Ok(());
        for member in &self.members {
            let Some(range) = member.device().and_then(|device| device.hardware_volume) else {
                continue;
            };
            let fraction = (db + 80.0) / 80.0;
            let low = range.min_db.max(range.max_db - 80.0);
            let member_db = if db <= -80.0 {
                range.min_db
            } else {
                low + (range.max_db - low) * fraction.clamp(0.0, 1.0)
            };
            if let Err(error) = member
                .bridge
                .set_hardware_volume(&member.device_id, member_db)
            {
                result = Err(error);
            }
        }
        result
    }

    fn set_hardware_mute(&self, _device_id: &str, muted: bool) -> io::Result<()> {
        let mut result = Ok(());
        for member in &self.members {
            if let Err(error) = member.bridge.set_hardware_mute(&member.device_id, muted) {
                result = Err(error);
            }
        }
        result
    }
}

/// Live sync state of a group stream, for status displays and tests.
#[derive(Clone, Debug, PartialEq)]
pub struct MemberSync {
    pub device_id: String,
    pub master: bool,
    pub progress: StreamProgress,
}

/// Open member streams by device id, master first.
type MemberStreams = Vec<(String, Arc<Mutex<RemoteStream>>)>;

struct GroupStream {
    members: Vec<GroupMember>,
    format: StreamFormat,
    exclusive: bool,
    /// Open member streams (master first); empty until the first write.
    streams: MemberStreams,
    stop: Arc<AtomicBool>,
    shared: Arc<Mutex<MemberStreams>>,
    offline: usize,
}

fn lock_stream(stream: &Mutex<RemoteStream>) -> std::sync::MutexGuard<'_, RemoteStream> {
    stream
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl GroupStream {
    /// Opens every member with one start time, master first.
    fn start(&mut self) -> Result<(), String> {
        let start_server = monotonic_us() + SYNC_LEAD.as_micros() as i64;
        let mut streams = Vec::new();
        for (index, member) in self.members.iter().enumerate() {
            let clock = member.bridge.link.clock();
            let sync = SyncStart {
                start_bridge_us: clock.to_remote(start_server),
                master: index == 0,
            };
            match member.bridge.open_stream(
                &member.device_id,
                self.format,
                self.exclusive,
                Some(sync),
            ) {
                Ok(stream) => {
                    streams.push((member.device_id.clone(), Arc::new(Mutex::new(stream))))
                }
                Err(error) if index == 0 => return Err(error),
                Err(error) => {
                    // A follower that cannot play leaves the others alone.
                    tracing::warn!(%error, device = member.device_id, "group member could not open");
                    self.offline += 1;
                }
            }
        }
        self.stop = Arc::new(AtomicBool::new(false));
        *self
            .shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = streams.clone();
        self.streams = streams;
        self.spawn_follower_sync();
        Ok(())
    }

    /// Sends followers the master's timeline four times a second.
    fn spawn_follower_sync(&self) {
        let stop = self.stop.clone();
        let links: Vec<(Arc<Link>, u32)> = self
            .streams
            .iter()
            .map(|(_, stream)| {
                let stream = lock_stream(stream);
                (stream.link.clone(), stream.stream)
            })
            .collect();
        let _ = std::thread::Builder::new()
            .name("parson-bridge-sync".into())
            .spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(250));
                    let Some((master, master_stream)) = links.first() else {
                        return;
                    };
                    let Some((at_server, frame, rate)) = timeline(master, *master_stream) else {
                        continue;
                    };
                    for (link, stream) in &links[1..] {
                        let bridge_us = link.clock().to_remote(at_server);
                        let _ = link.send(&ServerMessage::SyncRef {
                            stream: *stream,
                            frame,
                            bridge_us,
                            rate,
                        });
                    }
                }
            });
    }

    fn close(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.streams.clear();
        self.shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }
}

impl Sink for GroupStream {
    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        if self.streams.is_empty() {
            self.start().map_err(io::Error::other)?;
        }
        let mut failed = Vec::new();
        for (index, (device, stream)) in self.streams.iter().enumerate() {
            if let Err(error) = lock_stream(stream).put(data) {
                if index == 0 {
                    return Err(io::Error::other(error));
                }
                tracing::warn!(%error, %device, "group member dropped out");
                failed.push(index);
            }
        }
        for index in failed.into_iter().rev() {
            self.streams.remove(index);
            self.offline += 1;
        }
        Ok(())
    }

    fn delay_frames(&mut self) -> u64 {
        self.streams
            .first()
            .map(|(_, master)| lock_stream(master).queued())
            .unwrap_or(0)
    }

    fn pause(&mut self, _paused: bool) -> io::Result<()> {
        // Resuming starts the group again on a fresh common start time.
        Err(parson_output::unsupported(
            "synced groups restart on resume",
        ))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.close();
        Ok(())
    }

    fn drain(&mut self) -> io::Result<()> {
        for (_, stream) in &self.streams {
            lock_stream(stream).play_out();
        }
        self.close();
        Ok(())
    }
}

impl Drop for GroupStream {
    fn drop(&mut self) {
        self.close();
    }
}

impl SyncGroup {
    /// Opens a stream directly (tests and diagnostics); the zone service
    /// goes through [`Backend::open`].
    pub fn open_probe(
        &self,
        format: StreamFormat,
    ) -> (Box<dyn Sink>, impl Fn() -> Vec<MemberSync>) {
        let stream = GroupStream {
            members: self.members.clone(),
            format,
            exclusive: true,
            streams: Vec::new(),
            stop: Arc::new(AtomicBool::new(false)),
            shared: Arc::new(Mutex::new(Vec::new())),
            offline: 0,
        };
        let shared = stream.shared.clone();
        let probe = move || {
            shared
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .iter()
                .enumerate()
                .map(|(index, (device_id, stream))| MemberSync {
                    device_id: device_id.clone(),
                    master: index == 0,
                    progress: lock_stream(stream).progress(),
                })
                .collect()
        };
        (Box::new(stream), probe)
    }
}
