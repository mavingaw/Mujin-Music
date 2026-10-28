//! The Better versions service: requests to FETCH and their tracking, the
//! import and verification of what FETCH delivers, and the enhance queue.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use actix_web::web;
use diesel::sqlite::SqliteConnection;
use serde::Serialize;
use tokio::sync::Notify;

use super::candidates::{Candidate, find_candidates};
use super::enhancer::{
    EnhancerClient, EnhancerConfig, EnhancerError, EnhancerInfo, WorkerJobState,
};
use super::fetch::{
    FetchClient, FetchConfig, FetchError, FetchJob, JobStatus, OpenApiReport, SearchOutcome,
};
use super::matching::{MatchOutcome, match_album};
use super::query::{AlbumTags, build_query, search_terms};
use super::realtime::{HttpChunkEnhancer, RealtimeBackend, RealtimeStats};
use super::settings::{self, SecretName, UpgradeSettings};
use super::store::{self, EnhanceState, JobRow, NewRequest, RequestRow, RequestState};
use crate::domain::{Album, Artist};
use crate::library::state::{LibraryCache, LibraryLifecycle};
use crate::library::tools::layers::{album_facts, song_quality};
use crate::library::tools::marks::{self, FileMarks};
use crate::library::tools::quality::QualityTier;
use crate::library::tools::spectral::{self, SpectralResult, SpectralVerdict};
use crate::library::tools::store as tool_store;
use crate::library::tools::versions::{VersionChoice, group_versions};
use crate::persistence::connection::DbPool;
use crate::providers::vault::TokenVault;

/// First poll after a request, and the fastest polling while it moves.
const POLL_BASE_SECONDS: i64 = 5;
const POLL_MAX_SECONDS: i64 = 300;
/// A job another app owns is checked rarely.
const HANDED_OFF_POLL_SECONDS: i64 = 900;
const ERROR_POLL_MAX_SECONDS: i64 = 900;
/// Most files checked for a fake copy per import.
const MAX_VERIFIED_FILES: usize = 12;
/// Automatic requests are spaced at least this far apart.
const AUTO_SPACING_SECONDS: i64 = 600;
const MAX_SECRET_CHARS: usize = 4096;

#[derive(Debug)]
pub enum ServiceError {
    NotConfigured(&'static str),
    Invalid(String),
    Fetch(FetchError),
    Enhancer(EnhancerError),
    Storage(String),
    NotFound,
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfigured(what) => f.write_str(what),
            Self::Invalid(message) => f.write_str(message),
            Self::Fetch(error) => write!(f, "{error}"),
            Self::Enhancer(error) => write!(f, "{error}"),
            Self::Storage(_) => f.write_str("The Better versions database could not be read."),
            Self::NotFound => f.write_str("Nothing like that exists."),
        }
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/* ------------------------------------------------------------------ */
/* Pure decisions                                                       */
/* ------------------------------------------------------------------ */

/// What a request's jobs add up to.
#[derive(Clone, Debug, PartialEq)]
pub struct JobsSummary {
    pub state: RequestState,
    pub message: String,
    /// Mean progress of the jobs, 0 to 100.
    pub progress: Option<f64>,
    /// The final folder of the finished album, as FETCH names it.
    pub dest: Option<String>,
}

pub fn summarize_jobs(jobs: &[FetchJob]) -> JobsSummary {
    if jobs.is_empty() {
        return JobsSummary {
            state: RequestState::NotFound,
            message: "FETCH did not find anything to download.".into(),
            progress: None,
            dest: None,
        };
    }
    let progress = {
        let values: Vec<f64> = jobs.iter().filter_map(|job| job.progress).collect();
        (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
    };
    let statuses: Vec<JobStatus> = jobs.iter().map(FetchJob::status).collect();
    if let Some(done) = jobs.iter().find(|job| job.is_final())
        && statuses.iter().all(|status| status.is_terminal())
    {
        return JobsSummary {
            state: RequestState::Importing,
            message: "FETCH finished; adding the new copy to the library.".into(),
            progress: Some(100.0),
            dest: Some(done.dest.clone()),
        };
    }
    if statuses.iter().all(|status| status.is_terminal()) {
        if statuses.contains(&JobStatus::Done) {
            // Done but still in FETCH's staging area: it is moving files.
            return JobsSummary {
                state: RequestState::Downloading,
                message: "FETCH is moving the files into place.".into(),
                progress: Some(100.0),
                dest: None,
            };
        }
        if statuses
            .iter()
            .all(|status| *status == JobStatus::Cancelled)
        {
            return JobsSummary {
                state: RequestState::Cancelled,
                message: "The download was cancelled.".into(),
                progress,
                dest: None,
            };
        }
        let error = jobs
            .iter()
            .map(|job| job.error.trim())
            .find(|error| !error.is_empty())
            .unwrap_or("FETCH could not get a lossless copy.");
        return JobsSummary {
            state: RequestState::Failed,
            message: error.to_string(),
            progress,
            dest: None,
        };
    }
    if let Some(handed) = jobs.iter().find(|job| job.status() == JobStatus::HandedOff)
        && !statuses
            .iter()
            .any(|status| matches!(status, JobStatus::Running | JobStatus::Starting))
    {
        let engine = if handed.engine.trim().is_empty() {
            "another app".to_string()
        } else {
            handed.engine.trim().to_string()
        };
        return JobsSummary {
            state: RequestState::HandedOff,
            message: format!("Handed to {engine}"),
            progress,
            dest: None,
        };
    }
    let running = statuses
        .iter()
        .any(|status| matches!(status, JobStatus::Running | JobStatus::Starting))
        || progress.is_some_and(|progress| progress > 0.0);
    let unknown: Vec<&str> = statuses
        .iter()
        .filter_map(|status| match status {
            JobStatus::Other(value) => Some(value.as_str()),
            _ => None,
        })
        .collect();
    JobsSummary {
        state: if running {
            RequestState::Downloading
        } else {
            RequestState::Requested
        },
        message: if unknown.is_empty() {
            String::new()
        } else {
            format!("FETCH says: {}", unknown.join(", "))
        },
        progress,
        dest: None,
    }
}

/// Seconds until the next poll: soon while things move, backing off to
/// five minutes when nothing changes, and slower on errors or while
/// another app owns the job.
pub fn next_poll_delay(state: RequestState, quiet_polls: i32, failures: i32) -> i64 {
    if failures > 0 {
        return (30_i64 << failures.clamp(0, 5)).min(ERROR_POLL_MAX_SECONDS);
    }
    let backoff = (POLL_BASE_SECONDS << quiet_polls.clamp(0, 6)).min(POLL_MAX_SECONDS);
    match state {
        RequestState::HandedOff => backoff.clamp(60, HANDED_OFF_POLL_SECONDS),
        _ => backoff,
    }
}

/// `Music/Rise Against/Endgame` (FETCH's view) becomes
/// `<library root>/Rise Against/Endgame`: whatever follows a `Music`
/// component, else the last two components.
pub fn map_dest(dest: &str, library_root: &Path) -> Option<PathBuf> {
    let parts: Vec<&str> = dest
        .split(['/', '\\'])
        .filter(|part| !part.is_empty() && *part != "." && *part != "..")
        .collect();
    if parts.is_empty() {
        return None;
    }
    let after_music = parts
        .iter()
        .rposition(|part| part.eq_ignore_ascii_case("music"))
        .map(|index| &parts[index + 1..])
        .filter(|rest| !rest.is_empty());
    let tail = match after_music {
        Some(rest) => rest,
        None => &parts[parts.len().saturating_sub(2)..],
    };
    let mut path = library_root.to_path_buf();
    for part in tail {
        path.push(part);
    }
    Some(path)
}

#[derive(Clone, Debug, PartialEq)]
pub enum ImportDecision {
    /// Verified (or not contradicted): make it the preferred version.
    Upgrade {
        verified: usize,
        inconclusive: usize,
    },
    /// Most checked files are transcodes of lossy copies.
    Fake {
        cutoff_hz: Option<u32>,
        checked: usize,
        fakes: usize,
    },
    NoMatch,
    /// Matched, but nothing lossless arrived.
    NotLossless,
}

/// Decides from the match and the spectral checks of the new files.
pub fn decide_import(
    outcome: Option<&MatchOutcome>,
    new_lossless: usize,
    checks: &[SpectralResult],
) -> ImportDecision {
    if outcome.is_none() {
        return ImportDecision::NoMatch;
    }
    if new_lossless == 0 {
        return ImportDecision::NotLossless;
    }
    let fakes: Vec<&SpectralResult> = checks
        .iter()
        .filter(|check| check.is_confident_transcode())
        .collect();
    if !checks.is_empty() && fakes.len() * 2 >= checks.len() && !fakes.is_empty() {
        let mut cutoffs: Vec<u32> = fakes.iter().filter_map(|check| check.cutoff_hz).collect();
        cutoffs.sort_unstable();
        return ImportDecision::Fake {
            cutoff_hz: cutoffs.get(cutoffs.len() / 2).copied(),
            checked: checks.len(),
            fakes: fakes.len(),
        };
    }
    ImportDecision::Upgrade {
        verified: checks
            .iter()
            .filter(|check| check.verdict == SpectralVerdict::Genuine)
            .count(),
        inconclusive: checks
            .iter()
            .filter(|check| check.verdict == SpectralVerdict::Inconclusive)
            .count(),
    }
}

/// `<root>/<Artist>/<Album>/track.mp3` keeps its place under `replaced`:
/// `<replaced>/<Artist>/<Album>/track.mp3`, never over an existing file.
pub fn replaced_target(path: &Path, roots: &[String], replaced: &Path) -> PathBuf {
    let relative = roots
        .iter()
        .filter_map(|root| path.strip_prefix(root).ok())
        .min_by_key(|relative| relative.components().count())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(path.file_name().unwrap_or_default()));
    unique_path(&replaced.join(relative))
}

/// `path`, or `name (2).ext`, `name (3).ext`... when it exists.
pub fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("file")
        .to_string();
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| format!(".{extension}"))
        .unwrap_or_default();
    for index in 2..10_000 {
        let candidate = path.with_file_name(format!("{stem} ({index}){extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    path.with_file_name(format!("{stem} ({}){extension}", uuid::Uuid::new_v4()))
}

/// Where the enhanced copy of `source` goes: its place under its library
/// root, inside `output`, as FLAC; never over an existing file.
pub fn enhanced_target(source: &Path, roots: &[String], output: &Path) -> PathBuf {
    let relative = roots
        .iter()
        .filter_map(|root| source.strip_prefix(root).ok())
        .min_by_key(|relative| relative.components().count())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(source.file_name().unwrap_or_default()));
    let mut target = output.join(relative);
    let stem = target
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("track")
        .to_string();
    target.set_file_name(format!("{stem} (AI-enhanced).flac"));
    unique_path(&target)
}

/* ------------------------------------------------------------------ */
/* Views                                                                */
/* ------------------------------------------------------------------ */

#[derive(Clone, Debug, Serialize)]
pub struct RequestView {
    pub id: i64,
    pub album_id: String,
    pub album_title: String,
    pub artist_name: String,
    pub state: RequestState,
    pub message: String,
    pub origin: String,
    pub reason: String,
    pub created_at: String,
    pub updated_at: String,
    /// Mean progress of FETCH's jobs, 0 to 100.
    pub progress: Option<f64>,
    pub speed: String,
    pub eta: String,
    /// The app that took the job over, when handed off.
    pub engine: String,
    pub jobs: Vec<JobRow>,
    pub result_album_id: Option<String>,
}

/// The FETCH query itself is shown to admins only.
#[derive(Clone, Debug, Serialize)]
pub struct AdminRequestView {
    #[serde(flatten)]
    pub request: RequestView,
    pub query: String,
}

fn request_view(row: RequestRow, jobs: Vec<JobRow>) -> RequestView {
    let progress = {
        let values: Vec<f64> = jobs.iter().filter_map(|job| job.progress).collect();
        (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
    };
    let running = jobs
        .iter()
        .find(|job| matches!(JobStatus::parse(&job.status), JobStatus::Running));
    let handed = jobs
        .iter()
        .find(|job| JobStatus::parse(&job.status) == JobStatus::HandedOff);
    RequestView {
        state: row.state(),
        id: row.id,
        album_id: row.album_id,
        album_title: row.album_title,
        artist_name: row.artist_name,
        message: row.message,
        origin: row.origin,
        reason: row.reason,
        created_at: row.created_at,
        updated_at: row.updated_at,
        progress: if row.state == "upgraded" {
            Some(100.0)
        } else {
            progress
        },
        speed: running.map(|job| job.speed.clone()).unwrap_or_default(),
        eta: running.map(|job| job.eta.clone()).unwrap_or_default(),
        engine: handed
            .or(running)
            .or(jobs.first())
            .map(|job| job.engine.clone())
            .unwrap_or_default(),
        result_album_id: row.result_album_id,
        jobs,
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CandidateView {
    #[serde(flatten)]
    pub candidate: Candidate,
    pub request: Option<RequestView>,
    /// Tracks with an AI-enhanced copy.
    pub enhanced_tracks: usize,
    /// Tracks waiting in the enhance queue.
    pub enhance_queued: usize,
    /// FETCH's search, when it was looked up.
    pub search: Option<SearchOutcome>,
}

#[derive(Clone, Debug, Serialize)]
pub struct FetchCheck {
    pub ok: bool,
    pub health: bool,
    pub authorized: bool,
    pub me: Option<String>,
    pub openapi: Option<OpenApiReport>,
    pub free_space: Option<String>,
    pub error: Option<String>,
    pub checked_at: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct EnhancerCheck {
    pub ok: bool,
    pub info: Option<EnhancerInfo>,
    /// Real time is allowed (RTF under 0.5).
    pub realtime: bool,
    pub error: Option<String>,
    pub checked_at: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SettingsView {
    pub settings: UpgradeSettings,
    /// Which secrets are saved; never their values.
    pub saved: Vec<SecretName>,
    pub library_roots: Vec<String>,
    /// Where enhanced files go, after the default is applied.
    pub enhanced_folder: Option<String>,
    pub fetch_check: Option<FetchCheck>,
    pub enhancer_check: Option<EnhancerCheck>,
}

/* ------------------------------------------------------------------ */
/* The service                                                          */
/* ------------------------------------------------------------------ */

pub struct UpgradeService {
    pool: DbPool,
    vault: TokenVault,
    library: OnceLock<web::Data<LibraryLifecycle>>,
    wake: Notify,
    import_lock: tokio::sync::Mutex<()>,
    fetch_check: Mutex<Option<FetchCheck>>,
    enhancer_check: Mutex<Option<EnhancerCheck>>,
    search_cache: Mutex<HashMap<String, SearchOutcome>>,
    realtime_stats: Arc<RealtimeStats>,
    /// Library roots, for tests that run without a library.
    roots_override: Mutex<Option<Vec<String>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl UpgradeService {
    pub fn from_env(pool: DbPool) -> Result<Self, String> {
        let key = crate::settings::streaming_token_key()
            .map_err(|error| format!("Better versions key: {error}"))?;
        Ok(Self::new(pool, TokenVault::new(key)))
    }

    pub fn new(pool: DbPool, vault: TokenVault) -> Self {
        Self {
            pool,
            vault,
            library: OnceLock::new(),
            wake: Notify::new(),
            import_lock: tokio::sync::Mutex::new(()),
            fetch_check: Mutex::new(None),
            enhancer_check: Mutex::new(None),
            search_cache: Mutex::new(HashMap::new()),
            realtime_stats: Arc::new(RealtimeStats::default()),
            roots_override: Mutex::new(None),
        }
    }

    pub fn set_library(&self, library: web::Data<LibraryLifecycle>) {
        let _ = self.library.set(library);
    }

    pub fn realtime_stats(&self) -> Arc<RealtimeStats> {
        self.realtime_stats.clone()
    }

    pub fn wake(&self) {
        self.wake.notify_one();
    }

    pub(crate) async fn db<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut SqliteConnection) -> diesel::QueryResult<T> + Send + 'static,
    ) -> Result<T, ServiceError> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let mut connection = pool.get().map_err(|error| error.to_string())?;
            work(&mut connection).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| ServiceError::Storage(error.to_string()))?
        .map_err(ServiceError::Storage)
    }

    pub async fn settings(&self) -> Result<UpgradeSettings, ServiceError> {
        self.db(settings::load).await
    }

    async fn library_roots(&self) -> Vec<String> {
        if let Some(roots) = lock(&self.roots_override).clone() {
            return roots;
        }
        crate::api::library::read_library_paths().await
    }

    #[cfg(test)]
    pub(crate) fn set_roots_for_test(&self, roots: Vec<String>) {
        *lock(&self.roots_override) = Some(roots);
    }

    /// The folder enhanced files go to (the default is a "Mujin Enhanced"
    /// folder in the first library root).
    pub async fn enhanced_folder(&self, settings: &UpgradeSettings) -> Option<PathBuf> {
        if let Some(folder) = &settings.enhancer.output_folder {
            return Some(PathBuf::from(folder));
        }
        self.library_roots()
            .await
            .first()
            .map(|root| Path::new(root).join("Mujin Enhanced"))
    }

    pub async fn settings_view(&self) -> Result<SettingsView, ServiceError> {
        let (settings, saved) = self
            .db(|connection| {
                Ok((
                    settings::load(connection)?,
                    settings::saved_secrets(connection)?,
                ))
            })
            .await?;
        let enhanced_folder = self
            .enhanced_folder(&settings)
            .await
            .map(|path| path.to_string_lossy().into_owned());
        Ok(SettingsView {
            settings,
            saved,
            library_roots: self.library_roots().await,
            enhanced_folder,
            fetch_check: lock(&self.fetch_check).clone(),
            enhancer_check: lock(&self.enhancer_check).clone(),
        })
    }

    /// Saves settings; a change to the AI preference rebuilds the catalog.
    pub async fn save_settings(
        &self,
        next: UpgradeSettings,
    ) -> Result<UpgradeSettings, ServiceError> {
        let next = next
            .validated()
            .map_err(|message| ServiceError::Invalid(message.into()))?;
        let roots = self.library_roots().await;
        if let Some(root) = &next.fetch.library_root
            && !roots.iter().any(|known| known == root)
        {
            return Err(ServiceError::Invalid(
                "Choose one of Mujin Music's library folders.".into(),
            ));
        }
        let previous = self.settings().await?;
        let saved = next.clone();
        self.db(move |connection| settings::save(connection, &saved))
            .await?;
        if previous.enhancer.prefer_enhanced != next.enhancer.prefer_enhanced {
            self.rebuild_catalog().await;
        }
        if previous.enhancer != next.enhancer {
            self.refresh_realtime().await;
        }
        self.wake();
        Ok(next)
    }

    pub async fn set_secret(
        &self,
        name: SecretName,
        value: &str,
        admin_id: i32,
    ) -> Result<(), ServiceError> {
        let value = value.trim();
        if value.is_empty()
            || value.chars().count() > MAX_SECRET_CHARS
            || value.contains(['\n', '\r'])
        {
            return Err(ServiceError::Invalid(
                "Paste the token on one line, as FETCH or the enhancer shows it.".into(),
            ));
        }
        let sealed = self
            .vault
            .seal_in_context(&name.context(), &value.to_string())
            .map_err(|error| ServiceError::Storage(error.to_string()))?;
        self.db(move |connection| {
            settings::set_sealed_secret(connection, name, &sealed, Some(admin_id))
        })
        .await?;
        if name == SecretName::EnhancerToken {
            self.refresh_realtime().await;
        }
        Ok(())
    }

    pub async fn clear_secret(&self, name: SecretName) -> Result<(), ServiceError> {
        self.db(move |connection| settings::delete_secret(connection, name))
            .await?;
        if name == SecretName::EnhancerToken {
            super::realtime::set_backend(None);
        }
        Ok(())
    }

    async fn secret(&self, name: SecretName) -> Result<Option<String>, ServiceError> {
        let sealed = self
            .db(move |connection| settings::sealed_secret(connection, name))
            .await?;
        Ok(sealed.and_then(|sealed| {
            match self
                .vault
                .open_in_context::<String>(&name.context(), &sealed)
            {
                Ok(value) => Some(value),
                Err(_) => {
                    tracing::warn!(
                        secret = name.as_str(),
                        "a saved Better versions secret could not be opened; save it again"
                    );
                    None
                }
            }
        }))
    }

    async fn fetch_client(&self, settings: &UpgradeSettings) -> Result<FetchClient, ServiceError> {
        if !settings.fetch.enabled {
            return Err(ServiceError::NotConfigured(
                "FETCH is off. Turn it on in Settings → Better versions.",
            ));
        }
        let token = self.secret(SecretName::FetchToken).await?;
        if token.is_none() {
            return Err(ServiceError::NotConfigured(
                "Save FETCH's agent token for Mujin Music first.",
            ));
        }
        let access = match (
            self.secret(SecretName::CfAccessClientId).await?,
            self.secret(SecretName::CfAccessClientSecret).await?,
        ) {
            (Some(id), Some(secret)) => Some((id, secret)),
            _ => None,
        };
        FetchClient::new(FetchConfig {
            base_url: settings.fetch.base_url.clone(),
            token,
            access,
        })
        .map_err(ServiceError::Fetch)
    }

    async fn enhancer_client(
        &self,
        settings: &UpgradeSettings,
    ) -> Result<(EnhancerClient, EnhancerConfig), ServiceError> {
        if !settings.enhancer.enabled {
            return Err(ServiceError::NotConfigured(
                "The enhancer is off. Turn it on in Settings → Better versions.",
            ));
        }
        let token = self.secret(SecretName::EnhancerToken).await?;
        if token.is_none() {
            return Err(ServiceError::NotConfigured(
                "Paste the enhancer's shared secret first.",
            ));
        }
        let config = EnhancerConfig {
            base_url: settings.enhancer.base_url.clone(),
            token,
        };
        Ok((
            EnhancerClient::new(config.clone()).map_err(ServiceError::Enhancer)?,
            config,
        ))
    }

    /// "Check connection": health, the token, and FETCH's OpenAPI paths.
    pub async fn check_fetch(&self) -> Result<FetchCheck, ServiceError> {
        let settings = self.settings().await?;
        let mut settings_on = settings.clone();
        settings_on.fetch.enabled = true;
        let client = self.fetch_client(&settings_on).await;
        let check = match client {
            Err(error) => FetchCheck {
                ok: false,
                health: false,
                authorized: false,
                me: None,
                openapi: None,
                free_space: None,
                error: Some(error.to_string()),
                checked_at: now(),
            },
            Ok(client) => {
                let health = client.health().await;
                let me = client.me().await;
                let openapi = client.openapi().await.ok();
                let free_space = match &me {
                    Ok(_) => client.jobs(1).await.ok().and_then(|list| list.free_space),
                    Err(_) => None,
                };
                let error = match (&health, &me) {
                    (Err(error), _) | (_, Err(error)) => Some(error.to_string()),
                    _ => openapi
                        .as_ref()
                        .filter(|report| !report.missing.is_empty())
                        .map(|report| {
                            format!(
                                "Connected, but FETCH's API description lacks {}. Update FETCH, or requests may fail.",
                                report.missing.join(", ")
                            )
                        }),
                };
                FetchCheck {
                    ok: health.is_ok() && me.is_ok(),
                    health: health.is_ok(),
                    authorized: me.is_ok(),
                    me: me.ok().and_then(|value| {
                        ["name", "app", "agent", "user", "username"]
                            .iter()
                            .find_map(|key| value.get(*key).and_then(|v| v.as_str()))
                            .map(str::to_string)
                    }),
                    openapi,
                    free_space,
                    error,
                    checked_at: now(),
                }
            }
        };
        *lock(&self.fetch_check) = Some(check.clone());
        Ok(check)
    }

    pub async fn check_enhancer(&self) -> Result<EnhancerCheck, ServiceError> {
        let settings = self.settings().await?;
        let mut settings_on = settings.clone();
        settings_on.enhancer.enabled = true;
        let check = match self.enhancer_client(&settings_on).await {
            Err(error) => EnhancerCheck {
                ok: false,
                info: None,
                realtime: false,
                error: Some(error.to_string()),
                checked_at: now(),
            },
            Ok((client, _)) => match client.info().await {
                Ok(info) => EnhancerCheck {
                    ok: true,
                    realtime: info.realtime_capable(),
                    error: (info.device == "cpu").then(|| {
                        "The enhancer runs on the CPU: enhancing works, but slowly, and real time is not possible.".to_string()
                    }),
                    info: Some(info),
                    checked_at: now(),
                },
                Err(error) => EnhancerCheck {
                    ok: false,
                    info: None,
                    realtime: false,
                    error: Some(error.to_string()),
                    checked_at: now(),
                },
            },
        };
        *lock(&self.enhancer_check) = Some(check.clone());
        if settings.enhancer.enabled {
            self.install_realtime(&settings, check.info.as_ref()).await;
        }
        Ok(check)
    }

    async fn install_realtime(&self, settings: &UpgradeSettings, info: Option<&EnhancerInfo>) {
        let backend = match (info, self.enhancer_client(settings).await) {
            (Some(info), Ok((_, config))) if info.realtime_capable() => Some(RealtimeBackend {
                enhancer: Arc::new(HttpChunkEnhancer { config }),
                model_label: info.model_label(),
                realtime_factor: info.realtime_factor.unwrap_or(1.0),
                sample_rate: info.sample_rate,
                stats: self.realtime_stats.clone(),
            }),
            _ => None,
        };
        super::realtime::set_backend(backend);
    }

    /// Re-reads the worker's speed and allows real time only under RTF 0.5.
    pub async fn refresh_realtime(&self) {
        let Ok(settings) = self.settings().await else {
            return;
        };
        if !settings.enhancer.enabled {
            super::realtime::set_backend(None);
            return;
        }
        let _ = self.check_enhancer().await;
    }

    /* -------------------------------------------------------------- */
    /* Candidates and requests                                         */
    /* -------------------------------------------------------------- */

    async fn cache(&self) -> Result<Arc<LibraryCache>, ServiceError> {
        let library = self
            .library
            .get()
            .ok_or(ServiceError::NotConfigured("The library is not ready yet."))?;
        library
            .cache()
            .await
            .map_err(|_| ServiceError::NotConfigured("The library is not ready yet."))
    }

    pub async fn candidates(&self) -> Result<Vec<CandidateView>, ServiceError> {
        let settings = self.settings().await?;
        let cache = self.cache().await?;
        let marks = marks::current();
        let candidates = find_candidates(&cache.artists, &marks, settings.fetch.include_cd_quality);
        let (requests, queued) = self
            .db(|connection| {
                let mut latest: HashMap<String, RequestView> = HashMap::new();
                for row in store::requests(connection, 2_000)? {
                    if latest.contains_key(&row.album_id) {
                        continue;
                    }
                    let jobs = store::jobs_for(connection, row.id)?;
                    latest.insert(row.album_id.clone(), request_view(row, jobs));
                }
                let mut queued: HashMap<String, usize> = HashMap::new();
                for job in store::enhance_jobs(connection, 5_000)? {
                    if matches!(job.state.as_str(), "queued" | "running") {
                        *queued.entry(job.album_id).or_default() += 1;
                    }
                }
                Ok((latest, queued))
            })
            .await?;
        let mut requests = requests;
        let searches = lock(&self.search_cache).clone();
        Ok(candidates
            .into_iter()
            .map(|candidate| {
                let album = cache.album(&candidate.album_id);
                let enhanced_tracks = album.map_or(0, |album| enhanced_track_count(album, &marks));
                CandidateView {
                    request: requests.remove(&candidate.album_id),
                    enhance_queued: queued.get(&candidate.album_id).copied().unwrap_or(0),
                    search: searches.get(&candidate.album_id).cloned(),
                    enhanced_tracks,
                    candidate,
                }
            })
            .collect())
    }

    /// Every request, newest first (with the FETCH query, for admins).
    pub async fn requests(&self, limit: i64) -> Result<Vec<AdminRequestView>, ServiceError> {
        self.db(move |connection| {
            store::requests(connection, limit)?
                .into_iter()
                .map(|row| {
                    let jobs = store::jobs_for(connection, row.id)?;
                    let query = row.query.clone();
                    Ok(AdminRequestView {
                        request: request_view(row, jobs),
                        query,
                    })
                })
                .collect()
        })
        .await
    }

    /// Status for one album, for everyone (no query, no paths).
    pub async fn album_status(&self, album_id: String) -> Result<AlbumStatus, ServiceError> {
        let lookup = album_id.clone();
        let (request, queued) = self
            .db(move |connection| {
                let request = match store::latest_for_album(connection, &lookup)? {
                    Some(row) => {
                        let jobs = store::jobs_for(connection, row.id)?;
                        Some(request_view(row, jobs))
                    }
                    None => None,
                };
                let queued = store::enhance_jobs(connection, 5_000)?
                    .into_iter()
                    .filter(|job| {
                        job.album_id == lookup && matches!(job.state.as_str(), "queued" | "running")
                    })
                    .count();
                Ok((request, queued))
            })
            .await?;
        let marks = marks::current();
        let song_ids: Vec<String> = match self.cache().await {
            Ok(cache) => cache
                .album(&album_id)
                .map(|album| album.songs.iter().map(|song| song.id.clone()).collect())
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        let track_count = song_ids.len();
        let enhanced_tracks = match self.cache().await {
            Ok(cache) => cache
                .album(&album_id)
                .map_or(0, |album| enhanced_track_count(album, &marks)),
            Err(_) => 0,
        };
        let enhanced_copies = if marks.by_path.is_empty() {
            0
        } else {
            let marks = marks.clone();
            self.db(move |connection| {
                let mut count = 0;
                for song_id in song_ids {
                    if tool_store::track_copies(connection, &song_id)?
                        .iter()
                        .any(|copy| marks.is_ai_enhanced(&copy.path))
                    {
                        count += 1;
                    }
                }
                Ok(count)
            })
            .await?
        };
        Ok(AlbumStatus {
            album_id,
            request,
            track_count,
            enhanced_tracks,
            enhanced_copies,
            enhance_queued: queued,
        })
    }

    /// Plays the originals of an album's AI-enhanced tracks (or goes back
    /// to the ranking). The enhanced copies stay; returns how many tracks.
    pub async fn use_originals(
        &self,
        album_id: &str,
        originals: bool,
    ) -> Result<usize, ServiceError> {
        let cache = self.cache().await?;
        let album = cache.album(album_id).ok_or(ServiceError::NotFound)?;
        let song_ids: Vec<String> = album.songs.iter().map(|song| song.id.clone()).collect();
        let marks = marks::current();
        let changed = self
            .db(move |connection| {
                let mut changed = 0;
                for song_id in song_ids {
                    let copies = tool_store::track_copies(connection, &song_id)?;
                    let enhanced: Vec<String> = copies
                        .iter()
                        .filter(|copy| marks.is_ai_enhanced(&copy.path))
                        .map(|copy| copy.path.clone())
                        .collect();
                    if enhanced.is_empty() {
                        continue;
                    }
                    let original = copies
                        .iter()
                        .find(|copy| !enhanced.contains(&copy.path))
                        .map(|copy| copy.path.clone());
                    match (originals, original) {
                        (true, Some(path)) => {
                            tool_store::set_file_preference(connection, &song_id, Some(&path))?;
                            changed += 1;
                        }
                        (false, _) => {
                            tool_store::set_file_preference(connection, &song_id, None)?;
                            changed += 1;
                        }
                        _ => {}
                    }
                }
                Ok(changed)
            })
            .await?;
        self.rebuild_catalog().await;
        Ok(changed)
    }

    /// Asks FETCH whether any source has the album, before requesting.
    pub async fn search(&self, album_id: &str) -> Result<SearchOutcome, ServiceError> {
        let settings = self.settings().await?;
        let client = self.fetch_client(&settings).await?;
        let cache = self.cache().await?;
        let tags = album_tags(&cache, album_id).ok_or(ServiceError::NotFound)?;
        let outcome = client
            .search_music(&search_terms(&tags))
            .await
            .map_err(ServiceError::Fetch)?;
        lock(&self.search_cache).insert(album_id.to_string(), outcome.clone());
        Ok(outcome)
    }

    /// Requests a lossless copy of each album from FETCH. Albums with an
    /// open request are skipped. Returns the requests made.
    pub async fn request_albums(
        &self,
        library: &[Artist],
        album_ids: &[String],
        requested_by: Option<i32>,
        automatic: bool,
    ) -> Result<Vec<i64>, ServiceError> {
        let settings = self.settings().await?;
        let client = self.fetch_client(&settings).await?;
        let marks = marks::current();
        let candidates: HashMap<String, Candidate> = find_candidates(library, &marks, true)
            .into_iter()
            .map(|candidate| (candidate.album_id.clone(), candidate))
            .collect();
        let mut made = Vec::new();
        for album_id in album_ids {
            let Some((artist, album)) = find_album(library, album_id) else {
                continue;
            };
            let lookup = album_id.clone();
            if self
                .db(move |connection| store::open_request_for_album(connection, &lookup))
                .await?
                .is_some()
            {
                continue;
            }
            let tags = tags_of(artist, album);
            let query = build_query(&tags);
            let reason = candidates
                .get(album_id)
                .and_then(|candidate| candidate.reasons.first())
                .map(|reason| reason.label.clone())
                .unwrap_or_default();
            let submitted = client.submit(&query).await;
            let (state, message, jobs) = match submitted {
                Ok(jobs) => {
                    let summary = summarize_jobs(&jobs);
                    (summary.state, summary.message, jobs)
                }
                Err(error) => {
                    // Nothing was created in FETCH: report and stop.
                    if made.is_empty() {
                        return Err(ServiceError::Fetch(error));
                    }
                    tracing::warn!(%error, "FETCH refused a request; the rest are skipped");
                    break;
                }
            };
            let album_title = album.name.clone();
            let artist_name = artist.name.clone();
            let album_key = album_id.clone();
            let id = self
                .db(move |connection| {
                    let id = store::insert_request(
                        connection,
                        &NewRequest {
                            album_id: &album_key,
                            album_title: &album_title,
                            artist_name: &artist_name,
                            query: &query,
                            reason: &reason,
                            automatic,
                            requested_by,
                        },
                        state,
                        &message,
                    )?;
                    for job in &jobs {
                        store::upsert_job(connection, id, job)?;
                    }
                    store::schedule_poll(connection, id, now() + POLL_BASE_SECONDS, 0, 0)?;
                    Ok(id)
                })
                .await?;
            made.push(id);
        }
        self.wake();
        Ok(made)
    }

    async fn request_jobs_action(
        &self,
        id: i64,
        action: &'static str,
    ) -> Result<RequestRow, ServiceError> {
        let settings = self.settings().await?;
        let client = self.fetch_client(&settings).await?;
        let (row, jobs) = self
            .db(move |connection| {
                let row = store::request(connection, id)?;
                let jobs = store::jobs_for(connection, id)?;
                Ok((row, jobs))
            })
            .await?;
        let row = row.ok_or(ServiceError::NotFound)?;
        for job in &jobs {
            let status = JobStatus::parse(&job.status);
            let result = match action {
                "cancel" if !status.is_terminal() => client.cancel(&job.job_id).await,
                "retry" if matches!(status, JobStatus::Failed | JobStatus::Cancelled) => {
                    client.retry(&job.job_id).await
                }
                "delete" => client.delete(&job.job_id).await,
                _ => Ok(()),
            };
            match result {
                Ok(()) | Err(FetchError::NotFound) => {}
                Err(error) => return Err(ServiceError::Fetch(error)),
            }
        }
        Ok(row)
    }

    pub async fn cancel_request(&self, id: i64) -> Result<(), ServiceError> {
        let row = self.request_jobs_action(id, "cancel").await?;
        if row.state().is_open() {
            self.db(move |connection| {
                store::set_request_state(connection, id, RequestState::Cancelled, "Cancelled.")
            })
            .await?;
        }
        Ok(())
    }

    pub async fn retry_request(&self, id: i64) -> Result<(), ServiceError> {
        let row = self.request_jobs_action(id, "retry").await?;
        if row.state().is_open() {
            return Ok(());
        }
        if row.state() == RequestState::NotFound {
            // Nothing to retry in FETCH: ask again.
            let cache = self.cache().await?;
            self.db(move |connection| store::delete_request(connection, id))
                .await?;
            self.request_albums(
                &cache.artists,
                std::slice::from_ref(&row.album_id),
                None,
                false,
            )
            .await?;
            return Ok(());
        }
        self.db(move |connection| {
            store::set_request_state(
                connection,
                id,
                RequestState::Requested,
                "Asked FETCH to try again.",
            )?;
            store::schedule_poll(connection, id, now() + POLL_BASE_SECONDS, 0, 0)
        })
        .await?;
        self.wake();
        Ok(())
    }

    /// Forgets a finished request (and its jobs in FETCH's list).
    pub async fn delete_request(&self, id: i64) -> Result<(), ServiceError> {
        let row = self
            .db(move |connection| store::request(connection, id))
            .await?
            .ok_or(ServiceError::NotFound)?;
        if row.state().is_open() {
            return Err(ServiceError::Invalid(
                "Cancel the request before removing it.".into(),
            ));
        }
        if self.settings().await?.fetch.enabled {
            let _ = self.request_jobs_action(id, "delete").await;
        }
        self.db(move |connection| store::delete_request(connection, id))
            .await?;
        Ok(())
    }

    /* -------------------------------------------------------------- */
    /* Polling                                                          */
    /* -------------------------------------------------------------- */

    /// Polls every request that is due. Returns the requests now ready
    /// to import.
    pub async fn poll_once(&self) -> Result<Vec<i64>, ServiceError> {
        let due = self
            .db(|connection| store::due_requests(connection, now()))
            .await?;
        let due: Vec<RequestRow> = due
            .into_iter()
            .filter(|row| row.state() != RequestState::Importing)
            .collect();
        if due.is_empty() {
            return Ok(Vec::new());
        }
        let settings = self.settings().await?;
        let client = match self.fetch_client(&settings).await {
            Ok(client) => client,
            Err(_) => return Ok(Vec::new()),
        };
        let listed = client.jobs(200).await;
        let mut ready = Vec::new();
        for row in due {
            let id = row.id;
            let stored = self
                .db(move |connection| store::jobs_for(connection, id))
                .await?;
            let mut jobs = Vec::new();
            let mut failed = None;
            for job in &stored {
                let found = match &listed {
                    Ok(list) => list.jobs.iter().find(|item| item.id == job.job_id).cloned(),
                    Err(_) => None,
                };
                let fresh = match found {
                    Some(found) => Ok(found),
                    None => client.job(&job.job_id).await,
                };
                match fresh {
                    Ok(fresh) => jobs.push(fresh),
                    Err(FetchError::NotFound) => {
                        // FETCH forgot it: treat as cancelled there.
                        let mut gone = stored_job(job);
                        gone.status = "cancelled".into();
                        gone.error = "FETCH no longer lists this job.".into();
                        jobs.push(gone);
                    }
                    Err(error) => {
                        failed = Some(error);
                        break;
                    }
                }
            }
            if let Some(error) = failed {
                let failures = row.poll_failures + 1;
                let delay = next_poll_delay(row.state(), row.quiet_polls, failures);
                let message = format!("{error} Trying again later.");
                let state = row.state();
                self.db(move |connection| {
                    store::set_request_state(connection, id, state, &message)?;
                    store::schedule_poll(connection, id, now() + delay, failures, 0)
                })
                .await?;
                continue;
            }
            let summary = summarize_jobs(&jobs);
            let changed = summary.state != row.state()
                || jobs.iter().zip(&stored).any(|(fresh, old)| {
                    fresh.status != old.status || fresh.progress != old.progress
                });
            let quiet = if changed { 0 } else { row.quiet_polls + 1 };
            let delay = next_poll_delay(summary.state, quiet, 0);
            let state = summary.state;
            let message = summary.message.clone();
            let folder = summary.dest.clone();
            self.db(move |connection| {
                for job in &jobs {
                    store::upsert_job(connection, id, job)?;
                }
                store::set_request_state(connection, id, state, &message)?;
                if let Some(folder) = &folder {
                    store::set_request_result(connection, id, None, Some(folder))?;
                }
                store::schedule_poll(connection, id, now() + delay, 0, quiet)
            })
            .await?;
            if state == RequestState::Importing {
                ready.push(id);
            }
        }
        Ok(ready)
    }

    /* -------------------------------------------------------------- */
    /* Import and verification                                         */
    /* -------------------------------------------------------------- */

    /// Rescans where FETCH saved the album, matches the new files to the
    /// album, checks they are really lossless, and makes them preferred.
    pub async fn import(self: &Arc<Self>, id: i64) -> Result<RequestState, ServiceError> {
        let _guard = self.import_lock.lock().await;
        let row = self
            .db(move |connection| store::request(connection, id))
            .await?
            .ok_or(ServiceError::NotFound)?;
        let settings = self.settings().await?;
        let library = self
            .library
            .get()
            .cloned()
            .ok_or(ServiceError::NotConfigured("The library is not ready yet."))?;
        let folder = match (&settings.fetch.library_root, row.result_folder.as_deref()) {
            // Already mapped by an earlier attempt.
            (Some(root), Some(dest))
                if Path::new(dest).is_absolute() && super::files::path_is_under(dest, root) =>
            {
                Some(PathBuf::from(dest))
            }
            (Some(root), Some(dest)) => map_dest(dest, Path::new(root)),
            _ => None,
        };
        // Remember the local folder before rescanning: the indexer then
        // treats it as a copy added to an album already in the library, so
        // the album and its tracks keep their ids.
        if let Some(local) = &folder {
            let local = local.to_string_lossy().into_owned();
            self.db(move |connection| {
                store::set_request_result(connection, id, None, Some(&local))
            })
            .await?;
        }
        // 1. Rescan the folder FETCH saves into (or the whole library).
        if let Err(message) = crate::api::library::refresh_root_for_tools(
            library.clone(),
            settings.fetch.library_root.clone(),
            "Adding the lossless copy FETCH downloaded.",
        )
        .await
        {
            // A scan was already running: try again soon.
            self.db(move |connection| {
                store::set_request_state(connection, id, RequestState::Importing, &message)?;
                store::schedule_poll(connection, id, now() + 60, 0, 0)
            })
            .await?;
            return Ok(RequestState::Importing);
        }
        let cache = library
            .cache()
            .await
            .map_err(|_| ServiceError::NotConfigured("The library is not ready yet."))?;
        // 2. The album's original copies (never the files FETCH just
        // saved, which may already be copies of the same tracks), and the
        // new files.
        let old_album = row.album_id.clone();
        let folder_text = folder
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned());
        let identity = cache
            .album(&old_album)
            .zip(cache.album_owner(&old_album))
            .map(|(album, artist)| (artist.name.clone(), album.name.clone()));
        let (old_facts, new_facts) = self
            .db(move |connection| {
                let copies = super::files::album_copy_facts(connection, &old_album)?;
                let under = |path: &str| {
                    folder_text
                        .as_deref()
                        .is_some_and(|folder| super::files::path_is_under(path, folder))
                };
                let old = super::files::original_copies(&copies, under);
                let old_paths: HashSet<String> = old.iter().map(|fact| fact.path.clone()).collect();
                let new = match &folder_text {
                    Some(folder) => super::files::facts_under(connection, folder)?,
                    None => match &identity {
                        Some((artist, title)) => {
                            super::files::facts_like_album(connection, artist, title)?
                        }
                        None => Vec::new(),
                    },
                };
                let new = new
                    .into_iter()
                    .filter(|fact| !old_paths.contains(&fact.path))
                    .collect::<Vec<_>>();
                Ok((old, new))
            })
            .await?;
        let outcome = match_album(&old_facts, &new_facts);
        // 3. Verify the new lossless files with the spectral check.
        let new_lossless: Vec<String> = outcome
            .as_ref()
            .map(|outcome| {
                outcome
                    .tracks
                    .iter()
                    .map(|track| track.new_path.clone())
                    .filter(|path| super::files::is_lossless_path(path))
                    .collect()
            })
            .unwrap_or_default();
        let mut checks = Vec::new();
        for path in new_lossless.iter().take(MAX_VERIFIED_FILES) {
            let file = Path::new(path);
            let Ok(result) = spectral::check_file(file, None).await else {
                continue;
            };
            let version = super::files::file_version(file).unwrap_or_default();
            let saved = result.clone();
            let key = path.clone();
            self.db(move |connection| store::save_spectral(connection, &key, &version, &saved))
                .await?;
            checks.push(result);
        }
        let decision = decide_import(outcome.as_ref(), new_lossless.len(), &checks);
        // 4. Apply.
        let (state, message) = match &decision {
            ImportDecision::NoMatch => (
                RequestState::NoMatch,
                match &folder {
                    Some(folder) => format!(
                        "FETCH saved files in {}, but they do not match this album's tracks. The original stays preferred.",
                        folder.display()
                    ),
                    None => "No new files matching this album's tracks turned up. Set \"FETCH saves music into this library folder\" so Mujin Music knows where to look.".into(),
                },
            ),
            ImportDecision::NotLossless => (
                RequestState::NoMatch,
                "FETCH's copy matched, but it is not lossless either. The original stays preferred.".into(),
            ),
            ImportDecision::Fake { cutoff_hz, checked, fakes } => (
                RequestState::FakeLossless,
                format!(
                    "{fakes} of {checked} checked files stop at {} like an MP3: FETCH's FLAC was made from a lossy copy. The original stays preferred.",
                    cutoff_hz.map_or_else(|| "a lossy cut-off".to_string(), spectral::format_khz)
                ),
            ),
            ImportDecision::Upgrade { verified, inconclusive } => {
                let outcome = outcome.clone().expect("an upgrade has a match");
                let old_album = row.album_id.clone();
                let all_versions = crate::library::storage::fetch_library_with_all_versions()
                    .await
                    .map_err(|error| ServiceError::Storage(error.to_string()))?;
                let outcome_for_db = outcome.clone();
                self.db(move |connection| {
                    apply_preference(connection, &all_versions, &old_album, &outcome_for_db)
                })
                .await?;
                let mut message = format!(
                    "Now plays the lossless copy ({} of {} tracks; {verified} checked genuine{}). The old copy stays as an alternate version.",
                    outcome.tracks.len(),
                    old_facts.len(),
                    if *inconclusive > 0 {
                        format!(", {inconclusive} too quiet to judge")
                    } else {
                        String::new()
                    }
                );
                if settings.fetch.move_replaced
                    && let Some(replaced) = &settings.fetch.replaced_folder
                {
                    let roots = self.library_roots().await;
                    let moved = move_replaced(&outcome, &roots, Path::new(replaced)).await;
                    if moved > 0 {
                        message.push_str(&format!(" {moved} replaced lossy file(s) moved to {replaced}."));
                    }
                }
                let album = outcome.album_id.clone();
                let folder_text = folder.as_ref().map(|path| path.to_string_lossy().into_owned());
                self.db(move |connection| {
                    store::set_request_result(connection, id, Some(&album), folder_text.as_deref())
                })
                .await?;
                (RequestState::Upgraded, message)
            }
        };
        let saved_message = message.clone();
        self.db(move |connection| store::set_request_state(connection, id, state, &saved_message))
            .await?;
        tracing::info!(
            request = id,
            state = state.as_str(),
            "Better versions import finished"
        );
        self.rebuild_catalog().await;
        Ok(state)
    }

    pub async fn rebuild_catalog(&self) {
        let Some(library) = self.library.get() else {
            return;
        };
        crate::library::storage::refresh_cache().await;
        match LibraryCache::new().await {
            Ok(cache) => library.set_ready_and_persist(cache).await,
            Err(error) => tracing::warn!(%error, "catalog rebuild after Better versions failed"),
        }
    }

    /* -------------------------------------------------------------- */
    /* Automatic requests                                               */
    /* -------------------------------------------------------------- */

    /// Requests one candidate when automatic requests are on and today's
    /// allowance is not used up.
    pub async fn auto_request_once(&self) -> Result<Option<i64>, ServiceError> {
        let settings = self.settings().await?;
        if !settings.fetch.enabled || !settings.fetch.auto_request {
            return Ok(None);
        }
        let day_ago = now() - 24 * 3600;
        let spacing = now() - AUTO_SPACING_SECONDS;
        let (today, recent, requested) = self
            .db(move |connection| {
                Ok((
                    store::automatic_since(connection, day_ago)?,
                    store::automatic_since(connection, spacing)?,
                    store::requested_album_ids(connection)?,
                ))
            })
            .await?;
        if today >= i64::from(settings.fetch.auto_per_day) || recent > 0 {
            return Ok(None);
        }
        let cache = self.cache().await?;
        let requested: HashSet<String> = requested.into_iter().collect();
        let next = find_candidates(
            &cache.artists,
            &marks::current(),
            settings.fetch.include_cd_quality,
        )
        .into_iter()
        .find(|candidate| !requested.contains(&candidate.album_id));
        let Some(next) = next else {
            return Ok(None);
        };
        Ok(self
            .request_albums(&cache.artists, &[next.album_id], None, true)
            .await?
            .into_iter()
            .next())
    }

    /* -------------------------------------------------------------- */
    /* Enhancing                                                        */
    /* -------------------------------------------------------------- */

    /// Queues the lossy tracks of albums (or single songs) to enhance.
    pub async fn queue_enhance(
        &self,
        album_ids: &[String],
        song_ids: &[String],
        batch: &str,
    ) -> Result<usize, ServiceError> {
        let settings = self.settings().await?;
        if !settings.enhancer.enabled {
            return Err(ServiceError::NotConfigured(
                "The enhancer is off. Turn it on in Settings → Better versions.",
            ));
        }
        let cache = self.cache().await?;
        let marks = marks::current();
        let wanted_albums: HashSet<&str> = album_ids.iter().map(String::as_str).collect();
        let wanted_songs: HashSet<&str> = song_ids.iter().map(String::as_str).collect();
        let mut tracks = Vec::new();
        for artist in cache.artists.iter() {
            for album in &artist.albums {
                let whole = wanted_albums.contains(album.id.as_str());
                for song in &album.songs {
                    if !whole && !wanted_songs.contains(song.id.as_str()) {
                        continue;
                    }
                    if enhanceable(song, &marks) {
                        tracks.push((
                            song.id.clone(),
                            album.id.clone(),
                            song.path.clone(),
                            song.name.clone(),
                        ));
                    }
                }
            }
        }
        // Skip tracks that already have an enhanced copy.
        let batch = batch.to_string();
        let added = self
            .db(move |connection| {
                let mut fresh = Vec::new();
                for track in tracks {
                    if store::enhanced_from(connection, &track.2)?.is_none() {
                        fresh.push(track);
                    }
                }
                store::queue_enhance(connection, &fresh, &batch)
            })
            .await?;
        self.wake();
        Ok(added)
    }

    /// "Enhance all lossy that FETCH couldn't find": candidates whose
    /// request found nothing, failed or did not match.
    pub async fn queue_enhance_unfound(&self) -> Result<usize, ServiceError> {
        let views = self.candidates().await?;
        let albums: Vec<String> = views
            .into_iter()
            .filter(|view| {
                let unfound_request = view.request.as_ref().is_some_and(|request| {
                    matches!(
                        request.state,
                        RequestState::NotFound
                            | RequestState::Failed
                            | RequestState::NoMatch
                            | RequestState::FakeLossless
                    )
                });
                let unfound_search = view
                    .search
                    .as_ref()
                    .is_some_and(|search| search.found == Some(false));
                unfound_request || unfound_search
            })
            .map(|view| view.candidate.album_id)
            .collect();
        if albums.is_empty() {
            return Ok(0);
        }
        self.queue_enhance(&albums, &[], "not-found").await
    }

    pub async fn cancel_enhance(&self) -> Result<usize, ServiceError> {
        self.db(store::cancel_enhance_queue).await
    }

    /// Works one job of the enhance queue. Returns whether there was one.
    pub async fn enhance_once(&self) -> Result<bool, ServiceError> {
        let settings = self.settings().await?;
        if !settings.enhancer.enabled {
            return Ok(false);
        }
        let Some(job) = self.db(store::next_enhance_job).await? else {
            return Ok(false);
        };
        let (client, _) = match self.enhancer_client(&settings).await {
            Ok(client) => client,
            Err(_) => return Ok(false),
        };
        let Some(output) = self.enhanced_folder(&settings).await else {
            let id = job.id;
            self.db(move |connection| {
                store::update_enhance_job(
                    connection,
                    id,
                    EnhanceState::Failed,
                    None,
                    0.0,
                    None,
                    "Choose where enhanced files are saved.",
                )
            })
            .await?;
            return Ok(true);
        };
        let id = job.id;
        let source = PathBuf::from(&job.source_path);
        if !source.is_file() {
            self.db(move |connection| {
                store::update_enhance_job(
                    connection,
                    id,
                    EnhanceState::Skipped,
                    None,
                    0.0,
                    None,
                    "The original file is gone.",
                )
            })
            .await?;
            return Ok(true);
        }
        let info = client.info().await.map_err(ServiceError::Enhancer);
        let info = match info {
            Ok(info) => info,
            Err(error) => {
                // The worker is off: leave the job queued and try later.
                tracing::debug!(%error, "the enhancer is not reachable; the queue waits");
                return Ok(false);
            }
        };
        let worker_id = match job.worker_job_id.clone() {
            Some(worker_id) => worker_id,
            None => {
                let submitted = if settings.enhancer.shared_paths {
                    match client.submit_path(&job.source_path).await {
                        // The worker cannot see this file at that path: send
                        // the bytes instead.
                        Err(EnhancerError::PathRefused | EnhancerError::NotFound) => {
                            client.submit_upload(&source).await
                        }
                        other => other,
                    }
                } else {
                    client.submit_upload(&source).await
                };
                match submitted {
                    Ok(worker_job) => {
                        let worker_id = worker_job.id.clone();
                        let saved = worker_id.clone();
                        self.db(move |connection| {
                            store::update_enhance_job(
                                connection,
                                id,
                                EnhanceState::Running,
                                Some(&saved),
                                0.0,
                                None,
                                "",
                            )
                        })
                        .await?;
                        worker_id
                    }
                    Err(error) => {
                        let message = error.to_string();
                        self.db(move |connection| {
                            store::update_enhance_job(
                                connection,
                                id,
                                EnhanceState::Failed,
                                None,
                                0.0,
                                None,
                                &message,
                            )
                        })
                        .await?;
                        return Ok(true);
                    }
                }
            }
        };
        // Poll the worker until it finishes (or the job is cancelled).
        let started = std::time::Instant::now();
        let finished = loop {
            let state = self
                .db(move |connection| store::enhance_job_state(connection, id))
                .await?;
            if state.as_deref() == Some("cancelled") {
                let _ = client.cancel(&worker_id).await;
                return Ok(true);
            }
            match client.job(&worker_id).await {
                Ok(worker_job) => match worker_job.state {
                    WorkerJobState::Done => break Ok(()),
                    WorkerJobState::Failed | WorkerJobState::Cancelled => {
                        break Err(if worker_job.message.is_empty() {
                            "The enhancer could not process this file.".to_string()
                        } else {
                            worker_job.message
                        });
                    }
                    _ => {
                        let progress = worker_job.progress;
                        self.db(move |connection| {
                            store::update_enhance_job(
                                connection,
                                id,
                                EnhanceState::Running,
                                None,
                                progress,
                                None,
                                "",
                            )
                        })
                        .await?;
                    }
                },
                Err(EnhancerError::NotFound) => {
                    break Err(
                        "The enhancer restarted and lost the job; it will be sent again.".into(),
                    );
                }
                Err(error) => {
                    // Keep the job; try again on the next round.
                    tracing::debug!(%error, "lost touch with the enhancer");
                    return Ok(false);
                }
            }
            if started.elapsed() > Duration::from_secs(4 * 3600) {
                break Err("The enhancer took too long.".to_string());
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        };
        if let Err(message) = finished {
            self.db(move |connection| {
                store::update_enhance_job(
                    connection,
                    id,
                    EnhanceState::Failed,
                    None,
                    0.0,
                    None,
                    &message,
                )
            })
            .await?;
            return Ok(true);
        }
        let roots = self.library_roots().await;
        let target = enhanced_target(&source, &roots, &output);
        match client.download(&worker_id, &target).await {
            Ok(_) => {
                let _ = client.cancel(&worker_id).await;
                let path = target.to_string_lossy().into_owned();
                let source_path = job.source_path.clone();
                let tag = info.tag.clone();
                let saved = path.clone();
                self.db(move |connection| {
                    store::record_enhanced(connection, &path, &source_path, &tag)?;
                    store::update_enhance_job(
                        connection,
                        id,
                        EnhanceState::Done,
                        None,
                        1.0,
                        Some(&saved),
                        "",
                    )
                })
                .await?;
                if !roots.iter().any(|root| output.starts_with(root)) {
                    let _ =
                        crate::api::library::ensure_library_root(&output.to_string_lossy()).await;
                }
                copy_folder_art(&source, &target).await;
            }
            Err(error) => {
                let message = error.to_string();
                self.db(move |connection| {
                    store::update_enhance_job(
                        connection,
                        id,
                        EnhanceState::Failed,
                        None,
                        0.0,
                        None,
                        &message,
                    )
                })
                .await?;
            }
        }
        Ok(true)
    }

    /// Rescans the root that holds the enhanced folder after a batch.
    async fn index_enhanced(&self) {
        let (Some(library), Ok(settings)) = (self.library.get().cloned(), self.settings().await)
        else {
            return;
        };
        let Some(output) = self.enhanced_folder(&settings).await else {
            return;
        };
        let roots = self.library_roots().await;
        let root = roots
            .iter()
            .filter(|root| output.starts_with(root.as_str()))
            .max_by_key(|root| root.len())
            .cloned()
            .or_else(|| Some(output.to_string_lossy().into_owned()));
        if let Err(error) =
            crate::api::library::refresh_root_for_tools(library, root, "Adding AI-enhanced copies.")
                .await
        {
            tracing::info!(%error, "enhanced files will be indexed by the next scan");
        }
        self.rebuild_catalog().await;
    }

    /* -------------------------------------------------------------- */
    /* Background                                                       */
    /* -------------------------------------------------------------- */

    /// Polls FETCH, imports finished albums, works the enhance queue and
    /// makes automatic requests, until the server stops.
    pub async fn run(self: Arc<Self>) {
        // Startup: allow real time if the worker is fast enough.
        self.refresh_realtime().await;
        let enhancer = {
            let service = self.clone();
            tokio::spawn(async move {
                let mut done_since_index = 0usize;
                loop {
                    match service.enhance_once().await {
                        Ok(true) => {
                            done_since_index += 1;
                            if done_since_index >= 12 {
                                service.index_enhanced().await;
                                done_since_index = 0;
                            }
                            continue;
                        }
                        Ok(false) => {
                            if done_since_index > 0 {
                                service.index_enhanced().await;
                                done_since_index = 0;
                            }
                        }
                        Err(error) => tracing::warn!(%error, "the enhance queue stopped for now"),
                    }
                    tokio::select! {
                        () = service.wake.notified() => {}
                        () = tokio::time::sleep(Duration::from_secs(30)) => {}
                    }
                }
            })
        };
        let mut ticks: u64 = 0;
        loop {
            match self.poll_once().await {
                Ok(ready) => {
                    for id in ready {
                        let service = self.clone();
                        tokio::spawn(async move {
                            if let Err(error) = service.import(id).await {
                                tracing::warn!(%error, request = id, "Better versions import failed");
                            }
                        });
                    }
                }
                Err(error) => tracing::debug!(%error, "Better versions poll failed"),
            }
            // Imports that waited for a running scan.
            if let Ok(waiting) = self
                .db(|connection| store::due_requests(connection, now()))
                .await
            {
                for row in waiting
                    .into_iter()
                    .filter(|row| row.state() == RequestState::Importing)
                {
                    let id = row.id;
                    let _ = self
                        .db(move |connection| {
                            store::schedule_poll(connection, id, now() + 600, 0, 0)
                        })
                        .await;
                    let service = self.clone();
                    tokio::spawn(async move {
                        let _ = service.import(id).await;
                    });
                }
            }
            if ticks.is_multiple_of(30)
                && let Err(error) = self.auto_request_once().await
            {
                tracing::debug!(%error, "automatic request skipped");
            }
            if ticks.is_multiple_of(150) && ticks > 0 {
                self.refresh_realtime().await;
            }
            ticks += 1;
            if enhancer.is_finished() {
                tracing::warn!("the enhance queue worker stopped");
            }
            tokio::select! {
                () = self.wake.notified() => {}
                () = tokio::time::sleep(Duration::from_secs(2)) => {}
            }
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct AlbumStatus {
    pub album_id: String,
    pub request: Option<RequestView>,
    /// Tracks on the album.
    pub track_count: usize,
    /// Tracks that play an AI-enhanced copy now.
    pub enhanced_tracks: usize,
    /// Tracks that have an AI-enhanced copy (playing or not).
    pub enhanced_copies: usize,
    pub enhance_queued: usize,
}

fn stored_job(row: &JobRow) -> FetchJob {
    FetchJob {
        id: row.job_id.clone(),
        created: String::new(),
        updated: String::new(),
        input: String::new(),
        engine: row.engine.clone(),
        status: row.status.clone(),
        progress: row.progress,
        speed: row.speed.clone(),
        eta: row.eta.clone(),
        title: row.title.clone(),
        dest: row.dest.clone(),
        error: row.error.clone(),
        note: row.note.clone(),
        size: row.size.clone(),
        plan: serde_json::Value::Null,
    }
}

fn find_album<'a>(library: &'a [Artist], album_id: &str) -> Option<(&'a Artist, &'a Album)> {
    library.iter().find_map(|artist| {
        artist
            .albums
            .iter()
            .find(|album| album.id == album_id)
            .map(|album| (artist, album))
    })
}

fn tags_of(artist: &Artist, album: &Album) -> AlbumTags {
    AlbumTags {
        artist: artist.name.clone(),
        title: album.name.clone(),
        year: album.first_release_date.clone(),
        release_mbid: Some(album.musicbrainz_id.clone()).filter(|id| !id.is_empty()),
    }
}

fn album_tags(cache: &LibraryCache, album_id: &str) -> Option<AlbumTags> {
    Some(tags_of(
        cache.album_owner(album_id)?,
        cache.album(album_id)?,
    ))
}

fn enhanced_track_count(album: &Album, marks: &FileMarks) -> usize {
    album
        .songs
        .iter()
        .filter(|song| marks.is_ai_enhanced(&song.path))
        .count()
}

/// Lossy (or fake lossless), and not itself an AI-enhanced copy.
fn enhanceable(song: &crate::domain::Song, marks: &FileMarks) -> bool {
    if marks.is_ai_enhanced(&song.path) {
        return false;
    }
    matches!(song_quality(song).tier, QualityTier::Lossy)
}

/// Makes the new copy the preferred one: the copy that plays for each
/// matched track, and the preferred version of the album's group. Nothing
/// is deleted; the old copy stays as an alternate version.
pub fn apply_preference(
    connection: &mut SqliteConnection,
    all_versions: &[Artist],
    old_album: &str,
    outcome: &MatchOutcome,
) -> diesel::QueryResult<()> {
    if outcome.album_id == old_album {
        // Copies of the same tracks: the new file plays.
        for track in &outcome.tracks {
            if track.new_track_id == track.old_track_id {
                tool_store::set_file_preference(
                    connection,
                    &track.old_track_id,
                    Some(&track.new_path),
                )?;
            }
        }
        return Ok(());
    }
    let members = tool_store::version_members(connection)?;
    let choices = tool_store::version_choices(connection)?;
    let release_groups = tool_store::release_groups(connection)?;
    let groups = group_versions(
        &album_facts(all_versions, &release_groups),
        &members,
        &choices,
    );
    let existing = groups.iter().find(|group| {
        group
            .versions
            .iter()
            .any(|version| version.album_id == old_album)
            && group
                .versions
                .iter()
                .any(|version| version.album_id == outcome.album_id)
    });
    let key = match existing {
        Some(group) => group.key.clone(),
        None => {
            let key = members
                .get(old_album)
                .cloned()
                .flatten()
                .unwrap_or_else(|| format!("manual-upgrade-{old_album}"));
            tool_store::set_version_member(connection, old_album, Some(&key))?;
            tool_store::set_version_member(connection, &outcome.album_id, Some(&key))?;
            key
        }
    };
    let hide_others = choices.get(&key).is_some_and(|choice| choice.hide_others);
    tool_store::set_version_choice(
        connection,
        &key,
        Some(&VersionChoice {
            preferred_album_id: outcome.album_id.clone(),
            hide_others,
        }),
    )
}

/// Copies the source folder's cover image next to an enhanced file, so
/// the album keeps its art. Never replaces an image that is there.
async fn copy_folder_art(source: &Path, target: &Path) {
    let (Some(from), Some(to)) = (source.parent(), target.parent()) else {
        return;
    };
    for name in [
        "cover.jpg",
        "folder.jpg",
        "front.jpg",
        "cover.png",
        "folder.png",
    ] {
        let image = from.join(name);
        let destination = to.join(name);
        if image.is_file() && !destination.exists() {
            let _ = tokio::fs::copy(&image, &destination).await;
            return;
        }
    }
}

/// Moves the replaced lossy files, never over an existing file and never
/// a lossless one. Returns how many moved.
async fn move_replaced(outcome: &MatchOutcome, roots: &[String], replaced: &Path) -> usize {
    let mut moved = 0;
    for track in &outcome.tracks {
        let source = Path::new(&track.old_path);
        if super::files::is_lossless_path(&track.old_path) || !source.is_file() {
            continue;
        }
        let target = replaced_target(source, roots, replaced);
        if let Some(parent) = target.parent()
            && tokio::fs::create_dir_all(parent).await.is_err()
        {
            continue;
        }
        let result = match tokio::fs::rename(source, &target).await {
            Ok(()) => Ok(()),
            Err(_) => {
                // Another drive: copy, check, then remove the original.
                match tokio::fs::copy(source, &target).await {
                    Ok(bytes)
                        if tokio::fs::metadata(source)
                            .await
                            .is_ok_and(|metadata| metadata.len() == bytes) =>
                    {
                        tokio::fs::remove_file(source).await
                    }
                    Ok(_) => {
                        let _ = tokio::fs::remove_file(&target).await;
                        Err(std::io::Error::other("copy incomplete"))
                    }
                    Err(error) => Err(error),
                }
            }
        };
        if result.is_ok() {
            moved += 1;
        }
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upgrade::fake::FakeFetch;

    fn job(id: &str, status: &str, progress: Option<f64>, dest: &str) -> FetchJob {
        let mut job = super::super::fetch::parse_job(&serde_json::json!({
            "id": id, "status": status, "dest": dest, "engine": "lidarr"
        }))
        .unwrap();
        if status != "done" {
            job.progress = progress;
        }
        job
    }

    #[test]
    fn job_statuses_add_up_to_a_request_state() {
        assert_eq!(summarize_jobs(&[]).state, RequestState::NotFound);
        assert_eq!(
            summarize_jobs(&[job("1", "queued", None, "")]).state,
            RequestState::Requested
        );
        let running = summarize_jobs(&[job("1", "running", Some(45.0), "")]);
        assert_eq!(running.state, RequestState::Downloading);
        assert_eq!(running.progress, Some(45.0));
        let handed = summarize_jobs(&[job("1", "handed_off", None, "")]);
        assert_eq!(handed.state, RequestState::HandedOff);
        assert_eq!(handed.message, "Handed to lidarr");
        let staging = summarize_jobs(&[job("1", "done", None, "/staging/x")]);
        assert_eq!(
            staging.state,
            RequestState::Downloading,
            "not final in staging"
        );
        let done = summarize_jobs(&[job("1", "done", None, "Music/Rise Against/Endgame")]);
        assert_eq!(done.state, RequestState::Importing);
        assert_eq!(done.dest.as_deref(), Some("Music/Rise Against/Endgame"));
        let mut failed = job("1", "failed", None, "");
        failed.error = "No FLAC source found".into();
        assert_eq!(
            summarize_jobs(&[failed.clone()]).message,
            "No FLAC source found"
        );
        assert_eq!(
            summarize_jobs(&[job("1", "cancelled", None, "")]).state,
            RequestState::Cancelled
        );
        let unknown = summarize_jobs(&[job("1", "waiting_for_disk", None, "")]);
        assert_eq!(unknown.message, "FETCH says: waiting_for_disk");
    }

    #[test]
    fn polling_backs_off_and_slows_on_errors_and_hand_offs() {
        assert_eq!(next_poll_delay(RequestState::Downloading, 0, 0), 5);
        assert_eq!(next_poll_delay(RequestState::Downloading, 2, 0), 20);
        assert_eq!(next_poll_delay(RequestState::Downloading, 20, 0), 300);
        assert_eq!(next_poll_delay(RequestState::HandedOff, 0, 0), 60);
        assert_eq!(next_poll_delay(RequestState::Downloading, 0, 1), 60);
        assert_eq!(next_poll_delay(RequestState::Downloading, 0, 9), 900);
    }

    #[test]
    fn fetch_destinations_map_into_the_library_folder() {
        let root = Path::new("/srv/music");
        assert_eq!(
            map_dest("Music/Rise Against/Endgame", root),
            Some(PathBuf::from("/srv/music/Rise Against/Endgame"))
        );
        assert_eq!(
            map_dest("\\\\192.168.1.58\\Music\\Rise Against\\Endgame\\", root),
            Some(PathBuf::from("/srv/music/Rise Against/Endgame"))
        );
        assert_eq!(
            map_dest("/data/library/Rise Against/Endgame", root),
            Some(PathBuf::from("/srv/music/Rise Against/Endgame"))
        );
        assert_eq!(
            map_dest("../../etc/passwd", root),
            Some(PathBuf::from("/srv/music/etc/passwd")),
            "never escapes the root"
        );
        assert_eq!(map_dest("", root), None);
    }

    fn check(verdict: SpectralVerdict, confidence: f32, cutoff: Option<u32>) -> SpectralResult {
        SpectralResult {
            verdict,
            cutoff_hz: cutoff,
            confidence,
            detail: String::new(),
        }
    }

    #[test]
    fn imports_upgrade_only_verified_matching_lossless_copies() {
        let outcome = MatchOutcome {
            album_id: "new".into(),
            by_release_id: false,
            tracks: Vec::new(),
            coverage: 1.0,
        };
        assert_eq!(decide_import(None, 3, &[]), ImportDecision::NoMatch);
        assert_eq!(
            decide_import(Some(&outcome), 0, &[]),
            ImportDecision::NotLossless
        );
        let genuine = [
            check(SpectralVerdict::Genuine, 0.9, None),
            check(SpectralVerdict::Inconclusive, 0.0, None),
        ];
        assert_eq!(
            decide_import(Some(&outcome), 2, &genuine),
            ImportDecision::Upgrade {
                verified: 1,
                inconclusive: 1
            }
        );
        let fakes = [
            check(SpectralVerdict::Transcode, 0.9, Some(16_000)),
            check(SpectralVerdict::Transcode, 0.8, Some(16_250)),
            check(SpectralVerdict::Genuine, 0.7, None),
        ];
        assert!(matches!(
            decide_import(Some(&outcome), 3, &fakes),
            ImportDecision::Fake {
                fakes: 2,
                checked: 3,
                ..
            }
        ));
        // A single unsure transcode does not condemn the copy.
        let unsure = [
            check(SpectralVerdict::Transcode, 0.4, Some(19_800)),
            check(SpectralVerdict::Genuine, 0.9, None),
        ];
        assert!(matches!(
            decide_import(Some(&outcome), 2, &unsure),
            ImportDecision::Upgrade { .. }
        ));
    }

    #[test]
    fn replaced_and_enhanced_files_never_overwrite() {
        let dir = std::env::temp_dir().join(format!("mujin-replaced-{}", uuid::Uuid::new_v4()));
        let root = dir.join("music");
        std::fs::create_dir_all(root.join("A/B")).unwrap();
        let source = root.join("A/B/01.mp3");
        std::fs::write(&source, b"mp3").unwrap();
        let roots = vec![root.to_string_lossy().into_owned()];
        let replaced = dir.join("Replaced");
        assert_eq!(
            replaced_target(&source, &roots, &replaced),
            replaced.join("A/B/01.mp3")
        );
        std::fs::create_dir_all(replaced.join("A/B")).unwrap();
        std::fs::write(replaced.join("A/B/01.mp3"), b"x").unwrap();
        assert_eq!(
            replaced_target(&source, &roots, &replaced),
            replaced.join("A/B/01 (2).mp3")
        );
        let output = root.join("Mujin Enhanced");
        let target = enhanced_target(&source, &roots, &output);
        assert_eq!(target, output.join("A/B/01 (AI-enhanced).flac"));
        assert_ne!(target, source, "never the original");
        std::fs::create_dir_all(output.join("A/B")).unwrap();
        std::fs::write(&target, b"flac").unwrap();
        assert_eq!(
            enhanced_target(&source, &roots, &output),
            output.join("A/B/01 (AI-enhanced) (2).flac")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn library() -> Vec<Artist> {
        use crate::domain::{AudioProperties, Song};
        vec![Artist {
            id: "ra".into(),
            name: "Rise Against".into(),
            albums: vec![Album {
                id: "endgame".into(),
                name: "Endgame".into(),
                first_release_date: "2011".into(),
                songs: vec![Song {
                    id: "s1".into(),
                    name: "Architects".into(),
                    path: "/m/1.mp3".into(),
                    track_number: 1,
                    duration: 200.0,
                    audio: Some(AudioProperties {
                        codec: Some("mp3".into()),
                        sample_rate: Some(44_100),
                        bitrate_kbps: Some(320),
                        lossless: Some(false),
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }]
    }

    async fn service(fetch_url: &str) -> UpgradeService {
        let pool = crate::persistence::connection::migrated_memory_pool();
        let service = UpgradeService::new(pool, TokenVault::new([5; 32]));
        service.set_roots_for_test(vec!["/m".into()]);
        let mut settings = UpgradeSettings::default();
        settings.fetch.enabled = true;
        settings.fetch.base_url = fetch_url.into();
        service.save_settings(settings).await.unwrap();
        service
    }

    #[actix_web::test]
    async fn requests_are_tracked_through_fetch_with_the_token_kept_sealed() {
        let fake = FakeFetch::start("agent-token").await;
        let service = service(&fake.url()).await;
        // No token yet: nothing is sent.
        assert!(matches!(
            service
                .request_albums(&library(), &["endgame".into()], Some(1), false)
                .await,
            Err(ServiceError::NotConfigured(_))
        ));
        service
            .set_secret(SecretName::FetchToken, "agent-token", 1)
            .await
            .unwrap();
        // The token is sealed at rest, never stored in plain text.
        let raw = service
            .db(|connection| settings::sealed_secret(connection, SecretName::FetchToken))
            .await
            .unwrap()
            .unwrap();
        assert!(!raw.windows(11).any(|window| window == b"agent-token"));
        let view = service.settings_view().await.unwrap();
        assert_eq!(view.saved, vec![SecretName::FetchToken]);
        assert!(
            !serde_json::to_string(&view)
                .unwrap()
                .contains("agent-token")
        );

        let check = service.check_fetch().await.unwrap();
        assert!(check.ok, "{check:?}");
        assert_eq!(
            check.openapi.unwrap().version.as_deref(),
            Some("9.9.9-fake")
        );

        let made = service
            .request_albums(&library(), &["endgame".into()], Some(1), false)
            .await
            .unwrap();
        assert_eq!(made.len(), 1);
        assert_eq!(
            fake.submitted(),
            vec!["the album Endgame by Rise Against (2011) as flac"]
        );
        // A second request for the same album is not sent while one is open.
        assert!(
            service
                .request_albums(&library(), &["endgame".into()], Some(1), false)
                .await
                .unwrap()
                .is_empty()
        );
        let id = made[0];
        async fn force_due(service: &UpgradeService, id: i64) -> Result<(), ServiceError> {
            service
                .db(move |connection| store::schedule_poll(connection, id, 0, 0, 0))
                .await
        }

        fake.set_job("job-1", "running", 45.0, "/staging/1");
        force_due(&service, id).await.unwrap();
        assert!(service.poll_once().await.unwrap().is_empty());
        let status = service.album_status("endgame".into()).await.unwrap();
        let request = status.request.unwrap();
        assert_eq!(request.state, RequestState::Downloading);
        assert_eq!(request.progress, Some(45.0));
        assert_eq!(request.speed, "3.2 MB/s");

        fake.set_job("job-1", "handed_off", 45.0, "");
        fake.set_job_field("job-1", "engine", serde_json::json!("Lidarr"));
        force_due(&service, id).await.unwrap();
        service.poll_once().await.unwrap();
        let request = service
            .album_status("endgame".into())
            .await
            .unwrap()
            .request
            .unwrap();
        assert_eq!(request.state, RequestState::HandedOff);
        assert_eq!(request.message, "Handed to Lidarr");

        // FETCH is down: the request keeps its state and backs off.
        fake.set_failing(Some(503));
        force_due(&service, id).await.unwrap();
        service.poll_once().await.unwrap();
        let row = service
            .db(move |connection| store::request(connection, id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.state(), RequestState::HandedOff);
        assert_eq!(row.poll_failures, 1);
        assert!(row.next_poll_at > now() + 30);
        fake.set_failing(None);

        fake.set_job("job-1", "done", 100.0, "Music/Rise Against/Endgame");
        force_due(&service, id).await.unwrap();
        assert_eq!(service.poll_once().await.unwrap(), vec![id]);
        let row = service
            .db(move |connection| store::request(connection, id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.state(), RequestState::Importing);
        assert_eq!(
            row.result_folder.as_deref(),
            Some("Music/Rise Against/Endgame")
        );
        // Terminal requests stop being polled.
        let polls = fake.polls();
        service
            .db(move |connection| {
                store::set_request_state(connection, id, RequestState::Upgraded, "")
            })
            .await
            .unwrap();
        force_due(&service, id).await.unwrap();
        service.poll_once().await.unwrap();
        assert_eq!(fake.polls(), polls);

        // Cancel and retry reach FETCH.
        let second = service
            .db(|connection| {
                store::insert_request(
                    connection,
                    &NewRequest {
                        album_id: "other",
                        album_title: "Other",
                        artist_name: "X",
                        query: "q",
                        reason: "",
                        automatic: false,
                        requested_by: None,
                    },
                    RequestState::Requested,
                    "",
                )
            })
            .await
            .unwrap();
        let jobs = fake_jobs(&service, &fake).await;
        service
            .db(move |connection| store::upsert_job(connection, second, &jobs))
            .await
            .unwrap();
        service.cancel_request(second).await.unwrap();
        assert_eq!(
            service
                .album_status("other".into())
                .await
                .unwrap()
                .request
                .unwrap()
                .state,
            RequestState::Cancelled
        );
        service.retry_request(second).await.unwrap();
        assert_eq!(
            service
                .album_status("other".into())
                .await
                .unwrap()
                .request
                .unwrap()
                .state,
            RequestState::Requested
        );
        fake.stop().await;
    }

    async fn fake_jobs(service: &UpgradeService, fake: &FakeFetch) -> FetchJob {
        let settings = service.settings().await.unwrap();
        let client = service.fetch_client(&settings).await.unwrap();
        let _ = fake;
        client
            .submit("the album Other by X as flac")
            .await
            .unwrap()
            .remove(0)
    }

    #[actix_web::test]
    async fn automatic_requests_are_off_by_default_and_rate_limited() {
        let fake = FakeFetch::start("agent-token").await;
        let service = service(&fake.url()).await;
        service
            .set_secret(SecretName::FetchToken, "agent-token", 1)
            .await
            .unwrap();
        // Off by default: nothing happens (and no library is needed).
        assert_eq!(service.auto_request_once().await.unwrap(), None);
        assert!(fake.submitted().is_empty());
        let mut settings = service.settings().await.unwrap();
        settings.fetch.auto_request = true;
        settings.fetch.auto_per_day = 1;
        service.save_settings(settings).await.unwrap();
        // One automatic request already today uses up the allowance.
        service
            .db(|connection| {
                store::insert_request(
                    connection,
                    &NewRequest {
                        album_id: "x",
                        album_title: "",
                        artist_name: "",
                        query: "q",
                        reason: "",
                        automatic: true,
                        requested_by: None,
                    },
                    RequestState::Upgraded,
                    "",
                )
            })
            .await
            .unwrap();
        assert_eq!(service.auto_request_once().await.unwrap(), None);
        assert!(fake.submitted().is_empty());
        fake.stop().await;
    }

    #[test]
    fn preferring_a_new_album_version_groups_and_chooses_it() {
        let pool = crate::persistence::connection::migrated_memory_pool();
        let mut connection = pool.get().unwrap();
        let mut library = library();
        let mut lossless = library[0].albums[0].clone();
        lossless.id = "endgame-flac".into();
        lossless.name = "Endgame (2011 Remaster)".into();
        lossless.songs[0].path = "/m/flac/1.flac".into();
        library[0].albums.push(lossless);
        let outcome = MatchOutcome {
            album_id: "endgame-flac".into(),
            by_release_id: false,
            tracks: Vec::new(),
            coverage: 1.0,
        };
        apply_preference(&mut connection, &library, "endgame", &outcome).unwrap();
        let choices = tool_store::version_choices(&mut connection).unwrap();
        assert_eq!(choices.len(), 1);
        let choice = choices.values().next().unwrap();
        assert_eq!(choice.preferred_album_id, "endgame-flac");
        assert!(
            !choice.hide_others,
            "the old copy stays visible as a version"
        );

        // Same album id: the copy that plays switches to the new file.
        let merged = MatchOutcome {
            album_id: "endgame".into(),
            by_release_id: false,
            tracks: vec![super::super::matching::MatchedTrack {
                old_track_id: "s1".into(),
                old_path: "/m/1.mp3".into(),
                new_track_id: "s1".into(),
                new_path: "/m/flac/1.flac".into(),
            }],
            coverage: 1.0,
        };
        apply_preference(&mut connection, &library, "endgame", &merged).unwrap();
        let preferences = tool_store::file_preferences(&mut connection).unwrap();
        assert_eq!(preferences[0].path, "/m/flac/1.flac");
    }
}
