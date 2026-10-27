//! A protocol-level fake Chromecast for tests: TLS on its own port with a
//! self-signed certificate (like real devices), Cast v2 framing, the
//! connection/heartbeat/receiver/media namespaces, the Default Media
//! Receiver app with a transport id, and media playback that fetches the
//! content URL like hardware and ends a track after its duration.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::proto::{
    CastMessage, DEFAULT_MEDIA_RECEIVER, NS_CONNECTION, NS_HEARTBEAT, NS_MEDIA, NS_RECEIVER,
    take_frames,
};

const CERTIFICATE: &[u8] = include_bytes!("fake-device.crt");
const KEY: &[u8] = include_bytes!("fake-device.key");

/// What the device fetched for a LOAD.
#[derive(Clone, Debug, Default)]
pub struct Fetch {
    pub url: String,
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct FakeCastState {
    pub launched: bool,
    pub launches: u32,
    pub connects: Vec<String>,
    pub pings: u32,
    pub level: f64,
    pub muted: bool,
    pub loads: Vec<Value>,
    pub fetches: Vec<Fetch>,
    pub commands: Vec<String>,
    player_state: String,
    idle_reason: Option<String>,
    position: f64,
    since: Option<Instant>,
    duration: f64,
    media_session: i64,
    media: Value,
}

impl FakeCastState {
    fn current_time(&mut self) -> f64 {
        let running = match (self.player_state.as_str(), self.since) {
            ("PLAYING", Some(since)) => since.elapsed().as_secs_f64(),
            _ => 0.0,
        };
        let time = self.position + running;
        if self.player_state == "PLAYING" && self.duration > 0.0 && time >= self.duration {
            self.player_state = "IDLE".into();
            self.idle_reason = Some("FINISHED".into());
            self.position = 0.0;
            self.since = None;
            return 0.0;
        }
        time
    }

    fn freeze(&mut self) {
        self.position = self.current_time();
        self.since = None;
    }

    fn receiver_status(&self) -> Value {
        let mut applications = Vec::new();
        if self.launched {
            applications.push(json!({
                "appId": DEFAULT_MEDIA_RECEIVER,
                "displayName": "Default Media Receiver",
                "sessionId": "session-1",
                "transportId": "web-5",
                "namespaces": [{"name": NS_MEDIA}],
            }));
        }
        json!({
            "applications": applications,
            "volume": {"level": self.level, "muted": self.muted, "controlType": "attenuation"},
        })
    }

    fn media_status(&mut self) -> Value {
        if self.media_session == 0 {
            return json!([]);
        }
        let time = self.current_time();
        let mut status = json!({
            "mediaSessionId": self.media_session,
            "playerState": self.player_state,
            "currentTime": time,
            "media": self.media,
            "volume": {"level": self.level, "muted": self.muted},
        });
        if let Some(reason) = &self.idle_reason {
            status["idleReason"] = json!(reason);
        }
        json!([status])
    }
}

pub struct FakeCast {
    pub address: SocketAddr,
    pub state: Arc<Mutex<FakeCastState>>,
}

fn lock(state: &Mutex<FakeCastState>) -> std::sync::MutexGuard<'_, FakeCastState> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn server_config() -> Arc<rustls::ServerConfig> {
    use rustls::pki_types::pem::PemObject;
    let certificates: Vec<rustls::pki_types::CertificateDer<'static>> =
        rustls::pki_types::CertificateDer::pem_slice_iter(CERTIFICATE)
            .collect::<Result<_, _>>()
            .expect("test certificate");
    let key = rustls::pki_types::PrivateKeyDer::from_pem_slice(KEY).expect("test key");
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    Arc::new(
        rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("versions")
            .with_no_client_auth()
            .with_single_cert(certificates, key)
            .expect("server config"),
    )
}

/// Fetches an `http://` URL like a device: status, content type, body.
fn fetch(url: &str) -> Fetch {
    let mut result = Fetch {
        url: url.to_string(),
        ..Fetch::default()
    };
    let Some(rest) = url.strip_prefix("http://") else {
        return result;
    };
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let Ok(mut stream) = TcpStream::connect(authority) else {
        return result;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
    let request = format!(
        "GET /{path} HTTP/1.1\r\nHost: {authority}\r\nUser-Agent: FakeCast\r\nConnection: close\r\n\r\n"
    );
    if stream.write_all(request.as_bytes()).is_err() {
        return result;
    }
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    let Some(split) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
        return result;
    };
    let head = String::from_utf8_lossy(&response[..split]).to_string();
    let mut lines = head.lines();
    result.status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let mut chunked = false;
    for line in lines {
        if let Some((key, value)) = line.split_once(':') {
            if key.eq_ignore_ascii_case("content-type") {
                result.content_type = value.trim().to_string();
            }
            if key.eq_ignore_ascii_case("transfer-encoding") && value.contains("chunked") {
                chunked = true;
            }
        }
    }
    let body = &response[split + 4..];
    result.body = if chunked {
        dechunk(body)
    } else {
        body.to_vec()
    };
    result
}

fn dechunk(mut body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let Some(end) = body.windows(2).position(|window| window == b"\r\n") else {
            return out;
        };
        let size = usize::from_str_radix(
            String::from_utf8_lossy(&body[..end])
                .split(';')
                .next()
                .unwrap_or("0")
                .trim(),
            16,
        )
        .unwrap_or(0);
        body = &body[end + 2..];
        if size == 0 || body.len() < size {
            return out;
        }
        out.extend_from_slice(&body[..size]);
        body = &body[(size + 2).min(body.len())..];
    }
}

impl FakeCast {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listen");
        let address = listener.local_addr().expect("address");
        let state = Arc::new(Mutex::new(FakeCastState {
            level: 0.4,
            player_state: "IDLE".into(),
            ..FakeCastState::default()
        }));
        let config = server_config();
        let thread_state = state.clone();
        std::thread::spawn(move || {
            for connection in listener.incoming() {
                let Ok(connection) = connection else {
                    continue;
                };
                let state = thread_state.clone();
                let config = config.clone();
                std::thread::spawn(move || serve(connection, config, state));
            }
        });
        Self { address, state }
    }

    pub fn state(&self) -> FakeCastState {
        lock(&self.state).clone()
    }
}

fn serve(socket: TcpStream, config: Arc<rustls::ServerConfig>, state: Arc<Mutex<FakeCastState>>) {
    let Ok(connection) = rustls::ServerConnection::new(config) else {
        return;
    };
    let mut stream = rustls::StreamOwned::new(connection, socket);
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 16 * 1024];
    loop {
        let length = match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(length) => length,
        };
        buffer.extend_from_slice(&chunk[..length]);
        let Ok(messages) = take_frames(&mut buffer) else {
            return;
        };
        for message in messages {
            for reply in handle(&message, &state) {
                if stream.write_all(&reply.encode()).is_err() {
                    return;
                }
            }
            let _ = stream.flush();
        }
    }
}

fn handle(message: &CastMessage, state: &Arc<Mutex<FakeCastState>>) -> Vec<CastMessage> {
    let Ok(payload) = serde_json::from_str::<Value>(&message.payload) else {
        return Vec::new();
    };
    let kind = payload["type"].as_str().unwrap_or("").to_string();
    let request_id = payload["requestId"].clone();
    let reply = |namespace: &str, mut body: Value| {
        body["requestId"] = request_id.clone();
        CastMessage::new(
            &message.destination,
            &message.source,
            namespace,
            body.to_string(),
        )
    };
    let mut guard = lock(state);
    match (message.namespace.as_str(), kind.as_str()) {
        (NS_CONNECTION, "CONNECT") => {
            guard.connects.push(message.destination.clone());
            Vec::new()
        }
        (NS_HEARTBEAT, "PING") => {
            guard.pings += 1;
            vec![CastMessage::new(
                &message.destination,
                &message.source,
                NS_HEARTBEAT,
                json!({"type": "PONG"}).to_string(),
            )]
        }
        (NS_RECEIVER, "GET_STATUS") => vec![reply(
            NS_RECEIVER,
            json!({"type": "RECEIVER_STATUS", "status": guard.receiver_status()}),
        )],
        (NS_RECEIVER, "LAUNCH") => {
            if payload["appId"] != DEFAULT_MEDIA_RECEIVER {
                return vec![reply(NS_RECEIVER, json!({"type": "LAUNCH_ERROR"}))];
            }
            guard.launched = true;
            guard.launches += 1;
            vec![reply(
                NS_RECEIVER,
                json!({"type": "RECEIVER_STATUS", "status": guard.receiver_status()}),
            )]
        }
        (NS_RECEIVER, "SET_VOLUME") => {
            if let Some(level) = payload["volume"]["level"].as_f64() {
                guard.level = level.clamp(0.0, 1.0);
            }
            if let Some(muted) = payload["volume"]["muted"].as_bool() {
                guard.muted = muted;
            }
            vec![reply(
                NS_RECEIVER,
                json!({"type": "RECEIVER_STATUS", "status": guard.receiver_status()}),
            )]
        }
        (NS_MEDIA, _) if message.destination != "web-5" || !guard.launched => {
            vec![reply(
                NS_MEDIA,
                json!({"type": "INVALID_REQUEST", "reason": "INVALID_SESSION"}),
            )]
        }
        (NS_MEDIA, "LOAD") => {
            guard.loads.push(payload.clone());
            guard.media_session += 1;
            guard.media = payload["media"].clone();
            guard.duration = payload["media"]["duration"].as_f64().unwrap_or(0.0);
            guard.position = payload["currentTime"].as_f64().unwrap_or(0.0);
            guard.idle_reason = None;
            let autoplay = payload["autoplay"].as_bool().unwrap_or(true);
            guard.player_state = if autoplay { "PLAYING" } else { "PAUSED" }.into();
            guard.since = autoplay.then(Instant::now);
            // Like hardware, fetch what it was told to play.
            if let Some(url) = payload["media"]["contentId"].as_str() {
                let url = url.to_string();
                let state = state.clone();
                std::thread::spawn(move || {
                    let fetched = fetch(&url);
                    lock(&state).fetches.push(fetched);
                });
            }
            let status = guard.media_status();
            vec![reply(
                NS_MEDIA,
                json!({"type": "MEDIA_STATUS", "status": status}),
            )]
        }
        (NS_MEDIA, command) => {
            guard.commands.push(command.to_string());
            match command {
                "PLAY" if guard.player_state == "PAUSED" => {
                    guard.player_state = "PLAYING".into();
                    guard.since = Some(Instant::now());
                }
                "PAUSE" if guard.player_state == "PLAYING" => {
                    guard.freeze();
                    guard.player_state = "PAUSED".into();
                }
                "SEEK" => {
                    let playing = guard.player_state == "PLAYING";
                    guard.position = payload["currentTime"].as_f64().unwrap_or(0.0);
                    guard.since = playing.then(Instant::now);
                }
                "STOP" => {
                    guard.player_state = "IDLE".into();
                    guard.idle_reason = Some("CANCELLED".into());
                    guard.since = None;
                }
                _ => {}
            }
            let status = guard.media_status();
            vec![reply(
                NS_MEDIA,
                json!({"type": "MEDIA_STATUS", "status": status}),
            )]
        }
        _ => Vec::new(),
    }
}
