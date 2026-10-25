//! A fake Sonos household for tests: SSDP responder plus one UPnP HTTP
//! server per player answering device description, AVTransport,
//! RenderingControl, GroupRenderingControl and ZoneGroupTopology SOAP, GENA
//! SUBSCRIBE, and sending NOTIFY events. On `Play` each player fetches the
//! queued media URL like real hardware, so tests can prove the signed URL
//! streams.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};

use super::soap::{Service, parse_request};
use super::xml::{escape, format_duration, parse_duration};

/// A player to create.
#[derive(Clone)]
pub struct FakePlayerSpec {
    pub uuid: &'static str,
    pub room: &'static str,
    pub model: &'static str,
    /// For the hidden secondary of a stereo pair: the primary's uuid.
    pub pair_of: Option<&'static str>,
}

#[derive(Clone, Debug)]
pub struct FakePlayer {
    pub uuid: String,
    pub room: String,
    pub model: String,
    pub address: SocketAddr,
    pub pair_of: Option<String>,
    pub coordinator: String,
    pub queue: Vec<(String, String)>,
    pub transport_uri: String,
    pub state: &'static str,
    pub track: u32,
    pub position_ms: i64,
    /// When playback last (re)started, so the position advances in real time.
    pub playing_since: Option<std::time::Instant>,
    pub volume: u8,
    pub muted: bool,
    pub play_mode: String,
    pub crossfade: bool,
    /// Overrides the reported `TrackDuration`, like a speaker that still
    /// reports the previous track's length right after a change.
    pub reported_duration_ms: Option<i64>,
}

impl FakePlayer {
    /// Position including time spent playing since the last state change.
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
}

#[derive(Clone, Debug)]
pub struct FakeCall {
    pub player: String,
    pub action: String,
    pub arguments: HashMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct MediaFetch {
    pub uri: String,
    pub status: u16,
    pub content_type: Option<String>,
    pub bytes: usize,
}

#[derive(Default)]
pub struct Household {
    pub players: Vec<FakePlayer>,
    pub calls: Vec<FakeCall>,
    pub media_fetches: Vec<MediaFetch>,
    pub notifications_sent: usize,
    subscriptions: Vec<(String, &'static str, String)>,
}

impl Household {
    fn index(&self, uuid: &str) -> Option<usize> {
        self.players.iter().position(|player| player.uuid == uuid)
    }

    fn effective_coordinator(&self, index: usize) -> String {
        let player = &self.players[index];
        match &player.pair_of {
            Some(primary) => self
                .index(primary)
                .map(|primary| self.players[primary].coordinator.clone())
                .unwrap_or_else(|| player.coordinator.clone()),
            None => player.coordinator.clone(),
        }
    }

    fn topology(&self) -> String {
        let mut xml = String::from("<ZoneGroupState><ZoneGroups>");
        for (index, coordinator) in self.players.iter().enumerate() {
            if coordinator.pair_of.is_some() || coordinator.coordinator != coordinator.uuid {
                continue;
            }
            xml.push_str(&format!(
                r#"<ZoneGroup Coordinator="{0}" ID="{0}:{1}">"#,
                coordinator.uuid,
                index + 1
            ));
            for (member_index, member) in self.players.iter().enumerate() {
                if self.effective_coordinator(member_index) != coordinator.uuid {
                    continue;
                }
                xml.push_str(&format!(
                    r#"<ZoneGroupMember UUID="{}" Location="http://{}/xml/device_description.xml" ZoneName="{}"{}/>"#,
                    member.uuid,
                    member.address,
                    escape(&member.room),
                    if member.pair_of.is_some() {
                        r#" Invisible="1""#
                    } else {
                        ""
                    }
                ));
            }
            xml.push_str("</ZoneGroup>");
        }
        xml.push_str("</ZoneGroups><VanishedDevices/></ZoneGroupState>");
        xml
    }
}

/// Running fake household.
pub struct FakeSonos {
    pub ssdp_address: SocketAddr,
    pub household: Arc<Mutex<Household>>,
    http: reqwest::Client,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for FakeSonos {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl FakeSonos {
    pub async fn start(specs: &[FakePlayerSpec]) -> Self {
        let household = Arc::new(Mutex::new(Household::default()));
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("fake sonos http client");
        let mut tasks = Vec::new();
        for spec in specs {
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("fake http");
            let address = listener.local_addr().expect("fake http address");
            household.lock().unwrap().players.push(FakePlayer {
                uuid: spec.uuid.to_string(),
                room: spec.room.to_string(),
                model: spec.model.to_string(),
                address,
                pair_of: spec.pair_of.map(str::to_string),
                coordinator: spec.pair_of.unwrap_or(spec.uuid).to_string(),
                queue: Vec::new(),
                transport_uri: String::new(),
                state: "STOPPED",
                track: 0,
                position_ms: 0,
                playing_since: None,
                volume: 20,
                muted: false,
                play_mode: "NORMAL".to_string(),
                crossfade: false,
                reported_duration_ms: None,
            });
            let uuid = spec.uuid.to_string();
            let state = household.clone();
            let client = http.clone();
            tasks.push(tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        break;
                    };
                    let state = state.clone();
                    let uuid = uuid.clone();
                    let client = client.clone();
                    tokio::spawn(async move {
                        let _ = serve_connection(stream, &uuid, state, client).await;
                    });
                }
            }));
        }
        let udp = UdpSocket::bind("127.0.0.1:0").await.expect("fake ssdp");
        let ssdp_address = udp.local_addr().expect("fake ssdp address");
        let state = household.clone();
        tasks.push(tokio::spawn(async move {
            let mut buffer = vec![0_u8; 4096];
            loop {
                let Ok((length, peer)) = udp.recv_from(&mut buffer).await else {
                    break;
                };
                let request = String::from_utf8_lossy(&buffer[..length]).to_string();
                if !request.starts_with("M-SEARCH")
                    || !request.contains("urn:schemas-upnp-org:device:ZonePlayer:1")
                {
                    continue;
                }
                let answers: Vec<String> = state
                    .lock()
                    .unwrap()
                    .players
                    .iter()
                    .map(|player| {
                        format!(
                            "HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age = 1800\r\nEXT:\r\nLOCATION: http://{}/xml/device_description.xml\r\nSERVER: Linux UPnP/1.0 Sonos/79.1-56030 (ZPS9)\r\nST: urn:schemas-upnp-org:device:ZonePlayer:1\r\nUSN: uuid:{}::urn:schemas-upnp-org:device:ZonePlayer:1\r\nX-RINCON-HOUSEHOLD: Sonos_fake\r\n\r\n",
                            player.address, player.uuid
                        )
                    })
                    .collect();
                for answer in answers {
                    let _ = udp.send_to(answer.as_bytes(), peer).await;
                }
            }
        }));
        Self {
            ssdp_address,
            household,
            http,
            tasks,
        }
    }

    pub fn player(&self, uuid: &str) -> FakePlayer {
        let household = self.household.lock().unwrap();
        household.players[household.index(uuid).expect("fake player")].clone()
    }

    pub fn calls(&self, action: &str) -> Vec<FakeCall> {
        self.household
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|call| call.action == action)
            .cloned()
            .collect()
    }

    pub fn media_fetches(&self) -> Vec<MediaFetch> {
        self.household.lock().unwrap().media_fetches.clone()
    }

    pub fn notifications_sent(&self) -> usize {
        self.household.lock().unwrap().notifications_sent
    }

    /// Simulates the current track ending on its own.
    pub fn advance(&self, uuid: &str) {
        let subscribers = {
            let mut household = self.household.lock().unwrap();
            let index = household.index(uuid).expect("fake player");
            let player = &mut household.players[index];
            player.settle();
            if player.play_mode == "REPEAT_ONE" {
                player.position_ms = 0;
            } else if (player.track as usize) < player.queue.len() {
                player.track += 1;
                player.position_ms = 0;
            } else if player.play_mode == "REPEAT_ALL" && !player.queue.is_empty() {
                player.track = 1;
                player.position_ms = 0;
            } else {
                player.state = "STOPPED";
            }
            if player.state == "PLAYING" {
                player.playing_since = Some(std::time::Instant::now());
            }
            subscribers_for(&household, uuid, Service::AVTransport)
        };
        send_notifications(self.household.clone(), self.http.clone(), subscribers);
    }

    /// Makes the speaker report `duration_ms` as the track length (or the
    /// queued track's real length again with `None`).
    pub fn report_duration(&self, uuid: &str, duration_ms: Option<i64>) {
        let mut household = self.household.lock().unwrap();
        let index = household.index(uuid).expect("fake player");
        household.players[index].reported_duration_ms = duration_ms;
    }

    /// Simulates the Sonos app starting something Parson did not queue.
    pub fn play_external(&self, uuid: &str, uri: &str) {
        let mut household = self.household.lock().unwrap();
        let index = household.index(uuid).expect("fake player");
        let player = &mut household.players[index];
        player.transport_uri = uri.to_string();
        player.queue.clear();
        player.track = 1;
        player.position_ms = 0;
        player.state = "PLAYING";
        player.playing_since = Some(std::time::Instant::now());
    }

    pub async fn wait_for<F: Fn(&Household) -> bool>(&self, condition: F) -> bool {
        for _ in 0..200 {
            if condition(&self.household.lock().unwrap()) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    }
}

pub(crate) struct Request {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: String,
}

pub(crate) async fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break end;
        }
        if buffer.len() > 256 * 1024 {
            return None;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next()?.split_whitespace();
    let method = request_line.next()?.to_string();
    let path = request_line.next()?.to_string();
    let headers: HashMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    let length: usize = headers
        .get("content-length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let mut body = buffer[header_end + 4..].to_vec();
    while body.len() < length {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    Some(Request {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).to_string(),
    })
}

pub(crate) async fn write_response(
    stream: &mut TcpStream,
    status: &str,
    extra_headers: &[(&str, String)],
    body: &str,
) -> std::io::Result<()> {
    let mut response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/xml; charset=\"utf-8\"\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in extra_headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str("\r\n");
    response.push_str(body);
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await
}

async fn serve_connection(
    mut stream: TcpStream,
    uuid: &str,
    household: Arc<Mutex<Household>>,
    http: reqwest::Client,
) -> std::io::Result<()> {
    let Some(request) = read_request(&mut stream).await else {
        return Ok(());
    };
    if request.method == "GET" && request.path == "/xml/device_description.xml" {
        let description = {
            let household = household.lock().unwrap();
            let player = &household.players[household.index(uuid).unwrap()];
            format!(
                r#"<?xml version="1.0" encoding="utf-8" ?><root xmlns="urn:schemas-upnp-org:device-1-0"><device><deviceType>urn:schemas-upnp-org:device:ZonePlayer:1</deviceType><friendlyName>{addr} - {model}</friendlyName><manufacturer>Sonos, Inc.</manufacturer><modelNumber>S6</modelNumber><modelName>{model}</modelName><displayName>{display}</displayName><softwareVersion>79.1-56030</softwareVersion><roomName>{room}</roomName><UDN>uuid:{uuid}</UDN></device></root>"#,
                addr = player.address,
                model = escape(&player.model),
                display = escape(player.model.trim_start_matches("Sonos ")),
                room = escape(&player.room),
                uuid = player.uuid,
            )
        };
        return write_response(&mut stream, "200 OK", &[], &description).await;
    }
    if request.method == "SUBSCRIBE" {
        let Some(service) = Service::from_event_path(&request.path) else {
            return write_response(&mut stream, "404 Not Found", &[], "").await;
        };
        let callback = request
            .headers
            .get("callback")
            .map(|value| value.trim_matches(['<', '>']).to_string())
            .unwrap_or_default();
        let sid = format!("uuid:sub-{}", uuid::Uuid::new_v4());
        household.lock().unwrap().subscriptions.push((
            uuid.to_string(),
            service.event_path(),
            callback,
        ));
        return write_response(
            &mut stream,
            "200 OK",
            &[("SID", sid), ("TIMEOUT", "Second-300".to_string())],
            "",
        )
        .await;
    }
    let Some(service) = (request.method == "POST")
        .then(|| Service::from_control_path(&request.path))
        .flatten()
    else {
        return write_response(&mut stream, "404 Not Found", &[], "").await;
    };
    let Ok((action, arguments)) = parse_request(&request.body) else {
        return write_response(&mut stream, "400 Bad Request", &[], "").await;
    };
    let expected_header = format!("\"{}#{action}\"", service.urn());
    if request.headers.get("soapaction") != Some(&expected_header) {
        return write_response(&mut stream, "400 Bad Request", &[], "").await;
    }
    let (result, fetch, notify) = {
        let mut household = household.lock().unwrap();
        household.calls.push(FakeCall {
            player: uuid.to_string(),
            action: action.clone(),
            arguments: arguments.clone(),
        });
        let result = handle_action(&mut household, uuid, service, &action, &arguments);
        let index = household.index(uuid).unwrap();
        let fetch = (action == "Play" && result.is_ok())
            .then(|| current_uri(&household.players[index]))
            .flatten();
        let notify = if result.is_ok() && !action.starts_with("Get") {
            subscribers_for(&household, uuid, service)
        } else {
            Vec::new()
        };
        (result, fetch, notify)
    };
    if let Some(uri) = fetch {
        let household = household.clone();
        let http = http.clone();
        tokio::spawn(async move {
            let fetched = match http.get(&uri).send().await {
                Ok(response) => {
                    let status = response.status().as_u16();
                    let content_type = response
                        .headers()
                        .get("content-type")
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string);
                    let bytes = response.bytes().await.map(|body| body.len()).unwrap_or(0);
                    MediaFetch {
                        uri,
                        status,
                        content_type,
                        bytes,
                    }
                }
                Err(_) => MediaFetch {
                    uri,
                    status: 0,
                    content_type: None,
                    bytes: 0,
                },
            };
            household.lock().unwrap().media_fetches.push(fetched);
        });
    }
    send_notifications(household.clone(), http, notify);
    match result {
        Ok(outputs) => {
            let mut body = format!(
                r#"<?xml version="1.0"?><s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/"><s:Body><u:{action}Response xmlns:u="{}">"#,
                service.urn()
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

fn current_uri(player: &FakePlayer) -> Option<String> {
    if player.transport_uri.starts_with("x-rincon-queue:") {
        player
            .queue
            .get(player.track.saturating_sub(1) as usize)
            .map(|(uri, _)| uri.clone())
    } else {
        (!player.transport_uri.is_empty()).then(|| player.transport_uri.clone())
    }
}

fn subscribers_for(household: &Household, uuid: &str, service: Service) -> Vec<String> {
    household
        .subscriptions
        .iter()
        .filter(|(player, path, _)| player == uuid && *path == service.event_path())
        .map(|(_, _, callback)| callback.clone())
        .collect()
}

fn send_notifications(
    household: Arc<Mutex<Household>>,
    http: reqwest::Client,
    callbacks: Vec<String>,
) {
    if callbacks.is_empty() {
        return;
    }
    tokio::spawn(async move {
        let method = reqwest::Method::from_bytes(b"NOTIFY").unwrap();
        for callback in callbacks {
            let body = r#"<e:propertyset xmlns:e="urn:schemas-upnp-org:event-1-0"><e:property><LastChange>&lt;Event xmlns=&quot;urn:schemas-upnp-org:metadata-1-0/AVT/&quot;/&gt;</LastChange></e:property></e:propertyset>"#;
            let sent = http
                .request(method.clone(), &callback)
                .header("NT", "upnp:event")
                .header("NTS", "upnp:propchange")
                .header("SEQ", "0")
                .header("Content-Type", "text/xml")
                .body(body)
                .send()
                .await;
            if sent.is_ok() {
                household.lock().unwrap().notifications_sent += 1;
            }
        }
    });
}

type ActionResult = Result<Vec<(&'static str, String)>, u32>;

/// Track length from the queued DIDL-Lite `res@duration` (3 minutes if absent).
fn track_duration_ms(metadata: &str) -> i64 {
    roxmltree::Document::parse(metadata)
        .ok()
        .and_then(|document| {
            document
                .descendants()
                .find(|node| node.is_element() && node.tag_name().name() == "res")
                .and_then(|node| node.attribute("duration"))
                .and_then(parse_duration)
        })
        .unwrap_or(180_000)
}

/// Runs an action with real-time position: playback advances while
/// `PLAYING` and moves to the next queued track when one ends.
fn handle_action(
    household: &mut Household,
    uuid: &str,
    service: Service,
    action: &str,
    arguments: &HashMap<String, String>,
) -> ActionResult {
    if let Some(index) = household.index(uuid) {
        let player = &mut household.players[index];
        player.settle();
        if player.state == "PLAYING" && player.transport_uri.starts_with("x-rincon-queue:") {
            while let Some((_, metadata)) =
                player.queue.get(player.track.saturating_sub(1) as usize)
            {
                let duration = track_duration_ms(metadata);
                if player.position_ms < duration {
                    break;
                }
                if player.play_mode == "REPEAT_ONE" {
                    player.position_ms -= duration;
                } else if (player.track as usize) < player.queue.len() {
                    player.position_ms -= duration;
                    player.track += 1;
                } else if player.play_mode == "REPEAT_ALL" {
                    player.position_ms -= duration;
                    player.track = 1;
                } else {
                    player.position_ms = duration;
                    player.state = "STOPPED";
                    break;
                }
            }
        }
    }
    let result = dispatch_action(household, uuid, service, action, arguments);
    if let Some(index) = household.index(uuid) {
        let player = &mut household.players[index];
        if player.state == "PLAYING" {
            player.playing_since = Some(std::time::Instant::now());
        }
    }
    result
}

fn dispatch_action(
    household: &mut Household,
    uuid: &str,
    service: Service,
    action: &str,
    arguments: &HashMap<String, String>,
) -> ActionResult {
    let index = household.index(uuid).ok_or(402_u32)?;
    let argument = |name: &str| arguments.get(name).cloned().ok_or(402_u32);
    let is_coordinator = household.effective_coordinator(index) == uuid;
    match (service, action) {
        (Service::ZoneGroupTopology, "GetZoneGroupState") => {
            Ok(vec![("ZoneGroupState", household.topology())])
        }
        (Service::AVTransport, "SetAVTransportURI") => {
            let uri = argument("CurrentURI")?;
            if let Some(coordinator) = uri.strip_prefix("x-rincon:") {
                let coordinator_index = household.index(coordinator).ok_or(402_u32)?;
                let target = household.effective_coordinator(coordinator_index);
                // Players coordinated by this one would be orphaned; refuse like Sonos.
                household.players[index].coordinator = target;
                household.players[index].state = "PLAYING";
                return Ok(vec![]);
            }
            if !is_coordinator {
                return Err(800);
            }
            let player = &mut household.players[index];
            player.transport_uri = uri;
            player.track = if player.queue.is_empty() { 0 } else { 1 };
            player.position_ms = 0;
            player.state = "STOPPED";
            Ok(vec![])
        }
        (Service::AVTransport, "BecomeCoordinatorOfStandaloneGroup") => {
            let player = &mut household.players[index];
            player.coordinator = player.uuid.clone();
            player.state = "STOPPED";
            player.transport_uri.clear();
            Ok(vec![
                ("DelegatedGroupCoordinatorID", String::new()),
                ("NewGroupID", format!("{uuid}:99")),
            ])
        }
        (Service::AVTransport, _) if !is_coordinator && action != "GetTransportInfo" => Err(800),
        (Service::AVTransport, "RemoveAllTracksFromQueue") => {
            let player = &mut household.players[index];
            player.queue.clear();
            player.track = 0;
            Ok(vec![])
        }
        (Service::AVTransport, "AddURIToQueue") => {
            let uri = argument("EnqueuedURI")?;
            let metadata = argument("EnqueuedURIMetaData").unwrap_or_default();
            let player = &mut household.players[index];
            player.queue.push((uri, metadata));
            let length = player.queue.len().to_string();
            Ok(vec![
                ("FirstTrackNumberEnqueued", length.clone()),
                ("NumTracksAdded", "1".to_string()),
                ("NewQueueLength", length),
            ])
        }
        (Service::AVTransport, "RemoveTrackFromQueue") => {
            let object = argument("ObjectID")?;
            let number: usize = object
                .strip_prefix("Q:0/")
                .and_then(|value| value.parse().ok())
                .ok_or(402_u32)?;
            let player = &mut household.players[index];
            if number == 0 || number > player.queue.len() {
                return Err(701);
            }
            player.queue.remove(number - 1);
            if number < player.track as usize {
                player.track -= 1;
            }
            Ok(vec![("NewUpdateID", "1".to_string())])
        }
        (Service::AVTransport, "RemoveTrackRangeFromQueue") => {
            let start: usize = argument("StartingIndex")?.parse().map_err(|_| 402_u32)?;
            let count: usize = argument("NumberOfTracks")?.parse().map_err(|_| 402_u32)?;
            let player = &mut household.players[index];
            if start == 0 || start + count - 1 > player.queue.len() {
                return Err(701);
            }
            player.queue.drain(start - 1..start - 1 + count);
            if (player.track as usize) >= start + count {
                player.track -= count as u32;
            }
            Ok(vec![("NewUpdateID", "1".to_string())])
        }
        (Service::AVTransport, "ReorderTracksInQueue") => {
            let start: usize = argument("StartingIndex")?.parse().map_err(|_| 402_u32)?;
            let count: usize = argument("NumberOfTracks")?.parse().map_err(|_| 402_u32)?;
            let before: usize = argument("InsertBefore")?.parse().map_err(|_| 402_u32)?;
            let player = &mut household.players[index];
            let length = player.queue.len();
            if count != 1 || start == 0 || start > length || before == 0 || before > length + 1 {
                return Err(701);
            }
            let current = player.track as usize;
            let item = player.queue.remove(start - 1);
            let target = if before > start {
                before - 2
            } else {
                before - 1
            };
            player.queue.insert(target, item);
            // The playing track keeps playing wherever it ends up.
            if current == start {
                player.track = target as u32 + 1;
            } else if start < current && target + 1 >= current {
                player.track -= 1;
            } else if start > current && target < current {
                player.track += 1;
            }
            Ok(vec![])
        }
        (Service::AVTransport, "SetPlayMode") => {
            let mode = argument("NewPlayMode")?;
            if ![
                "NORMAL",
                "REPEAT_ALL",
                "REPEAT_ONE",
                "SHUFFLE",
                "SHUFFLE_NOREPEAT",
                "SHUFFLE_REPEAT_ONE",
            ]
            .contains(&mode.as_str())
            {
                return Err(712);
            }
            household.players[index].play_mode = mode;
            Ok(vec![])
        }
        (Service::AVTransport, "SetCrossfadeMode") => {
            household.players[index].crossfade = match argument("CrossfadeMode")?.as_str() {
                "1" | "true" => true,
                "0" | "false" => false,
                _ => return Err(402),
            };
            Ok(vec![])
        }
        (Service::AVTransport, "GetCrossfadeMode") => Ok(vec![(
            "CrossfadeMode",
            if household.players[index].crossfade {
                "1"
            } else {
                "0"
            }
            .to_string(),
        )]),
        (Service::AVTransport, "GetTransportSettings") => Ok(vec![
            ("PlayMode", household.players[index].play_mode.clone()),
            ("RecQualityMode", "NOT_IMPLEMENTED".to_string()),
        ]),
        (Service::AVTransport, "Play") => {
            let player = &mut household.players[index];
            if player.transport_uri.is_empty() {
                return Err(701);
            }
            player.state = "PLAYING";
            Ok(vec![])
        }
        (Service::AVTransport, "Pause") => {
            household.players[index].state = "PAUSED_PLAYBACK";
            Ok(vec![])
        }
        (Service::AVTransport, "Stop") => {
            household.players[index].state = "STOPPED";
            Ok(vec![])
        }
        (Service::AVTransport, "Next") => {
            let player = &mut household.players[index];
            if (player.track as usize) >= player.queue.len() {
                return Err(711);
            }
            player.track += 1;
            player.position_ms = 0;
            Ok(vec![])
        }
        (Service::AVTransport, "Previous") => {
            let player = &mut household.players[index];
            player.track = player.track.saturating_sub(1).max(1);
            player.position_ms = 0;
            Ok(vec![])
        }
        (Service::AVTransport, "Seek") => {
            let unit = argument("Unit")?;
            let target = argument("Target")?;
            let player = &mut household.players[index];
            match unit.as_str() {
                "TRACK_NR" => {
                    let track: u32 = target.parse().map_err(|_| 402_u32)?;
                    if track == 0 || track as usize > player.queue.len() {
                        return Err(711);
                    }
                    player.track = track;
                    player.position_ms = 0;
                }
                "REL_TIME" => {
                    player.position_ms = parse_duration(&target).ok_or(402_u32)?;
                }
                _ => return Err(710),
            }
            Ok(vec![])
        }
        (Service::AVTransport, "GetTransportInfo") => {
            let coordinator = household.effective_coordinator(index);
            let state = household
                .index(&coordinator)
                .map(|coordinator| household.players[coordinator].state)
                .unwrap_or("STOPPED");
            Ok(vec![
                ("CurrentTransportState", state.to_string()),
                ("CurrentTransportStatus", "OK".to_string()),
                ("CurrentSpeed", "1".to_string()),
            ])
        }
        (Service::AVTransport, "GetPositionInfo") => {
            let player = &household.players[index];
            let (uri, metadata) = if player.transport_uri.starts_with("x-rincon-queue:") {
                player
                    .queue
                    .get(player.track.saturating_sub(1) as usize)
                    .cloned()
                    .unwrap_or_default()
            } else {
                (player.transport_uri.clone(), String::new())
            };
            let duration = player
                .reported_duration_ms
                .unwrap_or_else(|| track_duration_ms(&metadata));
            Ok(vec![
                ("Track", player.track.to_string()),
                ("TrackDuration", format_duration(duration)),
                ("TrackMetaData", metadata),
                ("TrackURI", uri),
                ("RelTime", format_duration(player.position_ms)),
                ("AbsTime", "NOT_IMPLEMENTED".to_string()),
            ])
        }
        (Service::RenderingControl, "GetVolume") => Ok(vec![(
            "CurrentVolume",
            household.players[index].volume.to_string(),
        )]),
        (Service::RenderingControl, "SetVolume") => {
            let level: u8 = argument("DesiredVolume")?.parse().map_err(|_| 402_u32)?;
            household.players[index].volume = level.min(100);
            Ok(vec![])
        }
        (Service::RenderingControl, "GetMute") => Ok(vec![(
            "CurrentMute",
            if household.players[index].muted {
                "1"
            } else {
                "0"
            }
            .to_string(),
        )]),
        (Service::RenderingControl, "SetMute") => {
            household.players[index].muted = argument("DesiredMute")? == "1";
            Ok(vec![])
        }
        (Service::GroupRenderingControl, _) if !is_coordinator => Err(701),
        (Service::GroupRenderingControl, "SnapshotGroupVolume") => Ok(vec![]),
        (Service::GroupRenderingControl, "GetGroupVolume") => {
            let members = group_members(household, uuid);
            let total: u32 = members
                .iter()
                .map(|member| u32::from(household.players[*member].volume))
                .sum();
            Ok(vec![(
                "CurrentVolume",
                (total / members.len().max(1) as u32).to_string(),
            )])
        }
        (Service::GroupRenderingControl, "SetGroupVolume") => {
            let level: u8 = argument("DesiredVolume")?.parse().map_err(|_| 402_u32)?;
            for member in group_members(household, uuid) {
                household.players[member].volume = level.min(100);
            }
            Ok(vec![])
        }
        (Service::GroupRenderingControl, "GetGroupMute") => {
            let members = group_members(household, uuid);
            let muted = members
                .iter()
                .all(|member| household.players[*member].muted);
            Ok(vec![(
                "CurrentMute",
                if muted { "1" } else { "0" }.to_string(),
            )])
        }
        (Service::GroupRenderingControl, "SetGroupMute") => {
            let muted = argument("DesiredMute")? == "1";
            for member in group_members(household, uuid) {
                household.players[member].muted = muted;
            }
            Ok(vec![])
        }
        _ => Err(401),
    }
}

fn group_members(household: &Household, coordinator: &str) -> Vec<usize> {
    (0..household.players.len())
        .filter(|index| {
            household.players[*index].pair_of.is_none()
                && household.effective_coordinator(*index) == coordinator
        })
        .collect()
}
