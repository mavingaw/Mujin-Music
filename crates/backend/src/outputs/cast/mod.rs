//! Chromecast devices driven by the server (`chromecast:<id>`), so every
//! Parson app (web, desktop, iPhone, Android) can play to them, transfer to
//! them and use their volume, not only the Android app's Cast button.
//!
//! The server speaks Cast v2 itself: TLS to port 8009, the protobuf-framed
//! JSON namespaces (connection, heartbeat, receiver, media), and launches
//! the Default Media Receiver with signed media URLs the device fetches.
//! Google Home speaker groups are Cast devices of their own, so they show
//! up as zones too. Format limits are Chromecast's: FLAC up to 96 kHz/
//! 24-bit untouched, anything above converted (the signal path says how).

#[cfg(test)]
pub mod fake;
pub mod proto;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use serde::Serialize;
use serde_json::{Value, json};

use super::FormatLimits;
use super::renderer::{
    PlayState, Renderer, RendererInfo, RendererSettings, RendererState, TrackRequest,
};
use crate::zones::{DeviceVolumeRange, VolumeMode, ZoneKind};
use proto::{
    CastMessage, DEFAULT_MEDIA_RECEIVER, NS_CONNECTION, NS_HEARTBEAT, NS_MEDIA, NS_RECEIVER,
    take_frames,
};

pub const CAST_PREFIX: &str = "chromecast:";
pub const SERVICE_TYPE: &str = "_googlecast._tcp.local.";
const SENDER: &str = "sender-parson";
const RECEIVER: &str = "receiver-0";
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);
pub const CAST_NOTE: &str = "Played by the Parson server over Google Cast, so any Parson app can use it. Tested against a protocol-level fake Cast device, not yet on Google hardware. Tracks follow each other with a short gap (the Default Media Receiver is not gapless).";

/// Chromecast devices present self-signed certificates (their identity is
/// checked with the separate device-auth exchange, which media senders
/// skip), so the TLS layer accepts any certificate and only encrypts.
#[derive(Debug)]
struct CastDeviceCertificates(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for CastDeviceCertificates {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn tls_config() -> Arc<rustls::ClientConfig> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("TLS versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(CastDeviceCertificates(provider)))
        .with_no_client_auth();
    Arc::new(config)
}

type Tls = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;

#[derive(Default)]
struct Inbox {
    replies: HashMap<u64, Value>,
    receiver: Option<Value>,
    media: Option<Value>,
}

/// One TLS connection to a Cast device.
struct Connection {
    stream: Mutex<Tls>,
    inbox: Mutex<Inbox>,
    wake: Condvar,
    alive: AtomicBool,
    next_id: AtomicU64,
    /// Virtual connections opened (receiver-0 and the app's transport).
    connected: Mutex<Vec<String>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Connection {
    fn open(address: SocketAddr) -> Result<Arc<Self>, String> {
        let socket = TcpStream::connect_timeout(&address, Duration::from_secs(5))
            .map_err(|error| format!("could not reach the Chromecast: {error}"))?;
        let _ = socket.set_nodelay(true);
        socket
            .set_read_timeout(Some(Duration::from_millis(20)))
            .map_err(|error| error.to_string())?;
        let name = rustls::pki_types::ServerName::IpAddress(address.ip().into());
        let client =
            rustls::ClientConnection::new(tls_config(), name).map_err(|error| error.to_string())?;
        let mut stream = rustls::StreamOwned::new(client, socket);
        // Finish the handshake before handing the stream to two threads.
        let deadline = Instant::now() + Duration::from_secs(5);
        while stream.conn.is_handshaking() {
            match stream.conn.complete_io(&mut stream.sock) {
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) && Instant::now() < deadline => {}
                Err(error) => return Err(format!("Chromecast TLS: {error}")),
            }
        }
        let connection = Arc::new(Self {
            stream: Mutex::new(stream),
            inbox: Mutex::new(Inbox::default()),
            wake: Condvar::new(),
            alive: AtomicBool::new(true),
            next_id: AtomicU64::new(1),
            connected: Mutex::new(Vec::new()),
        });
        connection.virtual_connect(RECEIVER)?;
        let reader = Arc::downgrade(&connection);
        std::thread::Builder::new()
            .name("parson-cast-read".into())
            .spawn(move || read_loop(reader))
            .map_err(|error| error.to_string())?;
        let heartbeat = Arc::downgrade(&connection);
        std::thread::Builder::new()
            .name("parson-cast-ping".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_secs(5));
                    let Some(connection) = heartbeat.upgrade() else {
                        return;
                    };
                    if connection
                        .send(RECEIVER, NS_HEARTBEAT, json!({"type": "PING"}))
                        .is_err()
                    {
                        return;
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(connection)
    }

    fn send(&self, destination: &str, namespace: &str, payload: Value) -> Result<(), String> {
        if !self.alive.load(Ordering::Relaxed) {
            return Err("the Chromecast connection closed".into());
        }
        let frame = CastMessage::new(SENDER, destination, namespace, payload.to_string()).encode();
        let mut stream = lock(&self.stream);
        stream
            .write_all(&frame)
            .and_then(|()| stream.flush())
            .map_err(|error| {
                self.alive.store(false, Ordering::Relaxed);
                format!("Chromecast: {error}")
            })
    }

    fn virtual_connect(&self, destination: &str) -> Result<(), String> {
        if lock(&self.connected).iter().any(|id| id == destination) {
            return Ok(());
        }
        self.send(
            destination,
            NS_CONNECTION,
            json!({"type": "CONNECT", "userAgent": "Parson"}),
        )?;
        lock(&self.connected).push(destination.to_string());
        Ok(())
    }

    /// Sends a request and waits for the reply with its `requestId`.
    fn request(
        &self,
        destination: &str,
        namespace: &str,
        mut payload: Value,
    ) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        payload["requestId"] = json!(id);
        self.send(destination, namespace, payload)?;
        let deadline = Instant::now() + REPLY_TIMEOUT;
        let mut inbox = lock(&self.inbox);
        loop {
            if let Some(reply) = inbox.replies.remove(&id) {
                return match reply["type"].as_str() {
                    Some("LOAD_FAILED" | "LOAD_CANCELLED" | "INVALID_REQUEST" | "LAUNCH_ERROR") => {
                        Err(format!(
                            "Chromecast: {}",
                            reply["type"].as_str().unwrap_or("error")
                        ))
                    }
                    _ => Ok(reply),
                };
            }
            if !self.alive.load(Ordering::Relaxed) {
                return Err("the Chromecast connection closed".into());
            }
            let now = Instant::now();
            if now >= deadline {
                return Err("the Chromecast did not answer".into());
            }
            inbox = self
                .wake
                .wait_timeout(inbox, deadline - now)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }

    fn receive(&self, message: CastMessage) {
        let Ok(payload) = serde_json::from_str::<Value>(&message.payload) else {
            return;
        };
        if message.namespace == NS_HEARTBEAT {
            if payload["type"] == "PING" {
                let _ = self.send(&message.source, NS_HEARTBEAT, json!({"type": "PONG"}));
            }
            return;
        }
        if message.namespace == NS_CONNECTION && payload["type"] == "CLOSE" {
            lock(&self.connected).retain(|id| *id != message.source);
            return;
        }
        let mut inbox = lock(&self.inbox);
        match payload["type"].as_str() {
            Some("RECEIVER_STATUS") => inbox.receiver = Some(payload["status"].clone()),
            Some("MEDIA_STATUS") => {
                inbox.media = payload["status"].get(0).cloned().or(inbox.media.take())
            }
            _ => {}
        }
        if let Some(id) = payload["requestId"].as_u64().filter(|id| *id > 0) {
            inbox.replies.insert(id, payload);
        }
        drop(inbox);
        self.wake.notify_all();
    }
}

fn read_loop(connection: std::sync::Weak<Connection>) {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 16 * 1024];
    loop {
        let Some(connection) = connection.upgrade() else {
            return;
        };
        if !connection.alive.load(Ordering::Relaxed) {
            connection.wake.notify_all();
            return;
        }
        let read = lock(&connection.stream).read(&mut chunk);
        match read {
            Ok(0) => {
                connection.alive.store(false, Ordering::Relaxed);
                connection.wake.notify_all();
                return;
            }
            Ok(length) => {
                buffer.extend_from_slice(&chunk[..length]);
                match take_frames(&mut buffer) {
                    Ok(messages) => {
                        for message in messages {
                            connection.receive(message);
                        }
                    }
                    Err(_) => {
                        connection.alive.store(false, Ordering::Relaxed);
                        connection.wake.notify_all();
                        return;
                    }
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                drop(connection);
                // Let writers in between reads.
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => {
                connection.alive.store(false, Ordering::Relaxed);
                connection.wake.notify_all();
                return;
            }
        }
    }
}

/// A Chromecast device as found on the network.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CastDevice {
    pub id: String,
    pub name: String,
    pub model: String,
    pub address: String,
    /// A Google Home speaker group.
    pub group: bool,
}

#[derive(Default)]
struct Playing {
    serial: Option<u64>,
    song_id: Option<String>,
    duration_ms: i64,
    finished: Option<u64>,
    /// The device's session of our app, and its media session.
    transport: Option<String>,
    session: Option<String>,
    media_session: Option<i64>,
}

/// One Chromecast zone.
pub struct CastRenderer {
    zone_id: String,
    device: Mutex<CastDevice>,
    connection: Mutex<Option<Arc<Connection>>>,
    playing: Mutex<Playing>,
    state: Mutex<RendererState>,
}

impl CastRenderer {
    fn new(device: CastDevice) -> Self {
        Self {
            zone_id: format!("{CAST_PREFIX}{}", device.id),
            device: Mutex::new(device),
            connection: Mutex::new(None),
            playing: Mutex::new(Playing::default()),
            state: Mutex::new(RendererState::default()),
        }
    }

    fn address(&self) -> Option<SocketAddr> {
        lock(&self.device).address.to_socket_addrs().ok()?.next()
    }

    /// The live connection, opening one when needed (blocking).
    fn connection(&self) -> Result<Arc<Connection>, String> {
        let mut current = lock(&self.connection);
        if let Some(connection) = current.as_ref()
            && connection.alive.load(Ordering::Relaxed)
        {
            return Ok(connection.clone());
        }
        let address = self.address().ok_or("the Chromecast has no address")?;
        let connection = Connection::open(address)?;
        *current = Some(connection.clone());
        // A new connection starts with no app session of ours.
        let mut playing = lock(&self.playing);
        playing.transport = None;
        playing.media_session = None;
        Ok(connection)
    }

    /// Launches the Default Media Receiver (or joins it) and connects to it.
    fn app(&self, connection: &Connection) -> Result<String, String> {
        let known = lock(&self.playing).transport.clone();
        if let Some(transport) = known
            && lock(&connection.connected).contains(&transport)
        {
            return Ok(transport);
        }
        let status = connection.request(RECEIVER, NS_RECEIVER, json!({"type": "GET_STATUS"}))?;
        let running = |status: &Value| {
            status["status"]["applications"]
                .as_array()
                .and_then(|apps| {
                    apps.iter()
                        .find(|app| app["appId"] == DEFAULT_MEDIA_RECEIVER)
                        .cloned()
                })
        };
        let app = match running(&status) {
            Some(app) => app,
            None => {
                let launched = connection.request(
                    RECEIVER,
                    NS_RECEIVER,
                    json!({"type": "LAUNCH", "appId": DEFAULT_MEDIA_RECEIVER}),
                )?;
                running(&launched).ok_or("the Chromecast did not start the media receiver")?
            }
        };
        let transport = app["transportId"]
            .as_str()
            .ok_or("no transport id")?
            .to_string();
        connection.virtual_connect(&transport)?;
        let mut playing = lock(&self.playing);
        playing.transport = Some(transport.clone());
        playing.session = app["sessionId"].as_str().map(str::to_string);
        Ok(transport)
    }

    fn media(&self, command: &str, extra: Value) -> Result<(), String> {
        let connection = self.connection()?;
        let transport = self.app(&connection)?;
        let session = lock(&self.playing)
            .media_session
            .ok_or("nothing is loaded on the Chromecast")?;
        let mut payload = json!({"type": command, "mediaSessionId": session});
        if let (Some(target), Some(fields)) = (payload.as_object_mut(), extra.as_object()) {
            for (key, value) in fields {
                target.insert(key.clone(), value.clone());
            }
        }
        let reply = connection.request(&transport, NS_MEDIA, payload)?;
        self.apply_media(reply["status"].get(0));
        Ok(())
    }

    fn apply_receiver(&self, status: Option<&Value>) {
        let Some(status) = status else {
            return;
        };
        let mut state = lock(&self.state);
        if let Some(level) = status["volume"]["level"].as_f64() {
            state.volume = (level * 100.0).round().clamp(0.0, 100.0) as u8;
            state.device_volume = Some(level as f32);
        }
        if let Some(muted) = status["volume"]["muted"].as_bool() {
            state.muted = muted;
        }
        // Another app took the device over.
        let foreign = status["applications"].as_array().and_then(|apps| {
            apps.iter()
                .find(|app| app["appId"] != DEFAULT_MEDIA_RECEIVER && app["isIdleScreen"] != true)
                .and_then(|app| app["displayName"].as_str())
                .map(|name| format!("{name} is casting here"))
        });
        state.foreign = foreign;
    }

    fn apply_media(&self, status: Option<&Value>) {
        let Some(status) = status else {
            return;
        };
        let mut playing = lock(&self.playing);
        if let Some(session) = status["mediaSessionId"].as_i64() {
            playing.media_session = Some(session);
        }
        let mut state = lock(&self.state);
        state.polled_at_ms = crate::zones::service::now_ms();
        state.reachable = true;
        if let Some(seconds) = status["currentTime"].as_f64() {
            state.position_ms = (seconds * 1_000.0).round() as i64;
        }
        if let Some(duration) = status["media"]["duration"].as_f64() {
            state.duration_ms = (duration * 1_000.0).round() as i64;
        } else {
            state.duration_ms = playing.duration_ms;
        }
        state.serial = playing.serial;
        state.song_id = playing.song_id.clone();
        state.state = match status["playerState"].as_str() {
            Some("PLAYING") => PlayState::Playing,
            Some("PAUSED") => PlayState::Paused,
            Some("BUFFERING") => PlayState::Transitioning,
            _ => PlayState::Stopped,
        };
        if status["playerState"] == "IDLE" && status["idleReason"] == "FINISHED" {
            playing.finished = playing.serial;
            state.finished_serial = playing.serial;
            state.position_ms = 0;
        }
    }

    fn blocking<T: Send + 'static>(
        self: &Arc<Self>,
        work: impl FnOnce(&Self) -> Result<T, String> + Send + 'static,
    ) -> BoxFuture<'static, Result<T, String>> {
        let renderer = self.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || work(&renderer))
                .await
                .map_err(|error| error.to_string())?
        })
    }
}

/// [`Renderer`] needs `&self` futures; the work runs on a blocking thread
/// with a clone of the `Arc`.
pub struct CastZone(Arc<CastRenderer>);

impl Renderer for CastZone {
    fn info(&self) -> RendererInfo {
        let device = lock(&self.0.device).clone();
        RendererInfo {
            id: self.0.zone_id.clone(),
            kind: ZoneKind::Chromecast,
            name: device.name.clone(),
            device: if device.group {
                "Google Home speaker group".to_string()
            } else {
                device.model.clone()
            },
            limits: FormatLimits::CHROMECAST,
            volume_range: Some(DeviceVolumeRange {
                min: 0.0,
                max: 1.0,
                step: 0.01,
                unit: "level",
                hardware: true,
                current: lock(&self.0.state).device_volume,
            }),
            volume_modes: vec![VolumeMode::Hardware, VolumeMode::Fixed],
            gapless: false,
            exclusive_capable: false,
            network: true,
            notes: vec![CAST_NOTE.to_string()],
        }
    }

    fn state(&self) -> RendererState {
        lock(&self.0.state).clone()
    }

    fn configure(&self, _settings: RendererSettings) {}

    fn media_origin(&self, port: u16) -> Option<String> {
        let local = crate::sonos::driver::local_address_towards(self.0.address()?)?;
        Some(match local {
            IpAddr::V4(address) => format!("http://{address}:{port}"),
            IpAddr::V6(address) => format!("http://[{address}]:{port}"),
        })
    }

    fn load(
        &self,
        track: TrackRequest,
        _next: Option<TrackRequest>,
        position_ms: i64,
        play: bool,
    ) -> BoxFuture<'_, Result<(), String>> {
        self.0.blocking(move |renderer| {
            let connection = renderer.connection()?;
            let transport = renderer.app(&connection)?;
            {
                let mut playing = lock(&renderer.playing);
                playing.serial = Some(track.serial);
                playing.song_id = Some(track.song_id.clone());
                playing.duration_ms = track.duration_ms;
                playing.finished = None;
            }
            {
                let mut state = lock(&renderer.state);
                state.finished_serial = None;
                state.serial = Some(track.serial);
                state.song_id = Some(track.song_id.clone());
                state.state = PlayState::Transitioning;
            }
            let mut metadata = json!({
                "metadataType": 3,
                "title": track.title,
                "artist": track.artist,
                "albumName": track.album,
            });
            if let Some(art) = &track.art_uri {
                metadata["images"] = json!([{"url": art}]);
            }
            let session = lock(&renderer.playing).session.clone();
            let reply = connection.request(
                &transport,
                NS_MEDIA,
                json!({
                    "type": "LOAD",
                    "sessionId": session,
                    "autoplay": play,
                    "currentTime": position_ms as f64 / 1_000.0,
                    "media": {
                        "contentId": track.uri,
                        "contentType": track.mime_type,
                        "streamType": "BUFFERED",
                        "duration": track.duration_ms as f64 / 1_000.0,
                        "metadata": metadata,
                    },
                }),
            )?;
            renderer.apply_media(reply["status"].get(0));
            Ok(())
        })
    }

    fn set_next(&self, _next: Option<TrackRequest>) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(std::future::ready(Ok(())))
    }

    fn play(&self) -> BoxFuture<'_, Result<(), String>> {
        self.0
            .blocking(|renderer| renderer.media("PLAY", json!({})))
    }

    fn pause(&self) -> BoxFuture<'_, Result<(), String>> {
        self.0
            .blocking(|renderer| renderer.media("PAUSE", json!({})))
    }

    fn stop(&self) -> BoxFuture<'_, Result<(), String>> {
        self.0.blocking(|renderer| {
            let result = renderer.media("STOP", json!({}));
            let mut state = lock(&renderer.state);
            state.state = PlayState::Stopped;
            state.position_ms = 0;
            drop(state);
            result.or(Ok(()))
        })
    }

    fn seek(&self, position_ms: i64) -> BoxFuture<'_, Result<(), String>> {
        self.0.blocking(move |renderer| {
            renderer.media("SEEK", json!({"currentTime": position_ms as f64 / 1_000.0}))
        })
    }

    fn set_volume(&self, level: u8) -> BoxFuture<'_, Result<(), String>> {
        self.0.blocking(move |renderer| {
            let connection = renderer.connection()?;
            let reply = connection.request(
                RECEIVER,
                NS_RECEIVER,
                json!({"type": "SET_VOLUME", "volume": {"level": f64::from(level.min(100)) / 100.0}}),
            )?;
            renderer.apply_receiver(Some(&reply["status"]));
            Ok(())
        })
    }

    fn set_mute(&self, muted: bool) -> BoxFuture<'_, Result<(), String>> {
        self.0.blocking(move |renderer| {
            let connection = renderer.connection()?;
            let reply = connection.request(
                RECEIVER,
                NS_RECEIVER,
                json!({"type": "SET_VOLUME", "volume": {"muted": muted}}),
            )?;
            renderer.apply_receiver(Some(&reply["status"]));
            Ok(())
        })
    }

    fn poll(&self) -> BoxFuture<'_, ()> {
        let work = self.0.blocking(|renderer| {
            // Only devices we play on keep a connection open.
            let connected = lock(&renderer.connection)
                .as_ref()
                .is_some_and(|connection| connection.alive.load(Ordering::Relaxed));
            if !connected {
                return Ok(());
            }
            let connection = renderer.connection()?;
            let receiver =
                connection.request(RECEIVER, NS_RECEIVER, json!({"type": "GET_STATUS"}))?;
            renderer.apply_receiver(Some(&receiver["status"]));
            let transport = lock(&renderer.playing).transport.clone();
            if let Some(transport) = transport {
                let media =
                    connection.request(&transport, NS_MEDIA, json!({"type": "GET_STATUS"}))?;
                renderer.apply_media(media["status"].get(0));
            }
            Ok(())
        });
        Box::pin(async move {
            if let Err(error) = work.await {
                tracing::debug!(%error, "Chromecast poll failed");
            }
        })
    }
}

/// Every Chromecast the server knows.
pub struct CastController {
    devices: Mutex<HashMap<String, Arc<CastZone>>>,
    changed: tokio::sync::broadcast::Sender<()>,
}

impl CastController {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            devices: Mutex::new(HashMap::new()),
            changed: tokio::sync::broadcast::channel(16).0,
        })
    }

    /// On unless `PARSON_CHROMECAST=off`; devices from DNS-SD and
    /// `PARSON_CHROMECAST_DEVICES` (`host:port`, comma separated).
    pub fn from_env() -> Option<Arc<Self>> {
        let setting = std::env::var("PARSON_CHROMECAST").unwrap_or_default();
        if matches!(
            setting.trim().to_ascii_lowercase().as_str(),
            "off" | "0" | "false" | "no"
        ) {
            return None;
        }
        let controller = Self::new();
        if let Ok(list) = std::env::var("PARSON_CHROMECAST_DEVICES") {
            for (index, address) in list
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .enumerate()
            {
                controller.add(CastDevice {
                    id: format!("manual-{index}"),
                    name: format!("Chromecast {address}"),
                    model: "Chromecast".into(),
                    address: address.to_string(),
                    group: false,
                });
            }
        }
        controller.browse();
        Some(controller)
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.changed.subscribe()
    }

    pub fn add(&self, device: CastDevice) {
        let mut devices = lock(&self.devices);
        match devices.get(&device.id) {
            Some(existing) if *lock(&existing.0.device) == device => return,
            Some(existing) => *lock(&existing.0.device) = device,
            None => {
                devices.insert(
                    device.id.clone(),
                    Arc::new(CastZone(Arc::new(CastRenderer::new(device)))),
                );
            }
        }
        drop(devices);
        let _ = self.changed.send(());
    }

    pub fn devices(&self) -> Vec<CastDevice> {
        let mut devices: Vec<CastDevice> = lock(&self.devices)
            .values()
            .map(|zone| lock(&zone.0.device).clone())
            .collect();
        devices.sort_by(|left, right| left.name.cmp(&right.name));
        devices
    }

    pub fn renderers(&self) -> Vec<Arc<dyn Renderer>> {
        let mut zones: Vec<Arc<CastZone>> = lock(&self.devices).values().cloned().collect();
        zones.sort_by(|left, right| left.0.zone_id.cmp(&right.0.zone_id));
        zones
            .into_iter()
            .map(|zone| zone as Arc<dyn Renderer>)
            .collect()
    }

    pub fn renderer(&self, zone_id: &str) -> Option<Arc<dyn Renderer>> {
        let id = zone_id.strip_prefix(CAST_PREFIX)?;
        lock(&self.devices)
            .get(id)
            .cloned()
            .map(|zone| zone as Arc<dyn Renderer>)
    }

    fn browse(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let _ = std::thread::Builder::new()
            .name("parson-cast-browse".into())
            .spawn(move || {
                let Ok(daemon) = mdns_sd::ServiceDaemon::new() else {
                    return;
                };
                let Ok(events) = daemon.browse(SERVICE_TYPE) else {
                    return;
                };
                loop {
                    let event = events.recv_timeout(Duration::from_secs(5));
                    let Some(controller) = weak.upgrade() else {
                        let _ = daemon.shutdown();
                        return;
                    };
                    let Ok(mdns_sd::ServiceEvent::ServiceResolved(info)) = event else {
                        continue;
                    };
                    let Some(address) = info
                        .get_addresses_v4()
                        .into_iter()
                        .find(|address| !address.is_loopback() && !address.is_unspecified())
                    else {
                        continue;
                    };
                    let Some(id) = info.get_property_val_str("id").map(str::to_string) else {
                        continue;
                    };
                    let model = info
                        .get_property_val_str("md")
                        .unwrap_or("Chromecast")
                        .to_string();
                    controller.add(CastDevice {
                        id,
                        name: info
                            .get_property_val_str("fn")
                            .unwrap_or("Chromecast")
                            .to_string(),
                        group: model == "Google Cast Group",
                        model,
                        address: SocketAddr::new(IpAddr::V4(address), info.get_port()).to_string(),
                    });
                }
            });
    }

    pub fn status(&self) -> serde_json::Value {
        json!({
            "enabled": true,
            "devices": self.devices(),
            "note": CAST_NOTE,
        })
    }
}
