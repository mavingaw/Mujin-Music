//! A fake UPnP/DLNA media renderer for tests: an SSDP responder plus an
//! HTTP server answering the device description, service descriptions and
//! AVTransport / RenderingControl / ConnectionManager SOAP, or (as an
//! OpenHome renderer) the Playlist, Volume, Time and Info services. Like
//! real hardware it fetches the media URL when it starts a track, so tests
//! can check what the server actually streams, and it serves its icon.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::net::{TcpListener, TcpStream, UdpSocket};

use super::description::{
    AV_TRANSPORT, CONNECTION_MANAGER, OH_INFO, OH_PLAYLIST, OH_PRODUCT, OH_TIME, OH_VOLUME,
    RENDERING_CONTROL,
};
use crate::sonos::fake::{read_request, write_response};
use crate::sonos::soap::parse_request;
use crate::sonos::xml::{escape, format_duration, parse_duration};

#[derive(Clone)]
pub struct FakeRendererSpec {
    pub udn: &'static str,
    pub name: &'static str,
    pub manufacturer: &'static str,
    pub model: &'static str,
    /// ConnectionManager `Sink` protocol list.
    pub sink: &'static str,
    /// RenderingControl `Volume` maximum (the minimum is 0).
    pub volume_max: u16,
    /// Lists `SetNextAVTransportURI`.
    pub gapless: bool,
    /// An OpenHome renderer (Product, Playlist, Volume, Time, Info) with no
    /// AVTransport; the playlist's `ProtocolInfo` is `sink`.
    pub openhome: bool,
    /// Serves a PNG icon and lists it in the description.
    pub icon: bool,
    /// `Info.Details` reports this rate instead of what it fetched, as a
    /// renderer that resamples internally would.
    pub reports_rate: Option<u32>,
}

impl Default for FakeRendererSpec {
    fn default() -> Self {
        Self {
            udn: "5f9ec1b3-ed59-1900-4530-00a0deadbeef",
            name: "Living Room Streamer",
            manufacturer: "Acme Audio",
            model: "Stream 9",
            sink: "http-get:*:audio/flac:*,http-get:*:audio/mpeg:*",
            volume_max: 30,
            gapless: true,
            openhome: false,
            icon: false,
            reports_rate: None,
        }
    }
}

/// The bytes the fake serves as its icon.
pub const FAKE_ICON: &[u8] = b"\x89PNG\r\n\x1a\nfake renderer icon";

#[derive(Clone, Debug)]
pub struct FakeFetch {
    pub uri: String,
    pub status: u16,
    pub content_type: Option<String>,
    pub bytes: usize,
    /// The body, saved for ffprobe.
    pub saved: Option<PathBuf>,
}

#[derive(Clone, Debug, Default)]
pub struct FakeRendererState {
    pub uri: String,
    pub metadata: String,
    pub next_uri: String,
    pub next_metadata: String,
    pub transport: &'static str,
    pub position_ms: i64,
    pub playing_since: Option<Instant>,
    pub volume: u16,
    pub muted: bool,
    pub calls: Vec<(String, HashMap<String, String>)>,
    pub fetches: Vec<FakeFetch>,
    /// OpenHome: the playlist `(id, uri, metadata)` and the current id.
    pub playlist: Vec<(u32, String, String)>,
    pub playlist_id: u32,
    pub next_playlist_id: u32,
}

impl FakeRendererState {
    pub fn live_position_ms(&self) -> i64 {
        self.position_ms
            + self
                .playing_since
                .map(|since| since.elapsed().as_millis() as i64)
                .unwrap_or(0)
    }

    fn settle(&mut self) {
        self.position_ms = self.live_position_ms();
        self.playing_since = None;
    }

    pub fn calls(&self, action: &str) -> Vec<HashMap<String, String>> {
        self.calls
            .iter()
            .filter(|(name, _)| name == action)
            .map(|(_, arguments)| arguments.clone())
            .collect()
    }
}

pub struct FakeRenderer {
    pub spec: FakeRendererSpec,
    pub address: SocketAddr,
    pub ssdp_address: SocketAddr,
    pub location: String,
    pub state: Arc<Mutex<FakeRendererState>>,
    http: reqwest::Client,
    directory: PathBuf,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for FakeRenderer {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

impl FakeRenderer {
    pub async fn start(spec: FakeRendererSpec) -> Self {
        let state = Arc::new(Mutex::new(FakeRendererState {
            transport: "NO_MEDIA_PRESENT",
            volume: spec.volume_max / 2,
            ..FakeRendererState::default()
        }));
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(20))
            .build()
            .expect("fake renderer http client");
        let directory =
            std::env::temp_dir().join(format!("parson-fake-renderer-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).expect("fake renderer directory");
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("fake renderer http");
        let address = listener.local_addr().expect("fake renderer address");
        let location = format!("http://{address}/dmr/description.xml");
        let mut tasks = Vec::new();
        {
            let state = state.clone();
            let spec = spec.clone();
            let http = http.clone();
            let directory = directory.clone();
            tasks.push(tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        break;
                    };
                    let state = state.clone();
                    let spec = spec.clone();
                    let http = http.clone();
                    let directory = directory.clone();
                    tokio::spawn(async move {
                        let _ = serve(stream, &spec, state, http, directory).await;
                    });
                }
            }));
        }
        let udp = UdpSocket::bind("127.0.0.1:0").await.expect("fake ssdp");
        let ssdp_address = udp.local_addr().expect("fake ssdp address");
        {
            let location = location.clone();
            let udn = spec.udn;
            let openhome = spec.openhome;
            tasks.push(tokio::spawn(async move {
                let mut buffer = vec![0_u8; 4096];
                loop {
                    let Ok((length, peer)) = udp.recv_from(&mut buffer).await else {
                        break;
                    };
                    let request = String::from_utf8_lossy(&buffer[..length]).to_string();
                    let st = if openhome {
                        "urn:av-openhome-org:service:Product:1"
                    } else {
                        "urn:schemas-upnp-org:device:MediaRenderer:1"
                    };
                    if !request.starts_with("M-SEARCH") || !request.contains(st) {
                        continue;
                    }
                    let answer = format!(
                        "HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=1800\r\nEXT:\r\nLOCATION: {location}\r\nSERVER: Linux/5.10 UPnP/1.0 FakeRenderer/1.0\r\nST: {st}\r\nUSN: uuid:{udn}::{st}\r\n\r\n"
                    );
                    let _ = udp.send_to(answer.as_bytes(), peer).await;
                }
            }));
        }
        Self {
            spec,
            address,
            ssdp_address,
            location,
            state,
            http,
            directory,
            tasks,
        }
    }

    pub fn snapshot(&self) -> FakeRendererState {
        self.state.lock().unwrap().clone()
    }

    /// Simulates the playing track ending: the queued next URI plays
    /// (gapless), or the renderer stops.
    pub fn finish_track(&self) {
        let fetch = {
            let mut state = self.state.lock().unwrap();
            state.settle();
            if self.spec.openhome {
                let index = state
                    .playlist
                    .iter()
                    .position(|(id, _, _)| *id == state.playlist_id);
                match index.and_then(|index| state.playlist.get(index + 1).cloned()) {
                    Some((id, uri, metadata)) => {
                        state.playlist_id = id;
                        state.uri = uri;
                        state.metadata = metadata;
                        state.position_ms = 0;
                        state.transport = "PLAYING";
                        state.playing_since = Some(Instant::now());
                        Some(state.uri.clone())
                    }
                    None => {
                        state.transport = "STOPPED";
                        state.position_ms = 0;
                        None
                    }
                }
            } else if state.next_uri.is_empty() {
                state.transport = "STOPPED";
                state.position_ms = 0;
                None
            } else {
                state.uri = std::mem::take(&mut state.next_uri);
                state.metadata = std::mem::take(&mut state.next_metadata);
                state.position_ms = 0;
                state.transport = "PLAYING";
                state.playing_since = Some(Instant::now());
                Some(state.uri.clone())
            }
        };
        if let Some(uri) = fetch {
            spawn_fetch(
                self.state.clone(),
                self.http.clone(),
                self.directory.clone(),
                uri,
            );
        }
    }

    /// Simulates another app taking over the renderer.
    pub fn play_external(&self, uri: &str) {
        let mut state = self.state.lock().unwrap();
        state.uri = uri.to_string();
        state.metadata = String::new();
        state.next_uri.clear();
        state.transport = "PLAYING";
        state.position_ms = 0;
        state.playing_since = Some(Instant::now());
    }

    pub async fn wait_for<F: Fn(&FakeRendererState) -> bool>(&self, condition: F) -> bool {
        for _ in 0..400 {
            if condition(&self.state.lock().unwrap()) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        false
    }
}

fn spawn_fetch(
    state: Arc<Mutex<FakeRendererState>>,
    http: reqwest::Client,
    directory: PathBuf,
    uri: String,
) {
    tokio::spawn(async move {
        let fetch = match http.get(&uri).send().await {
            Ok(response) => {
                let status = response.status().as_u16();
                let content_type = response
                    .headers()
                    .get("content-type")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string);
                let body = response.bytes().await.unwrap_or_default();
                let saved = (!body.is_empty()).then(|| {
                    let path = directory.join(format!("{}.bin", uuid::Uuid::new_v4()));
                    std::fs::write(&path, &body).expect("save fetched media");
                    path
                });
                FakeFetch {
                    uri,
                    status,
                    content_type,
                    bytes: body.len(),
                    saved,
                }
            }
            Err(_) => FakeFetch {
                uri,
                status: 0,
                content_type: None,
                bytes: 0,
                saved: None,
            },
        };
        state.lock().unwrap().fetches.push(fetch);
    });
}

fn description(spec: &FakeRendererSpec) -> String {
    let icon = if spec.icon {
        "<iconList><icon><mimetype>image/png</mimetype><width>120</width><height>120</height><depth>24</depth><url>/dmr/icon.png</url></icon></iconList>"
    } else {
        ""
    };
    let (device_type, services) = if spec.openhome {
        let service = |kind: &str, name: &str| {
            format!(
                "<service><serviceType>{kind}</serviceType><serviceId>urn:av-openhome-org:serviceId:{name}</serviceId><controlURL>/oh/control/{name}</controlURL><eventSubURL>/oh/event/{name}</eventSubURL><SCPDURL>/oh/{name}.xml</SCPDURL></service>"
            )
        };
        (
            "urn:av-openhome-org:device:Source:1",
            [
                service(OH_PRODUCT, "Product"),
                service(OH_PLAYLIST, "Playlist"),
                service(OH_VOLUME, "Volume"),
                service(OH_TIME, "Time"),
                service(OH_INFO, "Info"),
            ]
            .concat(),
        )
    } else {
        (
            "urn:schemas-upnp-org:device:MediaRenderer:1",
            format!(
                "<service><serviceType>{AV_TRANSPORT}</serviceType><serviceId>urn:upnp-org:serviceId:AVTransport</serviceId><controlURL>/dmr/control/avt</controlURL><eventSubURL>/dmr/event/avt</eventSubURL><SCPDURL>avt.xml</SCPDURL></service><service><serviceType>{RENDERING_CONTROL}</serviceType><serviceId>urn:upnp-org:serviceId:RenderingControl</serviceId><controlURL>/dmr/control/rc</controlURL><eventSubURL>/dmr/event/rc</eventSubURL><SCPDURL>rc.xml</SCPDURL></service><service><serviceType>{CONNECTION_MANAGER}</serviceType><serviceId>urn:upnp-org:serviceId:ConnectionManager</serviceId><controlURL>/dmr/control/cm</controlURL><eventSubURL>/dmr/event/cm</eventSubURL><SCPDURL>cm.xml</SCPDURL></service>"
            ),
        )
    };
    format!(
        r#"<?xml version="1.0"?><root xmlns="urn:schemas-upnp-org:device-1-0"><specVersion><major>1</major><minor>0</minor></specVersion><device><deviceType>{device_type}</deviceType><friendlyName>{name}</friendlyName><manufacturer>{manufacturer}</manufacturer><modelName>{model}</modelName><modelDescription>Network music player</modelDescription><UDN>uuid:{udn}</UDN>{icon}<serviceList>{services}</serviceList></device></root>"#,
        name = escape(spec.name),
        manufacturer = escape(spec.manufacturer),
        model = escape(spec.model),
        udn = spec.udn,
    )
}

/// Rate and depth of what the renderer fetched last: a FLAC's STREAMINFO,
/// or the parameters of an LPCM type.
fn decoded_format(state: &FakeRendererState) -> Option<(u32, u8, &'static str)> {
    let fetch = state.fetches.last()?;
    let content_type = fetch.content_type.as_deref().unwrap_or_default();
    if let Some(parameters) = content_type.strip_prefix("audio/L") {
        let bits: u8 = parameters.split(';').next()?.parse().ok()?;
        let rate = parameters
            .split(';')
            .find_map(|part| part.trim().strip_prefix("rate="))?
            .parse()
            .ok()?;
        return Some((rate, bits, "PCM"));
    }
    let bytes = std::fs::read(fetch.saved.as_ref()?).ok()?;
    // fLaC, then the STREAMINFO block: rate is 20 bits at byte 18, bits
    // per sample (minus one) 5 bits after the 3 channel bits.
    if bytes.len() < 26 || &bytes[..4] != b"fLaC" {
        return None;
    }
    let packed = u64::from_be_bytes(bytes[18..26].try_into().ok()?);
    let rate = (packed >> 44) as u32;
    let bits = (((packed >> 36) & 0x1f) + 1) as u8;
    Some((rate, bits, "FLAC"))
}

fn scpd(actions: &[&str], variables: &str) -> String {
    let mut document = String::from(
        r#"<?xml version="1.0"?><scpd xmlns="urn:schemas-upnp-org:service-1-0"><actionList>"#,
    );
    for action in actions {
        document.push_str(&format!("<action><name>{action}</name></action>"));
    }
    document.push_str("</actionList><serviceStateTable>");
    document.push_str(variables);
    document.push_str("</serviceStateTable></scpd>");
    document
}

fn track_duration(metadata: &str) -> i64 {
    roxmltree::Document::parse(metadata)
        .ok()
        .and_then(|document| {
            document
                .descendants()
                .find(|node| node.is_element() && node.tag_name().name() == "res")
                .and_then(|node| node.attribute("duration"))
                .and_then(parse_duration)
        })
        .unwrap_or(0)
}

type ActionResult = Result<Vec<(&'static str, String)>, u32>;

fn act(
    state: &mut FakeRendererState,
    spec: &FakeRendererSpec,
    action: &str,
    arguments: &HashMap<String, String>,
) -> (ActionResult, Option<String>) {
    let argument = |name: &str| arguments.get(name).cloned().unwrap_or_default();
    match action {
        "SetAVTransportURI" => {
            state.settle();
            state.uri = argument("CurrentURI");
            state.metadata = argument("CurrentURIMetaData");
            state.next_uri.clear();
            state.next_metadata.clear();
            state.position_ms = 0;
            state.transport = "STOPPED";
            (Ok(Vec::new()), None)
        }
        "SetNextAVTransportURI" if spec.gapless => {
            state.next_uri = argument("NextURI");
            state.next_metadata = argument("NextURIMetaData");
            (Ok(Vec::new()), None)
        }
        "Play" => {
            if state.uri.is_empty() {
                return (Err(701), None);
            }
            let starting = state.transport != "PAUSED_PLAYBACK";
            state.transport = "PLAYING";
            state.playing_since = Some(Instant::now());
            // A real renderer opens the stream when it starts a track.
            let fetch = (starting && !state.fetches.iter().any(|fetch| fetch.uri == state.uri))
                .then(|| state.uri.clone());
            (Ok(Vec::new()), fetch)
        }
        "Pause" => {
            if state.transport != "PLAYING" {
                return (Err(701), None);
            }
            state.settle();
            state.transport = "PAUSED_PLAYBACK";
            (Ok(Vec::new()), None)
        }
        "Stop" => {
            state.settle();
            if !state.uri.is_empty() {
                state.transport = "STOPPED";
            }
            state.position_ms = 0;
            (Ok(Vec::new()), None)
        }
        "Seek" => match parse_duration(&argument("Target")) {
            Some(position) if argument("Unit") == "REL_TIME" => {
                let playing = state.playing_since.is_some();
                state.settle();
                state.position_ms = position;
                if playing {
                    state.playing_since = Some(Instant::now());
                }
                (Ok(Vec::new()), None)
            }
            _ => (Err(711), None),
        },
        "GetTransportInfo" => (
            Ok(vec![
                ("CurrentTransportState", state.transport.to_string()),
                ("CurrentTransportStatus", "OK".to_string()),
                ("CurrentSpeed", "1".to_string()),
            ]),
            None,
        ),
        "GetPositionInfo" => {
            let duration = track_duration(&state.metadata);
            let position = state.live_position_ms().min(duration.max(0));
            (
                Ok(vec![
                    (
                        "Track",
                        if state.uri.is_empty() { "0" } else { "1" }.to_string(),
                    ),
                    ("TrackDuration", format_duration(duration)),
                    ("TrackMetaData", state.metadata.clone()),
                    ("TrackURI", state.uri.clone()),
                    ("RelTime", format_duration(position)),
                    ("AbsTime", "NOT_IMPLEMENTED".to_string()),
                ]),
                None,
            )
        }
        "GetVolume" => (Ok(vec![("CurrentVolume", state.volume.to_string())]), None),
        "SetVolume" => match argument("DesiredVolume").parse::<u16>() {
            Ok(volume) if volume <= spec.volume_max => {
                state.volume = volume;
                (Ok(Vec::new()), None)
            }
            _ => (Err(402), None),
        },
        "GetMute" => (
            Ok(vec![(
                "CurrentMute",
                if state.muted { "1" } else { "0" }.to_string(),
            )]),
            None,
        ),
        "SetMute" => {
            state.muted = matches!(argument("DesiredMute").as_str(), "1" | "true");
            (Ok(Vec::new()), None)
        }
        "GetVolumeDBRange" => (
            Ok(vec![
                ("MinValue", (-80 * 256).to_string()),
                ("MaxValue", "0".to_string()),
            ]),
            None,
        ),
        "GetProtocolInfo" => (
            Ok(vec![
                ("Source", String::new()),
                ("Sink", spec.sink.to_string()),
            ]),
            None,
        ),
        _ => (Err(401), None),
    }
}

/// OpenHome actions, by service.
fn act_open_home(
    state: &mut FakeRendererState,
    spec: &FakeRendererSpec,
    service: &str,
    action: &str,
    arguments: &HashMap<String, String>,
) -> (ActionResult, Option<String>) {
    let argument = |name: &str| arguments.get(name).cloned().unwrap_or_default();
    let number = |name: &str| argument(name).trim().parse::<u32>().ok();
    let select = |state: &mut FakeRendererState, id: u32| -> bool {
        let Some((_, uri, metadata)) = state.playlist.iter().find(|(entry, _, _)| *entry == id)
        else {
            return false;
        };
        let (uri, metadata) = (uri.clone(), metadata.clone());
        state.playlist_id = id;
        state.uri = uri;
        state.metadata = metadata;
        state.position_ms = 0;
        true
    };
    match (service, action) {
        ("Playlist", "DeleteAll") => {
            state.settle();
            state.playlist.clear();
            state.playlist_id = 0;
            state.transport = "STOPPED";
            (Ok(Vec::new()), None)
        }
        ("Playlist", "Insert") => {
            let after = number("AfterId").unwrap_or(0);
            let position = if after == 0 {
                0
            } else {
                match state.playlist.iter().position(|(id, _, _)| *id == after) {
                    Some(index) => index + 1,
                    None => return (Err(800), None),
                }
            };
            state.next_playlist_id += 1;
            let id = state.next_playlist_id;
            state
                .playlist
                .insert(position, (id, argument("Uri"), argument("Metadata")));
            (Ok(vec![("NewId", id.to_string())]), None)
        }
        ("Playlist", "DeleteId") => {
            let id = number("Value").unwrap_or(0);
            state.playlist.retain(|(entry, _, _)| *entry != id);
            (Ok(Vec::new()), None)
        }
        ("Playlist", "SeekId") => {
            if !select(state, number("Value").unwrap_or(0)) {
                return (Err(800), None);
            }
            // Selecting a track starts it, as on a Linn.
            state.transport = "PLAYING";
            state.playing_since = Some(Instant::now());
            let fetch = (!state.fetches.iter().any(|fetch| fetch.uri == state.uri))
                .then(|| state.uri.clone());
            (Ok(Vec::new()), fetch)
        }
        ("Playlist", "Play") => {
            if state.uri.is_empty() {
                return (Err(800), None);
            }
            if state.transport != "PLAYING" {
                state.transport = "PLAYING";
                state.playing_since = Some(Instant::now());
            }
            let fetch = (!state.fetches.iter().any(|fetch| fetch.uri == state.uri))
                .then(|| state.uri.clone());
            (Ok(Vec::new()), fetch)
        }
        ("Playlist", "Pause") => {
            state.settle();
            state.transport = "PAUSED_PLAYBACK";
            (Ok(Vec::new()), None)
        }
        ("Playlist", "Stop") => {
            state.settle();
            state.transport = "STOPPED";
            state.position_ms = 0;
            (Ok(Vec::new()), None)
        }
        ("Playlist", "SeekSecondAbsolute") => {
            let playing = state.playing_since.is_some();
            state.settle();
            state.position_ms = i64::from(number("Value").unwrap_or(0)) * 1_000;
            if playing {
                state.playing_since = Some(Instant::now());
            }
            (Ok(Vec::new()), None)
        }
        ("Playlist", "TransportState") => (
            Ok(vec![(
                "Value",
                match state.transport {
                    "PLAYING" => "Playing",
                    "PAUSED_PLAYBACK" => "Paused",
                    _ => "Stopped",
                }
                .to_string(),
            )]),
            None,
        ),
        ("Playlist", "Id") => (Ok(vec![("Value", state.playlist_id.to_string())]), None),
        ("Playlist", "ProtocolInfo") => (Ok(vec![("Value", spec.sink.to_string())]), None),
        ("Time", "Time") => {
            let duration = track_duration(&state.metadata);
            let position = state.live_position_ms().min(duration.max(0));
            (
                Ok(vec![
                    ("TrackCount", state.playlist.len().to_string()),
                    ("Duration", (duration / 1_000).to_string()),
                    ("Seconds", (position / 1_000).to_string()),
                ]),
                None,
            )
        }
        ("Volume", "Characteristics") => (
            Ok(vec![
                ("VolumeMax", spec.volume_max.to_string()),
                ("VolumeUnity", spec.volume_max.to_string()),
                ("VolumeSteps", spec.volume_max.to_string()),
                ("VolumeMilliDbPerStep", "1000".to_string()),
                ("BalanceMax", "0".to_string()),
                ("FadeMax", "0".to_string()),
            ]),
            None,
        ),
        ("Volume", "Volume") => (Ok(vec![("Value", state.volume.to_string())]), None),
        ("Volume", "SetVolume") => match argument("Value").parse::<u16>() {
            Ok(volume) if volume <= spec.volume_max => {
                state.volume = volume;
                (Ok(Vec::new()), None)
            }
            _ => (Err(402), None),
        },
        ("Volume", "Mute") => (
            Ok(vec![(
                "Value",
                if state.muted { "true" } else { "false" }.to_string(),
            )]),
            None,
        ),
        ("Volume", "SetMute") => {
            state.muted = matches!(argument("Value").as_str(), "1" | "true");
            (Ok(Vec::new()), None)
        }
        ("Info", "Details") => match decoded_format(state) {
            Some((rate, bits, codec)) if state.transport == "PLAYING" => (
                Ok(vec![
                    (
                        "Duration",
                        (track_duration(&state.metadata) / 1_000).to_string(),
                    ),
                    ("BitRate", "0".to_string()),
                    ("BitDepth", bits.to_string()),
                    ("SampleRate", spec.reports_rate.unwrap_or(rate).to_string()),
                    ("Lossless", "true".to_string()),
                    ("CodecName", codec.to_string()),
                ]),
                None,
            ),
            _ => (
                Ok(vec![
                    ("Duration", "0".to_string()),
                    ("BitRate", "0".to_string()),
                    ("BitDepth", "0".to_string()),
                    ("SampleRate", "0".to_string()),
                    ("Lossless", "false".to_string()),
                    ("CodecName", String::new()),
                ]),
                None,
            ),
        },
        ("Info", "Track") => (
            Ok(vec![
                ("Uri", state.uri.clone()),
                ("Metadata", state.metadata.clone()),
            ]),
            None,
        ),
        _ => (Err(401), None),
    }
}

async fn write_bytes(
    stream: &mut TcpStream,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    stream
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        )
        .await?;
    stream.write_all(body).await
}

async fn serve(
    mut stream: TcpStream,
    spec: &FakeRendererSpec,
    state: Arc<Mutex<FakeRendererState>>,
    http: reqwest::Client,
    directory: PathBuf,
) -> std::io::Result<()> {
    let Some(request) = read_request(&mut stream).await else {
        return Ok(());
    };
    if request.method == "GET" {
        let body = match request.path.as_str() {
            "/dmr/description.xml" => description(spec),
            "/dmr/avt.xml" => {
                let mut actions = vec![
                    "SetAVTransportURI",
                    "Play",
                    "Pause",
                    "Stop",
                    "Seek",
                    "GetTransportInfo",
                    "GetPositionInfo",
                ];
                if spec.gapless {
                    actions.push("SetNextAVTransportURI");
                }
                scpd(&actions, "")
            }
            "/dmr/rc.xml" => scpd(
                &[
                    "GetVolume",
                    "SetVolume",
                    "GetMute",
                    "SetMute",
                    "GetVolumeDBRange",
                ],
                &format!(
                    r#"<stateVariable sendEvents="no"><name>Volume</name><dataType>ui2</dataType><allowedValueRange><minimum>0</minimum><maximum>{}</maximum><step>1</step></allowedValueRange></stateVariable>"#,
                    spec.volume_max
                ),
            ),
            "/dmr/cm.xml" => scpd(&["GetProtocolInfo"], ""),
            "/dmr/icon.png" if spec.icon => {
                return write_bytes(&mut stream, "image/png", FAKE_ICON).await;
            }
            path if path.starts_with("/oh/") && path.ends_with(".xml") => scpd(&[], ""),
            _ => return write_response(&mut stream, "404 Not Found", &[], "").await,
        };
        return write_response(&mut stream, "200 OK", &[], &body).await;
    }
    let (urn, open_home) = match request.path.as_str() {
        "/dmr/control/avt" => (AV_TRANSPORT, None),
        "/dmr/control/rc" => (RENDERING_CONTROL, None),
        "/dmr/control/cm" => (CONNECTION_MANAGER, None),
        "/oh/control/Product" => (OH_PRODUCT, Some("Product")),
        "/oh/control/Playlist" => (OH_PLAYLIST, Some("Playlist")),
        "/oh/control/Volume" => (OH_VOLUME, Some("Volume")),
        "/oh/control/Time" => (OH_TIME, Some("Time")),
        "/oh/control/Info" => (OH_INFO, Some("Info")),
        _ => return write_response(&mut stream, "404 Not Found", &[], "").await,
    };
    let Ok((action, arguments)) = parse_request(&request.body) else {
        return write_response(&mut stream, "400 Bad Request", &[], "").await;
    };
    if request.headers.get("soapaction") != Some(&format!("\"{urn}#{action}\"")) {
        return write_response(&mut stream, "400 Bad Request", &[], "").await;
    }
    let (result, fetch) = {
        let mut state = state.lock().unwrap();
        state.calls.push((action.clone(), arguments.clone()));
        match open_home {
            Some(service) => act_open_home(&mut state, spec, service, &action, &arguments),
            None => act(&mut state, spec, &action, &arguments),
        }
    };
    if let Some(uri) = fetch {
        spawn_fetch(state.clone(), http, directory, uri);
    }
    match result {
        Ok(outputs) => {
            let mut body = format!(
                r#"<?xml version="1.0"?><s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/"><s:Body><u:{action}Response xmlns:u="{urn}">"#
            );
            for (name, value) in outputs {
                body.push_str(&format!("<{name}>{}</{name}>", escape(&value)));
            }
            body.push_str(&format!("</u:{action}Response></s:Body></s:Envelope>"));
            write_response(&mut stream, "200 OK", &[], &body).await
        }
        Err(code) => {
            let body = format!(
                r#"<?xml version="1.0"?><s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body><s:Fault><faultcode>s:Client</faultcode><faultstring>UPnPError</faultstring><detail><UPnPError xmlns="urn:schemas-upnp-org:control-1-0"><errorCode>{code}</errorCode></UPnPError></detail></s:Fault></s:Body></s:Envelope>"#
            );
            write_response(&mut stream, "500 Internal Server Error", &[], &body).await
        }
    }
}
