//! Displays over HTTP.
//!
//! Signed-in apps manage them under `/displays`:
//! - `GET    /displays`            the listener's displays
//! - `POST   /displays`            a display opened from a link (returns its key once)
//! - `POST   /displays/approve`    approve the six-digit code a screen shows
//! - `PATCH  /displays/{id}`       rename, bind to a zone or follow, lyrics on/off
//! - `DELETE /displays/{id}`       remove; the key stops working at once
//!
//! The screens themselves use `/display` with no session, only their key in
//! the `X-Parson-Display` header:
//! - `POST /display/pair`, `POST /display/pair/status`  pairing by code
//! - `GET  /display/state`                 the zone, the track and what is next
//! - `GET  /display/lyrics/{song}`         lyrics of the track it shows
//! - `GET  /display/waveform/{song}`       waveform of the track it shows
//! - `GET  /display/artist-image/{id}`     the artist photo (signed URL)
//!
//! A key reads one zone's now-playing state; it cannot browse the library,
//! control playback or see other zones' queues.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use actix_web::{HttpRequest, HttpResponse, delete, get, patch, post, web};
use serde::{Deserialize, Serialize};

use crate::api::auth::{authenticated_user_id, record_auth_attempt};
use crate::api::editorial::ArtistPortrait;
use crate::api::error::{bad_request, internal_server_error, not_found};
use crate::api::lyrics::LyricsService;
use crate::displays::{
    self, DISPLAY_KEY_HEADER, DisplayError, DisplayPatch, DisplayRow, DisplayView, LyricsMode,
    PairingError, PairingStatus, TrackFormat, ZoneChoice,
};
use crate::library::state::{LibraryCache, LibraryLifecycle};
use crate::library::tools::{analysis, store as tools_store};
use crate::persistence::connection::DbPool;
use crate::zones::{RepeatMode, StreamQuality, Viewer, Zone, ZoneKind, ZoneService, ZoneStatus};

const PAIRING_ATTEMPTS_PER_MINUTE: usize = 12;
const APPROVALS_PER_MINUTE: usize = 10;
/// Signed image URLs stay the same for an hour so the screen's browser
/// keeps them cached, and expire well within the image route's limit.
const IMAGE_URL_BUCKET_SECONDS: i64 = 60 * 60;
const UP_NEXT: usize = 3;
const BIOGRAPHY_CHARS: usize = 420;
const PORTRAIT_TTL: Duration = Duration::from_secs(10 * 60);
const ARTIST_IMAGE_SCOPE: &str = "display-artist-image";

async fn db<T: Send + 'static>(
    pool: &DbPool,
    run: impl FnOnce(&mut diesel::SqliteConnection) -> diesel::QueryResult<T> + Send + 'static,
) -> Result<T, HttpResponse> {
    let pool = pool.clone();
    let result = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        run(&mut connection).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|result| result);
    result.map_err(|error| {
        tracing::error!(%error, "display storage failed");
        internal_server_error(
            "Could not reach the displays list.",
            "display_storage_failed",
        )
    })
}

async fn zones_for(
    service: &ZoneService,
    pool: &DbPool,
    lifecycle: &LibraryLifecycle,
    user_id: i32,
) -> (Vec<Zone>, Option<Arc<LibraryCache>>) {
    let cache = crate::api::zones::library_cache(lifecycle).await;
    // A display is no zone's own device, so private zones stay private.
    let zones =
        crate::api::zones::all_zones(service, pool, user_id, Viewer::default(), cache.as_deref())
            .await;
    (zones, cache)
}

fn views(rows: &[DisplayRow]) -> Vec<DisplayView> {
    let now = displays::now_utc();
    rows.iter().map(|row| row.view(now)).collect()
}

// ---------------------------------------------------------------------------
// Signed-in management.

#[derive(Deserialize)]
struct CreateDisplay {
    name: String,
    #[serde(default)]
    zone_id: Option<String>,
    #[serde(default)]
    lyrics: Option<LyricsMode>,
}

#[derive(Deserialize)]
struct ApproveDisplay {
    code: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    zone_id: Option<String>,
    #[serde(default)]
    lyrics: Option<LyricsMode>,
}

/// Checks that a zone a display is bound to is one the listener can see.
async fn check_zone(
    zone_id: Option<&str>,
    service: &ZoneService,
    pool: &DbPool,
    lifecycle: &LibraryLifecycle,
    user_id: i32,
) -> Result<(), HttpResponse> {
    let Some(zone_id) = zone_id else {
        return Ok(());
    };
    if !displays::valid_zone_id(zone_id) {
        return Err(bad_request(
            "Choose a zone for this display.",
            "invalid_zone",
        ));
    }
    let (zones, _) = zones_for(service, pool, lifecycle, user_id).await;
    if zones.iter().any(|zone| zone.id == zone_id) {
        Ok(())
    } else {
        Err(not_found(
            "That zone is not available. Choose one from the list.",
            "zone_not_found",
        ))
    }
}

fn too_many() -> HttpResponse {
    bad_request(
        "You have 32 displays. Remove one you no longer use first.",
        "too_many_displays",
    )
}

#[get("")]
async fn list(request: HttpRequest, pool: web::Data<DbPool>) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    match db(&pool, move |connection| {
        displays::list_displays(connection, user_id)
    })
    .await
    {
        Ok(rows) => HttpResponse::Ok().json(serde_json::json!({ "displays": views(&rows) })),
        Err(response) => response,
    }
}

#[post("")]
async fn create(
    request: HttpRequest,
    body: web::Json<CreateDisplay>,
    pool: web::Data<DbPool>,
    zones: web::Data<ZoneService>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    let body = body.into_inner();
    let Some(name) = displays::clean_name(&body.name) else {
        return bad_request("Give the display a name.", "invalid_display_name");
    };
    if let Err(response) =
        check_zone(body.zone_id.as_deref(), &zones, &pool, &lifecycle, user_id).await
    {
        return response;
    }
    let lyrics = body.lyrics.unwrap_or_default();
    let created = {
        let pool = pool.get_ref().clone();
        web::block(move || {
            let mut connection = pool.get().map_err(|error| error.to_string())?;
            match displays::create_display(
                &mut connection,
                user_id,
                &name,
                body.zone_id.as_deref(),
                lyrics,
            ) {
                Ok(created) => Ok(Ok(created)),
                Err(DisplayError::TooMany) => Ok(Err(())),
                Err(DisplayError::Database(error)) => Err(error.to_string()),
            }
        })
        .await
    };
    match created {
        Ok(Ok(Ok((row, key)))) => HttpResponse::Created().json(serde_json::json!({
            "display": row.view(displays::now_utc()),
            "key": key,
        })),
        Ok(Ok(Err(()))) => too_many(),
        Ok(Err(error)) => {
            tracing::error!(%error, "could not create a display");
            internal_server_error("Could not create the display.", "display_storage_failed")
        }
        Err(error) => {
            tracing::error!(%error, "could not create a display");
            internal_server_error("Could not create the display.", "display_storage_failed")
        }
    }
}

#[post("/approve")]
async fn approve(
    request: HttpRequest,
    body: web::Json<ApproveDisplay>,
    pool: web::Data<DbPool>,
    zones: web::Data<ZoneService>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    if let Err(response) = record_auth_attempt(&request, "display:approve", APPROVALS_PER_MINUTE) {
        return response;
    }
    let body = body.into_inner();
    let Some(code) = displays::normalized_code(&body.code) else {
        return bad_request(
            "Enter the six-digit code the screen shows.",
            "invalid_pairing_code",
        );
    };
    if let Err(response) =
        check_zone(body.zone_id.as_deref(), &zones, &pool, &lifecycle, user_id).await
    {
        return response;
    }
    let Some((pairing_id, suggested)) = displays::claim_code(&code, Instant::now()) else {
        return not_found(
            "No screen is showing that code. Codes last ten minutes; check the screen.",
            "pairing_code_not_found",
        );
    };
    let name = body
        .name
        .as_deref()
        .and_then(displays::clean_name)
        .unwrap_or(suggested);
    let lyrics = body.lyrics.unwrap_or_default();
    let zone_id = body.zone_id.clone();
    let created = {
        let pool = pool.get_ref().clone();
        web::block(move || {
            let mut connection = pool.get().map_err(|error| error.to_string())?;
            match displays::create_display(
                &mut connection,
                user_id,
                &name,
                zone_id.as_deref(),
                lyrics,
            ) {
                Ok(created) => Ok(Ok(created)),
                Err(DisplayError::TooMany) => Ok(Err(())),
                Err(DisplayError::Database(error)) => Err(error.to_string()),
            }
        })
        .await
    };
    match created {
        Ok(Ok(Ok((row, key)))) => {
            let view = row.view(displays::now_utc());
            if !displays::complete_pairing(&pairing_id, view.clone(), key) {
                // The screen gave up while we stored it; do not leave a
                // display behind that nobody holds the key of.
                let id = row.id.clone();
                let _ = db(&pool, move |connection| {
                    displays::delete_display(connection, user_id, &id)
                })
                .await;
                return not_found(
                    "The screen stopped waiting. Start pairing on it again.",
                    "pairing_code_not_found",
                );
            }
            HttpResponse::Created().json(serde_json::json!({ "display": view }))
        }
        Ok(Ok(Err(()))) => {
            displays::release_code(&pairing_id);
            too_many()
        }
        Ok(Err(error)) => {
            displays::release_code(&pairing_id);
            tracing::error!(%error, "could not approve a display");
            internal_server_error("Could not add the display.", "display_storage_failed")
        }
        Err(error) => {
            displays::release_code(&pairing_id);
            tracing::error!(%error, "could not approve a display");
            internal_server_error("Could not add the display.", "display_storage_failed")
        }
    }
}

#[patch("/{id}")]
async fn update(
    request: HttpRequest,
    id: web::Path<String>,
    body: web::Json<DisplayPatch>,
    pool: web::Data<DbPool>,
    zones: web::Data<ZoneService>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    let mut patch = body.into_inner();
    if let Some(name) = &patch.name {
        match displays::clean_name(name) {
            Some(name) => patch.name = Some(name),
            None => return bad_request("Give the display a name.", "invalid_display_name"),
        }
    }
    if let Some(Some(zone_id)) = &patch.zone_id
        && let Err(response) = check_zone(Some(zone_id), &zones, &pool, &lifecycle, user_id).await
    {
        return response;
    }
    let id = id.into_inner();
    match db(&pool, move |connection| {
        displays::update_display(connection, user_id, &id, &patch)
    })
    .await
    {
        Ok(Some(row)) => {
            displays::remember_followed(&row.id, None);
            HttpResponse::Ok().json(serde_json::json!({
                "display": row.view(displays::now_utc())
            }))
        }
        Ok(None) => not_found("That display was removed.", "display_not_found"),
        Err(response) => response,
    }
}

#[delete("/{id}")]
async fn remove(
    request: HttpRequest,
    id: web::Path<String>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    let id = id.into_inner();
    let forget = id.clone();
    match db(&pool, move |connection| {
        displays::delete_display(connection, user_id, &id)
    })
    .await
    {
        Ok(true) => {
            displays::remember_followed(&forget, None);
            HttpResponse::NoContent().finish()
        }
        Ok(false) => not_found("That display was removed.", "display_not_found"),
        Err(response) => response,
    }
}

/// Signed-in routes, under the music API's session check.
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/displays")
            .service(list)
            .service(create)
            .service(approve)
            .service(update)
            .service(remove),
    );
}

// ---------------------------------------------------------------------------
// The screen's own routes: no session, only the display key.

#[derive(Deserialize)]
struct PairStart {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct PairPoll {
    pairing_id: String,
    secret: String,
}

fn no_store(mut response: HttpResponse) -> HttpResponse {
    response.headers_mut().insert(
        actix_web::http::header::CACHE_CONTROL,
        actix_web::http::header::HeaderValue::from_static("no-store"),
    );
    response
}

async fn pair_start(request: HttpRequest, body: web::Json<PairStart>) -> HttpResponse {
    if let Err(response) =
        record_auth_attempt(&request, "display:pair", PAIRING_ATTEMPTS_PER_MINUTE)
    {
        return response;
    }
    let name = body
        .name
        .as_deref()
        .and_then(displays::clean_name)
        .unwrap_or_else(|| "Display".to_string());
    match displays::start_pairing(&name, Instant::now()) {
        Ok(started) => no_store(HttpResponse::Ok().json(started)),
        Err(_) => HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "error": "too_many_pairings",
            "message": "Too many screens are waiting to pair. Try again in a minute."
        })),
    }
}

async fn pair_status(body: web::Json<PairPoll>) -> HttpResponse {
    match displays::pairing_status(&body.pairing_id, &body.secret, Instant::now()) {
        Ok(PairingStatus::Pending { name }) => no_store(
            HttpResponse::Accepted().json(serde_json::json!({"status": "pending", "name": name})),
        ),
        Ok(PairingStatus::Approved { display, key }) => no_store(
            HttpResponse::Ok()
                .json(serde_json::json!({"status": "approved", "display": display, "key": key})),
        ),
        Err(PairingError::NotFound | PairingError::Busy) => {
            HttpResponse::NotFound().json(serde_json::json!({
                "status": "expired",
                "error": "pairing_expired",
                "message": "This code expired. A new one is on the way."
            }))
        }
    }
}

fn display_key(request: &HttpRequest) -> Option<String> {
    request
        .headers()
        .get(DISPLAY_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn unpaired() -> HttpResponse {
    HttpResponse::Forbidden().json(serde_json::json!({
        "error": "display_not_paired",
        "message": "This screen is no longer paired. Pair it again from Settings → Displays."
    }))
}

/// The display a request's key belongs to.
async fn authorized_display(
    request: &HttpRequest,
    pool: &DbPool,
) -> Result<DisplayRow, HttpResponse> {
    let Some(key) = display_key(request) else {
        return Err(unpaired());
    };
    match db(pool, move |connection| {
        displays::display_for_key(connection, &key)
    })
    .await?
    {
        Some(row) => Ok(row),
        None => Err(unpaired()),
    }
}

/// The zone the display shows now, from the listener's zones.
fn resolve_zone(row: &DisplayRow, zones: &[Zone]) -> (ZoneChoice, Option<Zone>) {
    let previous = displays::last_followed(&row.id);
    let choice = displays::choose_zone(zones, row.zone_id.as_deref(), previous.as_deref());
    let zone = match &choice {
        ZoneChoice::Zone(id) => zones.iter().find(|zone| &zone.id == id).cloned(),
        _ => None,
    };
    if row.zone_id.is_none() {
        displays::remember_followed(&row.id, zone.as_ref().map(|zone| zone.id.as_str()));
    }
    (choice, zone)
}

#[derive(Serialize)]
struct DisplayZone {
    id: String,
    name: String,
    icon: String,
    kind: ZoneKind,
    status: ZoneStatus,
    position_ms: i64,
    duration_ms: i64,
    shuffle: bool,
    repeat: RepeatMode,
    /// The signal path rating when the server plans this zone's stream.
    quality: Option<StreamQuality>,
    bit_perfect: Option<bool>,
    /// e.g. `FLAC 96 kHz/24-bit to Topping E30 (exclusive)`.
    output: Option<String>,
    steps: Vec<String>,
}

#[derive(Serialize)]
struct DisplayBiography {
    text: String,
    /// e.g. `English Wikipedia`.
    source: String,
    url: String,
    license: String,
}

#[derive(Serialize)]
struct DisplayTrack {
    song_id: Option<String>,
    title: String,
    artist: String,
    artist_id: Option<String>,
    album: String,
    album_id: Option<String>,
    year: Option<String>,
    composer: Option<String>,
    track_number: Option<u16>,
    track_count: Option<usize>,
    duration_ms: i64,
    format: Option<TrackFormat>,
    /// Signed, so the screen's `<img>` needs no session.
    cover_url: Option<String>,
    artist_image_url: Option<String>,
    artist_image_credit: Option<String>,
    biography: Option<DisplayBiography>,
    /// A live radio stream: no seek bar, no waveform.
    live: bool,
}

#[derive(Serialize)]
struct DisplayQueued {
    song_id: String,
    title: String,
    artist: String,
    cover_url: Option<String>,
}

#[derive(Serialize)]
struct DisplayState {
    display: DisplayView,
    server_time_ms: i64,
    /// `playing` (a track to show), `nothing` (no zone has one) or
    /// `zone_missing` (the bound zone is off or private).
    status: &'static str,
    zone: Option<DisplayZone>,
    track: Option<DisplayTrack>,
    up_next: Vec<DisplayQueued>,
}

fn image_expiry(now_seconds: i64) -> i64 {
    (now_seconds / IMAGE_URL_BUCKET_SECONDS + 2) * IMAGE_URL_BUCKET_SECONDS
}

/// A signed URL for a cover path the catalog stores, or the URL as it is
/// for covers that already need no session (station logos).
fn signed_cover(cover: &str, expires: i64) -> Option<String> {
    let cover = cover.trim();
    if cover.is_empty() {
        return None;
    }
    if cover.starts_with("http://") || cover.starts_with("https://") || cover.starts_with("/api/") {
        return Some(cover.to_string());
    }
    let signature = crate::api::image::sign_image_path(cover, expires);
    Some(format!(
        "/media/images/{}?expires={expires}&image_signature={signature}",
        crate::api::cast::encoded(cover)
    ))
}

fn signed_artist_image(image_id: &str, expires: i64) -> String {
    let signature = crate::api::cast::sign_media_url(ARTIST_IMAGE_SCOPE, image_id, expires);
    format!(
        "/api/v1/display/artist-image/{}?expires={expires}&signature={signature}",
        crate::api::cast::encoded(image_id)
    )
}

/// The opening of a biography: whole sentences up to about 420 characters.
fn opening(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= BIOGRAPHY_CHARS {
        return text;
    }
    let cut: String = text.chars().take(BIOGRAPHY_CHARS).collect();
    match cut.rfind(". ") {
        Some(end) if end > BIOGRAPHY_CHARS / 3 => cut[..=end].to_string(),
        _ => match cut.rfind(' ') {
            Some(end) => format!("{}…", &cut[..end]),
            None => format!("{cut}…"),
        },
    }
}

fn source_name(source: &str, language: &str) -> String {
    if source.starts_with("wikipedia") {
        match language {
            "en" | "" => "English Wikipedia".to_string(),
            "de" => "German Wikipedia".to_string(),
            "fr" => "French Wikipedia".to_string(),
            "es" => "Spanish Wikipedia".to_string(),
            "ja" => "Japanese Wikipedia".to_string(),
            other => format!("Wikipedia ({other})"),
        }
    } else {
        source.to_string()
    }
}

fn display_zone(zone: &Zone) -> DisplayZone {
    let stream = zone
        .output
        .as_ref()
        .and_then(|output| output.stream.as_ref());
    DisplayZone {
        id: zone.id.clone(),
        name: zone.name.clone(),
        icon: zone.icon.clone(),
        kind: zone.kind,
        status: zone.status,
        position_ms: zone.position_ms,
        duration_ms: zone.duration_ms,
        shuffle: zone.shuffle,
        repeat: zone.repeat,
        quality: stream.map(|stream| stream.quality),
        bit_perfect: stream.map(|stream| stream.bit_perfect),
        output: stream.map(|stream| stream.output.clone()),
        steps: stream
            .map(|stream| stream.steps.clone())
            .unwrap_or_default(),
    }
}

fn composer(song: &crate::domain::Song) -> Option<String> {
    let mut names = Vec::<String>::new();
    for credit in &song.credits {
        if credit.role == "composer" && !names.contains(&credit.name) {
            names.push(credit.name.clone());
        }
    }
    (!names.is_empty()).then(|| names.join(", "))
}

fn display_track(
    zone: &Zone,
    cache: Option<&LibraryCache>,
    portrait: Option<&ArtistPortrait>,
    expires: i64,
) -> Option<DisplayTrack> {
    let playing = zone.now_playing.as_ref()?;
    let song_id = playing.song_id.clone();
    let live = song_id
        .as_deref()
        .is_some_and(crate::live_radio::is_live_song_id);
    let song = song_id
        .as_deref()
        .filter(|_| !live)
        .and_then(|id| cache.and_then(|cache| cache.song(id)));
    let context = song_id
        .as_deref()
        .and_then(|id| cache.and_then(|cache| cache.song_map.get(id)));
    let album = context.and_then(|(_, album_id)| cache.and_then(|cache| cache.album(album_id)));
    Some(DisplayTrack {
        song_id,
        title: playing.title.clone(),
        artist: playing.artist.clone(),
        artist_id: context.map(|(artist_id, _)| artist_id.clone()),
        album: playing.album.clone(),
        album_id: playing.album_id.clone(),
        year: album.and_then(|album| displays::release_year(&album.first_release_date)),
        composer: song.and_then(composer),
        track_number: song
            .map(|song| song.track_number)
            .filter(|number| *number > 0),
        track_count: album
            .map(|album| album.songs.len())
            .filter(|count| *count > 0),
        duration_ms: playing.duration_ms,
        format: song.and_then(|song| displays::track_format(song.audio.as_ref())),
        cover_url: playing
            .cover_url
            .as_deref()
            .and_then(|cover| signed_cover(cover, expires)),
        artist_image_url: portrait
            .and_then(|portrait| portrait.image_id.as_deref())
            .map(|id| signed_artist_image(id, expires)),
        artist_image_credit: portrait
            .filter(|portrait| portrait.image_id.is_some())
            .and_then(|portrait| portrait.image_credit.clone()),
        biography: portrait
            .and_then(|portrait| portrait.biography.as_ref())
            .filter(|text| !text.body.trim().is_empty())
            .map(|text| DisplayBiography {
                text: opening(&text.body),
                source: source_name(&text.source, &text.language),
                url: text.url.clone(),
                license: text.license.clone(),
            }),
        live,
    })
}

fn up_next(zone: &Zone, cache: Option<&LibraryCache>, expires: i64) -> Vec<DisplayQueued> {
    let Some(cache) = cache else {
        return Vec::new();
    };
    if zone.repeat == RepeatMode::One {
        return Vec::new();
    }
    let start = zone.queue.current_position + 1;
    let mut ids = zone
        .queue
        .song_ids
        .iter()
        .skip(start)
        .take(UP_NEXT)
        .collect::<Vec<_>>();
    if ids.is_empty() && zone.repeat == RepeatMode::All {
        ids = zone.queue.song_ids.iter().take(UP_NEXT).collect();
    }
    ids.into_iter()
        .filter_map(|id| {
            let song = cache.song(id)?;
            let album = cache
                .song_map
                .get(id)
                .and_then(|(_, album_id)| cache.album(album_id));
            Some(DisplayQueued {
                song_id: id.clone(),
                title: song.name.clone(),
                artist: song.artist.clone(),
                cover_url: album.and_then(|album| signed_cover(&album.cover_url, expires)),
            })
        })
        .collect()
}

/// Recently looked-up artist photos and biographies, so a screen polling
/// every couple of seconds does not rebuild the artist identity each time.
type PortraitCache = HashMap<String, (Instant, Option<ArtistPortrait>)>;

fn portraits() -> &'static Mutex<PortraitCache> {
    static PORTRAITS: OnceLock<Mutex<PortraitCache>> = OnceLock::new();
    PORTRAITS.get_or_init(|| Mutex::new(HashMap::new()))
}

async fn portrait_for(
    pool: &DbPool,
    cache: Option<Arc<LibraryCache>>,
    artist_id: Option<&str>,
) -> Option<ArtistPortrait> {
    let artist_id = artist_id?;
    let cache = cache?;
    let now = Instant::now();
    if let Some((at, cached)) = portraits()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(artist_id)
        && now.duration_since(*at) < PORTRAIT_TTL
    {
        return cached.clone();
    }
    let portrait = crate::api::editorial::artist_portrait(pool.clone(), cache, artist_id).await;
    let mut cached = portraits()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if cached.len() > 512 {
        cached.clear();
    }
    cached.insert(artist_id.to_string(), (now, portrait.clone()));
    portrait
}

async fn state(
    request: HttpRequest,
    pool: web::Data<DbPool>,
    zones: web::Data<ZoneService>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let row = match authorized_display(&request, &pool).await {
        Ok(row) => row,
        Err(response) => return response,
    };
    {
        let id = row.id.clone();
        if let Err(response) = db(&pool, move |connection| {
            displays::touch_display(connection, &id)
        })
        .await
        {
            return response;
        }
    }
    let (all, cache) = zones_for(&zones, &pool, &lifecycle, row.user_id).await;
    let (choice, zone) = resolve_zone(&row, &all);
    let now_ms = chrono::Utc::now().timestamp_millis();
    let expires = image_expiry(now_ms / 1_000);
    let artist_id = zone.as_ref().and_then(|zone| {
        let song_id = zone.now_playing.as_ref()?.song_id.as_deref()?;
        cache
            .as_deref()?
            .song_map
            .get(song_id)
            .map(|(artist_id, _)| artist_id.clone())
    });
    let portrait = portrait_for(&pool, cache.clone(), artist_id.as_deref()).await;
    let track = zone
        .as_ref()
        .and_then(|zone| display_track(zone, cache.as_deref(), portrait.as_ref(), expires));
    let status = match (&choice, &track) {
        (ZoneChoice::Missing, _) => "zone_missing",
        (_, Some(_)) => "playing",
        _ => "nothing",
    };
    // It is asking right now, whatever the stored time says.
    let mut view = row.view(displays::now_utc());
    view.online = true;
    let body = DisplayState {
        display: view,
        server_time_ms: now_ms,
        status,
        up_next: zone
            .as_ref()
            .filter(|_| track.is_some())
            .map(|zone| up_next(zone, cache.as_deref(), expires))
            .unwrap_or_default(),
        zone: zone.as_ref().map(display_zone),
        track,
    };
    no_store(HttpResponse::Ok().json(body))
}

/// Songs a display may read lyrics and waveforms of: what it shows and what
/// comes next on that zone.
async fn shown_songs(
    row: &DisplayRow,
    zones: &ZoneService,
    pool: &DbPool,
    lifecycle: &LibraryLifecycle,
) -> Vec<String> {
    let (all, _) = zones_for(zones, pool, lifecycle, row.user_id).await;
    let previous = displays::last_followed(&row.id);
    let ZoneChoice::Zone(id) =
        displays::choose_zone(&all, row.zone_id.as_deref(), previous.as_deref())
    else {
        return Vec::new();
    };
    let Some(zone) = all.iter().find(|zone| zone.id == id) else {
        return Vec::new();
    };
    let mut songs = zone
        .now_playing
        .as_ref()
        .and_then(|playing| playing.song_id.clone())
        .into_iter()
        .collect::<Vec<_>>();
    songs.extend(
        zone.queue
            .song_ids
            .iter()
            .skip(zone.queue.current_position + 1)
            .take(UP_NEXT)
            .cloned(),
    );
    songs
}

fn not_shown() -> HttpResponse {
    HttpResponse::Forbidden().json(serde_json::json!({
        "error": "song_not_on_display",
        "message": "This display only reads the track it shows."
    }))
}

async fn lyrics(
    request: HttpRequest,
    song_id: web::Path<String>,
    pool: web::Data<DbPool>,
    zones: web::Data<ZoneService>,
    lifecycle: web::Data<LibraryLifecycle>,
    service: web::Data<LyricsService>,
) -> HttpResponse {
    let row = match authorized_display(&request, &pool).await {
        Ok(row) => row,
        Err(response) => return response,
    };
    if LyricsMode::parse(&row.lyrics) == LyricsMode::Off {
        return not_found("Lyrics are off on this display.", "lyrics_off");
    }
    let song_id = song_id.into_inner();
    if !shown_songs(&row, &zones, &pool, &lifecycle)
        .await
        .contains(&song_id)
    {
        return not_shown();
    }
    crate::api::lyrics::lyrics_for_song(song_id, &lifecycle, &service, &pool).await
}

/// Songs whose waveform is being worked out for a display right now.
fn analysing() -> &'static Mutex<std::collections::HashSet<String>> {
    static ANALYSING: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    ANALYSING.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

async fn waveform(
    request: HttpRequest,
    song_id: web::Path<String>,
    pool: web::Data<DbPool>,
    zones: web::Data<ZoneService>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let row = match authorized_display(&request, &pool).await {
        Ok(row) => row,
        Err(response) => return response,
    };
    let song_id = song_id.into_inner();
    if !shown_songs(&row, &zones, &pool, &lifecycle)
        .await
        .contains(&song_id)
    {
        return not_shown();
    }
    let Ok(cache) = lifecycle.cache().await else {
        return not_found("The library is loading.", "library_loading");
    };
    let Some(song) = cache.song(&song_id) else {
        return not_found("Song not found.", "song_not_found");
    };
    let path = song.path.clone();
    let lookup = path.clone();
    let stored = match db(&pool, move |connection| {
        tools_store::analysis_for(connection, &lookup)
    })
    .await
    {
        Ok(stored) => stored,
        Err(response) => return response,
    };
    let version = crate::api::library_tools::file_version(Path::new(&path));
    if let Some(row) = &stored
        && let Some(waveform) = row.waveform()
        && Some(&row.file_version) == version.as_ref()
    {
        let mut response = HttpResponse::Ok();
        response.insert_header(("Cache-Control", "private, max-age=3600"));
        return response.json(serde_json::json!({
            "status": "ready",
            "peaks": waveform.peaks,
            "rms": waveform.rms,
        }));
    }
    if stored
        .as_ref()
        .is_some_and(|row| row.error.is_some() && Some(&row.file_version) == version.as_ref())
    {
        return not_found(
            "This file could not be decoded for a waveform.",
            "waveform_unavailable",
        );
    }
    let Some(version) = version else {
        return not_found("The file is not on disk.", "waveform_unavailable");
    };
    // Work it out once, in the background: one file at a time for every
    // display, saved where Library tools keep theirs.
    let started = {
        let mut analysing = analysing()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        analysing.is_empty() && analysing.insert(path.clone())
    };
    if started {
        let pool = pool.get_ref().clone();
        tokio::spawn(async move {
            let result = analysis::analyze_file(Path::new(&path)).await;
            let saved_path = path.clone();
            let _ = web::block(move || {
                let mut connection = pool.get().map_err(|error| error.to_string())?;
                match &result {
                    Ok(result) => tools_store::save_analysis(
                        &mut connection,
                        &saved_path,
                        &version,
                        Some(&result.waveform),
                        result.fingerprint.as_ref(),
                        result.duration_seconds,
                        None,
                    ),
                    Err(error) => tools_store::save_analysis(
                        &mut connection,
                        &saved_path,
                        &version,
                        None,
                        None,
                        0.0,
                        Some(error),
                    ),
                }
                .map_err(|error| error.to_string())
            })
            .await;
            analysing()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&path);
        });
    }
    no_store(HttpResponse::Accepted().json(serde_json::json!({"status": "analysing"})))
}

#[derive(Deserialize)]
struct SignedQuery {
    expires: i64,
    signature: String,
}

async fn artist_image(
    image_id: web::Path<String>,
    query: web::Query<SignedQuery>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let image_id = image_id.into_inner();
    let now = chrono::Utc::now().timestamp();
    let fresh = query.expires >= now && query.expires <= now + 3 * IMAGE_URL_BUCKET_SECONDS;
    if !fresh
        || !crate::api::cast::verify_media_signature(
            ARTIST_IMAGE_SCOPE,
            &image_id,
            query.expires,
            &query.signature,
        )
    {
        return HttpResponse::Forbidden().finish();
    }
    crate::api::editorial::artist_image_response(pool.get_ref().clone(), image_id).await
}

/// The screen's routes, mounted without a session check under both API
/// prefixes.
pub fn configure_device_routes(cfg: &mut web::ServiceConfig) {
    for prefix in ["/api/v1/display", "/api/music/v1/display"] {
        cfg.route(&format!("{prefix}/pair"), web::post().to(pair_start))
            .route(
                &format!("{prefix}/pair/status"),
                web::post().to(pair_status),
            )
            .route(&format!("{prefix}/state"), web::get().to(state))
            .route(
                &format!("{prefix}/lyrics/{{song_id}}"),
                web::get().to(lyrics),
            )
            .route(
                &format!("{prefix}/waveform/{{song_id}}"),
                web::get().to(waveform),
            )
            .route(
                &format!("{prefix}/artist-image/{{image_id}}"),
                web::get().to(artist_image),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::dev::Service as _;
    use actix_web::test as actix_test;
    use actix_web::{App, HttpMessage};

    use crate::api::auth::Claims;
    use crate::domain::{Album, Artist, AudioProperties, Credit, Song};
    use crate::library::search::SearchIndex;
    use crate::zones::ClientRegistration;

    const USER: i32 = 1;

    fn claims(user: i32) -> Claims {
        Claims {
            sub: user.to_string(),
            exp: usize::MAX,
            username: "listener".to_string(),
            bitrate: 0,
            token_type: "access".to_string(),
            role: "user".to_string(),
            token_version: 0,
            session_id: None,
        }
    }

    fn library() -> LibraryCache {
        let songs = (1..=4)
            .map(|index| Song {
                id: format!("song-{index}"),
                name: format!("Track {index}"),
                artist: "Kimiko Ishizaka".to_string(),
                track_number: index,
                path: format!("/music/goldberg/{index:02}.flac"),
                duration: 180.0,
                audio: Some(AudioProperties {
                    codec: Some("flac".into()),
                    container: Some("flac".into()),
                    sample_rate: Some(96_000),
                    bit_depth: Some(24),
                    channels: Some(2),
                    bitrate_kbps: None,
                    lossless: Some(true),
                    enhanced_by: None,
                }),
                credits: vec![Credit {
                    artist_id: "bach".into(),
                    name: "Johann Sebastian Bach".into(),
                    role: "composer".into(),
                    instrument: None,
                }],
                ..Song::default()
            })
            .collect::<Vec<_>>();
        let album = Album {
            id: "album-1".to_string(),
            name: "The Open Goldberg Variations".to_string(),
            cover_url: "/music/goldberg/cover.jpg".to_string(),
            first_release_date: "2012-05-29".to_string(),
            songs: songs.clone(),
            ..Album::default()
        };
        let artist = Artist {
            id: "artist-1".to_string(),
            name: "Kimiko Ishizaka".to_string(),
            albums: vec![album],
            ..Artist::default()
        };
        LibraryCache {
            artists: Arc::new(vec![artist.clone()]),
            search_index: SearchIndex::build(std::slice::from_ref(&artist)).expect("index"),
            song_map: songs
                .iter()
                .map(|song| {
                    (
                        song.id.clone(),
                        ("artist-1".to_string(), "album-1".to_string()),
                    )
                })
                .collect(),
            album_genres: HashMap::new(),
            artist_positions: HashMap::from([("artist-1".to_string(), 0)]),
            album_positions: HashMap::from([("album-1".to_string(), (0, 0))]),
            song_positions: songs
                .iter()
                .enumerate()
                .map(|(index, song)| (song.id.clone(), (0, 0, index as u32)))
                .collect(),
            songs_flat: songs.iter().map(|song| song.id.clone()).collect(),
            songs_by_artist: HashMap::from([("artist-1".to_string(), vec![0, 1, 2, 3])]),
            songs_by_genre: HashMap::new(),
            image_paths: Default::default(),
        }
    }

    fn browser(status: ZoneStatus, position: usize) -> ClientRegistration {
        ClientRegistration {
            kind: ZoneKind::Web,
            name: "Living room".to_string(),
            device: "Chrome on Linux".to_string(),
            status,
            song_ids: Some((1..=4).map(|index| format!("song-{index}")).collect()),
            current_position: position,
            position_ms: 42_000,
            duration_ms: 180_000,
            volume: 70,
            muted: false,
            acknowledged_seq: 0,
            repeat: RepeatMode::Off,
            shuffle: false,
        }
    }

    macro_rules! app {
        ($zones:expr, $lifecycle:expr, $pool:expr) => {
            actix_test::init_service(
                App::new()
                    .app_data($zones.clone())
                    .app_data($lifecycle.clone())
                    .app_data($pool.clone())
                    .app_data(web::Data::new(LyricsService::new().expect("lyrics")))
                    .configure(configure_device_routes)
                    .service(
                        web::scope("/api/v1")
                            .wrap_fn(|request, next| {
                                request.extensions_mut().insert(claims(USER));
                                next.call(request)
                            })
                            .configure(configure),
                    ),
            )
            .await
        };
    }

    #[actix_web::test]
    async fn a_linked_display_follows_its_zone_and_reads_nothing_else() {
        let zones = web::Data::new(ZoneService::new(None, None));
        let lifecycle = web::Data::new(LibraryLifecycle::new());
        lifecycle.set_ready(library()).await;
        let pool = web::Data::new(crate::recommendation::test_support::radio_pool());
        zones
            .register_client(USER, "living-room-01", browser(ZoneStatus::Playing, 1))
            .expect("browser zone");
        let app = app!(zones, lifecycle, pool);

        // No key, no state.
        let refused = actix_test::call_service(
            &app,
            actix_test::TestRequest::get()
                .uri("/api/v1/display/state")
                .to_request(),
        )
        .await;
        assert_eq!(refused.status(), 403);

        let created: serde_json::Value = actix_test::call_and_read_body_json(
            &app,
            actix_test::TestRequest::post()
                .uri("/api/v1/displays")
                .set_json(serde_json::json!({
                    "name": "Living room TV",
                    "zone_id": "client:living-room-01"
                }))
                .to_request(),
        )
        .await;
        let key = created["key"].as_str().expect("key").to_string();
        assert_eq!(created["display"]["zone_id"], "client:living-room-01");

        let state: serde_json::Value = actix_test::call_and_read_body_json(
            &app,
            actix_test::TestRequest::get()
                .uri("/api/v1/display/state")
                .insert_header((DISPLAY_KEY_HEADER, key.clone()))
                .to_request(),
        )
        .await;
        assert_eq!(state["status"], "playing");
        assert_eq!(state["display"]["online"], true);
        assert_eq!(state["zone"]["name"], "Living room");
        // The zone list moves a playing zone's position on with the clock.
        let position = state["zone"]["position_ms"].as_i64().unwrap();
        assert!((42_000..50_000).contains(&position));
        assert_eq!(state["track"]["title"], "Track 2");
        assert_eq!(state["track"]["album"], "The Open Goldberg Variations");
        assert_eq!(state["track"]["year"], "2012");
        assert_eq!(state["track"]["composer"], "Johann Sebastian Bach");
        assert_eq!(state["track"]["track_number"], 2);
        assert_eq!(state["track"]["track_count"], 4);
        assert_eq!(state["track"]["format"]["label"], "FLAC 96 kHz/24-bit");
        assert_eq!(state["track"]["format"]["tier"], "hi_res");
        let cover = state["track"]["cover_url"].as_str().expect("cover");
        assert!(cover.starts_with("/media/images/"));
        assert!(cover.contains("image_signature="));
        assert_eq!(state["up_next"].as_array().map(Vec::len), Some(2));
        assert_eq!(state["up_next"][0]["title"], "Track 3");

        // The track it shows, yes; another track of the library, no.
        let other = actix_test::call_service(
            &app,
            actix_test::TestRequest::get()
                .uri("/api/v1/display/waveform/song-1")
                .insert_header((DISPLAY_KEY_HEADER, key.clone()))
                .to_request(),
        )
        .await;
        assert_eq!(other.status(), 403);
        let lyrics_elsewhere = actix_test::call_service(
            &app,
            actix_test::TestRequest::get()
                .uri("/api/v1/display/lyrics/song-1")
                .insert_header((DISPLAY_KEY_HEADER, key.clone()))
                .to_request(),
        )
        .await;
        assert_eq!(lyrics_elsewhere.status(), 403);
        // The fixture file is not on disk: no waveform, but no refusal.
        let shown = actix_test::call_service(
            &app,
            actix_test::TestRequest::get()
                .uri("/api/v1/display/waveform/song-2")
                .insert_header((DISPLAY_KEY_HEADER, key.clone()))
                .to_request(),
        )
        .await;
        assert_eq!(shown.status(), 404);

        // The zone moves on; the display follows.
        zones
            .register_client(USER, "living-room-01", browser(ZoneStatus::Paused, 3))
            .expect("browser zone");
        let state: serde_json::Value = actix_test::call_and_read_body_json(
            &app,
            actix_test::TestRequest::get()
                .uri("/api/v1/display/state")
                .insert_header((DISPLAY_KEY_HEADER, key.clone()))
                .to_request(),
        )
        .await;
        assert_eq!(state["track"]["title"], "Track 4");
        assert_eq!(state["zone"]["status"], "paused");
        assert_eq!(state["up_next"].as_array().map(Vec::len), Some(0));

        // Removing the display revokes the key.
        let id = created["display"]["id"].as_str().unwrap();
        let removed = actix_test::call_service(
            &app,
            actix_test::TestRequest::delete()
                .uri(&format!("/api/v1/displays/{id}"))
                .to_request(),
        )
        .await;
        assert_eq!(removed.status(), 204);
        let revoked = actix_test::call_service(
            &app,
            actix_test::TestRequest::get()
                .uri("/api/v1/display/state")
                .insert_header((DISPLAY_KEY_HEADER, key))
                .to_request(),
        )
        .await;
        assert_eq!(revoked.status(), 403);
    }

    #[actix_web::test]
    async fn a_screen_pairs_by_code_and_then_follows_what_plays() {
        let zones = web::Data::new(ZoneService::new(None, None));
        let lifecycle = web::Data::new(LibraryLifecycle::new());
        lifecycle.set_ready(library()).await;
        let pool = web::Data::new(crate::recommendation::test_support::radio_pool());
        let app = app!(zones, lifecycle, pool);

        let started: serde_json::Value = actix_test::call_and_read_body_json(
            &app,
            actix_test::TestRequest::post()
                .uri("/api/v1/display/pair")
                .set_json(serde_json::json!({"name": "Bedroom TV"}))
                .to_request(),
        )
        .await;
        let code = started["code"].as_str().unwrap().to_string();
        let poll = serde_json::json!({
            "pairing_id": started["pairing_id"],
            "secret": started["secret"],
        });
        let pending = actix_test::call_service(
            &app,
            actix_test::TestRequest::post()
                .uri("/api/v1/display/pair/status")
                .set_json(poll.clone())
                .to_request(),
        )
        .await;
        assert_eq!(pending.status(), 202);

        let wrong = actix_test::call_service(
            &app,
            actix_test::TestRequest::post()
                .uri("/api/v1/displays/approve")
                .set_json(serde_json::json!({"code": "12345"}))
                .to_request(),
        )
        .await;
        assert_eq!(wrong.status(), 400);
        let unknown_zone = actix_test::call_service(
            &app,
            actix_test::TestRequest::post()
                .uri("/api/v1/displays/approve")
                .set_json(serde_json::json!({"code": code, "zone_id": "client:nowhere-0001"}))
                .to_request(),
        )
        .await;
        assert_eq!(unknown_zone.status(), 404);

        let approved: serde_json::Value = actix_test::call_and_read_body_json(
            &app,
            actix_test::TestRequest::post()
                .uri("/api/v1/displays/approve")
                .set_json(serde_json::json!({"code": code}))
                .to_request(),
        )
        .await;
        assert_eq!(approved["display"]["name"], "Bedroom TV");
        assert!(
            approved.get("key").is_none(),
            "the key goes to the screen only"
        );

        let delivered: serde_json::Value = actix_test::call_and_read_body_json(
            &app,
            actix_test::TestRequest::post()
                .uri("/api/v1/display/pair/status")
                .set_json(poll.clone())
                .to_request(),
        )
        .await;
        assert_eq!(delivered["status"], "approved");
        let key = delivered["key"].as_str().unwrap().to_string();
        let again = actix_test::call_service(
            &app,
            actix_test::TestRequest::post()
                .uri("/api/v1/display/pair/status")
                .set_json(poll)
                .to_request(),
        )
        .await;
        assert_eq!(again.status(), 404);

        // Following: nothing yet, then whatever starts playing.
        let idle: serde_json::Value = actix_test::call_and_read_body_json(
            &app,
            actix_test::TestRequest::get()
                .uri("/api/v1/display/state")
                .insert_header((DISPLAY_KEY_HEADER, key.clone()))
                .to_request(),
        )
        .await;
        assert_eq!(idle["status"], "nothing");
        assert!(idle["track"].is_null());
        zones
            .register_client(USER, "kitchen-0001", browser(ZoneStatus::Playing, 0))
            .expect("kitchen zone");
        // Another listener's zone never shows.
        zones
            .register_client(2, "neighbour-0001", browser(ZoneStatus::Playing, 2))
            .expect("other listener");
        let playing: serde_json::Value = actix_test::call_and_read_body_json(
            &app,
            actix_test::TestRequest::get()
                .uri("/api/v1/display/state")
                .insert_header((DISPLAY_KEY_HEADER, key.clone()))
                .to_request(),
        )
        .await;
        assert_eq!(playing["status"], "playing");
        assert_eq!(playing["zone"]["id"], "client:kitchen-0001");
        assert_eq!(playing["track"]["title"], "Track 1");

        // Bound to a zone that goes away: the screen says so.
        let listed: serde_json::Value = actix_test::call_and_read_body_json(
            &app,
            actix_test::TestRequest::get()
                .uri("/api/v1/displays")
                .to_request(),
        )
        .await;
        let id = listed["displays"][0]["id"].as_str().unwrap().to_string();
        let bound = actix_test::call_service(
            &app,
            actix_test::TestRequest::patch()
                .uri(&format!("/api/v1/displays/{id}"))
                .set_json(serde_json::json!({"zone_id": "client:kitchen-0001", "lyrics": "off"}))
                .to_request(),
        )
        .await;
        assert_eq!(bound.status(), 200);
        zones.unregister_client(USER, "kitchen-0001");
        let missing: serde_json::Value = actix_test::call_and_read_body_json(
            &app,
            actix_test::TestRequest::get()
                .uri("/api/v1/display/state")
                .insert_header((DISPLAY_KEY_HEADER, key.clone()))
                .to_request(),
        )
        .await;
        assert_eq!(missing["status"], "zone_missing");
        assert_eq!(missing["display"]["lyrics"], "off");
    }

    #[test]
    fn signed_artist_images_are_bound_to_the_photo_and_expire() {
        let expires = image_expiry(chrono::Utc::now().timestamp());
        let url = signed_artist_image("artist-1", expires);
        let signature = url.split("signature=").nth(1).unwrap();
        assert!(crate::api::cast::verify_media_signature(
            ARTIST_IMAGE_SCOPE,
            "artist-1",
            expires,
            signature
        ));
        assert!(!crate::api::cast::verify_media_signature(
            ARTIST_IMAGE_SCOPE,
            "artist-2",
            expires,
            signature
        ));
        assert!(!crate::api::cast::verify_media_signature(
            "cast-session",
            "artist-1",
            expires,
            signature
        ));
    }

    #[test]
    fn image_urls_hold_still_within_the_hour() {
        let base = 1_800_000_000 - (1_800_000_000 % IMAGE_URL_BUCKET_SECONDS);
        assert_eq!(image_expiry(base + 5), image_expiry(base + 3_000));
        assert!(image_expiry(base) - base >= IMAGE_URL_BUCKET_SECONDS);
        assert!(image_expiry(base + 3_599) - (base + 3_599) <= 2 * IMAGE_URL_BUCKET_SECONDS);
        assert_eq!(signed_cover("", 1), None);
        assert_eq!(
            signed_cover("/api/v1/live-radio/stations/1/logo", 1).as_deref(),
            Some("/api/v1/live-radio/stations/1/logo")
        );
    }

    #[test]
    fn biographies_open_with_whole_sentences() {
        let short = "A pianist. She records Bach.";
        assert_eq!(opening(short), short);
        let long = format!(
            "{} Second sentence goes on and on.",
            "Word ".repeat(70).trim()
        );
        let cut = opening(&format!("First sentence is here. {long}"));
        assert!(cut.chars().count() <= BIOGRAPHY_CHARS + 1);
        assert!(cut.ends_with('.') || cut.ends_with('…'));
        assert_eq!(source_name("wikipedia", "en"), "English Wikipedia");
    }
}
