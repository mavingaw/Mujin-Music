//! A protocol-level fake AirPlay receiver, for tests and demos.
//!
//! No AirPlay speaker is attached where Parson is built and tested, so this
//! stands in for one at the wire level: an AirPlay 1 receiver (RTSP server,
//! RTP audio with ALAC decoding, timing requests, sync packets and a
//! retransmit request). It proves the protocol path, not any particular
//! device, and everything that shows it says so. (The fake UPnP and
//! OpenHome renderer lives with the zones that drive renderers.)

use std::io::{self, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::http::read_head;

/* ------------------------------------------------------------------ */
/* AirPlay receiver                                                    */
/* ------------------------------------------------------------------ */

struct BitReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl BitReader<'_> {
    fn read(&mut self, count: usize) -> Result<u32, String> {
        let mut value = 0u32;
        for _ in 0..count {
            let byte = *self
                .bytes
                .get(self.position / 8)
                .ok_or("the frame ended early")?;
            let bit = (byte >> (7 - self.position % 8)) & 1;
            value = (value << 1) | u32::from(bit);
            self.position += 1;
        }
        Ok(value)
    }
}

/// Decodes an uncompressed (escape) stereo 16-bit ALAC frame, reading the
/// bitstream the way Apple's `ALACDecoder::Decode` does.
pub fn decode_alac_escape(frame: &[u8]) -> Result<Vec<i16>, String> {
    let mut bits = BitReader {
        bytes: frame,
        position: 0,
    };
    if bits.read(3)? != 1 {
        return Err("not a channel pair element".into());
    }
    bits.read(4)?;
    if bits.read(12)? != 0 {
        return Err("unused header bits are set".into());
    }
    let header = bits.read(4)?;
    let partial = header >> 3 != 0;
    let shifted = (header >> 1) & 3;
    if header & 1 == 0 {
        return Err("a compressed frame; the fake only decodes escape frames".into());
    }
    if shifted != 0 {
        return Err("shifted bytes in a 16-bit frame".into());
    }
    let frames = if partial {
        bits.read(32)? as usize
    } else {
        352
    };
    if frames > 4096 {
        return Err("too many samples".into());
    }
    let mut samples = Vec::with_capacity(frames * 2);
    for _ in 0..frames * 2 {
        samples.push(bits.read(16)? as u16 as i16);
    }
    if bits.read(3)? != 7 {
        return Err("missing end tag".into());
    }
    Ok(samples)
}

#[derive(Clone, Debug, Default)]
pub struct FakeAirPlayConfig {
    /// Answer `ANNOUNCE` with 470, as a receiver that requires pairing.
    pub require_pairing: bool,
    /// Lose this audio packet (1-based) once and ask the sender to resend
    /// it on the control port, as a receiver does after a network drop.
    pub drop_packet: Option<u64>,
    /// `Audio-Latency` in the `RECORD` reply.
    pub audio_latency: Option<u32>,
}

/// What the fake receiver saw and decoded.
#[derive(Clone, Debug, Default)]
pub struct FakeAirPlayState {
    pub methods: Vec<String>,
    pub sdp: Option<String>,
    pub parameters: Vec<String>,
    /// Decoded samples in sequence order.
    pub samples: Vec<i16>,
    pub packets: u64,
    pub marker_packets: u64,
    pub first_seq: Option<u16>,
    pub out_of_order: u64,
    pub sync_packets: u64,
    pub first_sync_seen: bool,
    /// The sender answered a timing request correctly.
    pub timing_ok: bool,
    /// Packets the sender resent on request.
    pub resent: u64,
    /// The sequence number the receiver asked to have resent.
    pub dropped_seq: Option<u16>,
    /// Latency the last sync packet announced (RTP now minus RTP at the
    /// output), in frames.
    pub sync_latency: Option<u32>,
    /// `volume: ...` values the sender set, in dB.
    pub volumes: Vec<f32>,
    pub decode_errors: Vec<String>,
    pub flushes: u64,
    pub torn_down: bool,
}

struct AirPlayShared {
    config: FakeAirPlayConfig,
    state: Mutex<FakeAirPlayState>,
    stop: AtomicBool,
}

impl AirPlayShared {
    fn state(&self) -> std::sync::MutexGuard<'_, FakeAirPlayState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub struct FakeAirPlayReceiver {
    shared: Arc<AirPlayShared>,
    pub address: SocketAddr,
    threads: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl FakeAirPlayReceiver {
    pub fn start(config: FakeAirPlayConfig) -> io::Result<Self> {
        Self::start_on(config, SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
    }

    pub fn start_on(config: FakeAirPlayConfig, bind: SocketAddr) -> io::Result<Self> {
        let listener = TcpListener::bind(bind)?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let shared = Arc::new(AirPlayShared {
            config,
            state: Mutex::new(FakeAirPlayState::default()),
            stop: AtomicBool::new(false),
        });
        let threads = Arc::new(Mutex::new(Vec::new()));
        let accept_shared = Arc::clone(&shared);
        let accept_threads = Arc::clone(&threads);
        let accept = std::thread::spawn(move || {
            while !accept_shared.stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((connection, _)) => {
                        let shared = Arc::clone(&accept_shared);
                        let threads = Arc::clone(&accept_threads);
                        let handle = std::thread::spawn(move || {
                            let _ = handle_rtsp(connection, &shared, &threads);
                        });
                        if let Ok(mut list) = accept_threads.lock() {
                            list.push(handle);
                        }
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        });
        if let Ok(mut list) = threads.lock() {
            list.push(accept);
        }
        Ok(Self {
            shared,
            address,
            threads,
        })
    }

    pub fn state(&self) -> FakeAirPlayState {
        self.shared.state().clone()
    }

    pub fn wait_for(
        &self,
        timeout: Duration,
        predicate: impl Fn(&FakeAirPlayState) -> bool,
    ) -> FakeAirPlayState {
        let deadline = Instant::now() + timeout;
        loop {
            let state = self.state();
            if predicate(&state) || Instant::now() > deadline {
                return state;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for FakeAirPlayReceiver {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        let threads: Vec<_> = self
            .threads
            .lock()
            .map(|mut list| list.drain(..).collect())
            .unwrap_or_default();
        for thread in threads {
            let _ = thread.join();
        }
    }
}

fn transport_value(transport: &str, name: &str) -> Option<u16> {
    super::raop::transport_port(transport, name)
}

fn handle_rtsp(
    connection: TcpStream,
    shared: &Arc<AirPlayShared>,
    threads: &Arc<Mutex<Vec<JoinHandle<()>>>>,
) -> io::Result<()> {
    connection.set_nonblocking(false)?;
    connection.set_read_timeout(Some(Duration::from_millis(200)))?;
    let peer_ip = connection.peer_addr()?.ip();
    let local_ip = connection.local_addr()?.ip();
    let mut writer = connection.try_clone()?;
    let mut reader = BufReader::new(connection);
    while !shared.stop.load(Ordering::Relaxed) {
        let (request_line, headers) = match read_head(&mut reader) {
            Ok(head) => head,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(_) => return Ok(()),
        };
        let header = |name: &str| {
            headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.clone())
        };
        let length: usize = header("Content-Length")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        let mut body = vec![0; length.min(64 * 1024)];
        reader
            .get_mut()
            .set_read_timeout(Some(Duration::from_secs(2)))?;
        reader.read_exact(&mut body)?;
        reader
            .get_mut()
            .set_read_timeout(Some(Duration::from_millis(200)))?;
        let method = request_line
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string();
        let cseq = header("CSeq").unwrap_or_else(|| "0".into());
        shared.state().methods.push(method.clone());
        let mut extra = String::new();
        let mut status = "200 OK";
        match method.as_str() {
            "OPTIONS" => extra.push_str("Public: ANNOUNCE, SETUP, RECORD, PAUSE, FLUSH, TEARDOWN, OPTIONS, GET_PARAMETER, SET_PARAMETER\r\n"),
            "ANNOUNCE" => {
                if shared.config.require_pairing {
                    status = "470 Connection Authorization Required";
                } else {
                    shared.state().sdp = Some(String::from_utf8_lossy(&body).into_owned());
                }
            }
            "SETUP" => {
                let transport = header("Transport").unwrap_or_default();
                let client_control = transport_value(&transport, "control_port");
                let client_timing = transport_value(&transport, "timing_port");
                let audio = UdpSocket::bind(SocketAddr::new(local_ip, 0))?;
                let control = UdpSocket::bind(SocketAddr::new(local_ip, 0))?;
                let timing = UdpSocket::bind(SocketAddr::new(local_ip, 0))?;
                extra.push_str(&format!(
                    "Transport: RTP/AVP/UDP;unicast;mode=record;server_port={};control_port={};timing_port={}\r\nSession: 1\r\nAudio-Jack-Status: connected; type=analog\r\n",
                    audio.local_addr()?.port(),
                    control.local_addr()?.port(),
                    timing.local_addr()?.port()
                ));
                let receivers = spawn_udp(shared, audio, control, timing, client_timing.map(|port| SocketAddr::new(peer_ip, port)), client_control.map(|port| SocketAddr::new(peer_ip, port)))?;
                if let Ok(mut list) = threads.lock() {
                    list.extend(receivers);
                }
            }
            "RECORD" => {
                if let Some(latency) = shared.config.audio_latency {
                    extra.push_str(&format!("Audio-Latency: {latency}\r\n"));
                }
            }
            "SET_PARAMETER" => {
                let text = String::from_utf8_lossy(&body).into_owned();
                let mut state = shared.state();
                if let Some(volume) = text
                    .strip_prefix("volume:")
                    .and_then(|value| value.trim().parse::<f32>().ok())
                {
                    state.volumes.push(volume);
                }
                state.parameters.push(text);
            }
            "FLUSH" => shared.state().flushes += 1,
            "TEARDOWN" => shared.state().torn_down = true,
            _ => {}
        }
        writer.write_all(
            format!("RTSP/1.0 {status}\r\nCSeq: {cseq}\r\nServer: AirTunes/366.0\r\n{extra}\r\n")
                .as_bytes(),
        )?;
        if method == "TEARDOWN" {
            return Ok(());
        }
    }
    Ok(())
}

/// Puts a packet's samples in sequence order: packets arrive in order
/// except a resent one, which goes back where it belongs.
fn place_packet(
    state: &mut FakeAirPlayState,
    order: &mut std::collections::BTreeMap<u32, Vec<i16>>,
    first: u16,
    seq: u16,
    samples: Vec<i16>,
) {
    let index = u32::from(seq.wrapping_sub(first));
    let in_order = order.keys().next_back().is_none_or(|last| index > *last);
    if in_order {
        state.samples.extend_from_slice(&samples);
        order.insert(index, samples);
    } else {
        order.insert(index, samples);
        state.samples = order.values().flatten().copied().collect();
    }
}

fn spawn_udp(
    shared: &Arc<AirPlayShared>,
    audio: UdpSocket,
    control: UdpSocket,
    timing: UdpSocket,
    client_timing: Option<SocketAddr>,
    client_control: Option<SocketAddr>,
) -> io::Result<Vec<JoinHandle<()>>> {
    for socket in [&audio, &control, &timing] {
        socket.set_read_timeout(Some(Duration::from_millis(50)))?;
    }
    let order = Arc::new(Mutex::new(
        std::collections::BTreeMap::<u32, Vec<i16>>::new(),
    ));
    let mut handles = Vec::new();
    let audio_shared = Arc::clone(shared);
    let audio_order = Arc::clone(&order);
    let request_socket = control.try_clone()?;
    handles.push(std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        let mut last_seq: Option<u16> = None;
        let mut seen = 0u64;
        while !audio_shared.stop.load(Ordering::Relaxed) {
            let Ok(length) = audio.recv(&mut buffer) else {
                continue;
            };
            if length < 12 || buffer[0] != 0x80 || buffer[1] & 0x7F != 0x60 {
                continue;
            }
            seen += 1;
            let marker = buffer[1] & 0x80 != 0;
            let seq = u16::from_be_bytes([buffer[2], buffer[3]]);
            let mut state = audio_shared.state();
            if audio_shared.config.drop_packet == Some(seen) && state.dropped_seq.is_none() {
                // Lost on the way: ask for it again (0x80 0xD5, seq, first
                // missing, count).
                state.dropped_seq = Some(seq);
                if let Some(target) = client_control {
                    let mut request = [0u8; 8];
                    request[0] = 0x80;
                    request[1] = 0xD5;
                    request[2..4].copy_from_slice(&1u16.to_be_bytes());
                    request[4..6].copy_from_slice(&seq.to_be_bytes());
                    request[6..8].copy_from_slice(&1u16.to_be_bytes());
                    let _ = request_socket.send_to(&request, target);
                }
                last_seq = Some(seq);
                continue;
            }
            state.packets += 1;
            if marker {
                state.marker_packets += 1;
            }
            let first = *state.first_seq.get_or_insert(seq);
            if let Some(last) = last_seq
                && seq != last.wrapping_add(1)
                && !marker
            {
                state.out_of_order += 1;
            }
            last_seq = Some(seq);
            match decode_alac_escape(&buffer[12..length]) {
                Ok(samples) => {
                    let mut order = audio_order
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    place_packet(&mut state, &mut order, first, seq, samples);
                }
                Err(error) => {
                    if state.decode_errors.len() < 8 {
                        state.decode_errors.push(error);
                    }
                }
            }
        }
    }));
    let control_shared = Arc::clone(shared);
    let control_order = Arc::clone(&order);
    handles.push(std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        while !control_shared.stop.load(Ordering::Relaxed) {
            let Ok(length) = control.recv(&mut buffer) else {
                continue;
            };
            if length >= 20 && buffer[1] & 0x7F == 0x54 {
                let mut state = control_shared.state();
                state.sync_packets += 1;
                if buffer[0] == 0x90 {
                    state.first_sync_seen = true;
                }
                let minus = u32::from_be_bytes([buffer[4], buffer[5], buffer[6], buffer[7]]);
                let now = u32::from_be_bytes([buffer[16], buffer[17], buffer[18], buffer[19]]);
                state.sync_latency = Some(now.wrapping_sub(minus));
            } else if length > 16 && buffer[1] & 0x7F == 0x56 {
                // A resent packet: 4 bytes of header, then the RTP packet.
                let inner = &buffer[4..length];
                let seq = u16::from_be_bytes([inner[2], inner[3]]);
                let mut state = control_shared.state();
                state.resent += 1;
                state.packets += 1;
                let first = *state.first_seq.get_or_insert(seq);
                match decode_alac_escape(&inner[12..]) {
                    Ok(samples) => {
                        let mut order = control_order
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        place_packet(&mut state, &mut order, first, seq, samples);
                    }
                    Err(error) => {
                        if state.decode_errors.len() < 8 {
                            state.decode_errors.push(error);
                        }
                    }
                }
            }
        }
    }));
    let timing_shared = Arc::clone(shared);
    handles.push(std::thread::spawn(move || {
        let Some(client) = client_timing else {
            return;
        };
        let mut request = [0u8; 32];
        request[0] = 0x80;
        request[1] = 0xD2;
        request[3] = 0x07;
        request[24..32].copy_from_slice(&0x1122_3344_5566_7788u64.to_be_bytes());
        let mut buffer = [0u8; 64];
        let deadline = Instant::now() + Duration::from_secs(3);
        while !timing_shared.stop.load(Ordering::Relaxed) && Instant::now() < deadline {
            let _ = timing.send_to(&request, client);
            if let Ok(length) = timing.recv(&mut buffer)
                && length >= 32
                && buffer[1] == 0xD3
                && buffer[8..16] == request[24..32]
            {
                timing_shared.state().timing_ok = true;
                return;
            }
        }
    }));
    Ok(handles)
}
