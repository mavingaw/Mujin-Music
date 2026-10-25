//! Zones API: list, register clients, remote control, transfer, grouping,
//! settings, sleep timers, Sonos discovery, signed Sonos media and UPnP
//! event callbacks.

use actix_web::{HttpRequest, HttpResponse, delete, get, patch, post, put, web};
use futures::StreamExt;
use serde::Deserialize;

use crate::api::auth::authenticated_user_id;
use crate::api::cast::{self, CastZoneSnapshot};
use crate::library::state::{LibraryCache, LibraryLifecycle};
use crate::persistence::connection::DbPool;
use crate::zones::service::now_ms;
use crate::zones::{
    ClientRegistration, PlaybackSnapshot, RepeatMode, Zone, ZoneCapabilities, ZoneCommand,
    ZoneError, ZoneEvent, ZoneKind, ZoneNowPlaying, ZoneQueue, ZoneService, ZoneSettings,
    ZoneSource, ZoneStatus, ZoneVolume,
};

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
}

fn error_response(error: ZoneError) -> HttpResponse {
    match error {
        ZoneError::NotFound => {
            HttpResponse::NotFound().json(serde_json::json!({"error": "zone_not_found"}))
        }
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

async fn library_cache(lifecycle: &LibraryLifecycle) -> Option<std::sync::Arc<LibraryCache>> {
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
            None,
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
        },
        sleep_timer: None,
        notes: vec![
            "Chromecast playback is started from the Parson mobile app; Chromecast groups are made in Google Home.".to_string(),
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
        ZoneCommand::MoveQueueItem { .. }
        | ZoneCommand::RemoveQueueItem { .. }
        | ZoneCommand::ClearUpcoming
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

async fn all_zones(
    service: &ZoneService,
    pool: &DbPool,
    user_id: i32,
    cache: Option<&LibraryCache>,
) -> Vec<Zone> {
    let mut zones = service.list(user_id, cache);
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
    let zones = all_zones(&service, pool.get_ref(), user_id, cache.as_deref()).await;
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
            let zone = service.zone(user_id, &form.to, cache.as_deref());
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
    if form.zone_ids.iter().any(|id| id.starts_with("cast:")) {
        return error_response(ZoneError::Unsupported(
            "chromecast_groups_are_made_in_google_home",
        ));
    }
    match service.group(user_id, &form.zone_ids).await {
        Ok(group_id) => {
            let synced = group_id.starts_with("sonos:");
            HttpResponse::Created().json(serde_json::json!({
                "group_id": group_id,
                "synced": synced,
                "sync_note": if synced {
                    crate::zones::SONOS_SYNC_NOTE
                } else {
                    crate::zones::LINKED_SYNC_NOTE
                },
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
    let result = if zone_id.starts_with("cast:") {
        match cast_snapshot(pool.get_ref(), user_id).await {
            Some(snapshot) if format!("cast:{}", snapshot.session_id) == *zone_id => {
                let pool = pool.get_ref().clone();
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
                | ZoneCommand::SetShuffle { .. }
        ) {
            library_cache(&lifecycle).await
        } else {
            None
        };
        service
            .command(user_id, &zone_id, command, cache.as_deref())
            .await
    };
    match result {
        Ok(()) => HttpResponse::Accepted().json(serde_json::json!({"accepted": true})),
        Err(error) => error_response(error),
    }
}

#[patch("/{zone_id}/settings")]
async fn update_settings(
    request: HttpRequest,
    zone_id: web::Path<String>,
    form: web::Json<ZoneSettings>,
    service: web::Data<ZoneService>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    match service
        .update_settings(user_id, &zone_id, form.into_inner())
        .await
    {
        Ok(settings) => HttpResponse::Ok().json(settings),
        Err(error) => error_response(error),
    }
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
    match service.set_sleep_timer(user_id, &zone_id, form.minutes) {
        Ok(()) => HttpResponse::NoContent().finish(),
        Err(error) => error_response(error),
    }
}

/// Signed, token-less media for Sonos players (they cannot send cookies).
pub(crate) async fn zone_media(
    request: HttpRequest,
    song_id: web::Path<String>,
    query: web::Query<ZoneMediaQuery>,
    service: web::Data<ZoneService>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let song_id = song_id.into_inner();
    if query.expires < chrono::Utc::now().timestamp()
        || !cast::verify_media_signature(
            &format!("zone:{}", query.zone),
            &song_id,
            query.expires,
            &query.signature,
        )
    {
        return HttpResponse::Unauthorized().json(serde_json::json!({"error": "zone_url_invalid"}));
    }
    if !service.media_allowed(&query.zone, &song_id) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"error": "zone_queue_changed"}));
    }
    crate::api::library::stream_sonos_compatible(&request, &song_id, &lifecycle).await
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
            .service(send_command)
            .service(update_settings)
            .service(sleep_timer),
    );
}

/// Unauthenticated routes Sonos players call (signed media, UPnP events).
pub fn configure_device_routes(cfg: &mut web::ServiceConfig) {
    cfg.route(
        "/api/v1/zones/media/{song}/stream",
        web::get().to(zone_media),
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
