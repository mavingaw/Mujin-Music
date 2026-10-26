//! A protocol-level fake AirPlay (RAOP) receiver for tests, modelled on
//! what AirPort Express and shairport-sync do: RTSP on TCP (OPTIONS,
//! ANNOUNCE with an ALAC SDP, SETUP with UDP ports, RECORD, SET_PARAMETER
//! volume, FLUSH, TEARDOWN), RTP audio on UDP decoded with its own ALAC
//! reader, sync packets on the control port, a timing request to the
//! sender, and one deliberately lost packet that it asks to be resent.

use std::collections::BTreeMap;
use std::io::{BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::alac::decode_verbatim;
use super::raop::{ntp_now, read_message};

#[derive(Clone, Debug, Default)]
pub struct FakeState {
    pub methods: Vec<String>,
    pub sdp: String,
    pub volumes: Vec<f32>,
    /// Decoded audio by RTP sequence (unwrapped).
    pub packets: BTreeMap<u32, Vec<u8>>,
    pub markers: u32,
    pub syncs: u32,
    /// Latency the sync packets announce (now - "now minus latency").
    pub sync_latency: Option<u32>,
    pub timing_replies: u32,
    pub resent: u32,
    pub dropped: Option<u16>,
    pub flushes: u32,
    pub teardowns: u32,
    pub bad_frames: u32,
    first_seq: Option<u16>,
}

impl FakeState {
    /// Everything received, in sequence order.
    pub fn pcm(&self) -> Vec<u8> {
        self.packets.values().flatten().copied().collect()
    }

    fn unwrap_seq(&mut self, seq: u16) -> u32 {
        let first = *self.first_seq.get_or_insert(seq);
        u32::from(seq.wrapping_sub(first))
    }
}

pub struct FakeReceiver {
    pub address: SocketAddr,
    pub state: Arc<Mutex<FakeState>>,
}

fn lock(state: &Mutex<FakeState>) -> std::sync::MutexGuard<'_, FakeState> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl FakeReceiver {
    /// `airplay2_only`: refuse ANNOUNCE like a HomePod would.
    pub fn start(airplay2_only: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listen");
        let address = listener.local_addr().expect("address");
        let state = Arc::new(Mutex::new(FakeState::default()));
        let audio = UdpSocket::bind("127.0.0.1:0").expect("audio socket");
        let control = UdpSocket::bind("127.0.0.1:0").expect("control socket");
        let timing = UdpSocket::bind("127.0.0.1:0").expect("timing socket");
        let ports = (
            audio.local_addr().unwrap().port(),
            control.local_addr().unwrap().port(),
            timing.local_addr().unwrap().port(),
        );
        let sender_control: Arc<Mutex<Option<SocketAddr>>> = Arc::new(Mutex::new(None));
        // Audio: decode, drop packet 5 once and ask for it again.
        {
            let state = state.clone();
            let control = control.try_clone().unwrap();
            let sender_control = sender_control.clone();
            std::thread::spawn(move || {
                let mut buffer = vec![0_u8; 4_096];
                let mut seen = 0_u32;
                loop {
                    let Ok(length) = audio.recv(&mut buffer) else {
                        return;
                    };
                    if length < 12 || buffer[1] & 0x7f != 0x60 {
                        continue;
                    }
                    seen += 1;
                    let seq = u16::from_be_bytes([buffer[2], buffer[3]]);
                    if seen == 5 && lock(&state).dropped.is_none() {
                        lock(&state).dropped = Some(seq);
                        if let Some(target) = *sender_control.lock().unwrap() {
                            let mut request = [0_u8; 8];
                            request[0] = 0x80;
                            request[1] = 0xd5;
                            request[2..4].copy_from_slice(&1_u16.to_be_bytes());
                            request[4..6].copy_from_slice(&seq.to_be_bytes());
                            request[6..8].copy_from_slice(&1_u16.to_be_bytes());
                            let _ = control.send_to(&request, target);
                        }
                        continue;
                    }
                    let marker = buffer[1] & 0x80 != 0;
                    let mut state = lock(&state);
                    if marker {
                        state.markers += 1;
                    }
                    let index = state.unwrap_seq(seq);
                    match decode_verbatim(&buffer[12..length]) {
                        Some(pcm) => {
                            state.packets.insert(index, pcm);
                        }
                        None => state.bad_frames += 1,
                    }
                }
            });
        }
        // Control: sync packets and resent audio.
        {
            let state = state.clone();
            std::thread::spawn(move || {
                let mut buffer = vec![0_u8; 4_096];
                loop {
                    let Ok(length) = control.recv(&mut buffer) else {
                        return;
                    };
                    if length < 8 {
                        continue;
                    }
                    match buffer[1] & 0x7f {
                        0x54 if length >= 20 => {
                            let minus = u32::from_be_bytes(buffer[4..8].try_into().unwrap());
                            let now = u32::from_be_bytes(buffer[16..20].try_into().unwrap());
                            let mut state = lock(&state);
                            state.syncs += 1;
                            state.sync_latency = Some(now.wrapping_sub(minus));
                        }
                        0x56 if length > 16 => {
                            let inner = &buffer[4..length];
                            let seq = u16::from_be_bytes([inner[2], inner[3]]);
                            let mut state = lock(&state);
                            state.resent += 1;
                            let index = state.unwrap_seq(seq);
                            if let Some(pcm) = decode_verbatim(&inner[12..]) {
                                state.packets.insert(index, pcm);
                            }
                        }
                        _ => {}
                    }
                }
            });
        }
        // RTSP.
        {
            let state = state.clone();
            std::thread::spawn(move || {
                for connection in listener.incoming() {
                    let Ok(connection) = connection else {
                        continue;
                    };
                    let state = state.clone();
                    let timing = timing.try_clone().unwrap();
                    let sender_control = sender_control.clone();
                    std::thread::spawn(move || {
                        serve(
                            connection,
                            state,
                            ports,
                            timing,
                            sender_control,
                            airplay2_only,
                        )
                    });
                }
            });
        }
        Self { address, state }
    }

    pub fn state(&self) -> FakeState {
        lock(&self.state).clone()
    }
}

fn serve(
    connection: TcpStream,
    state: Arc<Mutex<FakeState>>,
    (audio_port, control_port, timing_port): (u16, u16, u16),
    timing: UdpSocket,
    sender_control: Arc<Mutex<Option<SocketAddr>>>,
    airplay2_only: bool,
) {
    let peer = connection.peer_addr().expect("peer");
    let mut writer = connection.try_clone().expect("clone");
    let mut reader = BufReader::new(connection);
    while let Ok((request_line, headers, body)) = read_message(&mut reader) {
        let method = request_line
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string();
        let header = |name: &str| {
            headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.clone())
        };
        let cseq = header("CSeq").unwrap_or_default();
        lock(&state).methods.push(method.clone());
        let mut extra = String::new();
        let mut status = "200 OK";
        match method.as_str() {
            "ANNOUNCE" => {
                if airplay2_only {
                    status = "403 Forbidden";
                }
                lock(&state).sdp = String::from_utf8_lossy(&body).to_string();
            }
            "SETUP" => {
                let transport = header("Transport").unwrap_or_default();
                let port = |name: &str| {
                    transport.split(';').find_map(|part| {
                        let (key, value) = part.split_once('=')?;
                        (key == name).then(|| value.parse::<u16>().ok()).flatten()
                    })
                };
                if let Some(port) = port("control_port") {
                    *sender_control.lock().unwrap() = Some(SocketAddr::new(peer.ip(), port));
                }
                if let Some(port) = port("timing_port") {
                    // Ask the sender for its clock, like real receivers.
                    let target = SocketAddr::new(peer.ip(), port);
                    let timing = timing.try_clone().unwrap();
                    let state = state.clone();
                    std::thread::spawn(move || {
                        let mut request = [0_u8; 32];
                        request[0] = 0x80;
                        request[1] = 0xd2;
                        request[2..4].copy_from_slice(&7_u16.to_be_bytes());
                        let sent = ntp_now();
                        request[24..32].copy_from_slice(&sent.to_be_bytes());
                        let _ = timing.set_read_timeout(Some(Duration::from_secs(2)));
                        let _ = timing.send_to(&request, target);
                        let mut reply = [0_u8; 64];
                        if let Ok(length) = timing.recv(&mut reply)
                            && length >= 32
                            && reply[1] == 0xd3
                            && reply[8..16] == sent.to_be_bytes()
                        {
                            lock(&state).timing_replies += 1;
                        }
                    });
                }
                extra = format!(
                    "Transport: RTP/AVP/UDP;unicast;mode=record;server_port={audio_port};control_port={control_port};timing_port={timing_port}\r\nSession: 1\r\n"
                );
            }
            "RECORD" => extra = "Audio-Latency: 11025\r\n".to_string(),
            "SET_PARAMETER" => {
                let text = String::from_utf8_lossy(&body).to_string();
                if let Some(value) = text
                    .strip_prefix("volume:")
                    .and_then(|value| value.trim().parse::<f32>().ok())
                {
                    lock(&state).volumes.push(value);
                }
            }
            "FLUSH" => lock(&state).flushes += 1,
            "TEARDOWN" => lock(&state).teardowns += 1,
            _ => {}
        }
        let response =
            format!("RTSP/1.0 {status}\r\nCSeq: {cseq}\r\nServer: AirTunes/105.1\r\n{extra}\r\n");
        if writer.write_all(response.as_bytes()).is_err() || method == "TEARDOWN" {
            return;
        }
    }
}
