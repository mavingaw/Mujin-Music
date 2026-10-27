//! The scrobbling service: connections, the offline queue and its worker.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use diesel::QueryResult;
use diesel::sqlite::SqliteConnection;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use super::store::{self, ConnectionRow, ImportStatus, LogEntry, QueueCounts, QueueRow};
use super::{ItemOutcome, Listen, Service, ServiceError, lastfm, listenbrainz, now_unix, rules};
use crate::library::state::LibraryCache;
use crate::persistence::connection::DbPool;
use crate::providers::transport::{ReqwestTransport, Transport};
use crate::providers::vault::TokenVault;
use crate::recommendation::PlaybackEventRequest;

/// How long a Last.fm approval may take.
const AUTHORIZATION_LIFETIME: Duration = Duration::from_secs(15 * 60);
const MAX_PENDING_AUTHORIZATIONS: usize = 256;
/// Profiles flushed per worker pass.
const GROUPS_PER_PASS: i64 = 32;
/// The worker never sleeps longer than this, so a missed wake-up is cheap.
const MAX_IDLE: Duration = Duration::from_secs(60);
/// Deferred listens (Last.fm daily limit) are retried after this long.
const DEFERRED_RETRY_SECONDS: i64 = 60 * 60;
const LOG_PRUNE_INTERVAL: Duration = Duration::from_secs(60 * 60);

#[derive(Debug)]
pub enum HubError {
    /// The server has no Last.fm API key yet.
    NotConfigured,
    NotConnected,
    Invalid(String),
    Expired,
    Busy,
    Service(ServiceError),
    Storage(String),
}

impl std::fmt::Display for HubError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfigured => formatter.write_str(
                "This server has no Last.fm API key yet. An administrator can add one in Settings.",
            ),
            Self::NotConnected => formatter.write_str("This profile is not connected."),
            Self::Invalid(message) => formatter.write_str(message),
            Self::Expired => {
                formatter.write_str("The approval took too long. Start connecting again.")
            }
            Self::Busy => formatter.write_str("An import is already running."),
            Self::Service(error) => formatter.write_str(error.message()),
            Self::Storage(message) => write!(formatter, "storage error: {message}"),
        }
    }
}

impl std::error::Error for HubError {}

impl From<ServiceError> for HubError {
    fn from(error: ServiceError) -> Self {
        Self::Service(error)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct SealedSecret {
    secret: String,
}

struct PendingAuthorization {
    user_id: i32,
    expires_at: Instant,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastfmSetup {
    pub configured: bool,
    /// "settings", "environment" or "sandbox".
    pub source: Option<&'static str>,
    pub key_hint: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionView {
    pub account_name: String,
    pub profile_url: String,
    pub enabled: bool,
    pub send_now_playing: bool,
    pub scrobble_radio: bool,
    /// "ok" or "reauth_required".
    pub status: String,
    pub last_error: Option<String>,
    pub last_scrobbled_at: Option<i64>,
    pub connected_at: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatus {
    pub service: Service,
    pub name: &'static str,
    /// Whether a profile can connect (Last.fm needs the server's API key).
    pub available: bool,
    pub connection: Option<ConnectionView>,
    pub queue: QueueCounts,
    pub import: Option<ImportStatus>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrobblingStatus {
    pub services: Vec<ServiceStatus>,
    pub lastfm_setup: LastfmSetup,
    pub recent: Vec<LogEntry>,
    pub sandbox: bool,
}

pub struct ScrobbleHub {
    pool: DbPool,
    vault: TokenVault,
    transport: Arc<dyn Transport>,
    lastfm_api: String,
    lastfm_auth: String,
    listenbrainz_api: String,
    env_lastfm: Option<lastfm::Credentials>,
    sandbox: bool,
    wake: Notify,
    pending: Mutex<HashMap<String, PendingAuthorization>>,
    imports: Mutex<HashSet<(i32, Service)>>,
}

fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Validates the origin a client reached the server at, used to build the
/// Last.fm callback. Only `http(s)://host[:port][/path]` is accepted.
pub fn callback_base(value: &str) -> Option<String> {
    let url = reqwest::Url::parse(value.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    Some(url.as_str().trim_end_matches('/').to_string())
}

/// What a song in the library is called on Last.fm and ListenBrainz, and
/// its duration in seconds.
pub fn listen_for_song(
    cache: &LibraryCache,
    song_id: &str,
    listened_at: i64,
    chosen_by_user: bool,
) -> Option<(Listen, f64)> {
    let song = cache.song(song_id)?;
    let (artist_id, album_id) = cache.song_map.get(song_id)?;
    let album = cache.album(album_id);
    let album_artist = cache
        .artist(artist_id)
        .and_then(|artist| super::clean(&artist.name));
    let artist = super::clean(&song.artist).or_else(|| album_artist.clone())?;
    let duration = song.duration;
    Some((
        Listen {
            artist,
            track: super::clean(&song.name)?,
            album: album.and_then(|album| super::clean(&album.name)),
            album_artist,
            duration_seconds: (duration.is_finite() && duration >= 1.0)
                .then(|| duration.round() as u32),
            track_number: (song.track_number > 0).then_some(u32::from(song.track_number)),
            release_mbid: album
                .map(|album| album.musicbrainz_id.trim())
                .filter(|mbid| uuid::Uuid::parse_str(mbid).is_ok())
                .map(str::to_string),
            isrc: song.isrc.as_deref().and_then(super::clean),
            listened_at,
            chosen_by_user,
        },
        duration,
    ))
}

impl ScrobbleHub {
    pub fn from_env(pool: DbPool) -> Result<Self, String> {
        let key = crate::settings::streaming_token_key()
            .map_err(|error| format!("scrobbling key: {error}"))?;
        let sandbox = super::sandbox::enabled_from_env();
        let transport: Arc<dyn Transport> = if sandbox {
            tracing::warn!("scrobbling runs in sandbox mode with recorded service replies");
            super::sandbox::transport()
        } else {
            Arc::new(ReqwestTransport::new().map_err(|error| error.to_string())?)
        };
        let env_lastfm = match (
            env_value("PARSON_LASTFM_API_KEY"),
            env_value("PARSON_LASTFM_API_SECRET"),
        ) {
            (Some(api_key), Some(api_secret)) => Some(lastfm::Credentials {
                api_key,
                api_secret,
            }),
            _ => None,
        };
        let mut hub = Self::new(pool, TokenVault::new(key), transport, sandbox);
        hub.env_lastfm = env_lastfm;
        if let Some(url) = env_value("PARSON_LISTENBRAINZ_API_URL") {
            hub.listenbrainz_api = url;
        }
        Ok(hub)
    }

    pub fn new(
        pool: DbPool,
        vault: TokenVault,
        transport: Arc<dyn Transport>,
        sandbox: bool,
    ) -> Self {
        Self {
            pool,
            vault,
            transport,
            lastfm_api: lastfm::API_URL.to_string(),
            lastfm_auth: lastfm::AUTH_URL.to_string(),
            listenbrainz_api: listenbrainz::API_URL.to_string(),
            env_lastfm: None,
            sandbox,
            wake: Notify::new(),
            pending: Mutex::new(HashMap::new()),
            imports: Mutex::new(HashSet::new()),
        }
    }

    pub fn is_sandbox(&self) -> bool {
        self.sandbox
    }

    pub(crate) fn transport(&self) -> &dyn Transport {
        self.transport.as_ref()
    }

    pub(crate) fn listenbrainz_api(&self) -> &str {
        &self.listenbrainz_api
    }

    pub(crate) fn lastfm_api(&self) -> &str {
        &self.lastfm_api
    }

    /// Wakes the queue worker.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    pub(crate) async fn db<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut SqliteConnection) -> QueryResult<T> + Send + 'static,
    ) -> Result<T, HubError> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let mut connection = pool.get().map_err(|error| error.to_string())?;
            work(&mut connection).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| HubError::Storage(error.to_string()))?
        .map_err(HubError::Storage)
    }

    fn secret_context(user_id: i32, service: Service) -> String {
        format!("parson-scrobbling:v1:{user_id}:{}:secret", service.as_str())
    }

    const SERVER_CONTEXT: &'static str = "parson-scrobbling:v1:0:lastfm:server";

    fn seal_secret(
        &self,
        user_id: i32,
        service: Service,
        secret: &str,
    ) -> Result<Vec<u8>, HubError> {
        self.vault
            .seal_in_context(
                &Self::secret_context(user_id, service),
                &SealedSecret {
                    secret: secret.to_string(),
                },
            )
            .map_err(|error| HubError::Storage(error.to_string()))
    }

    fn open_secret(&self, row: &ConnectionRow) -> Option<String> {
        let service = row.service()?;
        self.vault
            .open_in_context::<SealedSecret>(
                &Self::secret_context(row.user_id, service),
                &row.sealed_secret,
            )
            .ok()
            .map(|sealed| sealed.secret)
    }

    // ---- Last.fm server credentials -------------------------------------

    pub async fn lastfm_credentials(
        &self,
    ) -> Result<Option<(lastfm::Credentials, &'static str)>, HubError> {
        if let Some(credentials) = &self.env_lastfm {
            return Ok(Some((credentials.clone(), "environment")));
        }
        let sealed = self
            .db(|connection| store::server_config(connection, Service::Lastfm))
            .await?;
        if let Some(sealed) = sealed {
            match self
                .vault
                .open_in_context::<lastfm::Credentials>(Self::SERVER_CONTEXT, &sealed)
            {
                Ok(credentials) => return Ok(Some((credentials, "settings"))),
                Err(_) => tracing::warn!("stored Last.fm API credentials could not be opened"),
            }
        }
        if self.sandbox {
            return Ok(Some((super::sandbox::credentials(), "sandbox")));
        }
        Ok(None)
    }

    pub async fn lastfm_setup(&self) -> Result<LastfmSetup, HubError> {
        Ok(match self.lastfm_credentials().await? {
            Some((credentials, source)) => LastfmSetup {
                configured: true,
                source: Some(source),
                key_hint: Some(credentials.key_hint()),
            },
            None => LastfmSetup {
                configured: false,
                source: None,
                key_hint: None,
            },
        })
    }

    /// Saves the server's Last.fm API key and secret after Last.fm accepted
    /// them for a signed call.
    pub async fn set_lastfm_credentials(
        &self,
        admin_id: i32,
        api_key: &str,
        api_secret: &str,
    ) -> Result<LastfmSetup, HubError> {
        let credentials = lastfm::Credentials {
            api_key: api_key.trim().to_ascii_lowercase(),
            api_secret: api_secret.trim().to_ascii_lowercase(),
        };
        if !lastfm::Credentials::plausible(&credentials.api_key)
            || !lastfm::Credentials::plausible(&credentials.api_secret)
        {
            return Err(HubError::Invalid(
                "A Last.fm API key and shared secret are each 32 letters and digits.".into(),
            ));
        }
        let client = lastfm::Client {
            transport: self.transport.as_ref(),
            base: &self.lastfm_api,
            credentials: &credentials,
        };
        client.get_token().await.map_err(|error| match error {
            ServiceError::BadCredentials(_) | ServiceError::Rejected(_) => HubError::Invalid(
                "Last.fm rejected this API key and secret. Check both values on last.fm/api/accounts."
                    .into(),
            ),
            other => HubError::Service(other),
        })?;
        let sealed = self
            .vault
            .seal_in_context(Self::SERVER_CONTEXT, &credentials)
            .map_err(|error| HubError::Storage(error.to_string()))?;
        let now = now_unix();
        self.db(move |connection| {
            store::set_server_config(connection, Service::Lastfm, &sealed, admin_id, now)
        })
        .await?;
        self.lastfm_setup().await
    }

    pub async fn clear_lastfm_credentials(&self) -> Result<LastfmSetup, HubError> {
        self.db(|connection| store::delete_server_config(connection, Service::Lastfm))
            .await?;
        self.lastfm_setup().await
    }

    // ---- Connecting -------------------------------------------------------

    /// Starts Last.fm's web authentication: returns the page the listener
    /// opens. Last.fm sends the browser back to this server's
    /// `/connect/lastfm` page, which completes the connection with the
    /// one-time `state`; any browser works, signed in or not.
    pub async fn start_lastfm_authorization(
        &self,
        user_id: i32,
        origin: &str,
    ) -> Result<(String, u64), HubError> {
        let base = callback_base(origin)
            .ok_or_else(|| HubError::Invalid("The server address is not valid.".into()))?;
        let (credentials, _) = self
            .lastfm_credentials()
            .await?
            .ok_or(HubError::NotConfigured)?;
        let state = uuid::Uuid::new_v4().simple().to_string();
        {
            let mut pending = lock(&self.pending);
            let now = Instant::now();
            pending.retain(|_, request| request.expires_at > now);
            if pending.len() >= MAX_PENDING_AUTHORIZATIONS {
                return Err(HubError::Invalid(
                    "Too many Last.fm approvals are waiting. Try again in a few minutes.".into(),
                ));
            }
            pending.insert(
                state.clone(),
                PendingAuthorization {
                    user_id,
                    expires_at: now + AUTHORIZATION_LIFETIME,
                },
            );
        }
        let callback = format!("{base}/connect/lastfm?state={state}");
        let url = if self.sandbox {
            // The sandbox approves at once, as if the listener had clicked
            // "Yes, allow access" on Last.fm.
            format!("{callback}&token=sandbox-approved-token")
        } else {
            lastfm::authorize_url(&self.lastfm_auth, &credentials.api_key, &callback)
        };
        Ok((url, AUTHORIZATION_LIFETIME.as_secs()))
    }

    /// Finishes Last.fm's web authentication from the callback.
    pub async fn complete_lastfm_authorization(
        &self,
        state: &str,
        token: &str,
    ) -> Result<(i32, String), HubError> {
        let user_id = {
            let mut pending = lock(&self.pending);
            let request = pending.remove(state).ok_or(HubError::Expired)?;
            if request.expires_at <= Instant::now() {
                return Err(HubError::Expired);
            }
            request.user_id
        };
        let token = token.trim();
        if token.is_empty() || token.len() > 128 || !token.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(HubError::Invalid(
                "Last.fm did not approve the connection.".into(),
            ));
        }
        let (credentials, _) = self
            .lastfm_credentials()
            .await?
            .ok_or(HubError::NotConfigured)?;
        let client = lastfm::Client {
            transport: self.transport.as_ref(),
            base: &self.lastfm_api,
            credentials: &credentials,
        };
        let session = client.get_session(token).await?;
        let sealed = self.seal_secret(user_id, Service::Lastfm, &session.key)?;
        let name = session.name.clone();
        let now = now_unix();
        self.db(move |connection| {
            store::upsert_connection(connection, user_id, Service::Lastfm, &name, &sealed, now)
        })
        .await?;
        self.wake();
        Ok((user_id, session.name))
    }

    /// Connects ListenBrainz with the listener's user token.
    pub async fn connect_listenbrainz(
        &self,
        user_id: i32,
        token: &str,
    ) -> Result<String, HubError> {
        let token = token.trim();
        if !listenbrainz::plausible_token(token) {
            return Err(HubError::Invalid(
                "Paste the user token from listenbrainz.org/settings.".into(),
            ));
        }
        let client = listenbrainz::Client {
            transport: self.transport.as_ref(),
            base: &self.listenbrainz_api,
        };
        let name = client
            .validate_token(token)
            .await
            .map_err(|error| match error {
                ServiceError::AuthRevoked(_) | ServiceError::Rejected(_) => {
                    HubError::Invalid("ListenBrainz did not recognise that token.".into())
                }
                other => HubError::Service(other),
            })?;
        let sealed = self.seal_secret(user_id, Service::Listenbrainz, token)?;
        let stored_name = name.clone();
        let now = now_unix();
        self.db(move |connection| {
            store::upsert_connection(
                connection,
                user_id,
                Service::Listenbrainz,
                &stored_name,
                &sealed,
                now,
            )
        })
        .await?;
        self.wake();
        Ok(name)
    }

    pub async fn update_settings(
        &self,
        user_id: i32,
        service: Service,
        patch: store::SettingsPatch,
    ) -> Result<(), HubError> {
        let updated = self
            .db(move |connection| store::update_settings(connection, user_id, service, patch))
            .await?;
        if !updated {
            return Err(HubError::NotConnected);
        }
        if patch.enabled == Some(true) {
            let now = now_unix();
            self.db(move |connection| store::retry_now(connection, user_id, now))
                .await?;
            self.wake();
        }
        Ok(())
    }

    pub async fn disconnect(&self, user_id: i32, service: Service) -> Result<(), HubError> {
        let removed = self
            .db(move |connection| store::delete_connection(connection, user_id, service))
            .await?;
        if removed {
            Ok(())
        } else {
            Err(HubError::NotConnected)
        }
    }

    pub async fn retry(&self, user_id: i32) -> Result<usize, HubError> {
        let now = now_unix();
        let count = self
            .db(move |connection| store::retry_now(connection, user_id, now))
            .await?;
        self.wake();
        Ok(count)
    }

    pub async fn status(&self, user_id: i32) -> Result<ScrobblingStatus, HubError> {
        let lastfm_setup = self.lastfm_setup().await?;
        let (connections, counts, imports, recent) = self
            .db(move |connection| {
                let connections = store::connections(connection, user_id)?;
                let mut counts = Vec::new();
                let mut imports = Vec::new();
                for service in Service::ALL {
                    counts.push(store::queue_counts(connection, user_id, service)?);
                    imports.push(store::import_status(connection, user_id, service)?);
                }
                let recent = store::recent_log(connection, user_id, 40)?;
                Ok((connections, counts, imports, recent))
            })
            .await?;
        let services = Service::ALL
            .into_iter()
            .zip(counts.into_iter().zip(imports))
            .map(|(service, (queue, import))| {
                let connection = connections
                    .iter()
                    .find(|row| row.service() == Some(service))
                    .map(|row| ConnectionView {
                        account_name: row.account_name.clone(),
                        profile_url: profile_url(service, &row.account_name),
                        enabled: row.enabled != 0,
                        send_now_playing: row.send_now_playing != 0,
                        scrobble_radio: row.scrobble_radio != 0,
                        status: row.status.clone(),
                        last_error: row.last_error.clone(),
                        last_scrobbled_at: row.last_scrobbled_at,
                        connected_at: row.connected_at,
                    });
                ServiceStatus {
                    service,
                    name: service.display_name(),
                    available: service == Service::Listenbrainz || lastfm_setup.configured,
                    connection,
                    queue,
                    import,
                }
            })
            .collect();
        Ok(ScrobblingStatus {
            services,
            lastfm_setup,
            recent,
            sandbox: self.sandbox,
        })
    }

    // ---- Playback ----------------------------------------------------------

    /// Reacts to a playback event a client reported: "now playing" when a
    /// track starts, a queued scrobble when it qualifies.
    pub async fn observe(
        &self,
        user_id: i32,
        event: PlaybackEventRequest,
        cache: Arc<LibraryCache>,
    ) -> Result<usize, HubError> {
        let starting = matches!(
            event.event_type.as_str(),
            "play_started" | "manual_selection"
        );
        let qualified = event.event_type == "qualified_play";
        if !starting && !qualified {
            return Ok(0);
        }
        let connections = self
            .db(move |connection| store::connections(connection, user_id))
            .await?
            .into_iter()
            .filter(ConnectionRow::active)
            .collect::<Vec<_>>();
        if connections.is_empty() {
            return Ok(0);
        }
        let chosen_by_user = event.source.as_deref() != Some("generated");
        let now = now_unix();
        let Some((mut listen, library_duration)) =
            listen_for_song(&cache, &event.song_id, now, chosen_by_user)
        else {
            return Ok(0);
        };
        let duration = if library_duration.is_finite() && library_duration > 0.0 {
            library_duration
        } else {
            event.duration_seconds.unwrap_or(0.0)
        };
        let wanted = connections
            .into_iter()
            .filter(|row| chosen_by_user || row.scrobble_radio != 0)
            .collect::<Vec<_>>();

        if starting {
            let mut sent = 0;
            for row in wanted.iter().filter(|row| row.send_now_playing != 0) {
                if self.send_now_playing(row, &listen).await {
                    sent += 1;
                }
            }
            return Ok(sent);
        }

        if !rules::long_enough(duration) {
            return Ok(0);
        }
        let position = event.position_seconds.unwrap_or(0.0);
        let started = match event.session_id.clone() {
            Some(session_id) => {
                let song_id = event.song_id.clone();
                // A start older than the track plus an hour of pauses is
                // from an earlier play of the same song.
                let not_before = now - duration.ceil() as i64 - 3600;
                self.db(move |connection| {
                    store::playback_started_at(
                        connection,
                        user_id,
                        &session_id,
                        &song_id,
                        not_before,
                    )
                })
                .await?
            }
            None => None,
        };
        listen.listened_at = rules::listened_at(started, now, position);
        let song_id = event.song_id.clone();
        let services = wanted
            .iter()
            .filter_map(ConnectionRow::service)
            .collect::<Vec<_>>();
        let queued = self
            .db(move |connection| {
                let mut queued = 0;
                for service in services {
                    if store::enqueue(connection, user_id, service, Some(&song_id), &listen, now)? {
                        queued += 1;
                    }
                }
                Ok(queued)
            })
            .await?;
        if queued > 0 {
            self.wake();
        }
        Ok(queued)
    }

    async fn send_now_playing(&self, row: &ConnectionRow, listen: &Listen) -> bool {
        let (Some(service), Some(secret)) = (row.service(), self.open_secret(row)) else {
            return false;
        };
        let result = match service {
            Service::Lastfm => match self.lastfm_credentials().await {
                Ok(Some((credentials, _))) => {
                    lastfm::Client {
                        transport: self.transport.as_ref(),
                        base: &self.lastfm_api,
                        credentials: &credentials,
                    }
                    .update_now_playing(&secret, listen)
                    .await
                }
                _ => return false,
            },
            Service::Listenbrainz => {
                listenbrainz::Client {
                    transport: self.transport.as_ref(),
                    base: &self.listenbrainz_api,
                }
                .submit(
                    &secret,
                    listenbrainz::ListenType::PlayingNow,
                    std::slice::from_ref(listen),
                )
                .await
            }
        };
        match result {
            Ok(()) => true,
            Err(ServiceError::AuthRevoked(message)) => {
                let user_id = row.user_id;
                let _ = self
                    .db(move |connection| {
                        store::mark_reauth_required(connection, user_id, service, &message)
                    })
                    .await;
                false
            }
            Err(error) => {
                tracing::debug!(%error, service = service.as_str(), "now playing not delivered");
                false
            }
        }
    }

    // ---- Queue delivery ---------------------------------------------------

    async fn submit(
        &self,
        service: Service,
        secret: &str,
        listens: &[Listen],
    ) -> Result<Vec<ItemOutcome>, ServiceError> {
        match service {
            Service::Lastfm => {
                let Ok(Some((credentials, _))) = self.lastfm_credentials().await else {
                    return Err(ServiceError::BadCredentials(
                        "This server has no Last.fm API key.".into(),
                    ));
                };
                lastfm::Client {
                    transport: self.transport.as_ref(),
                    base: &self.lastfm_api,
                    credentials: &credentials,
                }
                .scrobble(secret, listens)
                .await
            }
            Service::Listenbrainz => {
                let kind = if listens.len() == 1 {
                    listenbrainz::ListenType::Single
                } else {
                    listenbrainz::ListenType::Import
                };
                listenbrainz::Client {
                    transport: self.transport.as_ref(),
                    base: &self.listenbrainz_api,
                }
                .submit(secret, kind, listens)
                .await
                .map(|()| vec![ItemOutcome::Accepted; listens.len()])
            }
        }
    }

    /// Delivers one batch of due listens for a profile and service.
    /// Returns how many listens the service accepted.
    pub async fn flush(&self, user_id: i32, service: Service) -> Result<usize, HubError> {
        let now = now_unix();
        let batch = service.max_batch();
        let (row, due) = self
            .db(move |connection| {
                let row = store::connection(connection, user_id, service)?;
                let due = match &row {
                    Some(row) if row.active() => {
                        store::due_rows(connection, user_id, service, now, batch)?
                    }
                    _ => Vec::new(),
                };
                Ok((row, due))
            })
            .await?;
        let Some(row) = row else {
            return Ok(0);
        };
        if due.is_empty() {
            return Ok(0);
        }
        let Some(secret) = self.open_secret(&row) else {
            self.db(move |connection| {
                store::mark_reauth_required(
                    connection,
                    user_id,
                    service,
                    "The saved sign-in could not be read. Connect again.",
                )
            })
            .await?;
            return Ok(0);
        };
        let listens = due.iter().map(QueueRow::listen).collect::<Vec<_>>();
        match self.submit(service, &secret, &listens).await {
            Ok(outcomes) => self.record_outcomes(user_id, service, due, outcomes).await,
            Err(ServiceError::AuthRevoked(message)) => {
                self.db(move |connection| {
                    store::mark_reauth_required(connection, user_id, service, &message)
                })
                .await?;
                Ok(0)
            }
            Err(ServiceError::Rejected(message)) if due.len() > 1 => {
                // One bad listen must not hold the rest back: deliver them
                // one by one so only the offending listen fails.
                tracing::warn!(%message, service = service.as_str(), "batch rejected; sending singly");
                let mut accepted = 0;
                for row in due {
                    let listen = row.listen();
                    match self
                        .submit(service, &secret, std::slice::from_ref(&listen))
                        .await
                    {
                        Ok(outcomes) => {
                            accepted += self
                                .record_outcomes(user_id, service, vec![row], outcomes)
                                .await?;
                        }
                        Err(error) => {
                            self.record_failure(user_id, service, vec![row], error)
                                .await?
                        }
                    }
                }
                Ok(accepted)
            }
            Err(error) => {
                self.record_failure(user_id, service, due, error).await?;
                Ok(0)
            }
        }
    }

    async fn record_outcomes(
        &self,
        user_id: i32,
        service: Service,
        rows: Vec<QueueRow>,
        outcomes: Vec<ItemOutcome>,
    ) -> Result<usize, HubError> {
        let now = now_unix();
        self.db(move |connection| {
            let mut accepted = 0;
            for (row, outcome) in rows.iter().zip(outcomes) {
                match outcome {
                    ItemOutcome::Accepted => {
                        accepted += 1;
                        store::mark_delivered(connection, row.id, "sent", None, now)?;
                    }
                    ItemOutcome::Ignored(reason) => {
                        store::mark_delivered(connection, row.id, "ignored", Some(&reason), now)?;
                    }
                    ItemOutcome::Deferred(reason) => {
                        store::defer(
                            connection,
                            row,
                            &reason,
                            now + DEFERRED_RETRY_SECONDS,
                            false,
                        )?;
                    }
                }
            }
            store::record_connection_result(
                connection,
                user_id,
                service,
                None,
                (accepted > 0).then_some(now),
            )?;
            Ok(accepted)
        })
        .await
    }

    async fn record_failure(
        &self,
        user_id: i32,
        service: Service,
        rows: Vec<QueueRow>,
        error: ServiceError,
    ) -> Result<(), HubError> {
        let now = now_unix();
        let retryable = error.retryable();
        let message = error.message().to_string();
        self.db(move |connection| {
            for row in &rows {
                let attempts = row.attempts + 1;
                let give_up = !retryable || attempts >= rules::MAX_ATTEMPTS;
                store::defer(
                    connection,
                    row,
                    &message,
                    now + rules::retry_delay_seconds(attempts),
                    give_up,
                )?;
            }
            store::record_connection_result(connection, user_id, service, Some(&message), None)
        })
        .await
    }

    /// Delivers everything that is due now. Returns accepted listens.
    pub async fn process_due(&self) -> Result<usize, HubError> {
        let now = now_unix();
        let groups = self
            .db(move |connection| store::due_groups(connection, now, GROUPS_PER_PASS))
            .await?;
        let mut accepted = 0;
        for (user_id, service) in groups {
            match self.flush(user_id, service).await {
                Ok(count) => accepted += count,
                Err(error) => {
                    tracing::warn!(%error, user_id, service = service.as_str(), "scrobble delivery failed")
                }
            }
        }
        Ok(accepted)
    }

    /// The queue worker: delivers due listens, then sleeps until the next
    /// one is due or a new listen arrives.
    pub async fn run(self: Arc<Self>) {
        let now = now_unix();
        if let Err(error) = self
            .db(move |connection| store::fail_interrupted_imports(connection, now))
            .await
        {
            tracing::warn!(%error, "could not close interrupted history imports");
        }
        let mut last_prune: Option<Instant> = None;
        loop {
            if let Err(error) = self.process_due().await {
                tracing::warn!(%error, "scrobble queue pass failed");
            }
            if last_prune.is_none_or(|at| at.elapsed() >= LOG_PRUNE_INTERVAL) {
                last_prune = Some(Instant::now());
                let now = now_unix();
                if let Err(error) = self
                    .db(move |connection| store::prune_log(connection, now))
                    .await
                {
                    tracing::warn!(%error, "could not prune the scrobble log");
                }
            }
            let next = self
                .db(store::next_due_at)
                .await
                .ok()
                .flatten()
                .map(|due| Duration::from_secs((due - now_unix()).clamp(1, 60) as u64))
                .unwrap_or(MAX_IDLE);
            tokio::select! {
                _ = self.wake.notified() => {}
                _ = tokio::time::sleep(next) => {}
            }
        }
    }

    // ---- History import ---------------------------------------------------

    pub(crate) fn begin_import(&self, user_id: i32, service: Service) -> bool {
        lock(&self.imports).insert((user_id, service))
    }

    pub(crate) fn end_import(&self, user_id: i32, service: Service) {
        lock(&self.imports).remove(&(user_id, service));
    }

    pub(crate) async fn active_connection(
        &self,
        user_id: i32,
        service: Service,
    ) -> Result<(ConnectionRow, String), HubError> {
        let row = self
            .db(move |connection| store::connection(connection, user_id, service))
            .await?
            .ok_or(HubError::NotConnected)?;
        let secret = self.open_secret(&row).ok_or(HubError::NotConnected)?;
        Ok((row, secret))
    }
}

pub fn profile_url(service: Service, account: &str) -> String {
    let encoded = percent_encoding::utf8_percent_encode(account, super::PATH_SEGMENT);
    match service {
        Service::Lastfm => format!("https://www.last.fm/user/{encoded}"),
        Service::Listenbrainz => format!("https://listenbrainz.org/user/{encoded}/"),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::providers::transport::{FixtureReply, FixtureRoute, FixtureTransport, Method};
    use diesel::RunQueryDsl;
    use diesel::connection::SimpleConnection;
    use diesel::r2d2::{ConnectionManager, Pool};
    use diesel_migrations::MigrationHarness;

    pub(crate) fn pool() -> DbPool {
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

    pub(crate) fn hub_with(pool: DbPool, transport: Arc<FixtureTransport>) -> ScrobbleHub {
        let mut hub = ScrobbleHub::new(pool, TokenVault::new([9; 32]), transport, false);
        hub.env_lastfm = Some(super::super::sandbox::credentials());
        hub
    }

    pub(crate) fn fixtures() -> Arc<FixtureTransport> {
        Arc::new(
            FixtureTransport::from_json(&[
                super::super::sandbox::LASTFM_FIXTURES,
                super::super::sandbox::LISTENBRAINZ_FIXTURES,
            ])
            .expect("fixtures"),
        )
    }

    pub(crate) fn listen(track: &str, at: i64) -> Listen {
        Listen {
            artist: "Ilse Marten".into(),
            track: track.into(),
            album: Some("Glass Orchard".into()),
            duration_seconds: Some(214),
            listened_at: at,
            chosen_by_user: true,
            ..Listen::default()
        }
    }

    const LB_TOKEN: &str = "6c4b6d2e-6f4c-4d2b-9a43-0f6f7a2b9c11";

    async fn connect_both(hub: &ScrobbleHub, user_id: i32) {
        let (url, _) = hub
            .start_lastfm_authorization(user_id, "http://192.168.1.20:1993")
            .await
            .expect("authorize");
        assert!(url.starts_with("https://www.last.fm/api/auth/?api_key=5a4db0cf"));
        let state = url
            .split("state%3D")
            .nth(1)
            .expect("state in callback")
            .to_string();
        let (connected_user, name) = hub
            .complete_lastfm_authorization(&state, "approved-token")
            .await
            .expect("session");
        assert_eq!(
            (connected_user, name.as_str()),
            (user_id, "parson-listener")
        );
        hub.connect_listenbrainz(user_id, LB_TOKEN)
            .await
            .expect("listenbrainz");
    }

    fn enqueue(pool: &DbPool, user_id: i32, service: Service, listen: &Listen) {
        let mut connection = pool.get().unwrap();
        assert!(
            store::enqueue(&mut connection, user_id, service, None, listen, now_unix()).unwrap()
        );
    }

    fn event(kind: &str, source: &str, position: f64) -> PlaybackEventRequest {
        PlaybackEventRequest {
            event_key: format!("s1:song-lanterns:{kind}"),
            song_id: "song-lanterns".into(),
            event_type: kind.into(),
            session_id: Some("s1".into()),
            queue_id: None,
            source: Some(source.into()),
            position_seconds: Some(position),
            duration_seconds: Some(214.0),
            ..Default::default()
        }
    }

    #[actix_web::test]
    async fn playback_events_send_now_playing_and_queue_listens_from_the_start_time() {
        let pool = pool();
        let transport = fixtures();
        let hub = hub_with(pool.clone(), transport.clone());
        connect_both(&hub, 1).await;
        let cache = crate::scrobbling::import::tests::library();
        let started = now_unix() - 200;
        {
            let mut connection = pool.get().unwrap();
            diesel::sql_query(
                "INSERT INTO playback_event (event_key, user_id, song_id, event_type, session_id, created_at)
                 VALUES ('start', 1, 'song-lanterns', 'play_started', 's1', datetime(?, 'unixepoch'))",
            )
            .bind::<diesel::sql_types::BigInt, _>(started)
            .execute(&mut connection)
            .unwrap();
        }

        let sent = hub
            .observe(1, event("play_started", "manual", 0.0), cache.clone())
            .await
            .unwrap();
        assert_eq!(sent, 2);
        let requests = transport.requests();
        let now_playing = requests
            .iter()
            .find(|request| request.form_value("method") == Some("track.updateNowPlaying"))
            .expect("Last.fm now playing");
        assert_eq!(now_playing.form_value("track"), Some("Lanterns"));
        assert_eq!(now_playing.form_value("album"), Some("Glass Orchard"));
        assert_eq!(now_playing.form_value("trackNumber"), Some("3"));
        let playing_now = requests
            .iter()
            .find(|request| request.url.ends_with("/submit-listens"))
            .expect("ListenBrainz playing_now");
        assert_eq!(
            playing_now.json.as_ref().unwrap()["listen_type"],
            "playing_now"
        );

        // Early events never scrobble; the qualified play does, timestamped
        // when playback started.
        assert_eq!(
            hub.observe(1, event("early_skip", "manual", 5.0), cache.clone())
                .await
                .unwrap(),
            0
        );
        let queued = hub
            .observe(1, event("qualified_play", "manual", 120.0), cache.clone())
            .await
            .unwrap();
        assert_eq!(queued, 2);
        let status = hub.status(1).await.unwrap();
        assert_eq!(status.recent.len(), 2);
        assert!(
            status
                .recent
                .iter()
                .all(|entry| entry.listened_at == started)
        );
        // The same qualified play reported twice is queued once.
        assert_eq!(
            hub.observe(1, event("qualified_play", "manual", 121.0), cache.clone())
                .await
                .unwrap(),
            0
        );

        // Radio picks follow each connection's "include Radio" setting.
        hub.update_settings(
            1,
            Service::Listenbrainz,
            store::SettingsPatch {
                scrobble_radio: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        {
            let mut connection = pool.get().unwrap();
            diesel::sql_query("DELETE FROM scrobble_queue")
                .execute(&mut connection)
                .unwrap();
            diesel::sql_query("DELETE FROM playback_event")
                .execute(&mut connection)
                .unwrap();
        }
        let queued = hub
            .observe(
                1,
                event("qualified_play", "generated", 150.0),
                cache.clone(),
            )
            .await
            .unwrap();
        assert_eq!(queued, 1);
        let status = hub.status(1).await.unwrap();
        assert_eq!(status.recent[0].service, "lastfm");
        // Without a recorded start, the time is estimated from the position.
        assert!((status.recent[0].listened_at - (now_unix() - 150)).abs() <= 2);
        hub.process_due().await.unwrap();
        let scrobble = transport
            .requests()
            .into_iter()
            .rev()
            .find(|request| request.form_value("method") == Some("track.scrobble"))
            .unwrap();
        assert_eq!(scrobble.form_value("chosenByUser[0]"), Some("0"));

        // A profile without connections sends nothing.
        assert_eq!(
            hub.observe(2, event("qualified_play", "manual", 120.0), cache)
                .await
                .unwrap(),
            0
        );
    }

    #[actix_web::test]
    async fn connections_are_per_profile_and_secrets_are_sealed() {
        let pool = pool();
        let hub = hub_with(pool.clone(), fixtures());
        connect_both(&hub, 1).await;

        let status = hub.status(1).await.expect("status");
        assert!(
            status
                .services
                .iter()
                .all(|service| service.connection.is_some())
        );
        assert_eq!(
            status.services[0].connection.as_ref().unwrap().profile_url,
            "https://www.last.fm/user/parson-listener"
        );
        let other = hub.status(2).await.expect("other profile");
        assert!(
            other
                .services
                .iter()
                .all(|service| service.connection.is_none())
        );

        // The stored session key and token are not readable in the database
        // and do not open for another profile.
        let mut connection = pool.get().unwrap();
        let rows = store::connections(&mut connection, 1).unwrap();
        for row in &rows {
            let raw = String::from_utf8_lossy(&row.sealed_secret);
            assert!(!raw.contains("recorded-session-key") && !raw.contains(LB_TOKEN));
            let mut moved = row.clone();
            moved.user_id = 2;
            assert!(hub.open_secret(&moved).is_none());
            assert!(hub.open_secret(row).is_some());
        }
    }

    #[actix_web::test]
    async fn an_unknown_or_reused_authorization_state_is_refused() {
        let hub = hub_with(pool(), fixtures());
        assert!(matches!(
            hub.complete_lastfm_authorization("nope", "token").await,
            Err(HubError::Expired)
        ));
        let (url, _) = hub
            .start_lastfm_authorization(1, "https://music.example.com/")
            .await
            .unwrap();
        assert!(url.contains("music%2Eexample%2Ecom%2Fconnect%2Flastfm%3Fstate%3D"));
        let state = url.split("state%3D").nth(1).unwrap().to_string();
        hub.complete_lastfm_authorization(&state, "approved-token")
            .await
            .unwrap();
        assert!(matches!(
            hub.complete_lastfm_authorization(&state, "approved-token")
                .await,
            Err(HubError::Expired)
        ));
        assert!(matches!(
            hub.start_lastfm_authorization(1, "javascript:alert(1)")
                .await,
            Err(HubError::Invalid(_))
        ));
    }

    #[actix_web::test]
    async fn queued_listens_are_delivered_in_batches_and_logged() {
        let pool = pool();
        let transport = fixtures();
        let hub = hub_with(pool.clone(), transport.clone());
        connect_both(&hub, 1).await;
        enqueue(
            &pool,
            1,
            Service::Lastfm,
            &listen("Lanterns", 1_700_000_000),
        );
        enqueue(
            &pool,
            1,
            Service::Lastfm,
            &listen("Copper Sky", 1_700_000_300),
        );
        enqueue(
            &pool,
            1,
            Service::Listenbrainz,
            &listen("Lanterns", 1_700_000_000),
        );

        assert_eq!(hub.process_due().await.expect("delivered"), 3);
        let status = hub.status(1).await.unwrap();
        assert_eq!(status.services[0].queue.sent, 2);
        assert_eq!(status.services[1].queue.sent, 1);
        assert_eq!(status.services[0].queue.pending, 0);
        assert!(
            status.services[0]
                .connection
                .as_ref()
                .unwrap()
                .last_scrobbled_at
                .is_some()
        );
        assert_eq!(status.recent.len(), 3);

        let requests = transport.requests();
        let scrobble = requests
            .iter()
            .find(|request| request.form_value("method") == Some("track.scrobble"))
            .expect("one batched scrobble");
        assert_eq!(scrobble.form_value("track[0]"), Some("Lanterns"));
        assert_eq!(scrobble.form_value("track[1]"), Some("Copper Sky"));
        let submission = requests
            .iter()
            .find(|request| request.url.ends_with("/1/submit-listens"))
            .expect("listenbrainz submission");
        assert_eq!(submission.json.as_ref().unwrap()["listen_type"], "single");
        // Nothing is sent twice.
        assert_eq!(hub.process_due().await.unwrap(), 0);
    }

    #[actix_web::test]
    async fn offline_listens_wait_with_backoff_and_survive_until_the_service_returns() {
        let pool = pool();
        let transport = fixtures();
        let hub = hub_with(pool.clone(), transport.clone());
        connect_both(&hub, 1).await;
        transport.prepend(FixtureRoute {
            method: Method::Post,
            url: "https://ws.audioscrobbler.com/2.0/".into(),
            query: Vec::new(),
            form: vec![("method".into(), "track.scrobble".into())],
            replies: [
                FixtureReply {
                    status: 503,
                    body: serde_json::json!({"error": 16, "message": "There was a temporary error processing your request. Please try again"}),
                },
                FixtureReply {
                    status: 200,
                    body: serde_json::json!({"scrobbles": {"scrobble": {"ignoredMessage": {"code": "0", "#text": ""}}, "@attr": {"accepted": 1, "ignored": 0}}}),
                },
            ]
            .into(),
        });
        enqueue(
            &pool,
            1,
            Service::Lastfm,
            &listen("Lanterns", 1_700_000_000),
        );

        assert_eq!(hub.process_due().await.unwrap(), 0);
        let status = hub.status(1).await.unwrap();
        let lastfm = &status.services[0];
        assert_eq!(lastfm.queue.pending, 1);
        assert!(lastfm.queue.next_attempt_at.unwrap() >= now_unix() + 25);
        assert!(lastfm.connection.as_ref().unwrap().last_error.is_some());
        // Not due yet, so nothing is sent.
        assert_eq!(hub.process_due().await.unwrap(), 0);

        // "Retry now" makes it due; the service is back.
        hub.retry(1).await.unwrap();
        assert_eq!(hub.process_due().await.unwrap(), 1);
        let status = hub.status(1).await.unwrap();
        assert_eq!(status.services[0].queue.sent, 1);
        assert!(
            status.services[0]
                .connection
                .as_ref()
                .unwrap()
                .last_error
                .is_none()
        );
        assert_eq!(status.recent[0].attempts, 2);
    }

    #[actix_web::test]
    async fn a_revoked_session_pauses_the_queue_until_the_profile_reconnects() {
        let pool = pool();
        let transport = fixtures();
        let hub = hub_with(pool.clone(), transport.clone());
        connect_both(&hub, 1).await;
        transport.prepend(FixtureRoute {
            method: Method::Post,
            url: "https://api.listenbrainz.org/1/submit-listens".into(),
            query: Vec::new(),
            form: Vec::new(),
            replies: [FixtureReply {
                status: 401,
                body: serde_json::json!({"code": 401, "error": "Invalid authorization token."}),
            }]
            .into(),
        });
        enqueue(
            &pool,
            1,
            Service::Listenbrainz,
            &listen("Lanterns", 1_700_000_000),
        );
        assert_eq!(hub.process_due().await.unwrap(), 0);
        let status = hub.status(1).await.unwrap();
        let listenbrainz = &status.services[1];
        assert_eq!(
            listenbrainz.connection.as_ref().unwrap().status,
            "reauth_required"
        );
        assert_eq!(listenbrainz.queue.pending, 1);
        // Paused: no further attempts while the token is revoked.
        let before = transport.requests().len();
        assert_eq!(hub.process_due().await.unwrap(), 0);
        assert_eq!(transport.requests().len(), before);
    }

    #[actix_web::test]
    async fn a_rejected_batch_is_retried_one_by_one() {
        let pool = pool();
        let transport = fixtures();
        let hub = hub_with(pool.clone(), transport.clone());
        connect_both(&hub, 1).await;
        transport.prepend(FixtureRoute {
            method: Method::Post,
            url: "https://api.listenbrainz.org/1/submit-listens".into(),
            query: Vec::new(),
            form: Vec::new(),
            replies: [
                FixtureReply {
                    status: 400,
                    body: serde_json::json!({"code": 400, "error": "JSON document may only contain listened_at, track_metadata, ..."}),
                },
                FixtureReply {
                    status: 200,
                    body: serde_json::json!({"status": "ok"}),
                },
                FixtureReply {
                    status: 400,
                    body: serde_json::json!({"code": 400, "error": "Value for key track_name is too long."}),
                },
                FixtureReply {
                    status: 200,
                    body: serde_json::json!({"status": "ok"}),
                },
            ]
            .into(),
        });
        enqueue(
            &pool,
            1,
            Service::Listenbrainz,
            &listen("Lanterns", 1_700_000_000),
        );
        enqueue(
            &pool,
            1,
            Service::Listenbrainz,
            &listen("Copper Sky", 1_700_000_300),
        );
        enqueue(
            &pool,
            1,
            Service::Listenbrainz,
            &listen("Fieldnotes", 1_700_000_600),
        );
        assert_eq!(hub.process_due().await.unwrap(), 2);
        let status = hub.status(1).await.unwrap();
        assert_eq!(status.services[1].queue.sent, 2);
        assert_eq!(status.services[1].queue.failed, 1);
        let failed = status
            .recent
            .iter()
            .find(|entry| entry.status == "failed")
            .unwrap();
        assert_eq!(failed.track, "Copper Sky");
    }

    #[actix_web::test]
    async fn last_fm_ignored_and_rate_limited_scrobbles_are_recorded() {
        let pool = pool();
        let transport = fixtures();
        let hub = hub_with(pool.clone(), transport.clone());
        connect_both(&hub, 1).await;
        transport.prepend(FixtureRoute {
            method: Method::Post,
            url: "https://ws.audioscrobbler.com/2.0/".into(),
            query: Vec::new(),
            form: vec![("method".into(), "track.scrobble".into())],
            replies: [FixtureReply {
                status: 200,
                body: serde_json::json!({"scrobbles": {"scrobble": [
                    {"ignoredMessage": {"code": "3", "#text": "Timestamp too old"}},
                    {"ignoredMessage": {"code": "5", "#text": "Daily scrobble limit exceeded"}}
                ], "@attr": {"accepted": 0, "ignored": 2}}}),
            }]
            .into(),
        });
        enqueue(
            &pool,
            1,
            Service::Lastfm,
            &listen("Lanterns", 1_600_000_000),
        );
        enqueue(
            &pool,
            1,
            Service::Lastfm,
            &listen("Copper Sky", 1_700_000_300),
        );
        assert_eq!(hub.process_due().await.unwrap(), 0);
        let status = hub.status(1).await.unwrap();
        assert_eq!(status.services[0].queue.ignored, 1);
        assert_eq!(status.services[0].queue.pending, 1);
        assert!(status.services[0].queue.next_attempt_at.unwrap() >= now_unix() + 3000);
    }

    #[actix_web::test]
    async fn disconnecting_drops_waiting_listens_and_disabling_pauses_them() {
        let pool = pool();
        let hub = hub_with(pool.clone(), fixtures());
        connect_both(&hub, 1).await;
        enqueue(
            &pool,
            1,
            Service::Lastfm,
            &listen("Lanterns", 1_700_000_000),
        );
        hub.update_settings(
            1,
            Service::Lastfm,
            store::SettingsPatch {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(hub.process_due().await.unwrap(), 0);
        assert_eq!(hub.status(1).await.unwrap().services[0].queue.pending, 1);
        hub.disconnect(1, Service::Lastfm).await.unwrap();
        let status = hub.status(1).await.unwrap();
        assert!(status.services[0].connection.is_none());
        assert_eq!(status.services[0].queue.pending, 0);
        assert!(matches!(
            hub.disconnect(1, Service::Lastfm).await,
            Err(HubError::NotConnected)
        ));
    }

    #[actix_web::test]
    async fn server_credentials_are_validated_sealed_and_preferred_from_settings() {
        let pool = pool();
        let transport = fixtures();
        let mut hub = ScrobbleHub::new(
            pool.clone(),
            TokenVault::new([9; 32]),
            transport.clone(),
            false,
        );
        assert!(!hub.lastfm_setup().await.unwrap().configured);
        assert!(matches!(
            hub.start_lastfm_authorization(1, "http://localhost:1993")
                .await,
            Err(HubError::NotConfigured)
        ));
        assert!(matches!(
            hub.set_lastfm_credentials(1, "short", "short").await,
            Err(HubError::Invalid(_))
        ));
        let setup = hub
            .set_lastfm_credentials(
                1,
                "0123456789ABCDEF0123456789abcdef",
                "fedcba9876543210fedcba9876543210",
            )
            .await
            .expect("saved");
        assert_eq!(setup.source, Some("settings"));
        assert_eq!(setup.key_hint.as_deref(), Some("0123…cdef"));
        let token_request = transport.requests().pop().unwrap();
        assert_eq!(token_request.query_value("method"), Some("auth.getToken"));
        let mut connection = pool.get().unwrap();
        let sealed = store::server_config(&mut connection, Service::Lastfm)
            .unwrap()
            .unwrap();
        assert!(!String::from_utf8_lossy(&sealed).contains("fedcba98"));
        drop(connection);

        // Environment credentials win over stored ones.
        hub.env_lastfm = Some(super::super::sandbox::credentials());
        assert_eq!(
            hub.lastfm_setup().await.unwrap().source,
            Some("environment")
        );
        hub.env_lastfm = None;
        assert!(!hub.clear_lastfm_credentials().await.unwrap().configured);

        // A key Last.fm rejects is not stored.
        transport.prepend(FixtureRoute {
            method: Method::Get,
            url: "https://ws.audioscrobbler.com/2.0/".into(),
            query: vec![("method".into(), "auth.getToken".into())],
            form: Vec::new(),
            replies: [FixtureReply {
                status: 403,
                body: serde_json::json!({"message": "Invalid API key - You must be granted a valid key by last.fm", "error": 10}),
            }]
            .into(),
        });
        assert!(matches!(
            hub.set_lastfm_credentials(
                1,
                "0123456789abcdef0123456789abcdef",
                "fedcba9876543210fedcba9876543210"
            )
            .await,
            Err(HubError::Invalid(_))
        ));
        assert!(!hub.lastfm_setup().await.unwrap().configured);
    }

    #[test]
    fn callback_bases_accept_only_plain_server_origins() {
        assert_eq!(
            callback_base("http://192.168.1.20:1993/").as_deref(),
            Some("http://192.168.1.20:1993")
        );
        assert_eq!(
            callback_base("https://music.example.com/parson").as_deref(),
            Some("https://music.example.com/parson")
        );
        assert!(callback_base("ftp://example.com").is_none());
        assert!(callback_base("https://user:pass@example.com").is_none());
        assert!(callback_base("https://example.com/?next=evil").is_none());
        assert!(callback_base("not a url").is_none());
    }

    #[actix_web::test]
    async fn prune_keeps_the_log_bounded() {
        let pool = pool();
        let mut connection = pool.get().unwrap();
        let now = now_unix();
        for index in 0..(store::MAX_LOG_ROWS + 5) {
            store::enqueue(
                &mut connection,
                1,
                Service::Lastfm,
                None,
                &listen(&format!("Track {index}"), now - 1000 + index),
                now,
            )
            .unwrap();
        }
        diesel::sql_query("UPDATE scrobble_queue SET status = 'sent', sent_at = ?")
            .bind::<diesel::sql_types::BigInt, _>(now)
            .execute(&mut connection)
            .unwrap();
        store::enqueue(
            &mut connection,
            1,
            Service::Lastfm,
            None,
            &listen("Old", 1),
            now,
        )
        .unwrap();
        diesel::sql_query(
            "UPDATE scrobble_queue SET status = 'sent', sent_at = 1 WHERE track = 'Old'",
        )
        .execute(&mut connection)
        .unwrap();
        let removed = store::prune_log(&mut connection, now).unwrap();
        assert_eq!(removed, 6);
    }
}
