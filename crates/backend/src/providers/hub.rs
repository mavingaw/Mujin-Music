//! Per-user orchestration: sealed credentials, sign-in flows, token refresh,
//! library add/remove/sync and stream resolution for every provider.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

use diesel::sqlite::SqliteConnection;
use serde::Serialize;

use super::artwork::ArtworkMode;
use super::key_service::{self, InProcessKeyService, KeyServiceManifest, ServiceKeys};
use super::media::{self, MediaTransport, ReqwestMedia};
use super::probe::{PROBE_BYTES, ProbedFormat};
use super::qobuz::QobuzClient;
use super::store::{
    self, AccountRow, AccountStatus, ItemOrigin, LibraryCounts, LibraryItemRow, LibraryQuery,
    NewLibraryItem,
};
use super::tidal::{BrowserAuthorization, DeviceAuthorization, DevicePoll, TidalClient};
use super::transport::{ReqwestTransport, Transport};
use super::vault::{SealPurpose, TokenVault};
use super::{
    AudioFormat, BrowsePage, BrowseSection, ClientCredentials, EditorialSection, ItemKind,
    ProviderAlbum, ProviderArtist, ProviderContext, ProviderError, ProviderGenre, ProviderId,
    ProviderPlaylist, ProviderSession, ProviderTrack, QualityOption, RADIO_TRACKS, Radio,
    RadioSeed, ResolvedStream, SearchResults, SourcePreference, StreamQuality, StreamSource,
    StreamingProvider, sandbox, sandbox_media, unix_now,
};
use crate::persistence::connection::DbPool;

/// Marker stored instead of credentials when the listener uses the
/// credentials the server owner configured through environment variables.
const SERVER_CREDENTIALS_MARKER: &str = "@server";
const REFRESH_MARGIN_SECONDS: i64 = 120;
const MAX_SYNC_ITEMS: u32 = 2_000;
const SYNC_PAGE: u32 = 100;
const MAX_CREDENTIAL_CHARACTERS: usize = 256;
/// Server-wide credentials are sealed under this user id, which no account
/// can have (SQLite ids start at 1).
const SERVER_SCOPE_USER: i32 = 0;
const MAX_DELIVERIES: usize = 512;
/// DASH segment-size indexes kept so a seek does not re-measure the track.
const MAX_SEGMENT_INDEXES: usize = 256;

#[derive(Debug)]
pub enum HubError {
    Provider(ProviderError),
    Database(String),
    Invalid(&'static str),
    /// Neither the listener nor the server has credentials for this service.
    CredentialsRequired(ProviderId),
    /// The change would overwrite something the listener did, and was not
    /// confirmed.
    Conflict(&'static str),
}

impl From<ProviderError> for HubError {
    fn from(error: ProviderError) -> Self {
        Self::Provider(error)
    }
}

impl std::fmt::Display for HubError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Provider(error) => error.fmt(formatter),
            Self::Database(error) => write!(formatter, "database error: {error}"),
            Self::Invalid(message) => formatter.write_str(message),
            Self::CredentialsRequired(provider) => {
                write!(formatter, "no {} credentials", provider.display_name())
            }
            Self::Conflict(message) => formatter.write_str(message),
        }
    }
}

pub type HubResult<T> = Result<T, HubError>;

#[derive(Clone, Debug, Serialize)]
pub struct CredentialSummary {
    pub configured: bool,
    /// The last characters of the client or app id, for recognition only.
    pub client_id_hint: Option<String>,
    pub has_secret: bool,
    pub uses_server_credentials: bool,
}

/// A browser sign-in to finish on TIDAL's login page.
#[derive(Clone, Debug, Serialize)]
pub struct BrowserSignIn {
    pub authorize_url: String,
    pub redirect_uri: String,
    pub expires_at: i64,
    /// Sandbox sign-ins skip TIDAL's page and come straight back.
    pub sandbox: bool,
}

/// The path TIDAL sends a browser sign-in back to. The web app finishes the
/// sign-in there, so this is the redirect URI to register with TIDAL.
pub const TIDAL_CALLBACK_PATH: &str = "/settings/services/tidal-callback";
const MAX_BROWSER_SIGN_INS: usize = 512;

/// Checks a browser sign-in redirect URI. It must be exactly one of
/// `origins` (this server's own addresses: the one the browser used, and
/// the configured public URL) followed by [`TIDAL_CALLBACK_PATH`], so a
/// sign-in can never hand TIDAL's authorization code to another host.
fn validated_redirect_uri(value: &str, origins: &[String]) -> HubResult<String> {
    let value = value.trim();
    let invalid = HubError::Invalid(
        "The redirect URI must be this server's address followed by /settings/services/tidal-callback.",
    );
    let Some(origin) = value.strip_suffix(TIDAL_CALLBACK_PATH) else {
        return Err(invalid);
    };
    let well_formed = value.len() <= 512
        && origin_is_well_formed(origin)
        && value.chars().all(|character| character.is_ascii_graphic());
    if !well_formed {
        return Err(invalid);
    }
    let ours = origins
        .iter()
        .any(|allowed| allowed.trim_end_matches('/').eq_ignore_ascii_case(origin));
    if ours {
        Ok(value.to_string())
    } else {
        Err(HubError::Invalid(
            "The redirect URI must point at this server, the address you opened Mujin Music on.",
        ))
    }
}

/// `http(s)://host[:port]` with no path, query, fragment or user info.
fn origin_is_well_formed(origin: &str) -> bool {
    let Some(authority) = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
    else {
        return false;
    };
    !authority.is_empty()
        && !authority.contains(['/', '?', '#', '@', '\\'])
        && !authority.starts_with(':')
}

#[derive(Clone, Debug, Serialize)]
pub struct PendingSignIn {
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_at: i64,
    pub interval: u64,
}

impl From<&DeviceAuthorization> for PendingSignIn {
    fn from(value: &DeviceAuthorization) -> Self {
        Self {
            user_code: value.user_code.clone(),
            verification_uri: value.verification_uri.clone(),
            verification_uri_complete: value.verification_uri_complete.clone(),
            expires_at: value.expires_at,
            interval: value.interval,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ProviderStatus {
    pub id: ProviderId,
    pub name: &'static str,
    /// `device_code` (TIDAL) or `password` (Qobuz).
    pub sign_in: &'static str,
    pub status: AccountStatus,
    pub status_message: Option<String>,
    pub account_name: Option<String>,
    pub plan: Option<String>,
    pub quality: StreamQuality,
    pub quality_options: &'static [QualityOption],
    /// The service's editorial album lists, in its own app's order.
    pub browse_sections: &'static [EditorialSection],
    pub last_library_sync: Option<i64>,
    pub credentials: CredentialSummary,
    pub server_credentials_available: bool,
    /// Where the server's credentials for this service come from.
    pub server_credentials_source: Option<CredentialSource>,
    /// The Parson key service this server uses, if any.
    pub key_service: Option<KeyServiceStatus>,
    pub pending: Option<PendingSignIn>,
    pub library: LibraryCounts,
    pub sandbox: bool,
    /// The last "Check connection" on this account, kept so Settings can
    /// say whether this account itself has been proven.
    pub last_check: Option<AccountCheck>,
}

/// What the last "Check connection" proved about one account. Only a check
/// against the real service (not the sandbox) that received the whole track
/// in the lossless format the service announced counts as `verified`.
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct AccountCheck {
    pub checked_at: i64,
    pub ok: bool,
    pub sandbox: bool,
    pub verified: bool,
    /// The step that failed: `session`, `favorites` or `stream`.
    pub failed_step: Option<String>,
    pub detail: String,
    pub requested: Option<StreamQuality>,
    pub announced: Option<String>,
    pub delivered: Option<String>,
    pub lossless: Option<bool>,
    pub matches_announced: Option<bool>,
    pub full_length: Option<bool>,
    pub delivered_seconds: Option<f64>,
    pub track_seconds: Option<f64>,
    pub track_title: Option<String>,
}

impl AccountCheck {
    pub fn from_check(check: &ConnectionCheck) -> Self {
        let failed = check.steps.iter().find(|step| !step.ok);
        let delivery = check.delivery.as_ref();
        let delivered = delivery.and_then(|delivery| delivery.delivered.as_ref());
        let verified = check.ok
            && !check.sandbox
            && delivery.is_some_and(|delivery| {
                !delivery.sandbox
                    && delivery.matches_announced
                    && delivery.full_length == Some(true)
            })
            && delivered.is_some_and(|format| format.lossless);
        Self {
            checked_at: check.checked_at,
            ok: check.ok,
            sandbox: check.sandbox,
            verified,
            failed_step: failed.map(|step| step.id.to_string()),
            detail: failed
                .or(check.steps.last())
                .map(|step| step.detail.clone())
                .unwrap_or_default(),
            requested: delivery.map(|delivery| delivery.requested),
            announced: delivery.map(|delivery| delivery.announced.label.clone()),
            delivered: delivered.map(|format| format.label.clone()),
            lossless: delivered.map(|format| format.lossless),
            matches_announced: delivery.map(|delivery| delivery.matches_announced),
            full_length: delivery.and_then(|delivery| delivery.full_length),
            delivered_seconds: delivery.and_then(|delivery| delivery.delivered_seconds),
            track_seconds: delivery.and_then(|delivery| delivery.track_seconds),
            track_title: check.track.as_ref().map(|track| track.title.clone()),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PollOutcome {
    pub status: &'static str,
    pub interval: u64,
    pub provider: Option<ProviderStatus>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SyncReport {
    pub albums_added: usize,
    pub albums_removed: usize,
    pub tracks_added: usize,
    pub tracks_removed: usize,
    pub synced_at: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct LibraryChange {
    pub in_library: bool,
    /// Whether the change also reached the service's favorites.
    pub service_updated: bool,
}

#[derive(Debug, Serialize)]
pub struct ProviderSearch {
    pub provider: ProviderId,
    pub name: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub results: Option<SearchResults>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A genre page: the genre and its albums.
#[derive(Debug, Serialize)]
pub struct GenreAlbums {
    pub genre: ProviderGenre,
    #[serde(flatten)]
    pub page: BrowsePage,
}

/// A page of a listener's streaming library.
pub struct LibraryRequest {
    pub provider: Option<ProviderId>,
    pub kind: ItemKind,
    pub text: Option<String>,
    pub sort: store::LibrarySort,
    pub offset: i64,
    pub limit: i64,
}

/// Where a server-wide credential came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialSource {
    /// `PARSON_TIDAL_CLIENT_ID` and friends; read-only in Settings.
    Environment,
    /// Saved by an admin in Settings > Services.
    Settings,
    /// Published by the Parson key service this server uses, so neither
    /// the owner nor the listener registers anything.
    KeyService,
}

#[derive(Clone, Debug)]
struct ServerCredential {
    credentials: ClientCredentials,
    source: CredentialSource,
    updated_at: Option<i64>,
}

/// What admins see about the server-wide credentials. Never the values.
#[derive(Clone, Debug, Serialize)]
pub struct ServerCredentialStatus {
    pub provider: ProviderId,
    pub name: &'static str,
    pub configured: bool,
    pub source: Option<CredentialSource>,
    pub client_id_hint: Option<String>,
    pub has_secret: bool,
    pub updated_at: Option<i64>,
}

/// The format a listener's stream actually delivered, read from its bytes
/// by the media proxy, beside what the service announced.
#[derive(Clone, Debug, Serialize)]
pub struct DeliveredStream {
    pub track_id: String,
    pub requested: StreamQuality,
    /// What the service said it would send.
    pub announced: AudioFormat,
    /// What arrived, read from the stream headers. `None` when the bytes
    /// could not be read as a known format.
    pub delivered: Option<AudioFormat>,
    pub container: Option<&'static str>,
    /// Whether the first audio frame was parsed and agrees with the header.
    pub frame_verified: bool,
    /// Whether the delivered format agrees with the announced one: the
    /// codec, and the bit depth and sample rate for lossless streams or the
    /// bitrate (within 10%) for lossy ones.
    pub matches_announced: bool,
    /// What disagrees, in words, e.g. `AAC at 255 kbps, not 320`.
    pub mismatches: Vec<String>,
    /// The player's id for this play, from the stream URL's `play`
    /// parameter, so a player can ask about exactly the stream it is
    /// playing rather than an earlier one of the same track.
    pub play: Option<String>,
    /// Orders deliveries; larger is newer.
    pub seq: u64,
    /// How long the delivered stream is, from its own header (FLAC
    /// `STREAMINFO`, MP4 `mvhd`/`mehd`, WAV) or, for DASH, the segments
    /// Parson relays. `None` when neither says (MP3, ADTS).
    pub delivered_seconds: Option<f64>,
    /// The track's catalog length, when the caller knew it.
    pub track_seconds: Option<f64>,
    /// Whether the whole track arrived rather than a preview: the delivered
    /// length is within 2 seconds of the catalog length. `None` when either
    /// length is unknown, and always in the sandbox, which renders 30
    /// seconds of every track by design.
    pub full_length: Option<bool>,
    pub sandbox: bool,
    /// The sandbox's 16/44.1 placeholder tone, which announces no real
    /// format. Rendered sandbox streams are `false` and are checked like
    /// live ones.
    pub tone: bool,
    pub at: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct CheckStep {
    /// `session`, `favorites` or `stream`.
    pub id: &'static str,
    pub ok: bool,
    pub detail: String,
}

/// The result of "Check connection": sign-in, favorites sync and a real
/// stream read through the media proxy.
#[derive(Clone, Debug, Serialize)]
pub struct ConnectionCheck {
    pub provider: ProviderId,
    pub ok: bool,
    pub sandbox: bool,
    pub account_name: Option<String>,
    pub plan: Option<String>,
    pub steps: Vec<CheckStep>,
    pub sync: Option<SyncReport>,
    pub track: Option<ProviderTrack>,
    pub delivery: Option<DeliveredStream>,
    pub checked_at: i64,
}

/// A service playlist, with when the listener last saved a copy of it.
#[derive(Clone, Debug, Serialize)]
pub struct PlaylistView {
    #[serde(flatten)]
    pub playlist: ProviderPlaylist,
    pub saved_copy_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlaylistViews {
    pub playlists: Vec<PlaylistView>,
    pub total: Option<u32>,
}

/// Ids of playlists Parson made itself (from a radio or a selection) start
/// with this; they have no service playlist behind them.
pub const PARSON_PLAYLIST_PREFIX: &str = "parson-";

/// Most tracks a copy holds, and most new tracks one edit may add.
pub const MAX_COPY_TRACKS: usize = super::MAX_PLAYLIST_TRACKS as usize;
const MAX_ADDED_PER_EDIT: usize = 200;

/// A copy of a service playlist kept in Parson, editable there. The
/// service playlist is never changed.
#[derive(Clone, Debug, Serialize)]
pub struct PlaylistCopy {
    pub id: String,
    pub provider: ProviderId,
    pub title: String,
    pub owner: Option<String>,
    pub cover_url: Option<String>,
    pub track_count: i32,
    pub duration: f64,
    pub saved_at: i64,
    /// When the listener last changed it in Parson.
    pub edited_at: Option<i64>,
    /// Made in Parson (from a radio or a selection), not saved from a
    /// service playlist.
    pub made_in_parson: bool,
    /// The saved tracks; only on a single copy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist: Option<ProviderPlaylist>,
}

impl PlaylistCopy {
    fn from_row(row: store::PlaylistCopyRow, with_tracks: bool) -> Option<Self> {
        let provider = ProviderId::parse(&row.provider)?;
        let playlist = with_tracks
            .then(|| serde_json::from_str::<ProviderPlaylist>(&row.payload).ok())
            .flatten();
        Some(Self {
            id: super::entity_id(provider, &row.provider_playlist_id),
            provider,
            title: row.title,
            owner: row.owner,
            cover_url: row.cover_url,
            track_count: row.track_count,
            duration: row.duration,
            saved_at: row.saved_at,
            edited_at: row.edited_at,
            made_in_parson: row.provider_playlist_id.starts_with(PARSON_PLAYLIST_PREFIX),
            playlist,
        })
    }
}

pub struct StreamingHub {
    pool: DbPool,
    vault: TokenVault,
    tidal: TidalClient,
    qobuz: QobuzClient,
    sandbox: bool,
    /// In sandbox mode, whether streams are the 16/44.1 WAV placeholder
    /// tone (no ffmpeg) rather than rendered in each announced format.
    sandbox_tone: bool,
    server_credentials: RwLock<HashMap<ProviderId, ServerCredential>>,
    media: Arc<dyn MediaTransport>,
    /// Keyed by listener, track and play id (empty without one).
    deliveries: Mutex<HashMap<(i32, String, String), DeliveredStream>>,
    delivery_seq: std::sync::atomic::AtomicU64,
    segment_indexes: Mutex<HashMap<String, media::SegmentIndex>>,
    /// Each service's genre list per listener (genres vary by country),
    /// with when it was read.
    genre_lists: Mutex<GenreLists>,
    /// Genres learned from genre pages, by album id. TIDAL albums carry no
    /// genre, so an album found under TIDAL's "Jazz" is filed as Jazz when
    /// it is added to the library.
    genre_hints: Mutex<HashMap<String, String>>,
    /// Browser sign-ins waiting for the listener to come back, by state.
    browser_sign_ins: Mutex<HashMap<String, (i32, BrowserAuthorization)>>,
    /// For key service requests (its manifest); the provider clients hold
    /// their own handle to the same transport.
    transport: Arc<dyn Transport>,
    /// The Parson key service this server uses, if any.
    key_service_url: Option<String>,
    key_service: RwLock<KeyServiceState>,
}

/// What this server last learned from its key service.
#[derive(Clone, Debug, Default)]
struct KeyServiceState {
    manifest: Option<KeyServiceManifest>,
    /// Unix seconds of the last attempt, successful or not.
    checked_at: i64,
    error: Option<String>,
}

/// A failed manifest read is retried after this long.
const KEY_SERVICE_RETRY_SECONDS: i64 = 60;
/// A manifest is re-read after this long, in case the service rotates ids.
const KEY_SERVICE_REFRESH_SECONDS: i64 = 12 * 60 * 60;

/// What a listener's Services page says about the key service.
#[derive(Clone, Debug, Serialize)]
pub struct KeyServiceStatus {
    pub url: String,
    pub issuer: Option<String>,
    /// Whether the service holds a key for this provider.
    pub offers: bool,
    /// TIDAL only: whether the service's client may use device-code sign-in.
    pub device_code: bool,
    /// Why the service could not be read, when it could not.
    pub error: Option<String>,
}

/// Genre lists by listener and service, with when each was read.
type GenreLists = HashMap<(i32, ProviderId), (i64, Vec<ProviderGenre>)>;

/// How long a service's genre list is reused.
const GENRE_LIST_SECONDS: i64 = 6 * 60 * 60;
const MAX_GENRE_HINTS: usize = 20_000;

fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn server_credentials_from_env() -> HashMap<ProviderId, ClientCredentials> {
    let mut credentials = HashMap::new();
    if let Some(client_id) = env_value("PARSON_TIDAL_CLIENT_ID") {
        credentials.insert(
            ProviderId::Tidal,
            ClientCredentials {
                client_id,
                client_secret: env_value("PARSON_TIDAL_CLIENT_SECRET"),
                ..Default::default()
            },
        );
    }
    if let Some(client_id) = env_value("PARSON_QOBUZ_APP_ID") {
        credentials.insert(
            ProviderId::Qobuz,
            ClientCredentials {
                client_id,
                client_secret: env_value("PARSON_QOBUZ_APP_SECRET"),
                ..Default::default()
            },
        );
    }
    credentials
}

fn valid_credential(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= MAX_CREDENTIAL_CHARACTERS
        && value.chars().all(|character| character.is_ascii_graphic())
}

fn hint(value: &str) -> String {
    let tail = value
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("•••• {tail}")
}

fn validated_credentials(
    provider: ProviderId,
    client_id: Option<String>,
    client_secret: Option<String>,
) -> HubResult<ClientCredentials> {
    let client_id = client_id
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    let client_secret = client_secret
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if !valid_credential(&client_id) || client_id == SERVER_CREDENTIALS_MARKER {
        return Err(HubError::Invalid("Enter a valid client or app ID."));
    }
    if client_secret
        .as_deref()
        .is_some_and(|secret| !valid_credential(secret))
    {
        return Err(HubError::Invalid("Enter a valid client or app secret."));
    }
    if provider == ProviderId::Qobuz && client_secret.is_none() {
        return Err(HubError::Invalid(
            "Qobuz needs both an app ID and an app secret to stream.",
        ));
    }
    Ok(ClientCredentials {
        client_id,
        client_secret,
        ..Default::default()
    })
}

impl StreamingHub {
    pub fn from_env(pool: DbPool) -> Result<Self, String> {
        let key = crate::settings::streaming_token_key()
            .map_err(|error| format!("streaming token key: {error}"))?;
        let sandbox = sandbox::enabled_from_env();
        let rendered = sandbox && sandbox_media::ffmpeg_available();
        let streams = Arc::new(sandbox_media::SandboxStreams::for_sandbox());
        let transport: Arc<dyn Transport> = if rendered {
            tracing::warn!(
                "streaming services run in sandbox mode: recorded responses, synthesized audio"
            );
            Arc::new(InProcessKeyService::new(
                ServiceKeys::sandbox(),
                sandbox_media::SandboxTransport::new(sandbox::transport(), streams.clone()),
            ))
        } else if sandbox {
            tracing::warn!(
                "streaming services run in sandbox mode with recorded responses; \
                 ffmpeg is missing, so streams are a 16/44.1 placeholder tone"
            );
            Arc::new(InProcessKeyService::new(
                ServiceKeys::sandbox(),
                sandbox::transport(),
            ))
        } else {
            Arc::new(ReqwestTransport::new().map_err(|error| error.to_string())?)
        };
        // The sandbox simulates a key service so its listeners see the
        // subscriber-only sign-in; a real server uses the configured one.
        let key_service_url = if sandbox {
            Some(key_service::SANDBOX_KEY_SERVICE_URL.to_string())
        } else {
            key_service::configured_url()
        };
        let hub = Self::new(
            pool,
            TokenVault::new(key),
            transport,
            sandbox,
            server_credentials_from_env(),
        )
        .with_key_service(key_service_url);
        let hub = if rendered {
            hub.with_rendered_sandbox(Arc::new(sandbox_media::SandboxMedia::new(streams)))
        } else {
            hub.with_media(Arc::new(
                ReqwestMedia::new().map_err(|error| error.to_string())?,
            ))
        };
        hub.load_server_credentials()?;
        Ok(hub)
    }

    pub fn new(
        pool: DbPool,
        vault: TokenVault,
        transport: Arc<dyn Transport>,
        sandbox: bool,
        server_credentials: HashMap<ProviderId, ClientCredentials>,
    ) -> Self {
        let artwork = if sandbox {
            ArtworkMode::Sandbox
        } else {
            ArtworkMode::Live
        };
        let server_credentials = server_credentials
            .into_iter()
            .map(|(provider, credentials)| {
                (
                    provider,
                    ServerCredential {
                        credentials,
                        source: CredentialSource::Environment,
                        updated_at: None,
                    },
                )
            })
            .collect();
        Self {
            pool,
            vault,
            tidal: TidalClient::new(transport.clone(), artwork),
            qobuz: QobuzClient::new(transport.clone(), artwork),
            transport,
            key_service_url: None,
            key_service: RwLock::new(KeyServiceState::default()),
            sandbox,
            sandbox_tone: sandbox,
            server_credentials: RwLock::new(server_credentials),
            media: Arc::new(media::FixtureMedia::new()),
            deliveries: Mutex::new(HashMap::new()),
            delivery_seq: std::sync::atomic::AtomicU64::new(0),
            segment_indexes: Mutex::new(HashMap::new()),
            genre_lists: Mutex::new(HashMap::new()),
            genre_hints: Mutex::new(HashMap::new()),
            browser_sign_ins: Mutex::new(HashMap::new()),
        }
    }

    /// Uses a Parson key service (its base URL) for TIDAL and Qobuz keys
    /// when neither the owner nor the listener has their own.
    pub fn with_key_service(mut self, url: Option<String>) -> Self {
        self.key_service_url = url.and_then(|url| key_service::normalized_service_url(&url));
        self
    }

    /// Reads the key service's manifest when it has not been read yet, or
    /// is due for a refresh. Cheap when there is nothing to do.
    async fn ensure_key_service(&self) {
        let Some(url) = self.key_service_url.clone() else {
            return;
        };
        let now = unix_now();
        let due = self
            .key_service
            .read()
            .map(|state| {
                let age = now - state.checked_at;
                if state.manifest.is_some() {
                    age >= KEY_SERVICE_REFRESH_SECONDS
                } else {
                    age >= KEY_SERVICE_RETRY_SECONDS
                }
            })
            .unwrap_or(false);
        if !due {
            return;
        }
        if let Ok(mut state) = self.key_service.write() {
            // Claim this attempt so concurrent requests do not all fetch.
            state.checked_at = now;
        }
        let result = match self
            .transport
            .send(super::transport::ApiRequest::get(format!("{url}/v1/apps")))
            .await
        {
            Ok(reply) if (200..300).contains(&reply.status) => {
                key_service::parse_manifest(&reply.body)
            }
            Ok(reply) => Err(format!("it answered HTTP {}", reply.status)),
            Err(error) => Err(error.to_string()),
        };
        if let Ok(mut state) = self.key_service.write() {
            match result {
                Ok(manifest) => {
                    state.manifest = Some(manifest);
                    state.error = None;
                }
                Err(error) => {
                    tracing::warn!(%error, "could not read the Mujin Music key service");
                    // A refresh failure keeps the last good manifest.
                    state.error = Some(error);
                }
            }
        }
    }

    fn key_service_manifest(&self) -> Option<KeyServiceManifest> {
        self.key_service
            .read()
            .ok()
            .and_then(|state| state.manifest.clone())
    }

    /// The credentials the key service publishes for a provider.
    fn key_service_credential(&self, provider: ProviderId) -> Option<ServerCredential> {
        let manifest = self.key_service_manifest()?;
        let credentials = match provider {
            ProviderId::Tidal => ClientCredentials {
                client_id: manifest.tidal?.client_id,
                client_secret: None,
                signing_service: None,
            },
            ProviderId::Qobuz => ClientCredentials {
                client_id: manifest.qobuz?.app_id,
                client_secret: None,
                signing_service: self.key_service_url.clone(),
            },
        };
        Some(ServerCredential {
            credentials,
            source: CredentialSource::KeyService,
            updated_at: None,
        })
    }

    fn key_service_status(&self, provider: ProviderId) -> Option<KeyServiceStatus> {
        let url = self.key_service_url.clone()?;
        let state = self.key_service.read().ok()?.clone();
        let manifest = state.manifest.as_ref();
        Some(KeyServiceStatus {
            url,
            issuer: manifest.map(|manifest| manifest.issuer.clone()),
            offers: match provider {
                ProviderId::Tidal => manifest.is_some_and(|manifest| manifest.tidal.is_some()),
                ProviderId::Qobuz => manifest.is_some_and(|manifest| manifest.qobuz.is_some()),
            },
            device_code: provider == ProviderId::Tidal
                && manifest
                    .and_then(|manifest| manifest.tidal.as_ref())
                    .is_some_and(|tidal| tidal.device_code),
            error: state.error,
        })
    }

    /// Whether this listener's account signs in with the key service's
    /// keys (it uses the server's credentials, and those come from there).
    fn uses_key_service(&self, user_id: i32, provider: ProviderId, row: &AccountRow) -> bool {
        let uses_server = self
            .vault
            .open::<ClientCredentials>(
                user_id,
                provider,
                SealPurpose::Credentials,
                &row.sealed_credentials,
            )
            .is_ok_and(|stored| stored.client_id == SERVER_CREDENTIALS_MARKER);
        uses_server
            && self
                .server_credential(provider)
                .is_some_and(|credential| credential.source == CredentialSource::KeyService)
    }

    /// Replaces how stream bytes are fetched (live HTTP in the server,
    /// recorded media files in tests).
    pub fn with_media(mut self, media: Arc<dyn MediaTransport>) -> Self {
        self.media = media;
        self
    }

    /// Sandbox streams go through the services' clients and this media
    /// transport, rendered in each announced format, instead of the
    /// placeholder tone.
    pub fn with_rendered_sandbox(mut self, media: Arc<dyn MediaTransport>) -> Self {
        self.sandbox_tone = false;
        self.media = media;
        self
    }

    pub fn media(&self) -> Arc<dyn MediaTransport> {
        self.media.clone()
    }

    /// The segment sizes of a DASH stream, measured once per track and
    /// quality. Signed segment URLs change between resolutions but the
    /// files behind them do not, so the index outlives the URLs.
    pub async fn segment_index(
        &self,
        key: String,
        urls: &[String],
    ) -> Result<media::SegmentIndex, media::MediaError> {
        let cached = self
            .segment_indexes
            .lock()
            .ok()
            .and_then(|indexes| indexes.get(&key).cloned())
            .filter(|index| index.segments() == urls.len());
        if let Some(index) = cached {
            return Ok(index);
        }
        let index = media::index_segments(self.media.clone(), urls).await?;
        if let Ok(mut indexes) = self.segment_indexes.lock() {
            if indexes.len() >= MAX_SEGMENT_INDEXES {
                indexes.clear();
            }
            indexes.insert(key, index.clone());
        }
        Ok(index)
    }

    /// Loads credentials an admin saved in Settings. Environment variables
    /// win, so a deployment's configuration cannot be overridden from the UI.
    pub fn load_server_credentials(&self) -> Result<(), String> {
        let rows = {
            let mut connection = self.pool.get().map_err(|error| error.to_string())?;
            store::server_credentials(&mut connection).map_err(|error| error.to_string())?
        };
        let mut map = self
            .server_credentials
            .write()
            .map_err(|_| "server credentials lock poisoned".to_string())?;
        for row in rows {
            let Some(provider) = ProviderId::parse(&row.provider) else {
                continue;
            };
            if map.contains_key(&provider) {
                continue;
            }
            match self.vault.open::<ClientCredentials>(
                SERVER_SCOPE_USER,
                provider,
                SealPurpose::ServerCredentials,
                &row.sealed_credentials,
            ) {
                Ok(credentials) => {
                    map.insert(
                        provider,
                        ServerCredential {
                            credentials,
                            source: CredentialSource::Settings,
                            updated_at: Some(row.updated_at),
                        },
                    );
                }
                Err(_) => tracing::warn!(
                    provider = provider.as_str(),
                    "saved server credentials could not be opened; add them again in Settings"
                ),
            }
        }
        Ok(())
    }

    /// The owner's credentials for a provider (environment, then Settings),
    /// else the key service's.
    fn server_credential(&self, provider: ProviderId) -> Option<ServerCredential> {
        self.server_credentials
            .read()
            .ok()
            .and_then(|map| map.get(&provider).cloned())
            .or_else(|| self.key_service_credential(provider))
    }

    fn has_server_credentials(&self, provider: ProviderId) -> bool {
        self.server_credential(provider).is_some()
    }

    pub fn server_credentials_status(&self) -> Vec<ServerCredentialStatus> {
        ProviderId::ALL
            .into_iter()
            .map(|provider| {
                let credential = self.server_credential(provider);
                ServerCredentialStatus {
                    provider,
                    name: provider.display_name(),
                    configured: credential.is_some(),
                    source: credential.as_ref().map(|credential| credential.source),
                    client_id_hint: credential
                        .as_ref()
                        .map(|credential| hint(&credential.credentials.client_id)),
                    has_secret: credential
                        .as_ref()
                        .is_some_and(|credential| credential.credentials.client_secret.is_some()),
                    updated_at: credential.and_then(|credential| credential.updated_at),
                }
            })
            .collect()
    }

    fn environment_owned(&self, provider: ProviderId) -> bool {
        self.server_credential(provider)
            .is_some_and(|credential| credential.source == CredentialSource::Environment)
    }

    /// Saves server-wide credentials (admins only; checked by the route).
    pub async fn save_server_credentials(
        &self,
        admin_id: i32,
        provider: ProviderId,
        client_id: Option<String>,
        client_secret: Option<String>,
    ) -> HubResult<Vec<ServerCredentialStatus>> {
        if self.environment_owned(provider) {
            return Err(HubError::Invalid(
                "These credentials are set by the server's environment variables. Change them there.",
            ));
        }
        let credentials = validated_credentials(provider, client_id, client_secret)?;
        let sealed = self
            .vault
            .seal(
                SERVER_SCOPE_USER,
                provider,
                SealPurpose::ServerCredentials,
                &credentials,
            )
            .map_err(|_| HubError::Database("could not seal credentials".into()))?;
        let updated_at = self
            .db(move |connection| {
                store::save_server_credentials(connection, provider, &sealed, admin_id)
            })
            .await?;
        if let Ok(mut map) = self.server_credentials.write() {
            map.insert(
                provider,
                ServerCredential {
                    credentials,
                    source: CredentialSource::Settings,
                    updated_at: Some(updated_at),
                },
            );
        }
        Ok(self.server_credentials_status())
    }

    pub async fn remove_server_credentials(
        &self,
        provider: ProviderId,
    ) -> HubResult<Vec<ServerCredentialStatus>> {
        if self.environment_owned(provider) {
            return Err(HubError::Invalid(
                "These credentials are set by the server's environment variables. Change them there.",
            ));
        }
        self.db(move |connection| store::remove_server_credentials(connection, provider))
            .await?;
        if let Ok(mut map) = self.server_credentials.write() {
            map.remove(&provider);
        }
        Ok(self.server_credentials_status())
    }

    /// The listener's account row, created on first sign-in with the
    /// server's credentials when the listener has none of their own. This is
    /// what lets a subscriber skip the credentials step entirely.
    async fn account_for_sign_in(
        &self,
        user_id: i32,
        provider: ProviderId,
    ) -> HubResult<AccountRow> {
        if let Some(row) = self.row(user_id, provider).await? {
            return Ok(row);
        }
        if !self.has_server_credentials(provider) {
            return Err(HubError::CredentialsRequired(provider));
        }
        self.save_credentials(user_id, provider, None, None, true)
            .await?;
        self.row(user_id, provider)
            .await?
            .ok_or(HubError::Provider(ProviderError::NotConnected))
    }

    pub fn is_sandbox(&self) -> bool {
        self.sandbox
    }

    pub fn provider(&self, id: ProviderId) -> &dyn StreamingProvider {
        match id {
            ProviderId::Tidal => &self.tidal,
            ProviderId::Qobuz => &self.qobuz,
        }
    }

    async fn db<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut SqliteConnection) -> diesel::QueryResult<T> + Send + 'static,
    ) -> HubResult<T> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let mut connection = pool
                .get()
                .map_err(|error| HubError::Database(error.to_string()))?;
            operation(&mut connection).map_err(|error| HubError::Database(error.to_string()))
        })
        .await
        .map_err(|error| HubError::Database(error.to_string()))?
    }

    async fn row(&self, user_id: i32, provider: ProviderId) -> HubResult<Option<AccountRow>> {
        self.ensure_key_service().await;
        self.db(move |connection| store::account(connection, user_id, provider))
            .await
    }

    fn credentials(
        &self,
        user_id: i32,
        provider: ProviderId,
        row: &AccountRow,
    ) -> HubResult<ClientCredentials> {
        let stored: ClientCredentials = self
            .vault
            .open(
                user_id,
                provider,
                SealPurpose::Credentials,
                &row.sealed_credentials,
            )
            .map_err(|_| HubError::Provider(ProviderError::NotConnected))?;
        if stored.client_id == SERVER_CREDENTIALS_MARKER {
            return self
                .server_credential(provider)
                .map(|credential| credential.credentials)
                .ok_or(HubError::Provider(ProviderError::NotConnected));
        }
        Ok(stored)
    }

    fn summarize(
        &self,
        user_id: i32,
        provider: ProviderId,
        row: Option<&AccountRow>,
    ) -> CredentialSummary {
        let stored = row.and_then(|row| {
            self.vault
                .open::<ClientCredentials>(
                    user_id,
                    provider,
                    SealPurpose::Credentials,
                    &row.sealed_credentials,
                )
                .ok()
        });
        match stored {
            Some(credentials) if credentials.client_id == SERVER_CREDENTIALS_MARKER => {
                let server = self.server_credential(provider);
                CredentialSummary {
                    configured: server.is_some(),
                    client_id_hint: None,
                    has_secret: server
                        .is_some_and(|server| server.credentials.client_secret.is_some()),
                    uses_server_credentials: true,
                }
            }
            Some(credentials) => CredentialSummary {
                configured: true,
                client_id_hint: Some(hint(&credentials.client_id)),
                has_secret: credentials.client_secret.is_some(),
                uses_server_credentials: false,
            },
            None => CredentialSummary {
                configured: false,
                client_id_hint: None,
                has_secret: false,
                uses_server_credentials: false,
            },
        }
    }

    pub async fn status(&self, user_id: i32) -> HubResult<Vec<ProviderStatus>> {
        self.ensure_key_service().await;
        let rows = self
            .db(move |connection| {
                let rows = store::accounts(connection, user_id)?;
                let mut counts = HashMap::new();
                for provider in ProviderId::ALL {
                    counts.insert(
                        provider,
                        store::library_counts(connection, user_id, provider)?,
                    );
                }
                Ok((rows, counts))
            })
            .await?;
        let (rows, mut counts) = rows;
        Ok(ProviderId::ALL
            .into_iter()
            .map(|provider| {
                let row = rows.iter().find(|row| row.provider_id() == Some(provider));
                self.describe(
                    user_id,
                    provider,
                    row,
                    counts.remove(&provider).unwrap_or_default(),
                )
            })
            .collect())
    }

    pub async fn provider_status(
        &self,
        user_id: i32,
        provider: ProviderId,
    ) -> HubResult<ProviderStatus> {
        self.ensure_key_service().await;
        let (row, counts) = self
            .db(move |connection| {
                Ok((
                    store::account(connection, user_id, provider)?,
                    store::library_counts(connection, user_id, provider)?,
                ))
            })
            .await?;
        Ok(self.describe(user_id, provider, row.as_ref(), counts))
    }

    fn describe(
        &self,
        user_id: i32,
        provider: ProviderId,
        row: Option<&AccountRow>,
        library: LibraryCounts,
    ) -> ProviderStatus {
        let client = self.provider(provider);
        let pending = row
            .and_then(|row| row.sealed_pending.as_deref())
            .and_then(|sealed| {
                self.vault
                    .open::<DeviceAuthorization>(
                        user_id,
                        provider,
                        SealPurpose::PendingAuthorization,
                        sealed,
                    )
                    .ok()
            })
            .filter(|pending| pending.expires_at > unix_now())
            .map(|pending| PendingSignIn::from(&pending));
        let mut status = row.map_or(AccountStatus::Configured, AccountRow::status);
        if status == AccountStatus::Pending && pending.is_none() {
            status = AccountStatus::Configured;
        }
        ProviderStatus {
            id: provider,
            name: provider.display_name(),
            sign_in: match provider {
                ProviderId::Tidal => "device_code",
                ProviderId::Qobuz => "password",
            },
            status,
            status_message: row.and_then(|row| row.status_message.clone()),
            account_name: row.and_then(|row| row.account_name.clone()),
            plan: row.and_then(|row| row.plan.clone()),
            quality: row.map_or(client.default_quality(), AccountRow::quality),
            quality_options: client.quality_options(),
            browse_sections: client.editorial_sections(),
            last_library_sync: row.and_then(|row| row.last_library_sync),
            credentials: self.summarize(user_id, provider, row),
            server_credentials_available: self.has_server_credentials(provider),
            server_credentials_source: self
                .server_credential(provider)
                .map(|credential| credential.source),
            key_service: self.key_service_status(provider),
            pending,
            library,
            sandbox: self.sandbox,
            last_check: row
                .and_then(|row| row.last_check.as_deref())
                .and_then(|json| serde_json::from_str(json).ok()),
        }
    }

    /// Saves listener-supplied credentials, or (with `use_server`) the
    /// server owner's configured ones.
    pub async fn save_credentials(
        &self,
        user_id: i32,
        provider: ProviderId,
        client_id: Option<String>,
        client_secret: Option<String>,
        use_server: bool,
    ) -> HubResult<ProviderStatus> {
        let credentials = if use_server {
            if !self.has_server_credentials(provider) {
                return Err(HubError::Invalid(
                    "This server has no credentials configured for that service.",
                ));
            }
            ClientCredentials {
                client_id: SERVER_CREDENTIALS_MARKER.to_string(),
                client_secret: None,
                ..Default::default()
            }
        } else {
            validated_credentials(provider, client_id, client_secret)?
        };
        let sealed = self
            .vault
            .seal(user_id, provider, SealPurpose::Credentials, &credentials)
            .map_err(|_| HubError::Database("could not seal credentials".into()))?;
        let quality = self.provider(provider).default_quality();
        self.db(move |connection| {
            store::save_credentials(connection, user_id, provider, &sealed, quality)
        })
        .await?;
        self.provider_status(user_id, provider).await
    }

    pub async fn start_tidal_sign_in(&self, user_id: i32) -> HubResult<PendingSignIn> {
        let row = self.account_for_sign_in(user_id, ProviderId::Tidal).await?;
        let credentials = self.credentials(user_id, ProviderId::Tidal, &row)?;
        let pending = self.tidal.start_device_authorization(&credentials).await?;
        let sealed = self
            .vault
            .seal(
                user_id,
                ProviderId::Tidal,
                SealPurpose::PendingAuthorization,
                &pending,
            )
            .map_err(|_| HubError::Database("could not seal sign-in".into()))?;
        self.db(move |connection| {
            store::save_pending(connection, user_id, ProviderId::Tidal, &sealed)
        })
        .await?;
        Ok(PendingSignIn::from(&pending))
    }

    /// Starts a TIDAL browser sign-in (authorization code with PKCE), the
    /// flow for clients registered on TIDAL's developer dashboard.
    pub async fn start_tidal_browser_sign_in(
        &self,
        user_id: i32,
        redirect_uri: &str,
        origins: &[String],
    ) -> HubResult<BrowserSignIn> {
        let redirect_uri = validated_redirect_uri(redirect_uri, origins)?;
        let row = self.account_for_sign_in(user_id, ProviderId::Tidal).await?;
        let credentials = self.credentials(user_id, ProviderId::Tidal, &row)?;
        // A key service's TIDAL client has the service's relay registered as
        // its redirect URI; the relay forwards back to `redirect_uri`.
        let relay = self
            .uses_key_service(user_id, ProviderId::Tidal, &row)
            .then(|| self.key_service_manifest())
            .flatten()
            .and_then(|manifest| manifest.tidal)
            .map(|tidal| tidal.redirect_uri);
        let (pending, mut authorize_url) = match &relay {
            Some(relay) => {
                TidalClient::relayed_browser_authorization(&credentials, relay, &redirect_uri)
            }
            None => TidalClient::browser_authorization(&credentials, &redirect_uri),
        };
        if self.sandbox {
            // There is no TIDAL login page to visit in the sandbox, so the
            // listener comes straight back as if they had approved.
            authorize_url = format!(
                "{redirect_uri}?code=sandbox-authorization-code&state={}",
                super::transport::form_encode(&pending.state)
            );
        }
        let sign_in = BrowserSignIn {
            authorize_url,
            redirect_uri: pending.redirect_uri.clone(),
            expires_at: pending.expires_at,
            sandbox: self.sandbox,
        };
        let mut pending_sign_ins = self
            .browser_sign_ins
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = unix_now();
        pending_sign_ins.retain(|_, (owner, other)| other.expires_at > now && *owner != user_id);
        if pending_sign_ins.len() >= MAX_BROWSER_SIGN_INS
            && let Some(oldest) = pending_sign_ins
                .iter()
                .min_by_key(|(_, (_, other))| other.expires_at)
                .map(|(state, _)| state.clone())
        {
            pending_sign_ins.remove(&oldest);
        }
        pending_sign_ins.insert(pending.state.clone(), (user_id, pending));
        Ok(sign_in)
    }

    /// Finishes a browser sign-in with the code and state TIDAL sent back.
    /// Each state works once, for the listener who started it.
    /// Signs in to TIDAL headlessly with a refresh token from an earlier
    /// sign-in on this client, the way an unattended check (CI with
    /// secrets) keeps a session without a person approving a code.
    pub async fn tidal_sign_in_with_refresh_token(
        &self,
        user_id: i32,
        refresh_token: &str,
    ) -> HubResult<ProviderStatus> {
        let refresh_token = refresh_token.trim();
        if refresh_token.is_empty() || refresh_token.len() > 4096 {
            return Err(HubError::Invalid("That TIDAL refresh token is not usable."));
        }
        let row = self.account_for_sign_in(user_id, ProviderId::Tidal).await?;
        let credentials = self.credentials(user_id, ProviderId::Tidal, &row)?;
        let seed = ProviderSession {
            access_token: String::new(),
            refresh_token: Some(refresh_token.to_string()),
            expires_at: Some(0),
            user_id: String::new(),
            account_name: String::new(),
            country_code: None,
            plan: None,
        };
        let session = self
            .tidal
            .refresh(&credentials, &seed)
            .await?
            .ok_or(HubError::Provider(ProviderError::SessionExpired))?;
        let session = self.tidal.identify(session).await?;
        self.store_session(user_id, ProviderId::Tidal, &session)
            .await?;
        self.provider_status(user_id, ProviderId::Tidal).await
    }

    pub async fn finish_tidal_browser_sign_in(
        &self,
        user_id: i32,
        state: &str,
        code: &str,
    ) -> HubResult<ProviderStatus> {
        let code = code.trim();
        if code.is_empty()
            || code.len() > 2048
            || !code.chars().all(|character| character.is_ascii_graphic())
        {
            return Err(HubError::Invalid("TIDAL sent back no usable sign-in code."));
        }
        let pending = {
            let mut pending_sign_ins = self
                .browser_sign_ins
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match pending_sign_ins.get(state) {
                Some((owner, _)) if *owner == user_id => {
                    pending_sign_ins.remove(state).map(|(_, pending)| pending)
                }
                _ => None,
            }
        }
        .filter(|pending| pending.expires_at > unix_now())
        .ok_or(HubError::Invalid(
            "This TIDAL sign-in expired or was already used. Start it again.",
        ))?;
        let row = self
            .row(user_id, ProviderId::Tidal)
            .await?
            .ok_or(HubError::Provider(ProviderError::NotConnected))?;
        let credentials = self.credentials(user_id, ProviderId::Tidal, &row)?;
        let session = self
            .tidal
            .exchange_authorization_code(&credentials, &pending, code)
            .await?;
        self.store_session(user_id, ProviderId::Tidal, &session)
            .await?;
        self.provider_status(user_id, ProviderId::Tidal).await
    }

    pub async fn poll_tidal_sign_in(&self, user_id: i32) -> HubResult<PollOutcome> {
        let row = self
            .row(user_id, ProviderId::Tidal)
            .await?
            .ok_or(HubError::Provider(ProviderError::NotConnected))?;
        let Some(pending) = row.sealed_pending.as_deref().and_then(|sealed| {
            self.vault
                .open::<DeviceAuthorization>(
                    user_id,
                    ProviderId::Tidal,
                    SealPurpose::PendingAuthorization,
                    sealed,
                )
                .ok()
        }) else {
            let status = if row.status() == AccountStatus::SignedIn {
                "signed_in"
            } else {
                "expired"
            };
            return Ok(PollOutcome {
                status,
                interval: 5,
                provider: Some(self.provider_status(user_id, ProviderId::Tidal).await?),
            });
        };
        let credentials = self.credentials(user_id, ProviderId::Tidal, &row)?;
        let (status, interval) = match self
            .tidal
            .poll_device_authorization(&credentials, &pending)
            .await?
        {
            DevicePoll::Pending => ("pending", pending.interval),
            DevicePoll::SlowDown => ("pending", pending.interval + 5),
            DevicePoll::Approved(session) => {
                self.store_session(user_id, ProviderId::Tidal, &session)
                    .await?;
                ("signed_in", pending.interval)
            }
            outcome @ (DevicePoll::Expired | DevicePoll::Denied) => {
                let (status, message) = if outcome == DevicePoll::Expired {
                    ("expired", "The sign-in code expired. Start again.")
                } else {
                    ("denied", "Sign-in was declined on TIDAL.")
                };
                self.db(move |connection| {
                    store::set_status(
                        connection,
                        user_id,
                        ProviderId::Tidal,
                        AccountStatus::Configured,
                        Some(message),
                    )
                })
                .await?;
                (status, pending.interval)
            }
        };
        Ok(PollOutcome {
            status,
            interval,
            provider: Some(self.provider_status(user_id, ProviderId::Tidal).await?),
        })
    }

    pub async fn qobuz_sign_in(
        &self,
        user_id: i32,
        username: &str,
        password: &str,
    ) -> HubResult<ProviderStatus> {
        let username = username.trim();
        if username.is_empty()
            || username.len() > 254
            || password.is_empty()
            || password.len() > 1024
        {
            return Err(HubError::Invalid(
                "Enter your Qobuz email or username and password.",
            ));
        }
        let row = self.account_for_sign_in(user_id, ProviderId::Qobuz).await?;
        let credentials = self.credentials(user_id, ProviderId::Qobuz, &row)?;
        let session = self.qobuz.login(&credentials, username, password).await?;
        self.store_session(user_id, ProviderId::Qobuz, &session)
            .await?;
        self.provider_status(user_id, ProviderId::Qobuz).await
    }

    async fn store_session(
        &self,
        user_id: i32,
        provider: ProviderId,
        session: &ProviderSession,
    ) -> HubResult<()> {
        let sealed = self
            .vault
            .seal(user_id, provider, SealPurpose::Session, session)
            .map_err(|_| HubError::Database("could not seal session".into()))?;
        let name = session.account_name.clone();
        let plan = session.plan.clone();
        self.db(move |connection| {
            store::save_session(
                connection,
                user_id,
                provider,
                &sealed,
                &name,
                plan.as_deref(),
            )
        })
        .await?;
        Ok(())
    }

    async fn mark_expired(&self, user_id: i32, provider: ProviderId) {
        let _ = self
            .db(move |connection| {
                store::set_status(
                    connection,
                    user_id,
                    provider,
                    AccountStatus::Expired,
                    Some("Your session expired. Sign in again."),
                )
            })
            .await;
    }

    pub(crate) async fn context(
        &self,
        user_id: i32,
        provider: ProviderId,
    ) -> HubResult<(ProviderContext, StreamQuality)> {
        let row = self
            .row(user_id, provider)
            .await?
            .ok_or(HubError::Provider(ProviderError::NotConnected))?;
        let credentials = self.credentials(user_id, provider, &row)?;
        let quality = row.quality();
        let Some(sealed) = row.sealed_session.as_deref() else {
            return Err(HubError::Provider(match row.status() {
                AccountStatus::Expired => ProviderError::SessionExpired,
                _ => ProviderError::NotConnected,
            }));
        };
        let session: ProviderSession =
            match self
                .vault
                .open(user_id, provider, SealPurpose::Session, sealed)
            {
                Ok(session) => session,
                Err(_) => {
                    // The key changed (for example after a restore without
                    // Secrets/); the listener signs in again.
                    self.mark_expired(user_id, provider).await;
                    return Err(HubError::Provider(ProviderError::SessionExpired));
                }
            };
        let mut context = ProviderContext {
            credentials,
            session,
        };
        if context
            .session
            .expires_at
            .is_some_and(|expires| expires - REFRESH_MARGIN_SECONDS <= unix_now())
        {
            self.refresh(user_id, provider, &mut context).await?;
        }
        Ok((context, quality))
    }

    async fn refresh(
        &self,
        user_id: i32,
        provider: ProviderId,
        context: &mut ProviderContext,
    ) -> HubResult<()> {
        match self
            .provider(provider)
            .refresh(&context.credentials, &context.session)
            .await
        {
            Ok(Some(session)) => {
                self.store_session(user_id, provider, &session).await?;
                context.session = session;
                Ok(())
            }
            Ok(None) | Err(ProviderError::SessionExpired) => {
                self.mark_expired(user_id, provider).await;
                Err(HubError::Provider(ProviderError::SessionExpired))
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Runs a provider call, refreshing the session once if the service
    /// reports it expired.
    async fn call<T, F>(
        &self,
        user_id: i32,
        provider: ProviderId,
        operation: F,
    ) -> HubResult<(T, StreamQuality)>
    where
        F: for<'a> Fn(
            &'a dyn StreamingProvider,
            &'a ProviderContext,
        ) -> futures::future::BoxFuture<'a, Result<T, ProviderError>>,
    {
        let (mut context, quality) = self.context(user_id, provider).await?;
        match operation(self.provider(provider), &context).await {
            Err(ProviderError::SessionExpired) => {
                self.refresh(user_id, provider, &mut context).await?;
                match operation(self.provider(provider), &context).await {
                    Err(ProviderError::SessionExpired) => {
                        self.mark_expired(user_id, provider).await;
                        Err(HubError::Provider(ProviderError::SessionExpired))
                    }
                    result => Ok((result?, quality)),
                }
            }
            result => Ok((result?, quality)),
        }
    }

    async fn library_ids(
        &self,
        user_id: i32,
        provider: ProviderId,
    ) -> HubResult<(HashSet<String>, HashSet<String>)> {
        self.db(move |connection| {
            Ok((
                store::item_ids(connection, user_id, Some(provider), ItemKind::Album)?,
                store::item_ids(connection, user_id, Some(provider), ItemKind::Track)?,
            ))
        })
        .await
    }

    pub async fn connected(&self, user_id: i32) -> HubResult<Vec<ProviderId>> {
        let rows = self
            .db(move |connection| store::accounts(connection, user_id))
            .await?;
        Ok(rows
            .iter()
            .filter(|row| row.status() == AccountStatus::SignedIn)
            .filter_map(AccountRow::provider_id)
            .collect())
    }

    pub async fn search(
        &self,
        user_id: i32,
        query: &str,
        providers: &[ProviderId],
        limit: u32,
    ) -> HubResult<Vec<ProviderSearch>> {
        let searches = providers.iter().map(|provider| {
            let provider = *provider;
            let query = query.to_string();
            async move {
                let result = self
                    .call(user_id, provider, move |client, context| {
                        let query = query.clone();
                        Box::pin(async move { client.search(context, &query, limit).await })
                    })
                    .await;
                (provider, result)
            }
        });
        let mut output = Vec::new();
        for (provider, result) in futures::future::join_all(searches).await {
            match result {
                Ok((mut results, _)) => {
                    if self.sandbox {
                        results = sandbox::filter_search(results, query);
                    }
                    let (albums, tracks) = self.library_ids(user_id, provider).await?;
                    mark_albums(&mut results.albums, &albums, &tracks);
                    mark_tracks(&mut results.tracks, &tracks);
                    output.push(ProviderSearch {
                        provider,
                        name: provider.display_name(),
                        results: Some(results),
                        error: None,
                    });
                }
                Err(error) => output.push(ProviderSearch {
                    provider,
                    name: provider.display_name(),
                    results: None,
                    error: Some(user_message(&error)),
                }),
            }
        }
        Ok(output)
    }

    pub async fn album(
        &self,
        user_id: i32,
        provider: ProviderId,
        raw_id: String,
    ) -> HubResult<ProviderAlbum> {
        let (mut album, _) = self
            .call(user_id, provider, move |client, context| {
                let raw_id = raw_id.clone();
                Box::pin(async move { client.album(context, &raw_id).await })
            })
            .await?;
        let (albums, tracks) = self.library_ids(user_id, provider).await?;
        mark_albums(std::slice::from_mut(&mut album), &albums, &tracks);
        Ok(album)
    }

    pub async fn artist(
        &self,
        user_id: i32,
        provider: ProviderId,
        raw_id: String,
    ) -> HubResult<ProviderArtist> {
        let (mut artist, _) = self
            .call(user_id, provider, move |client, context| {
                let raw_id = raw_id.clone();
                Box::pin(async move { client.artist(context, &raw_id).await })
            })
            .await?;
        let (albums, tracks) = self.library_ids(user_id, provider).await?;
        mark_albums(&mut artist.albums, &albums, &tracks);
        mark_tracks(&mut artist.top_tracks, &tracks);
        Ok(artist)
    }

    pub async fn browse(
        &self,
        user_id: i32,
        provider: ProviderId,
        section: BrowseSection,
        offset: u32,
        limit: u32,
    ) -> HubResult<BrowsePage> {
        let (mut page, _) = self
            .call(user_id, provider, move |client, context| {
                Box::pin(async move { client.browse(context, section, offset, limit).await })
            })
            .await?;
        let (albums, tracks) = self.library_ids(user_id, provider).await?;
        mark_albums(&mut page.albums, &albums, &tracks);
        mark_tracks(&mut page.tracks, &tracks);
        Ok(page)
    }

    /// The service's genres, read at most every few hours per listener.
    pub async fn genres(
        &self,
        user_id: i32,
        provider: ProviderId,
    ) -> HubResult<Vec<ProviderGenre>> {
        let key = (user_id, provider);
        if let Some((at, genres)) = self
            .genre_lists
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&key)
            && unix_now() - at < GENRE_LIST_SECONDS
        {
            return Ok(genres.clone());
        }
        let (genres, _) = self
            .call(user_id, provider, |client, context| {
                Box::pin(async move { client.genres(context).await })
            })
            .await?;
        self.genre_lists
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key, (unix_now(), genres.clone()));
        Ok(genres)
    }

    /// One genre's albums. Albums the service files without a genre (all of
    /// TIDAL's) are labelled with this one, and Parson remembers it for
    /// when they are added to the library.
    pub async fn genre_albums(
        &self,
        user_id: i32,
        provider: ProviderId,
        genre_id: String,
        section: BrowseSection,
        offset: u32,
        limit: u32,
    ) -> HubResult<GenreAlbums> {
        let genre = self
            .genres(user_id, provider)
            .await?
            .into_iter()
            .find(|genre| genre.id == genre_id)
            .ok_or(HubError::Provider(ProviderError::NotFound))?;
        let (mut page, _) = self
            .call(user_id, provider, move |client, context| {
                let genre_id = genre_id.clone();
                Box::pin(async move {
                    client
                        .genre_albums(context, &genre_id, section, offset, limit)
                        .await
                })
            })
            .await?;
        for album in &mut page.albums {
            album.genre.get_or_insert_with(|| genre.name.clone());
        }
        self.remember_genres(&page.albums);
        let (albums, tracks) = self.library_ids(user_id, provider).await?;
        mark_albums(&mut page.albums, &albums, &tracks);
        Ok(GenreAlbums { genre, page })
    }

    fn remember_genres(&self, albums: &[ProviderAlbum]) {
        let mut hints = self
            .genre_hints
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if hints.len() + albums.len() > MAX_GENRE_HINTS {
            hints.clear();
        }
        for album in albums {
            if let Some(genre) = &album.genre {
                hints
                    .entry(album.id.clone())
                    .or_insert_with(|| genre.clone());
            }
        }
    }

    /// Files an album without a genre under the genre it was found in.
    fn apply_genre_hint(&self, album: &mut ProviderAlbum) {
        if album.genre.is_none() {
            album.genre = self
                .genre_hints
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&album.id)
                .cloned();
        }
    }

    /// Every streaming album in the listener's library, for the unified
    /// album catalog.
    pub async fn library_albums(&self, user_id: i32) -> HubResult<Vec<LibraryItemRow>> {
        self.db(move |connection| store::all_albums(connection, user_id))
            .await
    }

    /// "Add to library": snapshots the album or track into the listener's
    /// library and favorites it on the service, like the service's own apps.
    pub async fn add_to_library(
        &self,
        user_id: i32,
        provider: ProviderId,
        kind: ItemKind,
        raw_id: String,
    ) -> HubResult<LibraryChange> {
        enum Fetched {
            Album(Box<ProviderAlbum>),
            Track(Box<ProviderTrack>),
        }
        let lookup_id = raw_id.clone();
        let (fetched, _) = self
            .call(user_id, provider, move |client, context| {
                let raw_id = lookup_id.clone();
                Box::pin(async move {
                    Ok(match kind {
                        ItemKind::Album => {
                            Fetched::Album(Box::new(client.album(context, &raw_id).await?))
                        }
                        ItemKind::Track => {
                            Fetched::Track(Box::new(client.track(context, &raw_id).await?))
                        }
                    })
                })
            })
            .await?;
        let item = match fetched {
            Fetched::Album(mut album) => {
                self.apply_genre_hint(&mut album);
                NewLibraryItem::album(&album)
            }
            Fetched::Track(track) => NewLibraryItem::track(&track),
        };
        let favorite_id = raw_id.clone();
        let service_updated = match self
            .call(user_id, provider, move |client, context| {
                let raw_id = favorite_id.clone();
                Box::pin(async move { client.set_favorite(context, kind, &raw_id, true).await })
            })
            .await
        {
            Ok(_) => true,
            Err(error) => {
                tracing::warn!(%error, provider = provider.as_str(), "could not add a service favorite");
                false
            }
        };
        self.db(move |connection| {
            store::upsert_item(connection, user_id, &item, ItemOrigin::Parson, unix_now())
        })
        .await?;
        Ok(LibraryChange {
            in_library: true,
            service_updated,
        })
    }

    pub async fn remove_from_library(
        &self,
        user_id: i32,
        provider: ProviderId,
        kind: ItemKind,
        raw_id: String,
    ) -> HubResult<LibraryChange> {
        let favorite_id = raw_id.clone();
        let service_updated = self
            .call(user_id, provider, move |client, context| {
                let raw_id = favorite_id.clone();
                Box::pin(async move { client.set_favorite(context, kind, &raw_id, false).await })
            })
            .await
            .is_ok();
        self.db(move |connection| store::remove_item(connection, user_id, provider, kind, &raw_id))
            .await?;
        Ok(LibraryChange {
            in_library: false,
            service_updated,
        })
    }

    /// Imports the service's favorite albums and tracks into the library and
    /// drops items that are no longer favorites.
    pub async fn sync_library(&self, user_id: i32, provider: ProviderId) -> HubResult<SyncReport> {
        let mut report = SyncReport::default();
        for (section, kind) in [
            (BrowseSection::FavoriteAlbums, ItemKind::Album),
            (BrowseSection::FavoriteTracks, ItemKind::Track),
        ] {
            let mut items = Vec::new();
            let mut offset = 0;
            let mut complete = false;
            while offset < MAX_SYNC_ITEMS {
                let (mut page, _) = self
                    .call(user_id, provider, move |client, context| {
                        Box::pin(
                            async move { client.browse(context, section, offset, SYNC_PAGE).await },
                        )
                    })
                    .await?;
                for album in &mut page.albums {
                    self.apply_genre_hint(album);
                }
                let received = page.albums.len() + page.tracks.len();
                items.extend(page.albums.iter().map(NewLibraryItem::album));
                items.extend(page.tracks.iter().map(NewLibraryItem::track));
                offset += SYNC_PAGE;
                if received < SYNC_PAGE as usize || page.total.is_some_and(|total| offset >= total)
                {
                    complete = true;
                    break;
                }
            }
            // Recorded sandbox favorites never change, so pruning there would
            // undo every "Add to library".
            let prune = complete && !self.sandbox;
            let (added, removed) = self
                .db(move |connection| {
                    store::apply_sync(connection, user_id, provider, kind, &items, prune)
                })
                .await?;
            match kind {
                ItemKind::Album => {
                    report.albums_added = added;
                    report.albums_removed = removed;
                }
                ItemKind::Track => {
                    report.tracks_added = added;
                    report.tracks_removed = removed;
                }
            }
        }
        report.synced_at = unix_now();
        let at = report.synced_at;
        self.db(move |connection| store::set_last_sync(connection, user_id, provider, at))
            .await?;
        Ok(report)
    }

    pub async fn library(
        &self,
        user_id: i32,
        request: LibraryRequest,
    ) -> HubResult<(Vec<LibraryItemRow>, i64)> {
        self.db(move |connection| {
            store::list_items(
                connection,
                user_id,
                &LibraryQuery {
                    provider: request.provider,
                    kind: request.kind,
                    text: request.text.as_deref(),
                    sort: request.sort,
                    offset: request.offset,
                    limit: request.limit,
                },
            )
        })
        .await
    }

    pub async fn set_quality(
        &self,
        user_id: i32,
        provider: ProviderId,
        quality: StreamQuality,
    ) -> HubResult<ProviderStatus> {
        if !self
            .provider(provider)
            .quality_options()
            .iter()
            .any(|option| option.id == quality)
        {
            return Err(HubError::Invalid(
                "That quality is not offered by this service.",
            ));
        }
        let changed = self
            .db(move |connection| store::set_quality(connection, user_id, provider, quality))
            .await?;
        if changed == 0 {
            return Err(HubError::Provider(ProviderError::NotConnected));
        }
        self.provider_status(user_id, provider).await
    }

    pub async fn source_preference(&self, user_id: i32) -> HubResult<SourcePreference> {
        let stored = self
            .db(move |connection| store::source_preference(connection, user_id))
            .await?;
        Ok(stored
            .as_deref()
            .and_then(SourcePreference::parse)
            .unwrap_or_default())
    }

    pub async fn set_source_preference(
        &self,
        user_id: i32,
        preference: SourcePreference,
    ) -> HubResult<SourcePreference> {
        self.db(move |connection| {
            store::set_source_preference(connection, user_id, preference.as_str())
        })
        .await?;
        Ok(preference)
    }

    pub async fn sign_out(&self, user_id: i32, provider: ProviderId) -> HubResult<ProviderStatus> {
        self.db(move |connection| store::sign_out(connection, user_id, provider))
            .await?;
        self.provider_status(user_id, provider).await
    }

    pub async fn remove(&self, user_id: i32, provider: ProviderId) -> HubResult<ProviderStatus> {
        self.db(move |connection| store::remove_account(connection, user_id, provider))
            .await?;
        self.provider_status(user_id, provider).await
    }

    /// Resolves a playable stream at the listener's chosen quality.
    /// `lossy_only` caps it to the service's lossy tier when the listener
    /// limited Parson's streaming bitrate.
    pub async fn resolve_stream(
        &self,
        user_id: i32,
        provider: ProviderId,
        raw_id: String,
        lossy_only: bool,
    ) -> HubResult<ResolvedStream> {
        let (_, chosen) = self.context(user_id, provider).await?;
        let quality = if lossy_only && chosen.is_lossless() {
            StreamQuality::High
        } else {
            chosen
        };
        if self.sandbox && self.sandbox_tone {
            let seed = sandbox::seed(&raw_id);
            let mut format = AudioFormat::lossless("pcm", Some(16), Some(44_100));
            format.label = "Sandbox preview".to_string();
            return Ok(ResolvedStream {
                source: StreamSource::SandboxTone {
                    seed,
                    seconds: sandbox::PREVIEW_SECONDS,
                },
                format,
                requested: quality,
                stream_seconds: Some(f64::from(sandbox::PREVIEW_SECONDS)),
            });
        }
        let (stream, _) = self
            .call(user_id, provider, move |client, context| {
                let raw_id = raw_id.clone();
                Box::pin(async move { client.resolve_stream(context, &raw_id, quality).await })
            })
            .await?;
        Ok(stream)
    }

    async fn saved_copies(
        &self,
        user_id: i32,
        provider: ProviderId,
    ) -> HubResult<HashMap<String, i64>> {
        let rows = self
            .db(move |connection| store::playlist_copies(connection, user_id))
            .await?;
        Ok(rows
            .into_iter()
            .filter(|row| row.provider == provider.as_str())
            .map(|row| (row.provider_playlist_id, row.saved_at))
            .collect())
    }

    /// The listener's playlists on a service, read-only.
    pub async fn playlists(
        &self,
        user_id: i32,
        provider: ProviderId,
        offset: u32,
        limit: u32,
    ) -> HubResult<PlaylistViews> {
        let (page, _) = self
            .call(user_id, provider, move |client, context| {
                Box::pin(async move { client.playlists(context, offset, limit).await })
            })
            .await?;
        let saved = self.saved_copies(user_id, provider).await?;
        Ok(PlaylistViews {
            total: page.total,
            playlists: page
                .playlists
                .into_iter()
                .map(|playlist| {
                    let saved_copy_at = super::parse_entity_id(&playlist.id)
                        .and_then(|(_, raw)| saved.get(&raw).copied());
                    PlaylistView {
                        playlist,
                        saved_copy_at,
                    }
                })
                .collect(),
        })
    }

    pub async fn playlist(
        &self,
        user_id: i32,
        provider: ProviderId,
        raw_id: String,
    ) -> HubResult<PlaylistView> {
        let lookup = raw_id.clone();
        let (mut playlist, _) = self
            .call(user_id, provider, move |client, context| {
                let raw_id = lookup.clone();
                Box::pin(async move { client.playlist(context, &raw_id).await })
            })
            .await?;
        let (_, tracks) = self.library_ids(user_id, provider).await?;
        mark_tracks(&mut playlist.tracks, &tracks);
        let saved_copy_at = self
            .saved_copies(user_id, provider)
            .await?
            .get(&raw_id)
            .copied();
        Ok(PlaylistView {
            playlist,
            saved_copy_at,
        })
    }

    /// "Save a copy": snapshots the playlist's current tracks into Parson.
    /// The service playlist is never changed. Saving again over a copy the
    /// listener edited needs `replace_edits`.
    pub async fn save_playlist_copy(
        &self,
        user_id: i32,
        provider: ProviderId,
        raw_id: String,
        replace_edits: bool,
    ) -> HubResult<PlaylistCopy> {
        if raw_id.starts_with(PARSON_PLAYLIST_PREFIX) {
            return Err(HubError::Invalid(
                "This playlist was made in Mujin Music and has no service playlist to save again.",
            ));
        }
        if !replace_edits {
            let lookup = raw_id.clone();
            let existing = self
                .db(move |connection| store::playlist_copy(connection, user_id, provider, &lookup))
                .await?;
            if existing.is_some_and(|row| row.edited_at.is_some()) {
                return Err(HubError::Conflict(
                    "You edited this copy in Mujin Music. Saving it again replaces your edits with the service playlist.",
                ));
            }
        }
        let view = self.playlist(user_id, provider, raw_id.clone()).await?;
        let mut playlist = view.playlist;
        for track in &mut playlist.tracks {
            track.in_library = false;
        }
        let saved_at = unix_now();
        self.db(move |connection| {
            store::save_playlist_copy(connection, user_id, &playlist, saved_at)?;
            store::playlist_copy(connection, user_id, provider, &raw_id)
        })
        .await?
        .and_then(|row| PlaylistCopy::from_row(row, false))
        .ok_or(HubError::Database("the copy was not saved".into()))
    }

    /// Edits a copy in Parson: its title, description and tracks (in the
    /// order given). Tracks already in the copy keep their saved details;
    /// new ones are read from the service.
    pub async fn edit_playlist_copy(
        &self,
        user_id: i32,
        provider: ProviderId,
        raw_id: String,
        edit: CopyEdit,
    ) -> HubResult<PlaylistCopy> {
        let lookup = raw_id.clone();
        let row = self
            .db(move |connection| store::playlist_copy(connection, user_id, provider, &lookup))
            .await?
            .ok_or(HubError::Provider(ProviderError::NotFound))?;
        let saved_at = row.saved_at;
        let mut playlist = serde_json::from_str::<ProviderPlaylist>(&row.payload)
            .map_err(|error| HubError::Database(error.to_string()))?;
        if let Some(title) = edit.title {
            playlist.title = copy_title(&title)?;
        }
        if let Some(description) = edit.description {
            let description = description.trim();
            if description.chars().count() > 2_000 {
                return Err(HubError::Invalid(
                    "Keep the description under 2,000 characters.",
                ));
            }
            playlist.description = (!description.is_empty()).then(|| description.to_string());
        }
        if let Some(track_ids) = edit.track_ids {
            let existing = std::mem::take(&mut playlist.tracks);
            playlist.tracks = self
                .tracks_for_copy(user_id, provider, &track_ids, existing)
                .await?;
        }
        self.store_copy(user_id, provider, raw_id, playlist, saved_at)
            .await
    }

    /// A new playlist made in Parson from service tracks, such as a radio
    /// the listener wants to keep.
    pub async fn create_playlist_copy(
        &self,
        user_id: i32,
        provider: ProviderId,
        title: String,
        description: Option<String>,
        track_ids: Vec<String>,
    ) -> HubResult<PlaylistCopy> {
        let title = copy_title(&title)?;
        if track_ids.is_empty() {
            return Err(HubError::Invalid("Add at least one track."));
        }
        let tracks = self
            .tracks_for_copy(user_id, provider, &track_ids, Vec::new())
            .await?;
        let now = unix_now();
        let raw_id = format!(
            "{PARSON_PLAYLIST_PREFIX}{}",
            &uuid::Uuid::new_v4().simple().to_string()[..12]
        );
        let playlist = ProviderPlaylist {
            id: super::entity_id(provider, &raw_id),
            provider,
            cover_url: tracks
                .iter()
                .find_map(|track| track.album.as_ref()?.cover_url.clone()),
            title,
            description: description
                .map(|text| text.trim().to_string())
                .filter(|text| !text.is_empty()),
            owner: Some("You, in Mujin Music".to_string()),
            own: true,
            track_count: tracks.len() as u32,
            duration: tracks.iter().map(|track| track.duration).sum(),
            updated_at: Some(now),
            web_url: None,
            tracks,
        };
        self.store_copy(user_id, provider, raw_id, playlist, now)
            .await
    }

    async fn tracks_for_copy(
        &self,
        user_id: i32,
        provider: ProviderId,
        track_ids: &[String],
        existing: Vec<ProviderTrack>,
    ) -> HubResult<Vec<ProviderTrack>> {
        if track_ids.len() > MAX_COPY_TRACKS {
            return Err(HubError::Invalid("A playlist holds up to 1,000 tracks."));
        }
        let mut known = existing
            .into_iter()
            .map(|track| (track.id.clone(), track))
            .collect::<HashMap<_, _>>();
        let mut seen = HashSet::new();
        let mut wanted = Vec::with_capacity(track_ids.len());
        let mut missing = Vec::new();
        for id in track_ids {
            let Some((track_provider, raw)) = super::parse_entity_id(id) else {
                return Err(HubError::Invalid("Unknown track in the playlist."));
            };
            if track_provider != provider {
                return Err(HubError::Invalid(
                    "A playlist copy holds tracks from one service.",
                ));
            }
            if !seen.insert(id.clone()) {
                continue;
            }
            if !known.contains_key(id) {
                missing.push(raw);
            }
            wanted.push(id.clone());
        }
        if missing.len() > MAX_ADDED_PER_EDIT {
            return Err(HubError::Invalid("Add up to 200 new tracks at a time."));
        }
        if !missing.is_empty() {
            let (fetched, _) = self
                .call(user_id, provider, move |client, context| {
                    let missing = missing.clone();
                    Box::pin(async move {
                        let mut tracks = Vec::with_capacity(missing.len());
                        for chunk in missing.chunks(8) {
                            let results = futures::future::join_all(
                                chunk.iter().map(|raw| client.track(context, raw)),
                            )
                            .await;
                            for result in results {
                                tracks.push(result?);
                            }
                        }
                        Ok(tracks)
                    })
                })
                .await?;
            for mut track in fetched {
                track.in_library = false;
                known.insert(track.id.clone(), track);
            }
        }
        Ok(wanted
            .into_iter()
            .filter_map(|id| known.remove(&id))
            .collect())
    }

    async fn store_copy(
        &self,
        user_id: i32,
        provider: ProviderId,
        raw_id: String,
        mut playlist: ProviderPlaylist,
        saved_at: i64,
    ) -> HubResult<PlaylistCopy> {
        playlist.track_count = playlist.tracks.len() as u32;
        playlist.duration = playlist.tracks.iter().map(|track| track.duration).sum();
        let edited_at = unix_now();
        self.db(move |connection| {
            store::write_playlist_copy(connection, user_id, &playlist, saved_at, Some(edited_at))?;
            store::playlist_copy(connection, user_id, provider, &raw_id)
        })
        .await?
        .and_then(|row| PlaylistCopy::from_row(row, true))
        .ok_or(HubError::Database("the copy was not saved".into()))
    }

    /// A radio from a track, artist or album on one service. Album radios
    /// start from the album's first playable track.
    pub async fn radio(
        &self,
        user_id: i32,
        provider: ProviderId,
        seed: RadioSeed,
        raw_id: String,
    ) -> HubResult<Radio> {
        let (seed, raw_id) = if seed == RadioSeed::Album {
            let album = self.album(user_id, provider, raw_id).await?;
            let first = album
                .tracks
                .iter()
                .find(|track| track.streamable)
                .and_then(|track| super::parse_entity_id(&track.id))
                .map(|(_, raw)| raw)
                .ok_or(HubError::Provider(ProviderError::Unavailable(
                    "This album has no playable tracks to start a radio from.".into(),
                )))?;
            (RadioSeed::Track, first)
        } else {
            (seed, raw_id)
        };
        let (mut radio, _) = self
            .call(user_id, provider, move |client, context| {
                let raw_id = raw_id.clone();
                Box::pin(async move { client.radio(context, seed, &raw_id, RADIO_TRACKS).await })
            })
            .await?;
        let (_, tracks) = self.library_ids(user_id, provider).await?;
        mark_tracks(&mut radio.tracks, &tracks);
        Ok(radio)
    }

    pub async fn playlist_copies(&self, user_id: i32) -> HubResult<Vec<PlaylistCopy>> {
        let rows = self
            .db(move |connection| store::playlist_copies(connection, user_id))
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| PlaylistCopy::from_row(row, false))
            .collect())
    }

    pub async fn playlist_copy(
        &self,
        user_id: i32,
        provider: ProviderId,
        raw_id: String,
    ) -> HubResult<PlaylistCopy> {
        let row = self
            .db(move |connection| store::playlist_copy(connection, user_id, provider, &raw_id))
            .await?
            .ok_or(HubError::Provider(ProviderError::NotFound))?;
        let mut copy =
            PlaylistCopy::from_row(row, true).ok_or(HubError::Provider(ProviderError::NotFound))?;
        if let Some(playlist) = copy.playlist.as_mut() {
            let (_, tracks) = self.library_ids(user_id, provider).await?;
            mark_tracks(&mut playlist.tracks, &tracks);
        }
        Ok(copy)
    }

    pub async fn remove_playlist_copy(
        &self,
        user_id: i32,
        provider: ProviderId,
        raw_id: String,
    ) -> HubResult<()> {
        let removed = self
            .db(move |connection| {
                store::remove_playlist_copy(connection, user_id, provider, &raw_id)
            })
            .await?;
        if removed == 0 {
            return Err(HubError::Provider(ProviderError::NotFound));
        }
        Ok(())
    }

    /// Records what a stream delivered, as read by the media proxy.
    pub fn record_delivery(
        &self,
        user_id: i32,
        track_id: String,
        stream: &ResolvedStream,
        probed: Option<ProbedFormat>,
    ) -> DeliveredStream {
        self.record_play(user_id, track_id, None, stream, probed, None)
    }

    /// Records what one play of a stream delivered. `play` is the
    /// player's id for that play.
    pub fn record_play(
        &self,
        user_id: i32,
        track_id: String,
        play: Option<String>,
        stream: &ResolvedStream,
        probed: Option<ProbedFormat>,
        track_seconds: Option<f64>,
    ) -> DeliveredStream {
        let mut delivery = delivered_stream(track_id.clone(), stream, probed, self.sandbox);
        check_length(&mut delivery, track_seconds);
        delivery.seq = self
            .delivery_seq
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        delivery.play = play.clone();
        if let Ok(mut deliveries) = self.deliveries.lock() {
            if deliveries.len() >= MAX_DELIVERIES
                && let Some(oldest) = deliveries
                    .iter()
                    .min_by_key(|(_, delivery)| delivery.seq)
                    .map(|(key, _)| key.clone())
            {
                deliveries.remove(&oldest);
            }
            deliveries.insert(
                (user_id, track_id, play.unwrap_or_default()),
                delivery.clone(),
            );
        }
        delivery
    }

    /// The most recent delivered format for one of the listener's streams.
    pub fn delivery(&self, user_id: i32, track_id: &str) -> Option<DeliveredStream> {
        self.deliveries
            .lock()
            .ok()?
            .iter()
            .filter(|((user, track, _), _)| *user == user_id && track == track_id)
            .max_by_key(|(_, delivery)| delivery.seq)
            .map(|(_, delivery)| delivery.clone())
    }

    /// What one play of a track delivered, or `None` until that play's
    /// first bytes have been read. Never an earlier play's report.
    pub fn play_delivery(
        &self,
        user_id: i32,
        track_id: &str,
        play: &str,
    ) -> Option<DeliveredStream> {
        self.deliveries
            .lock()
            .ok()?
            .get(&(user_id, track_id.to_string(), play.to_string()))
            .cloned()
    }

    /// Fetches the first bytes of a resolved stream through the media
    /// transport and reads their format, without playing the whole track.
    pub async fn sample_stream(
        &self,
        stream: &ResolvedStream,
    ) -> Result<Option<ProbedFormat>, media::MediaError> {
        match &stream.source {
            StreamSource::SandboxTone { seed, seconds } => {
                let wav = sandbox::preview_wav(*seed, *seconds, stream.requested);
                Ok(super::probe::probe(&wav[..wav.len().min(PROBE_BYTES)]))
            }
            StreamSource::Progressive { url, .. } => {
                let response = self
                    .media
                    .open(url.clone(), Some(format!("bytes=0-{}", PROBE_BYTES - 1)))
                    .await?;
                media::probe_body(response.body).await
            }
            StreamSource::Segmented {
                init_url,
                segment_urls,
                ..
            } => {
                // The init segment carries the format; the first media
                // segment carries the first frame to verify it against.
                let urls = init_url
                    .iter()
                    .chain(segment_urls.iter().take(1))
                    .cloned()
                    .collect();
                media::probe_body(media::segmented(self.media.clone(), urls)).await
            }
        }
    }

    /// "Check connection": proves the account works end to end. It checks
    /// the session, syncs favorites, then resolves a track at the chosen
    /// quality and reads the delivered format from the stream's own bytes
    /// through the same media path playback uses.
    pub async fn check_connection(
        &self,
        user_id: i32,
        provider: ProviderId,
        track: Option<String>,
    ) -> HubResult<ConnectionCheck> {
        let check = self.run_check(user_id, provider, track).await?;
        if let Ok(json) = serde_json::to_string(&AccountCheck::from_check(&check)) {
            self.db(move |connection| store::save_last_check(connection, user_id, provider, &json))
                .await?;
        }
        Ok(check)
    }

    async fn run_check(
        &self,
        user_id: i32,
        provider: ProviderId,
        track: Option<String>,
    ) -> HubResult<ConnectionCheck> {
        let mut check = ConnectionCheck {
            provider,
            ok: false,
            sandbox: self.sandbox,
            account_name: None,
            plan: None,
            steps: Vec::new(),
            sync: None,
            track: None,
            delivery: None,
            checked_at: unix_now(),
        };
        match self.context(user_id, provider).await {
            Ok((context, quality)) => {
                check.account_name = Some(context.session.account_name.clone());
                check.plan = context.session.plan.clone();
                check.steps.push(CheckStep {
                    id: "session",
                    ok: true,
                    detail: format!(
                        "Signed in as {}{}; streaming at {}.",
                        context.session.account_name,
                        context
                            .session
                            .plan
                            .as_deref()
                            .map(|plan| format!(" ({plan})"))
                            .unwrap_or_default(),
                        quality_label(self.provider(provider), quality),
                    ),
                });
            }
            Err(error) => {
                check.steps.push(CheckStep {
                    id: "session",
                    ok: false,
                    detail: user_message(&error),
                });
                return Ok(check);
            }
        }
        match self.sync_library(user_id, provider).await {
            Ok(report) => {
                let counts = self
                    .db(move |connection| store::library_counts(connection, user_id, provider))
                    .await?;
                check.steps.push(CheckStep {
                    id: "favorites",
                    ok: true,
                    detail: format!(
                        "Favorites synced: {} albums and {} tracks in your library.",
                        counts.albums, counts.tracks
                    ),
                });
                check.sync = Some(report);
            }
            Err(error) => {
                check.steps.push(CheckStep {
                    id: "favorites",
                    ok: false,
                    detail: user_message(&error),
                });
                return Ok(check);
            }
        }
        let track = match track {
            Some(raw) => Some(raw),
            None => self.first_library_track(user_id, provider).await?,
        };
        let Some(raw_id) = track else {
            check.steps.push(CheckStep {
                id: "stream",
                ok: false,
                detail: "No favorite tracks or albums to test with. Add one, then check again."
                    .into(),
            });
            return Ok(check);
        };
        let lookup = raw_id.clone();
        if let Ok((track, _)) = self
            .call(user_id, provider, move |client, context| {
                let raw_id = lookup.clone();
                Box::pin(async move { client.track(context, &raw_id).await })
            })
            .await
        {
            check.track = Some(track);
        }
        let stream = match self
            .resolve_stream(user_id, provider, raw_id.clone(), false)
            .await
        {
            Ok(stream) => stream,
            Err(error) => {
                check.steps.push(CheckStep {
                    id: "stream",
                    ok: false,
                    detail: user_message(&error),
                });
                return Ok(check);
            }
        };
        let track_id = super::entity_id(provider, &raw_id);
        let track_seconds = check.track.as_ref().map(|track| track.duration);
        match self.sample_stream(&stream).await {
            Ok(probed) => {
                let delivery =
                    self.record_play(user_id, track_id, None, &stream, probed, track_seconds);
                let ok = delivery.delivered.is_some() && delivery.full_length != Some(false);
                check.steps.push(CheckStep {
                    id: "stream",
                    ok,
                    detail: describe_delivery(&delivery),
                });
                check.delivery = Some(delivery);
                check.ok = ok;
            }
            Err(error) => check.steps.push(CheckStep {
                id: "stream",
                ok: false,
                detail: format!("The stream did not start: {error}."),
            }),
        }
        Ok(check)
    }

    async fn first_library_track(
        &self,
        user_id: i32,
        provider: ProviderId,
    ) -> HubResult<Option<String>> {
        let rows = |kind| {
            self.db(move |connection| {
                store::list_items(
                    connection,
                    user_id,
                    &LibraryQuery {
                        provider: Some(provider),
                        kind,
                        text: None,
                        sort: store::LibrarySort::Recent,
                        offset: 0,
                        limit: 1,
                    },
                )
            })
        };
        if let Some(row) = rows(ItemKind::Track).await?.0.into_iter().next() {
            return Ok(Some(row.provider_item_id));
        }
        let Some(album) = rows(ItemKind::Album).await?.0.into_iter().next() else {
            return Ok(None);
        };
        let album = self
            .album(user_id, provider, album.provider_item_id)
            .await?;
        Ok(album
            .tracks
            .iter()
            .find(|track| track.streamable)
            .and_then(|track| super::parse_entity_id(&track.id))
            .map(|(_, raw)| raw))
    }
}

fn quality_label(provider: &dyn StreamingProvider, quality: StreamQuality) -> String {
    provider
        .quality_options()
        .iter()
        .find(|option| option.id == quality)
        .map_or_else(
            || quality.as_str().to_string(),
            |option| option.label.to_string(),
        )
}

/// Whether a delivered lossy bitrate is the one announced: within 10%,
/// as encoders round and VBR streams average a little either side.
fn bitrate_matches(announced: u32, delivered: u32) -> bool {
    let tolerance = announced.div_ceil(10);
    delivered.abs_diff(announced) <= tolerance
}

/// What disagrees between the announced and delivered formats.
fn format_mismatches(announced: &AudioFormat, delivered: &AudioFormat) -> Vec<String> {
    let mut found = Vec::new();
    if delivered.lossless != announced.lossless {
        found.push(if announced.lossless {
            format!("lossy {}, not lossless", delivered.label)
        } else {
            format!("lossless {}, not lossy", delivered.label)
        });
        return found;
    }
    if delivered.codec != announced.codec {
        found.push(format!(
            "{}, not {}",
            delivered.codec.to_ascii_uppercase(),
            announced.codec.to_ascii_uppercase()
        ));
    }
    if announced.lossless {
        if let Some(depth) = announced.bit_depth
            && delivered.bit_depth != Some(depth)
        {
            found.push(format!(
                "{}-bit, not {depth}-bit",
                delivered
                    .bit_depth
                    .map_or_else(|| "unknown".to_string(), |bits| bits.to_string())
            ));
        }
        if let Some(rate) = announced.sample_rate
            && delivered.sample_rate != Some(rate)
        {
            found.push(format!(
                "{} kHz, not {} kHz",
                delivered
                    .sample_rate
                    .map_or_else(|| "unknown".to_string(), super::format_khz),
                super::format_khz(rate)
            ));
        }
    } else if let (Some(announced), Some(delivered)) =
        (announced.bitrate_kbps, delivered.bitrate_kbps)
        && !bitrate_matches(announced, delivered)
    {
        found.push(format!("{delivered} kbps, not {announced}"));
    }
    found
}

fn delivered_stream(
    track_id: String,
    stream: &ResolvedStream,
    probed: Option<ProbedFormat>,
    sandbox: bool,
) -> DeliveredStream {
    let tone = matches!(stream.source, StreamSource::SandboxTone { .. });
    let announced = stream.format.clone();
    let (delivered, container, frame_verified, header_seconds) = match probed {
        Some(probed) => (
            Some(probed.format),
            Some(probed.container),
            probed.frame_verified,
            probed.seconds,
        ),
        None => (None, None, false, None),
    };
    let mismatches = match &delivered {
        // The placeholder tone announces no real format.
        Some(delivered) if !tone => format_mismatches(&announced, delivered),
        _ => Vec::new(),
    };
    let matches_announced = delivered.is_some() && !tone && mismatches.is_empty();
    DeliveredStream {
        track_id,
        requested: stream.requested,
        announced,
        delivered,
        container,
        frame_verified,
        matches_announced,
        mismatches,
        play: None,
        seq: 0,
        // A DASH timeline lists every segment Parson relays; a fragmented
        // init segment's own header often leaves the length out.
        delivered_seconds: stream.stream_seconds.or(header_seconds),
        track_seconds: None,
        full_length: None,
        sandbox,
        tone,
        at: unix_now(),
    }
}

/// Fills in the catalog length and whether the whole track arrived.
fn check_length(delivery: &mut DeliveredStream, track_seconds: Option<f64>) {
    let track_seconds = track_seconds.filter(|seconds| *seconds > 0.0);
    delivery.track_seconds = track_seconds;
    delivery.full_length = match (delivery.delivered_seconds, track_seconds) {
        // The sandbox renders 30 seconds of every track by design.
        _ if delivery.sandbox => None,
        (Some(delivered), Some(track)) => Some(delivered + 2.0 >= track),
        _ => None,
    };
}

fn clock(seconds: f64) -> String {
    let total = seconds.round().max(0.0) as u64;
    format!("{}:{:02}", total / 60, total % 60)
}

fn describe_delivery(delivery: &DeliveredStream) -> String {
    let Some(delivered) = &delivery.delivered else {
        return "The stream started, but its format could not be read.".into();
    };
    let mut detail = format!(
        "Delivered {}{}",
        delivered.label,
        delivered
            .channels
            .map(|channels| match channels {
                1 => ", mono".to_string(),
                2 => ", stereo".to_string(),
                count => format!(", {count} channels"),
            })
            .unwrap_or_default()
    );
    if delivery.frame_verified {
        detail.push_str("; first audio frame verified");
    }
    match (
        delivery.full_length,
        delivery.delivered_seconds,
        delivery.track_seconds,
    ) {
        (Some(true), Some(delivered), _) => {
            detail.push_str(&format!("; full length ({})", clock(delivered)));
        }
        (Some(false), Some(delivered), Some(track)) => detail.push_str(&format!(
            "; only {} of {} arrived, a preview rather than the full track",
            clock(delivered),
            clock(track)
        )),
        _ => {}
    }
    if delivery.tone {
        detail.push_str(". Sandbox preview tone, not a real stream.");
    } else if delivery.matches_announced {
        detail.push_str(&format!(
            ", as the service announced ({}).",
            delivery.announced.label
        ));
        if delivery.sandbox {
            detail.push_str(
                " Sandbox: the reply is recorded and the audio synthesized, \
                 delivered through the real stream path.",
            );
        }
    } else {
        detail.push_str(&format!(
            ". The service announced {}",
            delivery.announced.label
        ));
        if delivery.mismatches.is_empty() {
            detail.push('.');
        } else {
            detail.push_str(&format!(": {}.", delivery.mismatches.join(", ")));
        }
        if delivery.sandbox {
            detail.push_str(
                " Sandbox: Mujin Music's test encoder (ffmpeg's built-in AAC) cannot \
                 reach every announced bitrate, so this check is working as \
                 intended.",
            );
        }
    }
    detail
}

/// Changes to a playlist copy; fields left out stay as they are.
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct CopyEdit {
    pub title: Option<String>,
    pub description: Option<String>,
    /// The copy's tracks in their new order. Ids not in the copy are added.
    pub track_ids: Option<Vec<String>>,
}

fn copy_title(title: &str) -> HubResult<String> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 200 {
        return Err(HubError::Invalid(
            "Give the playlist a name of up to 200 characters.",
        ));
    }
    Ok(title.to_string())
}

fn mark_tracks(tracks: &mut [ProviderTrack], ids: &HashSet<String>) {
    for track in tracks {
        track.in_library =
            super::parse_entity_id(&track.id).is_some_and(|(_, raw)| ids.contains(&raw));
    }
}

fn mark_albums(
    albums: &mut [ProviderAlbum],
    album_ids: &HashSet<String>,
    track_ids: &HashSet<String>,
) {
    for album in albums {
        album.in_library =
            super::parse_entity_id(&album.id).is_some_and(|(_, raw)| album_ids.contains(&raw));
        mark_tracks(&mut album.tracks, track_ids);
    }
}

/// A message safe to show listeners: no tokens, URLs or upstream bodies.
pub fn user_message(error: &HubError) -> String {
    match error {
        HubError::Provider(error) => match error {
            ProviderError::NotConnected => {
                "Not connected. Sign in from Settings > Services.".into()
            }
            ProviderError::SessionExpired => {
                "Your session expired. Sign in again from Settings > Services.".into()
            }
            ProviderError::InvalidCredentials(message) | ProviderError::Unavailable(message) => {
                message.clone()
            }
            ProviderError::NotFound => "That item is not available on this service.".into(),
            ProviderError::RateLimited => "The service is busy. Try again in a moment.".into(),
            ProviderError::Upstream(_) | ProviderError::Decode(_) => {
                "The service returned an unexpected answer. Try again later.".into()
            }
            ProviderError::Transport(_) => {
                "Could not reach the service. Check the server's internet connection.".into()
            }
        },
        HubError::Database(_) => "Mujin Music could not save this change.".into(),
        HubError::Invalid(message) | HubError::Conflict(message) => (*message).to_string(),
        HubError::CredentialsRequired(ProviderId::Tidal) => {
            "This server has no TIDAL client yet. Ask the server owner to add one in Settings > Services, or use your own client ID.".into()
        }
        HubError::CredentialsRequired(ProviderId::Qobuz) => {
            "This server has no Qobuz app yet. Ask the server owner to add one in Settings > Services, or use your own app ID and secret.".into()
        }
    }
}

#[cfg(test)]
mod delivery_tests {
    use super::*;

    fn stream(format: AudioFormat) -> ResolvedStream {
        ResolvedStream {
            source: StreamSource::Progressive {
                url: "https://media.test/a".into(),
                mime: "audio/mp4".into(),
            },
            format,
            requested: StreamQuality::High,
            stream_seconds: None,
        }
    }

    fn probed(format: AudioFormat, seconds: Option<f64>) -> ProbedFormat {
        ProbedFormat {
            format,
            container: "mp4",
            frame_verified: false,
            seconds,
        }
    }

    #[test]
    fn a_short_lossy_bitrate_is_a_mismatch_not_a_match() {
        let announced = AudioFormat::lossy("aac", 320);
        let delivery = delivered_stream(
            "tidal:1".into(),
            &stream(announced.clone()),
            Some(probed(AudioFormat::lossy("aac", 255), None)),
            false,
        );
        assert!(!delivery.matches_announced);
        assert_eq!(delivery.mismatches, vec!["255 kbps, not 320".to_string()]);
        assert!(
            describe_delivery(&delivery)
                .contains("The service announced AAC 320: 255 kbps, not 320.")
        );

        // Encoders round: within 10% is the announced bitrate.
        for close in [290, 318, 320, 330, 352] {
            let delivery = delivered_stream(
                "tidal:1".into(),
                &stream(announced.clone()),
                Some(probed(AudioFormat::lossy("aac", close), None)),
                false,
            );
            assert!(delivery.matches_announced, "{close}");
        }
        for far in [287, 353] {
            assert!(!bitrate_matches(320, far), "{far}");
        }

        // Another codec is a mismatch whatever the bitrate.
        let delivery = delivered_stream(
            "qobuz:1".into(),
            &stream(AudioFormat::lossy("mp3", 320)),
            Some(probed(AudioFormat::lossy("aac", 320), None)),
            false,
        );
        assert_eq!(delivery.mismatches, vec!["AAC, not MP3".to_string()]);
    }

    #[test]
    fn lossless_mismatches_name_the_depth_and_rate() {
        let delivery = delivered_stream(
            "qobuz:1".into(),
            &stream(AudioFormat::lossless("flac", Some(24), Some(192_000))),
            Some(probed(
                AudioFormat::lossless("flac", Some(16), Some(44_100)),
                None,
            )),
            false,
        );
        assert_eq!(
            delivery.mismatches,
            vec![
                "16-bit, not 24-bit".to_string(),
                "44.1 kHz, not 192 kHz".to_string()
            ]
        );
        let delivery = delivered_stream(
            "tidal:1".into(),
            &stream(AudioFormat::lossless("flac", Some(16), Some(44_100))),
            Some(probed(AudioFormat::lossy("aac", 320), None)),
            false,
        );
        assert_eq!(
            delivery.mismatches,
            vec!["lossy AAC 320, not lossless".to_string()]
        );
    }

    #[test]
    fn full_length_compares_the_delivered_length_with_the_catalog() {
        let format = AudioFormat::lossless("flac", Some(24), Some(96_000));
        let mut delivery = delivered_stream(
            "tidal:1".into(),
            &stream(format.clone()),
            Some(probed(format.clone(), Some(240.6))),
            false,
        );
        check_length(&mut delivery, Some(241.0));
        assert_eq!(delivery.full_length, Some(true));
        assert!(describe_delivery(&delivery).contains("full length (4:01)"));

        let mut preview = delivered_stream(
            "tidal:1".into(),
            &stream(format.clone()),
            Some(probed(format.clone(), Some(30.0))),
            false,
        );
        check_length(&mut preview, Some(241.0));
        assert_eq!(preview.full_length, Some(false));
        assert!(describe_delivery(&preview).contains("only 0:30 of 4:01 arrived"));

        // A DASH timeline's length wins over a header that leaves it out.
        let mut dash = stream(format.clone());
        dash.stream_seconds = Some(241.2);
        let mut delivery = delivered_stream(
            "tidal:1".into(),
            &dash,
            Some(probed(format.clone(), None)),
            false,
        );
        check_length(&mut delivery, Some(241.0));
        assert_eq!(delivery.delivered_seconds, Some(241.2));
        assert_eq!(delivery.full_length, Some(true));

        // Unknown lengths and the 30-second sandbox give no verdict.
        let mut unknown = delivered_stream(
            "tidal:1".into(),
            &stream(format.clone()),
            Some(probed(format.clone(), None)),
            false,
        );
        check_length(&mut unknown, Some(241.0));
        assert_eq!(unknown.full_length, None);
        let mut sandbox = delivered_stream(
            "tidal:1".into(),
            &stream(format.clone()),
            Some(probed(format, Some(30.0))),
            true,
        );
        check_length(&mut sandbox, Some(241.0));
        assert_eq!(sandbox.full_length, None);
    }
}
