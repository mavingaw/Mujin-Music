//! AirPlay receivers, through AirPlay 1 (RAOP).
//!
//! RAOP carries one format: ALAC at 44.1 kHz/16 bit stereo, 352 frames
//! per RTP packet. Parson runs its own chain first (decode, DSP, the
//! resample to 44.1 kHz and the dither to 16 bit, all shown in the signal
//! path), then sends each packet as an uncompressed ("escape") ALAC frame,
//! which keeps every sample exact and costs the receiver no decoding work.
//!
//! The session is RTSP (`ANNOUNCE` with the ALAC format, `SETUP` for the
//! UDP ports, `RECORD`, `FLUSH` on pause or seek, `TEARDOWN`), with RTP
//! audio paced in real time, a sync packet every second on the control
//! port, answers to the receiver's NTP-style timing requests and to its
//! retransmit requests.
//!
//! Receivers are found with mDNS (`_raop._tcp`), whose TXT record says
//! what they accept. Receivers that require RSA or FairPlay encryption, a
//! password, or HomeKit pairing are listed with the reason, but Parson
//! does not implement those and cannot play to them.

use std::collections::{HashMap, VecDeque};
use std::io::{self, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::http::read_head;
use super::{
    AIRPLAY_VERIFICATION, EndpointKind, ManualEndpoint, NetworkCapsSource, NetworkInfo,
    NetworkProtocol, NetworkRegistry, TrackInfo, local_ip_for, random_hex,
};
use crate::{Backend, DeviceCaps, DeviceInfo, Driver, SampleFormat, Sink, StreamFormat};

pub const SERVICE_TYPE: &str = "_raop._tcp.local.";
pub const SAMPLE_RATE: u32 = 44_100;
pub const FRAMES_PER_PACKET: usize = 352;
/// Default receiver latency (two seconds), when `RECORD` names none.
pub const DEFAULT_LATENCY: u32 = 88_200;
/// How far ahead of real time packets are sent.
const SEND_AHEAD: Duration = Duration::from_millis(500);
const RTSP_TIMEOUT: Duration = Duration::from_secs(5);
const RESEND_RING: usize = 1024;

/* ------------------------------------------------------------------ */
/* TXT record and identification                                       */
/* ------------------------------------------------------------------ */

/// What a receiver's `_raop._tcp` TXT record says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RaopTxt {
    /// Model, e.g. `AudioAccessory5,1`.
    pub am: Option<String>,
    /// Codecs: 0 PCM, 1 ALAC, 2 AAC, 3 AAC-ELD.
    pub cn: Option<String>,
    /// Encryption types: 0 none, 1 RSA, 3 FairPlay, 5 MFi.
    pub et: Option<String>,
    pub sr: Option<String>,
    pub ss: Option<String>,
    pub ch: Option<String>,
    pub pw: Option<String>,
    pub tp: Option<String>,
    pub vs: Option<String>,
    /// Status flags (bit 3: password, bit 4: PIN pairing).
    pub sf: Option<String>,
}

impl RaopTxt {
    pub fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut txt = Self::default();
        for (key, value) in pairs {
            let value = Some(value.to_string());
            match key.to_ascii_lowercase().as_str() {
                "am" => txt.am = value,
                "cn" => txt.cn = value,
                "et" => txt.et = value,
                "sr" => txt.sr = value,
                "ss" => txt.ss = value,
                "ch" => txt.ch = value,
                "pw" => txt.pw = value,
                "tp" => txt.tp = value,
                "vs" => txt.vs = value,
                "sf" => txt.sf = value,
                _ => {}
            }
        }
        txt
    }

    fn list_has(list: Option<&String>, item: &str) -> Option<bool> {
        list.map(|list| list.split(',').any(|entry| entry.trim() == item))
    }

    /// Why Parson cannot play to this receiver, if it cannot.
    pub fn unsupported_reason(&self) -> Option<String> {
        if self
            .pw
            .as_deref()
            .is_some_and(|pw| pw.eq_ignore_ascii_case("true"))
        {
            return Some("This receiver is password protected; Mujin Music does not implement AirPlay passwords. Turn the password off on the receiver to play to it.".into());
        }
        let flags = self
            .sf
            .as_deref()
            .and_then(|sf| u64::from_str_radix(sf.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0);
        if flags & 0x8 != 0 || flags & 0x10 != 0 {
            return Some("This receiver asks for a password or PIN pairing, which Mujin Music does not implement. Allow access for everyone on the network in the receiver's AirPlay settings.".into());
        }
        if Self::list_has(self.et.as_ref(), "0") == Some(false) {
            return Some(format!(
                "This receiver only accepts encrypted AirPlay (types {}), and Mujin Music sends unencrypted RAOP; it does not implement Apple's RSA or FairPlay keys.",
                self.et.as_deref().unwrap_or("")
            ));
        }
        if Self::list_has(self.cn.as_ref(), "1") == Some(false) {
            return Some(
                "This receiver does not accept ALAC, which Mujin Music sends over AirPlay.".into(),
            );
        }
        if self
            .tp
            .as_deref()
            .is_some_and(|tp| !tp.to_ascii_uppercase().contains("UDP"))
        {
            return Some(
                "This receiver only takes AirPlay over TCP; Mujin Music sends RTP over UDP.".into(),
            );
        }
        None
    }
}

/// `(manufacturer, model, kind)` for an `am` model identifier.
pub fn identify_model(am: Option<&str>) -> (Option<String>, Option<String>, EndpointKind) {
    let Some(am) = am.map(str::trim).filter(|am| !am.is_empty()) else {
        return (None, None, EndpointKind::Speaker);
    };
    let apple = |model: &str, kind| (Some("Apple".to_string()), Some(model.to_string()), kind);
    match am {
        "AudioAccessory1,1" | "AudioAccessory1,2" => apple("HomePod", EndpointKind::Speaker),
        "AudioAccessory5,1" => apple("HomePod mini", EndpointKind::Speaker),
        "AudioAccessory6,1" => apple("HomePod (2nd generation)", EndpointKind::Speaker),
        "AppleTV2,1" => apple("Apple TV (2nd generation)", EndpointKind::Television),
        "AppleTV3,1" | "AppleTV3,2" => apple("Apple TV (3rd generation)", EndpointKind::Television),
        "AppleTV5,3" => apple("Apple TV HD", EndpointKind::Television),
        "AppleTV6,2" => apple("Apple TV 4K", EndpointKind::Television),
        "AppleTV11,1" => apple("Apple TV 4K (2nd generation)", EndpointKind::Television),
        "AppleTV14,1" => apple("Apple TV 4K (3rd generation)", EndpointKind::Television),
        "AirPort4,107" => apple("AirPort Express (1st generation)", EndpointKind::Receiver),
        "AirPort10,115" => apple("AirPort Express (2nd generation)", EndpointKind::Receiver),
        other if other.to_ascii_lowercase().contains("shairport") => (
            None,
            Some("Shairport Sync".to_string()),
            EndpointKind::Computer,
        ),
        other
            if other.starts_with("MacBook")
                || other.starts_with("Mac")
                || other.starts_with("iMac") =>
        {
            apple(other, EndpointKind::Computer)
        }
        other => (None, Some(other.to_string()), EndpointKind::Speaker),
    }
}

/// `AABBCCDDEEFF@Kitchen` into `("AABBCCDDEEFF", "Kitchen")`.
pub fn split_instance(instance: &str) -> (Option<String>, String) {
    let instance = instance.split("._raop").next().unwrap_or(instance);
    match instance.split_once('@') {
        Some((id, name)) if !id.is_empty() && id.chars().all(|c| c.is_ascii_hexdigit()) => {
            (Some(id.to_ascii_uppercase()), name.to_string())
        }
        _ => (None, instance.to_string()),
    }
}

/// AirPlay 1's only format.
pub fn raop_caps() -> DeviceCaps {
    DeviceCaps {
        sample_rates: vec![SAMPLE_RATE],
        sample_formats: vec![SampleFormat::S16],
        min_channels: 2,
        max_channels: 2,
        dsd_formats: Vec::new(),
        dsd_rates: Vec::new(),
    }
}

/// One AirPlay receiver.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receiver {
    pub id: String,
    pub name: String,
    pub address: SocketAddr,
    pub txt: RaopTxt,
    pub manual: bool,
}

pub fn device_info(receiver: &Receiver) -> DeviceInfo {
    let (manufacturer, model, kind) = identify_model(receiver.txt.am.as_deref());
    let unsupported = receiver.txt.unsupported_reason();
    let mut accepts = Vec::new();
    for (code, label) in [("1", "ALAC"), ("0", "PCM"), ("2", "AAC"), ("3", "AAC-ELD")] {
        if RaopTxt::list_has(receiver.txt.cn.as_ref(), code) == Some(true) {
            accepts.push(format!("{label} 44.1kHz/16bit"));
        }
    }
    if accepts.is_empty() {
        accepts.push("ALAC 44.1kHz/16bit".into());
    }
    let mut detail = vec!["AirPlay".to_string()];
    if let Some(model) = &model {
        detail.push(model.clone());
    }
    detail.push(receiver.address.ip().to_string());
    DeviceInfo {
        id: receiver.id.clone(),
        name: receiver.name.clone(),
        detail: detail.join(" · "),
        driver: Driver::AirPlay,
        exclusive_capable: true,
        caps: unsupported.is_none().then(raop_caps),
        probe_error: unsupported,
        is_default: false,
        hardware: None,
        address: Some(receiver.address.to_string()),
        network: Some(NetworkInfo {
            protocol: NetworkProtocol::AirPlay,
            host: receiver.address.ip().to_string(),
            port: receiver.address.port(),
            location: None,
            manufacturer,
            model,
            model_description: receiver
                .txt
                .vs
                .as_ref()
                .map(|vs| format!("AirPlay version {vs}")),
            icon_url: None,
            kind,
            caps_source: NetworkCapsSource::Protocol,
            stream_format: "ALAC 44.1kHz 16bit over RTP (AirPlay 1)".into(),
            accepts,
            reports_format: false,
            manual: receiver.manual,
            verification: AIRPLAY_VERIFICATION.to_string(),
        }),
        hardware_volume: Some(AIRPLAY_VOLUME),
        busy: false,
    }
}

/// A receiver's device id: `airplay:` and its MAC address in lower case,
/// or its address when the instance name has none.
pub fn receiver_id(mac: Option<&str>, address: SocketAddr) -> String {
    format!(
        "airplay:{}",
        mac.map(str::to_ascii_lowercase)
            .unwrap_or_else(|| address.to_string())
    )
}

/* ------------------------------------------------------------------ */
/* ALAC and RTP                                                        */
/* ------------------------------------------------------------------ */

struct BitWriter {
    bytes: Vec<u8>,
    bits: usize,
}

impl BitWriter {
    fn new(capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(capacity),
            bits: 0,
        }
    }

    fn write(&mut self, value: u32, count: usize) {
        for index in (0..count).rev() {
            if self.bits.is_multiple_of(8) {
                self.bytes.push(0);
            }
            let bit = (value >> index) & 1;
            if bit != 0 {
                let last = self.bytes.len() - 1;
                self.bytes[last] |= 0x80 >> (self.bits % 8);
            }
            self.bits += 1;
        }
    }
}

/// One ALAC frame holding `samples` (interleaved stereo 16-bit) verbatim:
/// Apple's encoder writes this "escape" form for incompressible input, and
/// every ALAC decoder reads it. A frame shorter than 352 frames is marked
/// partial and carries its length.
pub fn alac_escape_frame(samples: &[i16]) -> Vec<u8> {
    let frames = samples.len() / 2;
    let mut writer = BitWriter::new(8 + samples.len() * 2);
    writer.write(1, 3); // ID_CPE: a channel pair
    writer.write(0, 4); // element instance tag
    writer.write(0, 12); // unused
    let partial = frames != FRAMES_PER_PACKET;
    // partial frame, bytes shifted (2 bits), escape flag
    writer.write((u32::from(partial) << 3) | 1, 4);
    if partial {
        writer.write(frames as u32, 32);
    }
    for frame in samples.as_chunks::<2>().0 {
        writer.write(u32::from(frame[0] as u16), 16);
        writer.write(u32::from(frame[1] as u16), 16);
    }
    writer.write(7, 3); // ID_END
    writer.bytes
}

pub fn rtp_audio_packet(
    marker: bool,
    seq: u16,
    timestamp: u32,
    ssrc: u32,
    payload: &[u8],
) -> Vec<u8> {
    let mut packet = Vec::with_capacity(12 + payload.len());
    packet.push(0x80);
    packet.push(if marker { 0xE0 } else { 0x60 });
    packet.extend_from_slice(&seq.to_be_bytes());
    packet.extend_from_slice(&timestamp.to_be_bytes());
    packet.extend_from_slice(&ssrc.to_be_bytes());
    packet.extend_from_slice(payload);
    packet
}

/// NTP time (seconds since 1900, 32.32 fixed point).
pub fn ntp_now() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = now.as_secs() + 2_208_988_800;
    let fraction = (u64::from(now.subsec_nanos()) << 32) / 1_000_000_000;
    (seconds << 32) | fraction
}

/// A sync packet: at NTP `now`, the receiver should be playing
/// `rtp_now - latency`.
pub fn sync_packet(first: bool, rtp_now: u32, latency: u32, now: u64) -> [u8; 20] {
    let mut packet = [0u8; 20];
    packet[0] = if first { 0x90 } else { 0x80 };
    packet[1] = 0xD4;
    packet[3] = 0x07;
    packet[4..8].copy_from_slice(&rtp_now.wrapping_sub(latency).to_be_bytes());
    packet[8..16].copy_from_slice(&now.to_be_bytes());
    packet[16..20].copy_from_slice(&rtp_now.to_be_bytes());
    packet
}

/// Answers a timing request (`0xD2`) with a reply (`0xD3`).
pub fn timing_reply(request: &[u8], now: u64) -> Option<[u8; 32]> {
    if request.len() < 32 || request[1] & 0x7F != 0x52 {
        return None;
    }
    let mut reply = [0u8; 32];
    reply[0] = 0x80;
    reply[1] = 0xD3;
    reply[3] = 0x07;
    reply[8..16].copy_from_slice(&request[24..32]);
    reply[16..24].copy_from_slice(&now.to_be_bytes());
    reply[24..32].copy_from_slice(&now.to_be_bytes());
    Some(reply)
}

/* ------------------------------------------------------------------ */
/* RTSP                                                                */
/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtspResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl RtspResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

struct Rtsp {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    cseq: u32,
    url: String,
    client_instance: String,
    session: Option<String>,
    receiver: String,
}

fn status_message(receiver: &str, method: &str, status: u16) -> String {
    match status {
        401 => format!("{receiver} asked for a password, which Mujin Music does not implement"),
        403 => format!(
            "{receiver} refused the connection (its AirPlay access may be limited to certain devices)"
        ),
        453 => format!("{receiver} is busy: another sender is playing to it"),
        470 => format!(
            "{receiver} requires AirPlay pairing (HomeKit or device verification), which Mujin Music does not implement"
        ),
        _ => format!("{receiver} answered RTSP {status} to {method}"),
    }
}

impl Rtsp {
    fn connect(address: SocketAddr, local_ip: IpAddr, receiver: &str) -> io::Result<Self> {
        let stream = TcpStream::connect_timeout(&address, RTSP_TIMEOUT)?;
        stream.set_read_timeout(Some(RTSP_TIMEOUT))?;
        stream.set_write_timeout(Some(RTSP_TIMEOUT))?;
        let writer = stream.try_clone()?;
        let session_number = u64::from_str_radix(&random_hex(4), 16).unwrap_or(1);
        Ok(Self {
            reader: BufReader::new(stream),
            writer,
            cseq: 0,
            url: format!("rtsp://{local_ip}/{session_number}"),
            client_instance: random_hex(8).to_ascii_uppercase(),
            session: None,
            receiver: receiver.to_string(),
        })
    }

    fn request(
        &mut self,
        method: &str,
        target: Option<&str>,
        headers: &[(&str, String)],
        content_type: Option<&str>,
        body: &[u8],
    ) -> io::Result<RtspResponse> {
        self.cseq += 1;
        let mut head = format!(
            "{method} {} RTSP/1.0\r\nCSeq: {}\r\nUser-Agent: Parson/1.0\r\nClient-Instance: {}\r\nDACP-ID: {}\r\nActive-Remote: 1986535575\r\n",
            target.unwrap_or(&self.url),
            self.cseq,
            self.client_instance,
            self.client_instance
        );
        if let Some(session) = &self.session {
            head.push_str(&format!("Session: {session}\r\n"));
        }
        for (name, value) in headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        if let Some(content_type) = content_type {
            head.push_str(&format!("Content-Type: {content_type}\r\n"));
        }
        if !body.is_empty() {
            head.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        head.push_str("\r\n");
        self.writer.write_all(head.as_bytes())?;
        self.writer.write_all(body)?;
        self.writer.flush()?;
        let (status_line, headers) = read_head(&mut self.reader)?;
        let status = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse::<u16>().ok())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("malformed RTSP status: {status_line}"),
                )
            })?;
        let length = headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("Content-Length"))
            .and_then(|(_, value)| value.parse::<usize>().ok())
            .unwrap_or(0)
            .min(64 * 1024);
        let mut body = vec![0; length];
        self.reader.read_exact(&mut body)?;
        let response = RtspResponse {
            status,
            headers,
            body,
        };
        if response.status != 200 {
            return Err(io::Error::new(
                if matches!(status, 401 | 403 | 470) {
                    io::ErrorKind::PermissionDenied
                } else {
                    io::ErrorKind::ConnectionRefused
                },
                status_message(&self.receiver, method, status),
            ));
        }
        Ok(response)
    }
}

/// Reads `name=value` pairs from an RTSP `Transport` header.
pub fn transport_port(transport: &str, name: &str) -> Option<u16> {
    transport.split(';').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        (key.trim() == name)
            .then(|| value.trim().parse().ok())
            .flatten()
    })
}

/// The ANNOUNCE body for ALAC 44.1 kHz/16 bit stereo.
pub fn announce_sdp(session: u64, local_ip: IpAddr, receiver_ip: IpAddr) -> String {
    let family = |ip: IpAddr| if ip.is_ipv4() { "IP4" } else { "IP6" };
    format!(
        "v=0\r\no=iTunes {session} 0 IN {} {local_ip}\r\ns=iTunes\r\nc=IN {} {receiver_ip}\r\nt=0 0\r\nm=audio 0 RTP/AVP 96\r\na=rtpmap:96 AppleLossless\r\na=fmtp:96 {FRAMES_PER_PACKET} 0 16 40 10 14 2 255 0 0 {SAMPLE_RATE}\r\n",
        family(local_ip),
        family(receiver_ip)
    )
}

/* ------------------------------------------------------------------ */
/* Sink                                                                */
/* ------------------------------------------------------------------ */

struct Timeline {
    /// When the current run's first packet was sent.
    start: Option<Instant>,
    /// RTP time of the run's first packet.
    rtp_start: u32,
    /// The first sync of a run carries the extension bit.
    first_sync: bool,
}

struct Shared {
    timeline: Mutex<Timeline>,
    ring: Mutex<VecDeque<(u16, Vec<u8>)>>,
    stop: AtomicBool,
    control: UdpSocket,
    timing: UdpSocket,
    remote_control: SocketAddr,
    latency: u32,
}

impl Shared {
    fn timeline(&self) -> std::sync::MutexGuard<'_, Timeline> {
        self.timeline
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn send_sync(&self) {
        let packet = {
            let mut timeline = self.timeline();
            let Some(start) = timeline.start else {
                return;
            };
            let elapsed = start.elapsed().as_secs_f64();
            let rtp_now = timeline
                .rtp_start
                .wrapping_add((elapsed * f64::from(SAMPLE_RATE)) as u32);
            let packet = sync_packet(timeline.first_sync, rtp_now, self.latency, ntp_now());
            timeline.first_sync = false;
            packet
        };
        let _ = self.control.send_to(&packet, self.remote_control);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The receiver's own volume, in dB on AirPlay's -30..0 scale; at or below
/// -30 dB (or muted) it is sent as -144, AirPlay's mute.
pub fn volume_parameter(db: f32, muted: bool) -> String {
    let value = if muted || db <= -30.0 {
        -144.0
    } else {
        db.min(0.0)
    };
    format!("volume: {value:.6}\r\n")
}

/// The volume scale every AirPlay receiver takes.
pub const AIRPLAY_VOLUME: crate::HardwareVolume = crate::HardwareVolume {
    min_db: -30.0,
    max_db: 0.0,
    step_db: 0.5,
};

/// An RTSP session and its RTP stream to one receiver.
pub struct RaopSink {
    rtsp: Arc<Mutex<Rtsp>>,
    audio: UdpSocket,
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
    seq: u16,
    rtptime: u32,
    ssrc: u32,
    marker: bool,
    pending: Vec<i16>,
    frames_in_run: u64,
}

fn random_u32() -> u32 {
    u32::from_str_radix(&random_hex(4), 16).unwrap_or(0x5041_5253)
}

impl RaopSink {
    /// Opens the RTSP session and sets up the RTP ports.
    pub fn open(receiver: &Receiver, track: Option<&TrackInfo>) -> io::Result<Self> {
        let local_ip = local_ip_for(receiver.address)?;
        let mut rtsp = Rtsp::connect(receiver.address, local_ip, &receiver.name)?;
        rtsp.request("OPTIONS", Some("*"), &[], None, &[])?;
        let session_number = rtsp
            .url
            .rsplit('/')
            .next()
            .and_then(|n| n.parse().ok())
            .unwrap_or(1);
        let sdp = announce_sdp(session_number, local_ip, receiver.address.ip());
        rtsp.request(
            "ANNOUNCE",
            None,
            &[],
            Some("application/sdp"),
            sdp.as_bytes(),
        )?;
        let bind = SocketAddr::new(local_ip, 0);
        let control = UdpSocket::bind(bind)?;
        let timing = UdpSocket::bind(bind)?;
        let transport = format!(
            "RTP/AVP/UDP;unicast;interleaved=0-1;mode=record;control_port={};timing_port={}",
            control.local_addr()?.port(),
            timing.local_addr()?.port()
        );
        let setup = rtsp.request("SETUP", None, &[("Transport", transport)], None, &[])?;
        let reply = setup.header("Transport").unwrap_or("").to_string();
        let server_port = transport_port(&reply, "server_port").ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "SETUP returned no server port")
        })?;
        let control_port = transport_port(&reply, "control_port").unwrap_or(server_port + 1);
        rtsp.session = Some(
            setup
                .header("Session")
                .unwrap_or("1")
                .split(';')
                .next()
                .unwrap_or("1")
                .trim()
                .to_string(),
        );
        let seq = random_u32() as u16;
        let rtptime = random_u32();
        let record = rtsp.request(
            "RECORD",
            None,
            &[
                ("Range", "npt=0-".into()),
                ("RTP-Info", format!("seq={seq};rtptime={rtptime}")),
            ],
            None,
            &[],
        )?;
        let latency = record
            .header("Audio-Latency")
            .and_then(|value| value.trim().parse::<u32>().ok())
            .filter(|latency| (4_410..=441_000).contains(latency))
            .unwrap_or(DEFAULT_LATENCY);
        if let Some(track) = track {
            let mut body = format!("title: {}\r\n", track.title);
            if let Some(artist) = &track.artist {
                body.push_str(&format!("artist: {artist}\r\n"));
            }
            // Text metadata is optional; receivers that reject it still play.
            let _ = rtsp.request(
                "SET_PARAMETER",
                None,
                &[],
                Some("text/parameters"),
                body.as_bytes(),
            );
        }
        let audio = UdpSocket::bind(bind)?;
        audio.connect(SocketAddr::new(receiver.address.ip(), server_port))?;
        control.set_read_timeout(Some(Duration::from_millis(100)))?;
        timing.set_read_timeout(Some(Duration::from_millis(100)))?;
        let shared = Arc::new(Shared {
            timeline: Mutex::new(Timeline {
                start: None,
                rtp_start: rtptime,
                first_sync: true,
            }),
            ring: Mutex::new(VecDeque::with_capacity(RESEND_RING)),
            stop: AtomicBool::new(false),
            control,
            timing,
            remote_control: SocketAddr::new(receiver.address.ip(), control_port),
            latency,
        });
        let mut threads = Vec::new();
        let timing_shared = Arc::clone(&shared);
        threads.push(
            std::thread::Builder::new()
                .name("parson-raop-timing".into())
                .spawn(move || timing_loop(&timing_shared))?,
        );
        let control_shared = Arc::clone(&shared);
        threads.push(
            std::thread::Builder::new()
                .name("parson-raop-control".into())
                .spawn(move || control_loop(&control_shared))?,
        );
        Ok(Self {
            rtsp: Arc::new(Mutex::new(rtsp)),
            audio,
            shared,
            threads,
            seq,
            rtptime,
            ssrc: random_u32(),
            marker: true,
            pending: Vec::with_capacity(FRAMES_PER_PACKET * 2),
            frames_in_run: 0,
        })
    }

    fn send_packet(&mut self, samples: &[i16]) -> io::Result<()> {
        let start = {
            let mut timeline = self.shared.timeline();
            match timeline.start {
                Some(start) => start,
                None => {
                    let now = Instant::now();
                    timeline.start = Some(now);
                    timeline.rtp_start = self.rtptime;
                    timeline.first_sync = true;
                    drop(timeline);
                    // A run starts with a sync, so the receiver can place
                    // the first packet.
                    self.shared.send_sync();
                    now
                }
            }
        };
        let due = Duration::from_secs_f64(self.frames_in_run as f64 / f64::from(SAMPLE_RATE));
        let elapsed = start.elapsed();
        if due > elapsed + SEND_AHEAD {
            std::thread::sleep(due - elapsed - SEND_AHEAD);
        }
        let payload = alac_escape_frame(samples);
        let packet = rtp_audio_packet(self.marker, self.seq, self.rtptime, self.ssrc, &payload);
        self.marker = false;
        self.audio.send(&packet)?;
        {
            let mut ring = self
                .shared
                .ring
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if ring.len() >= RESEND_RING {
                ring.pop_front();
            }
            ring.push_back((self.seq, packet));
        }
        let frames = (samples.len() / 2) as u32;
        self.seq = self.seq.wrapping_add(1);
        self.rtptime = self.rtptime.wrapping_add(frames);
        self.frames_in_run += u64::from(frames);
        Ok(())
    }

    fn send_full_packets(&mut self) -> io::Result<()> {
        let per_packet = FRAMES_PER_PACKET * 2;
        let mut offset = 0;
        while self.pending.len() - offset >= per_packet {
            let packet: Vec<i16> = self.pending[offset..offset + per_packet].to_vec();
            self.send_packet(&packet)?;
            offset += per_packet;
        }
        self.pending.drain(..offset);
        Ok(())
    }

    fn played_frames(&self) -> u64 {
        let timeline = self.shared.timeline();
        let Some(start) = timeline.start else {
            return 0;
        };
        let elapsed = start.elapsed().as_secs_f64() * f64::from(SAMPLE_RATE);
        (elapsed as u64).saturating_sub(u64::from(self.shared.latency))
    }

    /// RTSP FLUSH: the receiver drops what it buffered, and the next
    /// packet starts a new run.
    fn flush_receiver(&mut self) -> io::Result<()> {
        let info = format!("seq={};rtptime={}", self.seq, self.rtptime);
        lock(&self.rtsp).request("FLUSH", None, &[("RTP-Info", info)], None, &[])?;
        let mut timeline = self.shared.timeline();
        timeline.start = None;
        self.marker = true;
        self.frames_in_run = 0;
        Ok(())
    }
}

fn timing_loop(shared: &Shared) {
    let mut buffer = [0u8; 128];
    while !shared.stop.load(Ordering::Relaxed) {
        if let Ok((length, from)) = shared.timing.recv_from(&mut buffer)
            && let Some(reply) = timing_reply(&buffer[..length], ntp_now())
        {
            let _ = shared.timing.send_to(&reply, from);
        }
    }
}

fn control_loop(shared: &Shared) {
    let mut buffer = [0u8; 128];
    let mut last_sync = Instant::now();
    while !shared.stop.load(Ordering::Relaxed) {
        if last_sync.elapsed() >= Duration::from_secs(1) {
            shared.send_sync();
            last_sync = Instant::now();
        }
        let Ok((length, from)) = shared.control.recv_from(&mut buffer) else {
            continue;
        };
        // Retransmit request: 0x80 0xD5, seq, first missing seq, count.
        if length >= 8 && buffer[1] & 0x7F == 0x55 {
            let first = u16::from_be_bytes([buffer[4], buffer[5]]);
            let count = u16::from_be_bytes([buffer[6], buffer[7]]).min(256);
            let ring = shared
                .ring
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for offset in 0..count {
                let wanted = first.wrapping_add(offset);
                if let Some((_, packet)) = ring.iter().find(|(seq, _)| *seq == wanted) {
                    let mut resend = vec![0x80, 0xD6, 0x00, 0x01];
                    resend.extend_from_slice(packet);
                    let _ = shared.control.send_to(&resend, from);
                }
            }
        }
    }
}

impl Sink for RaopSink {
    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        self.pending.extend(
            data.as_chunks::<2>()
                .0
                .iter()
                .map(|bytes| i16::from_le_bytes(*bytes)),
        );
        self.send_full_packets()
    }

    fn delay_frames(&mut self) -> u64 {
        let sent = self.frames_in_run + (self.pending.len() / 2) as u64;
        sent.saturating_sub(self.played_frames())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.pending.clear();
        let had_run = self.shared.timeline().start.is_some();
        if had_run {
            self.flush_receiver()?;
        }
        Ok(())
    }

    fn drain(&mut self) -> io::Result<()> {
        if !self.pending.is_empty() {
            // Pad the last packet with silence to a whole ALAC frame.
            self.pending.resize(FRAMES_PER_PACKET * 2, 0);
            self.send_full_packets()?;
        }
        let total = self.frames_in_run + u64::from(self.shared.latency);
        let deadline = Instant::now() + Duration::from_secs(total / u64::from(SAMPLE_RATE) + 5);
        while self.played_frames() < self.frames_in_run && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    }
}

impl Drop for RaopSink {
    fn drop(&mut self) {
        let _ = lock(&self.rtsp).request("TEARDOWN", None, &[], None, &[]);
        self.shared.stop.store(true, Ordering::Relaxed);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

/* ------------------------------------------------------------------ */
/* Backend                                                             */
/* ------------------------------------------------------------------ */

/// Checks a receiver added by address with an RTSP `OPTIONS`.
pub fn probe_manual(endpoint: &ManualEndpoint) -> Receiver {
    let (host, port) = endpoint.host_port(7000).unwrap_or_default();
    let name = endpoint
        .name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| format!("AirPlay receiver at {host}"));
    let address = (host.as_str(), port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut addresses| addresses.next());
    let id = format!("airplay:manual:{}", endpoint.address.trim());
    let mut txt = RaopTxt::default();
    let Some(address) = address else {
        return Receiver {
            id,
            name,
            address: SocketAddr::from(([0, 0, 0, 0], port)),
            txt: RaopTxt {
                tp: Some("unresolved".into()),
                ..RaopTxt::default()
            },
            manual: true,
        };
    };
    if let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_secs(2)) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let _ =
            stream.write_all(b"OPTIONS * RTSP/1.0\r\nCSeq: 1\r\nUser-Agent: Parson/1.0\r\n\r\n");
        if let Ok((status, headers)) = read_head(&mut BufReader::new(&mut stream)) {
            if !status.starts_with("RTSP/1.0") {
                txt.tp = Some("not RTSP".into());
            }
            if let Some((_, server)) = headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case("Server"))
            {
                txt.vs = server.split('/').nth(1).map(str::to_string);
            }
        } else {
            txt.tp = Some("not RTSP".into());
        }
    } else {
        txt.tp = Some("unreachable".into());
    }
    Receiver {
        id,
        name,
        address,
        txt,
        manual: true,
    }
}

/// Called when the receivers a backend knows change.
pub type ChangeListener = Arc<dyn Fn() + Send + Sync>;

/// Plays to AirPlay receivers.
pub struct RaopBackend {
    registry: Arc<NetworkRegistry>,
    /// Receivers found by mDNS, kept up to date by [`RaopBackend::browse`].
    discovered: Mutex<HashMap<String, Receiver>>,
    /// Every receiver of the last listing, for `open`.
    receivers: Mutex<HashMap<String, Receiver>>,
    tracks: Mutex<HashMap<String, TrackInfo>>,
    /// The receiver's own volume (dB) and mute, kept across sessions.
    volume: Mutex<HashMap<String, f32>>,
    muted: Mutex<HashMap<String, bool>>,
    /// The RTSP connection of each receiver's open session.
    active: Mutex<HashMap<String, std::sync::Weak<Mutex<Rtsp>>>>,
    listener: Mutex<Option<ChangeListener>>,
}

impl RaopBackend {
    /// Receivers added by address (in `registry`) and those
    /// [`RaopBackend::browse`] finds.
    pub fn new(registry: Arc<NetworkRegistry>) -> Self {
        Self {
            registry,
            discovered: Mutex::new(HashMap::new()),
            receivers: Mutex::new(HashMap::new()),
            tracks: Mutex::new(HashMap::new()),
            volume: Mutex::new(HashMap::new()),
            muted: Mutex::new(HashMap::new()),
            active: Mutex::new(HashMap::new()),
            listener: Mutex::new(None),
        }
    }

    /// Calls `listener` whenever a receiver appears or changes.
    pub fn on_change(&self, listener: ChangeListener) {
        *lock(&self.listener) = Some(listener);
    }

    fn changed(&self) {
        let listener = lock(&self.listener).clone();
        if let Some(listener) = listener {
            listener();
        }
    }

    /// Adds or updates a receiver (as mDNS found it); true when something
    /// changed.
    pub fn upsert(&self, receiver: Receiver) -> bool {
        let changed = {
            let mut discovered = lock(&self.discovered);
            if discovered.get(&receiver.id) == Some(&receiver) {
                false
            } else {
                discovered.insert(receiver.id.clone(), receiver);
                true
            }
        };
        if changed {
            self.changed();
        }
        changed
    }

    /// Every receiver found so far, playable or not, by name.
    pub fn found(&self) -> Vec<Receiver> {
        let mut receivers: Vec<Receiver> = lock(&self.discovered).values().cloned().collect();
        receivers.sort_by_key(|receiver| receiver.name.to_lowercase());
        receivers
    }

    /// Keeps browsing mDNS in the background for as long as the backend
    /// lives, adding receivers as they answer.
    pub fn browse(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let _ = std::thread::Builder::new()
            .name("parson-airplay-browse".into())
            .spawn(move || {
                let Ok(daemon) = mdns_sd::ServiceDaemon::new() else {
                    return;
                };
                let Ok(events) = daemon.browse(SERVICE_TYPE) else {
                    let _ = daemon.shutdown();
                    return;
                };
                loop {
                    let event = events.recv_timeout(Duration::from_secs(5));
                    let Some(backend) = weak.upgrade() else {
                        let _ = daemon.shutdown();
                        return;
                    };
                    let Ok(mdns_sd::ServiceEvent::ServiceResolved(info)) = event else {
                        continue;
                    };
                    let Some(ip) = info
                        .get_addresses_v4()
                        .into_iter()
                        .find(|ip| !ip.is_unspecified() && !ip.is_loopback())
                    else {
                        continue;
                    };
                    let txt = RaopTxt::from_pairs(
                        info.get_properties()
                            .iter()
                            .map(|property| (property.key(), property.val_str())),
                    );
                    let (mac, name) = split_instance(info.get_fullname());
                    let address = SocketAddr::new(IpAddr::V4(ip), info.get_port());
                    backend.upsert(Receiver {
                        id: receiver_id(mac.as_deref(), address),
                        name,
                        address,
                        txt,
                        manual: false,
                    });
                }
            });
    }

    /// Sends the stored volume to the receiver's open session, if any.
    fn apply_volume(&self, device_id: &str) -> io::Result<()> {
        let Some(rtsp) = lock(&self.active)
            .get(device_id)
            .and_then(std::sync::Weak::upgrade)
        else {
            return Ok(());
        };
        let db = lock(&self.volume).get(device_id).copied().unwrap_or(-15.0);
        let muted = lock(&self.muted).get(device_id).copied().unwrap_or(false);
        lock(&rtsp)
            .request(
                "SET_PARAMETER",
                None,
                &[],
                Some("text/parameters"),
                volume_parameter(db, muted).as_bytes(),
            )
            .map(|_| ())
    }
}

impl Backend for RaopBackend {
    fn driver(&self) -> Driver {
        Driver::AirPlay
    }

    fn devices(&self) -> Vec<DeviceInfo> {
        let manual: Vec<ManualEndpoint> = self
            .registry
            .manual()
            .into_iter()
            .filter(|endpoint| endpoint.protocol == NetworkProtocol::AirPlay)
            .collect();
        let manual_receivers: Vec<Receiver> = std::thread::scope(|scope| {
            let probed: Vec<_> = manual
                .iter()
                .map(|endpoint| scope.spawn(move || probe_manual(endpoint)))
                .collect();
            probed
                .into_iter()
                .filter_map(|handle| handle.join().ok())
                .collect()
        });
        let mut receivers = self.found();
        for receiver in manual_receivers {
            if !receivers
                .iter()
                .any(|known| known.address == receiver.address)
            {
                receivers.push(receiver);
            }
        }
        let mut devices: Vec<DeviceInfo> = receivers
            .iter()
            .map(|receiver| {
                let mut device = device_info(receiver);
                if receiver.manual {
                    match receiver.txt.tp.as_deref() {
                        Some("unreachable") | Some("unresolved") => {
                            device.caps = None;
                            device.probe_error = Some(format!(
                                "Mujin Music could not reach an AirPlay receiver at {}.",
                                receiver.address
                            ));
                        }
                        Some("not RTSP") => {
                            device.caps = None;
                            device.probe_error = Some(format!(
                                "{} answered, but not as an AirPlay (RTSP) receiver.",
                                receiver.address
                            ));
                        }
                        _ => {}
                    }
                }
                device
            })
            .collect();
        devices.sort_by_key(|device| device.name.to_lowercase());
        *lock(&self.receivers) = receivers
            .into_iter()
            .map(|receiver| (receiver.id.clone(), receiver))
            .collect();
        devices
    }

    fn prepare(&self, device_id: &str, track: &TrackInfo) {
        lock(&self.tracks).insert(device_id.to_string(), track.clone());
    }

    fn open(
        &self,
        device_id: &str,
        format: StreamFormat,
        _exclusive: bool,
    ) -> io::Result<Box<dyn Sink>> {
        if format.sample_rate != SAMPLE_RATE
            || format.sample_format != SampleFormat::S16
            || format.channels != 2
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "AirPlay carries 44.1kHz 16-bit stereo only",
            ));
        }
        let receiver = lock(&self.receivers)
            .get(device_id)
            .cloned()
            .or_else(|| lock(&self.discovered).get(device_id).cloned())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "that AirPlay receiver is no longer on the network",
                )
            })?;
        if let Some(reason) = receiver.txt.unsupported_reason() {
            return Err(io::Error::new(io::ErrorKind::Unsupported, reason));
        }
        let track = lock(&self.tracks).get(device_id).cloned();
        let sink = RaopSink::open(&receiver, track.as_ref())?;
        lock(&self.active).insert(device_id.to_string(), Arc::downgrade(&sink.rtsp));
        // The receiver starts at its own level; set Parson's.
        let _ = self.apply_volume(device_id);
        Ok(Box::new(sink))
    }

    fn set_hardware_volume(&self, device_id: &str, db: f32) -> io::Result<()> {
        lock(&self.volume).insert(device_id.to_string(), db);
        self.apply_volume(device_id)
    }

    fn set_hardware_mute(&self, device_id: &str, muted: bool) -> io::Result<()> {
        lock(&self.muted).insert(device_id.to_string(), muted);
        self.apply_volume(device_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads an escape frame back the way Apple's ALAC decoder does.
    pub(crate) fn decode_escape(frame: &[u8]) -> Vec<i16> {
        super::super::fakes::decode_alac_escape(frame).expect("valid escape frame")
    }

    #[test]
    fn escape_frames_round_trip_full_and_partial() {
        let samples: Vec<i16> = (0..FRAMES_PER_PACKET * 2)
            .map(|index| (index as i32 * 97 - 30_000) as i16)
            .collect();
        let frame = alac_escape_frame(&samples);
        // 3+4+12+4 header bits, 352×32 sample bits, 3 end bits.
        assert_eq!(frame.len(), (23 + FRAMES_PER_PACKET * 32 + 3).div_ceil(8));
        assert_eq!(decode_escape(&frame), samples);
        let short = &samples[..100];
        assert_eq!(decode_escape(&alac_escape_frame(short)), short);
        assert_eq!(
            decode_escape(&alac_escape_frame(&[i16::MIN, i16::MAX])),
            [i16::MIN, i16::MAX]
        );
    }

    #[test]
    fn txt_records_decide_what_is_playable() {
        let homepod = RaopTxt::from_pairs([
            ("am", "AudioAccessory5,1"),
            ("cn", "0,1,2,3"),
            ("et", "0,3,5"),
            ("tp", "UDP"),
            ("sf", "0x4"),
        ]);
        assert!(homepod.unsupported_reason().is_none());
        assert_eq!(
            identify_model(homepod.am.as_deref()).1.as_deref(),
            Some("HomePod mini")
        );
        let encrypted = RaopTxt::from_pairs([("et", "1"), ("cn", "1")]);
        assert!(
            encrypted
                .unsupported_reason()
                .unwrap()
                .contains("encrypted")
        );
        let password = RaopTxt::from_pairs([("pw", "true")]);
        assert!(password.unsupported_reason().unwrap().contains("password"));
        let pin = RaopTxt::from_pairs([("sf", "0x204")]);
        assert!(pin.unsupported_reason().is_none());
        let pin = RaopTxt::from_pairs([("sf", "0x8")]);
        assert!(pin.unsupported_reason().unwrap().contains("PIN"));
        assert_eq!(
            split_instance("AABBCCDDEEFF@Kitchen._raop._tcp.local."),
            (Some("AABBCCDDEEFF".into()), "Kitchen".into())
        );
        assert_eq!(split_instance("Den"), (None, "Den".into()));
    }

    #[test]
    fn sync_timing_and_transport_parse() {
        let sync = sync_packet(true, 100_000, DEFAULT_LATENCY, 0x0102_0304_0506_0708);
        assert_eq!(sync[0], 0x90);
        assert_eq!(sync[1], 0xD4);
        assert_eq!(
            u32::from_be_bytes(sync[4..8].try_into().unwrap()),
            100_000 - DEFAULT_LATENCY
        );
        assert_eq!(
            u32::from_be_bytes(sync[16..20].try_into().unwrap()),
            100_000
        );
        let mut request = [0u8; 32];
        request[0] = 0x80;
        request[1] = 0xD2;
        request[24..32].copy_from_slice(&42u64.to_be_bytes());
        let reply = timing_reply(&request, 7).unwrap();
        assert_eq!(reply[1], 0xD3);
        assert_eq!(u64::from_be_bytes(reply[8..16].try_into().unwrap()), 42);
        assert_eq!(u64::from_be_bytes(reply[24..32].try_into().unwrap()), 7);
        let transport =
            "RTP/AVP/UDP;unicast;mode=record;server_port=6000;control_port=6001;timing_port=6002";
        assert_eq!(transport_port(transport, "server_port"), Some(6000));
        assert_eq!(transport_port(transport, "timing_port"), Some(6002));
        let sdp = announce_sdp(7, "10.0.0.2".parse().unwrap(), "10.0.0.9".parse().unwrap());
        assert!(sdp.contains("a=fmtp:96 352 0 16 40 10 14 2 255 0 0 44100"));
        assert!(sdp.contains("c=IN IP4 10.0.0.9"));
    }
}
