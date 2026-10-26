//! An AirPlay (RAOP) sender session: RTSP control over TCP, audio as RTP
//! over UDP (verbatim ALAC, 44.1 kHz/16-bit stereo), sync packets on the
//! control port, NTP-style answers on the timing port and retransmission of
//! packets the receiver missed.
//!
//! This is AirPlay 1 without encryption, which AirPort Express, most AV
//! receivers and speakers with AirPlay, and shairport-sync accept. Receivers
//! that only speak AirPlay 2 with HomeKit pairing (HomePod, Apple TV) refuse
//! it; discovery marks them unsupported instead of failing at play time.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::alac::{self, FRAMES_PER_PACKET};

pub const SAMPLE_RATE: u32 = 44_100;
/// How far behind "now" receivers play (iTunes' 2 s).
pub const LATENCY_FRAMES: u32 = 88_200;
/// Audio sent ahead of real time.
const LEAD: Duration = Duration::from_millis(300);
const RETRANSMIT_BUFFER: usize = 1_024;
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Seconds between 1900 (NTP) and 1970 (Unix).
const NTP_UNIX_OFFSET: u64 = 2_208_988_800;

pub fn ntp_now() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = now.as_secs() + NTP_UNIX_OFFSET;
    let fraction = (u64::from(now.subsec_nanos()) << 32) / 1_000_000_000;
    (seconds << 32) | fraction
}

/// An RTSP response.
#[derive(Debug, Default)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// An RTSP message: first line, headers, body.
pub type Message = (String, Vec<(String, String)>, Vec<u8>);

/// Reads one RTSP message (request or response) head and body.
pub fn read_message<R: BufRead>(reader: &mut R) -> std::io::Result<Message> {
    let mut first = String::new();
    while first.trim().is_empty() {
        first.clear();
        if reader.read_line(&mut first)? == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
    }
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            headers.push((key.trim().to_string(), value.trim().to_string()));
        }
        if headers.len() > 64 {
            return Err(std::io::Error::other("too many headers"));
        }
    }
    let length = headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0)
        .min(1 << 20);
    let mut body = vec![0_u8; length];
    reader.read_exact(&mut body)?;
    Ok((first.trim().to_string(), headers, body))
}

fn rtp_header(marker: bool, payload_type: u8, seq: u16, timestamp: u32, ssrc: u32) -> [u8; 12] {
    let mut header = [0_u8; 12];
    header[0] = 0x80;
    header[1] = payload_type | if marker { 0x80 } else { 0 };
    header[2..4].copy_from_slice(&seq.to_be_bytes());
    header[4..8].copy_from_slice(&timestamp.to_be_bytes());
    header[8..12].copy_from_slice(&ssrc.to_be_bytes());
    header
}

struct Shared {
    /// Recent packets by sequence number, for retransmission.
    sent: Mutex<VecDeque<(u16, Vec<u8>)>>,
    stop: AtomicBool,
}

/// One playback session on a receiver.
pub struct Session {
    control: BufReader<TcpStream>,
    url: String,
    cseq: u32,
    session: Option<String>,
    client_instance: String,
    audio: UdpSocket,
    control_socket: UdpSocket,
    audio_target: SocketAddr,
    control_target: SocketAddr,
    shared: Arc<Shared>,
    ssrc: u32,
    seq: u16,
    /// RTP time of the first frame since the last flush.
    base_rtp: u32,
    /// Frames sent since the last flush.
    frames_sent: u64,
    /// When the first frame since the last flush was sent.
    started: Option<Instant>,
    pending: Vec<u8>,
    first_packet: bool,
    last_sync: Option<Instant>,
    /// Audio latency the receiver asked for on top of ours.
    pub receiver_latency: u32,
}

fn random_u32() -> u32 {
    rand::random()
}

impl Session {
    /// Connects and sets up a stream: OPTIONS, ANNOUNCE (ALAC 44.1/16),
    /// SETUP (UDP ports), RECORD.
    pub fn open(receiver: SocketAddr) -> Result<Self, String> {
        let stream = TcpStream::connect_timeout(&receiver, IO_TIMEOUT)
            .map_err(|error| format!("could not reach the AirPlay receiver: {error}"))?;
        let _ = stream.set_nodelay(true);
        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .map_err(|error| error.to_string())?;
        let local = stream.local_addr().map_err(|error| error.to_string())?;
        let bind = if local.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        let udp = || UdpSocket::bind(bind).map_err(|error| error.to_string());
        let audio = udp()?;
        let control_socket = udp()?;
        let timing = udp()?;
        let session_id = random_u32();
        let url = format!("rtsp://{}/{session_id}", local.ip());
        let mut session = Self {
            control: BufReader::new(stream),
            url,
            cseq: 0,
            session: None,
            client_instance: format!("{:016X}", rand::random::<u64>()),
            audio,
            audio_target: receiver,
            control_target: receiver,
            control_socket,
            shared: Arc::new(Shared {
                sent: Mutex::new(VecDeque::new()),
                stop: AtomicBool::new(false),
            }),
            ssrc: random_u32(),
            seq: rand::random(),
            base_rtp: random_u32(),
            frames_sent: 0,
            started: None,
            pending: Vec::new(),
            first_packet: true,
            last_sync: None,
            receiver_latency: 0,
        };
        session.request("OPTIONS", "*", &[], None)?;
        let sdp = format!(
            "v=0\r\no=iTunes {session_id} 0 IN IP4 {local_ip}\r\ns=iTunes\r\nc=IN IP4 {remote_ip}\r\nt=0 0\r\nm=audio 0 RTP/AVP 96\r\na=rtpmap:96 AppleLossless\r\na=fmtp:{fmtp}\r\n",
            local_ip = local.ip(),
            remote_ip = receiver.ip(),
            fmtp = alac::FMTP,
        );
        let url = session.url.clone();
        session.request(
            "ANNOUNCE",
            &url,
            &[("Content-Type", "application/sdp")],
            Some(sdp.as_bytes()),
        )?;
        let control_port = session
            .control_socket
            .local_addr()
            .map_err(|error| error.to_string())?
            .port();
        let timing_port = timing
            .local_addr()
            .map_err(|error| error.to_string())?
            .port();
        let transport = format!(
            "RTP/AVP/UDP;unicast;interleaved=0-1;mode=record;control_port={control_port};timing_port={timing_port}"
        );
        let setup = session.request("SETUP", &url, &[("Transport", &transport)], None)?;
        let reply = setup
            .header("Transport")
            .ok_or("the receiver sent no transport")?
            .to_string();
        let port = |name: &str| -> Option<u16> {
            reply.split(';').find_map(|part| {
                let (key, value) = part.split_once('=')?;
                (key.trim() == name)
                    .then(|| value.trim().parse().ok())
                    .flatten()
            })
        };
        let server_port = port("server_port").ok_or("the receiver sent no audio port")?;
        session.audio_target = SocketAddr::new(receiver.ip(), server_port);
        if let Some(control) = port("control_port") {
            session.control_target = SocketAddr::new(receiver.ip(), control);
        }
        session.session = setup
            .header("Session")
            .map(|value| value.split(';').next().unwrap_or(value).trim().to_string());
        session.spawn_timing(timing);
        session.spawn_retransmit()?;
        let rtp_info = format!("seq={};rtptime={}", session.seq, session.base_rtp);
        let record = session.request(
            "RECORD",
            &url,
            &[("Range", "npt=0-"), ("RTP-Info", &rtp_info)],
            None,
        )?;
        session.receiver_latency = record
            .header("Audio-Latency")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        Ok(session)
    }

    /// Sends a request and reads the reply; non-200 is an error.
    fn request(
        &mut self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> Result<Response, String> {
        self.cseq += 1;
        let mut message = format!(
            "{method} {url} RTSP/1.0\r\nCSeq: {}\r\nUser-Agent: Parson/1.0\r\nClient-Instance: {}\r\nDACP-ID: {}\r\nActive-Remote: {}\r\n",
            self.cseq, self.client_instance, self.client_instance, self.ssrc
        );
        if let Some(session) = &self.session {
            message.push_str(&format!("Session: {session}\r\n"));
        }
        for (key, value) in headers {
            message.push_str(&format!("{key}: {value}\r\n"));
        }
        if let Some(body) = body {
            message.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        message.push_str("\r\n");
        let mut bytes = message.into_bytes();
        if let Some(body) = body {
            bytes.extend_from_slice(body);
        }
        self.control
            .get_mut()
            .write_all(&bytes)
            .map_err(|error| format!("{method}: {error}"))?;
        let (status_line, headers, body) =
            read_message(&mut self.control).map_err(|error| format!("{method}: {error}"))?;
        let status = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        let response = Response {
            status,
            headers,
            body,
        };
        match status {
            200 => Ok(response),
            401 | 403 | 470 => Err(format!(
                "{method}: the receiver wants a password or AirPlay 2 pairing ({status})"
            )),
            _ => Err(format!("{method}: {status_line}")),
        }
    }

    /// Answers the receiver's timing requests with our clock.
    fn spawn_timing(&self, socket: UdpSocket) {
        let shared = self.shared.clone();
        let _ = socket.set_read_timeout(Some(Duration::from_millis(500)));
        let _ = std::thread::Builder::new()
            .name("parson-airplay-timing".into())
            .spawn(move || {
                let mut buffer = [0_u8; 128];
                while !shared.stop.load(Ordering::Relaxed) {
                    let Ok((length, from)) = socket.recv_from(&mut buffer) else {
                        continue;
                    };
                    let received = ntp_now();
                    if length < 32 || buffer[1] & 0x7f != 0x52 {
                        continue;
                    }
                    let mut reply = [0_u8; 32];
                    reply[0] = 0x80;
                    reply[1] = 0xd3;
                    reply[2..4].copy_from_slice(&7_u16.to_be_bytes());
                    // Originate = the request's transmit time.
                    reply[8..16].copy_from_slice(&buffer[24..32]);
                    reply[16..24].copy_from_slice(&received.to_be_bytes());
                    reply[24..32].copy_from_slice(&ntp_now().to_be_bytes());
                    let _ = socket.send_to(&reply, from);
                }
            });
    }

    /// Resends packets the receiver reports missing.
    fn spawn_retransmit(&self) -> Result<(), String> {
        let socket = self
            .control_socket
            .try_clone()
            .map_err(|error| error.to_string())?;
        let _ = socket.set_read_timeout(Some(Duration::from_millis(500)));
        let shared = self.shared.clone();
        let _ = std::thread::Builder::new()
            .name("parson-airplay-resend".into())
            .spawn(move || {
                let mut buffer = [0_u8; 64];
                while !shared.stop.load(Ordering::Relaxed) {
                    let Ok((length, from)) = socket.recv_from(&mut buffer) else {
                        continue;
                    };
                    if length < 8 || buffer[1] & 0x7f != 0x55 {
                        continue;
                    }
                    let first = u16::from_be_bytes([buffer[4], buffer[5]]);
                    let count = u16::from_be_bytes([buffer[6], buffer[7]]).min(256);
                    let sent = shared
                        .sent
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    for offset in 0..count {
                        let seq = first.wrapping_add(offset);
                        if let Some((_, packet)) =
                            sent.iter().find(|(candidate, _)| *candidate == seq)
                        {
                            let mut resend = Vec::with_capacity(packet.len() + 4);
                            resend.extend_from_slice(&[0x80, 0xd6]);
                            resend.extend_from_slice(&1_u16.to_be_bytes());
                            resend.extend_from_slice(packet);
                            let _ = socket.send_to(&resend, from);
                        }
                    }
                }
            });
        Ok(())
    }

    fn rtp_now(&self) -> u32 {
        let elapsed = self
            .started
            .map(|started| (started.elapsed().as_secs_f64() * f64::from(SAMPLE_RATE)) as u64)
            .unwrap_or(0);
        self.base_rtp.wrapping_add(elapsed as u32)
    }

    fn send_sync(&mut self) {
        let now = self.rtp_now();
        let mut packet = [0_u8; 20];
        packet[0] = if self.last_sync.is_none() { 0x90 } else { 0x80 };
        packet[1] = 0xd4;
        packet[2..4].copy_from_slice(&7_u16.to_be_bytes());
        packet[4..8].copy_from_slice(&now.wrapping_sub(LATENCY_FRAMES).to_be_bytes());
        packet[8..16].copy_from_slice(&ntp_now().to_be_bytes());
        packet[16..20].copy_from_slice(&now.to_be_bytes());
        let _ = self.control_socket.send_to(&packet, self.control_target);
        self.last_sync = Some(Instant::now());
    }

    fn send_packet(&mut self, pcm: &[u8]) -> Result<(), String> {
        if self.started.is_none() {
            self.started = Some(Instant::now());
        }
        if self
            .last_sync
            .is_none_or(|last| last.elapsed() >= Duration::from_secs(1))
        {
            self.send_sync();
        }
        let timestamp = self.base_rtp.wrapping_add(self.frames_sent as u32);
        let mut packet =
            rtp_header(self.first_packet, 0x60, self.seq, timestamp, self.ssrc).to_vec();
        packet.extend_from_slice(&alac::encode(pcm));
        self.audio
            .send_to(&packet, self.audio_target)
            .map_err(|error| format!("AirPlay audio: {error}"))?;
        {
            let mut sent = self
                .shared
                .sent
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if sent.len() == RETRANSMIT_BUFFER {
                sent.pop_front();
            }
            sent.push_back((self.seq, packet));
        }
        self.first_packet = false;
        self.seq = self.seq.wrapping_add(1);
        self.frames_sent += (pcm.len() / 4) as u64;
        Ok(())
    }

    /// Sends s16 stereo PCM, paced to real time a little ahead.
    pub fn write(&mut self, pcm: &[u8]) -> Result<(), String> {
        self.pending.extend_from_slice(pcm);
        let packet_bytes = FRAMES_PER_PACKET * 4;
        while self.pending.len() >= packet_bytes {
            self.pace();
            let packet: Vec<u8> = self.pending.drain(..packet_bytes).collect();
            self.send_packet(&packet)?;
        }
        Ok(())
    }

    fn pace(&self) {
        let Some(started) = self.started else {
            return;
        };
        let due = Duration::from_secs_f64(self.frames_sent as f64 / f64::from(SAMPLE_RATE));
        let ahead = due.saturating_sub(started.elapsed());
        if ahead > LEAD {
            std::thread::sleep(ahead - LEAD);
        }
    }

    /// Frames sent but not yet heard (heard = played out after latency).
    pub fn delay_frames(&self) -> u64 {
        let Some(started) = self.started else {
            return self.pending.len() as u64 / 4;
        };
        let elapsed = (started.elapsed().as_secs_f64() * f64::from(SAMPLE_RATE)) as u64;
        let heard = elapsed.saturating_sub(u64::from(LATENCY_FRAMES + self.receiver_latency));
        (self.frames_sent + self.pending.len() as u64 / 4)
            .saturating_sub(heard.min(self.frames_sent))
    }

    /// Stops what is queued on the receiver; the next write starts afresh.
    pub fn flush(&mut self) -> Result<(), String> {
        self.pending.clear();
        let next_rtp = self.base_rtp.wrapping_add(self.frames_sent as u32);
        let rtp_info = format!("seq={};rtptime={next_rtp}", self.seq);
        let url = self.url.clone();
        let result = self
            .request("FLUSH", &url, &[("RTP-Info", &rtp_info)], None)
            .map(|_| ());
        self.base_rtp = next_rtp;
        self.frames_sent = 0;
        self.started = None;
        self.first_packet = true;
        self.last_sync = None;
        result
    }

    /// Pads the last packet with silence and waits until it is heard.
    pub fn drain(&mut self) {
        if !self.pending.is_empty() {
            let mut last = std::mem::take(&mut self.pending);
            last.resize(FRAMES_PER_PACKET * 4, 0);
            let _ = self.send_packet(&last);
        }
        let wait = Duration::from_secs_f64(self.delay_frames() as f64 / f64::from(SAMPLE_RATE));
        std::thread::sleep(wait.min(Duration::from_secs(5)));
    }

    /// AirPlay volume: -30..0 dB, -144 mutes.
    pub fn set_volume(&mut self, db: f32) -> Result<(), String> {
        let value = if db <= -30.0 { -144.0 } else { db.min(0.0) };
        let body = format!("volume: {value:.6}\r\n");
        let url = self.url.clone();
        self.request(
            "SET_PARAMETER",
            &url,
            &[("Content-Type", "text/parameters")],
            Some(body.as_bytes()),
        )
        .map(|_| ())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        let url = self.url.clone();
        let _ = self.request("TEARDOWN", &url, &[], None);
    }
}

#[cfg(test)]
mod tests {
    use super::{ntp_now, rtp_header};

    #[test]
    fn headers_and_clock_are_well_formed() {
        let header = rtp_header(true, 0x60, 0xfffe, 0x0102_0304, 7);
        assert_eq!(&header[..4], &[0x80, 0xe0, 0xff, 0xfe]);
        assert_eq!(&header[4..8], &[1, 2, 3, 4]);
        let seconds = ntp_now() >> 32;
        // Some time after 2020 in NTP seconds.
        assert!(seconds > 3_786_825_600, "{seconds}");
    }
}
