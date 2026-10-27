use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use std::time::{Duration, SystemTime};

use actix_web::http::header;
use actix_web::{HttpRequest, HttpResponse, Responder, delete, get, head, post, web};
use diesel::connection::SimpleConnection;
use diesel::deserialize::QueryableByName;
use diesel::sql_types::Text;
use diesel::{Connection, RunQueryDsl};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::{Mutex, Semaphore};
use tokio_util::io::ReaderStream;
use tracing::{error, info, warn};

use crate::api::error::{
    bad_request, conflict, forbidden, internal_server_error, not_found, range_not_satisfiable,
    service_unavailable,
};
use crate::domain::Artist;
use crate::signal_path::{ServerLegRequest, StreamTarget, describe_server_leg};

const MAX_CONCURRENT_TRANSCODES: usize = 4;
const DEFAULT_TRANSCODE_CACHE_MAX_BYTES: u64 = 10 * 1024 * 1024 * 1024;
static TRANSCODE_SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
static TRANSCODE_CACHE_LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();

fn new_transcode_slots() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(MAX_CONCURRENT_TRANSCODES))
}

fn transcode_slots() -> Arc<Semaphore> {
    TRANSCODE_SLOTS.get_or_init(new_transcode_slots).clone()
}

fn transcode_cache_locks() -> &'static Mutex<HashMap<PathBuf, Arc<Mutex<()>>>> {
    TRANSCODE_CACHE_LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

struct TranscodeCleanup {
    cache_lock: Arc<Mutex<()>>,
    cache_path: PathBuf,
    committed: bool,
    temp_path: PathBuf,
}

impl Drop for TranscodeCleanup {
    fn drop(&mut self) {
        let cache_lock = self.cache_lock.clone();
        let cache_path = self.cache_path.clone();
        let temp_path = (!self.committed).then(|| self.temp_path.clone());
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                if let Some(temp_path) = temp_path {
                    let _ = tokio::fs::remove_file(temp_path).await;
                }
                release_transcode_lock(&cache_path, &cache_lock).await;
            });
        } else if let Some(temp_path) = temp_path {
            let _ = std::fs::remove_file(temp_path);
        }
    }
}

fn transcode_cache_max_bytes() -> u64 {
    std::env::var("PARSON_TRANSCODE_CACHE_MAX_BYTES")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_TRANSCODE_CACHE_MAX_BYTES)
}

fn prune_transcode_cache(
    directory: &Path,
    protected: &Path,
    max_bytes: u64,
    minimum_age: Duration,
) -> std::io::Result<()> {
    let now = SystemTime::now();
    let mut files = Vec::new();
    let mut total = 0u64;
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = match entry.metadata() {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => continue,
        };
        if path.extension().and_then(|value| value.to_str()) == Some("tmp") {
            if metadata
                .modified()
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age > Duration::from_secs(60 * 60))
            {
                let _ = std::fs::remove_file(path);
            }
            continue;
        }
        if !matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("mp3" | "flac" | "wav")
        ) {
            continue;
        }
        total = total.saturating_add(metadata.len());
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if now.duration_since(modified).unwrap_or_default() >= minimum_age {
            files.push((modified, path, metadata.len()));
        }
    }
    files.sort_by_key(|(modified, path, _)| (*modified, path.clone()));
    for (_, path, size) in files {
        if total <= max_bytes {
            break;
        }
        if path == protected {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
    Ok(())
}
use crate::app::LocalApp;
use crate::library::delivery::{
    Delivery, DeliveryPlan, OutputCapabilities, PcmCodec, SourceFile, conversion_cache_name,
    conversion_ffmpeg_args, plan_delivery,
};
use crate::library::dsd;
use crate::library::indexer::{
    CatalogProgressSender, build_instant_library_preview,
    enrich_library_to_database_with_cancellation, hydrate_progressive_catalog_artwork,
    index_available_library_to_database,
    index_available_library_to_database_progressive_with_cancellation, index_library_to_database,
    repair_library_database_with_cancellation,
};
use crate::library::normalize::{LibraryIndexReport, normalize_library_data};
use crate::library::state::{
    LibraryCache, LibraryLifecycle, LibraryScanLease, library_unavailable_response,
};
use crate::library::storage::{
    fetch_library, get_libraries_config_path, refresh_cache, store_library,
};
use crate::persistence::connection::DbPool;

pub struct ProcessedLibrary {
    report: LibraryIndexReport,
}

#[derive(Serialize, Debug)]
pub struct LibraryRefreshSuccess {
    path: String,
    report: LibraryIndexReport,
}

#[derive(Serialize, Debug)]
pub struct LibraryRefreshFailure {
    path: String,
    message: String,
}

#[derive(Serialize, Debug)]
pub struct LibraryRefreshResult {
    refreshed: Vec<LibraryRefreshSuccess>,
    failures: Vec<LibraryRefreshFailure>,
}

fn classify_refresh_result(
    refreshed: Vec<LibraryRefreshSuccess>,
    failures: Vec<LibraryRefreshFailure>,
) -> Result<LibraryRefreshResult, std::io::Error> {
    if refreshed.is_empty() && !failures.is_empty() {
        return Err(std::io::Error::other(format!(
            "No libraries were refreshed. Failures: {}",
            failures
                .iter()
                .map(|failure| format!("{}: {}", failure.path, failure.message))
                .collect::<Vec<_>>()
                .join("; ")
        )));
    }
    Ok(LibraryRefreshResult {
        refreshed,
        failures,
    })
}

struct ScanFailure {
    message: String,
    code: &'static str,
}

impl ScanFailure {
    fn new(message: impl Into<String>, code: &'static str) -> Self {
        Self {
            message: message.into(),
            code,
        }
    }
}

fn merge_progressive_catalog(mut progress: Vec<Artist>, seed: &[Artist]) -> Vec<Artist> {
    let mut artist_positions = progress
        .iter()
        .enumerate()
        .map(|(position, artist)| (artist.id.clone(), position))
        .collect::<HashMap<_, _>>();
    for seed_artist in seed {
        let Some(artist_index) = artist_positions.get(&seed_artist.id).copied() else {
            artist_positions.insert(seed_artist.id.clone(), progress.len());
            progress.push(seed_artist.clone());
            continue;
        };
        let artist = &mut progress[artist_index];
        if artist.icon_url.is_empty() {
            artist.icon_url.clone_from(&seed_artist.icon_url);
        }
        let mut album_positions = artist
            .albums
            .iter()
            .enumerate()
            .map(|(position, album)| (album.id.clone(), position))
            .collect::<HashMap<_, _>>();
        for seed_album in &seed_artist.albums {
            if let Some(album_index) = album_positions.get(&seed_album.id).copied() {
                let album = &mut artist.albums[album_index];
                if album.cover_url.is_empty() {
                    album.cover_url.clone_from(&seed_album.cover_url);
                }
                let mut song_ids = album
                    .songs
                    .iter()
                    .map(|song| song.id.clone())
                    .collect::<HashSet<_>>();
                for seed_song in &seed_album.songs {
                    if song_ids.insert(seed_song.id.clone()) {
                        album.songs.push(seed_song.clone());
                    }
                }
                album.songs.sort_unstable_by(|left, right| {
                    left.track_number
                        .cmp(&right.track_number)
                        .then(left.name.cmp(&right.name))
                });
            } else {
                album_positions.insert(seed_album.id.clone(), artist.albums.len());
                artist.albums.push(seed_album.clone());
            }
        }
    }
    progress
}

/// Progressive batches represent large scans. Publish only presentation-ready
/// albums there; the bounded instant preview remains visible as a fallback
/// until at least one album with artwork is available.
fn retain_progressive_albums_with_artwork(catalog: &mut Vec<Artist>) -> bool {
    for artist in catalog.iter_mut() {
        artist
            .albums
            .retain(|album| !album.cover_url.trim().is_empty());
    }
    catalog.retain(|artist| !artist.albums.is_empty());
    !catalog.is_empty()
}

async fn run_index_scan(
    path: String,
    lifecycle: web::Data<LibraryLifecycle>,
    scan_lease: LibraryScanLease,
) -> Result<LibraryIndexReport, ScanFailure> {
    let cancellation = scan_lease.cancellation();
    lifecycle.set_indexing("Preparing your first albums.").await;
    let result = async {
        let preview_path = path.clone();
        let (mut preview, preview_report) =
            tokio::task::spawn_blocking(move || build_instant_library_preview(&preview_path))
                .await
                .map_err(|error| {
                    ScanFailure::new(
                        format!("Instant library task stopped unexpectedly: {error}"),
                        "library_index_failed",
                    )
                })?
                .map_err(|error| {
                    ScanFailure::new(
                        format!("Failed to process library: {error:?}"),
                        "library_index_failed",
                    )
                })?;
        if cancellation.is_cancelled() {
            return Err(ScanFailure::new(
                "Library scan was replaced by a newer folder selection.",
                "library_scan_replaced",
            ));
        }
        normalize_library_data(&mut preview);
        if let Ok(seed) = fetch_library().await {
            preview = merge_progressive_catalog(preview, seed.as_ref());
        }
        store_library(preview).await;
        let preview_cache = LibraryCache::available().await.map_err(|error| {
            ScanFailure::new(
                format!("Failed to build instant library cache: {error}"),
                "library_cache_build_failed",
            )
        })?;
        lifecycle
            .set_available_with_message(
                preview_cache,
                "Your library is ready. Adding the rest in the background…",
            )
            .await;
        if let Err(error) = save_library_path(&path).await {
            error!(%error, path = %path, "failed to mirror library root to JSON config");
        }

        // Return after the preview is searchable; complete the scan in background.
        let background_lifecycle = lifecycle.clone();
        let background_path = path.clone();
        tokio::spawn(async move {
            let _scan_lease = scan_lease;
            let (progress_sender, mut progress_receiver) =
                tokio::sync::mpsc::channel::<Vec<Artist>>(1);
            let publication_lifecycle = background_lifecycle.clone();
            let publication_cancellation = cancellation.clone();
            let publisher = tokio::spawn(async move {
                while let Some(mut batch) = progress_receiver.recv().await {
                    if publication_cancellation.is_cancelled() {
                        break;
                    }
                    batch = match tokio::task::spawn_blocking(move || {
                        hydrate_progressive_catalog_artwork(&mut batch);
                        batch
                    })
                    .await
                    {
                        Ok(batch) => batch,
                        Err(error) => {
                            tracing::warn!(%error, "progressive artwork task stopped unexpectedly");
                            continue;
                        }
                    };
                    let seed = fetch_library().await.ok();
                    let mut catalog = seed
                        .as_deref()
                        .map_or(batch.clone(), |seed| merge_progressive_catalog(batch, seed));
                    if !retain_progressive_albums_with_artwork(&mut catalog) {
                        continue;
                    }
                    store_library(catalog).await;
                    match LibraryCache::available().await {
                        Ok(cache) => {
                            let songs = cache.songs_flat.len();
                            publication_lifecycle
                                .set_available_with_message(
                                    cache,
                                    format!("Your library is ready. Added {songs} songs so far…"),
                                )
                                .await
                        }
                        Err(error) => {
                            tracing::warn!(%error, "progressive catalog publication failed")
                        }
                    }
                }
            });
            match process_available_library_progressive_with_cancellation(
                &background_path,
                progress_sender,
                cancellation.clone(),
            )
            .await
            {
                Ok(_) => {
                    let _ = publisher.await;
                    if cancellation.is_cancelled() {
                        return;
                    }
                    // Re-read SQLite after draining a partial-publication race.
                    refresh_cache().await;
                    match LibraryCache::available().await {
                        Ok(cache) if !cancellation.is_cancelled() => {
                            background_lifecycle.set_available(cache).await
                        }
                        Ok(_) => return,
                        Err(error) => {
                            background_lifecycle
                                .set_enrichment_failed(error.to_string())
                                .await;
                            return;
                        }
                    }
                }
                Err(error) => {
                    publisher.abort();
                    if cancellation.is_cancelled() {
                        return;
                    }
                    background_lifecycle
                        .set_enrichment_failed(error.to_string())
                        .await;
                    return;
                }
            }
            match process_enriched_library_with_cancellation(&background_path, cancellation.clone())
                .await
            {
                Ok(_) if !cancellation.is_cancelled() => match LibraryCache::new().await {
                    Ok(cache) if !cancellation.is_cancelled() => {
                        background_lifecycle.set_ready_and_persist(cache).await
                    }
                    Ok(_) => (),
                    Err(error) => {
                        background_lifecycle
                            .set_enrichment_failed(error.to_string())
                            .await
                    }
                },
                Ok(_) => (),
                Err(error) => {
                    if cancellation.is_cancelled() {
                        return;
                    }
                    background_lifecycle
                        .set_enrichment_failed(error.to_string())
                        .await
                }
            }
        });
        Ok::<_, ScanFailure>(preview_report)
    }
    .await;
    if let Err(failure) = &result
        && failure.code != "library_scan_replaced"
    {
        error!(message = %failure.message, "library index failed");
        lifecycle.set_scan_failed(&failure.message).await;
    }
    result
}

async fn run_refresh_scan(
    lifecycle: web::Data<LibraryLifecycle>,
    scan_lease: LibraryScanLease,
    scan_message: &'static str,
) -> Result<LibraryRefreshResult, ScanFailure> {
    let cancellation = scan_lease.cancellation();
    lifecycle.set_indexing(scan_message).await;
    let result = async {
        refresh_libraries_with(false, false, Some(cancellation.clone()))
            .await
            .map_err(|error| {
                ScanFailure::new(
                    format!("Failed to refresh libraries: {error}"),
                    "library_refresh_failed",
                )
            })?;
        if cancellation.is_cancelled() {
            return Err(ScanFailure::new(
                "Library refresh was replaced by a newer folder selection.",
                "library_scan_replaced",
            ));
        }
        let cache = LibraryCache::available().await.map_err(|error| {
            ScanFailure::new(
                format!("Failed to build library cache: {error}"),
                "library_cache_build_failed",
            )
        })?;
        lifecycle.set_available(cache).await;
        let _scan_lease = scan_lease;
        let enriched = refresh_libraries_with(true, true, Some(cancellation.clone()))
            .await
            .map_err(|error| {
                ScanFailure::new(
                    format!("Failed to refresh library artwork and metadata: {error}"),
                    "library_refresh_failed",
                )
            })?;
        let cache = LibraryCache::new().await.map_err(|error| {
            ScanFailure::new(
                format!("Failed to build library cache: {error}"),
                "library_cache_build_failed",
            )
        })?;
        if enriched.failures.is_empty() {
            lifecycle.set_ready_and_persist(cache).await;
        } else {
            lifecycle
                .set_ready_with_message(
                    cache,
                    format!(
                        "Library refresh completed with {} failed root(s).",
                        enriched.failures.len()
                    ),
                )
                .await;
        }
        Ok::<_, ScanFailure>(enriched)
    }
    .await;
    if let Err(failure) = &result
        && failure.code != "library_scan_replaced"
    {
        error!(message = %failure.message, "library refresh failed");
        lifecycle.set_scan_failed(&failure.message).await;
    }
    result
}

/// Runs the refresh scan for library tools (rescan, retry). Returns the
/// number of roots that failed, or why the scan could not run.
pub(crate) async fn refresh_for_tools(
    lifecycle: web::Data<LibraryLifecycle>,
    message: &'static str,
) -> Result<usize, String> {
    let Some(scan_lease) = lifecycle.try_begin_scan() else {
        return Err(
            "A library scan is already running. Try again when it has finished.".to_string(),
        );
    };
    match tokio::spawn(run_refresh_scan(lifecycle.clone(), scan_lease, message)).await {
        Ok(Ok(result)) => Ok(result.failures.len()),
        Ok(Err(failure)) => Err(failure.message),
        Err(error) => Err(format!("The scan stopped unexpectedly: {error}")),
    }
}

async fn run_automatic_refresh_scan(
    lifecycle: Arc<LibraryLifecycle>,
    scan_lease: LibraryScanLease,
) -> Result<LibraryRefreshResult, ScanFailure> {
    let cancellation = scan_lease.cancellation();
    let _scan_lease = scan_lease;
    lifecycle.set_indexing("Adding newly detected music.").await;
    let result = match refresh_libraries_with(true, false, Some(cancellation.clone())).await {
        Ok(result) => match LibraryCache::new().await {
            Ok(cache) if result.failures.is_empty() => {
                lifecycle
                    .set_ready_with_message(cache, "New music was added automatically.")
                    .await;
                Ok(result)
            }
            Ok(cache) => {
                lifecycle
                    .set_ready_with_message(
                        cache,
                        format!(
                            "New music was added; {} library root(s) could not be refreshed.",
                            result.failures.len()
                        ),
                    )
                    .await;
                Ok(result)
            }
            Err(error) => Err(ScanFailure::new(
                format!("Automatic refresh completed but the cache could not be rebuilt: {error}"),
                "library_cache_build_failed",
            )),
        },
        Err(error) => Err(ScanFailure::new(
            format!("Failed to automatically refresh libraries: {error}"),
            "library_auto_refresh_failed",
        )),
    };
    if let Err(failure) = &result
        && !cancellation.is_cancelled()
    {
        lifecycle.set_scan_failed(&failure.message).await;
    }
    result
}

const AUTO_REFRESH_POLL_MS: u64 = 250;
const AUTO_REFRESH_ROOT_RELOAD_SECS: u64 = 5;
const AUTO_REFRESH_MIN_SCAN_INTERVAL_SECS: u64 = 10;
const AUTO_REFRESH_RETRY_SECS: u64 = 30;
const AUTO_REFRESH_FALLBACK_INITIAL_SECS: u64 = 60;
const AUTO_REFRESH_FALLBACK_MAX_SECS: u64 = 15 * 60;

fn auto_refresh_duration(name: &str, default: Duration) -> Duration {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(default)
}

fn select_auto_refresh_quiet_window(
    path_events: usize,
    overflowed: bool,
    single_file: Duration,
    album_copy: Duration,
    large_copy: Duration,
) -> Duration {
    if overflowed || path_events > 256 {
        large_copy
    } else if path_events > 8 {
        album_copy
    } else {
        single_file
    }
}

fn auto_refresh_quiet_window(path_events: usize, overflowed: bool) -> Duration {
    select_auto_refresh_quiet_window(
        path_events,
        overflowed,
        auto_refresh_duration("PARSON_AUTO_REFRESH_QUIET_MS", Duration::from_millis(1_500)),
        auto_refresh_duration("PARSON_AUTO_REFRESH_ALBUM_QUIET_MS", Duration::from_secs(4)),
        auto_refresh_duration("PARSON_AUTO_REFRESH_LARGE_QUIET_MS", Duration::from_secs(8)),
    )
}

fn automatic_change_is_settled(
    now: Instant,
    latest_change: Option<Instant>,
    path_events: usize,
    overflowed: bool,
) -> bool {
    latest_change.is_some_and(|changed| {
        now.saturating_duration_since(changed) >= auto_refresh_quiet_window(path_events, overflowed)
    })
}

/// Watches roots and coalesces changes for incremental discovery.
pub(crate) fn start_automatic_library_refresh(lifecycle: Arc<LibraryLifecycle>) {
    tokio::spawn(async move {
        let poll = auto_refresh_duration(
            "PARSON_AUTO_REFRESH_POLL_MS",
            Duration::from_millis(AUTO_REFRESH_POLL_MS),
        )
        .max(Duration::from_millis(50));
        let root_reload = auto_refresh_duration(
            "PARSON_AUTO_REFRESH_ROOT_RELOAD_MS",
            Duration::from_secs(AUTO_REFRESH_ROOT_RELOAD_SECS),
        )
        .max(Duration::from_secs(1));
        let min_scan_interval = auto_refresh_duration(
            "PARSON_AUTO_REFRESH_MIN_SCAN_INTERVAL_MS",
            Duration::from_secs(AUTO_REFRESH_MIN_SCAN_INTERVAL_SECS),
        );
        let retry_interval = auto_refresh_duration(
            "PARSON_AUTO_REFRESH_RETRY_MS",
            Duration::from_secs(AUTO_REFRESH_RETRY_SECS),
        );
        let fallback_initial = auto_refresh_duration(
            "PARSON_AUTO_REFRESH_FALLBACK_INITIAL_MS",
            Duration::from_secs(AUTO_REFRESH_FALLBACK_INITIAL_SECS),
        );
        let fallback_max = auto_refresh_duration(
            "PARSON_AUTO_REFRESH_FALLBACK_MAX_MS",
            Duration::from_secs(AUTO_REFRESH_FALLBACK_MAX_SECS),
        )
        .max(fallback_initial);
        let new_file_minimum_age = auto_refresh_duration(
            "PARSON_AUTO_REFRESH_NEW_FILE_MIN_AGE_MS",
            Duration::from_secs(5),
        );
        let mut ticker = tokio::time::interval(poll);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut roots = Vec::<PathBuf>::new();
        let mut next_root_reload = Instant::now();
        let mut last_scan_started = None::<Instant>;
        let mut retry_at = None::<Instant>;
        let mut fallback_interval = fallback_initial;
        let mut fallback_at = Instant::now() + fallback_initial;

        loop {
            ticker.tick().await;
            let now = Instant::now();
            if now >= next_root_reload {
                let configured = read_library_paths().await;
                let mut refreshed_roots = Vec::with_capacity(configured.len());
                for path in configured {
                    let path = PathBuf::from(path);
                    let canonical = tokio::fs::canonicalize(&path).await.unwrap_or(path);
                    if canonical.is_dir() {
                        crate::library::discovery::ensure_incremental_watcher(&canonical);
                        refreshed_roots.push(canonical);
                    }
                }
                refreshed_roots.sort_unstable();
                refreshed_roots.dedup();
                roots = refreshed_roots;
                next_root_reload = now + root_reload;
            }
            if roots.is_empty() {
                continue;
            }

            let mut event_count = 0_usize;
            let mut overflowed = false;
            let mut latest_change = None::<Instant>;
            let mut watcher_missing = false;
            let mut youngest_new_audio = None::<Duration>;
            for root in &roots {
                let Some(stats) = crate::library::discovery::pending_change_stats(root) else {
                    continue;
                };
                event_count = event_count.saturating_add(stats.path_events);
                overflowed |= stats.overflowed;
                watcher_missing |= !stats.watcher_active;
                latest_change = match (latest_change, stats.last_change) {
                    (Some(left), Some(right)) => Some(left.max(right)),
                    (None, right) => right,
                    (left, None) => left,
                };
                if let Some(age) = crate::library::discovery::youngest_pending_new_audio_age(root) {
                    youngest_new_audio =
                        Some(youngest_new_audio.map_or(age, |known| known.min(age)));
                }
            }

            let retry_due = retry_at.is_some_and(|deadline| now >= deadline);
            let fallback_due = watcher_missing && now >= fallback_at;
            let changes_settled =
                automatic_change_is_settled(now, latest_change, event_count, overflowed);
            if !retry_due && !fallback_due && !changes_settled {
                continue;
            }
            if !retry_due
                && !fallback_due
                && youngest_new_audio.is_some_and(|age| age < new_file_minimum_age)
            {
                continue;
            }
            if last_scan_started
                .is_some_and(|started| now.saturating_duration_since(started) < min_scan_interval)
            {
                continue;
            }
            let Some(scan_lease) = lifecycle.try_begin_scan() else {
                continue;
            };
            last_scan_started = Some(now);
            let reason = if retry_due {
                "retry"
            } else if fallback_due {
                "fallback_reconciliation"
            } else if overflowed {
                "notification_overflow"
            } else {
                "filesystem_change"
            };
            info!(
                reason,
                event_count,
                roots = roots.len(),
                "automatic library refresh started"
            );
            match run_automatic_refresh_scan(lifecycle.clone(), scan_lease).await {
                Ok(result) => {
                    let failed_roots = result.failures.len();
                    retry_at = (failed_roots > 0).then(|| Instant::now() + retry_interval);
                    let indexed_files = result
                        .refreshed
                        .iter()
                        .map(|success| success.report.indexed_files)
                        .sum::<usize>();
                    if fallback_due {
                        fallback_interval = if indexed_files > 0 {
                            fallback_initial
                        } else {
                            fallback_interval.saturating_mul(2).min(fallback_max)
                        };
                    }
                    fallback_at = Instant::now() + fallback_interval;
                    info!(
                        reason,
                        indexed_files, failed_roots, "automatic library refresh published"
                    );
                }
                Err(failure) => {
                    retry_at = Some(Instant::now() + retry_interval);
                    warn!(reason, message = %failure.message, "automatic library refresh will retry");
                }
            }
        }
    });
}

#[get("/status")]
pub async fn library_readiness(lifecycle: web::Data<LibraryLifecycle>) -> impl Responder {
    let readiness = lifecycle.readiness().await;
    let setup_required =
        readiness.state == crate::library::state::LibraryReadinessState::NoLibraryIndexed;

    HttpResponse::Ok().json(serde_json::json!({
        "state": readiness.state,
        "message": readiness.message,
        "enrichment": readiness.enrichment,
        "catalog_revision": lifecycle.catalog_revision(),
        "setup_required": setup_required,
    }))
}

#[derive(Serialize)]
pub struct LibraryRootResponse {
    path: String,
    /// False when the folder is missing on this server, for example after a
    /// backup from another computer was restored here.
    available: bool,
    /// For a missing folder: the closest folder above it that exists on this
    /// server, where "Locate…" starts browsing (the music usually moved
    /// nearby, for example to another disk under the same mount point).
    #[serde(skip_serializing_if = "Option::is_none")]
    locate_from: Option<String>,
}

impl LibraryRootResponse {
    fn new(path: String, available: bool) -> Self {
        let locate_from = (!available)
            .then(|| nearest_existing_ancestor(Path::new(&path)))
            .flatten()
            .map(|path| path.to_string_lossy().into_owned());
        Self {
            path,
            available,
            locate_from,
        }
    }
}

/// The closest existing folder at or above `path` (an absolute path); `None`
/// when not even the root exists, which can't happen on a real system.
pub(crate) fn nearest_existing_ancestor(path: &Path) -> Option<std::path::PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    path.ancestors()
        .find(|candidate| candidate.is_dir())
        .map(Path::to_path_buf)
}

#[get("/roots")]
pub async fn library_roots() -> impl Responder {
    let mut roots = Vec::new();
    for path in read_library_paths().await {
        let available =
            tokio::time::timeout(Duration::from_secs(2), tokio::fs::metadata(path.clone()))
                .await
                .is_ok_and(|metadata| metadata.is_ok_and(|metadata| metadata.is_dir()));
        roots.push(LibraryRootResponse::new(path, available));
    }
    HttpResponse::Ok().json(roots)
}

#[derive(Deserialize)]
pub struct ClassificationDiagnosticsQuery {
    offset: Option<i64>,
    limit: Option<i64>,
}

#[derive(QueryableByName)]
struct ClassificationDiagnosticRow {
    #[diesel(sql_type = Text)]
    album_id: String,
    #[diesel(sql_type = Text)]
    title: String,
    #[diesel(sql_type = Text)]
    primary_type: String,
    #[diesel(sql_type = Text)]
    explanation_json: String,
}

#[derive(Serialize)]
pub struct ClassificationDiagnostic {
    album_id: String,
    title: String,
    primary_type: String,
    explanation: serde_json::Value,
}

/// Returns the exact title analysis, evidence-derived classification, and
/// confidence persisted for each album during indexing.
#[get("/diagnostics/classifications")]
pub async fn library_classification_diagnostics(
    pool: web::Data<DbPool>,
    query: web::Query<ClassificationDiagnosticsQuery>,
) -> HttpResponse {
    let offset = query.offset.unwrap_or(0).max(0);
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    classification_diagnostics_response(pool.get_ref(), offset, limit)
}

fn classification_diagnostics_response(pool: &DbPool, offset: i64, limit: i64) -> HttpResponse {
    let mut connection = match pool.get() {
        Ok(connection) => connection,
        Err(error) => {
            return service_unavailable(error.to_string(), "library_diagnostics_unavailable");
        }
    };
    let rows = match diesel::sql_query(
        "SELECT id AS album_id, title, primary_type,
                COALESCE(release_album_json, '{}') AS explanation_json
         FROM album_entity
         ORDER BY normalized_title, id
         LIMIT ? OFFSET ?",
    )
    .bind::<diesel::sql_types::BigInt, _>(limit)
    .bind::<diesel::sql_types::BigInt, _>(offset)
    .load::<ClassificationDiagnosticRow>(&mut connection)
    {
        Ok(rows) => rows,
        Err(error) => {
            return service_unavailable(error.to_string(), "library_diagnostics_unavailable");
        }
    };
    HttpResponse::Ok().json(
        rows.into_iter()
            .map(|row| ClassificationDiagnostic {
                album_id: row.album_id,
                title: row.title,
                primary_type: row.primary_type,
                explanation: serde_json::from_str(&row.explanation_json).unwrap_or_else(|error| {
                    serde_json::json!({
                        "diagnostic_error": format!("Stored explanation is malformed: {error}")
                    })
                }),
            })
            .collect::<Vec<_>>(),
    )
}

#[derive(Deserialize)]
pub struct RemoveLibraryRootQuery {
    path: String,
}

impl RemoveLibraryRootQuery {
    pub(crate) fn new(path: String) -> Self {
        Self { path }
    }
}

#[delete("/roots")]
pub async fn remove_library_root(
    query: web::Query<RemoveLibraryRootQuery>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    remove_root(query, lifecycle).await
}

pub(crate) async fn remove_root(
    query: web::Query<RemoveLibraryRootQuery>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let path = query.path.trim();
    if path.is_empty() {
        return bad_request("Library path cannot be empty.", "library_path_empty");
    }
    if path.len() > 4096 {
        return bad_request("Library path is too long.", "library_path_too_long");
    }
    let configured = read_library_paths().await;
    if !configured
        .iter()
        .any(|configured_path| configured_path == path)
    {
        return not_found("Library folder was not found.", "library_root_not_found");
    }
    let Some(_scan_lease) = lifecycle.try_begin_scan() else {
        return conflict(
            "A library scan is already running. Wait for it to finish before removing a folder.",
            "library_scan_in_progress",
        );
    };

    let database_path = path.to_string();
    let removal = tokio::task::spawn_blocking(move || remove_indexed_root(&database_path)).await;
    match removal {
        Ok(Ok(false)) => {
            return not_found("Library folder was not found.", "library_root_not_found");
        }
        Ok(Ok(true)) => {}
        Ok(Err(error)) => {
            return internal_server_error(error.to_string(), "library_root_remove_failed");
        }
        Err(error) => {
            return internal_server_error(
                format!("Library removal task stopped unexpectedly: {error}"),
                "library_root_remove_failed",
            );
        }
    }

    if let Err(error) = crate::product::unregister_library_root(Path::new(path)) {
        warn!(%error, %path, "Could not remove library root from Core registry");
    }
    let remaining = configured
        .into_iter()
        .filter(|configured_path| configured_path != path)
        .collect::<Vec<_>>();
    if let Err(error) = write_library_paths(&remaining) {
        return internal_server_error(error.to_string(), "library_root_config_failed");
    }

    refresh_cache().await;
    if remaining.is_empty() {
        lifecycle.set_no_library().await;
    } else {
        match LibraryCache::new().await {
            Ok(cache) => lifecycle.set_ready_and_persist(cache).await,
            Err(error) => {
                lifecycle.set_scan_failed(error.to_string()).await;
                return internal_server_error(error.to_string(), "library_cache_build_failed");
            }
        }
    }

    HttpResponse::Ok().json(
        remaining
            .into_iter()
            .map(|path| {
                let available = Path::new(&path).is_dir();
                LibraryRootResponse::new(path, available)
            })
            .collect::<Vec<_>>(),
    )
}

fn remove_indexed_root(path: &str) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let pool = crate::persistence::connection::connect()?;
    let mut connection = pool.get()?;
    connection.transaction::<_, Box<dyn std::error::Error + Send + Sync>, _>(|connection| {
        let removed = diesel::sql_query("DELETE FROM library_root WHERE path = ?")
            .bind::<Text, _>(path)
            .execute(connection)?;
        if removed == 0 {
            return Ok(false);
        }
        connection.batch_execute(
            "DELETE FROM track_entity
             WHERE NOT EXISTS (SELECT 1 FROM track_file WHERE track_file.track_id = track_entity.id);
             DELETE FROM album_inference_cache
             WHERE NOT EXISTS (SELECT 1 FROM album_entity WHERE album_entity.id = album_inference_cache.album_id);
             DELETE FROM album_entity
             WHERE NOT EXISTS (SELECT 1 FROM track_entity WHERE track_entity.album_id = album_entity.id);
             DELETE FROM recording_entity
             WHERE NOT EXISTS (SELECT 1 FROM track_entity WHERE track_entity.recording_id = recording_entity.id);
             DELETE FROM release_group_entity
             WHERE NOT EXISTS (SELECT 1 FROM album_entity WHERE album_entity.release_group_id = release_group_entity.id);
             DELETE FROM artist_entity
             WHERE NOT EXISTS (SELECT 1 FROM album_artist WHERE album_artist.artist_id = artist_entity.id)
               AND NOT EXISTS (SELECT 1 FROM track_artist WHERE track_artist.artist_id = artist_entity.id);
             DELETE FROM genre_entity
             WHERE NOT EXISTS (SELECT 1 FROM album_genre WHERE album_genre.genre_id = genre_entity.id)
               AND NOT EXISTS (SELECT 1 FROM track_genre WHERE track_genre.genre_id = genre_entity.id);
             DELETE FROM artwork
             WHERE NOT EXISTS (SELECT 1 FROM album_entity WHERE album_entity.artwork_id = artwork.id)
               AND NOT EXISTS (SELECT 1 FROM artist_entity WHERE artist_entity.artwork_id = artwork.id);
             DELETE FROM library_search_document
             WHERE (entity_type = 'song' AND NOT EXISTS (SELECT 1 FROM track_entity WHERE track_entity.id = library_search_document.entity_id))
                OR (entity_type = 'album' AND NOT EXISTS (SELECT 1 FROM album_entity WHERE album_entity.id = library_search_document.entity_id))
                OR (entity_type = 'artist' AND NOT EXISTS (SELECT 1 FROM artist_entity WHERE artist_entity.id = library_search_document.entity_id));
             DELETE FROM metadata_task
             WHERE (entity_type = 'track' AND NOT EXISTS (SELECT 1 FROM track_entity WHERE track_entity.id = metadata_task.entity_id))
                OR (entity_type = 'album' AND NOT EXISTS (SELECT 1 FROM album_entity WHERE album_entity.id = metadata_task.entity_id))
                OR (entity_type = 'artist' AND NOT EXISTS (SELECT 1 FROM artist_entity WHERE artist_entity.id = metadata_task.entity_id));",
        )?;
        Ok(true)
    })
}

#[derive(Deserialize)]
pub struct CatalogQuery {
    pub offset: Option<usize>,
    pub limit: Option<usize>,
    pub section: Option<String>,
}

#[get("")]
pub async fn library_catalog(
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
    query: web::Query<CatalogQuery>,
) -> impl Responder {
    let app = LocalApp {
        database: pool.get_ref().clone(),
        library: lifecycle.into_inner(),
    };
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(50);
    let result = match query.section.as_deref() {
        Some("albums") => app.catalog_albums(offset, limit).await,
        Some("songs") => app.catalog_songs(offset, limit).await,
        Some(_) => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "error": "invalid_catalog_section"
            }));
        }
        None => app.catalog(offset, limit).await,
    };
    match result {
        Ok(catalog) => HttpResponse::Ok().json(catalog),
        Err(error) => service_unavailable(error.to_string(), "library_catalog_unavailable"),
    }
}

#[get("/artists")]
pub async fn library_catalog_artists(
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
    query: web::Query<CatalogQuery>,
) -> impl Responder {
    let app = LocalApp {
        database: pool.get_ref().clone(),
        library: lifecycle.into_inner(),
    };
    match app
        .artists(query.offset.unwrap_or(0), query.limit.unwrap_or(50))
        .await
    {
        Ok(artists) => HttpResponse::Ok().json(artists),
        Err(error) => service_unavailable(error.to_string(), "library_artists_unavailable"),
    }
}

async fn process_music_library(
    path: &str,
) -> Result<ProcessedLibrary, Box<dyn std::error::Error + Send + Sync>> {
    process_library_with(path, index_library_to_database).await
}

async fn process_available_library(
    path: &str,
) -> Result<ProcessedLibrary, Box<dyn std::error::Error + Send + Sync>> {
    process_library_with(path, index_available_library_to_database).await
}

async fn process_available_library_with_cancellation(
    path: &str,
    cancellation: crate::library::indexer::ScanCancellation,
) -> Result<ProcessedLibrary, Box<dyn std::error::Error + Send + Sync>> {
    process_library_with(path, move |path| {
        crate::library::indexer::index_available_library_to_database_with_cancellation(
            path,
            &cancellation,
        )
    })
    .await
}

async fn process_available_library_progressive_with_cancellation(
    path: &str,
    progress: CatalogProgressSender,
    cancellation: crate::library::indexer::ScanCancellation,
) -> Result<ProcessedLibrary, Box<dyn std::error::Error + Send + Sync>> {
    process_library_with(path, move |path| {
        index_available_library_to_database_progressive_with_cancellation(
            path,
            progress,
            &cancellation,
        )
    })
    .await
}

async fn process_enriched_library_with_cancellation(
    path: &str,
    cancellation: crate::library::indexer::ScanCancellation,
) -> Result<ProcessedLibrary, Box<dyn std::error::Error + Send + Sync>> {
    process_library_with(path, move |path| {
        enrich_library_to_database_with_cancellation(path, &cancellation)
    })
    .await
}

async fn process_repaired_library_with_cancellation(
    path: &str,
    cancellation: crate::library::indexer::ScanCancellation,
) -> Result<ProcessedLibrary, Box<dyn std::error::Error + Send + Sync>> {
    process_library_with(path, move |path| {
        repair_library_database_with_cancellation(path, &cancellation)
    })
    .await
}

async fn process_library_with<F>(
    path: &str,
    indexer: F,
) -> Result<ProcessedLibrary, Box<dyn std::error::Error + Send + Sync>>
where
    F: FnOnce(
            &str,
        ) -> Result<
            (Vec<crate::domain::Artist>, LibraryIndexReport),
            Box<dyn std::error::Error + Send + Sync>,
        > + Send
        + 'static,
{
    let now = Instant::now();
    let path = path.to_owned();
    // Keep blocking library scans off Actix/Tokio workers.
    let (final_data, mut report) = tokio::task::spawn_blocking(move || {
        let (mut final_data, mut report) = indexer(&path)?;
        let normalization_started = Instant::now();
        normalize_library_data(&mut final_data);
        report.timing.normalization_inference_us =
            report.timing.normalization_inference_us.saturating_add(
                normalization_started
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64,
            );
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>((final_data, report))
    })
    .await
    .map_err(|error| std::io::Error::other(format!("Library index task failed: {error}")))??;

    let elapsed = now.elapsed();
    report.timing.total_us = elapsed.as_micros().min(u128::from(u64::MAX)) as u64;
    let finished_log = format!(
        "Finished Indexing Library in {} seconds ({} indexed, {} skipped, {} warnings)",
        elapsed.as_secs(),
        report.indexed_files,
        report.skipped_files,
        report.warnings.len()
    );
    info!(finished_log);
    info!(
        run_kind = report.timing.run_kind.as_str(),
        total_us = elapsed.as_micros().min(u128::from(u64::MAX)) as u64,
        normalization_inference_us = report.timing.normalization_inference_us,
        "library request total timing"
    );

    if report.scanned_files == 0 {
        return Err(
            std::io::Error::other("The selected folder contains no supported audio files").into(),
        );
    }

    store_library(final_data).await;
    Ok(ProcessedLibrary { report })
}

#[derive(Deserialize)]
pub struct LibraryIndexRequest {
    path: String,
}

impl LibraryIndexRequest {
    pub(crate) fn new(path: String) -> Self {
        Self { path }
    }
}

#[post("")]
pub async fn index(
    request: web::Json<LibraryIndexRequest>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    index_library(request, lifecycle).await
}

pub async fn index_library(
    request: web::Json<LibraryIndexRequest>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let requested_path = request.path.trim();
    if requested_path.is_empty() {
        return bad_request("Library path cannot be empty.", "library_path_empty");
    }
    let path_to_library = match tokio::fs::canonicalize(requested_path).await {
        Ok(path) if path.is_dir() => path.to_string_lossy().into_owned(),
        _ => {
            return bad_request(
                "Library path must be an accessible directory.",
                "library_path_invalid",
            );
        }
    };
    let Some(scan_lease) = lifecycle.begin_replacing_scan().await else {
        return conflict(
            "This library selection was replaced by a newer request.",
            "library_scan_replaced",
        );
    };
    info!("Indexing new library path...");
    let scan = tokio::spawn(run_index_scan(
        path_to_library,
        lifecycle.clone(),
        scan_lease,
    ));
    match scan.await {
        Ok(Ok(report)) => HttpResponse::Ok().json(serde_json::json!({
            "library": null,
            "report": report,
        })),
        Ok(Err(failure)) if failure.code == "library_scan_replaced" => {
            conflict(failure.message, failure.code)
        }
        Ok(Err(failure)) => internal_server_error(failure.message, failure.code),
        Err(error) => {
            let message = format!("Library scan coordinator stopped unexpectedly: {error}");
            lifecycle.set_scan_failed(&message).await;
            internal_server_error(message, "library_index_failed")
        }
    }
}

pub async fn refresh_libraries()
-> Result<LibraryRefreshResult, Box<dyn std::error::Error + Send + Sync>> {
    refresh_libraries_with(true, false, None).await
}

async fn refresh_libraries_with(
    enriched: bool,
    repair: bool,
    cancellation: Option<crate::library::indexer::ScanCancellation>,
) -> Result<LibraryRefreshResult, Box<dyn std::error::Error + Send + Sync>> {
    info!("Refreshing all library paths...");

    let paths = read_library_paths().await;
    if paths.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "No saved library paths found. Choose and index a library first.",
        )
        .into());
    }

    let mut refreshed = Vec::new();
    let mut failures = Vec::new();

    for path in paths {
        if cancellation
            .as_ref()
            .is_some_and(crate::library::indexer::ScanCancellation::is_cancelled)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "Library refresh was replaced by a newer folder selection",
            )
            .into());
        }
        let processed = match (enriched, repair, cancellation.clone()) {
            (true, true, Some(cancellation)) => {
                process_repaired_library_with_cancellation(&path, cancellation).await
            }
            (true, false, Some(cancellation)) => {
                process_enriched_library_with_cancellation(&path, cancellation).await
            }
            (false, _, Some(cancellation)) => {
                process_available_library_with_cancellation(&path, cancellation).await
            }
            (true, _, None) => process_music_library(&path).await,
            (false, _, None) => process_available_library(&path).await,
        };
        match processed {
            Ok(processed) => {
                // Persist roots recovered from the database after refresh.
                if let Err(error) = save_library_path(&path).await {
                    error!(%error, %path, "Could not repair libraries config after refresh");
                }
                refreshed.push(LibraryRefreshSuccess {
                    path,
                    report: processed.report,
                });
            }
            Err(e) => {
                error!("Failed to process library {}: {:?}", path, e);
                failures.push(LibraryRefreshFailure {
                    path,
                    message: e.to_string(),
                });
            }
        }
    }

    Ok(classify_refresh_result(refreshed, failures)?)
}

#[post("/refresh")]
pub async fn library_refresh(lifecycle: web::Data<LibraryLifecycle>) -> impl Responder {
    let Some(scan_lease) = lifecycle.try_begin_scan() else {
        return conflict(
            "A library scan is already running. Wait for it to finish before starting another.",
            "library_scan_in_progress",
        );
    };
    let scan = tokio::spawn(run_refresh_scan(
        lifecycle.clone(),
        scan_lease,
        "Refreshing the indexed libraries.",
    ));
    match scan.await {
        Ok(Ok(result)) if result.failures.is_empty() => HttpResponse::Ok().json(result),
        Ok(Ok(result)) => HttpResponse::MultiStatus().json(result),
        Ok(Err(failure)) => internal_server_error(failure.message, failure.code),
        Err(error) => {
            let message = format!("Library refresh coordinator stopped unexpectedly: {error}");
            lifecycle.set_scan_failed(&message).await;
            internal_server_error(message, "library_refresh_failed")
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Libraries {
    pub paths: Vec<String>,
}

#[derive(QueryableByName)]
struct LibraryRootPathRow {
    #[diesel(sql_type = Text)]
    path: String,
}

async fn read_database_library_paths() -> Vec<String> {
    match tokio::task::spawn_blocking(|| {
        let pool = crate::persistence::connection::connect()?;
        let mut connection = pool.get()?;
        let rows = diesel::sql_query("SELECT path FROM library_root ORDER BY id")
            .load::<LibraryRootPathRow>(&mut connection)?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(
            rows.into_iter().map(|row| row.path).collect::<Vec<_>>(),
        )
    })
    .await
    {
        Ok(Ok(paths)) => paths,
        Ok(Err(error)) => {
            error!(%error, "Could not recover library paths from the database");
            Vec::new()
        }
        Err(error) => {
            error!(%error, "Database library-path task failed");
            Vec::new()
        }
    }
}

pub async fn read_library_paths() -> Vec<String> {
    let libraries_file = get_libraries_config_path();

    if !libraries_file.exists() {
        return read_database_library_paths().await;
    }

    let content = match fs::read_to_string(&libraries_file) {
        Ok(content) => content,
        Err(e) => {
            error!(
                "Could not read libraries config file {:?}; treating it as empty: {}",
                libraries_file, e
            );
            return read_database_library_paths().await;
        }
    };
    let libraries: Libraries = match serde_json::from_str(&content) {
        Ok(libraries) => libraries,
        Err(e) => {
            error!(
                "Could not parse libraries config file {:?}; treating it as empty: {}",
                libraries_file, e
            );
            return read_database_library_paths().await;
        }
    };

    if libraries.paths.is_empty() {
        read_database_library_paths().await
    } else {
        libraries.paths
    }
}

async fn save_library_path(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let libraries_file = get_libraries_config_path();

    let mut libraries = if libraries_file.exists() {
        let content = fs::read_to_string(&libraries_file)?;
        // Preserve damaged configuration for manual recovery.
        serde_json::from_str(&content)?
    } else {
        Libraries { paths: Vec::new() }
    };

    if !libraries.paths.iter().any(|existing| existing == path) {
        libraries.paths.push(path.to_string());
        write_library_paths(&libraries.paths)?;
    }

    Ok(())
}

fn write_library_paths(paths: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let libraries_file = get_libraries_config_path();
    let json = serde_json::to_string_pretty(&Libraries {
        paths: paths.to_vec(),
    })?;
    if let Some(parent) = libraries_file.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = libraries_file.with_extension("json.tmp");
    fs::write(&temporary, json)?;
    let backup = libraries_file.with_extension("json.bak");
    if libraries_file.exists() {
        let _ = fs::remove_file(&backup);
        fs::rename(&libraries_file, &backup)?;
    }
    if let Err(error) = fs::rename(&temporary, &libraries_file) {
        if backup.exists() {
            let _ = fs::rename(&backup, &libraries_file);
        }
        return Err(error.into());
    }
    let _ = fs::remove_file(backup);
    Ok(())
}

#[derive(Deserialize)]
pub struct BitrateQueryParams {
    /// Lossy streaming bitrate in kbps; 0 (the default) means best quality.
    #[serde(default)]
    pub bitrate: u32,
    pub slowed_reverb: Option<bool>,
    pub audio_effect: Option<StreamAudioEffect>,
    /// Comma-separated format tokens the player decodes natively (see
    /// [`OutputCapabilities`]). Without it the original bytes are sent.
    pub accept: Option<String>,
    pub max_sample_rate: Option<u32>,
    pub max_bit_depth: Option<u8>,
    pub max_channels: Option<u8>,
    /// `pcm` (default), `dop` or `native`.
    pub dsd: Option<String>,
}

impl BitrateQueryParams {
    fn audio_effect(&self) -> StreamAudioEffect {
        self.audio_effect.unwrap_or_else(|| {
            if self.slowed_reverb.unwrap_or(false) {
                StreamAudioEffect::SlowedReverb
            } else {
                StreamAudioEffect::None
            }
        })
    }

    fn capabilities(&self) -> OutputCapabilities {
        OutputCapabilities::from_query(
            self.accept.as_deref(),
            self.max_sample_rate,
            self.max_bit_depth,
            self.max_channels,
            self.dsd.as_deref(),
        )
    }

    fn validate(&self) -> Result<(), HttpResponse> {
        if self.bitrate > MAX_STREAM_BITRATE
            || (self.bitrate > 0 && self.bitrate < MIN_STREAM_BITRATE)
        {
            return Err(bad_request("Invalid bitrate.", "invalid_bitrate"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StreamAudioEffect {
    #[default]
    None,
    LightReverb,
    Reverb,
    DeepReverb,
    PitchDown,
    SlowedReverb,
}

impl StreamAudioEffect {
    /// The effect's id in delivery plans and the signal path.
    fn plan_name(self) -> Option<&'static str> {
        self.signal_effect().map(|effect| effect.id)
    }

    fn cache_name(self) -> &'static str {
        match self {
            Self::None => "original",
            Self::LightReverb => "light-reverb-v1",
            Self::Reverb => "reverb-v1",
            Self::DeepReverb => "deep-reverb-v1",
            Self::PitchDown => "pitch-down-v2",
            Self::SlowedReverb => "slowed-reverb-v3",
        }
    }

    fn ffmpeg_filter(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            // Mirror the web preset's mix and speed.
            Self::LightReverb => Some("aecho=0.98:0.08:70:0.20"),
            Self::Reverb => Some("aecho=0.80:0.46:70:0.28"),
            Self::DeepReverb => Some("aecho=0.62:0.76:70:0.38"),
            // Resample to 44.1 kHz first so asetrate's fixed base rate gives
            // the same pitch for every source rate, then restore duration
            // after changing sample rate for pitch-only control.
            Self::PitchDown => {
                Some("aresample=44100,asetrate=44100*0.82,aresample=44100,atempo=1.219512195")
            }
            Self::SlowedReverb => {
                Some("aresample=44100,asetrate=44100*0.82,aresample=44100,aecho=0.65:0.42:70:0.28")
            }
        }
    }

    /// How the signal path presents this effect.
    fn signal_effect(self) -> Option<crate::signal_path::DspEffect> {
        use crate::signal_path::DspEffect;
        let (id, label, detail, output_sample_rate) = match self {
            Self::None => return None,
            Self::LightReverb => (
                "light_reverb",
                "Light reverb",
                "Server echo reverb, 20% wet",
                None,
            ),
            Self::Reverb => ("reverb", "Reverb", "Server echo reverb, 28% wet", None),
            Self::DeepReverb => (
                "deep_reverb",
                "Deep reverb",
                "Server echo reverb, 38% wet",
                None,
            ),
            Self::PitchDown => (
                "pitch_down",
                "Pitch down",
                "-3.4 semitones, tempo kept",
                Some(44_100),
            ),
            Self::SlowedReverb => (
                "slowed_reverb",
                "Slowed + reverb",
                "0.82x speed and pitch, echo reverb 28% wet",
                Some(44_100),
            ),
        };
        Some(DspEffect {
            id,
            label,
            detail,
            output_sample_rate,
        })
    }
}

/// Folds surround into stereo because the MP3 encoder takes at most two
/// channels; ffmpeg picks the layout conversion.
const TRANSCODE_CHANNEL_FILTER: &str = "aformat=channel_layouts=mono|stereo";

fn transcode_filter_graph(audio_effect: StreamAudioEffect) -> String {
    match audio_effect.ffmpeg_filter() {
        Some(filter) => format!("[0:a:0]{filter},{TRANSCODE_CHANNEL_FILTER}[a]"),
        None => format!("[0:a:0]{TRANSCODE_CHANNEL_FILTER}[a]"),
    }
}

const MAX_STREAM_BITRATE: u32 = 320;
const MIN_STREAM_BITRATE: u32 = 32;
const STREAM_BUFFER_SIZE: usize = 131072;

async fn resolve_stream_song(
    song_key: &str,
    cache: &LibraryCache,
) -> Result<(String, PathBuf), HttpResponse> {
    let song = cache
        .song(song_key)
        .or_else(|| {
            cache
                .songs_flat
                .iter()
                .filter_map(|song_id| cache.song(song_id))
                .find(|song| song.path == song_key)
        })
        .ok_or_else(|| not_found("Song not found.", "song_not_found"))?;

    let canonical_song = tokio::fs::canonicalize(&song.path)
        .await
        .map_err(|_| not_found("Song file not found.", "song_file_not_found"))?;

    let metadata = tokio::fs::metadata(&canonical_song)
        .await
        .map_err(|_| not_found("Song file not found.", "song_file_not_found"))?;
    if !metadata.is_file() {
        return Err(not_found("Song file not found.", "song_file_not_found"));
    }

    let library_paths = read_library_paths().await;
    for library_path in library_paths {
        let Ok(canonical_library) = tokio::fs::canonicalize(library_path).await else {
            continue;
        };

        if canonical_song.starts_with(canonical_library) {
            return Ok((song.id.clone(), canonical_song));
        }
    }

    for indexed_song in cache
        .songs_flat
        .iter()
        .filter_map(|song_id| cache.song(song_id))
    {
        let Ok(indexed_path) = tokio::fs::canonicalize(&indexed_song.path).await else {
            continue;
        };

        if indexed_path == canonical_song {
            return Ok((song.id.clone(), canonical_song));
        }
    }

    Err(forbidden(
        "Song file is outside configured library paths.",
        "song_path_forbidden",
    ))
}

fn parse_range(
    range_header: Option<&str>,
    file_size: u64,
) -> Result<(u64, u64, bool), HttpResponse> {
    if file_size == 0 {
        return Err(range_not_satisfiable(
            "Cannot stream an empty file.",
            "empty_file_range",
        ));
    }

    let Some(range_header) = range_header else {
        return Ok((0, file_size - 1, false));
    };

    let Some(bytes_range) = range_header.trim().strip_prefix("bytes=") else {
        return Ok((0, file_size - 1, false));
    };

    let mut parts = bytes_range.splitn(2, '-');
    let start_part = parts.next().unwrap_or_default();
    let end_part = parts.next().unwrap_or_default();

    let (start, end) = if start_part.is_empty() {
        let suffix_len = end_part
            .parse::<u64>()
            .map_err(|_| range_not_satisfiable("Invalid range header.", "invalid_range"))?;
        if suffix_len == 0 {
            return Err(range_not_satisfiable(
                "Invalid range header.",
                "invalid_range",
            ));
        }
        (file_size.saturating_sub(suffix_len), file_size - 1)
    } else {
        let start = start_part
            .parse::<u64>()
            .map_err(|_| range_not_satisfiable("Invalid range header.", "invalid_range"))?;
        let end = if end_part.is_empty() {
            file_size - 1
        } else {
            end_part
                .parse::<u64>()
                .map_err(|_| range_not_satisfiable("Invalid range header.", "invalid_range"))?
        };
        (start, end.min(file_size - 1))
    };

    if start > end || start >= file_size {
        return Err(range_not_satisfiable(
            "Requested range is not satisfiable.",
            "range_not_satisfiable",
        ));
    }

    Ok((start, end, true))
}

async fn stream_file_range(
    req: &HttpRequest,
    path: &Path,
    etag_key: &str,
    content_type: &'static str,
) -> HttpResponse {
    use tokio::io::AsyncSeekExt;

    let metadata = match tokio::fs::metadata(path).await {
        Ok(metadata) => metadata,
        Err(_) => return not_found("Song file not found.", "song_file_not_found"),
    };
    let file_size = metadata.len();

    let range = req.headers().get("Range").and_then(|v| v.to_str().ok());
    let (start, end, partial) = match parse_range(range, file_size) {
        Ok(range) => range,
        Err(mut response) => {
            if let Ok(value) = header::HeaderValue::from_str(&format!("bytes */{file_size}")) {
                response.headers_mut().insert(header::CONTENT_RANGE, value);
            }
            response.headers_mut().insert(
                header::ACCEPT_RANGES,
                header::HeaderValue::from_static("bytes"),
            );
            return response;
        }
    };

    let mut file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(_) => return not_found("Song file not found.", "song_file_not_found"),
    };

    if start > 0 && file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
        return internal_server_error("Failed to seek audio stream.", "stream_seek_failed");
    }

    let stream = ReaderStream::with_capacity(file.take(end - start + 1), STREAM_BUFFER_SIZE);
    let modified = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let mut response = if partial {
        HttpResponse::PartialContent()
    } else {
        HttpResponse::Ok()
    };
    response
        .insert_header((header::ACCEPT_RANGES, "bytes"))
        .insert_header((header::CONTENT_TYPE, content_type))
        .insert_header((header::CONTENT_LENGTH, (end - start + 1).to_string()))
        .insert_header((header::ETAG, format!("\"{}-{}\"", etag_key, modified)));
    if partial {
        response.insert_header((
            header::CONTENT_RANGE,
            format!("bytes {}-{}/{}", start, end, file_size),
        ));
    }

    response.streaming(stream)
}

fn content_type_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("flac") => "audio/flac",
        Some("mp3") => "audio/mpeg",
        Some("aac") => "audio/aac",
        Some("m4a") | Some("mp4") => "audio/mp4",
        Some("wav") => "audio/wav",
        Some("ogg") | Some("oga") => "audio/ogg",
        Some("opus") => "audio/opus",
        Some("webm") => "video/webm",
        Some("mkv") => "video/x-matroska",
        _ => crate::library::formats::content_type_for_path(path),
    }
}

fn transcode_cache_path(song_id: &str, path: &Path, bitrate: u32) -> PathBuf {
    transcode_cache_path_for(song_id, path, bitrate, TranscodeFormat::Mp3)
}

fn transcode_cache_path_for(
    song_id: &str,
    path: &Path,
    bitrate: u32,
    format: TranscodeFormat,
) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(song_id.as_bytes());
    hasher.update(path.to_string_lossy().as_bytes());
    hasher.update(bitrate.to_le_bytes());
    if format != TranscodeFormat::Mp3 {
        hasher.update(format.extension().as_bytes());
    }
    if let TranscodeFormat::SonosFlac {
        gain_tenths_db,
        dsp,
        sample_rate,
        bit_depth,
        ..
    } = format
    {
        // Files made before the high-quality resampler and 24-bit lossy
        // decoding are not reused.
        hasher.update(b"resampler-v2");
        if gain_tenths_db != 0 {
            hasher.update(b"gain");
            hasher.update(gain_tenths_db.to_le_bytes());
        }
        if dsp != 0 {
            hasher.update(b"dsp-chain");
            hasher.update(dsp.to_le_bytes());
        }
        if sample_rate > 48_000 || bit_depth > 24 {
            // Hi-res renderer conversions (Sonos keys stay unchanged).
            hasher.update(sample_rate.to_le_bytes());
            hasher.update([bit_depth]);
        }
    }
    if let Ok(metadata) = std::fs::metadata(path) {
        hasher.update(metadata.len().to_le_bytes());
        if let Ok(modified) = metadata.modified()
            && let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH)
        {
            hasher.update(duration.as_nanos().to_le_bytes());
        }
    }
    let cache_key = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect::<String>();

    let mut cache_dir = dirs::cache_dir().unwrap_or_else(std::env::temp_dir);
    cache_dir.push("Parson");
    cache_dir.push("Transcodes");
    cache_dir.push(format!("{}.{}", cache_key, format.extension()));
    cache_dir
}

/// Output of the ffmpeg transcoder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TranscodeFormat {
    /// MP3 at the requested bitrate.
    Mp3,
    /// FLAC within a renderer's limits (Sonos, Chromecast, UPnP), with
    /// optional leveling gain and zone DSP.
    SonosFlac {
        sample_rate: u32,
        bit_depth: u8,
        /// Volume leveling gain in tenths of a dB (0: none).
        gain_tenths_db: i16,
        /// Fingerprint of the server DSP chain the samples ran through (0:
        /// none); the chain itself feeds the encoder (see [`ChainInput`]).
        dsp: u64,
        /// FLAC, or uncompressed PCM for renderers that take no FLAC.
        container: RenderContainer,
    },
}

/// What a renderer gets when Parson converts for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum RenderContainer {
    Flac,
    /// RIFF WAV, little-endian PCM.
    Wav,
    /// Raw big-endian PCM (`audio/L16`, `audio/L24`), as DLNA's LPCM.
    Lpcm,
}

/// The MIME type of LPCM at `rate` and `bits` (stereo), as DLNA writes it.
pub(crate) fn lpcm_mime(rate: u32, bits: u8) -> &'static str {
    match (bits > 16, rate) {
        (false, 44_100) => "audio/L16;rate=44100;channels=2",
        (false, 48_000) => "audio/L16;rate=48000;channels=2",
        (false, 88_200) => "audio/L16;rate=88200;channels=2",
        (false, 96_000) => "audio/L16;rate=96000;channels=2",
        (false, 176_400) => "audio/L16;rate=176400;channels=2",
        (false, 192_000) => "audio/L16;rate=192000;channels=2",
        (true, 44_100) => "audio/L24;rate=44100;channels=2",
        (true, 48_000) => "audio/L24;rate=48000;channels=2",
        (true, 88_200) => "audio/L24;rate=88200;channels=2",
        (true, 96_000) => "audio/L24;rate=96000;channels=2",
        (true, 176_400) => "audio/L24;rate=176400;channels=2",
        (true, 192_000) => "audio/L24;rate=192000;channels=2",
        (false, _) => "audio/L16",
        (true, _) => "audio/L24",
    }
}

/// PCM from the server's own pipeline (decoded to 64-bit float, leveled
/// and run through the zone's DSP chain, resampled and dithered to the
/// renderer's format), which the encoder reads from stdin instead of
/// decoding the file itself.
pub(crate) struct ChainInput {
    pub open: parson_output::player::PcmOpener,
    pub sample_rate: u32,
    pub channels: u8,
    /// `s16le` or `s32le`.
    pub format: &'static str,
}

/// A fingerprint of a DSP chain for cache keys: 0 when it does nothing.
pub(crate) fn dsp_fingerprint(dsp: &crate::zones::DspSettings) -> u64 {
    if !dsp.processes() {
        return 0;
    }
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(dsp).unwrap_or_default());
    let digest = hasher.finalize();
    u64::from_le_bytes(digest[..8].try_into().unwrap_or([0; 8])).max(1)
}

/// The server pipeline's output for `song` as FLAC at `sample_rate` and
/// `bit_depth` will carry it, with leveling and the DSP chain applied in
/// 64-bit float; `None` when nothing needs processing.
pub(crate) fn chain_input(
    song: &crate::domain::Song,
    sample_rate: u32,
    bit_depth: u8,
    gain_db: Option<f32>,
    dsp: &crate::zones::DspSettings,
) -> Option<ChainInput> {
    use parson_output::{DeviceCaps, OutputSettings, SampleFormat};
    let pipeline = crate::outputs::pipeline::global();
    let channels = song
        .audio
        .as_ref()
        .and_then(|audio| audio.channels)
        .filter(|channels| *channels > 0)
        .unwrap_or(2);
    let container = match bit_depth {
        depth if depth > 24 => SampleFormat::S32,
        depth if depth > 16 => SampleFormat::S24Packed,
        _ => SampleFormat::S16,
    };
    // The renderer's format, as one fixed rate and depth: the chain
    // converts to exactly what the FLAC promises.
    let caps = DeviceCaps {
        sample_rates: vec![sample_rate],
        sample_formats: vec![container],
        min_channels: 1,
        max_channels: channels,
        dsd_formats: Vec::new(),
        dsd_rates: Vec::new(),
    };
    let settings = OutputSettings {
        dsd_strategy: parson_output::DsdStrategy::Pcm,
        ..OutputSettings::default()
    };
    let leveling = gain_db.map(|gain| crate::outputs::pipeline::TrackLeveling {
        gain_db: f64::from(gain),
        source: "zone leveling".to_string(),
    });
    let samples = pipeline
        .plan_samples(
            Some(&caps),
            &settings,
            dsp,
            song.audio.as_ref(),
            leveling.as_ref(),
        )
        .ok()?;
    samples.processing.as_ref()?;
    let open = pipeline.opener(
        PathBuf::from(&song.path),
        &samples,
        Arc::new(parson_output::dsp::DspStats::default()),
    );
    Some(ChainInput {
        open,
        sample_rate: samples.plan.sample_rate,
        channels: samples.plan.channels,
        format: if container == SampleFormat::S16 {
            "s16le"
        } else {
            "s32le"
        },
    })
}

impl TranscodeFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::SonosFlac {
                container: RenderContainer::Flac,
                ..
            } => "flac",
            Self::SonosFlac {
                container: RenderContainer::Wav,
                ..
            } => "wav",
            Self::SonosFlac {
                container: RenderContainer::Lpcm,
                ..
            } => "pcm",
        }
    }

    fn content_type(self) -> &'static str {
        match self {
            Self::Mp3 => "audio/mpeg",
            Self::SonosFlac {
                container: RenderContainer::Flac,
                ..
            } => "audio/flac",
            Self::SonosFlac {
                container: RenderContainer::Wav,
                ..
            } => "audio/wav",
            Self::SonosFlac {
                container: RenderContainer::Lpcm,
                sample_rate,
                bit_depth,
                ..
            } => lpcm_mime(sample_rate, bit_depth),
        }
    }
}

/// How a song reaches a renderer (Sonos, Chromecast, UPnP).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RendererStreamPlan {
    pub mime_type: &'static str,
    /// `None` streams the original file; otherwise `container` at this
    /// rate/depth (or MP3, see `lossy`).
    pub convert_to: Option<(u32, u8)>,
    pub container: RenderContainer,
    /// The renderer takes no lossless format: convert to 320 kbps MP3.
    pub lossy: bool,
}

pub(crate) type SonosStreamPlan = RendererStreamPlan;

/// The codec of `song`, from its tags or extension.
fn stream_codec(song: &crate::domain::Song) -> (String, String) {
    let extension = Path::new(&song.path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let codec = song
        .audio
        .as_ref()
        .and_then(|audio| audio.codec.clone())
        .unwrap_or_else(|| match extension.as_str() {
            "mp3" => "mp3".to_string(),
            "aac" => "aac".to_string(),
            "flac" => "flac".to_string(),
            "wav" => "pcm".to_string(),
            "m4a" | "mp4" => "aac".to_string(),
            other => other.to_string(),
        });
    (codec, extension)
}

/// The highest rate at or below `limit` in the source's rate family
/// (44.1 kHz or 48 kHz multiples), so conversions never cross families
/// unless they must.
fn renderer_rate(sample_rate: u32, limit: u32) -> u32 {
    let base = if sample_rate.is_multiple_of(11_025) {
        44_100
    } else {
        48_000
    };
    let ceiling = sample_rate.min(limit);
    [8, 4, 2, 1]
        .iter()
        .map(|factor| base * factor)
        .find(|rate| *rate <= ceiling)
        .unwrap_or(if base <= limit { base } else { limit })
}

/// What a renderer with `limits` gets for `song`: the original file when
/// it plays it natively within its limits, else FLAC at the closest rate
/// and depth it takes (MP3 for renderers without a lossless format).
/// `processed` (leveling, DSP) always converts, so the gain can be applied.
pub(crate) fn renderer_stream_plan(
    song: &crate::domain::Song,
    limits: crate::outputs::FormatLimits,
    processed: bool,
) -> RendererStreamPlan {
    let (codec, extension) = stream_codec(song);
    let audio = song.audio.clone().unwrap_or_default();
    let sample_rate = audio.sample_rate.unwrap_or(44_100);
    let bit_depth = audio.bit_depth.unwrap_or(16);
    let within_limits = sample_rate <= limits.max_sample_rate && bit_depth <= limits.max_bit_depth;
    let direct = match codec.as_str() {
        "mp3" if limits.mp3 => Some("audio/mpeg"),
        "aac" if extension == "aac" && limits.aac => Some("audio/aac"),
        "aac" if matches!(extension.as_str(), "m4a" | "mp4") && limits.aac => Some("audio/mp4"),
        "alac" if matches!(extension.as_str(), "m4a" | "mp4") && limits.alac && within_limits => {
            Some("audio/mp4")
        }
        "flac" if extension == "flac" && limits.flac && within_limits => Some("audio/flac"),
        "pcm" if extension == "wav" && limits.wav && within_limits => Some("audio/wav"),
        _ => None,
    };
    if let Some(mime_type) = direct.filter(|_| !processed) {
        return RendererStreamPlan {
            mime_type,
            convert_to: None,
            container: RenderContainer::Flac,
            lossy: false,
        };
    }
    // FLAC when it takes it, else uncompressed PCM, else MP3.
    let container = if limits.flac {
        RenderContainer::Flac
    } else if limits.wav {
        RenderContainer::Wav
    } else if limits.lpcm {
        RenderContainer::Lpcm
    } else {
        return RendererStreamPlan {
            mime_type: "audio/mpeg",
            convert_to: None,
            container: RenderContainer::Flac,
            lossy: true,
        };
    };
    let depth_limit = limits.max_bit_depth.clamp(16, 24);
    let rate = renderer_rate(sample_rate, limits.max_sample_rate);
    let depth = if processed || !lossless_codec(&codec) {
        // 24 bits keep the resolution a gain change would otherwise lose,
        // and everything a lossy decoder puts out (it decodes to floating
        // point, not to 16 bits).
        depth_limit
    } else {
        bit_depth.clamp(16, depth_limit)
    };
    RendererStreamPlan {
        mime_type: match container {
            RenderContainer::Flac => "audio/flac",
            RenderContainer::Wav => "audio/wav",
            RenderContainer::Lpcm => lpcm_mime(rate, depth),
        },
        convert_to: Some((rate, depth)),
        container,
        lossy: false,
    }
}

/// The resampler the FLAC a renderer gets is made with.
fn renderer_resample_filter(sample_rate: u32, bit_depth: u8) -> String {
    let (sample_format, dither) = if bit_depth > 16 {
        ("s32", "none")
    } else {
        ("s16", "triangular_hp")
    };
    format!(
        "aresample={sample_rate}:filter_size=64:phase_shift=10:cutoff=0.97:linear_interp=1:osf={sample_format}:dither_method={dither}"
    )
}

/// Codecs that keep every sample of the original recording.
pub(crate) fn lossless_codec(codec: &str) -> bool {
    matches!(
        codec,
        "flac" | "alac" | "pcm" | "wav" | "aiff" | "ape" | "wavpack" | "wv"
    )
}

/// Sonos plays MP3, AAC, FLAC, ALAC and WAV up to 48 kHz/24-bit. Anything
/// above that, or any other codec, becomes FLAC at the nearest supported rate
/// (44.1 kHz for the 44.1 family, else 48 kHz): lossless sources at their own
/// depth (at most 24 bits), lossy ones (Ogg Vorbis, Opus) at 24 bits.
pub(crate) fn sonos_stream_plan(song: &crate::domain::Song) -> SonosStreamPlan {
    renderer_stream_plan(song, crate::outputs::FormatLimits::SONOS, false)
}

/// How a leveled song reaches a Sonos player: always converted (so the gain
/// can be applied) to FLAC within 48 kHz/24-bit at the source's rate family.
pub(crate) fn sonos_leveled_stream_plan(song: &crate::domain::Song) -> SonosStreamPlan {
    renderer_stream_plan(song, crate::outputs::FormatLimits::SONOS, true)
}

#[cfg(test)]
mod reliability_tests {
    use std::path::Path;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use actix_web::body::to_bytes;
    use diesel::connection::SimpleConnection;
    use diesel::r2d2::ConnectionManager;
    use diesel::sqlite::SqliteConnection;
    use tokio::sync::Mutex;

    use super::{
        LibraryRefreshFailure, LibraryRefreshSuccess, MAX_CONCURRENT_TRANSCODES, StreamAudioEffect,
        TranscodeCleanup, automatic_change_is_settled, classify_refresh_result,
        content_type_for_path, merge_progressive_catalog, new_transcode_slots, parse_range,
        prune_transcode_cache, release_transcode_lock, retain_progressive_albums_with_artwork,
        select_auto_refresh_quiet_window, stream_file_range, transcode_cache_locks,
        transcode_cache_path, transcode_slots,
    };
    use crate::domain::{Album, Artist, Song};
    use crate::library::normalize::LibraryIndexReport;
    use crate::persistence::connection::DbPool;

    #[actix_web::test]
    async fn classification_diagnostics_return_persisted_evidence_and_confidence() {
        let manager = ConnectionManager::<SqliteConnection>::new(":memory:");
        let pool: DbPool = Arc::new(
            diesel::r2d2::Pool::builder()
                .max_size(1)
                .build(manager)
                .expect("diagnostics pool"),
        );
        {
            let mut connection = pool.get().expect("diagnostics connection");
            connection
                .batch_execute(
                    "CREATE TABLE album_entity (
                         id TEXT PRIMARY KEY, title TEXT NOT NULL,
                         normalized_title TEXT NOT NULL, primary_type TEXT NOT NULL,
                         release_album_json TEXT
                     );
                     INSERT INTO album_entity VALUES (
                         'album-1', 'Short Release', 'short release', 'EP',
                         '{\"classification\":{\"primary_type\":\"EP\",\"confidence\":0.91,\"evidence\":[\"four tracks\"]}}'
                     );",
                )
                .expect("diagnostics fixture");
        }

        let response = super::classification_diagnostics_response(&pool, 0, 10);
        assert!(response.status().is_success());
        let body = to_bytes(response.into_body())
            .await
            .expect("diagnostics response body");
        let diagnostics: serde_json::Value =
            serde_json::from_slice(&body).expect("diagnostics response JSON");
        assert_eq!(diagnostics[0]["album_id"], "album-1");
        assert_eq!(
            diagnostics[0]["explanation"]["classification"]["confidence"],
            0.91
        );
        assert_eq!(
            diagnostics[0]["explanation"]["classification"]["evidence"][0],
            "four tracks"
        );
    }

    #[test]
    fn stream_audio_effects_keep_speed_pitch_and_reverb_distinct() {
        assert!(StreamAudioEffect::None.ffmpeg_filter().is_none());
        let reverb = StreamAudioEffect::Reverb.ffmpeg_filter().unwrap();
        assert!(reverb.contains("aecho"));
        assert!(!reverb.contains("asetrate"));
        assert!(
            StreamAudioEffect::LightReverb
                .ffmpeg_filter()
                .unwrap()
                .contains("0.08")
        );
        assert!(
            StreamAudioEffect::DeepReverb
                .ffmpeg_filter()
                .unwrap()
                .contains("0.76")
        );
        let pitch = StreamAudioEffect::PitchDown.ffmpeg_filter().unwrap();
        assert!(pitch.contains("asetrate"));
        assert!(pitch.contains("atempo"));
        assert!(!pitch.contains("aecho"));
        let combined = StreamAudioEffect::SlowedReverb.ffmpeg_filter().unwrap();
        assert!(combined.contains("asetrate"));
        assert!(combined.contains("aecho"));
    }

    fn signal_path_cache() -> crate::library::state::LibraryCache {
        use std::collections::HashMap;
        let song = Song {
            id: "song-1".into(),
            name: "Fixture".into(),
            path: "/music/fixture.flac".into(),
            audio: Some(crate::domain::AudioProperties {
                codec: Some("flac".into()),
                container: Some("flac".into()),
                sample_rate: Some(96_000),
                bit_depth: Some(24),
                channels: Some(2),
                bitrate_kbps: Some(2_800),
                lossless: Some(true),
            }),
            ..Song::default()
        };
        let album = Album {
            id: "album-1".into(),
            songs: vec![song],
            ..Album::default()
        };
        let artist = Artist {
            id: "artist-1".into(),
            albums: vec![album],
            ..Artist::default()
        };
        crate::library::state::LibraryCache {
            search_index: crate::library::search::SearchIndex::build(std::slice::from_ref(&artist))
                .expect("search index"),
            artists: Arc::new(vec![artist]),
            song_map: HashMap::from([("song-1".into(), ("artist-1".into(), "album-1".into()))]),
            album_genres: HashMap::new(),
            artist_positions: HashMap::from([("artist-1".into(), 0)]),
            album_positions: HashMap::from([("album-1".into(), (0, 0))]),
            song_positions: HashMap::from([("song-1".into(), (0, 0, 0))]),
            songs_flat: vec!["song-1".into()],
            songs_by_artist: HashMap::new(),
            songs_by_genre: HashMap::new(),
            image_paths: Default::default(),
        }
    }

    #[actix_web::test]
    async fn signal_path_route_describes_the_requested_stream() {
        use actix_web::{App, test, web};
        let lifecycle = crate::library::state::LibraryLifecycle::new();
        lifecycle.set_ready(signal_path_cache()).await;
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(lifecycle))
                .service(super::song_signal_path),
        )
        .await;

        let original: serde_json::Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::get()
                .uri("/media/songs/song-1/signal-path")
                .to_request(),
        )
        .await;
        assert_eq!(original["mode"], "original");
        assert_eq!(original["quality"], "lossless");
        assert_eq!(original["source"]["summary"], "FLAC 96kHz 24bit 2ch");
        assert_eq!(original["source"]["high_res"], true);
        assert_eq!(original["content_type"], "audio/flac");

        let reverb: serde_json::Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::get()
                .uri("/media/songs/song-1/signal-path?bitrate=0&audio_effect=slowed_reverb")
                .to_request(),
        )
        .await;
        assert_eq!(reverb["mode"], "transcode");
        assert_eq!(reverb["effect"], "slowed_reverb");
        assert_eq!(reverb["delivered"]["sample_rate"], 44_100);
        assert_eq!(reverb["delivered"]["bitrate_kbps"], 192);

        let cast: serde_json::Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::get()
                .uri("/media/songs/song-1/signal-path?target=cast")
                .to_request(),
        )
        .await;
        assert_eq!(cast["target"], "cast");
        // Cast receivers take FLAC up to 24-bit/96 kHz, so this file is sent as is.
        assert_eq!(cast["mode"], "original");
        assert_eq!(cast["delivered"]["codec"], "flac");
        assert_eq!(cast["quality"], "lossless");

        // The player's formats plan the leg exactly as they plan the stream.
        let limited: serde_json::Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::get()
                .uri("/media/songs/song-1/signal-path?accept=flac,mp3&max_sample_rate=48000")
                .to_request(),
        )
        .await;
        assert_eq!(limited["mode"], "transcode");
        assert_eq!(limited["delivered"]["codec"], "flac");
        assert_eq!(limited["delivered"]["sample_rate"], 48_000);
        assert_eq!(limited["content_type"], "audio/flac");

        for (uri, status) in [
            ("/media/songs/song-1/signal-path?bitrate=999", 400),
            ("/media/songs/missing/signal-path", 404),
            ("/media/songs/song-1/signal-path?target=sonos", 400),
            ("/media/songs/song-1/signal-path?target=native", 400),
        ] {
            let response =
                test::call_service(&app, test::TestRequest::get().uri(uri).to_request()).await;
            assert_eq!(response.status().as_u16(), status, "{uri}");
        }
    }

    #[test]
    fn transcodes_fold_surround_and_pitch_effects_normalize_the_base_rate() {
        assert_eq!(
            super::transcode_filter_graph(StreamAudioEffect::None),
            "[0:a:0]aformat=channel_layouts=mono|stereo[a]"
        );
        let graph = super::transcode_filter_graph(StreamAudioEffect::Reverb);
        assert!(graph.starts_with("[0:a:0]aecho="));
        assert!(graph.ends_with(",aformat=channel_layouts=mono|stereo[a]"));
        for effect in [
            StreamAudioEffect::PitchDown,
            StreamAudioEffect::SlowedReverb,
        ] {
            let filter = effect.ffmpeg_filter().unwrap();
            assert!(filter.starts_with("aresample=44100,asetrate="), "{filter}");
            assert_eq!(
                effect.signal_effect().unwrap().output_sample_rate,
                Some(44_100)
            );
        }
        assert!(StreamAudioEffect::None.signal_effect().is_none());
        assert_eq!(
            StreamAudioEffect::DeepReverb.signal_effect().unwrap().id,
            "deep_reverb"
        );
    }

    #[test]
    fn partial_refreshes_preserve_root_diagnostics_but_total_failures_error() {
        let failure = || LibraryRefreshFailure {
            path: "offline-library".into(),
            message: "drive unavailable".into(),
        };
        let partial = classify_refresh_result(
            vec![LibraryRefreshSuccess {
                path: "available-library".into(),
                report: LibraryIndexReport::default(),
            }],
            vec![failure()],
        )
        .expect("partial refresh result");
        assert_eq!(partial.refreshed.len(), 1);
        assert_eq!(partial.failures.len(), 1);
        assert!(classify_refresh_result(Vec::new(), vec![failure()]).is_err());
    }

    #[test]
    fn progressive_catalog_keeps_seed_artwork_and_visible_seed_entities() {
        let seed = vec![
            Artist {
                id: "artist-a".into(),
                name: "A".into(),
                icon_url: "artist-a.jpg".into(),
                albums: vec![Album {
                    id: "album-a".into(),
                    name: "A album".into(),
                    cover_url: "album-a.jpg".into(),
                    songs: vec![Song {
                        id: "old-song".into(),
                        name: "Old song".into(),
                        ..Song::default()
                    }],
                    ..Album::default()
                }],
                ..Artist::default()
            },
            Artist {
                id: "artist-c".into(),
                name: "C".into(),
                icon_url: "artist-c.jpg".into(),
                ..Artist::default()
            },
        ];
        let progress = vec![
            Artist {
                id: "artist-a".into(),
                name: "A".into(),
                albums: vec![Album {
                    id: "album-a".into(),
                    name: "A album".into(),
                    songs: vec![Song {
                        id: "new-song".into(),
                        name: "New song".into(),
                        ..Song::default()
                    }],
                    ..Album::default()
                }],
                ..Artist::default()
            },
            Artist {
                id: "artist-b".into(),
                name: "B".into(),
                ..Artist::default()
            },
        ];
        let merged = merge_progressive_catalog(progress, &seed);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].icon_url, "artist-a.jpg");
        assert_eq!(merged[0].albums[0].cover_url, "album-a.jpg");
        assert_eq!(merged[0].albums[0].songs.len(), 2);
        assert_eq!(merged[2].icon_url, "artist-c.jpg");
    }

    #[test]
    fn large_progressive_publications_hide_albums_without_artwork() {
        let mut catalog = vec![Artist {
            id: "artist-a".into(),
            name: "A".into(),
            albums: vec![
                Album {
                    id: "covered".into(),
                    name: "Covered".into(),
                    cover_url: "cover.jpg".into(),
                    ..Album::default()
                },
                Album {
                    id: "uncovered".into(),
                    name: "Uncovered".into(),
                    cover_url: "  ".into(),
                    ..Album::default()
                },
            ],
            ..Artist::default()
        }];

        assert!(retain_progressive_albums_with_artwork(&mut catalog));
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].albums.len(), 1);
        assert_eq!(catalog[0].albums[0].id, "covered");
    }

    #[test]
    fn progressive_publication_keeps_the_preview_when_no_artwork_is_ready() {
        let mut catalog = vec![Artist {
            id: "artist-a".into(),
            name: "A".into(),
            albums: vec![Album {
                id: "uncovered".into(),
                name: "Uncovered".into(),
                ..Album::default()
            }],
            ..Artist::default()
        }];

        assert!(!retain_progressive_albums_with_artwork(&mut catalog));
        assert!(catalog.is_empty());
    }

    #[test]
    fn automatic_refresh_debounce_scales_with_copy_size_and_waits_for_quiet() {
        let single = Duration::from_millis(1_500);
        let album = Duration::from_secs(4);
        let large = Duration::from_secs(8);
        assert_eq!(
            select_auto_refresh_quiet_window(8, false, single, album, large),
            single
        );
        assert_eq!(
            select_auto_refresh_quiet_window(9, false, single, album, large),
            album
        );
        assert_eq!(
            select_auto_refresh_quiet_window(257, false, single, album, large),
            large
        );
        assert_eq!(
            select_auto_refresh_quiet_window(1, true, single, album, large),
            large
        );

        let now = Instant::now();
        assert!(!automatic_change_is_settled(now, Some(now), 1, false));
        assert!(automatic_change_is_settled(
            now,
            Some(now - Duration::from_secs(20)),
            1,
            false
        ));
    }

    #[test]
    fn transcode_cache_key_includes_bitrate() {
        let source = Path::new("library/song.flac");
        assert_ne!(
            transcode_cache_path("song", source, 128),
            transcode_cache_path("song", source, 320)
        );
        assert_eq!(
            transcode_cache_path("song", source, 128),
            transcode_cache_path("song", source, 128)
        );
    }

    #[test]
    fn transcode_cache_key_changes_when_the_source_changes() {
        let directory =
            std::env::temp_dir().join(format!("music-transcode-key-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).expect("cache key directory");
        let source = directory.join("song.flac");
        std::fs::write(&source, [1u8]).expect("first source version");
        let first = transcode_cache_path("song", &source, 128);
        std::fs::write(&source, [1u8, 2u8]).expect("second source version");
        let second = transcode_cache_path("song", &source, 128);
        assert_ne!(first, second);
        std::fs::remove_dir_all(directory).expect("cache key cleanup");
    }

    #[test]
    fn transcode_capacity_is_bounded() {
        // A pool built exactly like the shared one: tests running alongside
        // this one may hold the shared pool's permits for their conversions,
        // and taking them all here would starve those tests.
        let slots = new_transcode_slots();
        let permits = (0..MAX_CONCURRENT_TRANSCODES)
            .map(|_| slots.clone().try_acquire_owned().expect("available slot"))
            .collect::<Vec<_>>();
        assert!(slots.clone().try_acquire_owned().is_err());
        drop(permits);
        assert!(slots.try_acquire_owned().is_ok());
        // The shared pool is built the same way.
        assert!(transcode_slots().available_permits() <= MAX_CONCURRENT_TRANSCODES);
    }

    #[test]
    fn full_and_partial_byte_ranges_are_distinguished() {
        assert_eq!(parse_range(None, 100).expect("full range"), (0, 99, false));
        assert_eq!(
            parse_range(Some("items=0-9"), 100).expect("ignored range unit"),
            (0, 99, false)
        );
        assert_eq!(
            parse_range(Some("bytes=10-19"), 100).expect("explicit range"),
            (10, 19, true)
        );
        assert_eq!(
            parse_range(Some("bytes=-10"), 100).expect("suffix range"),
            (90, 99, true)
        );
        assert!(parse_range(Some("bytes=100-"), 100).is_err());
        assert!(parse_range(Some("bytes=20-10"), 100).is_err());

        let very_large_file = u64::from(u32::MAX) + 10_000;
        assert_eq!(
            parse_range(
                Some(&format!("bytes={}-", very_large_file - 512)),
                very_large_file,
            )
            .expect("64-bit end range"),
            (very_large_file - 512, very_large_file - 1, true)
        );
    }

    #[test]
    fn direct_stream_content_types_cover_mixed_library_formats() {
        for (path, expected) in [
            ("track.flac", "audio/flac"),
            ("track.MP3", "audio/mpeg"),
            ("track.m4a", "audio/mp4"),
            ("track.wav", "audio/wav"),
            ("track.ogg", "audio/ogg"),
            ("track.opus", "audio/opus"),
            ("track.webm", "video/webm"),
        ] {
            assert_eq!(content_type_for_path(Path::new(path)), expected);
        }
    }

    #[actix_web::test]
    async fn stream_ranges_handle_boundaries_and_disappearing_files() {
        let path = std::env::temp_dir().join(format!(
            "parson-stream-reliability-{}.bin",
            uuid::Uuid::new_v4()
        ));
        tokio::fs::write(&path, b"0123456789")
            .await
            .expect("stream fixture");

        let request = actix_web::test::TestRequest::get()
            .insert_header((actix_web::http::header::RANGE, "bytes=0-0"))
            .to_http_request();
        let response = stream_file_range(&request, &path, "boundary-test", "audio/mpeg").await;
        assert_eq!(
            response.status(),
            actix_web::http::StatusCode::PARTIAL_CONTENT
        );
        assert_eq!(
            response
                .headers()
                .get(actix_web::http::header::CONTENT_RANGE)
                .and_then(|value| value.to_str().ok()),
            Some("bytes 0-0/10")
        );
        let body = actix_web::body::to_bytes(response.into_body())
            .await
            .expect("range body");
        assert_eq!(body.as_ref(), b"0");

        tokio::fs::remove_file(&path)
            .await
            .expect("remove stream fixture");
        let missing = stream_file_range(&request, &path, "missing-test", "audio/mpeg").await;
        assert_eq!(missing.status(), actix_web::http::StatusCode::NOT_FOUND);
    }

    #[test]
    fn transcode_cache_evicts_old_files_but_keeps_active_output() {
        let directory = std::env::temp_dir().join(format!(
            "music-transcode-prune-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&directory).expect("test cache directory");
        let active = directory.join("active.mp3");
        std::fs::write(directory.join("a.mp3"), [0u8; 6]).expect("cache fixture");
        std::fs::write(directory.join("b.mp3"), [0u8; 6]).expect("cache fixture");
        std::fs::write(&active, [0u8; 6]).expect("cache fixture");

        prune_transcode_cache(&directory, &active, 8, std::time::Duration::ZERO)
            .expect("cache cleanup");

        let total = std::fs::read_dir(&directory)
            .expect("cache listing")
            .filter_map(Result::ok)
            .filter_map(|entry| entry.metadata().ok())
            .map(|metadata| metadata.len())
            .sum::<u64>();
        assert!(active.exists());
        assert!(total <= 8);
        std::fs::remove_dir_all(directory).expect("test cleanup");
    }

    #[actix_web::test]
    async fn transcode_lock_is_retained_until_the_last_waiter_finishes() {
        let path = std::env::temp_dir().join(format!(
            "music-transcode-lock-test-{}.mp3",
            uuid::Uuid::new_v4()
        ));
        let lock = Arc::new(Mutex::new(()));
        transcode_cache_locks()
            .lock()
            .await
            .insert(path.clone(), lock.clone());
        let waiter = lock.clone();

        release_transcode_lock(&path, &lock).await;
        assert!(transcode_cache_locks().lock().await.contains_key(&path));

        drop(lock);
        release_transcode_lock(&path, &waiter).await;
        assert!(!transcode_cache_locks().lock().await.contains_key(&path));
    }

    #[actix_web::test]
    async fn abandoned_transcodes_remove_temporary_files_and_lock_entries() {
        let identity = uuid::Uuid::new_v4();
        let cache_path = std::env::temp_dir().join(format!("music-transcode-{identity}.mp3"));
        let temp_path = std::env::temp_dir().join(format!("music-transcode-{identity}.tmp"));
        std::fs::write(&temp_path, b"partial transcode").expect("temporary transcode fixture");
        let cache_lock = Arc::new(Mutex::new(()));
        transcode_cache_locks()
            .lock()
            .await
            .insert(cache_path.clone(), cache_lock.clone());

        drop(TranscodeCleanup {
            cache_lock,
            cache_path: cache_path.clone(),
            committed: false,
            temp_path: temp_path.clone(),
        });

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if !temp_path.exists()
                    && !transcode_cache_locks()
                        .lock()
                        .await
                        .contains_key(&cache_path)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("abandoned transcode cleanup");
    }
}

async fn stream_cached_transcode(
    req: &HttpRequest,
    input_path: &Path,
    cache_path: &Path,
    etag_key: &str,
    bitrate: u32,
    audio_effect: StreamAudioEffect,
    (format, chain): (TranscodeFormat, Option<ChainInput>),
) -> HttpResponse {
    if tokio::fs::metadata(cache_path).await.is_ok() {
        return stream_file_range(req, cache_path, etag_key, format.content_type()).await;
    }

    let cache_lock = {
        let mut locks = transcode_cache_locks().lock().await;
        locks
            .entry(cache_path.to_path_buf())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let cache_guard = cache_lock.clone().lock_owned().await;
    if tokio::fs::metadata(cache_path).await.is_ok() {
        drop(cache_guard);
        release_transcode_lock(cache_path, &cache_lock).await;
        return stream_file_range(req, cache_path, etag_key, format.content_type()).await;
    }
    let Some(parent) = cache_path.parent() else {
        drop(cache_guard);
        release_transcode_lock(cache_path, &cache_lock).await;
        return internal_server_error("Failed to prepare transcoded stream.", "transcode_failed");
    };

    if tokio::fs::create_dir_all(parent).await.is_err() {
        drop(cache_guard);
        release_transcode_lock(cache_path, &cache_lock).await;
        return internal_server_error("Failed to prepare transcoded stream.", "transcode_failed");
    }

    let permit = match transcode_slots().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            drop(cache_guard);
            release_transcode_lock(cache_path, &cache_lock).await;
            return service_unavailable(
                "The transcoder is busy. Retry shortly.",
                "transcode_capacity_reached",
            );
        }
    };

    let temp_path = cache_path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let bitrate_arg = format!("{}k", bitrate);
    let mut command = Command::new("ffmpeg");
    command.kill_on_drop(true);
    // The server's chain feeds the encoder through a pipe, from a thread of
    // its own (it decodes and processes as fast as ffmpeg reads).
    let mut feeder = None;
    if let Some(chain) = chain {
        let fed = std::io::pipe().and_then(|(reader, writer)| {
            let mut open = chain.open;
            let source = open(0)?;
            Ok((reader, writer, source))
        });
        let Ok((reader, writer, source)) = fed else {
            drop(cache_guard);
            release_transcode_lock(cache_path, &cache_lock).await;
            return internal_server_error("Failed to start transcoder.", "transcode_failed");
        };
        command
            .args(["-y", "-f", chain.format, "-ar"])
            .arg(chain.sample_rate.to_string())
            .arg("-ac")
            .arg(chain.channels.to_string())
            .args(["-i", "pipe:0"])
            .stdin(std::process::Stdio::from(reader));
        feeder = Some((writer, source));
    } else {
        command
            .args(["-y", "-i"])
            .arg(input_path)
            .stdin(Stdio::null());
    }
    match format {
        TranscodeFormat::Mp3 => {
            command.args([
                "-filter_complex",
                &transcode_filter_graph(audio_effect),
                "-map",
                "[a]",
                "-b:a",
                &bitrate_arg,
                "-f",
                "mp3",
                "-c:a",
                "libmp3lame",
                "-compression_level",
                "0",
                "-write_xing",
                "0",
            ]);
        }
        TranscodeFormat::SonosFlac {
            sample_rate,
            bit_depth,
            gain_tenths_db,
            dsp: _,
            container,
        } => {
            if let Some(filter) = audio_effect.ffmpeg_filter() {
                command.args([
                    "-filter_complex",
                    &format!("[0:a:0]{filter}[a]"),
                    "-map",
                    "[a]",
                ]);
            } else {
                command.args(["-map", "0:a:0"]);
            }
            let sample_format = if bit_depth > 16 { "s32" } else { "s16" };
            if feeder.is_none() {
                let mut filters = Vec::new();
                if gain_tenths_db != 0 {
                    filters.push(format!("volume={:.1}dB", f32::from(gain_tenths_db) / 10.0));
                }
                // A long linear-phase filter for any rate change, and TPDF
                // dither (high-passed) when the result is cut to 16 bits,
                // instead of ffmpeg's short default filter and truncation.
                filters.push(renderer_resample_filter(sample_rate, bit_depth));
                if audio_effect.ffmpeg_filter().is_none() {
                    command.args(["-af", &filters.join(",")]);
                }
            }
            // With a chain, it already leveled, processed, resampled and
            // dithered: the encoder only packs the samples.
            match container {
                RenderContainer::Flac => {
                    command
                        .args(["-vn", "-c:a", "flac", "-ar"])
                        .arg(sample_rate.to_string())
                        .args(["-sample_fmt", sample_format, "-bits_per_raw_sample"])
                        .arg(bit_depth.to_string())
                        .args(["-compression_level", "5", "-f", "flac"]);
                }
                RenderContainer::Wav => {
                    command
                        .args(["-vn", "-ac", "2", "-ar"])
                        .arg(sample_rate.to_string())
                        .args(if bit_depth > 16 {
                            ["-c:a", "pcm_s24le", "-f", "wav"]
                        } else {
                            ["-c:a", "pcm_s16le", "-f", "wav"]
                        });
                }
                RenderContainer::Lpcm => {
                    command
                        .args(["-vn", "-ac", "2", "-ar"])
                        .arg(sample_rate.to_string())
                        .args(if bit_depth > 16 {
                            ["-c:a", "pcm_s24be", "-f", "s24be"]
                        } else {
                            ["-c:a", "pcm_s16be", "-f", "s16be"]
                        });
                }
            }
        }
    }
    command.args([
        "-flush_packets",
        "1",
        "-threads",
        "2",
        "-v",
        "error",
        "pipe:1",
    ]);
    command.stdout(Stdio::piped()).stderr(Stdio::null());
    let Ok(mut child) = command.spawn() else {
        drop(cache_guard);
        release_transcode_lock(cache_path, &cache_lock).await;
        return internal_server_error("Failed to start transcoder.", "transcode_failed");
    };
    // The command holds the pipe's read end; drop it so the encoder sees
    // the end of the input when the feeder finishes.
    drop(command);
    if let Some((mut writer, mut source)) = feeder {
        std::thread::spawn(move || {
            // A closed pipe (the encoder stopped) ends the copy.
            let _ = std::io::copy(&mut source, &mut writer);
        });
    }
    let Some(stdout) = child.stdout.take() else {
        drop(cache_guard);
        release_transcode_lock(cache_path, &cache_lock).await;
        return internal_server_error("Failed to start transcoder.", "transcode_failed");
    };
    let Ok(output) = tokio::fs::File::create(&temp_path).await else {
        drop(child);
        drop(cache_guard);
        release_transcode_lock(cache_path, &cache_lock).await;
        return internal_server_error("Failed to prepare transcode cache.", "transcode_failed");
    };

    struct LiveTranscode {
        cache_guard: tokio::sync::OwnedMutexGuard<()>,
        child: tokio::process::Child,
        cleanup: TranscodeCleanup,
        failed: bool,
        output: tokio::fs::File,
        permit: tokio::sync::OwnedSemaphorePermit,
        stdout: tokio::process::ChildStdout,
    }

    let state = LiveTranscode {
        cache_guard,
        child,
        cleanup: TranscodeCleanup {
            cache_lock,
            cache_path: cache_path.to_path_buf(),
            committed: false,
            temp_path,
        },
        failed: false,
        output,
        permit,
        stdout,
    };
    let stream = futures::stream::unfold(state, |mut state| async move {
        if state.failed {
            let _ = state.child.kill().await;
            return None;
        }
        let mut buffer = vec![0; STREAM_BUFFER_SIZE];
        match state.stdout.read(&mut buffer).await {
            Ok(0) => {
                let success = state
                    .child
                    .wait()
                    .await
                    .is_ok_and(|status| status.success())
                    && state.output.flush().await.is_ok();
                drop(state.output);
                let promoted = success
                    && tokio::fs::rename(&state.cleanup.temp_path, &state.cleanup.cache_path)
                        .await
                        .is_ok();
                if promoted {
                    state.cleanup.committed = true;
                    let directory = state.cleanup.cache_path.parent().map(Path::to_path_buf);
                    let protected = state.cleanup.cache_path.clone();
                    if let Some(directory) = directory {
                        tokio::spawn(async move {
                            let max_bytes = transcode_cache_max_bytes();
                            let cleanup = tokio::task::spawn_blocking(move || {
                                prune_transcode_cache(
                                    &directory,
                                    &protected,
                                    max_bytes,
                                    Duration::from_secs(10 * 60),
                                )
                            })
                            .await;
                            match cleanup {
                                Ok(Ok(())) => {}
                                Ok(Err(error)) => {
                                    tracing::warn!(%error, "transcode cache cleanup failed")
                                }
                                Err(error) => {
                                    tracing::warn!(%error, "transcode cache cleanup task failed")
                                }
                            }
                        });
                    }
                }
                drop(state.permit);
                drop(state.cache_guard);
                None
            }
            Ok(size) => {
                buffer.truncate(size);
                if let Err(error) = state.output.write_all(&buffer).await {
                    let _ = state.child.kill().await;
                    state.failed = true;
                    return Some((Err(error), state));
                }
                Some((Ok(web::Bytes::from(buffer)), state))
            }
            Err(error) => {
                state.failed = true;
                Some((Err(error), state))
            }
        }
    });

    HttpResponse::Ok()
        .insert_header((header::CONTENT_TYPE, format.content_type()))
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .streaming(stream)
}

async fn release_transcode_lock(cache_path: &Path, cache_lock: &Arc<Mutex<()>>) {
    let mut locks = transcode_cache_locks().lock().await;
    let is_current = locks
        .get(cache_path)
        .is_some_and(|current| Arc::ptr_eq(current, cache_lock));
    // Additional references belong to callers sharing this mutex.
    if is_current && Arc::strong_count(cache_lock) == 2 {
        locks.remove(cache_path);
    }
}

#[get("/media/songs/{song}/stream")]
pub(crate) async fn stream_song(
    req: HttpRequest,
    path: web::Path<String>,
    query: web::Query<BitrateQueryParams>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> impl Responder {
    if let Err(response) = query.validate() {
        return response;
    }
    let song_key = path.into_inner();
    // Live radio stations stream through the same route as `radio:<id>`.
    if crate::live_radio::is_live_song_id(&song_key) {
        return crate::api::live_radio::stream_station(&req, &song_key).await;
    }
    // TIDAL and Qobuz tracks share the stream route under provider-scoped ids.
    if let Some((provider, raw_id)) = crate::providers::parse_entity_id(&song_key) {
        return crate::api::providers::stream_provider_track(&req, provider, raw_id, query.bitrate)
            .await;
    }
    stream_song_planned(
        &req,
        &song_key,
        query.bitrate,
        query.audio_effect(),
        &query.capabilities(),
        &lifecycle,
    )
    .await
}

#[head("/media/songs/{song}/stream")]
async fn head_stream_song(
    path: web::Path<String>,
    query: web::Query<BitrateQueryParams>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    if let Err(response) = query.validate() {
        return response;
    }
    let audio_effect = query.audio_effect();
    let song_key = path.into_inner();
    if crate::live_radio::is_live_song_id(&song_key) {
        // A live stream has no length; the GET opens the station.
        return HttpResponse::Ok()
            .insert_header((header::ACCEPT_RANGES, "none"))
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .finish();
    }
    if crate::providers::parse_entity_id(&song_key).is_some() {
        // Resolving a streaming-service URL costs an API call, so HEAD only
        // confirms the route; the GET reports format and length.
        return HttpResponse::Ok()
            .insert_header((header::ACCEPT_RANGES, "bytes"))
            .finish();
    }
    let (song_id, song_path) = match resolve_stream_song_with_refresh(&song_key, &lifecycle).await {
        Ok(resolved) => resolved,
        Err(response) => return response,
    };
    let source = stream_source(&lifecycle, &song_id, &song_path).await;
    let plan = plan_delivery(
        &source,
        &query.capabilities(),
        query.bitrate,
        audio_effect.plan_name(),
    );
    let cached_length = |path: PathBuf| async move {
        tokio::fs::metadata(&path).await.ok().map(|metadata| {
            let modified = metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs())
                .unwrap_or(0);
            (metadata.len(), modified)
        })
    };
    let mut response = match &plan.delivery {
        Delivery::Original => {
            let Some((length, modified)) = cached_length(song_path.clone()).await else {
                return not_found("Song file not found.", "song_file_not_found");
            };
            let mut response = HttpResponse::Ok();
            response
                .insert_header((header::ACCEPT_RANGES, "bytes"))
                .insert_header((header::CONTENT_TYPE, content_type_for_path(&song_path)))
                .insert_header((header::CONTENT_LENGTH, length.to_string()))
                .insert_header((header::ETAG, format!("\"{}-{}\"", song_id, modified)));
            response
        }
        Delivery::Dop { .. } => {
            let path = song_path.clone();
            let length = tokio::task::spawn_blocking(move || {
                read_dsd_stream(&path).map(|stream| dsd::dop_wav_length(&stream))
            })
            .await
            .ok()
            .and_then(Result::ok);
            let Some(length) = length else {
                return unsupported_media("This DSD file cannot be sent as DoP.");
            };
            let mut response = HttpResponse::Ok();
            response
                .insert_header((header::ACCEPT_RANGES, "bytes"))
                .insert_header((header::CONTENT_TYPE, "audio/wav"))
                .insert_header((header::CONTENT_LENGTH, length.to_string()));
            response
        }
        Delivery::Pcm { .. } => {
            let cache_path = conversion_cache_path(&song_id, &song_path, &plan.delivery);
            let mut response = HttpResponse::Ok();
            response.insert_header((header::CONTENT_TYPE, plan.content_type.clone()));
            if let Some(cache_path) = cache_path
                && let Some((length, modified)) = cached_length(cache_path).await
            {
                response
                    .insert_header((header::ACCEPT_RANGES, "bytes"))
                    .insert_header((header::CONTENT_LENGTH, length.to_string()))
                    .insert_header((header::ETAG, format!("\"{song_id}-pcm-{modified}\"")));
            }
            response
        }
        Delivery::Mp3 { bitrate } => {
            let cache_identity = mp3_cache_identity(&song_id, audio_effect);
            let cache_path = transcode_cache_path(&cache_identity, &song_path, *bitrate);
            let mut response = HttpResponse::Ok();
            response.insert_header((header::CONTENT_TYPE, "audio/mpeg"));
            if let Some((length, modified)) = cached_length(cache_path).await {
                response
                    .insert_header((header::ACCEPT_RANGES, "bytes"))
                    .insert_header((header::CONTENT_LENGTH, length.to_string()))
                    .insert_header((
                        header::ETAG,
                        format!("\"{}-{}k-{}\"", cache_identity, bitrate, modified),
                    ));
            }
            response
        }
    };
    response.insert_header((DELIVERY_HEADER, plan.mode));
    response.finish()
}

/// Describes how the server would deliver a track to this player: the same
/// decision `GET .../stream` makes for the same query.
#[get("/media/songs/{song}/delivery")]
async fn song_delivery(
    path: web::Path<String>,
    query: web::Query<BitrateQueryParams>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    if let Err(response) = query.validate() {
        return response;
    }
    let song_key = path.into_inner();
    let (song_id, song_path) = match resolve_stream_song_with_refresh(&song_key, &lifecycle).await {
        Ok(resolved) => resolved,
        Err(response) => return response,
    };
    let source = stream_source(&lifecycle, &song_id, &song_path).await;
    let plan = plan_delivery(
        &source,
        &query.capabilities(),
        query.bitrate,
        query.audio_effect().plan_name(),
    );
    HttpResponse::Ok()
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .json(serde_json::json!({ "song_id": song_id, "plan": plan }))
}

#[derive(Deserialize)]
pub struct SignalPathQuery {
    #[serde(default)]
    pub bitrate: u32,
    pub slowed_reverb: Option<bool>,
    pub audio_effect: Option<StreamAudioEffect>,
    pub target: Option<StreamTarget>,
    /// The player's formats, as sent with the stream request (see
    /// [`BitrateQueryParams`]).
    pub accept: Option<String>,
    pub max_sample_rate: Option<u32>,
    pub max_bit_depth: Option<u8>,
    pub max_channels: Option<u8>,
    pub dsd: Option<String>,
}

/// Describes the server leg of the signal path for a stream request with
/// the same parameters as `/media/songs/{song}/stream`.
#[get("/media/songs/{song}/signal-path")]
pub async fn song_signal_path(
    request: HttpRequest,
    path: web::Path<String>,
    query: web::Query<SignalPathQuery>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let bitrate = query.bitrate;
    if bitrate > MAX_STREAM_BITRATE || (bitrate > 0 && bitrate < MIN_STREAM_BITRATE) {
        return bad_request("Invalid bitrate.", "invalid_bitrate");
    }
    let audio_effect = query.audio_effect.unwrap_or_else(|| {
        if query.slowed_reverb.unwrap_or(false) {
            StreamAudioEffect::SlowedReverb
        } else {
            StreamAudioEffect::None
        }
    });
    if crate::live_radio::is_live_song_id(&path) {
        let target = query.target.unwrap_or(StreamTarget::Direct);
        return crate::api::live_radio::station_signal_path(&request, &path, target).await;
    }
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    let Some(song) = cache.song(&path) else {
        return not_found("Song not found.", "song_not_found");
    };
    let target = query.target.unwrap_or(StreamTarget::Direct);
    if target == StreamTarget::Native {
        // The server plays its own outputs as zones; each zone reports the
        // path of what it plays (`output.stream.signal_path`).
        return bad_request(
            "Server outputs are zones now; read the zone's signal path.",
            "native_target_is_a_zone",
        );
    }
    let song_path = Path::new(&song.path);
    let extension = song_path
        .extension()
        .and_then(|extension| extension.to_str());
    let request = ServerLegRequest {
        bitrate_kbps: bitrate,
        effect: audio_effect.signal_effect(),
        target,
        // The same capabilities the stream route plans with.
        capabilities: if target == StreamTarget::Cast {
            OutputCapabilities::google_cast()
        } else {
            OutputCapabilities::from_query(
                query.accept.as_deref(),
                query.max_sample_rate,
                query.max_bit_depth,
                query.max_channels,
                query.dsd.as_deref(),
            )
        },
    };
    HttpResponse::Ok().json(describe_server_leg(
        &song.id,
        song.audio.as_ref(),
        extension,
        content_type_for_path(song_path),
        &request,
    ))
}

async fn resolve_stream_song_with_refresh(
    song_key: &str,
    lifecycle: &web::Data<LibraryLifecycle>,
) -> Result<(String, PathBuf), HttpResponse> {
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return Err(library_unavailable_response(readiness)),
    };
    match resolve_stream_song(song_key, cache.as_ref()).await {
        Ok(resolved) => Ok(resolved),
        Err(response) if response.status() == actix_web::http::StatusCode::NOT_FOUND => {
            match LibraryCache::new().await {
                Ok(fresh_cache) => {
                    lifecycle.set_ready_and_persist(fresh_cache).await;
                    let cache = match lifecycle.cache().await {
                        Ok(cache) => cache,
                        Err(readiness) => return Err(library_unavailable_response(readiness)),
                    };
                    resolve_stream_song(song_key, cache.as_ref()).await
                }
                Err(_) => Err(response),
            }
        }
        Err(response) => Err(response),
    }
}

/// Response header naming the delivery mode (`original`, `lossless_conversion`, `dop`, `lossy_transcode`).
const DELIVERY_HEADER: &str = "x-parson-delivery";
/// Polling interval while following a conversion that is still being written.
const CONVERSION_POLL: Duration = Duration::from_millis(25);
/// A conversion that runs longer than this is abandoned.
const CONVERSION_TIMEOUT: Duration = Duration::from_secs(60 * 60);

fn unsupported_media(message: &str) -> HttpResponse {
    HttpResponse::UnsupportedMediaType()
        .json(serde_json::json!({ "error": "unsupported_media", "message": message }))
}

/// What the library knows about a track's file, for delivery planning.
async fn stream_source(
    lifecycle: &web::Data<LibraryLifecycle>,
    song_id: &str,
    song_path: &Path,
) -> SourceFile {
    let audio = lifecycle
        .cache()
        .await
        .ok()
        .and_then(|cache| cache.song(song_id).and_then(|song| song.audio.clone()));
    SourceFile::new(&song_path.to_string_lossy(), audio.as_ref())
}

fn mp3_cache_identity(song_id: &str, audio_effect: StreamAudioEffect) -> String {
    if audio_effect == StreamAudioEffect::None {
        song_id.to_string()
    } else {
        format!("{song_id}-{}", audio_effect.cache_name())
    }
}

fn conversion_cache_path(song_id: &str, path: &Path, delivery: &Delivery) -> Option<PathBuf> {
    let name = conversion_cache_name(delivery)?;
    let mp3_style = transcode_cache_path(&format!("{song_id}-{name}"), path, 0);
    let extension = match delivery {
        Delivery::Pcm {
            codec: PcmCodec::Wav,
            ..
        } => "wav",
        _ => "flac",
    };
    Some(mp3_style.with_extension(extension))
}

fn read_dsd_stream(path: &Path) -> Result<dsd::DsdInfo, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    dsd::read(&mut std::io::BufReader::new(file)).map(|file| file.info)
}

fn with_delivery_header(mut response: HttpResponse, plan: &DeliveryPlan) -> HttpResponse {
    if let Ok(value) = header::HeaderValue::from_str(plan.mode) {
        response
            .headers_mut()
            .insert(header::HeaderName::from_static(DELIVERY_HEADER), value);
    }
    response
}

async fn stream_song_planned(
    req: &HttpRequest,
    song_key: &str,
    bitrate: u32,
    audio_effect: StreamAudioEffect,
    capabilities: &OutputCapabilities,
    lifecycle: &web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let (song_id, song_path) = match resolve_stream_song_with_refresh(song_key, lifecycle).await {
        Ok(resolved) => resolved,
        Err(response) => return response,
    };
    let source = stream_source(lifecycle, &song_id, &song_path).await;
    let plan = plan_delivery(&source, capabilities, bitrate, audio_effect.plan_name());
    let response = match &plan.delivery {
        Delivery::Original => {
            stream_file_range(req, &song_path, &song_id, content_type_for_path(&song_path)).await
        }
        Delivery::Mp3 { bitrate } => {
            let cache_identity = mp3_cache_identity(&song_id, audio_effect);
            let cache_path = transcode_cache_path(&cache_identity, &song_path, *bitrate);
            stream_cached_transcode(
                req,
                &song_path,
                &cache_path,
                &format!("{}-{}k", cache_identity, bitrate),
                *bitrate,
                audio_effect,
                (TranscodeFormat::Mp3, None),
            )
            .await
        }
        Delivery::Pcm { .. } => {
            let (Some(cache_path), Some(args)) = (
                conversion_cache_path(&song_id, &song_path, &plan.delivery),
                conversion_ffmpeg_args(&source, &plan.delivery),
            ) else {
                return internal_server_error("Failed to plan conversion.", "transcode_failed");
            };
            stream_conversion(
                req,
                &song_path,
                &cache_path,
                &format!("{song_id}-pcm"),
                &plan.content_type,
                args,
            )
            .await
        }
        Delivery::Dop { .. } => stream_dop(req, &song_path, &song_id).await,
    };
    with_delivery_header(response, &plan)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConversionState {
    Running,
    Done,
    Failed,
}

type ConversionJobs =
    Mutex<HashMap<PathBuf, (PathBuf, tokio::sync::watch::Receiver<ConversionState>)>>;
static CONVERSION_JOBS: OnceLock<ConversionJobs> = OnceLock::new();

fn conversion_jobs() -> &'static ConversionJobs {
    CONVERSION_JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Starts (or joins) a background ffmpeg conversion into the transcode cache.
///
/// The conversion writes a seekable temporary file so the finished cache entry
/// has a complete header (exact length, seek table), and it runs to the end
/// even when the first listener disconnects, so seeking and replays are served
/// from the cache with byte ranges.
async fn start_conversion(
    input_path: &Path,
    cache_path: &Path,
    args: Vec<String>,
) -> Result<(PathBuf, tokio::sync::watch::Receiver<ConversionState>), HttpResponse> {
    let mut jobs = conversion_jobs().lock().await;
    if let Some((temp_path, state)) = jobs.get(cache_path) {
        return Ok((temp_path.clone(), state.clone()));
    }
    let Some(parent) = cache_path.parent() else {
        return Err(internal_server_error(
            "Failed to prepare converted stream.",
            "transcode_failed",
        ));
    };
    if tokio::fs::create_dir_all(parent).await.is_err() {
        return Err(internal_server_error(
            "Failed to prepare converted stream.",
            "transcode_failed",
        ));
    }
    let Ok(permit) = transcode_slots().try_acquire_owned() else {
        return Err(service_unavailable(
            "The transcoder is busy. Retry shortly.",
            "transcode_capacity_reached",
        ));
    };
    let temp_path = cache_path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut command = Command::new("ffmpeg");
    command
        .kill_on_drop(true)
        .args(["-y", "-nostdin", "-v", "error", "-i"])
        .arg(input_path)
        .args(&args)
        .args(["-threads", "2"])
        .arg(&temp_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let Ok(mut child) = command.spawn() else {
        return Err(internal_server_error(
            "Failed to start transcoder.",
            "transcode_failed",
        ));
    };
    let (sender, receiver) = tokio::sync::watch::channel(ConversionState::Running);
    jobs.insert(
        cache_path.to_path_buf(),
        (temp_path.clone(), receiver.clone()),
    );
    drop(jobs);
    let cache_path = cache_path.to_path_buf();
    let job_temp = temp_path.clone();
    tokio::spawn(async move {
        let mut stderr = child.stderr.take();
        let status = tokio::time::timeout(CONVERSION_TIMEOUT, child.wait()).await;
        let mut message = String::new();
        if let Some(stderr) = stderr.as_mut() {
            let _ = stderr.take(4096).read_to_string(&mut message).await;
        }
        let succeeded = matches!(status, Ok(Ok(status)) if status.success());
        let promoted = succeeded && tokio::fs::rename(&job_temp, &cache_path).await.is_ok();
        if !promoted {
            if status.is_err() {
                let _ = child.kill().await;
            }
            warn!(path = %cache_path.display(), error = %message.trim(), "audio conversion failed");
            let _ = tokio::fs::remove_file(&job_temp).await;
        }
        let _ = sender.send(if promoted {
            ConversionState::Done
        } else {
            ConversionState::Failed
        });
        conversion_jobs().lock().await.remove(&cache_path);
        drop(permit);
        if promoted && let Some(directory) = cache_path.parent().map(Path::to_path_buf) {
            let protected = cache_path.clone();
            let max_bytes = transcode_cache_max_bytes();
            let _ = tokio::task::spawn_blocking(move || {
                prune_transcode_cache(
                    &directory,
                    &protected,
                    max_bytes,
                    Duration::from_secs(10 * 60),
                )
            })
            .await;
        }
    });
    Ok((temp_path, receiver))
}

/// Streams a lossless conversion: from the cache with byte ranges when it is
/// complete, otherwise by following the file ffmpeg is still writing.
async fn stream_conversion(
    req: &HttpRequest,
    input_path: &Path,
    cache_path: &Path,
    etag_key: &str,
    content_type: &str,
    args: Vec<String>,
) -> HttpResponse {
    let content_type: &'static str = match content_type {
        "audio/wav" => "audio/wav",
        _ => "audio/flac",
    };
    if tokio::fs::metadata(cache_path).await.is_ok() {
        return stream_file_range(req, cache_path, etag_key, content_type).await;
    }
    let (temp_path, mut state) = match start_conversion(input_path, cache_path, args).await {
        Ok(job) => job,
        Err(response) => return response,
    };
    // Wait for ffmpeg to create its output.
    let started = Instant::now();
    let file = loop {
        if let Ok(file) = tokio::fs::File::open(&temp_path).await {
            break file;
        }
        let current = *state.borrow();
        match current {
            ConversionState::Done => {
                return stream_file_range(req, cache_path, etag_key, content_type).await;
            }
            ConversionState::Failed => {
                return unsupported_media("The server could not decode this file.");
            }
            ConversionState::Running if started.elapsed() > Duration::from_secs(15) => {
                return internal_server_error("The transcoder did not start.", "transcode_failed");
            }
            ConversionState::Running => {
                let _ = tokio::time::timeout(CONVERSION_POLL, state.changed()).await;
            }
        }
    };

    struct Follow {
        file: tokio::fs::File,
        state: tokio::sync::watch::Receiver<ConversionState>,
        finished: bool,
    }
    let stream = futures::stream::unfold(
        Follow {
            file,
            state,
            finished: false,
        },
        |mut follow| async move {
            if follow.finished {
                return None;
            }
            let mut buffer = vec![0; STREAM_BUFFER_SIZE];
            loop {
                let current = *follow.state.borrow();
                match follow.file.read(&mut buffer).await {
                    Ok(0) => match current {
                        ConversionState::Running => {
                            let _ =
                                tokio::time::timeout(CONVERSION_POLL, follow.state.changed()).await;
                        }
                        // Everything ffmpeg wrote before finishing has been read.
                        ConversionState::Done => return None,
                        ConversionState::Failed => {
                            follow.finished = true;
                            return Some((
                                Err(std::io::Error::other("audio conversion failed")),
                                follow,
                            ));
                        }
                    },
                    Ok(size) => {
                        buffer.truncate(size);
                        return Some((Ok(web::Bytes::from(buffer)), follow));
                    }
                    Err(error) => {
                        follow.finished = true;
                        return Some((Err(error), follow));
                    }
                }
            }
        },
    );
    HttpResponse::Ok()
        .insert_header((header::CONTENT_TYPE, content_type))
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .streaming(stream)
}

/// Streams a DSF/DFF file as DoP v1.1 in a 24-bit WAV, with byte ranges.
async fn stream_dop(req: &HttpRequest, path: &Path, song_id: &str) -> HttpResponse {
    const CHUNK: u64 = 192 * 1024;
    let source = path.to_path_buf();
    let opened = tokio::task::spawn_blocking(move || {
        let stream = read_dsd_stream(&source)?;
        let file = std::fs::File::open(&source).map_err(|error| error.to_string())?;
        Ok::<_, String>((stream, file))
    })
    .await;
    let Ok(Ok((stream, file))) = opened else {
        return unsupported_media("This DSD file cannot be sent as DoP.");
    };
    if !dsd::supports_dop(&stream) {
        return unsupported_media("DST-compressed DSD cannot be sent as DoP.");
    }
    let total = dsd::dop_wav_length(&stream);
    let range = req.headers().get("Range").and_then(|v| v.to_str().ok());
    let (start, end, partial) = match parse_range(range, total) {
        Ok(range) => range,
        Err(mut response) => {
            if let Ok(value) = header::HeaderValue::from_str(&format!("bytes */{total}")) {
                response.headers_mut().insert(header::CONTENT_RANGE, value);
            }
            return response;
        }
    };
    let stream = Arc::new(stream);
    let body = futures::stream::unfold(
        (Some(std::io::BufReader::new(file)), start),
        move |(reader, offset)| {
            let stream = Arc::clone(&stream);
            async move {
                let mut reader = reader?;
                if offset > end {
                    return None;
                }
                let chunk_end = (offset + CHUNK - 1).min(end);
                let result = tokio::task::spawn_blocking(move || {
                    let bytes = dsd::dop_wav_range(&mut reader, &stream, offset, chunk_end);
                    (reader, bytes)
                })
                .await;
                match result {
                    Ok((reader, Ok(bytes))) if !bytes.is_empty() => Some((
                        Ok::<_, std::io::Error>(web::Bytes::from(bytes)),
                        (Some(reader), chunk_end + 1),
                    )),
                    Ok((_, Ok(_))) => None,
                    Ok((_, Err(error))) => {
                        Some((Err(std::io::Error::other(error)), (None, end + 1)))
                    }
                    Err(error) => Some((Err(std::io::Error::other(error)), (None, end + 1))),
                }
            }
        },
    );
    let mut response = if partial {
        HttpResponse::PartialContent()
    } else {
        HttpResponse::Ok()
    };
    response
        .insert_header((header::ACCEPT_RANGES, "bytes"))
        .insert_header((header::CONTENT_TYPE, "audio/wav"))
        .insert_header((header::CONTENT_LENGTH, (end - start + 1).to_string()))
        .insert_header((header::ETAG, format!("\"{song_id}-dop\"")));
    if partial {
        response.insert_header((
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{total}"),
        ));
    }
    response.no_chunking(end - start + 1).streaming(body)
}

/// Plans delivery to a Google Cast receiver (FLAC up to 24-bit/96 kHz).
pub(crate) fn cast_delivery_plan(song: &crate::domain::Song) -> DeliveryPlan {
    plan_delivery(
        &SourceFile::new(&song.path, song.audio.as_ref()),
        &OutputCapabilities::google_cast(),
        0,
        None,
    )
}

/// Streams Sonos-compatible audio: the original file when Sonos plays it
/// natively, else FLAC within 48 kHz/24-bit. Leveled audio (`gain_db` is
/// set, even to 0) is always converted, as its DIDL-Lite promised FLAC.
pub(crate) async fn stream_sonos_compatible(
    req: &HttpRequest,
    song_key: &str,
    lifecycle: &web::Data<LibraryLifecycle>,
    gain_db: Option<f32>,
) -> HttpResponse {
    stream_renderer_compatible(
        req,
        song_key,
        lifecycle,
        crate::outputs::FormatLimits::SONOS,
        gain_db,
        crate::zones::DspSettings::default(),
    )
    .await
}

/// Streams what a renderer with `limits` plays: the original file when it
/// can (bit-perfect), else FLAC within its limits. Leveling (`gain_db`,
/// even 0) and DSP always convert, so the processing can be applied.
pub(crate) async fn stream_renderer_compatible(
    req: &HttpRequest,
    song_key: &str,
    lifecycle: &web::Data<LibraryLifecycle>,
    limits: crate::outputs::FormatLimits,
    gain_db: Option<f32>,
    dsp: crate::zones::DspSettings,
) -> HttpResponse {
    let gain_tenths_db = gain_db
        .map(|gain| (gain * 10.0).round().clamp(-480.0, 240.0) as i16)
        .unwrap_or(0);
    let processed = gain_db.is_some() || dsp.processes();
    let song = lifecycle
        .cache()
        .await
        .ok()
        .and_then(|cache| cache.song(song_key).cloned());
    let Some(song) = song else {
        return not_found("Song not found.", "song_not_found");
    };
    let plan = renderer_stream_plan(&song, limits, processed);
    let (song_id, song_path) = match resolve_stream_song_with_refresh(song_key, lifecycle).await {
        Ok(resolved) => resolved,
        Err(response) => return response,
    };
    if plan.lossy {
        let cache_path = transcode_cache_path(&song_id, &song_path, MAX_STREAM_BITRATE);
        return stream_cached_transcode(
            req,
            &song_path,
            &cache_path,
            &format!("{song_id}-renderer-mp3"),
            MAX_STREAM_BITRATE,
            StreamAudioEffect::None,
            (TranscodeFormat::Mp3, None),
        )
        .await;
    }
    match plan.convert_to {
        None => stream_file_range(req, &song_path, &song_id, plan.mime_type).await,
        Some((sample_rate, bit_depth)) => {
            let chain = processed
                .then(|| {
                    let song = crate::domain::Song {
                        path: song_path.to_string_lossy().to_string(),
                        ..song.clone()
                    };
                    chain_input(&song, sample_rate, bit_depth, gain_db, &dsp)
                })
                .flatten();
            let dsp = if chain.is_some() {
                dsp_fingerprint(&dsp)
            } else {
                0
            };
            let format = TranscodeFormat::SonosFlac {
                sample_rate,
                bit_depth,
                gain_tenths_db,
                dsp,
                container: plan.container,
            };
            let cache_path = transcode_cache_path_for(&song_id, &song_path, 0, format);
            let dsp_key = if dsp == 0 {
                String::new()
            } else {
                format!("-dsp{:08x}", dsp as u32)
            };
            stream_cached_transcode(
                req,
                &song_path,
                &cache_path,
                &format!("{song_id}-sonos-{sample_rate}-{bit_depth}-{gain_tenths_db}{dsp_key}"),
                0,
                StreamAudioEffect::None,
                (format, chain),
            )
            .await
        }
    }
}

/// Streams receiver-safe audio for Google Cast, at the best quality it accepts.
pub(crate) async fn stream_cast_compatible(
    req: &HttpRequest,
    song_key: &str,
    lifecycle: &web::Data<LibraryLifecycle>,
) -> HttpResponse {
    stream_song_planned(
        req,
        song_key,
        0,
        StreamAudioEffect::None,
        &OutputCapabilities::google_cast(),
        lifecycle,
    )
    .await
}

#[cfg(test)]
mod locate_tests {
    use super::{LibraryRootResponse, nearest_existing_ancestor};

    #[test]
    fn locate_starts_at_the_nearest_folder_that_still_exists() {
        let root = std::env::temp_dir().join(format!("parson-locate-{}", uuid::Uuid::new_v4()));
        let mount = root.join("mnt/music-disk");
        std::fs::create_dir_all(&mount).unwrap();
        let moved = mount.join("Old Library/FLAC");
        assert_eq!(nearest_existing_ancestor(&moved), Some(mount.clone()));
        assert_eq!(nearest_existing_ancestor(&mount), Some(mount.clone()));
        assert_eq!(
            nearest_existing_ancestor(std::path::Path::new("relative/x")),
            None
        );

        let missing = LibraryRootResponse::new(moved.to_string_lossy().into_owned(), false);
        assert_eq!(
            missing.locate_from.as_deref(),
            Some(mount.to_string_lossy().as_ref())
        );
        let present = LibraryRootResponse::new(mount.to_string_lossy().into_owned(), true);
        assert_eq!(present.locate_from, None);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod delivery_tests {
    //! On-demand conversion and DoP streaming, end to end through ffmpeg.

    use std::path::{Path, PathBuf};

    use actix_web::body::to_bytes;
    use actix_web::http::{StatusCode, header};

    use super::{conversion_ffmpeg_args, dsd, plan_delivery, stream_conversion, stream_dop};
    use crate::domain::AudioProperties;
    use crate::library::delivery::{Delivery, OutputCapabilities, SourceFile};

    fn ffmpeg_available() -> bool {
        ["ffmpeg", "ffprobe"].iter().all(|program| {
            std::process::Command::new(program)
                .arg("-version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        })
    }

    fn scratch() -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("parson-delivery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    /// `(codec, sample_rate, bits_per_raw_sample, channels, samples)` of a file.
    fn probe(path: &Path) -> (String, u32, u32, u32, u64) {
        let output = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "a:0",
                "-count_packets",
                "-show_entries",
                "stream=codec_name,sample_rate,bits_per_raw_sample,channels,duration_ts",
                "-of",
                "default=noprint_wrappers=1",
            ])
            .arg(path)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        let field = |name: &str| {
            text.lines()
                .find_map(|line| line.strip_prefix(&format!("{name}=")))
                .unwrap_or_default()
                .to_string()
        };
        let decoded = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(path)
            .args(["-f", "s32le", "-ac", "1", "-"])
            .output()
            .unwrap()
            .stdout
            .len() as u64
            / 4;
        (
            field("codec_name"),
            field("sample_rate").parse().unwrap_or_default(),
            field("bits_per_raw_sample").parse().unwrap_or_default(),
            field("channels").parse().unwrap_or_default(),
            decoded,
        )
    }

    async fn convert(
        directory: &Path,
        input: &Path,
        audio: AudioProperties,
        capabilities: &OutputCapabilities,
    ) -> (PathBuf, Vec<u8>) {
        let source = SourceFile::new(&input.to_string_lossy(), Some(&audio));
        let plan = plan_delivery(&source, capabilities, 0, None);
        assert_eq!(plan.mode, "lossless_conversion");
        let args = conversion_ffmpeg_args(&source, &plan.delivery).unwrap();
        let cache = directory.join(format!("{}.flac", uuid::Uuid::new_v4()));
        let request = actix_web::test::TestRequest::get().to_http_request();
        let response =
            stream_conversion(&request, input, &cache, "test", &plan.content_type, args).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body()).await.unwrap().to_vec();
        (cache, body)
    }

    /// Peak of a file's samples, as a fraction of full scale.
    fn peak(path: &Path) -> f64 {
        let decoded = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(path)
            .args(["-f", "s32le", "-"])
            .output()
            .unwrap()
            .stdout;
        decoded
            .as_chunks::<4>()
            .0
            .iter()
            .map(|sample| f64::from(i32::from_le_bytes(*sample)).abs() / f64::from(i32::MAX))
            .fold(0.0, f64::max)
    }

    #[actix_web::test]
    async fn served_streams_run_the_zone_dsp_chain_before_the_encoder() {
        if !ffmpeg_available() {
            return;
        }
        let directory = scratch();
        let input = directory.join("tone.flac");
        assert!(
            std::process::Command::new("ffmpeg")
                .args(["-v", "error", "-f", "lavfi", "-i"])
                .arg("sine=frequency=1000:sample_rate=44100:duration=1")
                .args(["-af", "volume=-6dB", "-ac", "2", "-sample_fmt", "s16", "-y"])
                .arg(&input)
                .status()
                .unwrap()
                .success()
        );
        let song = crate::domain::Song {
            id: "tone".into(),
            path: input.to_string_lossy().to_string(),
            duration: 1.0,
            audio: Some(AudioProperties {
                codec: Some("flac".into()),
                container: Some("flac".into()),
                sample_rate: Some(44_100),
                bit_depth: Some(16),
                channels: Some(2),
                bitrate_kbps: None,
                lossless: Some(true),
            }),
            ..crate::domain::Song::default()
        };
        // Nothing to process: no chain, the encoder reads the file.
        let flat = crate::zones::DspSettings::default();
        assert!(super::chain_input(&song, 48_000, 24, None, &flat).is_none());
        assert_eq!(super::dsp_fingerprint(&flat), 0);
        // 6 dB of headroom halves the level, in the server's own chain,
        // which also converts to the renderer's 48 kHz/24-bit.
        let mut dsp = crate::zones::DspSettings::default();
        dsp.headroom.enabled = true;
        dsp.headroom.gain_db = -6.0;
        assert_ne!(super::dsp_fingerprint(&dsp), 0);
        let chain = super::chain_input(&song, 48_000, 24, None, &dsp).expect("a chain");
        assert_eq!((chain.sample_rate, chain.format), (48_000, "s32le"));
        let cache = directory.join("processed.flac");
        let request = actix_web::test::TestRequest::get().to_http_request();
        let response = super::stream_cached_transcode(
            &request,
            &input,
            &cache,
            "tone-processed",
            0,
            super::StreamAudioEffect::None,
            (
                super::TranscodeFormat::SonosFlac {
                    sample_rate: 48_000,
                    bit_depth: 24,
                    gain_tenths_db: 0,
                    dsp: super::dsp_fingerprint(&dsp),
                    container: super::RenderContainer::Flac,
                },
                Some(chain),
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body()).await.unwrap();
        assert!(!body.is_empty());
        let (codec, rate, bits, channels, samples) = probe(&cache);
        assert_eq!(
            (codec.as_str(), rate, bits, channels),
            ("flac", 48_000, 24, 2)
        );
        assert!((47_000..=49_000).contains(&samples), "{samples} samples");
        let (source, processed) = (peak(&input), peak(&cache));
        let ratio = processed / source;
        assert!((0.48..0.52).contains(&ratio), "{source} -> {processed}");
        let _ = std::fs::remove_dir_all(directory);
    }

    #[actix_web::test]
    async fn wavpack_converts_to_bit_identical_24_bit_flac_and_is_cached() {
        if !ffmpeg_available() {
            eprintln!("skipping: ffmpeg is not installed");
            return;
        }
        let directory = scratch();
        let input = directory.join("track.wv");
        std::fs::write(
            &input,
            include_bytes!("../../testdata/audio/wavpack-96k-24bit-stereo.wv"),
        )
        .unwrap();
        let audio = AudioProperties {
            codec: Some("wavpack".into()),
            container: Some("wavpack".into()),
            sample_rate: Some(96_000),
            bit_depth: Some(24),
            channels: Some(2),
            bitrate_kbps: None,
            lossless: Some(true),
        };
        let browser = OutputCapabilities::from_query(Some("flac,mp3,opus"), None, None, None, None);
        let (cache, live) = convert(&directory, &input, audio, &browser).await;
        assert_eq!(&live[..4], b"fLaC");
        assert!(cache.exists(), "the finished conversion is cached");

        let (codec, rate, bits, channels, samples) = probe(&cache);
        assert_eq!(
            (codec.as_str(), rate, bits, channels),
            ("flac", 96_000, 24, 2)
        );
        assert_eq!(samples, probe(&input).4, "no samples added or lost");
        // The decoded audio is bit-identical to the WavPack source.
        let pcm = |path: &Path| {
            std::process::Command::new("ffmpeg")
                .args(["-v", "error", "-i"])
                .arg(path)
                .args(["-f", "s32le", "-"])
                .output()
                .unwrap()
                .stdout
        };
        assert_eq!(pcm(&cache), pcm(&input));

        // Replays come from the cache with byte ranges.
        let request = actix_web::test::TestRequest::get()
            .insert_header((header::RANGE, "bytes=0-3"))
            .to_http_request();
        let response =
            stream_conversion(&request, &input, &cache, "test", "audio/flac", Vec::new()).await;
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            to_bytes(response.into_body()).await.unwrap().as_ref(),
            b"fLaC"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[actix_web::test]
    async fn dsd64_converts_to_24_bit_176k_flac() {
        if !ffmpeg_available() {
            eprintln!("skipping: ffmpeg is not installed");
            return;
        }
        let directory = scratch();
        let input = directory.join("track.dsf");
        let samples = 2_822_400 / 5;
        std::fs::write(&input, dsd::fixtures::dsf(2_822_400, 2, samples, &[])).unwrap();
        let audio = AudioProperties {
            codec: Some("dsd".into()),
            container: Some("dsf".into()),
            sample_rate: Some(2_822_400),
            bit_depth: Some(1),
            channels: Some(2),
            bitrate_kbps: None,
            lossless: Some(true),
        };
        let browser = OutputCapabilities::from_query(Some("flac,mp3"), None, None, None, None);
        let (cache, _) = convert(&directory, &input, audio, &browser).await;
        let (codec, rate, bits, channels, decoded) = probe(&cache);
        assert_eq!(
            (codec.as_str(), rate, bits, channels),
            ("flac", 176_400, 24, 2)
        );
        let expected = samples / 16;
        assert!(
            decoded.abs_diff(expected) <= 64,
            "{decoded} frames, expected about {expected}"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[actix_web::test]
    async fn dop_streams_serve_exact_byte_ranges() {
        let directory = scratch();
        let input = directory.join("track.dsf");
        let bytes = dsd::fixtures::dsf(2_822_400, 2, 160_000, &[]);
        std::fs::write(&input, &bytes).unwrap();
        let stream = dsd::read(&mut std::io::Cursor::new(&bytes)).unwrap().info;
        let total = dsd::dop_wav_length(&stream);
        let whole =
            dsd::dop_wav_range(&mut std::io::Cursor::new(&bytes), &stream, 0, total - 1).unwrap();

        let request = actix_web::test::TestRequest::get().to_http_request();
        let response = stream_dop(&request, &input, "song").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok()),
            Some(total.to_string().as_str())
        );
        assert_eq!(
            to_bytes(response.into_body()).await.unwrap().as_ref(),
            whole
        );

        let request = actix_web::test::TestRequest::get()
            .insert_header((header::RANGE, "bytes=1000-299999"))
            .to_http_request();
        let response = stream_dop(&request, &input, "song").await;
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            to_bytes(response.into_body()).await.unwrap().as_ref(),
            &whole[1000..300_000.min(whole.len())]
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn cast_receivers_get_hi_res_flac_instead_of_mp3() {
        let song = crate::domain::Song {
            path: "/m/track.wv".into(),
            audio: Some(AudioProperties {
                codec: Some("wavpack".into()),
                container: Some("wavpack".into()),
                sample_rate: Some(192_000),
                bit_depth: Some(24),
                channels: Some(2),
                bitrate_kbps: None,
                lossless: Some(true),
            }),
            ..Default::default()
        };
        let plan = super::cast_delivery_plan(&song);
        assert_eq!(plan.content_type, "audio/flac");
        assert!(matches!(
            plan.delivery,
            Delivery::Pcm {
                sample_rate: 96_000,
                bit_depth: 24,
                ..
            }
        ));
    }
}
