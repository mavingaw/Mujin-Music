//! `/providers`: TIDAL and Qobuz sign-in, catalog, library and streaming.

use std::sync::OnceLock;
use std::time::Duration;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, delete, get, patch, post, put, web};
use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::api::auth::authenticated_user_id;
use crate::api::error::json_error;
use crate::providers::hub::{HubError, LibraryRequest, StreamingHub, user_message};
use crate::providers::store::{LibraryItemRow, LibrarySort};
use crate::providers::{
    BrowseSection, ItemKind, ProviderError, ProviderId, ResolvedStream, StreamQuality,
    StreamSource, entity_id, sandbox, valid_raw_id,
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

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/providers")
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

fn media_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(6))
            .read_timeout(Duration::from_secs(30))
            .user_agent(concat!("Parson/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default()
    })
}

/// Parses a single `bytes=` range against a known length.
fn byte_range(header: Option<&str>, length: u64) -> Option<(u64, u64)> {
    let spec = header?.trim().strip_prefix("bytes=")?;
    if spec.contains(',') || length == 0 {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    let (start, end) = if start.is_empty() {
        let suffix = end.parse::<u64>().ok()?.min(length);
        (length - suffix, length - 1)
    } else {
        let start = start.parse::<u64>().ok()?;
        let end = if end.is_empty() {
            length - 1
        } else {
            end.parse::<u64>().ok()?.min(length - 1)
        };
        (start, end)
    };
    (start <= end && start < length).then_some((start, end))
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

fn chunk_stream(
    response: reqwest::Response,
) -> impl futures::Stream<Item = Result<Bytes, actix_web::Error>> {
    futures::stream::unfold(Some(response), |state| async move {
        let mut response = state?;
        match response.chunk().await {
            Ok(Some(chunk)) => Some((Ok(chunk), Some(response))),
            Ok(None) => None,
            Err(error) => Some((
                Err(actix_web::error::ErrorBadGateway(error.without_url())),
                None,
            )),
        }
    })
}

fn segmented_stream(
    urls: Vec<String>,
) -> impl futures::Stream<Item = Result<Bytes, actix_web::Error>> {
    futures::stream::unfold(
        (urls.into_iter(), None::<reqwest::Response>),
        |(mut urls, mut current)| async move {
            loop {
                if let Some(mut response) = current.take() {
                    match response.chunk().await {
                        Ok(Some(chunk)) => return Some((Ok(chunk), (urls, Some(response)))),
                        Ok(None) => {}
                        Err(error) => {
                            return Some((
                                Err(actix_web::error::ErrorBadGateway(error.without_url())),
                                (urls, None),
                            ));
                        }
                    }
                }
                let url = urls.next()?;
                match media_client().get(&url).send().await {
                    Ok(response) if response.status().is_success() => current = Some(response),
                    Ok(response) => {
                        return Some((
                            Err(actix_web::error::ErrorBadGateway(format!(
                                "segment answered HTTP {}",
                                response.status().as_u16()
                            ))),
                            (Vec::new().into_iter(), None),
                        ));
                    }
                    Err(error) => {
                        return Some((
                            Err(actix_web::error::ErrorBadGateway(error.without_url())),
                            (Vec::new().into_iter(), None),
                        ));
                    }
                }
            }
        },
    )
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
    let stream = match hub
        .resolve_stream(user_id, provider, raw_id, bitrate > 0)
        .await
    {
        Ok(stream) => stream,
        Err(error) => return hub_error_response(error),
    };
    match &stream.source {
        StreamSource::SandboxTone { seed, seconds } => {
            let body = sandbox::preview_wav(*seed, *seconds, stream.requested);
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
            let mut upstream = media_client().get(url);
            if let Some(range) = request.headers().get(header::RANGE) {
                upstream = upstream.header(header::RANGE.as_str(), range.as_bytes());
            }
            let response = match upstream.send().await {
                Ok(response) => response,
                Err(error) => {
                    tracing::warn!(error = %error.without_url(), "streaming media request failed");
                    return hub_error_response(HubError::Provider(ProviderError::Upstream(502)));
                }
            };
            let status = response.status().as_u16();
            if !(200..300).contains(&status) {
                return hub_error_response(HubError::Provider(ProviderError::Upstream(status)));
            }
            let mut builder = format_headers(
                HttpResponse::build(StatusCode::from_u16(status).unwrap_or(StatusCode::OK)),
                &stream,
            );
            builder.insert_header((header::CONTENT_TYPE, mime.clone()));
            builder.insert_header((header::ACCEPT_RANGES, "bytes"));
            for name in [header::CONTENT_LENGTH, header::CONTENT_RANGE] {
                if let Some(value) = response.headers().get(name.as_str())
                    && let Ok(value) = header::HeaderValue::from_bytes(value.as_bytes())
                {
                    builder.insert_header((name, value));
                }
            }
            builder.streaming(chunk_stream(response))
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
            builder.streaming(segmented_stream(urls))
        }
    }
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
        Claims {
            sub: user.to_string(),
            exp: usize::MAX,
            username: format!("user{user}"),
            bitrate: 0,
            token_type: "access".into(),
            role: "user".into(),
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
