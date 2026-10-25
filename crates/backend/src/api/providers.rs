//! `/providers`: TIDAL and Qobuz sign-in, catalog, library and streaming.

use actix_web::http::{StatusCode, header};
use actix_web::{HttpMessage, HttpRequest, HttpResponse, delete, get, patch, post, put, web};
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use crate::api::auth::{Claims, authenticated_user_id};
use crate::api::error::{forbidden, json_error};
use crate::providers::hub::{HubError, LibraryRequest, StreamingHub, user_message};
pub(crate) use crate::providers::media::byte_range;
use crate::providers::media::{self, MediaError};
use crate::providers::store::{LibraryItemRow, LibrarySort};
use crate::providers::{
    BrowseSection, ItemKind, ProviderError, ProviderId, ResolvedStream, StreamQuality,
    StreamSource, entity_id, probe, sandbox, valid_raw_id,
};

const MAX_SEARCH_QUERY_CHARACTERS: usize = 200;
const MAX_SEARCH_LIMIT: u32 = 25;

pub fn hub_error_response(error: HubError) -> HttpResponse {
    let message = user_message(&error);
    let (status, code) = match &error {
        HubError::Provider(error) => match error {
            ProviderError::NotConnected => (StatusCode::CONFLICT, "provider_not_connected"),
            ProviderError::SessionExpired => (StatusCode::CONFLICT, "provider_session_expired"),
            ProviderError::InvalidCredentials(_) => {
                (StatusCode::BAD_REQUEST, "provider_credentials_rejected")
            }
            ProviderError::NotFound => (StatusCode::NOT_FOUND, "provider_item_not_found"),
            ProviderError::Unavailable(_) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "provider_content_unavailable",
            ),
            ProviderError::RateLimited => {
                (StatusCode::SERVICE_UNAVAILABLE, "provider_rate_limited")
            }
            ProviderError::Upstream(_) | ProviderError::Decode(_) => {
                (StatusCode::BAD_GATEWAY, "provider_unavailable")
            }
            ProviderError::Transport(_) => (StatusCode::GATEWAY_TIMEOUT, "provider_unreachable"),
        },
        HubError::Database(_) => (StatusCode::INTERNAL_SERVER_ERROR, "provider_storage_failed"),
        HubError::Invalid(_) => (StatusCode::BAD_REQUEST, "invalid_provider_request"),
        HubError::CredentialsRequired(_) => (StatusCode::CONFLICT, "provider_credentials_required"),
    };
    if status.is_server_error() {
        tracing::warn!(%error, code, "streaming provider request failed");
    }
    json_error(status, code, message, code)
}

fn unknown_provider() -> HttpResponse {
    json_error(
        StatusCode::NOT_FOUND,
        "not_found",
        "Unknown streaming service.",
        "unknown_provider",
    )
}

macro_rules! user_and_provider {
    ($request:expr, $provider:expr) => {{
        let user_id = match authenticated_user_id(&$request) {
            Ok(id) => id,
            Err(response) => return response,
        };
        let Some(provider) = ProviderId::parse($provider) else {
            return unknown_provider();
        };
        (user_id, provider)
    }};
}

fn respond<T: Serialize>(result: Result<T, HubError>) -> HttpResponse {
    match result {
        Ok(value) => HttpResponse::Ok().json(value),
        Err(error) => hub_error_response(error),
    }
}

#[derive(Serialize)]
struct ProvidersResponse {
    sandbox: bool,
    providers: Vec<crate::providers::hub::ProviderStatus>,
}

#[get("")]
async fn list_providers(request: HttpRequest, hub: web::Data<StreamingHub>) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    match hub.status(user_id).await {
        Ok(providers) => HttpResponse::Ok().json(ProvidersResponse {
            sandbox: hub.is_sandbox(),
            providers,
        }),
        Err(error) => hub_error_response(error),
    }
}

#[derive(Deserialize)]
struct CredentialsRequest {
    client_id: Option<String>,
    client_secret: Option<String>,
    #[serde(default)]
    use_server_credentials: bool,
}

#[put("/{provider}/credentials")]
async fn save_credentials(
    request: HttpRequest,
    path: web::Path<String>,
    body: web::Json<CredentialsRequest>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let (user_id, provider) = user_and_provider!(request, &path);
    let body = body.into_inner();
    respond(
        hub.save_credentials(
            user_id,
            provider,
            body.client_id,
            body.client_secret,
            body.use_server_credentials,
        )
        .await,
    )
}

#[post("/tidal/sign-in")]
async fn start_tidal_sign_in(request: HttpRequest, hub: web::Data<StreamingHub>) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    respond(hub.start_tidal_sign_in(user_id).await)
}

#[post("/tidal/sign-in/poll")]
async fn poll_tidal_sign_in(request: HttpRequest, hub: web::Data<StreamingHub>) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    respond(hub.poll_tidal_sign_in(user_id).await)
}

#[derive(Deserialize)]
struct PasswordSignIn {
    username: String,
    password: String,
}

#[post("/qobuz/sign-in")]
async fn qobuz_sign_in(
    request: HttpRequest,
    body: web::Json<PasswordSignIn>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    respond(
        hub.qobuz_sign_in(user_id, &body.username, &body.password)
            .await,
    )
}

#[derive(Deserialize)]
struct UpdateProvider {
    quality: String,
}

#[patch("/{provider}")]
async fn update_provider(
    request: HttpRequest,
    path: web::Path<String>,
    body: web::Json<UpdateProvider>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let (user_id, provider) = user_and_provider!(request, &path);
    let Some(quality) = StreamQuality::parse(&body.quality) else {
        return hub_error_response(HubError::Invalid("Unknown streaming quality."));
    };
    respond(hub.set_quality(user_id, provider, quality).await)
}

#[post("/{provider}/sign-out")]
async fn sign_out(
    request: HttpRequest,
    path: web::Path<String>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let (user_id, provider) = user_and_provider!(request, &path);
    respond(hub.sign_out(user_id, provider).await)
}

#[delete("/{provider}")]
async fn remove_provider(
    request: HttpRequest,
    path: web::Path<String>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let (user_id, provider) = user_and_provider!(request, &path);
    respond(hub.remove(user_id, provider).await)
}

#[post("/{provider}/library/sync")]
async fn sync_library(
    request: HttpRequest,
    path: web::Path<String>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let (user_id, provider) = user_and_provider!(request, &path);
    respond(hub.sync_library(user_id, provider).await)
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    providers: Option<String>,
    limit: Option<u32>,
}

#[derive(Serialize)]
struct SearchResponse {
    query: String,
    services: Vec<crate::providers::hub::ProviderSearch>,
}

#[get("/search")]
async fn search(
    request: HttpRequest,
    query: web::Query<SearchQuery>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let text = query.q.trim();
    if text.is_empty() || text.chars().count() > MAX_SEARCH_QUERY_CHARACTERS {
        return hub_error_response(HubError::Invalid("Enter a search of up to 200 characters."));
    }
    let connected = match hub.connected(user_id).await {
        Ok(connected) => connected,
        Err(error) => return hub_error_response(error),
    };
    let requested = query.providers.as_deref().map(|value| {
        value
            .split(',')
            .filter_map(|name| ProviderId::parse(name.trim()))
            .collect::<Vec<_>>()
    });
    let providers = connected
        .into_iter()
        .filter(|provider| {
            requested
                .as_ref()
                .is_none_or(|requested| requested.contains(provider))
        })
        .collect::<Vec<_>>();
    let limit = query.limit.unwrap_or(10).clamp(1, MAX_SEARCH_LIMIT);
    match hub.search(user_id, text, &providers, limit).await {
        Ok(services) => HttpResponse::Ok().json(SearchResponse {
            query: text.to_string(),
            services,
        }),
        Err(error) => hub_error_response(error),
    }
}

#[derive(Deserialize)]
struct LibraryParameters {
    provider: Option<String>,
    kind: Option<String>,
    q: Option<String>,
    sort: Option<String>,
    offset: Option<i64>,
    limit: Option<i64>,
}

/// A library row with its provider-scoped id and the stored snapshot.
#[derive(Serialize)]
struct LibraryItem {
    id: String,
    provider: String,
    kind: String,
    title: String,
    artist_name: String,
    album_title: Option<String>,
    cover_url: Option<String>,
    release_date: Option<String>,
    duration: f64,
    track_count: i32,
    format_label: Option<String>,
    origin: String,
    added_at: i64,
    item: serde_json::Value,
}

impl From<LibraryItemRow> for LibraryItem {
    fn from(row: LibraryItemRow) -> Self {
        let id = ProviderId::parse(&row.provider)
            .map(|provider| entity_id(provider, &row.provider_item_id))
            .unwrap_or_default();
        Self {
            id,
            item: serde_json::from_str(&row.payload).unwrap_or(serde_json::Value::Null),
            provider: row.provider,
            kind: row.kind,
            title: row.title,
            artist_name: row.artist_name,
            album_title: row.album_title,
            cover_url: row.cover_url,
            release_date: row.release_date,
            duration: row.duration,
            track_count: row.track_count,
            format_label: row.format_label,
            origin: row.origin,
            added_at: row.added_at,
        }
    }
}

#[derive(Serialize)]
struct LibraryResponse {
    items: Vec<LibraryItem>,
    total: i64,
}

#[get("/library")]
async fn library(
    request: HttpRequest,
    query: web::Query<LibraryParameters>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let provider = match query.provider.as_deref().filter(|value| !value.is_empty()) {
        Some(value) => match ProviderId::parse(value) {
            Some(provider) => Some(provider),
            None => return unknown_provider(),
        },
        None => None,
    };
    let Some(kind) = ItemKind::parse(query.kind.as_deref().unwrap_or("album")) else {
        return hub_error_response(HubError::Invalid("Unknown library item kind."));
    };
    let sort = match query.sort.as_deref() {
        None | Some("title") => LibrarySort::Title,
        Some("recent") => LibrarySort::Recent,
        Some("artist") => LibrarySort::Artist,
        Some(_) => return hub_error_response(HubError::Invalid("Unknown library sort.")),
    };
    let text = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(MAX_SEARCH_QUERY_CHARACTERS).collect());
    match hub
        .library(
            user_id,
            LibraryRequest {
                provider,
                kind,
                text,
                sort,
                offset: query.offset.unwrap_or(0),
                limit: query.limit.unwrap_or(100),
            },
        )
        .await
    {
        Ok((rows, total)) => HttpResponse::Ok().json(LibraryResponse {
            items: rows.into_iter().map(LibraryItem::from).collect(),
            total,
        }),
        Err(error) => hub_error_response(error),
    }
}

fn provider_and_raw(provider: &str, raw: &str) -> Option<(ProviderId, String)> {
    let provider = ProviderId::parse(provider)?;
    valid_raw_id(raw).then(|| (provider, raw.to_string()))
}

#[get("/{provider}/albums/{id}")]
async fn album(
    request: HttpRequest,
    path: web::Path<(String, String)>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let Some((provider, raw)) = provider_and_raw(&path.0, &path.1) else {
        return unknown_provider();
    };
    respond(hub.album(user_id, provider, raw).await)
}

#[get("/{provider}/artists/{id}")]
async fn artist(
    request: HttpRequest,
    path: web::Path<(String, String)>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let Some((provider, raw)) = provider_and_raw(&path.0, &path.1) else {
        return unknown_provider();
    };
    respond(hub.artist(user_id, provider, raw).await)
}

#[derive(Deserialize)]
struct PageParameters {
    offset: Option<u32>,
    limit: Option<u32>,
}

#[get("/{provider}/browse/{section}")]
async fn browse(
    request: HttpRequest,
    path: web::Path<(String, String)>,
    page: web::Query<PageParameters>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let (user_id, provider) = user_and_provider!(request, &path.0);
    let Some(section) = BrowseSection::parse(&path.1) else {
        return hub_error_response(HubError::Invalid("Unknown browse section."));
    };
    respond(
        hub.browse(
            user_id,
            provider,
            section,
            page.offset.unwrap_or(0).min(10_000),
            page.limit.unwrap_or(50).clamp(1, 100),
        )
        .await,
    )
}

#[get("/{provider}/playlists")]
async fn playlists(
    request: HttpRequest,
    path: web::Path<String>,
    page: web::Query<PageParameters>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let (user_id, provider) = user_and_provider!(request, &path);
    respond(
        hub.playlists(
            user_id,
            provider,
            page.offset.unwrap_or(0).min(10_000),
            page.limit.unwrap_or(50).clamp(1, 100),
        )
        .await,
    )
}

#[get("/{provider}/playlists/{id}")]
async fn playlist(
    request: HttpRequest,
    path: web::Path<(String, String)>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let Some((provider, raw)) = provider_and_raw(&path.0, &path.1) else {
        return unknown_provider();
    };
    respond(hub.playlist(user_id, provider, raw).await)
}

#[derive(Serialize)]
struct PlaylistCopiesResponse {
    copies: Vec<crate::providers::hub::PlaylistCopy>,
}

#[get("/playlist-copies")]
async fn playlist_copies(request: HttpRequest, hub: web::Data<StreamingHub>) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    respond(
        hub.playlist_copies(user_id)
            .await
            .map(|copies| PlaylistCopiesResponse { copies }),
    )
}

/// A saved copy of a service playlist, with its tracks.
#[get("/{provider}/playlists/{id}/copy")]
async fn playlist_copy(
    request: HttpRequest,
    path: web::Path<(String, String)>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let Some((provider, raw)) = provider_and_raw(&path.0, &path.1) else {
        return unknown_provider();
    };
    respond(hub.playlist_copy(user_id, provider, raw).await)
}

/// Saves (or refreshes) a copy. The service playlist is not changed.
#[put("/{provider}/playlists/{id}/copy")]
async fn save_playlist_copy(
    request: HttpRequest,
    path: web::Path<(String, String)>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let Some((provider, raw)) = provider_and_raw(&path.0, &path.1) else {
        return unknown_provider();
    };
    respond(hub.save_playlist_copy(user_id, provider, raw).await)
}

#[delete("/{provider}/playlists/{id}/copy")]
async fn remove_playlist_copy(
    request: HttpRequest,
    path: web::Path<(String, String)>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let Some((provider, raw)) = provider_and_raw(&path.0, &path.1) else {
        return unknown_provider();
    };
    match hub.remove_playlist_copy(user_id, provider, raw).await {
        Ok(()) => HttpResponse::NoContent().finish(),
        Err(error) => hub_error_response(error),
    }
}

#[put("/{provider}/library/{kind}/{id}")]
async fn add_to_library(
    request: HttpRequest,
    path: web::Path<(String, String, String)>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let (Some((provider, raw)), Some(kind)) =
        (provider_and_raw(&path.0, &path.2), ItemKind::parse(&path.1))
    else {
        return unknown_provider();
    };
    respond(hub.add_to_library(user_id, provider, kind, raw).await)
}

#[delete("/{provider}/library/{kind}/{id}")]
async fn remove_from_library(
    request: HttpRequest,
    path: web::Path<(String, String, String)>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let (Some((provider, raw)), Some(kind)) =
        (provider_and_raw(&path.0, &path.2), ItemKind::parse(&path.1))
    else {
        return unknown_provider();
    };
    respond(hub.remove_from_library(user_id, provider, kind, raw).await)
}

#[derive(Serialize)]
struct StreamInfo {
    format: crate::providers::AudioFormat,
    requested: StreamQuality,
    sandbox: bool,
}

/// What a stream would deliver (format, bit depth, sample rate), without
/// exposing the signed media URL.
#[get("/{provider}/tracks/{id}/stream-info")]
async fn stream_info(
    request: HttpRequest,
    path: web::Path<(String, String)>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let Some((provider, raw)) = provider_and_raw(&path.0, &path.1) else {
        return unknown_provider();
    };
    match hub.resolve_stream(user_id, provider, raw, false).await {
        Ok(stream) => HttpResponse::Ok().json(StreamInfo {
            format: stream.format,
            requested: stream.requested,
            sandbox: hub.is_sandbox(),
        }),
        Err(error) => hub_error_response(error),
    }
}

/// Server-wide credentials are an owner setting.
fn require_admin(request: &HttpRequest) -> Result<i32, HttpResponse> {
    let user_id = authenticated_user_id(request)?;
    let is_admin = request
        .extensions()
        .get::<Claims>()
        .is_some_and(|claims| claims.role == "admin");
    if is_admin {
        Ok(user_id)
    } else {
        Err(forbidden(
            "Only the server owner can change the server's streaming credentials.",
            "admin_required",
        ))
    }
}

#[derive(Serialize)]
struct ServerCredentialsResponse {
    providers: Vec<crate::providers::hub::ServerCredentialStatus>,
}

#[get("/server-credentials")]
async fn list_server_credentials(
    request: HttpRequest,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    if let Err(response) = require_admin(&request) {
        return response;
    }
    HttpResponse::Ok().json(ServerCredentialsResponse {
        providers: hub.server_credentials_status(),
    })
}

#[derive(Deserialize)]
struct ServerCredentialsRequest {
    client_id: Option<String>,
    client_secret: Option<String>,
}

#[put("/{provider}/server-credentials")]
async fn save_server_credentials(
    request: HttpRequest,
    path: web::Path<String>,
    body: web::Json<ServerCredentialsRequest>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let admin_id = match require_admin(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let Some(provider) = ProviderId::parse(&path) else {
        return unknown_provider();
    };
    let body = body.into_inner();
    respond(
        hub.save_server_credentials(admin_id, provider, body.client_id, body.client_secret)
            .await
            .map(|providers| ServerCredentialsResponse { providers }),
    )
}

#[delete("/{provider}/server-credentials")]
async fn remove_server_credentials(
    request: HttpRequest,
    path: web::Path<String>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    if let Err(response) = require_admin(&request) {
        return response;
    }
    let Some(provider) = ProviderId::parse(&path) else {
        return unknown_provider();
    };
    respond(
        hub.remove_server_credentials(provider)
            .await
            .map(|providers| ServerCredentialsResponse { providers }),
    )
}

#[derive(Deserialize, Default)]
struct CheckRequest {
    track_id: Option<String>,
}

/// Signs in, syncs favorites and reads a real stream's delivered format.
#[post("/{provider}/check")]
async fn check_connection(
    request: HttpRequest,
    path: web::Path<String>,
    body: Option<web::Json<CheckRequest>>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let (user_id, provider) = user_and_provider!(request, &path);
    let track = match body.and_then(|body| body.into_inner().track_id) {
        Some(id) => match crate::providers::parse_entity_id(&id) {
            Some((owner, raw)) if owner == provider => Some(raw),
            None if valid_raw_id(&id) => Some(id),
            _ => return hub_error_response(HubError::Invalid("Unknown track id.")),
        },
        None => None,
    };
    respond(hub.check_connection(user_id, provider, track).await)
}

/// The format the listener's latest stream of this track actually
/// delivered, read from its bytes by the media proxy.
#[get("/{provider}/tracks/{id}/delivery")]
async fn stream_delivery(
    request: HttpRequest,
    path: web::Path<(String, String)>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let Some((provider, raw)) = provider_and_raw(&path.0, &path.1) else {
        return unknown_provider();
    };
    match hub.delivery(user_id, &entity_id(provider, &raw)) {
        Some(delivery) => HttpResponse::Ok().json(delivery),
        None => json_error(
            StatusCode::NOT_FOUND,
            "not_found",
            "This track has not streamed yet.",
            "delivery_unknown",
        ),
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/providers")
            .service(list_server_credentials)
            .service(save_server_credentials)
            .service(remove_server_credentials)
            .service(check_connection)
            .service(stream_delivery)
            .service(playlist_copies)
            .service(playlists)
            .service(playlist_copy)
            .service(save_playlist_copy)
            .service(remove_playlist_copy)
            .service(playlist)
            .service(list_providers)
            .service(search)
            .service(library)
            .service(start_tidal_sign_in)
            .service(poll_tidal_sign_in)
            .service(qobuz_sign_in)
            .service(save_credentials)
            .service(sync_library)
            .service(sign_out)
            .service(album)
            .service(artist)
            .service(browse)
            .service(stream_info)
            .service(add_to_library)
            .service(remove_from_library)
            .service(update_provider)
            .service(remove_provider),
    );
}

fn serve_bytes(request: &HttpRequest, body: Vec<u8>, content_type: &str) -> HttpResponse {
    let length = body.len() as u64;
    let range = request
        .headers()
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());
    match byte_range(range, length) {
        Some((start, end)) => HttpResponse::PartialContent()
            .insert_header((header::CONTENT_TYPE, content_type.to_string()))
            .insert_header((header::ACCEPT_RANGES, "bytes"))
            .insert_header((
                header::CONTENT_RANGE,
                format!("bytes {start}-{end}/{length}"),
            ))
            .insert_header((header::CACHE_CONTROL, "private, no-store"))
            .body(body[start as usize..=end as usize].to_vec()),
        None if range.is_some() => HttpResponse::RangeNotSatisfiable()
            .insert_header((header::CONTENT_RANGE, format!("bytes */{length}")))
            .finish(),
        None => HttpResponse::Ok()
            .insert_header((header::CONTENT_TYPE, content_type.to_string()))
            .insert_header((header::ACCEPT_RANGES, "bytes"))
            .insert_header((header::CACHE_CONTROL, "private, no-store"))
            .body(body),
    }
}

fn proxy_error(error: MediaError) -> actix_web::Error {
    actix_web::error::ErrorBadGateway(error.to_string())
}

fn format_headers(
    mut builder: actix_web::HttpResponseBuilder,
    stream: &ResolvedStream,
) -> actix_web::HttpResponseBuilder {
    builder.insert_header(("X-Parson-Stream-Format", stream.format.label.clone()));
    builder.insert_header(("X-Parson-Stream-Quality", stream.requested.as_str()));
    builder.insert_header((header::CACHE_CONTROL, "private, no-store"));
    builder
}

/// Plays a TIDAL or Qobuz track through Parson. The service's signed media
/// URL never reaches the client: Parson proxies the bytes, forwarding byte
/// ranges for progressive streams.
pub async fn stream_provider_track(
    request: &HttpRequest,
    provider: ProviderId,
    raw_id: String,
    bitrate: u32,
) -> HttpResponse {
    let user_id = match authenticated_user_id(request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let Some(hub) = request.app_data::<web::Data<StreamingHub>>() else {
        return hub_error_response(HubError::Provider(ProviderError::NotConnected));
    };
    let track_id = entity_id(provider, &raw_id);
    let stream = match hub
        .resolve_stream(user_id, provider, raw_id, bitrate > 0)
        .await
    {
        Ok(stream) => stream,
        Err(error) => return hub_error_response(error),
    };
    let range = request
        .headers()
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let probe_this = media::starts_at_beginning(range.as_deref());
    match &stream.source {
        StreamSource::SandboxTone { seed, seconds } => {
            let body = sandbox::preview_wav(*seed, *seconds, stream.requested);
            if probe_this {
                let probed = probe::probe(&body[..body.len().min(probe::PROBE_BYTES)]);
                hub.record_delivery(user_id, track_id, &stream, probed);
            }
            let mut response = serve_bytes(request, body, "audio/wav");
            for (name, value) in [
                ("x-parson-stream-format", stream.format.label.clone()),
                (
                    "x-parson-stream-quality",
                    stream.requested.as_str().to_string(),
                ),
            ] {
                if let Ok(value) = header::HeaderValue::from_str(&value) {
                    response
                        .headers_mut()
                        .insert(header::HeaderName::from_static(name), value);
                }
            }
            response
        }
        StreamSource::Progressive { url, mime } => {
            let response = match hub.media().open(url.clone(), range).await {
                Ok(response) => response,
                Err(MediaError::Status(status)) => {
                    return hub_error_response(HubError::Provider(ProviderError::Upstream(status)));
                }
                Err(error) => {
                    tracing::warn!(%error, "streaming media request failed");
                    return hub_error_response(HubError::Provider(ProviderError::Upstream(502)));
                }
            };
            let mut builder = format_headers(
                HttpResponse::build(
                    StatusCode::from_u16(response.status).unwrap_or(StatusCode::OK),
                ),
                &stream,
            );
            builder.insert_header((header::CONTENT_TYPE, mime.clone()));
            builder.insert_header((header::ACCEPT_RANGES, "bytes"));
            if let Some(length) = response.content_length {
                builder.insert_header((header::CONTENT_LENGTH, length.to_string()));
            }
            if let Some(range) = response.content_range {
                builder.insert_header((header::CONTENT_RANGE, range));
            }
            let body = if probe_this {
                recorded(
                    hub.clone(),
                    user_id,
                    track_id,
                    stream.clone(),
                    response.body,
                )
            } else {
                response.body
            };
            builder.streaming(body.map(|chunk| chunk.map_err(proxy_error)))
        }
        StreamSource::Segmented {
            mime,
            init_url,
            segment_urls,
        } => {
            let urls = init_url
                .iter()
                .cloned()
                .chain(segment_urls.iter().cloned())
                .collect::<Vec<_>>();
            let mut builder = format_headers(HttpResponse::Ok(), &stream);
            builder.insert_header((header::CONTENT_TYPE, mime.clone()));
            let body = recorded(
                hub.clone(),
                user_id,
                track_id,
                stream.clone(),
                media::segmented(hub.media(), urls),
            );
            builder.streaming(body.map(|chunk| chunk.map_err(proxy_error)))
        }
    }
}

/// Reads the delivered format from the bytes as they pass through.
fn recorded(
    hub: web::Data<StreamingHub>,
    user_id: i32,
    track_id: String,
    stream: ResolvedStream,
    body: media::MediaBody,
) -> media::MediaBody {
    media::tap(body, move |probed| {
        let delivery = hub.record_delivery(user_id, track_id, &stream, probed);
        if !delivery.matches_announced {
            tracing::info!(
                track = delivery.track_id,
                announced = delivery.announced.label,
                delivered = delivery
                    .delivered
                    .as_ref()
                    .map(|format| format.label.as_str()),
                "stream delivered a different format than announced"
            );
        }
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use actix_web::dev::Service;
    use actix_web::http::StatusCode;
    use actix_web::{App, HttpMessage, HttpRequest, test, web};
    use diesel::connection::SimpleConnection;
    use diesel::r2d2::{ConnectionManager, Pool};
    use diesel::sqlite::SqliteConnection;
    use diesel_migrations::MigrationHarness;
    use serde_json::{Value, json};

    use super::{byte_range, configure, stream_provider_track};
    use crate::api::auth::Claims;
    use crate::persistence::connection::DbPool;
    use crate::providers::hub::StreamingHub;
    use crate::providers::media::FixtureMedia;
    use crate::providers::transport::{FixtureReply, FixtureRoute, FixtureTransport, Method};
    use crate::providers::vault::TokenVault;
    use crate::providers::{ProviderId, parse_entity_id};

    fn pool() -> DbPool {
        let pool = Arc::new(
            Pool::builder()
                .max_size(1)
                .build(ConnectionManager::<SqliteConnection>::new(":memory:"))
                .expect("pool"),
        );
        let mut connection = pool.get().expect("connection");
        connection
            .run_pending_migrations(crate::persistence::connection::MIGRATIONS)
            .expect("migrations");
        connection
            .batch_execute(
                "PRAGMA foreign_keys = ON;
                 INSERT INTO user(id, username, password) VALUES (1, 'ada', 'x'), (2, 'grace', 'x');",
            )
            .expect("users");
        drop(connection);
        pool
    }

    fn hub(pool: DbPool, sandbox: bool) -> (StreamingHub, Arc<FixtureTransport>) {
        let transport = Arc::new(crate::providers::sandbox::transport());
        (
            StreamingHub::new(
                pool,
                TokenVault::new([5; 32]),
                transport.clone(),
                sandbox,
                HashMap::new(),
            ),
            transport,
        )
    }

    fn claims(user: i32) -> Claims {
        claims_as(user, "user")
    }

    fn claims_as(user: i32, role: &str) -> Claims {
        Claims {
            sub: user.to_string(),
            exp: usize::MAX,
            username: format!("user{user}"),
            bitrate: 0,
            token_type: "access".into(),
            role: role.into(),
            token_version: 0,
            session_id: None,
        }
    }

    async fn stream_route(
        request: HttpRequest,
        path: web::Path<String>,
    ) -> actix_web::HttpResponse {
        let (provider, raw) = parse_entity_id(&path).expect("provider id");
        stream_provider_track(&request, provider, raw, 0).await
    }

    macro_rules! app {
        ($hub:expr, $user:expr) => {
            test::init_service(
                App::new()
                    .app_data(web::Data::new($hub))
                    .wrap_fn(move |request, service| {
                        request.extensions_mut().insert(claims($user));
                        service.call(request)
                    })
                    .configure(configure)
                    .route("/media/songs/{song}/stream", web::get().to(stream_route)),
            )
            .await
        };
    }

    /// An app over a shared hub, for one user and role.
    macro_rules! app_as {
        ($data:expr, $user:expr, $role:expr) => {
            test::init_service(
                App::new()
                    .app_data($data.clone())
                    .wrap_fn(move |request, service| {
                        request.extensions_mut().insert(claims_as($user, $role));
                        service.call(request)
                    })
                    .configure(configure)
                    .route("/media/songs/{song}/stream", web::get().to(stream_route)),
            )
            .await
        };
    }

    macro_rules! call {
        ($app:expr, $request:expr $(,)?) => {
            async {
                let response = test::call_service($app, $request.to_request()).await;
                let status = response.status();
                let body = test::read_body(response).await;
                (
                    status,
                    serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null),
                )
            }
        };
    }

    #[actix_web::test]
    async fn tidal_device_sign_in_seals_tokens_and_reports_status() {
        let pool = pool();
        let (hub, _) = hub(pool.clone(), false);
        let app = app!(hub, 1);

        let (status, body) = call!(&app, test::TestRequest::get().uri("/providers")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["providers"][0]["id"], "tidal");
        assert_eq!(body["providers"][0]["credentials"]["configured"], false);
        assert_eq!(body["sandbox"], false);

        let (status, _) = call!(
            &app,
            test::TestRequest::post().uri("/providers/tidal/sign-in"),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "credentials come first");

        let (status, body) = call!(
            &app,
            test::TestRequest::put()
                .uri("/providers/tidal/credentials")
                .set_json(json!({"client_id": "my-client-id-1234", "client_secret": "s3cret"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["credentials"]["client_id_hint"], "•••• 1234");
        assert!(body.to_string().find("s3cret").is_none());

        let (status, body) = call!(
            &app,
            test::TestRequest::post().uri("/providers/tidal/sign-in"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["user_code"], "PRSN7");

        let (_, body) = call!(
            &app,
            test::TestRequest::post().uri("/providers/tidal/sign-in/poll"),
        )
        .await;
        assert_eq!(body["status"], "pending");
        assert_eq!(body["provider"]["status"], "pending");
        let (_, body) = call!(
            &app,
            test::TestRequest::post().uri("/providers/tidal/sign-in/poll"),
        )
        .await;
        assert_eq!(body["status"], "signed_in");
        assert_eq!(body["provider"]["account_name"], "recorded.listener");
        assert_eq!(body["provider"]["status"], "signed_in");

        #[derive(diesel::QueryableByName)]
        struct Sealed {
            #[diesel(sql_type = diesel::sql_types::Binary)]
            sealed_session: Vec<u8>,
            #[diesel(sql_type = diesel::sql_types::Binary)]
            sealed_credentials: Vec<u8>,
        }
        let mut connection = pool.get().expect("connection");
        let sealed: Sealed = diesel::RunQueryDsl::get_result(
            diesel::sql_query(
                "SELECT sealed_session, sealed_credentials FROM provider_account WHERE user_id = 1",
            ),
            &mut connection,
        )
        .expect("row");
        drop(connection);
        for secret in ["recorded-access-token", "recorded-refresh-token", "s3cret"] {
            for blob in [&sealed.sealed_session, &sealed.sealed_credentials] {
                assert!(
                    !blob
                        .windows(secret.len())
                        .any(|window| window == secret.as_bytes()),
                    "{secret} is stored sealed"
                );
            }
        }

        let other = app!(hub_for(pool.clone()), 2);
        let (_, body) = call!(&other, test::TestRequest::get().uri("/providers")).await;
        assert_eq!(
            body["providers"][0]["status"], "configured",
            "accounts are per user"
        );
    }

    fn hub_for(pool: DbPool) -> StreamingHub {
        hub(pool, false).0
    }

    async fn signed_in_app_state(pool: &DbPool) -> StreamingHub {
        let (hub, _) = hub(pool.clone(), false);
        hub.save_credentials(1, ProviderId::Tidal, Some("client".into()), None, false)
            .await
            .expect("tidal credentials");
        hub.start_tidal_sign_in(1).await.expect("start");
        hub.poll_tidal_sign_in(1).await.expect("pending");
        hub.poll_tidal_sign_in(1).await.expect("approved");
        hub.save_credentials(
            1,
            ProviderId::Qobuz,
            Some("100000001".into()),
            Some("app-secret".into()),
            false,
        )
        .await
        .expect("qobuz credentials");
        hub.qobuz_sign_in(1, "listener@example.test", "pw")
            .await
            .expect("qobuz sign-in");
        hub
    }

    #[actix_web::test]
    async fn add_to_library_shows_streaming_albums_beside_local_ones() {
        let pool = pool();
        let hub = signed_in_app_state(&pool).await;
        let app = app!(hub, 1);

        let (status, body) = call!(
            &app,
            test::TestRequest::get().uri("/providers/search?q=harbor+lights"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let services = body["services"].as_array().expect("services");
        assert_eq!(services.len(), 2, "both connected services answer");
        assert_eq!(services[0]["results"]["albums"][0]["in_library"], false);

        let (status, body) = call!(
            &app,
            test::TestRequest::put().uri("/providers/qobuz/library/album/q0harborhr04"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["in_library"], true);
        assert_eq!(body["service_updated"], true);

        let (_, body) = call!(
            &app,
            test::TestRequest::get().uri("/providers/library?provider=qobuz&kind=album"),
        )
        .await;
        assert_eq!(body["total"], 1);
        assert_eq!(body["items"][0]["id"], "qobuz:q0harborhr04");
        assert_eq!(body["items"][0]["format_label"], "24/192 FLAC");
        assert_eq!(body["items"][0]["item"]["title"], "Harbor Lights");

        let (_, body) = call!(
            &app,
            test::TestRequest::get().uri("/providers/library?q=harbor"),
        )
        .await;
        assert_eq!(body["total"], 1, "in-library search finds streaming albums");

        let (_, body) = call!(
            &app,
            test::TestRequest::get().uri("/providers/qobuz/albums/q0harborhr04"),
        )
        .await;
        assert_eq!(body["in_library"], true);
        assert_eq!(body["tracks"].as_array().map(Vec::len), Some(4));

        let (status, body) = call!(
            &app,
            test::TestRequest::delete().uri("/providers/qobuz/library/album/q0harborhr04"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["in_library"], false);
        let (_, body) = call!(&app, test::TestRequest::get().uri("/providers/library")).await;
        assert_eq!(body["total"], 0);
    }

    #[actix_web::test]
    async fn library_sync_imports_service_favorites() {
        let pool = pool();
        let hub = signed_in_app_state(&pool).await;
        let app = app!(hub, 1);
        let (status, body) = call!(
            &app,
            test::TestRequest::post().uri("/providers/tidal/library/sync"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["albums_added"], 2);
        assert_eq!(body["tracks_added"], 2);
        let (_, body) = call!(&app, test::TestRequest::get().uri("/providers")).await;
        assert_eq!(body["providers"][0]["library"]["albums"], 2);
        assert!(body["providers"][0]["last_library_sync"].as_i64().is_some());
    }

    #[actix_web::test]
    async fn quality_choices_are_validated_per_service() {
        let pool = pool();
        let hub = signed_in_app_state(&pool).await;
        let app = app!(hub, 1);
        let (status, body) = call!(
            &app,
            test::TestRequest::patch()
                .uri("/providers/qobuz")
                .set_json(json!({"quality": "hi_res_96"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["quality"], "hi_res_96");
        let (status, _) = call!(
            &app,
            test::TestRequest::patch()
                .uri("/providers/qobuz")
                .set_json(json!({"quality": "low"})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = call!(
            &app,
            test::TestRequest::patch()
                .uri("/providers/napster")
                .set_json(json!({"quality": "low"})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[actix_web::test]
    async fn qobuz_requires_an_app_secret_and_rejects_bad_logins() {
        let pool = pool();
        let (hub, transport) = hub(pool, false);
        transport.prepend(FixtureRoute {
            method: Method::Get,
            url: "https://www.qobuz.com/api.json/0.2/user/login".into(),
            query: vec![("username".into(), "wrong".into())],
            form: Vec::new(),
            replies: [FixtureReply {
                status: 401,
                body: json!({"status": "error", "code": 401, "message": "User authentication is required."}),
            }]
            .into(),
        });
        let app = app!(hub, 1);
        let (status, _) = call!(
            &app,
            test::TestRequest::put()
                .uri("/providers/qobuz/credentials")
                .set_json(json!({"client_id": "100000001"})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = call!(
            &app,
            test::TestRequest::put()
                .uri("/providers/qobuz/credentials")
                .set_json(json!({"client_id": "100000001", "client_secret": "secret"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, body) = call!(
            &app,
            test::TestRequest::post()
                .uri("/providers/qobuz/sign-in")
                .set_json(json!({"username": "wrong", "password": "nope"})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "provider_credentials_rejected");
        let (status, _) = call!(
            &app,
            test::TestRequest::put()
                .uri("/providers/qobuz/credentials")
                .set_json(json!({"use_server_credentials": true})),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "no server credentials configured"
        );
    }

    #[actix_web::test]
    async fn expired_tidal_tokens_refresh_once_and_retry() {
        let pool = pool();
        let (hub, transport) = hub(pool.clone(), false);
        hub.save_credentials(1, ProviderId::Tidal, Some("client".into()), None, false)
            .await
            .expect("credentials");
        hub.start_tidal_sign_in(1).await.expect("start");
        hub.poll_tidal_sign_in(1).await.expect("pending");
        hub.poll_tidal_sign_in(1).await.expect("approved");
        transport.prepend(FixtureRoute {
            method: Method::Get,
            url: "https://api.tidal.com/v1/albums/300200".into(),
            query: Vec::new(),
            form: Vec::new(),
            replies: [
                FixtureReply {
                    status: 401,
                    body: json!({"status": 401, "subStatus": 11003, "userMessage": "expired"}),
                },
                FixtureReply {
                    status: 200,
                    body: json!({"id": 300200, "title": "Paper Moons", "artist": {"id": 400100, "name": "Mara Vell"}}),
                },
            ]
            .into(),
        });
        let app = app!(hub, 1);
        let (status, body) = call!(
            &app,
            test::TestRequest::get().uri("/providers/tidal/albums/300200"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["title"], "Paper Moons");
        let requests = transport.requests();
        let refresh = requests
            .iter()
            .find(|request| request.form_value("grant_type") == Some("refresh_token"))
            .expect("refresh request");
        assert_eq!(
            refresh.form_value("refresh_token"),
            Some("recorded-refresh-token")
        );
        let retried = requests
            .iter()
            .rfind(|request| request.url.ends_with("/albums/300200"))
            .expect("retry");
        assert_eq!(
            retried.header_value("authorization"),
            Some("Bearer recorded-access-token-2")
        );
    }

    #[actix_web::test]
    async fn signed_out_services_answer_with_a_clear_conflict() {
        let pool = pool();
        let app = app!(hub_for(pool), 1);
        let (status, body) = call!(
            &app,
            test::TestRequest::get().uri("/providers/tidal/albums/300100"),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["code"], "provider_not_connected");
        let (status, _) = call!(
            &app,
            test::TestRequest::get().uri("/providers/tidal/albums/..%2Fsecret"),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[actix_web::test]
    async fn sandbox_streams_serve_ranged_preview_audio() {
        let pool = pool();
        let (hub, _) = hub(pool, true);
        hub.save_credentials(
            1,
            ProviderId::Qobuz,
            Some("sandbox".into()),
            Some("sandbox".into()),
            false,
        )
        .await
        .expect("credentials");
        hub.qobuz_sign_in(1, "sandbox", "sandbox")
            .await
            .expect("sign in");
        let app = app!(hub, 1);
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/songs/qobuz:7700001/stream")
                .insert_header(("range", "bytes=0-99"))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            response
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok()),
            Some(format!(
                "bytes 0-99/{}",
                44 + 2 * 44_100 * crate::providers::sandbox::PREVIEW_SECONDS
            ))
            .as_deref()
        );
        assert_eq!(
            response
                .headers()
                .get("x-parson-stream-format")
                .and_then(|v| v.to_str().ok()),
            Some("Sandbox preview")
        );
        let body = test::read_body(response).await;
        assert_eq!(&body[..4], b"RIFF");

        let (status, body) = call!(
            &app,
            test::TestRequest::get().uri("/providers/search?q=kites"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let albums = body["services"][0]["results"]["albums"]
            .as_array()
            .expect("albums");
        assert_eq!(albums.len(), 1, "sandbox search filters recorded results");
        assert!(
            albums[0]["cover_url"]
                .as_str()
                .is_some_and(|url| url.starts_with("data:image/svg+xml"))
        );
    }

    const QOBUZ_HIRES_URL: &str =
        "https://streaming-qobuz-std.akamaized.test/file?uid=recorded&fmt=27";
    const HIRES_192_FLAC: &[u8] = include_bytes!("../providers/fixtures/media/hires-192.flac");
    const HIRES_96_DASH: &[u8] = include_bytes!("../providers/fixtures/media/hires-96-dash.mp4");

    /// Splits a fragmented MP4 into its init segment and its media
    /// fragments, as a DASH packager serves them.
    fn split_init_segment(file: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let mut at = 0;
        while at + 8 <= file.len() {
            let size = u32::from_be_bytes(file[at..at + 4].try_into().expect("size")) as usize;
            let kind = &file[at + 4..at + 8];
            at += size;
            if kind == b"moov" {
                break;
            }
        }
        (file[..at].to_vec(), file[at..].to_vec())
    }

    fn tidal_dash_media() -> Arc<FixtureMedia> {
        let (init, fragments) = split_init_segment(HIRES_96_DASH);
        let base = "https://sp-recorded.audio.tidal.test/hr/300101";
        let mut media = FixtureMedia::new()
            .with(&format!("{base}/0.mp4?token=a&sig=b"), init)
            .with(&format!("{base}/1.mp4?token=a&sig=b"), fragments);
        for number in 2..=4 {
            media = media.with(&format!("{base}/{number}.mp4?token=a&sig=b"), Vec::new());
        }
        Arc::new(media)
    }

    #[actix_web::test]
    async fn server_credentials_let_subscribers_skip_the_credentials_step() {
        let pool = pool();
        let data = web::Data::new(hub(pool.clone(), false).0);
        let admin = app_as!(data, 1, "admin");
        let listener = app_as!(data, 2, "user");

        let (status, _) = call!(
            &listener,
            test::TestRequest::get().uri("/providers/server-credentials"),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = call!(
            &listener,
            test::TestRequest::put()
                .uri("/providers/qobuz/server-credentials")
                .set_json(json!({"client_id": "100000001", "client_secret": "x"})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "only the owner sets them");

        let (status, body) = call!(
            &admin,
            test::TestRequest::put()
                .uri("/providers/qobuz/server-credentials")
                .set_json(json!({"client_id": "100000001", "client_secret": "server-app-secret"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(!body.to_string().contains("server-app-secret"));
        let qobuz = &body["providers"][1];
        assert_eq!(qobuz["provider"], "qobuz");
        assert_eq!(qobuz["configured"], true);
        assert_eq!(qobuz["source"], "settings");
        assert_eq!(qobuz["client_id_hint"], "•••• 0001");
        assert_eq!(qobuz["has_secret"], true);
        let (status, _) = call!(
            &admin,
            test::TestRequest::put()
                .uri("/providers/tidal/server-credentials")
                .set_json(json!({"client_id": "server-tidal-client"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // A listener with no credentials of their own goes straight to the
        // service sign-in.
        let (_, body) = call!(&listener, test::TestRequest::get().uri("/providers")).await;
        assert_eq!(body["providers"][1]["server_credentials_available"], true);
        assert_eq!(body["providers"][1]["credentials"]["configured"], false);
        let (status, body) = call!(
            &listener,
            test::TestRequest::post()
                .uri("/providers/qobuz/sign-in")
                .set_json(json!({"username": "listener@example.test", "password": "pw"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["status"], "signed_in");
        assert_eq!(body["credentials"]["uses_server_credentials"], true);
        assert_eq!(body["credentials"]["client_id_hint"], Value::Null);
        let (status, body) = call!(
            &listener,
            test::TestRequest::post().uri("/providers/tidal/sign-in"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            body["user_code"]
                .as_str()
                .is_some_and(|code| !code.is_empty())
        );

        // Sealed at rest, and loaded again after a restart.
        let blobs: Vec<Vec<u8>> = {
            use diesel::RunQueryDsl;
            #[derive(diesel::QueryableByName)]
            struct Blob {
                #[diesel(sql_type = diesel::sql_types::Binary)]
                sealed_credentials: Vec<u8>,
            }
            let mut connection = pool.get().expect("connection");
            diesel::sql_query("SELECT sealed_credentials FROM provider_server_credential")
                .load::<Blob>(&mut connection)
                .expect("rows")
                .into_iter()
                .map(|row| row.sealed_credentials)
                .collect()
        };
        assert_eq!(blobs.len(), 2);
        for blob in &blobs {
            for secret in [&b"server-app-secret"[..], b"server-tidal-client"] {
                assert!(!blob.windows(secret.len()).any(|window| window == secret));
            }
        }
        let restarted = hub(pool.clone(), false).0;
        restarted.load_server_credentials().expect("load");
        assert!(
            restarted
                .server_credentials_status()
                .iter()
                .all(|status| status.configured)
        );

        // Removing them leaves listeners who relied on them disconnected,
        // not silently on someone else's credentials.
        let (status, body) = call!(
            &admin,
            test::TestRequest::delete().uri("/providers/qobuz/server-credentials"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["providers"][1]["configured"], false);
        let (status, body) = call!(
            &listener,
            test::TestRequest::get().uri("/providers/qobuz/search?q=kites"),
        )
        .await;
        assert_ne!(status, StatusCode::OK, "{body}");
        let (status, body) = call!(
            &listener,
            test::TestRequest::post()
                .uri("/providers/qobuz/sign-in")
                .set_json(json!({"username": "listener@example.test", "password": "pw"})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
    }

    #[actix_web::test]
    async fn environment_credentials_cannot_be_overridden_from_settings() {
        let pool = pool();
        let environment = HashMap::from([(
            ProviderId::Tidal,
            crate::providers::ClientCredentials {
                client_id: "deployment-client".into(),
                client_secret: None,
            },
        )]);
        let hub = StreamingHub::new(
            pool,
            TokenVault::new([5; 32]),
            Arc::new(crate::providers::sandbox::transport()),
            false,
            environment,
        );
        let data = web::Data::new(hub);
        let admin = app_as!(data, 1, "admin");
        let (_, body) = call!(
            &admin,
            test::TestRequest::get().uri("/providers/server-credentials"),
        )
        .await;
        assert_eq!(body["providers"][0]["source"], "environment");
        for request in [
            test::TestRequest::put()
                .uri("/providers/tidal/server-credentials")
                .set_json(json!({"client_id": "other"})),
            test::TestRequest::delete().uri("/providers/tidal/server-credentials"),
        ] {
            let (status, _) = call!(&admin, request).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
        }
    }

    #[actix_web::test]
    async fn qobuz_hi_res_streams_report_the_format_read_from_the_bytes() {
        let pool = pool();
        let media = Arc::new(FixtureMedia::new().with(QOBUZ_HIRES_URL, HIRES_192_FLAC.to_vec()));
        let hub = signed_in_app_state(&pool).await.with_media(media.clone());
        let app = app!(hub, 1);

        let (status, _) = call!(
            &app,
            test::TestRequest::get().uri("/providers/qobuz/tracks/7700001/delivery"),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "nothing streamed yet");

        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/songs/qobuz:7700001/stream")
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("x-parson-stream-format")
                .and_then(|value| value.to_str().ok()),
            Some("24/192 FLAC")
        );
        let body = test::read_body(response).await;
        assert_eq!(&body[..], HIRES_192_FLAC, "bytes pass through unchanged");
        assert_eq!(media.requests(), vec![(QOBUZ_HIRES_URL.to_string(), None)]);

        let (status, delivery) = call!(
            &app,
            test::TestRequest::get().uri("/providers/qobuz/tracks/7700001/delivery"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(delivery["requested"], "hi_res_max");
        assert_eq!(delivery["announced"]["label"], "24/192 FLAC");
        assert_eq!(delivery["delivered"]["label"], "24/192 FLAC");
        assert_eq!(delivery["delivered"]["bit_depth"], 24);
        assert_eq!(delivery["delivered"]["sample_rate"], 192_000);
        assert_eq!(delivery["delivered"]["channels"], 2);
        assert_eq!(delivery["container"], "flac");
        assert_eq!(delivery["frame_verified"], true);
        assert_eq!(delivery["matches_announced"], true);

        // Seeking forwards the range and keeps the delivered format.
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/songs/qobuz:7700001/stream")
                .insert_header(("range", "bytes=1000-1999"))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            response
                .headers()
                .get("content-range")
                .and_then(|value| value.to_str().ok()),
            Some(format!("bytes 1000-1999/{}", HIRES_192_FLAC.len()).as_str())
        );
        assert_eq!(
            &test::read_body(response).await[..],
            &HIRES_192_FLAC[1000..2000]
        );
        let (_, delivery) = call!(
            &app,
            test::TestRequest::get().uri("/providers/qobuz/tracks/7700001/delivery"),
        )
        .await;
        assert_eq!(delivery["delivered"]["label"], "24/192 FLAC");
    }

    #[actix_web::test]
    async fn tidal_max_dash_streams_are_read_through_the_proxy() {
        let pool = pool();
        let hub = signed_in_app_state(&pool)
            .await
            .with_media(tidal_dash_media());
        let app = app!(hub, 1);
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/songs/tidal:300101/stream")
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = test::read_body(response).await;
        assert_eq!(&body[..], HIRES_96_DASH, "segments play back to back");
        let (_, delivery) = call!(
            &app,
            test::TestRequest::get().uri("/providers/tidal/tracks/300101/delivery"),
        )
        .await;
        assert_eq!(delivery["requested"], "hi_res_max");
        assert_eq!(delivery["container"], "mp4");
        assert_eq!(delivery["delivered"]["codec"], "flac");
        assert_eq!(delivery["delivered"]["label"], "24/96 FLAC");
        assert_eq!(delivery["frame_verified"], true);
        assert_eq!(delivery["matches_announced"], true);
    }

    #[actix_web::test]
    async fn mismatched_streams_show_what_actually_arrived() {
        use crate::providers::probe::tests::{flac_file, frame_header};
        let pool = pool();
        let cd = flac_file(44_100, 2, 16, &frame_header(9, 1, 4));
        let media = Arc::new(FixtureMedia::new().with(QOBUZ_HIRES_URL, cd));
        let hub = signed_in_app_state(&pool).await.with_media(media);
        let app = app!(hub, 1);
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/songs/qobuz:7700001/stream")
                .to_request(),
        )
        .await;
        test::read_body(response).await;
        let (_, delivery) = call!(
            &app,
            test::TestRequest::get().uri("/providers/qobuz/tracks/7700001/delivery"),
        )
        .await;
        assert_eq!(delivery["announced"]["label"], "24/192 FLAC");
        assert_eq!(delivery["delivered"]["label"], "16/44.1 FLAC");
        assert_eq!(delivery["matches_announced"], false);
    }

    #[actix_web::test]
    async fn connection_check_signs_in_syncs_favorites_and_reads_a_stream() {
        let pool = pool();
        let media = Arc::new(FixtureMedia::new().with(QOBUZ_HIRES_URL, HIRES_192_FLAC.to_vec()));
        let hub = signed_in_app_state(&pool).await.with_media(media.clone());
        let data = web::Data::new(hub);
        let app = app_as!(data, 1, "user");
        let (status, check) = call!(
            &app,
            test::TestRequest::post().uri("/providers/qobuz/check"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{check}");
        assert_eq!(check["ok"], true, "{check}");
        let steps = check["steps"].as_array().expect("steps");
        assert_eq!(
            steps
                .iter()
                .map(|step| step["id"].clone())
                .collect::<Vec<_>>(),
            vec![json!("session"), json!("favorites"), json!("stream")]
        );
        assert!(steps.iter().all(|step| step["ok"] == true));
        assert!(
            steps[0]["detail"]
                .as_str()
                .is_some_and(|detail| detail.contains("Recorded Listener"))
        );
        assert_eq!(check["delivery"]["delivered"]["label"], "24/192 FLAC");
        assert!(
            steps[2]["detail"]
                .as_str()
                .is_some_and(|detail| detail.contains("first audio frame verified"))
        );
        // The check reads only the start of the stream.
        assert_eq!(
            media.requests(),
            vec![(
                QOBUZ_HIRES_URL.to_string(),
                Some(format!(
                    "bytes=0-{}",
                    crate::providers::probe::PROBE_BYTES - 1
                ))
            )]
        );

        let (status, check) = call!(
            &app,
            test::TestRequest::post()
                .uri("/providers/qobuz/check")
                .set_json(json!({"track_id": "tidal:300101"})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{check}");

        let other = app_as!(data, 2, "user");
        let (status, check) = call!(
            &other,
            test::TestRequest::post().uri("/providers/qobuz/check"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(check["ok"], false);
        assert_eq!(check["steps"].as_array().map(Vec::len), Some(1));
        assert_eq!(check["steps"][0]["ok"], false);
    }

    #[actix_web::test]
    async fn sandbox_checks_say_the_preview_is_not_a_real_stream() {
        let pool = pool();
        let (hub, _) = hub(pool, true);
        hub.save_credentials(
            1,
            ProviderId::Qobuz,
            Some("sandbox".into()),
            Some("sandbox".into()),
            false,
        )
        .await
        .expect("credentials");
        hub.qobuz_sign_in(1, "sandbox", "sandbox")
            .await
            .expect("sign in");
        let check = hub
            .check_connection(1, ProviderId::Qobuz, None)
            .await
            .expect("check");
        let delivery = check.delivery.expect("delivery");
        assert!(delivery.sandbox);
        assert!(!delivery.matches_announced);
        assert_eq!(
            delivery.delivered.map(|format| format.label),
            Some("16/44.1 PCM".to_string())
        );
        assert!(check.steps[2].detail.contains("Sandbox preview tone"));
    }

    #[actix_web::test]
    async fn service_playlists_are_read_only_with_saved_copies() {
        let pool = pool();
        let (hub, transport) = hub(pool.clone(), false);
        let hub = {
            let hub = hub;
            hub.save_credentials(1, ProviderId::Tidal, Some("client".into()), None, false)
                .await
                .expect("tidal credentials");
            hub.start_tidal_sign_in(1).await.expect("start");
            hub.poll_tidal_sign_in(1).await.expect("pending");
            hub.poll_tidal_sign_in(1).await.expect("approved");
            hub.save_credentials(
                1,
                ProviderId::Qobuz,
                Some("100000001".into()),
                Some("app-secret".into()),
                false,
            )
            .await
            .expect("qobuz credentials");
            hub.qobuz_sign_in(1, "listener@example.test", "pw")
                .await
                .expect("qobuz sign-in");
            hub
        };
        let app = app!(hub, 1);

        let (status, body) = call!(
            &app,
            test::TestRequest::get().uri("/providers/tidal/playlists"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let lists = body["playlists"].as_array().expect("playlists");
        assert_eq!(lists.len(), 2);
        assert_eq!(lists[0]["title"], "Late Night Hi-Res");
        assert_eq!(lists[0]["own"], true);
        assert_eq!(lists[0]["track_count"], 4);
        assert_eq!(lists[1]["owner"], "TIDAL");
        assert_eq!(lists[1]["own"], false);
        assert_eq!(lists[0]["saved_copy_at"], Value::Null);
        let id = lists[0]["id"].as_str().expect("id").to_string();
        let raw = id.strip_prefix("tidal:").expect("tidal id").to_string();

        let (status, body) = call!(
            &app,
            test::TestRequest::get().uri(&format!("/providers/tidal/playlists/{raw}")),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let tracks = body["tracks"].as_array().expect("tracks");
        assert_eq!(tracks.len(), 4);
        assert_eq!(tracks[0]["id"], "tidal:300101");
        assert_eq!(tracks[0]["title"], "Harbor Lights");

        let (status, copy) = call!(
            &app,
            test::TestRequest::put().uri(&format!("/providers/tidal/playlists/{raw}/copy")),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{copy}");
        assert_eq!(copy["title"], "Late Night Hi-Res");
        assert_eq!(copy["track_count"], 4);
        assert!(
            transport.requests().iter().all(
                |request| request.method == Method::Get || !request.url.contains("/playlists/")
            ),
            "Parson never writes to service playlists"
        );

        let (_, body) = call!(
            &app,
            test::TestRequest::get().uri("/providers/playlist-copies"),
        )
        .await;
        assert_eq!(body["copies"].as_array().map(Vec::len), Some(1));
        assert!(
            body["copies"][0].get("playlist").is_none(),
            "lists stay light"
        );
        let (_, body) = call!(
            &app,
            test::TestRequest::get().uri("/providers/tidal/playlists"),
        )
        .await;
        assert!(body["playlists"][0]["saved_copy_at"].as_i64().is_some());

        // The copy keeps its tracks when the service playlist disappears.
        transport.prepend(FixtureRoute {
            method: Method::Get,
            url: format!("https://api.tidal.com/v1/playlists/{raw}"),
            query: Vec::new(),
            form: Vec::new(),
            replies: [FixtureReply {
                status: 404,
                body: json!({"status": 404}),
            }]
            .into(),
        });
        let (status, _) = call!(
            &app,
            test::TestRequest::get().uri(&format!("/providers/tidal/playlists/{raw}")),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, copy) = call!(
            &app,
            test::TestRequest::get().uri(&format!("/providers/tidal/playlists/{raw}/copy")),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(copy["playlist"]["tracks"].as_array().map(Vec::len), Some(4));

        let (status, body) = call!(
            &app,
            test::TestRequest::get().uri("/providers/qobuz/playlists"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["playlists"][0]["title"], "Hi-Res Mornings");
        assert_eq!(body["playlists"][0]["own"], true);
        let (status, body) = call!(
            &app,
            test::TestRequest::get().uri("/providers/qobuz/playlists/9100001"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["tracks"].as_array().map(Vec::len), Some(4));
        assert_eq!(body["tracks"][0]["format"]["label"], "24/192 FLAC");

        let response = test::call_service(
            &app,
            test::TestRequest::delete()
                .uri(&format!("/providers/tidal/playlists/{raw}/copy"))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let (status, _) = call!(
            &app,
            test::TestRequest::get().uri(&format!("/providers/tidal/playlists/{raw}/copy")),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    fn live(name: &str) -> Option<String> {
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
    }

    /// End-to-end check against real accounts. No accounts exist in CI, so
    /// it is ignored by default. Run it with your own subscription:
    ///
    /// ```sh
    /// PARSON_LIVE_QOBUZ_APP_ID=… PARSON_LIVE_QOBUZ_APP_SECRET=… \
    /// PARSON_LIVE_QOBUZ_EMAIL=… PARSON_LIVE_QOBUZ_PASSWORD=… \
    /// PARSON_LIVE_TIDAL_CLIENT_ID=… \
    ///   cargo test -p parson-music live_accounts -- --ignored --nocapture
    /// ```
    ///
    /// Each service signs in (TIDAL prints a link to approve within five
    /// minutes), syncs favorites, resolves a favorite at the highest quality
    /// and reads the delivered format from the real stream's bytes.
    #[actix_web::test]
    #[ignore = "needs real TIDAL or Qobuz accounts; see the doc comment"]
    async fn live_accounts_sign_in_sync_favorites_and_stream_hi_res() {
        let pool = pool();
        let transport =
            Arc::new(crate::providers::transport::ReqwestTransport::new().expect("http client"));
        let hub = StreamingHub::new(
            pool,
            TokenVault::new([5; 32]),
            transport,
            false,
            HashMap::new(),
        )
        .with_media(Arc::new(
            crate::providers::media::ReqwestMedia::new().expect("media client"),
        ));
        let mut ran = false;
        if let (Some(app), Some(secret), Some(email), Some(password)) = (
            live("PARSON_LIVE_QOBUZ_APP_ID"),
            live("PARSON_LIVE_QOBUZ_APP_SECRET"),
            live("PARSON_LIVE_QOBUZ_EMAIL"),
            live("PARSON_LIVE_QOBUZ_PASSWORD"),
        ) {
            ran = true;
            hub.save_credentials(1, ProviderId::Qobuz, Some(app), Some(secret), false)
                .await
                .expect("qobuz credentials");
            hub.qobuz_sign_in(1, &email, &password)
                .await
                .expect("qobuz sign-in");
            let check = hub
                .check_connection(1, ProviderId::Qobuz, live("PARSON_LIVE_QOBUZ_TRACK"))
                .await
                .expect("qobuz check");
            println!(
                "{}",
                serde_json::to_string_pretty(&check).unwrap_or_default()
            );
            assert!(check.ok, "Qobuz check failed: {:?}", check.steps);
            let delivered = check.delivery.and_then(|delivery| delivery.delivered);
            assert!(delivered.is_some_and(|format| format.lossless));
        }
        if let Some(client) = live("PARSON_LIVE_TIDAL_CLIENT_ID") {
            ran = true;
            hub.save_credentials(
                1,
                ProviderId::Tidal,
                Some(client),
                live("PARSON_LIVE_TIDAL_CLIENT_SECRET"),
                false,
            )
            .await
            .expect("tidal credentials");
            let pending = hub.start_tidal_sign_in(1).await.expect("device code");
            println!(
                "Approve Parson on TIDAL: {} (code {})",
                pending
                    .verification_uri_complete
                    .as_deref()
                    .unwrap_or(&pending.verification_uri),
                pending.user_code
            );
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
            loop {
                let outcome = hub.poll_tidal_sign_in(1).await.expect("poll");
                if outcome.status == "signed_in" {
                    break;
                }
                assert_eq!(outcome.status, "pending", "TIDAL sign-in ended");
                assert!(std::time::Instant::now() < deadline, "not approved in time");
                tokio::time::sleep(std::time::Duration::from_secs(outcome.interval)).await;
            }
            let check = hub
                .check_connection(1, ProviderId::Tidal, live("PARSON_LIVE_TIDAL_TRACK"))
                .await
                .expect("tidal check");
            println!(
                "{}",
                serde_json::to_string_pretty(&check).unwrap_or_default()
            );
            assert!(check.ok, "TIDAL check failed: {:?}", check.steps);
        }
        assert!(
            ran,
            "set PARSON_LIVE_QOBUZ_* or PARSON_LIVE_TIDAL_CLIENT_ID"
        );
    }

    #[actix_web::test]
    async fn byte_ranges_follow_rfc_7233() {
        assert_eq!(byte_range(Some("bytes=0-99"), 1000), Some((0, 99)));
        assert_eq!(byte_range(Some("bytes=900-"), 1000), Some((900, 999)));
        assert_eq!(byte_range(Some("bytes=-100"), 1000), Some((900, 999)));
        assert_eq!(byte_range(Some("bytes=0-5000"), 1000), Some((0, 999)));
        assert_eq!(byte_range(Some("bytes=1000-"), 1000), None);
        assert_eq!(byte_range(Some("bytes=0-1,4-5"), 1000), None);
        assert_eq!(byte_range(None, 1000), None);
    }
}
