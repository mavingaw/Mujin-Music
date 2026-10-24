//! Per-user orchestration: sealed credentials, sign-in flows, token refresh,
//! library add/remove/sync and stream resolution for every provider.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use diesel::sqlite::SqliteConnection;
use serde::Serialize;

use super::artwork::ArtworkMode;
use super::qobuz::QobuzClient;
use super::store::{
    self, AccountRow, AccountStatus, ItemOrigin, LibraryCounts, LibraryItemRow, LibraryQuery,
    NewLibraryItem,
};
use super::tidal::{DeviceAuthorization, DevicePoll, TidalClient};
use super::transport::{ReqwestTransport, Transport};
use super::vault::{SealPurpose, TokenVault};
use super::{
    AudioFormat, BrowsePage, BrowseSection, ClientCredentials, ItemKind, ProviderAlbum,
    ProviderArtist, ProviderContext, ProviderError, ProviderId, ProviderSession, ProviderTrack,
    QualityOption, ResolvedStream, SearchResults, StreamQuality, StreamSource, StreamingProvider,
    sandbox, unix_now,
};
use crate::persistence::connection::DbPool;

/// Marker stored instead of credentials when the listener uses the
/// credentials the server owner configured through environment variables.
const SERVER_CREDENTIALS_MARKER: &str = "@server";
const REFRESH_MARGIN_SECONDS: i64 = 120;
const MAX_SYNC_ITEMS: u32 = 2_000;
const SYNC_PAGE: u32 = 100;
const MAX_CREDENTIAL_CHARACTERS: usize = 256;

#[derive(Debug)]
pub enum HubError {
    Provider(ProviderError),
    Database(String),
    Invalid(&'static str),
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

/// A page of a listener's streaming library.
pub struct LibraryRequest {
    pub provider: Option<ProviderId>,
    pub kind: ItemKind,
    pub text: Option<String>,
    pub sort: store::LibrarySort,
    pub offset: i64,
    pub limit: i64,
}

pub struct StreamingHub {
    pool: DbPool,
    vault: TokenVault,
    tidal: TidalClient,
    qobuz: QobuzClient,
    sandbox: bool,
    server_credentials: HashMap<ProviderId, ClientCredentials>,
}

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
        Ok(Self::new(
            pool,
            TokenVault::new(key),
            transport,
            sandbox,
            server_credentials_from_env(),
        ))
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
        Self {
            pool,
            vault,
            tidal: TidalClient::new(transport.clone(), artwork),
            qobuz: QobuzClient::new(transport, artwork),
            sandbox,
            server_credentials,
        }
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
                .server_credentials
                .get(&provider)
                .cloned()
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
                CredentialSummary {
                    configured: self.server_credentials.contains_key(&provider),
                    client_id_hint: None,
                    has_secret: self
                        .server_credentials
                        .get(&provider)
                        .is_some_and(|credentials| credentials.client_secret.is_some()),
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
            last_library_sync: row.and_then(|row| row.last_library_sync),
            credentials: self.summarize(user_id, provider, row),
            server_credentials_available: self.server_credentials.contains_key(&provider),
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
            if !self.server_credentials.contains_key(&provider) {
                return Err(HubError::Invalid(
                    "This server has no credentials configured for that service.",
                ));
            }
            ClientCredentials {
                client_id: SERVER_CREDENTIALS_MARKER.to_string(),
                client_secret: None,
            }
        } else {
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
            ClientCredentials {
                client_id,
                client_secret,
            }
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
        let row = self
            .row(user_id, ProviderId::Tidal)
            .await?
            .ok_or(HubError::Provider(ProviderError::NotConnected))?;
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
        let row = self
            .row(user_id, ProviderId::Qobuz)
            .await?
            .ok_or(HubError::Provider(ProviderError::NotConnected))?;
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

    /// "Add to library": snapshots the album or track into the listener's
    /// library and favorites it on the service, like the service's own apps.
    pub async fn add_to_library(
        &self,
        user_id: i32,
        provider: ProviderId,
        kind: ItemKind,
        raw_id: String,
    ) -> HubResult<LibraryChange> {
        let lookup_id = raw_id.clone();
        let (item, _) = self
            .call(user_id, provider, move |client, context| {
                let raw_id = lookup_id.clone();
                Box::pin(async move {
                    Ok(match kind {
                        ItemKind::Album => {
                            NewLibraryItem::album(&client.album(context, &raw_id).await?)
                        }
                        ItemKind::Track => {
                            NewLibraryItem::track(&client.track(context, &raw_id).await?)
                        }
                    })
                })
            })
            .await?;
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
                let (page, _) = self
                    .call(user_id, provider, move |client, context| {
                        Box::pin(
                            async move { client.browse(context, section, offset, SYNC_PAGE).await },
                        )
                    })
                    .await?;
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
        HubError::Invalid(message) => (*message).to_string(),
    }
}
