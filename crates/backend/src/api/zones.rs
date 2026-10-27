//! Zones API: list, register clients, remote control, transfer, grouping,
//! settings, sleep timers, Sonos discovery, signed Sonos media and UPnP
//! event callbacks.

use actix_web::{HttpRequest, HttpResponse, delete, get, patch, post, put, web};
use futures::StreamExt;
use serde::Deserialize;

use crate::api::auth::authenticated_user_id;
use crate::api::cast::{self, CastZoneSnapshot};
use crate::library::state::{LibraryCache, LibraryLifecycle};
use crate::outputs::FormatLimits;
use crate::persistence::connection::DbPool;
use crate::zones::leveling::LevelBy;
use crate::zones::service::now_ms;
use crate::zones::service::{StreamLeveling, zone_media_scope};
use crate::zones::{
    ClientRegistration, PlaybackSnapshot, RepeatMode, Viewer, VolumeLimits, Zone, ZoneCapabilities,
    ZoneCommand, ZoneError, ZoneEvent, ZoneKind, ZoneNowPlaying, ZoneOutputInfo, ZoneQueue,
    ZoneService, ZoneSettingsPatch, ZoneSettingsView, ZoneSource, ZoneStatus, ZoneVolume,
};

/// Header a Parson app sends with its own client zone device id, so the
/// server can show it its private zone (and hide it from everyone else).
pub const ZONE_DEVICE_HEADER: &str = "x-parson-zone-device";
/// Most songs one loudness request covers.
const MAX_LOUDNESS_SONGS: usize = 20;
/// Unmeasured songs one request waits for; the rest measure in the background.
const MAX_LOUDNESS_MEASURED_NOW: usize = 2;

/// Who sent a request: the app's zone device and whether it came from
/// this computer (private audio-device zones only show there).
pub(crate) struct RequestViewer {
    device: Option<String>,
    local: bool,
}

impl RequestViewer {
    pub(crate) fn view(&self) -> Viewer<'_> {
        Viewer {
            device: self.device.as_deref(),
            local: self.local,
        }
    }
}

pub(crate) fn viewer_device(request: &HttpRequest) -> RequestViewer {
    RequestViewer {
        device: request
            .headers()
            .get(ZONE_DEVICE_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| crate::zones::service::client_zone_id(value).is_some())
            .map(str::to_string),
        local: request
            .peer_addr()
            .is_some_and(|address| address.ip().is_loopback()),
    }
}

#[derive(Deserialize)]
struct LoudnessRequest {
    song_ids: Vec<String>,
}

#[derive(Deserialize)]
struct TransferRequest {
    from: String,
    to: String,
}

#[derive(Deserialize)]
struct GroupRequest {
    zone_ids: Vec<String>,
}

#[derive(Deserialize)]
struct UngroupQuery {
    member: Option<String>,
}

#[derive(Deserialize)]
struct SleepRequest {
    /// Minutes until playback pauses; `null` cancels the timer.
    minutes: Option<u32>,
}

#[derive(Deserialize)]
pub(crate) struct ZoneMediaQuery {
    zone: String,
    expires: i64,
    signature: String,
    /// Volume leveling (signed with the URL): `track` or `album`.
    level: Option<String>,
    target: Option<i8>,
    adjust: Option<i8>,
}

impl ZoneMediaQuery {
    fn leveling(&self) -> Result<Option<StreamLeveling>, ()> {
        match (&self.level, self.target, self.adjust) {
            (None, None, None) => Ok(None),
            (Some(level), Some(target_lufs), Some(adjustment_db)) => Ok(Some(StreamLeveling {
                by: LevelBy::parse(level).ok_or(())?,
                target_lufs,
                adjustment_db,
            })),
            _ => Err(()),
        }
    }
}

pub(crate) fn error_response(error: ZoneError) -> HttpResponse {
    match error {
        ZoneError::NotFound => {
            HttpResponse::NotFound().json(serde_json::json!({"error": "zone_not_found"}))
        }
        ZoneError::Missing(code) => HttpResponse::NotFound().json(serde_json::json!({
            "error": code,
        })),
        ZoneError::Invalid(code) => HttpResponse::BadRequest().json(serde_json::json!({
            "error": code,
        })),
        ZoneError::Unsupported(code) => HttpResponse::Conflict().json(serde_json::json!({
            "error": code,
        })),
        ZoneError::Device(message) => HttpResponse::BadGateway().json(serde_json::json!({
            "error": "zone_device_error",
            "message": message,
        })),
    }
}

pub(crate) async fn library_cache(
    lifecycle: &LibraryLifecycle,
) -> Option<std::sync::Arc<LibraryCache>> {
    lifecycle.cache().await.ok()
}

async fn cast_snapshot(pool: &DbPool, user_id: i32) -> Option<CastZoneSnapshot> {
    let pool = pool.clone();
    web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        cast::zone_snapshot(&mut connection, user_id).map_err(|error| error.to_string())
    })
    .await
    .ok()
    .and_then(Result::ok)
    .flatten()
}

fn cast_zone(snapshot: &CastZoneSnapshot, cache: Option<&LibraryCache>) -> Zone {
    let status = match snapshot.status.as_str() {
        "playing" => ZoneStatus::Playing,
        "paused" => ZoneStatus::Paused,
        "connecting" => ZoneStatus::Buffering,
        _ => ZoneStatus::Stopped,
    };
    let now_playing = snapshot
        .song_ids
        .get(snapshot.current_position)
        .and_then(|song_id| {
            let cache = cache?;
            let song = cache.song(song_id)?;
            let album = cache
                .song_map
                .get(song_id)
                .and_then(|(_, album_id)| cache.album(album_id));
            Some(ZoneNowPlaying {
                song_id: Some(song.id.clone()),
                title: song.name.clone(),
                artist: song.artist.clone(),
                album: album.map(|album| album.name.clone()).unwrap_or_default(),
                album_id: album.map(|album| album.id.clone()),
                cover_url: album
                    .map(|album| album.cover_url.clone())
                    .filter(|cover| !cover.trim().is_empty()),
                duration_ms: (song.duration.max(0.0) * 1_000.0).round() as i64,
            })
        });
    Zone {
        id: format!("cast:{}", snapshot.session_id),
        kind: ZoneKind::Chromecast,
        ecosystem: "chromecast",
        name: snapshot.receiver_name.clone(),
        icon: crate::zones::default_zone_icon(ZoneKind::Chromecast).to_string(),
        device: "Chromecast".to_string(),
        status,
        source: ZoneSource::Parson,
        now_playing,
        queue: ZoneQueue {
            song_ids: snapshot.song_ids.clone(),
            current_position: snapshot.current_position,
        },
        repeat: RepeatMode::Off,
        shuffle: false,
        position_ms: snapshot.position_ms,
        duration_ms: snapshot.duration_ms,
        volume: Some(ZoneVolume::new(
            (snapshot.volume.clamp(0.0, 1.0) * 100.0).round() as u8,
            snapshot.muted,
            VolumeLimits::default(),
        )),
        group: None,
        grouped_into: None,
        capabilities: ZoneCapabilities {
            transport: true,
            seek: true,
            volume: true,
            mute: true,
            queue: true,
            queue_edit: false,
            repeat: false,
            shuffle: false,
            group: false,
            transfer_target: false,
            sleep_timer: false,
            standby: false,
        },
        sleep_timer: None,
        settings: ZoneSettingsView::unsupported(&snapshot.receiver_name),
        output: Some(ZoneOutputInfo {
            max_sample_rate: FormatLimits::CHROMECAST.max_sample_rate,
            max_bit_depth: FormatLimits::CHROMECAST.max_bit_depth,
            formats: FormatLimits::CHROMECAST.formats(),
            note: FormatLimits::CHROMECAST.note("Chromecast"),
            exclusive: None,
            volume_mode: None,
            stream: None,
        }),
        endpoints: Vec::new(),
        notes: vec![
            FormatLimits::CHROMECAST.note("Chromecast"),
            "Chromecast playback is started from the Mujin Music mobile app; Chromecast groups are made in Google Home and show up as their own zone.".to_string(),
        ],
    }
}

/// Maps a zone command onto the existing cast command channel.
fn cast_command(
    pool: &DbPool,
    user_id: i32,
    snapshot: &CastZoneSnapshot,
    command: &ZoneCommand,
) -> Result<(), ZoneError> {
    let (name, position_ms, volume, muted, queue_position) = match command {
        ZoneCommand::Play => ("play", None, None, None, None),
        ZoneCommand::Pause => ("pause", None, None, None, None),
        ZoneCommand::PlayPause => (
            if snapshot.playing { "pause" } else { "play" },
            None,
            None,
            None,
            None,
        ),
        ZoneCommand::Stop => ("stop", None, None, None, None),
        ZoneCommand::Next => ("next", None, None, None, None),
        ZoneCommand::Previous => ("previous", None, None, None, None),
        ZoneCommand::Seek { position_ms } => ("seek", Some(*position_ms), None, None, None),
        ZoneCommand::Jump { queue_position } => (
            "jump",
            None,
            None,
            None,
            Some(
                i32::try_from(*queue_position)
                    .map_err(|_| ZoneError::Invalid("invalid_queue_position"))?,
            ),
        ),
        ZoneCommand::SetVolume { volume, .. } => (
            "set_volume",
            None,
            Some(f64::from(*volume) / 100.0),
            None,
            None,
        ),
        ZoneCommand::AdjustVolume { delta, .. } => (
            "set_volume",
            None,
            Some((snapshot.volume + f64::from(*delta) / 100.0).clamp(0.0, 1.0)),
            None,
            None,
        ),
        ZoneCommand::SetMute { muted, .. } => ("set_mute", None, None, Some(*muted), None),
        ZoneCommand::Load { .. } => {
            return Err(ZoneError::Unsupported("zone_cannot_receive_transfer"));
        }
        ZoneCommand::Standby | ZoneCommand::Wake => {
            return Err(ZoneError::Unsupported("zone_has_no_standby"));
        }
        ZoneCommand::MoveQueueItem { .. }
        | ZoneCommand::RemoveQueueItem { .. }
        | ZoneCommand::ClearUpcoming
        | ZoneCommand::AddToQueue { .. }
        | ZoneCommand::SetRepeat { .. }
        | ZoneCommand::SetShuffle { .. } => {
            // The phone that started the cast owns the Chromecast queue.
            return Err(ZoneError::Unsupported("edit_the_cast_queue_on_the_phone"));
        }
    };
    let mut connection = pool
        .get()
        .map_err(|error| ZoneError::Device(error.to_string()))?;
    match cast::zone_command(
        &mut connection,
        user_id,
        &snapshot.session_id,
        name,
        position_ms,
        volume,
        muted,
        queue_position,
    ) {
        Ok(true) => Ok(()),
        Ok(false) => Err(ZoneError::Invalid("invalid_zone_command")),
        Err(error) => Err(ZoneError::Device(error)),
    }
}

pub(crate) async fn all_zones(
    service: &ZoneService,
    pool: &DbPool,
    user_id: i32,
    viewer: Viewer<'_>,
    cache: Option<&LibraryCache>,
) -> Vec<Zone> {
    let mut zones = service.list_for(user_id, viewer, cache);
    if let Some(snapshot) = cast_snapshot(pool, user_id).await {
        zones.push(cast_zone(&snapshot, cache));
    }
    zones
}

#[get("")]
async fn list_zones(
    request: HttpRequest,
    service: web::Data<ZoneService>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    let cache = library_cache(&lifecycle).await;
    let viewer = viewer_device(&request);
    let zones = all_zones(
        &service,
        pool.get_ref(),
        user_id,
        viewer.view(),
        cache.as_deref(),
    )
    .await;
    let sonos = service.sonos().map(|sonos| {
        // Rooms, not devices: stereo pair secondaries and satellites are hidden.
        let rooms: usize = sonos.groups().iter().map(|group| group.members.len()).sum();
        serde_json::json!({
            "enabled": true,
            "players": rooms,
            "last_error": sonos.last_error(),
        })
    });
    HttpResponse::Ok().json(serde_json::json!({
        "zones": zones,
        "server_time_ms": now_ms(),
        "sonos": sonos.unwrap_or_else(|| serde_json::json!({"enabled": false, "players": 0, "last_error": null})),
        "outputs": service
            .outputs()
            .map(|outputs| outputs.status())
            .unwrap_or_else(|| crate::outputs::Outputs::default().status()),
    }))
}

#[get("/events")]
async fn zone_events(
    request: HttpRequest,
    payload: web::Payload,
    service: web::Data<ZoneService>,
) -> Result<HttpResponse, actix_web::Error> {
    let user_id = match authenticated_user_id(&request) {
        Ok(user_id) => user_id,
        Err(response) => return Ok(response),
    };
    let (response, mut session, mut messages) = actix_ws::handle(&request, payload)?;
    let mut events = service.subscribe();
    let mut cast_events = cast::subscribe_cast_changes();
    actix_web::rt::spawn(async move {
        if session.text(r#"{"type":"ready"}"#).await.is_err() {
            return;
        }
        loop {
            let outgoing = tokio::select! {
                event = events.recv() => match event {
                    Ok(ZoneEvent::Changed { user_id: target }) if target.is_none_or(|target| target == user_id) => {
                        Some(r#"{"type":"zones_changed"}"#.to_string())
                    }
                    Ok(ZoneEvent::Command { user_id: target, zone_id, seq, command }) if target == user_id => {
                        Some(serde_json::json!({
                            "type": "zone_command",
                            "zone_id": zone_id,
                            "seq": seq,
                            "command": command,
                        }).to_string())
                    }
                    Ok(ZoneEvent::VolumeRequest { user_id: target, request }) if target.is_none_or(|target| target == user_id) => {
                        // Home Assistant carries these out (an amplifier on
                        // an IR blaster, for example).
                        let mut event = serde_json::json!({"type": "volume_request"});
                        if let (Some(event), serde_json::Value::Object(fields)) =
                            (event.as_object_mut(), serde_json::json!(request))
                        {
                            event.extend(fields);
                        }
                        Some(event.to_string())
                    }
                    Ok(_) => None,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        Some(r#"{"type":"zones_changed"}"#.to_string())
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                event = cast_events.recv() => match event {
                    Ok(changed) if changed == user_id => Some(r#"{"type":"zones_changed"}"#.to_string()),
                    Ok(_) => None,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        Some(r#"{"type":"zones_changed"}"#.to_string())
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                message = messages.next() => match message {
                    Some(Ok(actix_ws::Message::Ping(bytes))) => {
                        if session.pong(&bytes).await.is_err() {
                            break;
                        }
                        None
                    }
                    Some(Ok(actix_ws::Message::Close(reason))) => {
                        let _ = session.close(reason).await;
                        break;
                    }
                    Some(Ok(_)) => None,
                    Some(Err(_)) | None => break,
                }
            };
            if let Some(text) = outgoing
                && session.text(text).await.is_err()
            {
                break;
            }
        }
    });
    Ok(response)
}

#[put("/clients/{device_id}")]
async fn register_client(
    request: HttpRequest,
    device_id: web::Path<String>,
    form: web::Json<ClientRegistration>,
    service: web::Data<ZoneService>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    match service.register_client(user_id, &device_id, form.into_inner()) {
        Ok(result) => HttpResponse::Ok().json(result),
        Err(error) => error_response(error),
    }
}

#[delete("/clients/{device_id}")]
async fn unregister_client(
    request: HttpRequest,
    device_id: web::Path<String>,
    service: web::Data<ZoneService>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    service.unregister_client(user_id, &device_id);
    HttpResponse::NoContent().finish()
}

#[post("/transfer")]
async fn transfer(
    request: HttpRequest,
    form: web::Json<TransferRequest>,
    service: web::Data<ZoneService>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    let viewer = viewer_device(&request);
    if !service.visible_to(&form.from, viewer.view())
        || !service.visible_to(&form.to, viewer.view())
    {
        return error_response(ZoneError::NotFound);
    }
    let cache = library_cache(&lifecycle).await;
    if form.to.starts_with("cast:") {
        return error_response(ZoneError::Unsupported("zone_cannot_receive_transfer"));
    }
    let result = if form.from.starts_with("cast:") {
        match cast_snapshot(pool.get_ref(), user_id).await {
            Some(snapshot) if format!("cast:{}", snapshot.session_id) == form.from => {
                let playback = PlaybackSnapshot {
                    song_ids: snapshot.song_ids.clone(),
                    current_position: snapshot.current_position,
                    position_ms: snapshot.position_ms,
                    playing: snapshot.playing,
                };
                let moved = service
                    .transfer_snapshot(user_id, &form.from, &form.to, playback, cache.as_deref())
                    .await;
                if moved.is_ok() {
                    let pool = pool.get_ref().clone();
                    let session_id = snapshot.session_id.clone();
                    let _ = web::block(move || {
                        let mut connection = pool.get().map_err(|error| error.to_string())?;
                        cast::zone_stop(&mut connection, user_id, &session_id)
                            .map_err(|error| error.to_string())
                    })
                    .await;
                }
                moved
            }
            _ => Err(ZoneError::NotFound),
        }
    } else {
        service
            .transfer(user_id, &form.from, &form.to, cache.as_deref())
            .await
    };
    match result {
        Ok(()) => {
            let zone = service
                .list_for(user_id, viewer.view(), cache.as_deref())
                .into_iter()
                .find(|zone| zone.id == form.to);
            HttpResponse::Ok().json(serde_json::json!({ "zone": zone }))
        }
        Err(error) => error_response(error),
    }
}

#[post("/groups")]
async fn create_group(
    request: HttpRequest,
    form: web::Json<GroupRequest>,
    service: web::Data<ZoneService>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    let viewer = viewer_device(&request);
    if form
        .zone_ids
        .iter()
        .any(|id| !service.visible_to(id, viewer.view()))
    {
        return error_response(ZoneError::NotFound);
    }
    if form.zone_ids.iter().any(|id| id.starts_with("cast:")) {
        return error_response(ZoneError::Unsupported(
            "chromecast_groups_are_made_in_google_home",
        ));
    }
    match service.group(user_id, &form.zone_ids).await {
        Ok(group_id) => {
            let (synced, sync_note) = if group_id.starts_with("sonos:") {
                (true, crate::zones::SONOS_SYNC_NOTE)
            } else if group_id.starts_with(crate::outputs::bridge::GROUP_PREFIX) {
                (true, crate::outputs::bridge::GROUP_SYNC_NOTE)
            } else {
                (false, crate::zones::LINKED_SYNC_NOTE)
            };
            HttpResponse::Created().json(serde_json::json!({
                "group_id": group_id,
                "synced": synced,
                "sync_note": sync_note,
            }))
        }
        Err(error) => error_response(error),
    }
}

#[delete("/groups/{group_id}")]
async fn delete_group(
    request: HttpRequest,
    group_id: web::Path<String>,
    query: web::Query<UngroupQuery>,
    service: web::Data<ZoneService>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    match service
        .ungroup(user_id, &group_id, query.member.as_deref())
        .await
    {
        Ok(()) => HttpResponse::NoContent().finish(),
        Err(error) => error_response(error),
    }
}

#[get("/sonos")]
async fn sonos_status(request: HttpRequest, service: web::Data<ZoneService>) -> HttpResponse {
    if let Err(response) = authenticated_user_id(&request) {
        return response;
    }
    match service.sonos() {
        Some(sonos) => HttpResponse::Ok().json(serde_json::json!({
            "enabled": true,
            "players": sonos.players(),
            "groups": sonos.groups().iter().map(|group| serde_json::json!({
                "coordinator": group.coordinator,
                "name": group.name,
                "members": group.members,
            })).collect::<Vec<_>>(),
            "last_error": sonos.last_error(),
        })),
        None => HttpResponse::Ok().json(serde_json::json!({
            "enabled": false,
            "players": [],
            "groups": [],
            "last_error": null,
        })),
    }
}

#[post("/sonos/discover")]
async fn sonos_discover(request: HttpRequest, service: web::Data<ZoneService>) -> HttpResponse {
    if let Err(response) = authenticated_user_id(&request) {
        return response;
    }
    let Some(sonos) = service.sonos().cloned() else {
        return error_response(ZoneError::Unsupported("sonos_disabled"));
    };
    let discovered = sonos.discover().await;
    sonos.ensure_subscriptions(service.port()).await;
    sonos.poll_all().await;
    match discovered {
        Ok(players) => HttpResponse::Ok().json(serde_json::json!({ "players": players })),
        Err(error) => HttpResponse::Ok().json(serde_json::json!({
            "players": sonos.players().len(),
            "last_error": error,
        })),
    }
}

#[derive(Deserialize)]
struct BridgeRequest {
    address: String,
}

fn is_admin(request: &HttpRequest) -> bool {
    use actix_web::HttpMessage;
    request
        .extensions()
        .get::<crate::api::auth::Claims>()
        .is_some_and(|claims| claims.role == "admin")
}

fn bridges_of(service: &ZoneService) -> Option<std::sync::Arc<crate::outputs::bridge::BridgeHub>> {
    service
        .outputs()
        .and_then(|outputs| outputs.bridges.clone())
}

/// Parson Bridges the server knows (found on the network or added).
#[get("/bridges")]
async fn bridge_status(request: HttpRequest, service: web::Data<ZoneService>) -> HttpResponse {
    if let Err(response) = authenticated_user_id(&request) {
        return response;
    }
    match bridges_of(&service) {
        Some(hub) => HttpResponse::Ok().json(serde_json::json!({
            "enabled": true,
            "bridges": hub.status(),
        })),
        None => HttpResponse::Ok().json(serde_json::json!({"enabled": false, "bridges": []})),
    }
}

/// Adds a bridge by address (when the network blocks discovery). Admins
/// only: the server connects to whatever address is given.
#[post("/bridges")]
async fn add_bridge(
    request: HttpRequest,
    form: web::Json<BridgeRequest>,
    service: web::Data<ZoneService>,
) -> HttpResponse {
    if let Err(response) = authenticated_user_id(&request) {
        return response;
    }
    if !is_admin(&request) {
        return HttpResponse::Forbidden().json(serde_json::json!({"error": "admin_required"}));
    }
    let Some(hub) = bridges_of(&service) else {
        return error_response(ZoneError::Unsupported("bridges_disabled"));
    };
    let address = form.into_inner().address;
    match web::block(move || hub.add(&address)).await {
        Ok(Ok(status)) => {
            if let Some(outputs) = service.outputs() {
                for renderer in outputs.renderers() {
                    renderer.poll().await;
                }
            }
            HttpResponse::Ok().json(status)
        }
        Ok(Err(error)) => HttpResponse::BadRequest().json(serde_json::json!({"error": error})),
        Err(_) => HttpResponse::InternalServerError().finish(),
    }
}

#[delete("/bridges/{address}")]
async fn remove_bridge(
    request: HttpRequest,
    address: web::Path<String>,
    service: web::Data<ZoneService>,
) -> HttpResponse {
    if let Err(response) = authenticated_user_id(&request) {
        return response;
    }
    if !is_admin(&request) {
        return HttpResponse::Forbidden().json(serde_json::json!({"error": "admin_required"}));
    }
    let Some(hub) = bridges_of(&service) else {
        return error_response(ZoneError::Unsupported("bridges_disabled"));
    };
    if hub.remove(&address) {
        HttpResponse::NoContent().finish()
    } else {
        error_response(ZoneError::NotFound)
    }
}

fn network_players_of(
    service: &ZoneService,
) -> Option<std::sync::Arc<crate::outputs::network::NetworkPlayers>> {
    service
        .outputs()
        .and_then(|outputs| outputs.network.clone())
}

/// UPnP, OpenHome and AirPlay players added by address, and whether the
/// server searches the network for them.
#[get("/outputs/network")]
async fn network_players(request: HttpRequest, service: web::Data<ZoneService>) -> HttpResponse {
    if let Err(response) = authenticated_user_id(&request) {
        return response;
    }
    match network_players_of(&service) {
        Some(players) => HttpResponse::Ok().json(players.view()),
        None => HttpResponse::Ok().json(serde_json::json!({"discovery": false, "manual": []})),
    }
}

/// Adds (or renames) a network player by address. Admins only: the server
/// connects to whatever address is given.
#[post("/outputs/network")]
async fn add_network_player(
    request: HttpRequest,
    form: web::Json<parson_output::network::ManualEndpoint>,
    service: web::Data<ZoneService>,
) -> HttpResponse {
    if let Err(response) = authenticated_user_id(&request) {
        return response;
    }
    if !is_admin(&request) {
        return HttpResponse::Forbidden().json(serde_json::json!({"error": "admin_required"}));
    }
    let Some(players) = network_players_of(&service) else {
        return error_response(ZoneError::Unsupported("network_players_disabled"));
    };
    let endpoint = form.into_inner();
    if let Err(message) = players.add(endpoint) {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({"error": "invalid_network_player", "message": message}));
    }
    if let Some(outputs) = service.outputs() {
        outputs.network_changed().await;
    }
    HttpResponse::Ok().json(players.view())
}

#[derive(Deserialize)]
struct RemoveNetworkPlayer {
    address: String,
}

#[delete("/outputs/network")]
async fn remove_network_player(
    request: HttpRequest,
    form: web::Json<RemoveNetworkPlayer>,
    service: web::Data<ZoneService>,
) -> HttpResponse {
    if let Err(response) = authenticated_user_id(&request) {
        return response;
    }
    if !is_admin(&request) {
        return HttpResponse::Forbidden().json(serde_json::json!({"error": "admin_required"}));
    }
    let Some(players) = network_players_of(&service) else {
        return error_response(ZoneError::Unsupported("network_players_disabled"));
    };
    if let Err(message) = players.remove(&form.address) {
        return HttpResponse::NotFound()
            .json(serde_json::json!({"error": "network_player_not_found", "message": message}));
    }
    if let Some(outputs) = service.outputs() {
        outputs.network_changed().await;
    }
    HttpResponse::Ok().json(players.view())
}

/// A network player's own picture (its UPnP icon), fetched from it.
#[get("/{zone_id}/icon")]
async fn zone_icon(
    request: HttpRequest,
    zone_id: web::Path<String>,
    service: web::Data<ZoneService>,
) -> HttpResponse {
    if let Err(response) = authenticated_user_id(&request) {
        return response;
    }
    let icon = match service.outputs().and_then(|outputs| outputs.upnp.clone()) {
        Some(upnp) => upnp.icon(&zone_id).await,
        None => None,
    };
    match icon {
        Some((content_type, bytes)) => HttpResponse::Ok()
            .content_type(content_type)
            .insert_header(("Cache-Control", "private, max-age=3600"))
            .body(bytes),
        None => HttpResponse::NotFound().json(serde_json::json!({"error": "zone_icon_not_found"})),
    }
}

/// Roon's "Pause all": pauses every playing zone this viewer can see.
#[post("/pause-all")]
async fn pause_all(
    request: HttpRequest,
    service: web::Data<ZoneService>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    let viewer = viewer_device(&request);
    let cache = library_cache(&lifecycle).await;
    let mut paused = service
        .pause_all(user_id, viewer.view(), cache.as_deref())
        .await;
    if let Some(snapshot) = cast_snapshot(pool.get_ref(), user_id).await
        && snapshot.playing
    {
        let pool = pool.get_ref().clone();
        let paused_cast =
            web::block(move || cast_command(&pool, user_id, &snapshot, &ZoneCommand::Pause))
                .await
                .is_ok_and(|result| result.is_ok());
        if paused_cast {
            paused += 1;
        }
    }
    HttpResponse::Ok().json(serde_json::json!({ "paused": paused }))
}

/// Looks for UPnP renderers and audio devices now.
#[post("/outputs/discover")]
async fn outputs_discover(request: HttpRequest, service: web::Data<ZoneService>) -> HttpResponse {
    if let Err(response) = authenticated_user_id(&request) {
        return response;
    }
    let Some(outputs) = service.outputs().cloned() else {
        return HttpResponse::Ok().json(crate::outputs::Outputs::default().status());
    };
    outputs.discover().await;
    for renderer in outputs.renderers() {
        renderer.poll().await;
    }
    HttpResponse::Ok().json(outputs.status())
}

/// Sends one command to a zone: cast sessions through their command
/// channel, every other zone through the zone service.
pub(crate) async fn dispatch_command(
    user_id: i32,
    zone_id: &str,
    command: ZoneCommand,
    service: &ZoneService,
    pool: &DbPool,
    lifecycle: &LibraryLifecycle,
) -> Result<(), ZoneError> {
    if zone_id.starts_with("cast:") {
        match cast_snapshot(pool, user_id).await {
            Some(snapshot) if format!("cast:{}", snapshot.session_id) == zone_id => {
                let pool = pool.clone();
                web::block(move || cast_command(&pool, user_id, &snapshot, &command))
                    .await
                    .unwrap_or_else(|error| Err(ZoneError::Device(error.to_string())))
            }
            _ => Err(ZoneError::NotFound),
        }
    } else {
        let cache = if matches!(
            command,
            ZoneCommand::Load { .. }
                | ZoneCommand::Jump { .. }
                | ZoneCommand::Next
                | ZoneCommand::Previous
                | ZoneCommand::MoveQueueItem { .. }
                | ZoneCommand::RemoveQueueItem { .. }
                | ZoneCommand::ClearUpcoming
                | ZoneCommand::AddToQueue { .. }
                | ZoneCommand::SetShuffle { .. }
        ) {
            library_cache(lifecycle).await
        } else {
            None
        };
        service
            .command(user_id, zone_id, command, cache.as_deref())
            .await
    }
}

#[post("/{zone_id}/commands")]
async fn send_command(
    request: HttpRequest,
    zone_id: web::Path<String>,
    form: web::Json<ZoneCommand>,
    service: web::Data<ZoneService>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    let command = form.into_inner();
    if let Err(code) = command.validate() {
        return error_response(ZoneError::Invalid(code));
    }
    if !service.visible_to(&zone_id, viewer_device(&request).view()) {
        return error_response(ZoneError::NotFound);
    }
    let result = dispatch_command(
        user_id,
        &zone_id,
        command,
        &service,
        pool.get_ref(),
        &lifecycle,
    )
    .await;
    match result {
        Ok(()) => HttpResponse::Accepted().json(serde_json::json!({"accepted": true})),
        Err(error) => error_response(error),
    }
}

#[patch("/{zone_id}/settings")]
async fn update_settings(
    request: HttpRequest,
    zone_id: web::Path<String>,
    form: web::Json<ZoneSettingsPatch>,
    service: web::Data<ZoneService>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    let viewer = viewer_device(&request);
    let patch = form.into_inner();
    let cache = if patch.leveling.is_some()
        || patch.leveling_target_lufs.is_some()
        || patch.leveling_adjustment_db.is_some()
    {
        library_cache(&lifecycle).await
    } else {
        None
    };
    match service
        .update_settings(user_id, &zone_id, patch, viewer.view(), cache.as_deref())
        .await
    {
        Ok(settings) => {
            let zone = service
                .list_for(user_id, viewer.view(), None)
                .into_iter()
                .find(|zone| zone.id == *zone_id);
            HttpResponse::Ok().json(serde_json::json!({
                "settings": settings,
                "zone": zone,
            }))
        }
        Err(error) => error_response(error),
    }
}

/// Measured loudness for volume leveling in Parson apps. Up to two
/// unmeasured songs are measured now; the rest are measured in the
/// background and come back with `null` until then.
#[post("/loudness")]
async fn loudness(
    request: HttpRequest,
    form: web::Json<LoudnessRequest>,
    service: web::Data<ZoneService>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    if let Err(response) = authenticated_user_id(&request) {
        return response;
    }
    if form.song_ids.is_empty() || form.song_ids.len() > MAX_LOUDNESS_SONGS {
        return error_response(ZoneError::Invalid("invalid_song_count"));
    }
    let Some(cache) = library_cache(&lifecycle).await else {
        return error_response(ZoneError::Unsupported("library_unavailable"));
    };
    let store = service.loudness().clone();
    let mut measured_now = 0;
    let mut tracks = serde_json::Map::new();
    let mut later = Vec::new();
    for song_id in &form.song_ids {
        let Some(song) = cache.song(song_id) else {
            continue;
        };
        let path = std::path::Path::new(&song.path);
        let leveling =
            if store.cached(song_id, path).is_some() || measured_now < MAX_LOUDNESS_MEASURED_NOW {
                if store.cached(song_id, path).is_none() {
                    measured_now += 1;
                }
                Some(store.leveling_for(&cache, song_id).await)
            } else {
                later.push((song_id.clone(), song.path.clone()));
                None
            };
        tracks.insert(
            song_id.clone(),
            serde_json::to_value(leveling).unwrap_or(serde_json::Value::Null),
        );
    }
    let pending = !later.is_empty()
        || tracks
            .values()
            .any(|value| value.is_null() || value["album"].is_null());
    store.measure_in_background(later);
    HttpResponse::Ok().json(serde_json::json!({
        "tracks": tracks,
        "pending": pending,
    }))
}

#[post("/{zone_id}/sleep")]
async fn sleep_timer(
    request: HttpRequest,
    zone_id: web::Path<String>,
    form: web::Json<SleepRequest>,
    service: web::Data<ZoneService>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    if !service.visible_to(&zone_id, viewer_device(&request).view()) {
        return error_response(ZoneError::NotFound);
    }
    match service.set_sleep_timer(user_id, &zone_id, form.minutes) {
        Ok(()) => HttpResponse::NoContent().finish(),
        Err(error) => error_response(error),
    }
}

/// Signed, token-less media for Sonos players (they cannot send cookies).
/// Leveled URLs carry their (signed) leveling; the server applies the gain.
pub(crate) async fn zone_media(
    request: HttpRequest,
    song_id: web::Path<String>,
    query: web::Query<ZoneMediaQuery>,
    service: web::Data<ZoneService>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let song_id = song_id.into_inner();
    let invalid =
        || HttpResponse::Unauthorized().json(serde_json::json!({"error": "zone_url_invalid"}));
    let Ok(leveling) = query.leveling() else {
        return invalid();
    };
    if query.expires < chrono::Utc::now().timestamp()
        || !cast::verify_media_signature(
            &zone_media_scope(&query.zone, leveling),
            &song_id,
            query.expires,
            &query.signature,
        )
    {
        return invalid();
    }
    if !service.media_allowed(&query.zone, &song_id) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"error": "zone_queue_changed"}));
    }
    let gain = match leveling {
        Some(leveling) => match library_cache(&lifecycle).await {
            Some(cache) => Some(
                service
                    .stream_gain(&cache, &song_id, leveling)
                    .await
                    .unwrap_or(0.0),
            ),
            None => Some(0.0),
        },
        None => None,
    };
    crate::api::library::stream_sonos_compatible(&request, &song_id, &lifecycle, gain).await
}

#[derive(Deserialize)]
pub(crate) struct RenderMediaQuery {
    zone: String,
    expires: i64,
    signature: String,
}

/// Signed media for UPnP renderers: the zone's format limits, leveling and
/// DSP decide whether the original file or a conversion is served.
pub(crate) async fn render_media(
    request: HttpRequest,
    song_id: web::Path<String>,
    query: web::Query<RenderMediaQuery>,
    service: web::Data<ZoneService>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let song_id = song_id.into_inner();
    if query.expires < chrono::Utc::now().timestamp()
        || !cast::verify_media_signature(
            &crate::zones::service::render_scope(&query.zone),
            &song_id,
            query.expires,
            &query.signature,
        )
    {
        return HttpResponse::Unauthorized().json(serde_json::json!({"error": "zone_url_invalid"}));
    }
    let Some(cache) = library_cache(&lifecycle).await else {
        return HttpResponse::ServiceUnavailable()
            .json(serde_json::json!({"error": "library_unavailable"}));
    };
    let Some(stream) = service.hosted_stream(&query.zone, &song_id, &cache) else {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"error": "zone_queue_changed"}));
    };
    let gain = match stream.leveling {
        Some(leveling) => Some(
            service
                .stream_gain(&cache, &song_id, leveling)
                .await
                .unwrap_or(0.0),
        ),
        None => None,
    };
    crate::api::library::stream_renderer_compatible(
        &request,
        &song_id,
        &lifecycle,
        stream.limits,
        gain,
        stream.dsp,
    )
    .await
}

/// UPnP GENA `NOTIFY` from a Sonos player: refresh that player's group now.
pub(crate) async fn sonos_notify(
    uuid: web::Path<String>,
    service: web::Data<ZoneService>,
) -> HttpResponse {
    let uuid = uuid.into_inner();
    if !crate::sonos::description::valid_player_uuid(&uuid) {
        return HttpResponse::NotFound().finish();
    }
    let service = service.into_inner();
    actix_web::rt::spawn(async move { service.sonos_event(&uuid).await });
    HttpResponse::Ok().finish()
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/zones")
            .service(list_zones)
            .service(zone_events)
            .service(register_client)
            .service(unregister_client)
            .service(transfer)
            .service(create_group)
            .service(delete_group)
            .service(sonos_status)
            .service(sonos_discover)
            .service(pause_all)
            .service(outputs_discover)
            .service(network_players)
            .service(add_network_player)
            .service(remove_network_player)
            .service(bridge_status)
            .service(add_bridge)
            .service(remove_bridge)
            .service(crate::api::control::play_item)
            .service(crate::api::control::start_radio)
            .service(crate::api::control::rate_track)
            .service(send_command)
            .service(update_settings)
            .service(loudness)
            .service(sleep_timer)
            .service(zone_icon),
    );
}

/// Unauthenticated routes Sonos players call (signed media, UPnP events).
pub fn configure_device_routes(cfg: &mut web::ServiceConfig) {
    cfg.route(
        "/api/v1/zones/media/{song}/stream",
        web::get().to(zone_media),
    )
    .route(
        "/api/v1/zones/render/{song}/stream",
        web::get().to(render_media),
    )
    .route(
        "/api/v1/zones/render/{song}/stream",
        web::head().to(render_media),
    )
    .route(
        "/api/v1/zones/media/{song}/stream",
        web::head().to(zone_media),
    )
    .route(
        "/api/v1/zones/sonos/notify/{uuid}",
        web::method(actix_web::http::Method::from_bytes(b"NOTIFY").expect("NOTIFY method"))
            .to(sonos_notify),
    );
}
