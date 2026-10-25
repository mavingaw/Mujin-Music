//! Per-user orchestration: sealed credentials, sign-in flows, token refresh,
//! library add/remove/sync and stream resolution for every provider.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

use diesel::sqlite::SqliteConnection;
use serde::Serialize;

use super::artwork::ArtworkMode;
use super::media::{self, MediaTransport, ReqwestMedia};
use super::probe::{PROBE_BYTES, ProbedFormat};
use super::qobuz::QobuzClient;
use super::store::{
    self, AccountRow, AccountStatus, ItemOrigin, LibraryCounts, LibraryItemRow, LibraryQuery,
    NewLibraryItem,
};
use super::tidal::{DeviceAuthorization, DevicePoll, TidalClient};
use super::transport::{ReqwestTransport, Transport};
use super::vault::{SealPurpose, TokenVault};
use super::{
    AudioFormat, BrowsePage, BrowseSection, ClientCredentials, EditorialSection, ItemKind,
    ProviderAlbum, ProviderArtist, ProviderContext, ProviderError, ProviderGenre, ProviderId,
    ProviderPlaylist, ProviderSession, ProviderTrack, QualityOption, RADIO_TRACKS, Radio,
    RadioSeed, ResolvedStream, SearchResults, SourcePreference, StreamQuality, StreamSource,
    StreamingProvider, sandbox, unix_now,
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
    pub pending: Option<PendingSignIn>,
    pub library: LibraryCounts,
    pub sandbox: bool,
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
    /// Whether the delivered format agrees with the announced one.
    pub matches_announced: bool,
    pub sandbox: bool,
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
    server_credentials: RwLock<HashMap<ProviderId, ServerCredential>>,
    media: Arc<dyn MediaTransport>,
    deliveries: Mutex<HashMap<(i32, String), DeliveredStream>>,
    segment_indexes: Mutex<HashMap<String, media::SegmentIndex>>,
    /// Each service's genre list per listener (genres vary by country),
    /// with when it was read.
    genre_lists: Mutex<GenreLists>,
    /// Genres learned from genre pages, by album id. TIDAL albums carry no
    /// genre, so an album found under TIDAL's "Jazz" is filed as Jazz when
    /// it is added to the library.
    genre_hints: Mutex<HashMap<String, String>>,
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
            },
        );
    }
    if let Some(client_id) = env_value("PARSON_QOBUZ_APP_ID") {
        credentials.insert(
            ProviderId::Qobuz,
            ClientCredentials {
                client_id,
                client_secret: env_value("PARSON_QOBUZ_APP_SECRET"),
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
    })
}

impl StreamingHub {
    pub fn from_env(pool: DbPool) -> Result<Self, String> {
        let key = crate::settings::streaming_token_key()
            .map_err(|error| format!("streaming token key: {error}"))?;
        let sandbox = sandbox::enabled_from_env();
        let transport: Arc<dyn Transport> = if sandbox {
            tracing::warn!("streaming services run in sandbox mode with recorded responses");
            Arc::new(sandbox::transport())
        } else {
            Arc::new(ReqwestTransport::new().map_err(|error| error.to_string())?)
        };
        let media: Arc<dyn MediaTransport> =
            Arc::new(ReqwestMedia::new().map_err(|error| error.to_string())?);
        let hub = Self::new(
            pool,
            TokenVault::new(key),
            transport,
            sandbox,
            server_credentials_from_env(),
        )
        .with_media(media);
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
            qobuz: QobuzClient::new(transport, artwork),
            sandbox,
            server_credentials: RwLock::new(server_credentials),
            media: Arc::new(media::FixtureMedia::new()),
            deliveries: Mutex::new(HashMap::new()),
            segment_indexes: Mutex::new(HashMap::new()),
            genre_lists: Mutex::new(HashMap::new()),
            genre_hints: Mutex::new(HashMap::new()),
        }
    }

    /// Replaces how stream bytes are fetched (live HTTP in the server,
    /// recorded media files in tests).
    pub fn with_media(mut self, media: Arc<dyn MediaTransport>) -> Self {
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

    fn server_credential(&self, provider: ProviderId) -> Option<ServerCredential> {
        self.server_credentials
            .read()
            .ok()
            .and_then(|map| map.get(&provider).cloned())
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
            pending,
            library,
            sandbox: self.sandbox,
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

    async fn context(
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
        if self.sandbox {
            let seed = raw_id.bytes().fold(1469598103934665603_u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(1099511628211)
            });
            let mut format = AudioFormat::lossless("pcm", Some(16), Some(44_100));
            format.label = "Sandbox preview".to_string();
            return Ok(ResolvedStream {
                source: StreamSource::SandboxTone {
                    seed,
                    seconds: sandbox::PREVIEW_SECONDS,
                },
                format,
                requested: quality,
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
                "This playlist was made in Parson and has no service playlist to save again.",
            ));
        }
        if !replace_edits {
            let lookup = raw_id.clone();
            let existing = self
                .db(move |connection| store::playlist_copy(connection, user_id, provider, &lookup))
                .await?;
            if existing.is_some_and(|row| row.edited_at.is_some()) {
                return Err(HubError::Conflict(
                    "You edited this copy in Parson. Saving it again replaces your edits with the service playlist.",
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
            owner: Some("You, in Parson".to_string()),
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
        let delivery = delivered_stream(track_id.clone(), stream, probed, self.sandbox);
        if let Ok(mut deliveries) = self.deliveries.lock() {
            if deliveries.len() >= MAX_DELIVERIES
                && let Some(oldest) = deliveries
                    .iter()
                    .min_by_key(|(_, delivery)| delivery.at)
                    .map(|(key, _)| key.clone())
            {
                deliveries.remove(&oldest);
            }
            deliveries.insert((user_id, track_id), delivery.clone());
        }
        delivery
    }

    /// The most recent delivered format for one of the listener's streams.
    pub fn delivery(&self, user_id: i32, track_id: &str) -> Option<DeliveredStream> {
        self.deliveries
            .lock()
            .ok()?
            .get(&(user_id, track_id.to_string()))
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
        match self.sample_stream(&stream).await {
            Ok(probed) => {
                let delivery = self.record_delivery(user_id, track_id, &stream, probed);
                let ok = delivery.delivered.is_some();
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

fn delivered_stream(
    track_id: String,
    stream: &ResolvedStream,
    probed: Option<ProbedFormat>,
    sandbox: bool,
) -> DeliveredStream {
    let announced = stream.format.clone();
    let (delivered, container, frame_verified) = match probed {
        Some(probed) => (
            Some(probed.format),
            Some(probed.container),
            probed.frame_verified,
        ),
        None => (None, None, false),
    };
    let matches_announced = delivered.as_ref().is_some_and(|delivered| {
        // Sandbox previews announce no real format.
        !sandbox
            && delivered.lossless == announced.lossless
            && (!announced.lossless
                || (delivered.codec == announced.codec
                    && announced
                        .bit_depth
                        .is_none_or(|depth| delivered.bit_depth == Some(depth))
                    && announced
                        .sample_rate
                        .is_none_or(|rate| delivered.sample_rate == Some(rate))))
    });
    DeliveredStream {
        track_id,
        requested: stream.requested,
        announced,
        delivered,
        container,
        frame_verified,
        matches_announced,
        sandbox,
        at: unix_now(),
    }
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
    if delivery.sandbox {
        detail.push_str(". Sandbox preview tone, not a real stream.");
    } else if delivery.matches_announced {
        detail.push_str(&format!(
            ", as the service announced ({}).",
            delivery.announced.label
        ));
    } else {
        detail.push_str(&format!(
            ". The service announced {}.",
            delivery.announced.label
        ));
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
        HubError::Database(_) => "Parson could not save this change.".into(),
        HubError::Invalid(message) | HubError::Conflict(message) => (*message).to_string(),
        HubError::CredentialsRequired(ProviderId::Tidal) => {
            "This server has no TIDAL client yet. Ask the server owner to add one in Settings > Services, or use your own client ID.".into()
        }
        HubError::CredentialsRequired(ProviderId::Qobuz) => {
            "This server has no Qobuz app yet. Ask the server owner to add one in Settings > Services, or use your own app ID and secret.".into()
        }
    }
}
