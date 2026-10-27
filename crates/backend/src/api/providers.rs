//! `/providers`: TIDAL and Qobuz sign-in, catalog, library and streaming.

use actix_web::http::{StatusCode, header};
use actix_web::{HttpMessage, HttpRequest, HttpResponse, delete, get, patch, post, put, web};
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use crate::api::auth::{Claims, authenticated_user_id};
use crate::api::error::{forbidden, json_error};
use crate::library::state::LibraryLifecycle;
use crate::providers::catalog::{
    self, CatalogFilter, CatalogPage, CatalogSort, CatalogSource, QualityTier,
};
use crate::providers::hub::{HubError, LibraryRequest, StreamingHub, user_message};
pub(crate) use crate::providers::media::byte_range;
use crate::providers::media::{self, MediaError};
use crate::providers::store::{LibraryItemRow, LibrarySort};
use crate::providers::{
    BrowseSection, ItemKind, ProviderError, ProviderId, ResolvedStream, SourcePreference,
    StreamQuality, StreamSource, entity_id, probe, sandbox, valid_raw_id,
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
        HubError::Conflict(_) => (StatusCode::CONFLICT, "provider_copy_edited"),
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

#[derive(Serialize, Deserialize)]
struct SourcePreferenceBody {
    source: String,
}

/// Which copy plays when an album is in the library and on a service.
#[get("/preferences/source")]
async fn get_source_preference(request: HttpRequest, hub: web::Data<StreamingHub>) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    respond(
        hub.source_preference(user_id)
            .await
            .map(|preference| SourcePreferenceBody {
                source: preference.as_str().to_string(),
            }),
    )
}

#[put("/preferences/source")]
async fn set_source_preference(
    request: HttpRequest,
    body: web::Json<SourcePreferenceBody>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let Some(preference) = SourcePreference::parse(&body.source) else {
        return hub_error_response(HubError::Invalid(
            "Choose highest_quality, local, tidal or qobuz.",
        ));
    };
    respond(
        hub.set_source_preference(user_id, preference)
            .await
            .map(|preference| SourcePreferenceBody {
                source: preference.as_str().to_string(),
            }),
    )
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

#[derive(Deserialize)]
struct BrowserSignInStart {
    redirect_uri: String,
}

/// Starts a TIDAL browser sign-in. The answer's `authorize_url` is where the
/// listener approves; TIDAL then sends them to `redirect_uri`.
#[post("/tidal/sign-in/browser")]
async fn start_tidal_browser_sign_in(
    request: HttpRequest,
    body: web::Json<BrowserSignInStart>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let origins = server_origins(&request);
    respond(
        hub.start_tidal_browser_sign_in(user_id, &body.redirect_uri, &origins)
            .await,
    )
}

/// The addresses this server answers on, for checking a redirect URI: the
/// host the browser asked for (over http or https, since a TLS proxy may sit
/// in front) and the configured public and development origins. The `Host`
/// header is what the listener's own browser sent, so a page elsewhere
/// cannot choose it.
fn server_origins(request: &HttpRequest) -> Vec<String> {
    let mut origins = crate::settings::allowed_origins();
    let host = request
        .headers()
        .get(actix_web::http::header::HOST)
        .and_then(|host| host.to_str().ok())
        .map(str::to_string)
        .or_else(|| {
            request
                .uri()
                .authority()
                .map(|authority| authority.to_string())
        });
    if let Some(host) = host.filter(|host| !host.is_empty()) {
        origins.push(format!("http://{host}"));
        origins.push(format!("https://{host}"));
    }
    origins
}

#[derive(Deserialize)]
struct BrowserSignInFinish {
    state: String,
    code: String,
}

#[post("/tidal/sign-in/browser/finish")]
async fn finish_tidal_browser_sign_in(
    request: HttpRequest,
    body: web::Json<BrowserSignInFinish>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    respond(
        hub.finish_tidal_browser_sign_in(user_id, &body.state, &body.code)
            .await,
    )
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

#[derive(Serialize)]
struct GenresResponse {
    genres: Vec<crate::providers::ProviderGenre>,
}

#[get("/{provider}/genres")]
async fn genres(
    request: HttpRequest,
    path: web::Path<String>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let (user_id, provider) = user_and_provider!(request, &path);
    respond(
        hub.genres(user_id, provider)
            .await
            .map(|genres| GenresResponse { genres }),
    )
}

#[derive(Deserialize)]
struct GenreAlbumsParameters {
    section: Option<String>,
    offset: Option<u32>,
    limit: Option<u32>,
}

/// A genre's albums, optionally from one editorial list (Qobuz).
#[get("/{provider}/genres/{genre}/albums")]
async fn genre_albums(
    request: HttpRequest,
    path: web::Path<(String, String)>,
    query: web::Query<GenreAlbumsParameters>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let (user_id, provider) = user_and_provider!(request, &path.0);
    if !valid_raw_id(&path.1) {
        return hub_error_response(HubError::Invalid("Unknown genre."));
    }
    let section = match query.section.as_deref() {
        None | Some("") => BrowseSection::NewReleases,
        Some(value) => match BrowseSection::parse(value).filter(|section| section.is_editorial()) {
            Some(section) => section,
            None => return hub_error_response(HubError::Invalid("Unknown browse section.")),
        },
    };
    respond(
        hub.genre_albums(
            user_id,
            provider,
            path.1.clone(),
            section,
            query.offset.unwrap_or(0).min(10_000),
            query.limit.unwrap_or(50).clamp(1, 100),
        )
        .await,
    )
}

#[derive(Deserialize)]
struct CatalogParameters {
    source: Option<String>,
    genre: Option<String>,
    decade: Option<u16>,
    quality: Option<String>,
    q: Option<String>,
    sort: Option<String>,
    offset: Option<usize>,
    limit: Option<usize>,
}

const MAX_GENRE_FILTER_CHARACTERS: usize = 128;

fn catalog_filter(query: &CatalogParameters) -> Result<CatalogFilter, &'static str> {
    let source = match query.source.as_deref() {
        None | Some("") | Some("all") => None,
        Some(value) => Some(CatalogSource::parse(value).ok_or("Unknown source.")?),
    };
    let quality = match query.quality.as_deref() {
        None | Some("") => None,
        Some(value) => Some(QualityTier::parse(value).ok_or("Unknown quality.")?),
    };
    let genre = query
        .genre
        .as_deref()
        .map(str::trim)
        .filter(|genre| !genre.is_empty())
        .map(str::to_string);
    if genre
        .as_deref()
        .is_some_and(|genre| genre.chars().count() > MAX_GENRE_FILTER_CHARACTERS)
    {
        return Err("Genre names are at most 128 characters.");
    }
    if query
        .decade
        .is_some_and(|decade| !decade.is_multiple_of(10) || !(1000..=2990).contains(&decade))
    {
        return Err("A decade is a year ending in 0, such as 1990.");
    }
    let sort = match query.sort.as_deref() {
        None | Some("") => CatalogSort::default(),
        Some(value) => CatalogSort::parse(value).ok_or("Unknown sort.")?,
    };
    Ok(CatalogFilter {
        source,
        genre,
        decade: query.decade,
        quality,
        sort,
        text: query
            .q
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(|text| text.chars().take(MAX_SEARCH_QUERY_CHARACTERS).collect()),
    })
}

#[derive(Serialize)]
struct CatalogResponse {
    #[serde(flatten)]
    page: CatalogPage,
    /// Whether local albums are included (the local library is indexed).
    local_ready: bool,
}

/// Local and streaming albums in one catalog, filtered by source, genre,
/// decade and quality together, with counts for each filter.
#[get("/catalog/albums")]
async fn catalog_albums(
    request: HttpRequest,
    query: web::Query<CatalogParameters>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let filter = match catalog_filter(&query) {
        Ok(filter) => filter,
        Err(message) => return hub_error_response(HubError::Invalid(message)),
    };
    let rows = match hub.library_albums(user_id).await {
        Ok(rows) => rows,
        Err(error) => return hub_error_response(error),
    };
    let lifecycle = request.app_data::<web::Data<LibraryLifecycle>>().cloned();
    let cache = match lifecycle {
        Some(lifecycle) => lifecycle.cache().await.ok(),
        None => None,
    };
    let local_ready = cache.is_some();
    let offset = query.offset.unwrap_or(0).min(100_000);
    let limit = query.limit.unwrap_or(100);
    let page = tokio::task::spawn_blocking(move || {
        let mut albums = cache
            .map(|cache| catalog::local_albums(&cache.artists, &cache.album_genres))
            .unwrap_or_default();
        albums.extend(catalog::streaming_albums(&rows));
        catalog::fill_missing_genres(&mut albums);
        catalog::query(albums, &filter, offset, limit)
    })
    .await;
    match page {
        Ok(page) => HttpResponse::Ok().json(CatalogResponse { page, local_ready }),
        Err(error) => hub_error_response(HubError::Database(error.to_string())),
    }
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

#[derive(Deserialize)]
struct SaveCopyQuery {
    #[serde(default)]
    replace_edits: bool,
}

/// Saves (or refreshes) a copy. The service playlist is not changed. A
/// copy edited in Parson is only replaced with `?replace_edits=true`.
#[put("/{provider}/playlists/{id}/copy")]
async fn save_playlist_copy(
    request: HttpRequest,
    path: web::Path<(String, String)>,
    query: web::Query<SaveCopyQuery>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let Some((provider, raw)) = provider_and_raw(&path.0, &path.1) else {
        return unknown_provider();
    };
    respond(
        hub.save_playlist_copy(user_id, provider, raw, query.replace_edits)
            .await,
    )
}

/// Renames, reorders, removes or adds tracks in a copy, in Parson only.
#[patch("/{provider}/playlists/{id}/copy")]
async fn edit_playlist_copy(
    request: HttpRequest,
    path: web::Path<(String, String)>,
    body: web::Json<crate::providers::hub::CopyEdit>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let Some((provider, raw)) = provider_and_raw(&path.0, &path.1) else {
        return unknown_provider();
    };
    respond(
        hub.edit_playlist_copy(user_id, provider, raw, body.into_inner())
            .await,
    )
}

#[derive(Deserialize)]
struct NewCopyBody {
    title: String,
    #[serde(default)]
    description: Option<String>,
    track_ids: Vec<String>,
}

/// A new playlist made in Parson from one service's tracks.
#[post("/{provider}/playlist-copies")]
async fn create_playlist_copy(
    request: HttpRequest,
    path: web::Path<String>,
    body: web::Json<NewCopyBody>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let Some(provider) = ProviderId::parse(&path) else {
        return unknown_provider();
    };
    let body = body.into_inner();
    respond(
        hub.create_playlist_copy(
            user_id,
            provider,
            body.title,
            body.description,
            body.track_ids,
        )
        .await,
    )
}

/// A radio from a track, artist or album: TIDAL's own radio, or for Qobuz
/// Parson's mix of the artist's and similar artists' top tracks.
#[get("/{provider}/radio/{seed}/{id}")]
async fn radio(
    request: HttpRequest,
    path: web::Path<(String, String, String)>,
    hub: web::Data<StreamingHub>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let (provider, seed, id) = path.into_inner();
    let Some((provider, raw)) = provider_and_raw(&provider, &id) else {
        return unknown_provider();
    };
    let Some(seed) = crate::providers::RadioSeed::parse(&seed) else {
        return hub_error_response(HubError::Invalid(
            "Start a radio from a track, artist or album.",
        ));
    };
    respond(hub.radio(user_id, provider, seed, raw).await)
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
/// delivered, read from its bytes by the media proxy. With `?play=`, only
/// that play's report: a player asking about the stream it is playing
/// never gets an earlier stream's format.
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
    let track_id = entity_id(provider, &raw);
    let found = match play_id(request.query_string()) {
        Some(play) => hub.play_delivery(user_id, &track_id, &play),
        None => hub.delivery(user_id, &track_id),
    };
    match found {
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
            .service(get_source_preference)
            .service(set_source_preference)
            .service(list_server_credentials)
            .service(save_server_credentials)
            .service(remove_server_credentials)
            .service(catalog_albums)
            .service(check_connection)
            .service(stream_delivery)
            .service(genres)
            .service(genre_albums)
            .service(playlist_copies)
            .service(playlists)
            .service(playlist_copy)
            .service(save_playlist_copy)
            .service(edit_playlist_copy)
            .service(create_playlist_copy)
            .service(radio)
            .service(remove_playlist_copy)
            .service(playlist)
            .service(list_providers)
            .service(search)
            .service(library)
            .service(start_tidal_sign_in)
            .service(poll_tidal_sign_in)
            .service(start_tidal_browser_sign_in)
            .service(finish_tidal_browser_sign_in)
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
    let play = play_id(request.query_string());
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
                hub.record_play(user_id, track_id, play, &stream, probed, None);
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
                    (track_id, play),
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
            let key = format!("{track_id}@{}", stream.requested.as_str());
            let index = match hub.segment_index(key, &urls).await {
                Ok(index) => Some(index),
                Err(error) => {
                    // Still playable from the start, just not seekable.
                    tracing::warn!(%error, "could not measure DASH segments");
                    None
                }
            };
            let Some(index) = index else {
                if range.is_some() && !probe_this {
                    return HttpResponse::RangeNotSatisfiable().finish();
                }
                let mut builder = format_headers(HttpResponse::Ok(), &stream);
                builder.insert_header((header::CONTENT_TYPE, mime.clone()));
                let body = recorded(
                    hub.clone(),
                    user_id,
                    (track_id, play),
                    stream.clone(),
                    media::segmented(hub.media(), urls),
                );
                return builder.streaming(body.map(|chunk| chunk.map_err(proxy_error)));
            };
            let total = index.total();
            let (start, end, content_range) = match range.as_deref() {
                None => (0, total.saturating_sub(1), None),
                Some(header_value) => match byte_range(Some(header_value), total) {
                    Some((start, end)) => {
                        (start, end, Some(format!("bytes {start}-{end}/{total}")))
                    }
                    None => {
                        return HttpResponse::RangeNotSatisfiable()
                            .insert_header((header::CONTENT_RANGE, format!("bytes */{total}")))
                            .finish();
                    }
                },
            };
            let status = if content_range.is_some() {
                StatusCode::PARTIAL_CONTENT
            } else {
                StatusCode::OK
            };
            let mut builder = format_headers(HttpResponse::build(status), &stream);
            if let Some(content_range) = content_range {
                builder.insert_header((header::CONTENT_RANGE, content_range));
            }
            builder.insert_header((header::CONTENT_TYPE, mime.clone()));
            builder.insert_header((header::ACCEPT_RANGES, "bytes"));
            let length = if total == 0 { 0 } else { end - start + 1 };
            builder.no_chunking(length);
            let body = media::pieces(hub.media(), index.plan(&urls, start, end));
            let body = if probe_this {
                recorded(hub.clone(), user_id, (track_id, play), stream.clone(), body)
            } else {
                body
            };
            builder.streaming(body.map(|chunk| chunk.map_err(proxy_error)))
        }
    }
}

/// The player's id for one play, from a stream URL's `play` parameter.
fn play_id(query: &str) -> Option<String> {
    web::Query::<std::collections::HashMap<String, String>>::from_query(query)
        .ok()?
        .into_inner()
        .remove("play")
        .filter(|value| valid_raw_id(value))
}

/// Reads the delivered format from the bytes as they pass through.
fn recorded(
    hub: web::Data<StreamingHub>,
    user_id: i32,
    (track_id, play): (String, Option<String>),
    stream: ResolvedStream,
    body: media::MediaBody,
) -> media::MediaBody {
    media::tap(body, move |probed| {
        let delivery = hub.record_play(user_id, track_id, play, &stream, probed, None);
        if !delivery.matches_announced {
            tracing::info!(
                track = delivery.track_id,
                announced = delivery.announced.label,
                mismatches = ?delivery.mismatches,
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
    use actix_web::{App, HttpMessage, test, web};
    use diesel::connection::SimpleConnection;
    use diesel::r2d2::{ConnectionManager, Pool};
    use diesel::sqlite::SqliteConnection;
    use diesel_migrations::MigrationHarness;
    use serde_json::{Value, json};

    use super::{byte_range, configure};
    use crate::api::auth::Claims;
    use crate::library::state::LibraryLifecycle;
    use crate::persistence::connection::DbPool;
    use crate::providers::ProviderId;
    use crate::providers::hub::StreamingHub;
    use crate::providers::media::FixtureMedia;
    use crate::providers::transport::{FixtureReply, FixtureRoute, FixtureTransport, Method};
    use crate::providers::vault::TokenVault;

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

    macro_rules! app {
        ($hub:expr, $user:expr) => {
            test::init_service(
                App::new()
                    .app_data(web::Data::new($hub))
                    .wrap_fn(move |request, service| {
                        request.extensions_mut().insert(claims($user));
                        service.call(request)
                    })
                    .app_data(web::Data::new(LibraryLifecycle::new()))
                    .configure(configure)
                    .service(crate::api::library::stream_song),
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
                    .app_data(web::Data::new(LibraryLifecycle::new()))
                    .configure(configure)
                    .service(crate::api::library::stream_song),
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
        for plaintext in ["recorded-access-token", "recorded-refresh-token", "s3cret"] {
            for blob in [&sealed.sealed_session, &sealed.sealed_credentials] {
                assert!(
                    !blob
                        .windows(plaintext.len())
                        .any(|window| window == plaintext.as_bytes()),
                    "{plaintext} is stored sealed"
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
        signed_in_with_transport(pool).await.0
    }

    async fn signed_in_with_transport(pool: &DbPool) -> (StreamingHub, Arc<FixtureTransport>) {
        let (hub, transport) = hub(pool.clone(), false);
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
        (hub, transport)
    }

    #[actix_web::test]
    async fn the_stream_route_plays_without_a_bitrate_and_honors_one() {
        let pool = pool();
        let (hub, transport) = signed_in_with_transport(&pool).await;
        let media = Arc::new(FixtureMedia::new().with(QOBUZ_HIRES_URL, HIRES_192_FLAC.to_vec()));
        let app = app!(hub.with_media(media), 1);
        let file_url_formats = || {
            transport
                .requests()
                .iter()
                .filter(|request| request.url.ends_with("track/getFileUrl"))
                .filter_map(|request| request.query_value("format_id").map(str::to_string))
                .collect::<Vec<_>>()
        };

        // Players that omit `bitrate` get the listener's chosen quality.
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
                .get("x-parson-stream-quality")
                .and_then(|value| value.to_str().ok()),
            Some("hi_res_max")
        );
        assert_eq!(file_url_formats(), ["27"]);

        // `bitrate=0` is the same; a lossy bitrate asks Qobuz for MP3.
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/songs/qobuz:7700001/stream?bitrate=0")
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let _ = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/songs/qobuz:7700001/stream?bitrate=320")
                .to_request(),
        )
        .await;
        assert_eq!(file_url_formats(), ["27", "27", "5"]);

        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/songs/qobuz:7700001/stream?bitrate=abc")
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_web::test]
    async fn radios_start_from_tracks_artists_and_albums() {
        let pool = pool();
        let hub = signed_in_app_state(&pool).await;
        let app = app!(hub, 1);

        let (status, body) = call!(
            &app,
            test::TestRequest::get().uri("/providers/tidal/radio/track/300101"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["origin"], "service");
        assert_eq!(body["tracks"].as_array().map(Vec::len), Some(13));

        // Album radio starts from the album's first track.
        let (status, album_radio) = call!(
            &app,
            test::TestRequest::get().uri("/providers/tidal/radio/album/300100"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{album_radio}");
        assert_eq!(album_radio["tracks"], body["tracks"]);

        let (status, body) = call!(
            &app,
            test::TestRequest::get().uri("/providers/qobuz/radio/artist/51004"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["origin"], "similar_artists");
        assert_eq!(body["tracks"][0]["artist"]["name"], "Mara Vell");

        let (status, _) = call!(
            &app,
            test::TestRequest::get().uri("/providers/qobuz/radio/genre/51004"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[actix_web::test]
    async fn playlist_copies_are_editable_and_radios_can_be_kept() {
        let pool = pool();
        let hub = signed_in_app_state(&pool).await;
        let app = app!(hub, 1);
        let raw = "7a3f2c10-5b4e-4d1a-9c2b-0e1f2a3b4c5d";
        let uri = format!("/providers/tidal/playlists/{raw}/copy");
        let (status, copy) = call!(&app, test::TestRequest::put().uri(&uri)).await;
        assert_eq!(status, StatusCode::OK, "{copy}");
        assert_eq!(copy["edited_at"], Value::Null);
        assert_eq!(copy["made_in_parson"], false);

        // Rename, drop a track, move the last one first and add one from
        // another album.
        let (status, edited) = call!(
            &app,
            test::TestRequest::patch().uri(&uri).set_json(json!({
                "title": "  Late Night (mine)  ",
                "description": "Edited in Mujin Music",
                "track_ids": ["tidal:300104", "tidal:300101", "tidal:300102", "tidal:300301"],
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{edited}");
        assert_eq!(edited["title"], "Late Night (mine)");
        assert!(edited["edited_at"].as_i64().is_some());
        let ids = edited["playlist"]["tracks"]
            .as_array()
            .expect("tracks")
            .iter()
            .map(|track| track["id"].as_str().unwrap_or_default().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            [
                "tidal:300104",
                "tidal:300101",
                "tidal:300102",
                "tidal:300301"
            ]
        );
        assert_eq!(edited["playlist"]["tracks"][3]["title"], "Low Tide Hymn");
        assert_eq!(edited["playlist"]["description"], "Edited in Mujin Music");
        assert_eq!(edited["track_count"], 4);

        // Saving the service playlist again asks before replacing edits.
        let (status, body) = call!(&app, test::TestRequest::put().uri(&uri)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["code"], "provider_copy_edited");
        let (_, kept) = call!(&app, test::TestRequest::get().uri(&uri)).await;
        assert_eq!(kept["title"], "Late Night (mine)");
        let (status, replaced) = call!(
            &app,
            test::TestRequest::put().uri(&format!("{uri}?replace_edits=true")),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{replaced}");
        assert_eq!(replaced["title"], "Late Night Hi-Res");
        assert_eq!(replaced["edited_at"], Value::Null);

        // Tracks from another service, unknown ids and empty names are refused.
        for body in [
            json!({"track_ids": ["qobuz:7700001"]}),
            json!({"track_ids": ["spotify:1"]}),
            json!({"title": "   "}),
        ] {
            let (status, _) =
                call!(&app, test::TestRequest::patch().uri(&uri).set_json(body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
        }

        // A radio kept as a playlist made in Parson.
        let (_, radio) = call!(
            &app,
            test::TestRequest::get().uri("/providers/qobuz/radio/track/7700202"),
        )
        .await;
        let track_ids = radio["tracks"]
            .as_array()
            .expect("radio")
            .iter()
            .take(5)
            .map(|track| track["id"].clone())
            .collect::<Vec<_>>();
        let (status, made) = call!(
            &app,
            test::TestRequest::post()
                .uri("/providers/qobuz/playlist-copies")
                .set_json(json!({"title": "Kenroku-en radio", "track_ids": track_ids})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{made}");
        assert_eq!(made["made_in_parson"], true);
        assert_eq!(made["track_count"], 5);
        assert_eq!(made["owner"], "You, in Mujin Music");
        let id = made["id"].as_str().expect("id");
        let made_raw = id.strip_prefix("qobuz:").expect("qobuz id");
        assert!(made_raw.starts_with("parson-"));
        let (status, _) = call!(
            &app,
            test::TestRequest::put().uri(&format!("/providers/qobuz/playlists/{made_raw}/copy")),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "no service playlist to save again"
        );
        let (_, copies) = call!(
            &app,
            test::TestRequest::get().uri("/providers/playlist-copies"),
        )
        .await;
        assert_eq!(copies["copies"].as_array().map(Vec::len), Some(2));

        // Another listener cannot see or edit it.
        let other = app!(hub_for(pool.clone()), 2);
        let (status, _) = call!(
            &other,
            test::TestRequest::patch()
                .uri(&format!("/providers/qobuz/playlists/{made_raw}/copy"))
                .set_json(json!({"title": "Mine now"})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[actix_web::test]
    async fn album_and_artist_pages_include_reviews_credits_and_bios() {
        let pool = pool();
        let hub = signed_in_app_state(&pool).await;
        let app = app!(hub, 1);
        let (status, album) = call!(
            &app,
            test::TestRequest::get().uri("/providers/qobuz/albums/q0glassorch01"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{album}");
        assert_eq!(album["review"]["source"], "Qobuz");
        assert_eq!(album["awards"][0], "Qobuzissime");
        assert_eq!(album["tracks"][0]["credits"][2]["role"], "Producer");
        assert_eq!(album["tracks"][0]["credits"][2]["names"][0], "Ines Calder");

        let (status, artist) = call!(
            &app,
            test::TestRequest::get().uri("/providers/tidal/artists/400100"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{artist}");
        assert!(
            artist["bio"]["text"]
                .as_str()
                .is_some_and(|text| text.starts_with("Mara Vell is a songwriter"))
        );
        assert_eq!(artist["similar"].as_array().map(Vec::len), Some(2));
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
    async fn genre_pages_and_editorial_lists_browse_each_service() {
        let pool = pool();
        let hub = signed_in_app_state(&pool).await;
        let app = app!(hub, 1);

        let (_, body) = call!(&app, test::TestRequest::get().uri("/providers")).await;
        let sections = |index: usize| {
            body["providers"][index]["browse_sections"]
                .as_array()
                .expect("sections")
                .iter()
                .map(|section| section["id"].as_str().unwrap_or_default().to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(sections(0), ["new_releases", "editors_picks", "popular"]);
        assert_eq!(
            sections(1),
            ["new_releases", "editors_picks", "popular", "press_awards"]
        );

        let (status, body) = call!(
            &app,
            test::TestRequest::get().uri("/providers/tidal/genres")
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["genres"][0]["name"], "Pop");
        assert_eq!(body["genres"].as_array().map(Vec::len), Some(4));

        let (status, body) = call!(
            &app,
            test::TestRequest::get().uri("/providers/tidal/genres/Jazz/albums"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["genre"]["name"], "Jazz");
        assert_eq!(body["albums"][0]["title"], "Night Shift Standards");
        assert_eq!(
            body["albums"][0]["genre"], "Jazz",
            "TIDAL albums are labelled with the genre page they came from"
        );

        let (status, body) = call!(
            &app,
            test::TestRequest::get()
                .uri("/providers/qobuz/genres/10/albums?section=press_awards&limit=10"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["genre"]["name"], "Classical");
        assert_eq!(body["albums"][0]["title"], "Glass Orchard");

        let (status, _) = call!(
            &app,
            test::TestRequest::get().uri("/providers/tidal/genres/Polka/albums"),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = call!(
            &app,
            test::TestRequest::get()
                .uri("/providers/qobuz/genres/10/albums?section=favorite_tracks"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, body) = call!(
            &app,
            test::TestRequest::get().uri("/providers/qobuz/browse/editors_picks"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["albums"][0]["title"], "Glass Orchard");
        let (status, _) = call!(
            &app,
            test::TestRequest::get().uri("/providers/tidal/browse/press_awards"),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[actix_web::test]
    async fn the_album_catalog_combines_source_genre_and_decade_filters() {
        let pool = pool();
        let hub = signed_in_app_state(&pool).await;
        let app = app!(hub, 1);

        // Visiting TIDAL's Jazz page files its albums under Jazz.
        call!(
            &app,
            test::TestRequest::get().uri("/providers/tidal/genres/Jazz/albums"),
        )
        .await;
        for uri in [
            "/providers/tidal/library/album/300400",
            "/providers/tidal/library/album/300100",
            "/providers/qobuz/library/album/q0harborhr04",
            "/providers/qobuz/library/album/q0brasslate02",
        ] {
            let (status, _) = call!(&app, test::TestRequest::put().uri(uri)).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
        }

        let (status, body) = call!(
            &app,
            test::TestRequest::get().uri("/providers/catalog/albums"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["total"], 4);
        assert_eq!(body["local_ready"], false);
        assert_eq!(body["albums"][0]["artist_name"], "Mara Vell");

        let (_, body) = call!(
            &app,
            test::TestRequest::get().uri("/providers/catalog/albums?source=tidal&genre=jazz"),
        )
        .await;
        assert_eq!(body["total"], 1);
        assert_eq!(body["albums"][0]["id"], "tidal:300400");
        assert_eq!(body["albums"][0]["genres"][0], "Jazz");
        assert_eq!(body["albums"][0]["source"], "tidal");
        let jazz_sources = body["facets"]["sources"]
            .as_array()
            .expect("sources")
            .iter()
            .map(|facet| (facet["value"].to_string(), facet["count"].as_u64()))
            .collect::<Vec<_>>();
        assert_eq!(
            jazz_sources,
            [
                ("\"tidal\"".to_string(), Some(1)),
                ("\"qobuz\"".to_string(), Some(1))
            ],
            "Jazz on each service"
        );

        let (_, body) = call!(
            &app,
            test::TestRequest::get().uri("/providers/catalog/albums?genre=Pop%2FRock"),
        )
        .await;
        assert_eq!(body["total"], 2, "the TIDAL copy borrows Qobuz's genre");
        assert_eq!(body["albums"][0]["source"], "tidal");
        assert_eq!(body["albums"][0]["genre_inferred"], true);

        let (_, body) = call!(
            &app,
            test::TestRequest::get()
                .uri("/providers/catalog/albums?source=qobuz&decade=2020&quality=hi_res"),
        )
        .await;
        assert_eq!(body["total"], 1);
        assert_eq!(body["albums"][0]["format_label"], "24/192 FLAC");
        let decades = body["facets"]["decades"]
            .as_array()
            .expect("decades")
            .iter()
            .map(|facet| facet["value"].as_u64())
            .collect::<Vec<_>>();
        assert_eq!(decades, [Some(2020)]);

        for bad in ["decade=1995", "source=spotify", "quality=mqa"] {
            let (status, _) = call!(
                &app,
                test::TestRequest::get().uri(&format!("/providers/catalog/albums?{bad}")),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        }

        let other = app!(hub_for(pool.clone()), 2);
        let (_, body) = call!(
            &other,
            test::TestRequest::get().uri("/providers/catalog/albums"),
        )
        .await;
        assert_eq!(body["total"], 0, "each listener has their own catalog");
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
        // The manifest lists four media segments; cut the fragments into
        // four so ranges cross real segment boundaries.
        let mut media = FixtureMedia::new().with(&format!("{base}/0.mp4?token=a&sig=b"), init);
        let quarter = fragments.len().div_ceil(4);
        for (number, piece) in fragments.chunks(quarter).enumerate() {
            media = media.with(
                &format!("{base}/{}.mp4?token=a&sig=b", number + 1),
                piece.to_vec(),
            );
        }
        Arc::new(media)
    }

    #[actix_web::test]
    async fn source_preference_defaults_to_highest_quality_and_is_per_listener() {
        let pool = pool();
        let (hub, _) = hub(pool, false);
        let data = web::Data::new(hub);
        let ada = app_as!(data, 1, "user");
        let grace = app_as!(data, 2, "user");
        let (status, body) = call!(
            &ada,
            test::TestRequest::get().uri("/providers/preferences/source"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["source"], "highest_quality");
        let (status, body) = call!(
            &ada,
            test::TestRequest::put()
                .uri("/providers/preferences/source")
                .set_json(json!({"source": "qobuz"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["source"], "qobuz");
        let (_, body) = call!(
            &ada,
            test::TestRequest::get().uri("/providers/preferences/source"),
        )
        .await;
        assert_eq!(body["source"], "qobuz");
        let (_, body) = call!(
            &grace,
            test::TestRequest::get().uri("/providers/preferences/source"),
        )
        .await;
        assert_eq!(
            body["source"], "highest_quality",
            "other listeners unchanged"
        );
        let (status, _) = call!(
            &ada,
            test::TestRequest::put()
                .uri("/providers/preferences/source")
                .set_json(json!({"source": "spotify"})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[actix_web::test]
    async fn tidal_dash_streams_seek_with_byte_ranges_across_segments() {
        let pool = pool();
        let media = tidal_dash_media();
        let hub = signed_in_app_state(&pool).await.with_media(media.clone());
        let app = app!(hub, 1);
        let total = HIRES_96_DASH.len();
        let header = |response: &actix_web::dev::ServiceResponse, name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
        };

        // A player's first request: the whole track, with its length, so
        // the browser knows it can seek.
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/songs/tidal:300101/stream")
                .insert_header(("range", "bytes=0-"))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(header(&response, "accept-ranges").as_deref(), Some("bytes"));
        assert_eq!(
            header(&response, "content-range"),
            Some(format!("bytes 0-{}/{total}", total - 1))
        );
        assert_eq!(header(&response, "content-length"), Some(total.to_string()));
        assert_eq!(&test::read_body(response).await[..], HIRES_96_DASH);
        let measured = media
            .requests()
            .iter()
            .filter(|(_, range)| range.as_deref() == Some("bytes=0-0"))
            .count();
        assert_eq!(measured, 5, "init and four media segments measured once");

        // Seeking into the middle, across two segment boundaries.
        let (start, end) = (600, total - 700);
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/songs/tidal:300101/stream")
                .insert_header(("range", format!("bytes={start}-{end}")))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            header(&response, "content-range"),
            Some(format!("bytes {start}-{end}/{total}"))
        );
        assert_eq!(
            &test::read_body(response).await[..],
            &HIRES_96_DASH[start..=end]
        );

        // The last bytes, as a demuxer reads an index at the end.
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/songs/tidal:300101/stream")
                .insert_header(("range", "bytes=-67"))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            &test::read_body(response).await[..],
            &HIRES_96_DASH[total - 67..]
        );
        let measured_again = media
            .requests()
            .iter()
            .filter(|(_, range)| range.as_deref() == Some("bytes=0-0"))
            .count();
        assert_eq!(measured_again, measured, "seeks reuse the segment index");

        // Past the end is refused with the real length.
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/songs/tidal:300101/stream")
                .insert_header(("range", format!("bytes={total}-")))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(
            header(&response, "content-range"),
            Some(format!("bytes */{total}"))
        );

        // A seek does not overwrite what the first bytes proved.
        let (_, delivery) = call!(
            &app,
            test::TestRequest::get().uri("/providers/tidal/tracks/300101/delivery"),
        )
        .await;
        assert_eq!(delivery["delivered"]["label"], "24/96 FLAC");
        assert_eq!(delivery["frame_verified"], true);
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
            for plaintext in [&b"server-app-secret"[..], b"server-tidal-client"] {
                assert!(
                    !blob
                        .windows(plaintext.len())
                        .any(|window| window == plaintext)
                );
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
                ..Default::default()
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
        let media =
            Arc::new(FixtureMedia::new().with(QOBUZ_HIRES_URL, stated_length(HIRES_192_FLAC, 274)));
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
        // The whole track arrived: the header states the catalog length.
        assert_eq!(check["delivery"]["full_length"], true, "{check}");
        assert_eq!(check["delivery"]["track_seconds"], 274.0);
        assert!(
            steps[2]["detail"]
                .as_str()
                .is_some_and(|detail| detail.contains("full length (4:34)")),
            "{check}"
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

        // Settings keeps the result: this account itself is now proven.
        let (_, listed) = call!(&app, test::TestRequest::get().uri("/providers")).await;
        let qobuz = listed["providers"]
            .as_array()
            .and_then(|providers| providers.iter().find(|provider| provider["id"] == "qobuz"))
            .expect("qobuz")
            .clone();
        let last = &qobuz["last_check"];
        assert_eq!(last["verified"], true, "{last}");
        assert_eq!(last["sandbox"], false);
        assert_eq!(last["delivered"], "24/192 FLAC");
        assert_eq!(last["full_length"], true);
        assert_eq!(last["lossless"], true);
        assert_eq!(last["failed_step"], Value::Null);
        let tidal = listed["providers"]
            .as_array()
            .and_then(|providers| providers.iter().find(|provider| provider["id"] == "tidal"))
            .expect("tidal");
        assert_eq!(tidal["last_check"], Value::Null, "never checked");

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
        // A sandbox check is kept too, but never counts as proof.
        let status = hub
            .provider_status(1, ProviderId::Qobuz)
            .await
            .expect("status");
        let last = status.last_check.expect("kept");
        assert!(last.sandbox && !last.verified, "{last:?}");
        // New credentials belong to another client: the proof is cleared.
        hub.save_credentials(
            1,
            ProviderId::Qobuz,
            Some("sandbox-2".into()),
            Some("sandbox".into()),
            false,
        )
        .await
        .expect("new credentials");
        let status = hub
            .provider_status(1, ProviderId::Qobuz)
            .await
            .expect("status");
        assert_eq!(status.last_check, None);
        let delivery = check.delivery.expect("delivery");
        assert!(delivery.sandbox);
        assert!(!delivery.matches_announced);
        assert_eq!(
            delivery.delivered.map(|format| format.label),
            Some("16/44.1 PCM".to_string())
        );
        assert!(check.steps[2].detail.contains("Sandbox preview tone"));
    }

    /// A sandbox hub whose streams are rendered in each announced format
    /// (as `PARSON_STREAMING_SANDBOX=1` runs when ffmpeg is installed).
    fn sandbox_root(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("parson-sandbox-test-{name}-{}", std::process::id()))
    }

    /// Removes a test's rendered streams when the test ends, pass or fail.
    struct RemoveOnDrop(std::path::PathBuf);

    impl Drop for RemoveOnDrop {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn rendered_sandbox(pool: &DbPool, name: &str) -> StreamingHub {
        use crate::providers::sandbox_media::{SandboxMedia, SandboxStreams, SandboxTransport};
        let root = sandbox_root(name);
        let _ = std::fs::remove_dir_all(&root);
        let streams = Arc::new(SandboxStreams::for_sandbox_in(root));
        let transport = Arc::new(SandboxTransport::new(
            crate::providers::sandbox::transport(),
            streams.clone(),
        ));
        let hub = StreamingHub::new(
            pool.clone(),
            TokenVault::new([5; 32]),
            transport,
            true,
            HashMap::new(),
        )
        .with_rendered_sandbox(Arc::new(SandboxMedia::new(streams)));
        hub.save_credentials(1, ProviderId::Tidal, Some("sandbox".into()), None, false)
            .await
            .expect("tidal credentials");
        hub.start_tidal_sign_in(1).await.expect("start");
        hub.poll_tidal_sign_in(1).await.expect("pending");
        hub.poll_tidal_sign_in(1).await.expect("approved");
        hub.save_credentials(
            1,
            ProviderId::Qobuz,
            Some("sandbox".into()),
            Some("sandbox".into()),
            false,
        )
        .await
        .expect("qobuz credentials");
        hub.qobuz_sign_in(1, "sandbox", "sandbox")
            .await
            .expect("qobuz sign-in");
        hub
    }

    /// Stamps a FLAC file's `STREAMINFO` with a total length, the way an
    /// encoder writes it for a whole track.
    fn stated_length(file: &[u8], seconds: u64) -> Vec<u8> {
        let mut file = file.to_vec();
        // "fLaC", the block header, then STREAMINFO: the rate is in bytes
        // 10-12 and the 36-bit sample count in bytes 13-17.
        let info = 8;
        let rate = (u64::from(file[info + 10]) << 12)
            | (u64::from(file[info + 11]) << 4)
            | (u64::from(file[info + 12]) >> 4);
        let samples = seconds * rate;
        file[info + 13] = (file[info + 13] & 0xf0) | ((samples >> 32) as u8 & 0x0f);
        file[info + 14..info + 18].copy_from_slice(&(samples as u32).to_be_bytes());
        file
    }

    #[actix_web::test]
    async fn tidal_signs_in_headlessly_with_a_refresh_token() {
        let pool = pool();
        let (hub, transport) = hub(pool, false);
        hub.save_credentials(1, ProviderId::Tidal, Some("client".into()), None, false)
            .await
            .expect("tidal credentials");
        assert!(matches!(
            hub.tidal_sign_in_with_refresh_token(1, "  ").await,
            Err(crate::providers::hub::HubError::Invalid(_))
        ));
        let status = hub
            .tidal_sign_in_with_refresh_token(1, "recorded-refresh-token")
            .await
            .expect("signed in");
        assert_eq!(
            status.status,
            crate::providers::store::AccountStatus::SignedIn
        );
        let refresh = transport
            .requests()
            .into_iter()
            .find(|request| request.form_value("grant_type") == Some("refresh_token"))
            .expect("refresh request");
        assert_eq!(
            refresh.form_value("refresh_token"),
            Some("recorded-refresh-token")
        );
        // The session works: favorites sync through it.
        hub.sync_library(1, ProviderId::Tidal).await.expect("sync");
    }

    #[actix_web::test]
    async fn connection_check_fails_a_preview_posing_as_the_full_track() {
        let pool = pool();
        // 0.03 seconds of audio for a 4:34 track, though the reply said FULL.
        let media = Arc::new(FixtureMedia::new().with(QOBUZ_HIRES_URL, HIRES_192_FLAC.to_vec()));
        let hub = signed_in_app_state(&pool).await.with_media(media);
        let check = hub
            .check_connection(1, ProviderId::Qobuz, Some("7700001".into()))
            .await
            .expect("check");
        assert!(!check.ok, "{:?}", check.steps);
        let last = hub
            .provider_status(1, ProviderId::Qobuz)
            .await
            .expect("status")
            .last_check
            .expect("kept");
        assert!(!last.verified && !last.ok);
        assert_eq!(last.failed_step.as_deref(), Some("stream"));
        assert_eq!(last.full_length, Some(false));
        assert!(last.detail.contains("a preview"), "{}", last.detail);
        let delivery = check.delivery.expect("delivery");
        assert_eq!(delivery.full_length, Some(false));
        assert_eq!(delivery.track_seconds, Some(274.0));
        assert!(
            delivery
                .delivered_seconds
                .is_some_and(|seconds| seconds < 1.0)
        );
        let stream = check.steps.last().expect("stream step");
        assert_eq!(stream.id, "stream");
        assert!(!stream.ok);
        assert!(
            stream
                .detail
                .contains("only 0:00 of 4:34 arrived, a preview"),
            "{}",
            stream.detail
        );
    }

    #[actix_web::test]
    async fn rendered_sandbox_streams_deliver_every_tier_as_announced() {
        if !crate::providers::sandbox_media::ffmpeg_available() {
            eprintln!("ffmpeg is not installed; rendered sandbox streams cannot be checked");
            return;
        }
        let _cleanup = RemoveOnDrop(sandbox_root("tiers"));
        let pool = pool();
        let hub = web::Data::new(rendered_sandbox(&pool, "tiers").await);
        let app = app_as!(hub, 1, "user");
        // (service, quality, track, delivered label, container, lossless)
        let cases = [
            (
                "qobuz",
                "hi_res_max",
                "7700001",
                "24/192 FLAC",
                "flac",
                true,
            ),
            ("qobuz", "hi_res_max", "7700201", "24/96 FLAC", "flac", true),
            ("qobuz", "hi_res_96", "7700001", "24/96 FLAC", "flac", true),
            (
                "qobuz",
                "hi_res_max",
                "7700101",
                "16/44.1 FLAC",
                "flac",
                true,
            ),
            ("qobuz", "lossless", "7700001", "16/44.1 FLAC", "flac", true),
            ("qobuz", "high", "7700001", "", "mp3", false),
            ("tidal", "hi_res_max", "300101", "24/192 FLAC", "mp4", true),
            (
                "tidal",
                "hi_res_max",
                "300201",
                "16/44.1 FLAC",
                "flac",
                true,
            ),
            ("tidal", "lossless", "300101", "16/44.1 FLAC", "flac", true),
            ("tidal", "high", "300101", "", "mp4", false),
            ("tidal", "low", "300101", "", "mp4", false),
        ];
        for (service, quality, track, label, container, lossless) in cases {
            let provider = ProviderId::parse(service).expect("service");
            hub.set_quality(
                1,
                provider,
                crate::providers::StreamQuality::parse(quality).expect("quality"),
            )
            .await
            .expect("quality");
            // The player names each play, as the web and phone apps do.
            let play = format!("{service}-{quality}-{track}");
            let response = test::call_service(
                &app,
                test::TestRequest::get()
                    .uri(&format!(
                        "/media/songs/{service}:{track}/stream?bitrate=0&play={play}"
                    ))
                    .to_request(),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK, "{service} {quality}");
            let length = response
                .headers()
                .get("content-length")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<usize>().ok());
            let body = test::read_body(response).await;
            assert!(body.len() > 100_000, "{service} {quality}: a whole stream");
            if let Some(length) = length {
                assert_eq!(length, body.len(), "{service} {quality}");
            }
            let (status, delivery) = call!(
                &app,
                test::TestRequest::get().uri(&format!(
                    "/providers/{service}/tracks/{track}/delivery?play={play}"
                )),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            let context = format!("{service} {quality} {track}: {delivery}");
            assert_eq!(delivery["play"], play.as_str(), "{context}");
            assert_eq!(delivery["sandbox"], true, "{context}");
            assert_eq!(delivery["tone"], false, "{context}");
            // ffmpeg's built-in AAC encoder cannot fill 320 kbps, and the
            // report says so rather than calling it a match.
            let short_aac = service == "tidal" && quality == "high";
            assert_eq!(delivery["matches_announced"], !short_aac, "{context}");
            if short_aac {
                let delivered = delivery["delivered"]["bitrate_kbps"]
                    .as_u64()
                    .expect("bitrate");
                assert!(delivered < 288, "{context}");
                assert_eq!(
                    delivery["mismatches"],
                    json!([format!("{delivered} kbps, not 320")]),
                    "{context}"
                );
            } else {
                assert_eq!(delivery["mismatches"], json!([]), "{context}");
            }
            // Sandbox streams are 30 seconds by design, so no full-length
            // verdict; the delivered length is still measured.
            assert_eq!(delivery["full_length"], Value::Null, "{context}");
            if container != "mp3" {
                let seconds = delivery["delivered_seconds"].as_f64().expect("length");
                assert!((seconds - 30.0).abs() < 0.5, "{context}");
            }
            assert_eq!(delivery["container"], container, "{context}");
            assert_eq!(delivery["delivered"]["lossless"], lossless, "{context}");
            assert_eq!(delivery["delivered"]["channels"], 2, "{context}");
            if lossless {
                assert_eq!(delivery["delivered"]["label"], label, "{context}");
                assert_eq!(delivery["announced"]["label"], label, "{context}");
                assert_eq!(delivery["frame_verified"], true, "{context}");
            }
        }

        // Each play keeps its own report: asking about the first play of
        // TIDAL 300101 (Max) after it was replayed at AAC still says Max.
        let (status, first) = call!(
            &app,
            test::TestRequest::get()
                .uri("/providers/tidal/tracks/300101/delivery?play=tidal-hi_res_max-300101"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(first["delivered"]["label"], "24/192 FLAC", "{first}");
        let (_, latest) = call!(
            &app,
            test::TestRequest::get().uri("/providers/tidal/tracks/300101/delivery"),
        )
        .await;
        assert_eq!(latest["requested"], "low", "{latest}");
        // A play whose bytes have not arrived yet has no report, never an
        // older stream's.
        let (status, _) = call!(
            &app,
            test::TestRequest::get()
                .uri("/providers/tidal/tracks/300101/delivery?play=not-started"),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // TIDAL Max DASH seeks across segment boundaries like a file.
        hub.set_quality(
            1,
            ProviderId::Tidal,
            crate::providers::StreamQuality::parse("hi_res_max").expect("quality"),
        )
        .await
        .expect("quality");
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/songs/tidal:300101/stream")
                .insert_header(("range", "bytes=0-"))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        let whole = test::read_body(response).await;
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/media/songs/tidal:300101/stream")
                .insert_header(("range", "bytes=900000-900099"))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            &test::read_body(response).await[..],
            &whole[900_000..900_100]
        );
        let (_, delivery) = call!(
            &app,
            test::TestRequest::get().uri("/providers/tidal/tracks/300101/delivery"),
        )
        .await;
        assert_eq!(delivery["delivered"]["label"], "24/192 FLAC");

        let check = hub
            .check_connection(1, ProviderId::Qobuz, None)
            .await
            .expect("check");
        let delivery = check.delivery.expect("delivery");
        assert!(delivery.matches_announced && !delivery.tone);
        assert!(
            check.steps[2]
                .detail
                .contains("Sandbox: the reply is recorded")
        );
    }

    #[actix_web::test]
    async fn tidal_browser_sign_in_uses_pkce_and_each_state_once() {
        let pool = pool();
        let (hub, transport) = hub(pool, false);
        hub.save_credentials(
            1,
            ProviderId::Tidal,
            Some("dashboard-client".into()),
            None,
            false,
        )
        .await
        .expect("credentials");
        let data = web::Data::new(hub);
        let app = app_as!(data, 1, "user");
        let callback = "https://music.example.test/settings/services/tidal-callback";

        for redirect_uri in [
            "javascript:alert(1)",
            "https://music.example.test/elsewhere",
            "https://music.example.test/settings/services/tidal-callback?next=x",
            "https:///settings/services/tidal-callback",
            // Another host must never receive TIDAL's authorization code.
            "https://attacker.example/settings/services/tidal-callback",
            "https://music.example.test.attacker.example/settings/services/tidal-callback",
            "https://user@music.example.test/settings/services/tidal-callback",
            "https://music.example.test/x/settings/services/tidal-callback",
        ] {
            let (status, _) = call!(
                &app,
                test::TestRequest::post()
                    .uri("/providers/tidal/sign-in/browser")
                    .insert_header(("host", "music.example.test"))
                    .set_json(json!({ "redirect_uri": redirect_uri })),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{redirect_uri}");
        }

        // The right path on a host the browser did not ask for is refused.
        let (status, body) = call!(
            &app,
            test::TestRequest::post()
                .uri("/providers/tidal/sign-in/browser")
                .insert_header(("host", "192.168.1.25:1993"))
                .set_json(json!({ "redirect_uri": callback })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body.to_string().contains("point at this server"), "{body}");

        let (status, body) = call!(
            &app,
            test::TestRequest::post()
                .uri("/providers/tidal/sign-in/browser")
                .insert_header(("host", "music.example.test"))
                .set_json(json!({ "redirect_uri": callback })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["sandbox"], false);
        let url = body["authorize_url"].as_str().expect("url");
        assert!(url.starts_with("https://login.tidal.com/authorize?response_type=code&"));
        assert!(url.contains("client_id=dashboard%2Dclient"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains(
            "redirect_uri=https%3A%2F%2Fmusic%2Eexample%2Etest%2Fsettings%2Fservices%2Ftidal%2Dcallback"
        ));
        let state = url
            .split('&')
            .find_map(|pair| pair.strip_prefix("state="))
            .expect("state")
            .replace("%2D", "-")
            .replace("%5F", "_");
        let challenge = url
            .split('&')
            .find_map(|pair| pair.strip_prefix("code_challenge="))
            .expect("challenge")
            .replace("%2D", "-")
            .replace("%5F", "_");

        // Another listener cannot finish this sign-in.
        let other = app_as!(data, 2, "user");
        let (status, _) = call!(
            &other,
            test::TestRequest::post()
                .uri("/providers/tidal/sign-in/browser/finish")
                .set_json(json!({ "state": state, "code": "abc" })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, body) = call!(
            &app,
            test::TestRequest::post()
                .uri("/providers/tidal/sign-in/browser/finish")
                .set_json(json!({ "state": state, "code": "returned-code" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["status"], "signed_in");
        assert_eq!(body["account_name"], "recorded.listener");

        let exchange = transport
            .requests()
            .into_iter()
            .rev()
            .find(|request| request.form_value("grant_type") == Some("authorization_code"))
            .expect("token exchange");
        assert_eq!(exchange.form_value("code"), Some("returned-code"));
        assert_eq!(exchange.form_value("redirect_uri"), Some(callback));
        assert_eq!(exchange.form_value("client_id"), Some("dashboard-client"));
        let verifier = exchange.form_value("code_verifier").expect("verifier");
        assert!(verifier.len() >= 43, "RFC 7636 verifier length");
        assert_eq!(crate::providers::tidal::code_challenge(verifier), challenge);

        let (status, _) = call!(
            &app,
            test::TestRequest::post()
                .uri("/providers/tidal/sign-in/browser/finish")
                .set_json(json!({ "state": state, "code": "returned-code" })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "a state works once");
    }

    #[actix_web::test]
    async fn sandbox_browser_sign_in_comes_straight_back() {
        let pool = pool();
        let (hub, _) = hub(pool, true);
        hub.save_credentials(1, ProviderId::Tidal, Some("sandbox".into()), None, false)
            .await
            .expect("credentials");
        let callback = "http://127.0.0.1:1993/settings/services/tidal-callback";
        let sign_in = hub
            .start_tidal_browser_sign_in(1, callback, &["http://127.0.0.1:1993".to_string()])
            .await
            .expect("start");
        assert!(sign_in.sandbox);
        let (path, query) = sign_in.authorize_url.split_once('?').expect("query");
        assert_eq!(path, callback);
        let state = query
            .split('&')
            .find_map(|pair| pair.strip_prefix("state="))
            .expect("state");
        let status = hub
            .finish_tidal_browser_sign_in(1, state, "sandbox-authorization-code")
            .await
            .expect("finish");
        assert_eq!(
            serde_json::to_value(&status).expect("status")["status"],
            "signed_in"
        );
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
            "Mujin Music never writes to service playlists"
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

    /// Asserts a live check proved what Parson claims: the whole track
    /// arrived (not a 30-second preview) in the announced format.
    fn assert_live_stream(service: &str, check: &crate::providers::hub::ConnectionCheck) {
        println!(
            "{}",
            serde_json::to_string_pretty(check).unwrap_or_default()
        );
        assert!(check.ok, "{service} check failed: {:?}", check.steps);
        let record = crate::providers::hub::AccountCheck::from_check(check);
        assert!(
            record.verified,
            "{service}: not a full-length lossless stream in the announced format: {record:?}"
        );
        let delivery = check.delivery.as_ref().expect("delivery");
        assert!(
            !delivery.sandbox,
            "{service}: a live check never uses the sandbox"
        );
        assert!(
            delivery.matches_announced,
            "{service}: {:?}",
            delivery.mismatches
        );
        assert_eq!(
            delivery.full_length,
            Some(true),
            "{service}: delivered {:?} s of a {:?} s track",
            delivery.delivered_seconds,
            delivery.track_seconds
        );
    }

    /// Prints what a live run proved and, with `PARSON_LIVE_RESULT_DIR`,
    /// writes it as `<service>.json` (no tokens or credentials, only the
    /// account check), so the workflow keeps the record of each run.
    fn record_live_result(service: &str, check: &crate::providers::hub::ConnectionCheck) {
        let record = crate::providers::hub::AccountCheck::from_check(check);
        let json = serde_json::to_string_pretty(&json!({
            "service": service,
            "plan": check.plan,
            "check": record,
        }))
        .expect("json");
        println!("{service} live result: {json}");
        if let Some(dir) = live("PARSON_LIVE_RESULT_DIR") {
            let dir = std::path::PathBuf::from(dir);
            std::fs::create_dir_all(&dir).expect("result dir");
            std::fs::write(
                dir.join(format!("{}.json", service.to_ascii_lowercase())),
                json,
            )
            .expect("write result");
        }
    }

    /// End-to-end check against real accounts. No accounts exist in this
    /// repository's sandboxes, so it is ignored by default; the
    /// `live-streaming` workflow runs it when the repository has secrets.
    /// Run it with your own subscription:
    ///
    /// ```sh
    /// PARSON_LIVE_QOBUZ_APP_ID=… PARSON_LIVE_QOBUZ_APP_SECRET=… \
    /// PARSON_LIVE_QOBUZ_EMAIL=… PARSON_LIVE_QOBUZ_PASSWORD=… \
    /// PARSON_LIVE_TIDAL_CLIENT_ID=… [PARSON_LIVE_TIDAL_REFRESH_TOKEN=…] \
    ///   cargo test -p parson-music live_accounts -- --ignored --nocapture
    /// ```
    ///
    /// Each service signs in, syncs favorites, resolves a favorite (or
    /// `PARSON_LIVE_*_TRACK`) at the highest quality and reads the real
    /// stream's bytes: the delivered format must match the announced one
    /// and the stated length must be the catalog length, so a 30-second
    /// preview fails. TIDAL uses `PARSON_LIVE_TIDAL_REFRESH_TOKEN` when set
    /// (unattended); otherwise it prints a link to approve within five
    /// minutes and, outside CI, the refresh token to save as that secret.
    ///
    /// With `PARSON_LIVE_KEY_SERVICE_URL` (a Parson key service) instead of
    /// app or client ids, it signs in the way a subscriber does: a Qobuz
    /// email and password only, and TIDAL by refresh token or, when the
    /// service's client allows it, a device code.
    #[actix_web::test]
    #[ignore = "needs real TIDAL or Qobuz accounts; see the doc comment"]
    async fn live_accounts_sign_in_sync_favorites_and_stream_hi_res() {
        let pool = pool();
        let transport =
            Arc::new(crate::providers::transport::ReqwestTransport::new().expect("http client"));
        let key_service = live("PARSON_LIVE_KEY_SERVICE_URL");
        let hub = StreamingHub::new(
            pool,
            TokenVault::new([5; 32]),
            transport,
            false,
            HashMap::new(),
        )
        .with_key_service(key_service.clone())
        .with_media(Arc::new(
            crate::providers::media::ReqwestMedia::new().expect("media client"),
        ));
        let mut ran = false;
        let qobuz_app = live("PARSON_LIVE_QOBUZ_APP_ID").zip(live("PARSON_LIVE_QOBUZ_APP_SECRET"));
        if let (true, Some(email), Some(password)) = (
            qobuz_app.is_some() || key_service.is_some(),
            live("PARSON_LIVE_QOBUZ_EMAIL"),
            live("PARSON_LIVE_QOBUZ_PASSWORD"),
        ) {
            ran = true;
            if let Some((app, secret)) = qobuz_app {
                hub.save_credentials(1, ProviderId::Qobuz, Some(app), Some(secret), false)
                    .await
                    .expect("qobuz credentials");
            }
            hub.qobuz_sign_in(1, &email, &password)
                .await
                .expect("qobuz sign-in");
            let check = hub
                .check_connection(1, ProviderId::Qobuz, live("PARSON_LIVE_QOBUZ_TRACK"))
                .await
                .expect("qobuz check");
            record_live_result("Qobuz", &check);
            assert_live_stream("Qobuz", &check);
            let delivered = check.delivery.and_then(|delivery| delivery.delivered);
            assert!(delivered.is_some_and(|format| format.lossless));
        }
        let tidal_client = live("PARSON_LIVE_TIDAL_CLIENT_ID");
        if (tidal_client.is_some() || key_service.is_some())
            && live("PARSON_LIVE_TIDAL_SKIP").is_none()
        {
            ran = true;
            if let Some(client) = tidal_client {
                hub.save_credentials(
                    1,
                    ProviderId::Tidal,
                    Some(client),
                    live("PARSON_LIVE_TIDAL_CLIENT_SECRET"),
                    false,
                )
                .await
                .expect("tidal credentials");
            }
            if let Some(token) = live("PARSON_LIVE_TIDAL_REFRESH_TOKEN") {
                hub.tidal_sign_in_with_refresh_token(1, &token)
                    .await
                    .expect("TIDAL refresh-token sign-in");
            } else {
                let pending = hub.start_tidal_sign_in(1).await.expect("device code");
                println!(
                    "Approve Mujin Music on TIDAL: {} (code {})",
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
                if std::env::var_os("CI").is_none()
                    && let Ok((context, _)) = hub.context(1, ProviderId::Tidal).await
                    && let Some(token) = context.session.refresh_token
                {
                    println!(
                        "Save this as the PARSON_LIVE_TIDAL_REFRESH_TOKEN secret to run unattended: {token}"
                    );
                }
            }
            let check = hub
                .check_connection(1, ProviderId::Tidal, live("PARSON_LIVE_TIDAL_TRACK"))
                .await
                .expect("tidal check");
            record_live_result("TIDAL", &check);
            assert_live_stream("TIDAL", &check);
            let delivery = check.delivery.expect("delivery");
            if delivery.requested == crate::providers::StreamQuality::HiResMax
                && delivery.container == Some("mp4")
            {
                // Max arrives as DASH FLAC; a 24-bit master must say so.
                let format = delivery.delivered.expect("format");
                assert_eq!(format.codec, "flac");
                assert!(format.bit_depth.is_some_and(|bits| bits >= 16));
            }
        }
        assert!(
            ran,
            "set PARSON_LIVE_QOBUZ_*, PARSON_LIVE_TIDAL_CLIENT_ID or PARSON_LIVE_KEY_SERVICE_URL"
        );
    }

    const KEY_SERVICE: &str = "https://keys.parson.test";

    fn partner_keys() -> crate::providers::key_service::ServiceKeys {
        crate::providers::key_service::ServiceKeys {
            issuer: "Mujin Music".into(),
            public_url: KEY_SERVICE.into(),
            tidal_client_id: Some("parson-tidal-client".into()),
            tidal_device_code: false,
            qobuz_app_id: Some("100000001".into()),
            qobuz_app_secret: Some("partner-secret".into()),
        }
    }

    /// A hub whose server has no keys of its own and uses a key service,
    /// played by the real service code in-process.
    fn key_service_hub(
        pool: DbPool,
    ) -> (
        StreamingHub,
        Arc<crate::providers::key_service::InProcessKeyService<FixtureTransport>>,
    ) {
        let transport = Arc::new(crate::providers::key_service::InProcessKeyService::new(
            partner_keys(),
            crate::providers::sandbox::transport(),
        ));
        let hub = StreamingHub::new(
            pool,
            TokenVault::new([5; 32]),
            transport.clone(),
            false,
            HashMap::new(),
        )
        .with_key_service(Some(KEY_SERVICE.into()));
        (hub, transport)
    }

    fn url_value(url: &str, key: &str) -> String {
        let raw = url
            .split(['?', '&'])
            .find_map(|pair| pair.strip_prefix(&format!("{key}=")))
            .unwrap_or_default();
        percent_encoding::percent_decode_str(raw)
            .decode_utf8_lossy()
            .into_owned()
    }

    #[actix_web::test]
    async fn a_subscriber_signs_in_to_qobuz_with_only_email_and_password() {
        let pool = pool();
        let (hub, transport) = key_service_hub(pool.clone());
        let media = Arc::new(FixtureMedia::new().with(QOBUZ_HIRES_URL, HIRES_192_FLAC.to_vec()));
        let data = web::Data::new(hub.with_media(media));
        let listener = app_as!(data, 2, "user");

        // Nobody configured anything: the key service supplies the app.
        let (_, body) = call!(&listener, test::TestRequest::get().uri("/providers")).await;
        let qobuz = &body["providers"][1];
        assert_eq!(qobuz["id"], "qobuz");
        assert_eq!(qobuz["server_credentials_available"], true, "{body}");
        assert_eq!(qobuz["server_credentials_source"], "key_service");
        assert_eq!(qobuz["key_service"]["issuer"], "Mujin Music");
        assert_eq!(qobuz["key_service"]["offers"], true);
        assert_eq!(qobuz["key_service"]["error"], Value::Null);
        assert_eq!(qobuz["credentials"]["configured"], false);

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

        let response = test::call_service(
            &listener,
            test::TestRequest::get()
                .uri("/media/songs/qobuz:7700001/stream")
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let requests = transport.inner().requests();
        let file_url = requests
            .iter()
            .find(|request| request.url.ends_with("track/getFileUrl"))
            .expect("getFileUrl");
        assert_eq!(file_url.header_value("x-app-id"), Some("100000001"));
        let timestamp: i64 = file_url
            .query_value("request_ts")
            .and_then(|value| value.parse().ok())
            .expect("timestamp");
        // Signed by the service with the partner secret, which never
        // reached this server.
        assert_eq!(
            file_url.query_value("request_sig"),
            Some(
                crate::providers::qobuz::file_url_signature(
                    27,
                    "7700001",
                    timestamp,
                    "partner-secret"
                )
                .as_str()
            )
        );
        assert!(
            !format!("{requests:?}").contains("partner-secret"),
            "the secret is never sent from this server"
        );
        let sealed: Vec<Vec<u8>> = {
            use diesel::RunQueryDsl;
            #[derive(diesel::QueryableByName)]
            struct Blob {
                #[diesel(sql_type = diesel::sql_types::Binary)]
                sealed_credentials: Vec<u8>,
            }
            let mut connection = pool.get().expect("connection");
            diesel::sql_query("SELECT sealed_credentials FROM provider_account")
                .load::<Blob>(&mut connection)
                .expect("rows")
                .into_iter()
                .map(|row| row.sealed_credentials)
                .collect()
        };
        let opened = sealed
            .iter()
            .filter_map(|blob| {
                TokenVault::new([5; 32])
                    .open::<crate::providers::ClientCredentials>(
                        2,
                        ProviderId::Qobuz,
                        crate::providers::vault::SealPurpose::Credentials,
                        blob,
                    )
                    .ok()
            })
            .collect::<Vec<_>>();
        assert_eq!(opened.len(), 1);
        assert_eq!(opened[0].client_id, "@server", "only a marker is stored");
    }

    #[actix_web::test]
    async fn a_subscriber_signs_in_to_tidal_through_the_key_service_relay() {
        let pool = pool();
        let (hub, transport) = key_service_hub(pool);
        let data = web::Data::new(hub);
        let listener = app_as!(data, 2, "user");
        let callback = "https://music.example.test/settings/services/tidal-callback";

        let (_, body) = call!(&listener, test::TestRequest::get().uri("/providers")).await;
        assert_eq!(
            body["providers"][0]["server_credentials_source"],
            "key_service"
        );
        assert_eq!(body["providers"][0]["key_service"]["device_code"], false);

        let (status, body) = call!(
            &listener,
            test::TestRequest::post()
                .uri("/providers/tidal/sign-in/browser")
                .insert_header(("host", "music.example.test"))
                .set_json(json!({ "redirect_uri": callback })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let url = body["authorize_url"].as_str().expect("url");
        assert!(url.starts_with("https://login.tidal.com/authorize?"));
        assert_eq!(url_value(url, "client_id"), "parson-tidal-client");
        let relay = format!("{KEY_SERVICE}/v1/tidal/callback");
        assert_eq!(url_value(url, "redirect_uri"), relay);
        assert_eq!(body["redirect_uri"], relay.as_str());

        // TIDAL sends the listener to the relay, which forwards the code to
        // this server's callback named in the state.
        let state = url_value(url, "state");
        let location = crate::providers::key_service::relay_location(&[
            ("code".to_string(), "relayed-code".to_string()),
            ("state".to_string(), state.clone()),
        ])
        .expect("relay");
        assert!(location.starts_with(&format!("{callback}?code=relayed-code&state=")));
        assert_eq!(url_value(&location, "state"), state);

        let (status, body) = call!(
            &listener,
            test::TestRequest::post()
                .uri("/providers/tidal/sign-in/browser/finish")
                .set_json(json!({ "state": state, "code": "relayed-code" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["status"], "signed_in");
        assert_eq!(body["credentials"]["uses_server_credentials"], true);
        let exchange = transport
            .inner()
            .requests()
            .into_iter()
            .rev()
            .find(|request| request.form_value("grant_type") == Some("authorization_code"))
            .expect("token exchange");
        assert_eq!(
            exchange.form_value("client_id"),
            Some("parson-tidal-client")
        );
        assert_eq!(exchange.form_value("client_secret"), None);
        assert_eq!(
            exchange.form_value("redirect_uri"),
            Some(relay.as_str()),
            "TIDAL checks the exchange against the registered relay URI"
        );
    }

    #[actix_web::test]
    async fn the_owner_overrides_the_key_service_and_an_outage_is_reported() {
        let pool = pool();
        let (hub, _) = key_service_hub(pool.clone());
        let data = web::Data::new(hub);
        let admin = app_as!(data, 1, "admin");
        let (status, _) = call!(
            &admin,
            test::TestRequest::put()
                .uri("/providers/qobuz/server-credentials")
                .set_json(json!({"client_id": "100000001", "client_secret": "owner-secret"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (_, body) = call!(&admin, test::TestRequest::get().uri("/providers")).await;
        assert_eq!(
            body["providers"][1]["server_credentials_source"],
            "settings"
        );
        assert_eq!(
            body["providers"][0]["server_credentials_source"],
            "key_service"
        );

        // A key service that cannot be reached offers nothing, says why,
        // and the listener is asked for credentials as before.
        let unreachable = StreamingHub::new(
            pool,
            TokenVault::new([5; 32]),
            Arc::new(crate::providers::sandbox::transport()),
            false,
            HashMap::new(),
        )
        .with_key_service(Some("https://down.parson.test".into()));
        let status = unreachable
            .provider_status(2, ProviderId::Qobuz)
            .await
            .expect("status");
        assert!(!status.server_credentials_available);
        let key_service = status.key_service.expect("key service");
        assert!(!key_service.offers);
        assert!(key_service.error.is_some());
        assert!(matches!(
            unreachable
                .qobuz_sign_in(2, "listener@example.test", "pw")
                .await,
            Err(crate::providers::hub::HubError::CredentialsRequired(
                ProviderId::Qobuz
            ))
        ));
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
