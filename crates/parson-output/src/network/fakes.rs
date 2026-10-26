//! Protocol-level fakes of network endpoints, for tests and demos.
//!
//! No streamer or AirPlay speaker is attached where Parson is built and
//! tested, so these stand in for them at the wire level: a UPnP AV /
//! OpenHome renderer (SSDP responder, device description, SOAP control,
//! and a client that really pulls and parses the HTTP stream) and an
//! AirPlay 1 receiver (RTSP server, RTP audio with ALAC decoding, timing
//! requests and sync packets). They prove the protocol paths, not any
//! particular device, and everything that shows them says so.

use std::io::{self, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::http::read_head;
use super::xml;

/* ------------------------------------------------------------------ */
/* UPnP renderer                                                       */
/* ------------------------------------------------------------------ */

#[derive(Clone, Debug)]
pub struct FakeRendererConfig {
    pub name: String,
    pub manufacturer: String,
    pub model_name: String,
    pub model_number: String,
    pub udn: String,
    /// Has an AVTransport service (UPnP AV).
    pub av_transport: bool,
    /// Has OpenHome Product, Playlist and Info services.
    pub openhome: bool,
    /// ConnectionManager `Sink`; `None` leaves the service out.
    pub sink_protocol_info: Option<String>,
    /// A PNG served as the device icon.
    pub icon: Option<Vec<u8>>,
    /// Answers unicast `M-SEARCH` on a UDP port.
    pub ssdp: bool,
    /// How fast the renderer consumes audio, relative to real time; it
    /// first buffers one second as fast as the server sends.
    pub speed: f64,
    /// Makes `Info.Details` report this rate instead of the stream's, as
    /// a renderer that resamples internally would.
    pub reports_rate: Option<u32>,
}

impl Default for FakeRendererConfig {
    fn default() -> Self {
        Self {
            name: "Parson Test Renderer".into(),
            manufacturer: "Parson test bench".into(),
            model_name: "Fake OpenHome Renderer".into(),
            model_number: "1".into(),
            udn: "uuid:5061-7273-6f6e-fake-renderer01".into(),
            av_transport: true,
            openhome: true,
            sink_protocol_info: Some(
                "http-get:*:audio/wav:*,http-get:*:audio/L16;rate=44100;channels=2:*,http-get:*:audio/flac:*,http-get:*:audio/mpeg:*".into(),
            ),
            icon: None,
            ssdp: true,
            speed: 1.0,
            reports_rate: None,
        }
    }
}

/// What the fake renderer saw and played.
#[derive(Clone, Debug, Default)]
pub struct FakeRendererState {
    /// SOAP actions in order, e.g. `AVTransport#SetAVTransportURI`.
    pub actions: Vec<String>,
    pub uri: Option<String>,
    pub metadata: Option<String>,
    pub transport_state: String,
    pub content_type: Option<String>,
    /// The WAV header of the last stream, when it had one.
    pub wav_header: Option<Vec<u8>>,
    /// PCM bytes received (after any header), across fetches of one URI.
    pub received: Vec<u8>,
    /// `(sample_rate, bits, channels)` of the last stream.
    pub stream_format: Option<(u32, u16, u16)>,
    pub fetches: u32,
    pub ended: bool,
    pub playlist_id: u32,
}

struct RendererShared {
    config: FakeRendererConfig,
    state: Mutex<FakeRendererState>,
    stop: AtomicBool,
    fetch_generation: Mutex<u64>,
}

impl RendererShared {
    fn state(&self) -> std::sync::MutexGuard<'_, FakeRendererState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// A renderer on loopback. Dropping it stops it.
pub struct FakeRenderer {
    shared: Arc<RendererShared>,
    pub http_address: SocketAddr,
    pub ssdp_address: Option<SocketAddr>,
    threads: Vec<JoinHandle<()>>,
}

impl FakeRenderer {
    pub fn start(config: FakeRendererConfig) -> io::Result<Self> {
        Self::start_on(config, SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
    }

    pub fn start_on(config: FakeRendererConfig, bind: SocketAddr) -> io::Result<Self> {
        let listener = TcpListener::bind(bind)?;
        let http_address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let shared = Arc::new(RendererShared {
            config,
            state: Mutex::new(FakeRendererState {
                transport_state: "NO_MEDIA_PRESENT".into(),
                ..FakeRendererState::default()
            }),
            stop: AtomicBool::new(false),
            fetch_generation: Mutex::new(0),
        });
        let mut threads = Vec::new();
        let http_shared = Arc::clone(&shared);
        threads.push(std::thread::spawn(move || {
            while !http_shared.stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((connection, _)) => {
                        let shared = Arc::clone(&http_shared);
                        std::thread::spawn(move || {
                            let _ = handle_http(connection, &shared);
                        });
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        }));
        let mut ssdp_address = None;
        if shared.config.ssdp {
            let socket = UdpSocket::bind(SocketAddr::new(bind.ip(), 0))?;
            socket.set_read_timeout(Some(Duration::from_millis(50)))?;
            ssdp_address = Some(socket.local_addr()?);
            let ssdp_shared = Arc::clone(&shared);
            threads.push(std::thread::spawn(move || {
                let mut buffer = [0u8; 2048];
                while !ssdp_shared.stop.load(Ordering::Relaxed) {
                    let Ok((length, from)) = socket.recv_from(&mut buffer) else {
                        continue;
                    };
                    let message = String::from_utf8_lossy(&buffer[..length]);
                    if !message.starts_with("M-SEARCH") {
                        continue;
                    }
                    let st = message
                        .lines()
                        .find_map(|line| line.strip_prefix("ST:").or_else(|| line.strip_prefix("st:")))
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let matches = st == "ssdp:all"
                        || st.contains("MediaRenderer")
                        || (ssdp_shared.config.openhome && st.contains("av-openhome-org"));
                    if !matches {
                        continue;
                    }
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=1800\r\nEXT:\r\nLOCATION: http://{http_address}/description.xml\r\nSERVER: Linux/6 UPnP/1.0 ParsonFake/1\r\nST: {st}\r\nUSN: {}::{st}\r\n\r\n",
                        ssdp_shared.config.udn
                    );
                    let _ = socket.send_to(response.as_bytes(), from);
                }
            }));
        }
        Ok(Self {
            shared,
            http_address,
            ssdp_address,
            threads,
        })
    }

    pub fn description_url(&self) -> String {
        format!("http://{}/description.xml", self.http_address)
    }

    pub fn state(&self) -> FakeRendererState {
        self.shared.state().clone()
    }

    /// Waits until `predicate` holds, up to `timeout`.
    pub fn wait_for(
        &self,
        timeout: Duration,
        predicate: impl Fn(&FakeRendererState) -> bool,
    ) -> FakeRendererState {
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

impl Drop for FakeRenderer {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Ok(mut generation) = self.shared.fetch_generation.lock() {
            *generation += 1;
        }
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

fn description_xml(config: &FakeRendererConfig) -> String {
    let mut services = String::new();
    let service = |kind: &str, path: &str| {
        format!(
            "<service><serviceType>{kind}</serviceType><serviceId>urn:x:{path}</serviceId><SCPDURL>/{path}.xml</SCPDURL><controlURL>/control/{path}</controlURL><eventSubURL>/event/{path}</eventSubURL></service>"
        )
    };
    if config.sink_protocol_info.is_some() {
        services.push_str(&service(
            "urn:schemas-upnp-org:service:ConnectionManager:1",
            "cm",
        ));
    }
    if config.av_transport {
        services.push_str(&service(
            "urn:schemas-upnp-org:service:AVTransport:1",
            "avt",
        ));
        services.push_str(&service(
            "urn:schemas-upnp-org:service:RenderingControl:1",
            "rc",
        ));
    }
    if config.openhome {
        services.push_str(&service("urn:av-openhome-org:service:Product:1", "product"));
        services.push_str(&service(
            "urn:av-openhome-org:service:Playlist:1",
            "playlist",
        ));
        services.push_str(&service("urn:av-openhome-org:service:Info:1", "info"));
    }
    let icon = if config.icon.is_some() {
        "<iconList><icon><mimetype>image/png</mimetype><width>256</width><height>256</height><depth>24</depth><url>/icon.png</url></icon></iconList>"
    } else {
        ""
    };
    format!(
        "<?xml version=\"1.0\"?><root xmlns=\"urn:schemas-upnp-org:device-1-0\"><specVersion><major>1</major><minor>0</minor></specVersion><device><deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType><friendlyName>{}</friendlyName><manufacturer>{}</manufacturer><modelName>{}</modelName><modelNumber>{}</modelNumber><modelDescription>Protocol-level fake renderer</modelDescription><UDN>{}</UDN>{icon}<serviceList>{services}</serviceList></device></root>",
        xml::escape(&config.name),
        xml::escape(&config.manufacturer),
        xml::escape(&config.model_name),
        xml::escape(&config.model_number),
        xml::escape(&config.udn)
    )
}

fn respond(
    mut connection: &TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    connection.write_all(
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .as_bytes(),
    )?;
    connection.write_all(body)
}

fn soap_response(action: &str, service: &str, fields: &[(&str, String)]) -> String {
    let mut body = format!(
        "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><u:{action}Response xmlns:u=\"{service}\">"
    );
    for (name, value) in fields {
        body.push_str(&format!("<{name}>{}</{name}>", xml::escape(value)));
    }
    body.push_str(&format!("</u:{action}Response></s:Body></s:Envelope>"));
    body
}

fn soap_fault(code: u32, description: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><s:Fault><faultcode>s:Client</faultcode><faultstring>UPnPError</faultstring><detail><UPnPError xmlns=\"urn:schemas-upnp-org:control-1-0\"><errorCode>{code}</errorCode><errorDescription>{description}</errorDescription></UPnPError></detail></s:Fault></s:Body></s:Envelope>"
    )
}

fn handle_http(connection: TcpStream, shared: &Arc<RendererShared>) -> io::Result<()> {
    connection.set_read_timeout(Some(Duration::from_secs(5)))?;
    connection.set_nonblocking(false)?;
    let mut reader = BufReader::new(connection.try_clone()?);
    let (request_line, headers) = read_head(&mut reader)?;
    let header = |name: &str| {
        headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
    };
    let length: usize = header("Content-Length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length.min(1 << 20)];
    reader.read_exact(&mut body)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    match (method, path) {
        ("GET", "/description.xml") => respond(
            &connection,
            "200 OK",
            "text/xml",
            description_xml(&shared.config).as_bytes(),
        ),
        ("GET", "/icon.png") => match &shared.config.icon {
            Some(icon) => respond(&connection, "200 OK", "image/png", icon),
            None => respond(&connection, "404 Not Found", "text/plain", b""),
        },
        ("POST", control) if control.starts_with("/control/") => {
            let soap_action = header("SOAPACTION").unwrap_or_default();
            let soap_action = soap_action.trim_matches('"');
            let (service, action) = soap_action.split_once('#').unwrap_or(("", ""));
            let text = String::from_utf8_lossy(&body).into_owned();
            let (status, reply) = soap(shared, service, action, &text);
            respond(
                &connection,
                status,
                "text/xml; charset=\"utf-8\"",
                reply.as_bytes(),
            )
        }
        _ => respond(&connection, "404 Not Found", "text/plain", b""),
    }
}

fn soap(
    shared: &Arc<RendererShared>,
    service: &str,
    action: &str,
    body: &str,
) -> (&'static str, String) {
    let short = service.rsplit(':').nth(1).unwrap_or(service);
    shared.state().actions.push(format!("{short}#{action}"));
    let ok = |fields: &[(&str, String)]| ("200 OK", soap_response(action, service, fields));
    match action {
        "GetProtocolInfo" => ok(&[
            ("Source", String::new()),
            (
                "Sink",
                shared.config.sink_protocol_info.clone().unwrap_or_default(),
            ),
        ]),
        "GetTransportInfo" => {
            let state = shared.state().transport_state.clone();
            ok(&[
                ("CurrentTransportState", state),
                ("CurrentTransportStatus", "OK".into()),
                ("CurrentSpeed", "1".into()),
            ])
        }
        "TransportState" => {
            let state = shared.state().transport_state.clone();
            ok(&[(
                "Value",
                match state.as_str() {
                    "PLAYING" => "Playing".into(),
                    "STOPPED" => "Stopped".into(),
                    _ => "Stopped".into(),
                },
            )])
        }
        "SetAVTransportURI" => {
            let mut state = shared.state();
            state.uri = xml::text(body, "CurrentURI");
            state.metadata = xml::text(body, "CurrentURIMetaData");
            state.transport_state = "STOPPED".into();
            drop(state);
            ok(&[])
        }
        "DeleteAll" => ok(&[]),
        "Insert" => {
            let mut state = shared.state();
            state.uri = xml::text(body, "Uri");
            state.metadata = xml::text(body, "Metadata");
            state.playlist_id += 1;
            let id = state.playlist_id.to_string();
            drop(state);
            ok(&[("NewId", id)])
        }
        "SeekId" | "Play" => {
            let uri = {
                let mut state = shared.state();
                if state.transport_state == "PLAYING" && action == "Play" {
                    None
                } else {
                    state.transport_state = "PLAYING".into();
                    state.uri.clone()
                }
            };
            if let Some(uri) = uri {
                start_fetch(shared, uri);
            }
            ok(&[])
        }
        "Stop" | "Pause" => {
            if let Ok(mut generation) = shared.fetch_generation.lock() {
                *generation += 1;
            }
            shared.state().transport_state = if action == "Stop" {
                "STOPPED"
            } else {
                "PAUSED_PLAYBACK"
            }
            .into();
            ok(&[])
        }
        "Details" => {
            let state = shared.state();
            match state.stream_format {
                Some((rate, bits, _)) if state.transport_state == "PLAYING" => {
                    let rate = shared.config.reports_rate.unwrap_or(rate);
                    (
                        "200 OK",
                        soap_response(
                            action,
                            service,
                            &[
                                ("Duration", "0".into()),
                                ("BitRate", (rate as u64 * u64::from(bits) * 2).to_string()),
                                ("BitDepth", bits.to_string()),
                                ("SampleRate", rate.to_string()),
                                ("Lossless", "true".into()),
                                (
                                    "CodecName",
                                    if state.wav_header.is_some() {
                                        "WAV"
                                    } else {
                                        "PCM"
                                    }
                                    .into(),
                                ),
                            ],
                        ),
                    )
                }
                _ => ok(&[
                    ("Duration", "0".into()),
                    ("BitRate", "0".into()),
                    ("BitDepth", "0".into()),
                    ("SampleRate", "0".into()),
                    ("Lossless", "false".into()),
                    ("CodecName", String::new()),
                ]),
            }
        }
        _ => (
            "500 Internal Server Error",
            soap_fault(401, "Invalid Action"),
        ),
    }
}

/// Pulls the stream like a renderer: header, then PCM at the configured
/// speed after a one-second buffer.
fn start_fetch(shared: &Arc<RendererShared>, uri: String) {
    let generation = {
        let mut generation = shared
            .fetch_generation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *generation += 1;
        *generation
    };
    {
        let mut state = shared.state();
        state.received.clear();
        state.wav_header = None;
        state.stream_format = None;
        state.ended = false;
    }
    let shared = Arc::clone(shared);
    std::thread::spawn(move || {
        let current = || {
            *shared
                .fetch_generation
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                == generation
        };
        let Ok(url) = super::http::parse_url(&uri) else {
            return;
        };
        let Ok(address) = url.socket_addr() else {
            return;
        };
        let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_secs(2)) else {
            return;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
        if stream
            .write_all(
                format!(
                    "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: ParsonFakeRenderer/1\r\n\r\n",
                    url.path,
                    url.authority()
                )
                .as_bytes(),
            )
            .is_err()
        {
            return;
        }
        let mut reader = BufReader::new(stream);
        let Ok((_, headers)) = read_head(&mut reader) else {
            return;
        };
        let content_type = headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("Content-Type"))
            .map(|(_, value)| value.clone())
            .unwrap_or_default();
        let mut format = None;
        let mut wav_header = None;
        if content_type.starts_with("audio/wav") {
            let mut header = [0u8; 44];
            if reader.read_exact(&mut header).is_err() || &header[0..4] != b"RIFF" {
                return;
            }
            let channels = u16::from_le_bytes([header[22], header[23]]);
            let rate = u32::from_le_bytes(header[24..28].try_into().unwrap_or_default());
            let bits = u16::from_le_bytes([header[34], header[35]]);
            format = Some((rate, bits, channels));
            wav_header = Some(header.to_vec());
        } else if let Some(params) = content_type.strip_prefix("audio/L") {
            let bits: u16 = params
                .split(';')
                .next()
                .and_then(|bits| bits.parse().ok())
                .unwrap_or(16);
            let value = |name: &str| {
                params
                    .split(';')
                    .find_map(|part| part.trim().strip_prefix(name))
                    .and_then(|value| value.parse::<u32>().ok())
            };
            format = Some((
                value("rate=").unwrap_or(44_100),
                bits,
                value("channels=").unwrap_or(2) as u16,
            ));
        }
        {
            let mut state = shared.state();
            state.fetches += 1;
            state.content_type = Some(content_type);
            state.stream_format = format;
            state.wav_header = wav_header;
        }
        let byte_rate = format.map_or(176_400.0, |(rate, bits, channels)| {
            f64::from(rate) * f64::from(bits / 8) * f64::from(channels)
        });
        let speed = shared.config.speed.max(0.01);
        let started = Instant::now();
        let mut total = 0usize;
        let mut buffer = vec![0u8; 16 * 1024];
        while current() && !shared.stop.load(Ordering::Relaxed) {
            // One second of buffer, then real time (scaled by speed).
            let allowed = byte_rate * (1.0 + started.elapsed().as_secs_f64() * speed);
            if total as f64 >= allowed {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            match reader.read(&mut buffer) {
                Ok(0) => {
                    let mut state = shared.state();
                    state.ended = true;
                    state.transport_state = "STOPPED".into();
                    return;
                }
                Ok(read) => {
                    total += read;
                    shared.state().received.extend_from_slice(&buffer[..read]);
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => return,
            }
        }
    });
}

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
                let receivers = spawn_udp(shared, audio, control, timing, client_timing.map(|port| SocketAddr::new(peer_ip, port)), client_control)?;
                if let Ok(mut list) = threads.lock() {
                    list.extend(receivers);
                }
            }
            "RECORD" => {
                if let Some(latency) = shared.config.audio_latency {
                    extra.push_str(&format!("Audio-Latency: {latency}\r\n"));
                }
            }
            "SET_PARAMETER" => shared.state().parameters.push(String::from_utf8_lossy(&body).into_owned()),
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

fn spawn_udp(
    shared: &Arc<AirPlayShared>,
    audio: UdpSocket,
    control: UdpSocket,
    timing: UdpSocket,
    client_timing: Option<SocketAddr>,
    _client_control: Option<u16>,
) -> io::Result<Vec<JoinHandle<()>>> {
    for socket in [&audio, &control, &timing] {
        socket.set_read_timeout(Some(Duration::from_millis(50)))?;
    }
    let mut handles = Vec::new();
    let audio_shared = Arc::clone(shared);
    handles.push(std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        let mut last_seq: Option<u16> = None;
        while !audio_shared.stop.load(Ordering::Relaxed) {
            let Ok(length) = audio.recv(&mut buffer) else {
                continue;
            };
            if length < 12 || buffer[0] != 0x80 || buffer[1] & 0x7F != 0x60 {
                continue;
            }
            let marker = buffer[1] & 0x80 != 0;
            let seq = u16::from_be_bytes([buffer[2], buffer[3]]);
            let mut state = audio_shared.state();
            state.packets += 1;
            if marker {
                state.marker_packets += 1;
            }
            state.first_seq.get_or_insert(seq);
            if let Some(last) = last_seq
                && seq != last.wrapping_add(1)
                && !marker
            {
                state.out_of_order += 1;
            }
            last_seq = Some(seq);
            match decode_alac_escape(&buffer[12..length]) {
                Ok(samples) => state.samples.extend_from_slice(&samples),
                Err(error) => {
                    if state.decode_errors.len() < 8 {
                        state.decode_errors.push(error);
                    }
                }
            }
        }
    }));
    let control_shared = Arc::clone(shared);
    handles.push(std::thread::spawn(move || {
        let mut buffer = [0u8; 256];
        while !control_shared.stop.load(Ordering::Relaxed) {
            let Ok(length) = control.recv(&mut buffer) else {
                continue;
            };
            if length >= 20 && buffer[1] == 0xD4 {
                let mut state = control_shared.state();
                state.sync_packets += 1;
                if buffer[0] == 0x90 {
                    state.first_sync_seen = true;
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

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// Encodes RGB pixels as a PNG (stored deflate blocks, no compression).
pub fn encode_png(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(rgb.len() + height as usize);
    for row in rgb.chunks_exact(width as usize * 3) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut zlib = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (index, block) in blocks.iter().enumerate() {
        zlib.push(u8::from(index + 1 == blocks.len()));
        let length = block.len() as u16;
        zlib.extend_from_slice(&length.to_le_bytes());
        zlib.extend_from_slice(&(!length).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for byte in &raw {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    zlib.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    png_chunk(&mut png, b"IHDR", &header);
    png_chunk(&mut png, b"IDAT", &zlib);
    png_chunk(&mut png, b"IEND", &[]);
    png
}

/// A picture of a hi-fi streamer (a brushed front panel with a display
/// and a volume knob), served as the fake renderer's UPnP icon.
pub fn sample_icon() -> Vec<u8> {
    let (width, height) = (160u32, 160u32);
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            let (fx, fy) = (f64::from(x), f64::from(y));
            let in_box = (12.0..148.0).contains(&fx) && (52.0..108.0).contains(&fy);
            let display = (28.0..92.0).contains(&fx) && (68.0..84.0).contains(&fy);
            let knob = ((fx - 122.0).powi(2) + (fy - 80.0).powi(2)).sqrt();
            let pixel: [u8; 3] = if !in_box {
                [0, 0, 0]
            } else if knob < 13.0 {
                let shade = (150.0 + (fx - 122.0) * 3.0) as u8;
                [shade, shade, shade.saturating_add(6)]
            } else if display {
                let lit = (34.0..70.0).contains(&fx) && (73.0..79.0).contains(&fy);
                if lit { [190, 150, 255] } else { [18, 16, 26] }
            } else {
                let brushed =
                    70 + ((fx * 7.0 + fy).sin() * 4.0) as i32 + ((fy - 52.0) * 0.6) as i32;
                let value = brushed.clamp(0, 255) as u8;
                [value, value, value.saturating_add(4)]
            };
            rgb.extend_from_slice(&pixel);
        }
    }
    encode_png(width, height, &rgb)
}
