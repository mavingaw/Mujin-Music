//! Library tools, mounted at `/library/tools` (admins only).
//!
//! - `GET  /overview`                 statistics, counts and running jobs
//! - `GET  /jobs`, `GET /jobs/{id}`, `POST /jobs/{id}/cancel`
//! - `GET  /skipped`                  files not imported, with reasons and fixes
//! - `POST /skipped/check`            audits every music folder (job)
//! - `POST /skipped/ignore`           `{paths, ignored}`
//! - `POST /skipped/retry`            `{paths}` reads them again (job)
//! - `GET  /skipped/playlist?path=`   a skipped playlist file's text, to import
//! - `GET  /duplicates`               duplicate groups, multi-copy tracks, hidden copies
//! - `POST /duplicates/resolve`       `{key, action, keep_song_id?, song_ids?}`
//! - `POST /duplicates/resolve-suggested` `{action}` keeps the best copy everywhere it is safe
//! - `POST /duplicates/restore`       `{paths}` brings hidden or merged copies back
//! - `POST /copies/prefer`            `{track_id, path}` picks the copy that plays
//! - `GET  /versions`, `GET /albums/{id}/versions`
//! - `POST /versions/choice`          `{key, preferred_album_id?, hide_others}`
//! - `POST /versions/group`, `POST /versions/ungroup` `{album_ids}`
//! - `GET  /albumizer`, `POST /albumizer`, `DELETE /albumizer/{id}`
//! - `GET  /missing`, `POST /missing/forget` `{paths?, confirm}`
//! - `POST /rescan`, `POST /reidentify`, `POST /reanalyze` `{scope, ids, force?}`
//! - `GET  /songs/{id}/analysis`      waveform, loudness, fingerprint and copies
//!
//! Nothing here writes to, moves or deletes a music file.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use actix_web::{HttpResponse, delete, get, post, web};
use serde::{Deserialize, Serialize};

use crate::api::error::{bad_request, conflict, internal_server_error, not_found};
use crate::domain::{Album, Artist, Song};
use crate::library::state::{LibraryCache, LibraryLifecycle, library_unavailable_response};
use crate::library::tools::albumizer::{self, AlbumSuggestion};
use crate::library::tools::analysis;
use crate::library::tools::duplicates::{DuplicateGroup, find_duplicates};
use crate::library::tools::jobs::{self, JobHandle, JobKind, JobStatus};
use crate::library::tools::layers::{
    album_facts, album_quality, song_quality, track_facts, year_of,
};
use crate::library::tools::quality::{Quality, grade};
use crate::library::tools::skipped::{self, Fix, SkipReason};
use crate::library::tools::stats::{self, AlbumIdentity, LibraryStats};
use crate::library::tools::store;
use crate::library::tools::versions::{VersionChoice, VersionGroup, group_versions};
use crate::persistence::connection::DbPool;
use crate::zones::ZoneService;

const MAX_IDS: usize = 5_000;
const MAX_PATHS: usize = 5_000;
const MAX_TEXT: usize = 512;
const MAX_PLAYLIST_BYTES: u64 = 1024 * 1024;
const ALBUM_TYPES: &[&str] = &["album", "ep", "single", "compilation", "live", "soundtrack"];

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Runs database work off the async threads. Background jobs use this
/// directly; handlers use [`blocking`], which maps errors to a response.
async fn db_task<T: Send + 'static>(
    pool: &DbPool,
    work: impl FnOnce(&mut diesel::SqliteConnection) -> diesel::QueryResult<T> + Send + 'static,
) -> Result<T, String> {
    let pool = pool.clone();
    match web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        work(&mut connection).map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => {
            tracing::error!(%error, "library tools query failed");
            Err(error)
        }
        Err(error) => Err(error.to_string()),
    }
}

async fn blocking<T: Send + 'static>(
    pool: &DbPool,
    work: impl FnOnce(&mut diesel::SqliteConnection) -> diesel::QueryResult<T> + Send + 'static,
) -> Result<T, HttpResponse> {
    db_task(pool, work).await.map_err(|_| {
        internal_server_error(
            "The library database could not be read.",
            "library_tools_failed",
        )
    })
}

async fn cache_or_unavailable(
    lifecycle: &LibraryLifecycle,
) -> Result<Arc<LibraryCache>, HttpResponse> {
    lifecycle
        .cache()
        .await
        .map_err(library_unavailable_response)
}

/// Rebuilds the catalog after a change to the library-tools layer.
async fn rebuild_catalog(lifecycle: &LibraryLifecycle) -> Result<(), HttpResponse> {
    crate::library::storage::refresh_cache().await;
    match LibraryCache::new().await {
        Ok(cache) => {
            lifecycle.set_ready_and_persist(cache).await;
            Ok(())
        }
        Err(error) => {
            tracing::error!(%error, "catalog rebuild after a library tools change failed");
            Err(internal_server_error(
                "The change was saved, but the library could not be rebuilt yet.",
                "catalog_rebuild_failed",
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Catalog lookups
// ---------------------------------------------------------------------------

/// A track as the tools pages show it.
#[derive(Serialize, Clone)]
pub struct SongCard {
    id: String,
    title: String,
    artist: String,
    artist_id: String,
    album_id: String,
    album: String,
    cover_url: String,
    track_number: u16,
    duration: f64,
    path: String,
    quality: Quality,
}

struct Located<'a> {
    artist: &'a Artist,
    album: &'a Album,
    song: &'a Song,
}

fn index_songs(library: &[Artist]) -> HashMap<&str, Located<'_>> {
    let mut index = HashMap::new();
    for artist in library {
        for album in &artist.albums {
            for song in &album.songs {
                index.insert(
                    song.id.as_str(),
                    Located {
                        artist,
                        album,
                        song,
                    },
                );
            }
        }
    }
    index
}

fn card(located: &Located<'_>) -> SongCard {
    SongCard {
        id: located.song.id.clone(),
        title: located.song.name.clone(),
        artist: located.song.artist.clone(),
        artist_id: located.artist.id.clone(),
        album_id: located.album.id.clone(),
        album: located.album.name.clone(),
        cover_url: located.album.cover_url.clone(),
        track_number: located.song.track_number,
        duration: located.song.duration,
        path: located.song.path.clone(),
        quality: song_quality(located.song),
    }
}

/// What an operation covers.
#[derive(Deserialize, Clone)]
#[serde(rename_all = "snake_case", tag = "scope")]
pub enum Scope {
    Library,
    Artists { ids: Vec<String> },
    Albums { ids: Vec<String> },
    Songs { ids: Vec<String> },
}

impl Scope {
    fn ids(&self) -> &[String] {
        match self {
            Self::Library => &[],
            Self::Artists { ids } | Self::Albums { ids } | Self::Songs { ids } => ids,
        }
    }
}

/// `(song id, path, album id)` for every song in scope, and a label.
fn resolve_scope(library: &[Artist], scope: &Scope) -> (Vec<(String, String, String)>, String) {
    let wanted: HashSet<&str> = scope.ids().iter().map(String::as_str).collect();
    let mut songs = Vec::new();
    let mut names = Vec::new();
    for artist in library {
        let artist_match =
            matches!(scope, Scope::Artists { .. }) && wanted.contains(artist.id.as_str());
        if artist_match {
            names.push(artist.name.clone());
        }
        for album in &artist.albums {
            let album_match =
                matches!(scope, Scope::Albums { .. }) && wanted.contains(album.id.as_str());
            if album_match {
                names.push(album.name.clone());
            }
            for song in &album.songs {
                let song_match =
                    matches!(scope, Scope::Songs { .. }) && wanted.contains(song.id.as_str());
                if song_match {
                    names.push(song.name.clone());
                }
                if matches!(scope, Scope::Library) || artist_match || album_match || song_match {
                    songs.push((song.id.clone(), song.path.clone(), album.id.clone()));
                }
            }
        }
    }
    let label = match scope {
        Scope::Library => "Whole library".to_string(),
        _ if names.len() == 1 => names.remove(0),
        _ => format!("{} items", names.len()),
    };
    (songs, label)
}

// ---------------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct Overview {
    stats: LibraryStats,
    skipped: i64,
    skipped_checked_at: Option<String>,
    missing: usize,
    duplicates: usize,
    duplicates_suggested: usize,
    multi_copy_tracks: usize,
    version_groups: usize,
    loose_tracks: usize,
    user_albums: usize,
    last_scan_at: Option<String>,
    jobs: Vec<JobStatus>,
}

struct DbFacts {
    size_bytes: i64,
    identities: HashMap<String, AlbumIdentity>,
    identification_enabled: bool,
    measured: HashSet<String>,
    analyzed: HashSet<String>,
    skipped: i64,
    skipped_checked_at: Option<String>,
    missing: usize,
    multi_copy_tracks: usize,
    user_albums: usize,
    last_scan_at: Option<String>,
    fingerprints: HashMap<String, analysis::Fingerprint>,
    dismissed: HashSet<String>,
    members: HashMap<String, Option<String>>,
    choices: HashMap<String, VersionChoice>,
    release_groups: HashMap<String, String>,
}

fn load_db_facts(connection: &mut diesel::SqliteConnection) -> diesel::QueryResult<DbFacts> {
    let identities = store::identification(connection)?
        .into_iter()
        .map(|row| {
            let identity = stats::album_identity(
                row.task_status.as_deref(),
                row.match_status.as_deref(),
                false,
            );
            (row.album_id, identity)
        })
        .collect();
    let copies = store::multi_copy_tracks(connection)?;
    let multi_copy_tracks = copies
        .iter()
        .map(|copy| copy.track_id.as_str())
        .collect::<HashSet<_>>()
        .len();
    Ok(DbFacts {
        size_bytes: store::library_bytes(connection)?,
        identities,
        identification_enabled: store::identification_enabled(connection),
        measured: store::measured_song_ids(connection)?,
        analyzed: store::analyzed_paths(connection)?,
        skipped: store::skipped_count(connection)?,
        skipped_checked_at: store::last_skipped_check(connection)?,
        missing: store::missing_files(connection)?.len(),
        multi_copy_tracks,
        user_albums: store::user_albums(connection)?.0.len(),
        last_scan_at: store::last_scan_at(connection)?,
        fingerprints: store::fingerprints(connection)?,
        dismissed: store::dismissed_duplicates(connection)?,
        members: store::version_members(connection)?,
        choices: store::version_choices(connection)?,
        release_groups: store::release_groups(connection)?,
    })
}

/// What duplicate detection needs besides the catalog.
struct DuplicateInputs {
    fingerprints: HashMap<String, analysis::Fingerprint>,
    members: HashMap<String, Option<String>>,
    choices: HashMap<String, VersionChoice>,
    release_groups: HashMap<String, String>,
}

fn load_duplicate_inputs(
    connection: &mut diesel::SqliteConnection,
) -> diesel::QueryResult<DuplicateInputs> {
    Ok(DuplicateInputs {
        fingerprints: store::fingerprints(connection)?,
        members: store::version_members(connection)?,
        choices: store::version_choices(connection)?,
        release_groups: store::release_groups(connection)?,
    })
}

/// Duplicate groups, leaving copies on versions of one album to the
/// album versions page.
fn detect_duplicates(
    library: &[Artist],
    fingerprints: &HashMap<String, analysis::Fingerprint>,
    members: &HashMap<String, Option<String>>,
    choices: &HashMap<String, VersionChoice>,
    release_groups: &HashMap<String, String>,
) -> Vec<DuplicateGroup> {
    let clusters: HashMap<String, String> =
        group_versions(&album_facts(library, release_groups), members, choices)
            .into_iter()
            .flat_map(|group| {
                let key = group.key;
                group
                    .versions
                    .into_iter()
                    .map(move |version| (version.album_id, key.clone()))
            })
            .collect();
    find_duplicates(&track_facts(library), fingerprints, &clusters)
}

fn detect_with(library: &[Artist], inputs: &DuplicateInputs) -> Vec<DuplicateGroup> {
    detect_duplicates(
        library,
        &inputs.fingerprints,
        &inputs.members,
        &inputs.choices,
        &inputs.release_groups,
    )
}

fn open_duplicates(library: &[Artist], facts: &DbFacts) -> Vec<DuplicateGroup> {
    detect_duplicates(
        library,
        &facts.fingerprints,
        &facts.members,
        &facts.choices,
        &facts.release_groups,
    )
    .into_iter()
    .filter(|group| !facts.dismissed.contains(&group.key))
    .collect()
}

#[get("/overview")]
async fn overview(lifecycle: web::Data<LibraryLifecycle>, pool: web::Data<DbPool>) -> HttpResponse {
    let cache = match cache_or_unavailable(&lifecycle).await {
        Ok(cache) => cache,
        Err(response) => return response,
    };
    let facts = match blocking(&pool, load_db_facts).await {
        Ok(facts) => facts,
        Err(response) => return response,
    };
    let library = cache.artists.clone();
    let response = web::block(move || {
        let stats = stats::compute(stats::StatsInputs {
            library: &library,
            size_bytes: facts.size_bytes,
            identities: &facts.identities,
            identification_enabled: facts.identification_enabled,
            measured_songs: &facts.measured,
            analyzed_paths: &facts.analyzed,
        });
        let duplicates = open_duplicates(&library, &facts);
        let versions = group_versions(
            &album_facts(&library, &facts.release_groups),
            &facts.members,
            &facts.choices,
        );
        let loose_tracks = albumizer::suggestions(&library)
            .iter()
            .map(|suggestion| suggestion.tracks.len())
            .sum();
        Overview {
            stats,
            skipped: facts.skipped,
            skipped_checked_at: facts.skipped_checked_at,
            missing: facts.missing,
            duplicates_suggested: duplicates.iter().filter(|group| group.suggested).count(),
            duplicates: duplicates.len(),
            multi_copy_tracks: facts.multi_copy_tracks,
            version_groups: versions.len()
                + facts
                    .choices
                    .values()
                    .filter(|choice| choice.hide_others)
                    .count(),
            loose_tracks,
            user_albums: facts.user_albums,
            last_scan_at: facts.last_scan_at,
            jobs: jobs::list(),
        }
    })
    .await;
    match response {
        Ok(overview) => HttpResponse::Ok().json(overview),
        Err(_) => {
            internal_server_error("Statistics could not be computed.", "library_tools_failed")
        }
    }
}

// ---------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------

#[get("/jobs")]
async fn list_jobs() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({ "jobs": jobs::list() }))
}

#[get("/jobs/{id}")]
async fn get_job(id: web::Path<u64>) -> HttpResponse {
    match jobs::get(id.into_inner()) {
        Some(job) => HttpResponse::Ok().json(job),
        None => not_found("No such job.", "job_not_found"),
    }
}

#[post("/jobs/{id}/cancel")]
async fn cancel_job(id: web::Path<u64>) -> HttpResponse {
    let id = id.into_inner();
    if jobs::cancel(id) {
        HttpResponse::Ok().json(jobs::get(id))
    } else {
        not_found("That job is not running.", "job_not_running")
    }
}

fn job_started(handle: &JobHandle) -> HttpResponse {
    HttpResponse::Accepted().json(jobs::get(handle.id()))
}

fn job_busy(status: JobStatus) -> HttpResponse {
    HttpResponse::Conflict().json(serde_json::json!({
        "error": "conflict",
        "code": "job_running",
        "message": format!("{} is already running.", status.title),
        "job": status,
    }))
}

// ---------------------------------------------------------------------------
// Skipped files
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct SkippedItem {
    path: String,
    file_name: String,
    folder: String,
    root_path: String,
    reason: String,
    label: String,
    detail: String,
    extension: String,
    size_bytes: i64,
    ignored: bool,
    imported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    song_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    album_id: Option<String>,
    fix: Fix,
    first_seen_at: String,
}

#[derive(Serialize)]
struct ReasonCount {
    reason: &'static str,
    label: &'static str,
    count: usize,
}

#[get("/skipped")]
async fn list_skipped(
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let (rows, checked_at) = match blocking(&pool, |connection| {
        Ok((
            store::skipped_rows(connection)?,
            store::last_skipped_check(connection)?,
        ))
    })
    .await
    {
        Ok(rows) => rows,
        Err(response) => return response,
    };
    let song_by_path: HashMap<String, (String, String)> = match lifecycle.cache().await {
        Ok(cache) => cache
            .artists
            .iter()
            .flat_map(|artist| artist.albums.iter())
            .flat_map(|album| {
                album
                    .songs
                    .iter()
                    .map(|song| (song.path.clone(), (song.id.clone(), album.id.clone())))
            })
            .collect(),
        Err(_) => HashMap::new(),
    };
    let mut counts: HashMap<SkipReason, usize> = HashMap::new();
    let items: Vec<SkippedItem> = rows
        .into_iter()
        .filter_map(|row| {
            let reason = SkipReason::parse(&row.reason)?;
            if row.ignored == 0 {
                *counts.entry(reason).or_default() += 1;
            }
            let path = Path::new(&row.path);
            Some(SkippedItem {
                file_name: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| row.path.clone()),
                folder: path
                    .parent()
                    .map(|parent| parent.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                fix: skipped::fix_for(reason, &row.extension, &row.path),
                song_id: song_by_path.get(&row.path).map(|(song, _)| song.clone()),
                album_id: song_by_path.get(&row.path).map(|(_, album)| album.clone()),
                label: reason.label().into(),
                imported: reason.imported(),
                reason: row.reason,
                path: row.path,
                root_path: row.root_path,
                detail: row.detail,
                extension: row.extension,
                size_bytes: row.size_bytes,
                ignored: row.ignored != 0,
                first_seen_at: row.first_seen_at,
            })
        })
        .collect();
    let reasons: Vec<ReasonCount> = SkipReason::ALL
        .into_iter()
        .filter_map(|reason| {
            counts.get(&reason).map(|count| ReasonCount {
                reason: reason.as_str(),
                label: reason.label(),
                count: *count,
            })
        })
        .collect();
    HttpResponse::Ok().json(serde_json::json!({
        "items": items,
        "reasons": reasons,
        "checked_at": checked_at,
        "job": jobs::list().into_iter().find(|job| job.kind == JobKind::SkippedCheck && job.state == jobs::JobState::Running),
    }))
}

async fn run_skipped_check(job: JobHandle, pool: DbPool) {
    let loaded = web::block({
        let pool = pool.clone();
        move || {
            let mut connection = pool.get().map_err(|error| error.to_string())?;
            let roots = store::root_paths(&mut connection).map_err(|error| error.to_string())?;
            let imported =
                store::imported_files(&mut connection).map_err(|error| error.to_string())?;
            Ok::<_, String>((roots, imported))
        }
    })
    .await;
    let (roots, imported) = match loaded {
        Ok(Ok(loaded)) => loaded,
        _ => return job.fail("The library database could not be read."),
    };
    let imported = Arc::new(imported);
    let mut found = 0usize;
    let mut seen_total = 0u64;
    for root in &roots {
        if job.is_cancelled() {
            break;
        }
        job.set_current(root.clone());
        if !Path::new(root).is_dir() {
            continue;
        }
        let cancel = job.cancel_flag();
        let imported = imported.clone();
        let root_path = root.clone();
        let (progress_sender, mut progress_receiver) =
            tokio::sync::mpsc::unbounded_channel::<usize>();
        let audit = tokio::task::spawn_blocking(move || {
            skipped::audit_root(
                Path::new(&root_path),
                &imported,
                &|| cancel.load(std::sync::atomic::Ordering::Acquire),
                &mut |seen| {
                    let _ = progress_sender.send(seen);
                },
            )
        });
        tokio::pin!(audit);
        let records = loop {
            tokio::select! {
                seen = progress_receiver.recv() => {
                    if let Some(seen) = seen {
                        job.set_done(seen_total + seen as u64);
                    }
                }
                result = &mut audit => break result,
            }
        };
        let Ok(records) = records else {
            return job.fail("The folder check stopped unexpectedly.");
        };
        if job.is_cancelled() {
            break;
        }
        seen_total += records.len() as u64;
        while let Ok(seen) = progress_receiver.try_recv() {
            job.set_done(seen_total.max(seen as u64));
        }
        found += records.len();
        let root = root.clone();
        let saved = db_task(&pool, move |connection| {
            store::replace_skipped(connection, &root, &records)
        })
        .await;
        if saved.is_err() {
            return job.fail("The findings could not be saved.");
        }
    }
    if job.is_cancelled() {
        return job.finish("");
    }
    let roots_for_prune = roots.clone();
    let _ = db_task(&pool, move |connection| {
        store::prune_skipped_roots(connection, &roots_for_prune)?;
        store::record_skipped_check(connection)
    })
    .await;
    job.finish(match found {
        0 => "Every file in your music folders is in the library.".to_string(),
        1 => "1 file needs attention.".to_string(),
        count => format!("{count} files need attention."),
    });
}

#[post("/skipped/check")]
async fn check_skipped(pool: web::Data<DbPool>) -> HttpResponse {
    match jobs::start(JobKind::SkippedCheck, "All music folders") {
        Ok(job) => {
            let response = job_started(&job);
            tokio::spawn(run_skipped_check(job, pool.get_ref().clone()));
            response
        }
        Err(status) => job_busy(status),
    }
}

#[derive(Deserialize)]
struct PathsBody {
    paths: Vec<String>,
    #[serde(default)]
    ignored: Option<bool>,
}

fn valid_paths(paths: &[String]) -> bool {
    !paths.is_empty()
        && paths.len() <= MAX_PATHS
        && paths
            .iter()
            .all(|path| !path.is_empty() && path.len() <= 32 * 1024)
}

#[post("/skipped/ignore")]
async fn ignore_skipped(body: web::Json<PathsBody>, pool: web::Data<DbPool>) -> HttpResponse {
    let PathsBody { paths, ignored } = body.into_inner();
    if !valid_paths(&paths) {
        return bad_request("Choose between 1 and 5000 files.", "invalid_paths");
    }
    let ignored = ignored.unwrap_or(true);
    match blocking(&pool, move |connection| {
        store::set_skipped_ignored(connection, &paths, ignored)
    })
    .await
    {
        Ok(changed) => HttpResponse::Ok().json(serde_json::json!({ "changed": changed })),
        Err(response) => response,
    }
}

#[post("/skipped/retry")]
async fn retry_skipped(
    body: web::Json<PathsBody>,
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let paths = body.into_inner().paths;
    if !valid_paths(&paths) {
        return bad_request("Choose between 1 and 5000 files.", "invalid_paths");
    }
    let label = if paths.len() == 1 {
        Path::new(&paths[0])
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "1 file".into())
    } else {
        format!("{} files", paths.len())
    };
    match jobs::start(JobKind::Rescan, label) {
        Ok(job) => {
            let response = job_started(&job);
            tokio::spawn(run_rescan(
                job,
                lifecycle,
                pool.get_ref().clone(),
                Some(paths),
                true,
            ));
            response
        }
        Err(status) => job_busy(status),
    }
}

#[derive(Deserialize)]
struct PathQuery {
    path: String,
}

#[get("/skipped/playlist")]
async fn skipped_playlist(query: web::Query<PathQuery>, pool: web::Data<DbPool>) -> HttpResponse {
    let path = query.into_inner().path;
    let lookup = path.clone();
    let row = match blocking(&pool, move |connection| {
        Ok(store::skipped_rows(connection)?
            .into_iter()
            .find(|row| row.path == lookup && row.reason == SkipReason::PlaylistFile.as_str()))
    })
    .await
    {
        Ok(row) => row,
        Err(response) => return response,
    };
    // Only playlist files the audit found inside a music folder are served.
    let Some(row) = row else {
        return not_found(
            "That playlist file is not in the skipped files list.",
            "playlist_not_found",
        );
    };
    let file = Path::new(&row.path);
    match tokio::fs::metadata(file).await {
        Ok(metadata) if metadata.is_file() && metadata.len() <= MAX_PLAYLIST_BYTES => {}
        Ok(_) => {
            return bad_request(
                "The playlist file is too large to import.",
                "playlist_too_large",
            );
        }
        Err(_) => return not_found("The playlist file is no longer there.", "playlist_missing"),
    }
    match tokio::fs::read(file).await {
        Ok(bytes) => HttpResponse::Ok().json(serde_json::json!({
            "file_name": file.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(),
            "content": String::from_utf8_lossy(&bytes),
        })),
        Err(_) => not_found("The playlist file could not be read.", "playlist_unreadable"),
    }
}

// ---------------------------------------------------------------------------
// Duplicates
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct DuplicateView {
    #[serde(flatten)]
    group: DuplicateGroup,
    songs: Vec<SongCard>,
}

#[derive(Serialize)]
struct CopyFile {
    path: String,
    quality: Quality,
    size_bytes: i64,
    duration: f64,
    playing: bool,
}

#[derive(Serialize)]
struct CopyGroup {
    song: SongCard,
    files: Vec<CopyFile>,
    chosen: bool,
}

#[derive(Serialize)]
struct HiddenCopy {
    path: String,
    state: String,
    kept_path: Option<String>,
    song_id: String,
    title: String,
    album: String,
    updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    kept: Option<SongCard>,
}

fn file_title(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

#[get("/duplicates")]
async fn list_duplicates(
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let cache = match cache_or_unavailable(&lifecycle).await {
        Ok(cache) => cache,
        Err(response) => return response,
    };
    let loaded = blocking(&pool, |connection| {
        Ok((
            load_duplicate_inputs(connection)?,
            store::dismissed_duplicates(connection)?,
            store::multi_copy_tracks(connection)?,
            store::file_preferences(connection)?,
            store::visibility_rows(connection)?,
        ))
    })
    .await;
    let (inputs, dismissed, copies, preferences, hidden_rows) = match loaded {
        Ok(loaded) => loaded,
        Err(response) => return response,
    };
    let library = cache.artists.clone();
    let result = web::block(move || {
        let index = index_songs(&library);
        let fingerprints = &inputs.fingerprints;
        let groups: Vec<DuplicateView> = detect_with(&library, &inputs)
            .into_iter()
            .filter(|group| !dismissed.contains(&group.key))
            .map(|group| DuplicateView {
                songs: group
                    .members
                    .iter()
                    .filter_map(|id| index.get(id.as_str()).map(card))
                    .collect(),
                group,
            })
            .collect();
        let chosen: HashSet<String> = preferences.into_iter().map(|row| row.track_id).collect();
        let mut by_track: Vec<(String, Vec<store::CopyRow>)> = Vec::new();
        for copy in copies {
            match by_track.last_mut() {
                Some((track, files)) if *track == copy.track_id => files.push(copy),
                _ => by_track.push((copy.track_id.clone(), vec![copy])),
            }
        }
        let copy_groups: Vec<CopyGroup> = by_track
            .into_iter()
            .filter_map(|(track_id, files)| {
                let located = index.get(track_id.as_str())?;
                let playing = located.song.path.clone();
                let mut files: Vec<CopyFile> = files
                    .into_iter()
                    .map(|file| CopyFile {
                        quality: grade(file.audio().as_ref(), &file.extension),
                        playing: file.path == playing,
                        path: file.path,
                        size_bytes: file.size_bytes,
                        duration: file.duration_seconds,
                    })
                    .collect();
                files.sort_by_key(|file| std::cmp::Reverse(file.quality.score));
                Some(CopyGroup {
                    chosen: chosen.contains(&track_id),
                    song: card(located),
                    files,
                })
            })
            .collect();
        let by_path: HashMap<&str, &Located<'_>> = index
            .values()
            .map(|located| (located.song.path.as_str(), located))
            .collect();
        let hidden: Vec<HiddenCopy> = hidden_rows
            .into_iter()
            .map(|row| HiddenCopy {
                title: file_title(&row.path),
                album: Path::new(&row.path)
                    .parent()
                    .and_then(|parent| parent.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                kept: row
                    .kept_path
                    .as_deref()
                    .and_then(|kept| by_path.get(kept))
                    .map(|located| card(located)),
                path: row.path,
                state: row.state,
                kept_path: row.kept_path,
                song_id: row.song_id,
                updated_at: row.updated_at,
            })
            .collect();
        let printed = index
            .values()
            .filter(|located| fingerprints.contains_key(&located.song.path))
            .count();
        let method = fingerprints
            .values()
            .next()
            .map(|print| print.kind().as_str());
        serde_json::json!({
            "groups": groups,
            "copies": copy_groups,
            "hidden": hidden,
            "dismissed": dismissed.len(),
            "fingerprints": { "tracks": printed, "total": index.len(), "method": method },
        })
    })
    .await;
    match result {
        Ok(body) => HttpResponse::Ok().json(body),
        Err(_) => {
            internal_server_error("Duplicates could not be computed.", "library_tools_failed")
        }
    }
}

#[derive(Deserialize)]
struct ResolveBody {
    key: String,
    /// `merge` keeps one copy and moves likes and playlist entries to it;
    /// `hide` hides the listed copies; `dismiss` marks "not duplicates".
    action: String,
    keep_song_id: Option<String>,
    song_ids: Option<Vec<String>>,
}

/// Paths and ids of a group's members, best first.
fn group_members(library: &[Artist], group: &DuplicateGroup) -> Vec<(String, String)> {
    let index = index_songs(library);
    group
        .members
        .iter()
        .filter_map(|id| {
            index
                .get(id.as_str())
                .map(|located| (id.clone(), located.song.path.clone()))
        })
        .collect()
}

fn merge_writes(
    connection: &mut diesel::SqliteConnection,
    members: &[(String, String)],
    keep: &(String, String),
) -> diesel::QueryResult<usize> {
    let mut merged = 0;
    for (song_id, path) in members {
        if path == &keep.1 {
            continue;
        }
        store::set_visibility(connection, path, "merged", Some(&keep.1), song_id)?;
        store::carry_references(connection, song_id, &keep.0)?;
        merged += 1;
    }
    Ok(merged)
}

#[post("/duplicates/resolve")]
async fn resolve_duplicate(
    body: web::Json<ResolveBody>,
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let body = body.into_inner();
    if body.key.is_empty() || body.key.len() > 64 {
        return bad_request("Unknown duplicate group.", "invalid_group");
    }
    if body.action == "dismiss" {
        let key = body.key.clone();
        return match blocking(&pool, move |connection| {
            store::set_duplicate_dismissed(connection, &key, true)
        })
        .await
        {
            Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "dismissed": body.key })),
            Err(response) => response,
        };
    }
    let cache = match cache_or_unavailable(&lifecycle).await {
        Ok(cache) => cache,
        Err(response) => return response,
    };
    let inputs = match blocking(&pool, load_duplicate_inputs).await {
        Ok(inputs) => inputs,
        Err(response) => return response,
    };
    let Some(group) = detect_with(&cache.artists, &inputs)
        .into_iter()
        .find(|group| group.key == body.key)
    else {
        return conflict(
            "This group changed since the page loaded. Reload and try again.",
            "duplicate_group_changed",
        );
    };
    let members = group_members(&cache.artists, &group);
    let changed = match body.action.as_str() {
        "merge" => {
            let keep_id = body
                .keep_song_id
                .clone()
                .unwrap_or_else(|| group.best.clone());
            let Some(keep) = members.iter().find(|(id, _)| *id == keep_id).cloned() else {
                return bad_request("Keep one of the group's copies.", "invalid_keep");
            };
            blocking(&pool, move |connection| {
                use diesel::Connection;
                connection.transaction(|connection| merge_writes(connection, &members, &keep))
            })
            .await
        }
        "hide" => {
            let hide: HashSet<String> = body
                .song_ids
                .clone()
                .unwrap_or_default()
                .into_iter()
                .collect();
            if hide.is_empty()
                || hide.len() >= members.len()
                || members.iter().filter(|(id, _)| hide.contains(id)).count() != hide.len()
            {
                return bad_request(
                    "Hide some copies of the group, keeping at least one.",
                    "invalid_hide",
                );
            }
            blocking(&pool, move |connection| {
                let mut hidden = 0;
                for (song_id, path) in members.iter().filter(|(id, _)| hide.contains(id)) {
                    store::set_visibility(connection, path, "hidden", None, song_id)?;
                    hidden += 1;
                }
                Ok(hidden)
            })
            .await
        }
        _ => return bad_request("Choose merge, hide or dismiss.", "invalid_action"),
    };
    match changed {
        Ok(changed) => match rebuild_catalog(&lifecycle).await {
            Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "changed": changed })),
            Err(response) => response,
        },
        Err(response) => response,
    }
}

/// A group's `(song id, path)` members and the member kept.
type MergePlan = (Vec<(String, String)>, (String, String));

#[derive(Deserialize)]
struct BulkResolveBody {
    action: String,
}

#[post("/duplicates/resolve-suggested")]
async fn resolve_suggested(
    body: web::Json<BulkResolveBody>,
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let merge = match body.action.as_str() {
        "merge" => true,
        "hide" => false,
        _ => return bad_request("Choose merge or hide.", "invalid_action"),
    };
    let cache = match cache_or_unavailable(&lifecycle).await {
        Ok(cache) => cache,
        Err(response) => return response,
    };
    let loaded = blocking(&pool, |connection| {
        Ok((
            load_duplicate_inputs(connection)?,
            store::dismissed_duplicates(connection)?,
        ))
    })
    .await;
    let (inputs, dismissed) = match loaded {
        Ok(loaded) => loaded,
        Err(response) => return response,
    };
    let plans: Vec<MergePlan> = detect_with(&cache.artists, &inputs)
        .into_iter()
        .filter(|group| group.suggested && !dismissed.contains(&group.key))
        .filter_map(|group| {
            let members = group_members(&cache.artists, &group);
            let keep = members.iter().find(|(id, _)| *id == group.best).cloned()?;
            Some((members, keep))
        })
        .collect();
    let groups = plans.len();
    let changed = blocking(&pool, move |connection| {
        use diesel::Connection;
        connection.transaction(|connection| {
            let mut changed = 0;
            for (members, keep) in &plans {
                if merge {
                    changed += merge_writes(connection, members, keep)?;
                } else {
                    for (song_id, path) in members.iter().filter(|(_, path)| *path != keep.1) {
                        store::set_visibility(connection, path, "hidden", None, song_id)?;
                        changed += 1;
                    }
                }
            }
            Ok(changed)
        })
    })
    .await;
    match changed {
        Ok(changed) => match rebuild_catalog(&lifecycle).await {
            Ok(()) => {
                HttpResponse::Ok().json(serde_json::json!({ "groups": groups, "changed": changed }))
            }
            Err(response) => response,
        },
        Err(response) => response,
    }
}

#[post("/duplicates/restore")]
async fn restore_duplicates(
    body: web::Json<PathsBody>,
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let paths = body.into_inner().paths;
    if !valid_paths(&paths) {
        return bad_request("Choose between 1 and 5000 files.", "invalid_paths");
    }
    let restored = blocking(&pool, move |connection| {
        let mut restored = 0;
        for path in &paths {
            restored += store::clear_visibility(connection, path)?;
        }
        Ok(restored)
    })
    .await;
    match restored {
        Ok(restored) => match rebuild_catalog(&lifecycle).await {
            Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "restored": restored })),
            Err(response) => response,
        },
        Err(response) => response,
    }
}

#[derive(Deserialize)]
struct PreferCopyBody {
    track_id: String,
    /// `None` returns to the highest quality copy.
    path: Option<String>,
}

#[post("/copies/prefer")]
async fn prefer_copy(
    body: web::Json<PreferCopyBody>,
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let PreferCopyBody { track_id, path } = body.into_inner();
    if track_id.is_empty() || track_id.len() > 128 {
        return bad_request("Unknown track.", "invalid_track");
    }
    let saved = blocking(&pool, move |connection| {
        if let Some(path) = &path
            && !store::track_copies(connection, &track_id)?
                .iter()
                .any(|copy| copy.path == *path)
        {
            return Ok(false);
        }
        store::set_file_preference(connection, &track_id, path.as_deref())?;
        Ok(true)
    })
    .await;
    match saved {
        Ok(true) => match rebuild_catalog(&lifecycle).await {
            Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "saved": true })),
            Err(response) => response,
        },
        Ok(false) => bad_request("That file is not a copy of this track.", "invalid_copy"),
        Err(response) => response,
    }
}

// ---------------------------------------------------------------------------
// Album versions
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
struct VersionAlbum {
    album_id: String,
    title: String,
    edition: String,
    preferred: bool,
    hidden: bool,
    year: String,
    track_count: usize,
    duration: f64,
    quality: Quality,
    mixed: bool,
    cover_url: String,
    artist_id: String,
}

#[derive(Serialize, Clone)]
struct VersionGroupView {
    key: String,
    title: String,
    artist_name: String,
    preferred: String,
    chosen: bool,
    hide_others: bool,
    manual: bool,
    versions: Vec<VersionAlbum>,
}

/// Version groups over the catalog including hidden versions, cached per
/// catalog revision (the album page asks on every visit).
type VersionsCache = Mutex<Option<(u64, Arc<Vec<VersionGroupView>>)>>;
static VERSIONS_CACHE: OnceLock<VersionsCache> = OnceLock::new();

async fn version_groups(
    lifecycle: &LibraryLifecycle,
    pool: &DbPool,
) -> Result<Arc<Vec<VersionGroupView>>, HttpResponse> {
    let revision = lifecycle.catalog_revision();
    let cache = VERSIONS_CACHE.get_or_init(|| Mutex::new(None));
    if let Some((cached_revision, groups)) =
        cache.lock().unwrap_or_else(|p| p.into_inner()).as_ref()
        && *cached_revision == revision
    {
        return Ok(groups.clone());
    }
    let library = crate::library::storage::fetch_library_with_all_versions()
        .await
        .map_err(|error: BoxError| {
            tracing::error!(%error, "could not load album versions");
            internal_server_error(
                "Album versions could not be loaded.",
                "library_tools_failed",
            )
        })?;
    let (members, choices, release_groups) = blocking(pool, |connection| {
        Ok((
            store::version_members(connection)?,
            store::version_choices(connection)?,
            store::release_groups(connection)?,
        ))
    })
    .await?;
    let groups = web::block(move || {
        let albums: HashMap<&str, (&Artist, &Album)> = library
            .iter()
            .flat_map(|artist| {
                artist
                    .albums
                    .iter()
                    .map(move |album| (album.id.as_str(), (artist, album)))
            })
            .collect();
        let grouped: Vec<VersionGroup> =
            group_versions(&album_facts(&library, &release_groups), &members, &choices);
        grouped
            .into_iter()
            .map(|group| VersionGroupView {
                versions: group
                    .versions
                    .iter()
                    .filter_map(|version| {
                        let (artist, album) = albums.get(version.album_id.as_str())?;
                        let (quality, mixed) = album_quality(album);
                        Some(VersionAlbum {
                            album_id: album.id.clone(),
                            title: album.name.clone(),
                            edition: version.edition.clone(),
                            preferred: version.preferred,
                            hidden: group.hide_others && !version.preferred,
                            year: year_of(&album.first_release_date),
                            track_count: album.songs.len(),
                            duration: album.songs.iter().map(|song| song.duration).sum(),
                            quality,
                            mixed,
                            cover_url: album.cover_url.clone(),
                            artist_id: artist.id.clone(),
                        })
                    })
                    .collect(),
                key: group.key,
                title: group.title,
                artist_name: group.artist_name,
                preferred: group.preferred,
                chosen: group.chosen,
                hide_others: group.hide_others,
                manual: group.manual,
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|_| {
        internal_server_error(
            "Album versions could not be grouped.",
            "library_tools_failed",
        )
    })?;
    let groups = Arc::new(groups);
    *cache.lock().unwrap_or_else(|p| p.into_inner()) = Some((revision, groups.clone()));
    Ok(groups)
}

fn forget_versions_cache() {
    if let Some(cache) = VERSIONS_CACHE.get() {
        *cache.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
}

#[get("/versions")]
async fn list_versions(
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    match version_groups(&lifecycle, &pool).await {
        Ok(groups) => HttpResponse::Ok().json(serde_json::json!({ "groups": *groups })),
        Err(response) => response,
    }
}

#[get("/albums/{id}/versions")]
async fn album_versions(
    id: web::Path<String>,
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let id = id.into_inner();
    match version_groups(&lifecycle, &pool).await {
        Ok(groups) => {
            let group = groups
                .iter()
                .find(|group| group.versions.iter().any(|version| version.album_id == id));
            HttpResponse::Ok().json(serde_json::json!({ "group": group }))
        }
        Err(response) => response,
    }
}

#[derive(Deserialize)]
struct VersionChoiceBody {
    key: String,
    /// `None` returns the group to automatic (highest quality, all shown).
    preferred_album_id: Option<String>,
    #[serde(default)]
    hide_others: bool,
}

#[post("/versions/choice")]
async fn choose_version(
    body: web::Json<VersionChoiceBody>,
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let body = body.into_inner();
    let groups = match version_groups(&lifecycle, &pool).await {
        Ok(groups) => groups,
        Err(response) => return response,
    };
    let Some(group) = groups.iter().find(|group| group.key == body.key) else {
        return conflict(
            "This group changed since the page loaded. Reload and try again.",
            "version_group_changed",
        );
    };
    if let Some(preferred) = &body.preferred_album_id
        && !group
            .versions
            .iter()
            .any(|version| version.album_id == *preferred)
    {
        return bad_request("Choose one of the group's versions.", "invalid_version");
    }
    let key = body.key.clone();
    let choice = body
        .preferred_album_id
        .map(|preferred_album_id| VersionChoice {
            preferred_album_id,
            hide_others: body.hide_others,
        });
    let saved = blocking(&pool, move |connection| {
        store::set_version_choice(connection, &key, choice.as_ref())
    })
    .await;
    if let Err(response) = saved {
        return response;
    }
    forget_versions_cache();
    match rebuild_catalog(&lifecycle).await {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "saved": true })),
        Err(response) => response,
    }
}

#[derive(Deserialize)]
struct AlbumIdsBody {
    album_ids: Vec<String>,
}

fn valid_ids(ids: &[String]) -> bool {
    !ids.is_empty()
        && ids.len() <= MAX_IDS
        && ids.iter().all(|id| !id.is_empty() && id.len() <= 128)
}

#[post("/versions/group")]
async fn group_albums(
    body: web::Json<AlbumIdsBody>,
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let ids = body.into_inner().album_ids;
    if ids.len() < 2 || !valid_ids(&ids) {
        return bad_request("Choose at least two albums to group.", "invalid_albums");
    }
    let key = format!("manual-{}", uuid::Uuid::new_v4().simple());
    let saved = blocking(&pool, move |connection| {
        use diesel::Connection;
        connection.transaction(|connection| {
            for id in &ids {
                store::set_version_member(connection, id, Some(&key))?;
            }
            Ok(key)
        })
    })
    .await;
    forget_versions_cache();
    match saved {
        Ok(key) => match rebuild_catalog(&lifecycle).await {
            Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "key": key })),
            Err(response) => response,
        },
        Err(response) => response,
    }
}

#[post("/versions/ungroup")]
async fn ungroup_albums(
    body: web::Json<AlbumIdsBody>,
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let ids = body.into_inner().album_ids;
    if !valid_ids(&ids) {
        return bad_request(
            "Choose the albums to take out of their group.",
            "invalid_albums",
        );
    }
    let saved = blocking(&pool, move |connection| {
        for id in &ids {
            // Out of any group, automatic or manual.
            store::set_version_member(connection, id, None)?;
        }
        Ok(())
    })
    .await;
    forget_versions_cache();
    match saved {
        Ok(()) => match rebuild_catalog(&lifecycle).await {
            Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "saved": true })),
            Err(response) => response,
        },
        Err(response) => response,
    }
}

// ---------------------------------------------------------------------------
// Albumizer
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct UserAlbumView {
    id: String,
    title: String,
    artist_id: Option<String>,
    artist_name: String,
    year: String,
    primary_type: String,
    cover_url: String,
    tracks: Vec<SongCard>,
}

#[get("/albumizer")]
async fn list_albumizer(
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let cache = match cache_or_unavailable(&lifecycle).await {
        Ok(cache) => cache,
        Err(response) => return response,
    };
    let (albums, _) = match blocking(&pool, store::user_albums).await {
        Ok(rows) => rows,
        Err(response) => return response,
    };
    let library = cache.artists.clone();
    let result = web::block(move || {
        let index = index_songs(&library);
        let catalog_albums: HashMap<&str, &Album> = library
            .iter()
            .flat_map(|artist| artist.albums.iter())
            .map(|album| (album.id.as_str(), album))
            .collect();
        let made: Vec<UserAlbumView> = albums
            .into_iter()
            .map(|row| {
                let album = catalog_albums.get(row.id.as_str());
                UserAlbumView {
                    cover_url: album
                        .map(|album| album.cover_url.clone())
                        .unwrap_or_default(),
                    tracks: album
                        .map(|album| {
                            album
                                .songs
                                .iter()
                                .filter_map(|song| index.get(song.id.as_str()).map(card))
                                .collect()
                        })
                        .unwrap_or_default(),
                    id: row.id,
                    title: row.title,
                    artist_id: row.artist_id,
                    artist_name: row.artist_name,
                    year: row.year,
                    primary_type: row.primary_type,
                }
            })
            .collect();
        let suggestions: Vec<AlbumSuggestion> = albumizer::suggestions(&library);
        serde_json::json!({ "albums": made, "suggestions": suggestions })
    })
    .await;
    match result {
        Ok(body) => HttpResponse::Ok().json(body),
        Err(_) => {
            internal_server_error("Loose tracks could not be listed.", "library_tools_failed")
        }
    }
}

#[derive(Deserialize)]
struct AlbumizeBody {
    id: Option<String>,
    title: String,
    artist_name: String,
    artist_id: Option<String>,
    #[serde(default)]
    year: String,
    #[serde(default)]
    primary_type: Option<String>,
    cover_song_id: Option<String>,
    /// Tracks in album order.
    song_ids: Vec<String>,
}

#[post("/albumizer")]
async fn albumize(
    body: web::Json<AlbumizeBody>,
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let body = body.into_inner();
    let title = body.title.trim().to_string();
    let artist_name = body.artist_name.trim().to_string();
    if title.is_empty()
        || title.len() > MAX_TEXT
        || artist_name.is_empty()
        || artist_name.len() > MAX_TEXT
    {
        return bad_request("Give the album a title and an artist.", "invalid_album");
    }
    let year = body.year.trim().to_string();
    if !(year.is_empty() || year.len() == 4 && year.chars().all(|c| c.is_ascii_digit())) {
        return bad_request("The year has four digits, like 1997.", "invalid_year");
    }
    let primary_type = body
        .primary_type
        .unwrap_or_else(|| "album".into())
        .to_ascii_lowercase();
    if !ALBUM_TYPES.contains(&primary_type.as_str()) {
        return bad_request(
            "Choose album, EP, single, compilation, live or soundtrack.",
            "invalid_type",
        );
    }
    if body.song_ids.is_empty() || body.song_ids.len() > 999 || !valid_ids(&body.song_ids) {
        return bad_request("Choose between 1 and 999 tracks.", "invalid_tracks");
    }
    if body
        .id
        .as_deref()
        .is_some_and(|id| !id.starts_with("ua-") || id.len() > 64)
    {
        return bad_request("Unknown album.", "invalid_album");
    }
    let cache = match cache_or_unavailable(&lifecycle).await {
        Ok(cache) => cache,
        Err(response) => return response,
    };
    let index = index_songs(&cache.artists);
    let mut seen = HashSet::new();
    let mut tracks = Vec::new();
    for id in &body.song_ids {
        let Some(located) = index.get(id.as_str()) else {
            return bad_request(
                "A chosen track is no longer in the library.",
                "unknown_track",
            );
        };
        if seen.insert(located.song.path.clone()) {
            tracks.push((located.song.path.clone(), 1));
        }
    }
    let cover_path = body
        .cover_song_id
        .as_deref()
        .and_then(|id| index.get(id))
        .map(|located| located.album.cover_url.clone())
        .unwrap_or_default();
    let artist_id = body
        .artist_id
        .filter(|id| cache.artists.iter().any(|artist| artist.id == *id));
    let row = store::UserAlbumRow {
        id: body
            .id
            .unwrap_or_else(|| format!("ua-{}", &uuid::Uuid::new_v4().simple().to_string()[..20])),
        title,
        artist_id,
        artist_name,
        year,
        primary_type,
        cover_path,
    };
    drop(index);
    let album_id = row.id.clone();
    let saved = blocking(&pool, move |connection| {
        store::save_user_album(connection, &row, &tracks)
    })
    .await;
    if let Err(response) = saved {
        return response;
    }
    forget_versions_cache();
    match rebuild_catalog(&lifecycle).await {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "album_id": album_id })),
        Err(response) => response,
    }
}

#[delete("/albumizer/{id}")]
async fn unalbumize(
    id: web::Path<String>,
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let id = id.into_inner();
    if !id.starts_with("ua-") || id.len() > 64 {
        return bad_request("Unknown album.", "invalid_album");
    }
    match blocking(&pool, move |connection| {
        store::delete_user_album(connection, &id)
    })
    .await
    {
        Ok(0) => not_found("No such album.", "album_not_found"),
        Ok(_) => {
            forget_versions_cache();
            match rebuild_catalog(&lifecycle).await {
                Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "deleted": true })),
                Err(response) => response,
            }
        }
        Err(response) => response,
    }
}

// ---------------------------------------------------------------------------
// Missing files
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct MissingFile {
    path: String,
    root_path: String,
    title: String,
    album: String,
    artist: String,
    size_bytes: i64,
    last_seen_at: String,
    /// False while the folder holding it cannot be reached (a drive or
    /// share is offline): such files are kept, never forgotten.
    root_reachable: bool,
    /// The file is back at its path; the next scan restores it.
    present_again: bool,
}

fn root_reachable(root: &str) -> bool {
    std::fs::read_dir(root).is_ok_and(|mut entries| entries.next().is_some())
}

async fn missing_view(
    pool: &DbPool,
) -> Result<(Vec<MissingFile>, Vec<serde_json::Value>), HttpResponse> {
    let (rows, roots) = blocking(pool, |connection| {
        Ok((
            store::missing_files(connection)?,
            store::root_paths(connection)?,
        ))
    })
    .await?;
    web::block(move || {
        let reachable: HashMap<String, bool> = roots
            .iter()
            .map(|root| (root.clone(), root_reachable(root)))
            .collect();
        let files = rows
            .into_iter()
            .map(|row| {
                let artists: Vec<String> = row
                    .artists_json
                    .as_deref()
                    .and_then(|json| serde_json::from_str(json).ok())
                    .unwrap_or_default();
                MissingFile {
                    title: row
                        .title
                        .filter(|title| !title.is_empty())
                        .unwrap_or_else(|| file_title(&row.path)),
                    album: row.album.unwrap_or_default(),
                    artist: artists.join(", "),
                    root_reachable: reachable.get(&row.root_path).copied().unwrap_or(false),
                    present_again: Path::new(&row.path).exists(),
                    size_bytes: row.size_bytes,
                    last_seen_at: row.updated_at,
                    root_path: row.root_path,
                    path: row.path,
                }
            })
            .collect();
        let roots = reachable
            .into_iter()
            .map(|(path, reachable)| serde_json::json!({ "path": path, "reachable": reachable }))
            .collect();
        (files, roots)
    })
    .await
    .map_err(|_| {
        internal_server_error("Missing files could not be listed.", "library_tools_failed")
    })
}

#[get("/missing")]
async fn list_missing(pool: web::Data<DbPool>) -> HttpResponse {
    match missing_view(&pool).await {
        Ok((files, roots)) => {
            let forgettable = files
                .iter()
                .filter(|file| file.root_reachable && !file.present_again)
                .count();
            HttpResponse::Ok().json(serde_json::json!({
                "files": files,
                "roots": roots,
                "forgettable": forgettable,
            }))
        }
        Err(response) => response,
    }
}

#[derive(Deserialize)]
struct ForgetBody {
    paths: Option<Vec<String>>,
    /// The number of files the user agreed to forget; must match.
    confirm: usize,
}

#[post("/missing/forget")]
async fn forget_missing(
    body: web::Json<ForgetBody>,
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let ForgetBody { paths, confirm } = body.into_inner();
    if paths.as_ref().is_some_and(|paths| !valid_paths(paths)) {
        return bad_request("Choose between 1 and 5000 files.", "invalid_paths");
    }
    let files = match missing_view(&pool).await {
        Ok((files, _)) => files,
        Err(response) => return response,
    };
    let wanted: Option<HashSet<String>> = paths.map(|paths| paths.into_iter().collect());
    let forgettable: Vec<String> = files
        .into_iter()
        .filter(|file| file.root_reachable && !file.present_again)
        .filter(|file| {
            wanted
                .as_ref()
                .is_none_or(|wanted| wanted.contains(&file.path))
        })
        .map(|file| file.path)
        .collect();
    if forgettable.is_empty() {
        return bad_request(
            "Nothing to forget: files on an offline drive are always kept.",
            "nothing_to_forget",
        );
    }
    if forgettable.len() != confirm {
        return conflict(
            format!(
                "{} files can be forgotten now, not {confirm}. Review the list again.",
                forgettable.len()
            ),
            "confirmation_mismatch",
        );
    }
    match blocking(&pool, move |connection| {
        store::forget_missing(connection, &forgettable)
    })
    .await
    {
        Ok(removed) => {
            forget_versions_cache();
            match rebuild_catalog(&lifecycle).await {
                Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "forgotten": removed })),
                Err(response) => response,
            }
        }
        Err(response) => response,
    }
}

// ---------------------------------------------------------------------------
// Rescan, re-identify, re-analyze
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct OperationBody {
    #[serde(flatten)]
    scope: Scope,
    #[serde(default)]
    force: bool,
}

fn validate_scope(scope: &Scope) -> Option<HttpResponse> {
    match scope {
        Scope::Library => None,
        _ if valid_ids(scope.ids()) => None,
        _ => Some(bad_request(
            "Choose between 1 and 5000 items.",
            "invalid_scope",
        )),
    }
}

async fn run_rescan(
    job: JobHandle,
    lifecycle: web::Data<LibraryLifecycle>,
    pool: DbPool,
    paths: Option<Vec<String>>,
    recheck_skipped: bool,
) {
    job.set_current("Marking files to read again");
    let count = paths.as_ref().map(Vec::len);
    let marked = db_task(&pool, move |connection| match &paths {
        Some(paths) => store::mark_files_for_reread(connection, paths),
        None => store::mark_library_for_reread(connection),
    })
    .await;
    let Ok(marked) = marked else {
        return job.fail("The library database could not be updated.");
    };
    let total = count.unwrap_or(marked) as u64;
    job.set_total(total);
    if job.is_cancelled() {
        return job.finish("");
    }
    job.set_current("Reading tags, stream headers and artwork");
    match crate::api::library::refresh_for_tools(lifecycle, "Rescanning files from library tools.")
        .await
    {
        Ok(failures) => {
            job.set_done(total);
            forget_versions_cache();
            if recheck_skipped
                && let Ok(check) = jobs::start(JobKind::SkippedCheck, "All music folders")
            {
                tokio::spawn(run_skipped_check(check, pool));
            }
            if failures == 0 {
                job.finish(match total {
                    1 => "1 file read again.".to_string(),
                    count => format!("{count} files read again."),
                });
            } else {
                job.finish(format!(
                    "Done, but {failures} music folder(s) could not be scanned."
                ));
            }
        }
        Err(message) => job.fail(message),
    }
}

#[post("/rescan")]
async fn rescan(
    body: web::Json<OperationBody>,
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let OperationBody { scope, .. } = body.into_inner();
    if let Some(response) = validate_scope(&scope) {
        return response;
    }
    let (paths, label) = match &scope {
        Scope::Library => (None, "Whole library".to_string()),
        _ => {
            let cache = match cache_or_unavailable(&lifecycle).await {
                Ok(cache) => cache,
                Err(response) => return response,
            };
            let (songs, label) = resolve_scope(&cache.artists, &scope);
            if songs.is_empty() {
                return not_found("Nothing in the library matches.", "scope_empty");
            }
            // Every copy of each track, not only the one that plays.
            let track_ids: Vec<String> = songs.iter().map(|(id, _, _)| id.clone()).collect();
            let mut paths = match blocking(&pool, move |connection| {
                store::paths_for_tracks(connection, &track_ids)
            })
            .await
            {
                Ok(paths) => paths,
                Err(response) => return response,
            };
            paths.extend(songs.into_iter().map(|(_, path, _)| path));
            paths.sort_unstable();
            paths.dedup();
            (Some(paths), label)
        }
    };
    match jobs::start(JobKind::Rescan, label) {
        Ok(job) => {
            let response = job_started(&job);
            tokio::spawn(run_rescan(
                job,
                lifecycle,
                pool.get_ref().clone(),
                paths,
                false,
            ));
            response
        }
        Err(status) => job_busy(status),
    }
}

async fn run_reidentify(job: JobHandle, pool: DbPool, album_ids: Option<Vec<String>>) {
    job.set_current("Queueing lookups");
    let queued = db_task(&pool, move |connection| {
        let ids = match album_ids {
            Some(ids) => ids,
            None => store::identification(connection)?
                .into_iter()
                .map(|row| row.album_id)
                .collect(),
        };
        for id in &ids {
            crate::enrichment::store::refresh_entity(connection, "album", id)?;
        }
        Ok(ids)
    })
    .await;
    let Ok(ids) = queued else {
        return job.fail("The lookups could not be queued.");
    };
    job.set_total(ids.len() as u64);
    crate::enrichment::worker::wake().notify_all();
    job.set_current("Matching albums with MusicBrainz");
    let started = std::time::Instant::now();
    let ids = Arc::new(ids);
    loop {
        if job.is_cancelled() {
            return job.finish("");
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let watched = ids.clone();
        let states = db_task(&pool, move |connection| {
            let rows = store::identification(connection)?;
            let wanted: HashSet<&str> = watched.iter().map(String::as_str).collect();
            Ok(rows
                .into_iter()
                .filter(|row| wanted.contains(row.album_id.as_str()))
                .map(|row| {
                    stats::album_identity(
                        row.task_status.as_deref(),
                        row.match_status.as_deref(),
                        false,
                    )
                })
                .collect::<Vec<_>>())
        })
        .await;
        let Ok(states) = states else {
            return job.fail("Progress could not be read.");
        };
        let waiting = states
            .iter()
            .filter(|state| **state == AlbumIdentity::Awaiting)
            .count();
        let identified = states
            .iter()
            .filter(|state| **state == AlbumIdentity::Identified)
            .count();
        job.set_done((states.len() - waiting) as u64);
        if waiting == 0 {
            forget_versions_cache();
            return job.finish(format!(
                "{identified} of {} album(s) matched a MusicBrainz release.",
                states.len()
            ));
        }
        if started.elapsed() > std::time::Duration::from_secs(60 * 60) {
            return job.finish(format!(
                "{waiting} album(s) are still waiting; they keep going in the background."
            ));
        }
    }
}

#[post("/reidentify")]
async fn reidentify(
    body: web::Json<OperationBody>,
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let OperationBody { scope, .. } = body.into_inner();
    if let Some(response) = validate_scope(&scope) {
        return response;
    }
    match blocking(&pool, |connection| {
        Ok(store::identification_enabled(connection))
    })
    .await
    {
        Ok(true) => {}
        Ok(false) => {
            return conflict(
                "Online metadata is off. Turn it on in Settings → Library to identify albums.",
                "identification_disabled",
            );
        }
        Err(response) => return response,
    }
    let (album_ids, label) = match &scope {
        Scope::Library => (None, "Whole library".to_string()),
        _ => {
            let cache = match cache_or_unavailable(&lifecycle).await {
                Ok(cache) => cache,
                Err(response) => return response,
            };
            let (songs, label) = resolve_scope(&cache.artists, &scope);
            let mut albums: Vec<String> = songs.into_iter().map(|(_, _, album)| album).collect();
            albums.sort_unstable();
            albums.dedup();
            // Albums made with the Albumizer have no release to look up.
            albums.retain(|album| !album.starts_with("ua-"));
            if albums.is_empty() {
                return not_found("Nothing in the library matches.", "scope_empty");
            }
            (Some(albums), label)
        }
    };
    match jobs::start(JobKind::Reidentify, label) {
        Ok(job) => {
            let response = job_started(&job);
            tokio::spawn(run_reidentify(job, pool.get_ref().clone(), album_ids));
            response
        }
        Err(status) => job_busy(status),
    }
}

/// `size:mtime_ns` of a file, to notice when it changed.
pub(crate) fn file_version(path: &Path) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    Some(format!("{}:{modified}", metadata.len()))
}

async fn analyze_one(
    zones: &ZoneService,
    pool: &DbPool,
    song_id: &str,
    path: &str,
    force: bool,
) -> bool {
    let file = Path::new(path);
    let Some(version) = file_version(file) else {
        return false;
    };
    let stored = {
        let lookup = path.to_string();
        db_task(pool, move |connection| {
            store::analysis_for(connection, &lookup)
        })
        .await
        .ok()
        .flatten()
    };
    let up_to_date = stored
        .as_ref()
        .is_some_and(|row| row.file_version == version && row.error.is_none());
    if force {
        zones.loudness().forget(song_id);
    } else if up_to_date && zones.loudness().cached(song_id, file).is_some() {
        return true;
    }
    let loudness = zones.loudness().track(song_id, file);
    let analysis = async {
        if up_to_date && !force {
            None
        } else {
            Some(analysis::analyze_file(file).await)
        }
    };
    let (loudness, analysis) = tokio::join!(loudness, analysis);
    let saved = match analysis {
        None => true,
        Some(result) => {
            let path = path.to_string();
            let ok = result.is_ok();
            let saved = db_task(pool, move |connection| match &result {
                Ok(result) => store::save_analysis(
                    connection,
                    &path,
                    &version,
                    Some(&result.waveform),
                    result.fingerprint.as_ref(),
                    result.duration_seconds,
                    None,
                ),
                Err(error) => {
                    store::save_analysis(connection, &path, &version, None, None, 0.0, Some(error))
                }
            })
            .await
            .is_ok();
            ok && saved
        }
    };
    loudness.is_some() && saved
}

async fn run_reanalyze(
    job: JobHandle,
    zones: web::Data<ZoneService>,
    pool: DbPool,
    songs: Vec<(String, String)>,
    force: bool,
) {
    job.set_total(songs.len() as u64);
    let mut failed = 0usize;
    for (song_id, path) in songs {
        if job.is_cancelled() {
            break;
        }
        job.set_current(file_title(&path));
        let ok = analyze_one(&zones, &pool, &song_id, &path, force).await;
        if !ok {
            failed += 1;
        }
        job.advance(ok);
    }
    job.finish(if failed == 0 {
        "Loudness, waveforms and fingerprints are up to date.".to_string()
    } else {
        format!("{failed} file(s) could not be decoded; see Skipped files after a check.")
    });
}

#[post("/reanalyze")]
async fn reanalyze(
    body: web::Json<OperationBody>,
    lifecycle: web::Data<LibraryLifecycle>,
    zones: web::Data<ZoneService>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let OperationBody { scope, force } = body.into_inner();
    if let Some(response) = validate_scope(&scope) {
        return response;
    }
    let cache = match cache_or_unavailable(&lifecycle).await {
        Ok(cache) => cache,
        Err(response) => return response,
    };
    let (songs, label) = resolve_scope(&cache.artists, &scope);
    if songs.is_empty() {
        return not_found("Nothing in the library matches.", "scope_empty");
    }
    let songs: Vec<(String, String)> = songs.into_iter().map(|(id, path, _)| (id, path)).collect();
    match jobs::start(JobKind::Reanalyze, label) {
        Ok(job) => {
            let response = job_started(&job);
            tokio::spawn(run_reanalyze(
                job,
                zones,
                pool.get_ref().clone(),
                songs,
                force,
            ));
            response
        }
        Err(status) => job_busy(status),
    }
}

#[get("/songs/{id}/analysis")]
async fn song_analysis(
    id: web::Path<String>,
    lifecycle: web::Data<LibraryLifecycle>,
    zones: web::Data<ZoneService>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let id = id.into_inner();
    let cache = match cache_or_unavailable(&lifecycle).await {
        Ok(cache) => cache,
        Err(response) => return response,
    };
    let index = index_songs(&cache.artists);
    let Some(located) = index.get(id.as_str()) else {
        return not_found("Song not found.", "song_not_found");
    };
    let song = card(located);
    let tags = located.song.loudness;
    drop(index);
    let path = song.path.clone();
    let track_id = id.clone();
    let (row, copies) = match blocking(&pool, move |connection| {
        Ok((
            store::analysis_for(connection, &path)?,
            store::track_copies(connection, &track_id)?,
        ))
    })
    .await
    {
        Ok(loaded) => loaded,
        Err(response) => return response,
    };
    let file = Path::new(&song.path);
    let size_bytes = std::fs::metadata(file)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let current = file_version(file);
    let measured = zones.loudness().cached(&id, file);
    let copies: Vec<CopyFile> = copies
        .into_iter()
        .map(|copy| CopyFile {
            quality: grade(copy.audio().as_ref(), &copy.extension),
            playing: copy.path == song.path,
            path: copy.path,
            size_bytes: copy.size_bytes,
            duration: copy.duration_seconds,
        })
        .collect();
    HttpResponse::Ok().json(serde_json::json!({
        "song": song,
        "size_bytes": size_bytes,
        "waveform": row.as_ref().and_then(|row| row.waveform()),
        "fingerprint": row.as_ref().and_then(|row| row.fingerprint()).map(|print| print.kind()),
        "analyzed_at": row.as_ref().filter(|row| row.error.is_none()).map(|row| row.analyzed_at.clone()),
        "analysis_error": row.as_ref().and_then(|row| row.error.clone()),
        "stale": row.as_ref().is_some_and(|row| Some(&row.file_version) != current.as_ref()),
        "loudness": measured,
        "replay_gain": tags,
        "copies": copies,
    }))
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(overview)
        .service(list_jobs)
        .service(get_job)
        .service(cancel_job)
        .service(list_skipped)
        .service(check_skipped)
        .service(ignore_skipped)
        .service(retry_skipped)
        .service(skipped_playlist)
        .service(list_duplicates)
        .service(resolve_duplicate)
        .service(resolve_suggested)
        .service(restore_duplicates)
        .service(prefer_copy)
        .service(list_versions)
        .service(album_versions)
        .service(choose_version)
        .service(group_albums)
        .service(ungroup_albums)
        .service(list_albumizer)
        .service(albumize)
        .service(unalbumize)
        .service(list_missing)
        .service(forget_missing)
        .service(rescan)
        .service(reidentify)
        .service(reanalyze)
        .service(song_analysis);
}

#[cfg(test)]
mod tests {
    use actix_web::http::StatusCode;
    use actix_web::{App, test as actix_test, web};
    use diesel::connection::SimpleConnection;

    use super::*;
    use crate::domain::AudioProperties;

    fn song(id: &str, title: &str, path: &str, codec: &str, duration: f64) -> Song {
        let lossless = codec == "flac";
        Song {
            id: id.into(),
            name: title.into(),
            artist: "Example Artist".into(),
            path: path.into(),
            duration,
            track_number: 1,
            audio: Some(AudioProperties {
                codec: Some(codec.into()),
                sample_rate: Some(if lossless { 96_000 } else { 44_100 }),
                bit_depth: lossless.then_some(24),
                bitrate_kbps: Some(320),
                lossless: Some(lossless),
                ..AudioProperties::default()
            }),
            ..Song::default()
        }
    }

    fn library() -> Vec<Artist> {
        vec![Artist {
            id: "artist".into(),
            name: "Example Artist".into(),
            albums: vec![
                Album {
                    id: "album".into(),
                    name: "Fixture Album".into(),
                    first_release_date: "1999-01-01".into(),
                    songs: vec![
                        song("a", "Opening Signal", "/music/Album/01.flac", "flac", 241.0),
                        song("b", "Closing Signal", "/music/Album/02.flac", "flac", 200.0),
                    ],
                    ..Album::default()
                },
                Album {
                    id: "remaster".into(),
                    name: "Fixture Album (2019 Remaster)".into(),
                    songs: vec![song(
                        "r",
                        "Opening Signal",
                        "/music/Remaster/01.mp3",
                        "mp3",
                        300.0,
                    )],
                    ..Album::default()
                },
                Album {
                    id: "loose".into(),
                    name: "Singles".into(),
                    songs: vec![song(
                        "c",
                        "Opening Signal (Remastered)",
                        "/music/Loose/c.mp3",
                        "mp3",
                        241.6,
                    )],
                    ..Album::default()
                },
            ],
            ..Artist::default()
        }]
    }

    macro_rules! tools_app {
        ($pool:expr) => {{
            let lifecycle = LibraryLifecycle::new();
            lifecycle
                .set_ready(LibraryCache::from_artists(library()))
                .await;
            actix_test::init_service(
                App::new()
                    .app_data(web::Data::new($pool))
                    .app_data(web::Data::new(lifecycle))
                    .configure(configure),
            )
            .await
        }};
    }

    #[test]
    fn scopes_resolve_to_songs_with_a_readable_label() {
        let catalog = library();
        let (songs, label) = resolve_scope(&catalog, &Scope::Library);
        assert_eq!((songs.len(), label.as_str()), (4, "Whole library"));
        let (songs, label) = resolve_scope(
            &catalog,
            &Scope::Albums {
                ids: vec!["album".into()],
            },
        );
        assert_eq!(songs.len(), 2);
        assert_eq!(label, "Fixture Album");
        let (songs, label) = resolve_scope(
            &catalog,
            &Scope::Songs {
                ids: vec!["a".into(), "c".into()],
            },
        );
        assert_eq!((songs.len(), label.as_str()), (2, "2 items"));
    }

    #[actix_web::test]
    async fn overview_and_duplicates_describe_the_catalog() {
        let pool = crate::persistence::connection::migrated_memory_pool();
        let app = tools_app!(pool);
        let response = actix_test::call_service(
            &app,
            actix_test::TestRequest::get().uri("/overview").to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = actix_test::read_body_json(response).await;
        assert_eq!(body["stats"]["tracks"], 4);
        assert_eq!(body["stats"]["tiers"][0]["key"], "hi_res");
        assert_eq!(body["duplicates"], 1);
        assert_eq!(body["duplicates_suggested"], 1);
        assert_eq!(body["loose_tracks"], 2);

        let response = actix_test::call_service(
            &app,
            actix_test::TestRequest::get()
                .uri("/duplicates")
                .to_request(),
        )
        .await;
        let body: serde_json::Value = actix_test::read_body_json(response).await;
        let group = &body["groups"][0];
        assert_eq!(group["best"], "a");
        assert_eq!(group["kind"], "loose");
        assert_eq!(group["songs"][0]["quality"]["label"], "FLAC 24/96");
        assert_eq!(group["songs"][1]["id"], "c");
    }

    #[actix_web::test]
    async fn skipped_files_list_ignore_and_missing_files_need_a_reachable_root() {
        let pool = crate::persistence::connection::migrated_memory_pool();
        pool.get()
            .unwrap()
            .batch_execute(
                "INSERT INTO library_root (id, path) VALUES (1, '/parson-test-offline-drive');
                 INSERT INTO library_skipped_file (path, root_path, reason, detail, extension)
                 VALUES ('/parson-test-offline-drive/mix.m3u', '/parson-test-offline-drive', 'playlist_file', '', 'm3u'),
                        ('/parson-test-offline-drive/a.m4p', '/parson-test-offline-drive', 'drm_protected', '', 'm4p');
                 INSERT INTO file_entry (id, root_id, path, directory, file_name, extension, size_bytes, modified_at_ns, availability)
                 VALUES (1, 1, '/parson-test-offline-drive/gone.flac', '/parson-test-offline-drive', 'gone.flac', 'flac', 1, 1, 'missing');",
            )
            .unwrap();
        let app = tools_app!(pool);
        let response = actix_test::call_service(
            &app,
            actix_test::TestRequest::get().uri("/skipped").to_request(),
        )
        .await;
        let body: serde_json::Value = actix_test::read_body_json(response).await;
        assert_eq!(body["items"].as_array().unwrap().len(), 2);
        assert_eq!(body["items"][0]["reason"], "drm_protected");
        assert_eq!(body["items"][1]["fix"]["action"], "import_playlist");
        assert_eq!(body["reasons"].as_array().unwrap().len(), 2);

        let response = actix_test::call_service(
            &app,
            actix_test::TestRequest::post()
                .uri("/skipped/ignore")
                .set_json(serde_json::json!({ "paths": ["/parson-test-offline-drive/a.m4p"] }))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let response = actix_test::call_service(
            &app,
            actix_test::TestRequest::get().uri("/skipped").to_request(),
        )
        .await;
        let body: serde_json::Value = actix_test::read_body_json(response).await;
        assert_eq!(body["items"][0]["ignored"], true);
        assert_eq!(body["reasons"].as_array().unwrap().len(), 1);

        // The playlist file is not on disk: nothing is served.
        let response = actix_test::call_service(
            &app,
            actix_test::TestRequest::get()
                .uri("/skipped/playlist?path=%2Fetc%2Fpasswd")
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        // A missing file under an offline folder is listed but never forgotten.
        let response = actix_test::call_service(
            &app,
            actix_test::TestRequest::get().uri("/missing").to_request(),
        )
        .await;
        let body: serde_json::Value = actix_test::read_body_json(response).await;
        assert_eq!(body["files"][0]["root_reachable"], false);
        assert_eq!(body["forgettable"], 0);
        let response = actix_test::call_service(
            &app,
            actix_test::TestRequest::post()
                .uri("/missing/forget")
                .set_json(serde_json::json!({ "confirm": 1 }))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
