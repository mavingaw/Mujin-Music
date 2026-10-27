//! `/imports`: connect Spotify, Apple Music, YouTube Music and Deezer (or
//! upload a playlist file), import playlists, review the matches against the
//! person's own library, and save them as Parson playlists. Spotify Connect
//! handoff also lives here. No audio is ever streamed from these services.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use actix_web::http::StatusCode;
use actix_web::{HttpRequest, HttpResponse, delete, get, patch, post, put, web};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Nullable, Text};
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use crate::api::error::{bad_request, internal_server_error, json_error, not_found};
use crate::imports::apple::{self, AppleCredentials};
use crate::imports::files::{MAX_PLAYLIST_FILE_BYTES, parse_playlist_file};
use crate::imports::http::{ProviderError, client};
use crate::imports::spotify::{self, HandoffTrack, SpotifyCredentials};
use crate::imports::urls::{ParsedSource, SourceKind, parse_source_url};
use crate::imports::{
    ImportService, ImportedPlaylist, ImportedTrack, RemotePlaylist, ServiceDescriptor, deezer,
    service_descriptor, youtube,
};
use crate::library::matching::{MatchCandidate, MatchIndex, MatchReason, MatchStatus};
use crate::library::state::{LibraryCache, LibraryLifecycle, library_unavailable_response};
use crate::persistence::connection::DbPool;
use crate::playlist_rules::{
    MAX_PLAYLIST_NAME_CHARACTERS, MAX_PLAYLISTS, MAX_TRACKS_PER_PLAYLIST, valid_song_id,
};

/// Pending OAuth sign-ins expire after ten minutes.
const AUTH_STATE_TTL_SECONDS: i64 = 600;
/// A job still fetching after this long was interrupted (e.g. a restart).
const STALE_JOB_SECONDS: i64 = 900;
const MAX_JOBS_LISTED: i64 = 100;
const MAX_JOBS_PER_USER: i64 = 200;
const MAX_ITEM_UPDATES: usize = 2_000;
const HANDOFF_SEARCH_CONCURRENCY: usize = 6;
/// Scope-local JSON limit so playlist files fit (the global limit is 1 MB).
pub const MAX_IMPORT_JSON_BYTES: usize = MAX_PLAYLIST_FILE_BYTES + 64 * 1024;

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn provider_error(error: ProviderError) -> HttpResponse {
    let message = error.to_string();
    // Never 401: that status means "your Parson session expired" to clients.
    match error {
        ProviderError::Unauthorized => json_error(
            StatusCode::CONFLICT,
            "conflict",
            message,
            "import_service_reauth",
        ),
        ProviderError::NotFound => not_found(message, "import_source_not_found"),
        ProviderError::RateLimited => json_error(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_requests",
            message,
            "import_service_rate_limited",
        ),
        ProviderError::Forbidden(_) | ProviderError::Invalid(_) => json_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "unprocessable_entity",
            message,
            "import_service_refused",
        ),
        ProviderError::Upstream(_) => json_error(
            StatusCode::BAD_GATEWAY,
            "bad_gateway",
            message,
            "import_service_unavailable",
        ),
    }
}

fn worker_failed(context: &str, error: impl std::fmt::Display) -> HttpResponse {
    tracing::error!(%error, context, "import request failed");
    internal_server_error("The import could not be completed.", "import_failed")
}

async fn blocking<T: Send + 'static>(
    pool: &web::Data<DbPool>,
    work: impl FnOnce(&mut SqliteConnection) -> Result<T, diesel::result::Error> + Send + 'static,
) -> Result<T, String> {
    let pool = pool.get_ref().clone();
    web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        work(&mut connection).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

#[derive(QueryableByName, Debug)]
struct ConnectionRow {
    #[diesel(sql_type = Text)]
    service: String,
    #[diesel(sql_type = Text)]
    credentials: String,
    #[diesel(sql_type = Nullable<Text>)]
    account_label: Option<String>,
    #[diesel(sql_type = Text)]
    connected_at: String,
}

fn load_connections(
    connection: &mut SqliteConnection,
    user_id: i32,
) -> QueryResult<Vec<ConnectionRow>> {
    diesel::sql_query(
        "SELECT service, credentials, account_label, CAST(connected_at AS TEXT) AS connected_at \
         FROM import_connection WHERE user_id = ?",
    )
    .bind::<Integer, _>(user_id)
    .load(connection)
}

fn load_connection(
    connection: &mut SqliteConnection,
    user_id: i32,
    service: ImportService,
) -> QueryResult<Option<ConnectionRow>> {
    diesel::sql_query(
        "SELECT service, credentials, account_label, CAST(connected_at AS TEXT) AS connected_at \
         FROM import_connection WHERE user_id = ? AND service = ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(service.as_str())
    .get_result(connection)
    .optional()
}

fn save_connection(
    connection: &mut SqliteConnection,
    user_id: i32,
    service: ImportService,
    credentials: &str,
    account_label: Option<&str>,
) -> QueryResult<usize> {
    diesel::sql_query(
        "INSERT INTO import_connection (user_id, service, credentials, account_label) \
         VALUES (?, ?, ?, ?) \
         ON CONFLICT(user_id, service) DO UPDATE SET \
           credentials = excluded.credentials, \
           account_label = COALESCE(excluded.account_label, import_connection.account_label), \
           updated_at = CURRENT_TIMESTAMP",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(service.as_str())
    .bind::<Text, _>(credentials)
    .bind::<Nullable<Text>, _>(account_label)
    .execute(connection)
}

fn delete_connection(
    connection: &mut SqliteConnection,
    user_id: i32,
    service: ImportService,
) -> QueryResult<usize> {
    diesel::sql_query("DELETE FROM import_connection WHERE user_id = ? AND service = ?")
        .bind::<Integer, _>(user_id)
        .bind::<Text, _>(service.as_str())
        .execute(connection)
}

#[derive(QueryableByName, Debug)]
struct AuthStateRow {
    #[diesel(sql_type = Text)]
    code_verifier: String,
    #[diesel(sql_type = Text)]
    client_id: String,
    #[diesel(sql_type = Text)]
    redirect_uri: String,
}

/// A pending OAuth sign-in, stored until the service redirects back.
struct PendingAuth<'a> {
    state: &'a str,
    user_id: i32,
    service: ImportService,
    verifier: &'a str,
    client_id: &'a str,
    redirect_uri: &'a str,
    now: i64,
}

fn insert_auth_state(
    connection: &mut SqliteConnection,
    pending: &PendingAuth<'_>,
) -> QueryResult<()> {
    diesel::sql_query("DELETE FROM import_auth_state WHERE created_at < ? OR user_id = ?")
        .bind::<BigInt, _>(pending.now - AUTH_STATE_TTL_SECONDS)
        .bind::<Integer, _>(pending.user_id)
        .execute(connection)?;
    diesel::sql_query(
        "INSERT INTO import_auth_state \
         (state, user_id, service, code_verifier, client_id, redirect_uri, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Text, _>(pending.state)
    .bind::<Integer, _>(pending.user_id)
    .bind::<Text, _>(pending.service.as_str())
    .bind::<Text, _>(pending.verifier)
    .bind::<Text, _>(pending.client_id)
    .bind::<Text, _>(pending.redirect_uri)
    .bind::<BigInt, _>(pending.now)
    .execute(connection)?;
    Ok(())
}

/// Consumes a pending sign-in: a state can be used once, by its owner, in time.
fn take_auth_state(
    connection: &mut SqliteConnection,
    state: &str,
    user_id: i32,
    service: ImportService,
    now: i64,
) -> QueryResult<Option<AuthStateRow>> {
    connection.transaction(|connection| {
        let row = diesel::sql_query(
            "SELECT code_verifier, client_id, redirect_uri FROM import_auth_state \
             WHERE state = ? AND user_id = ? AND service = ? AND created_at >= ?",
        )
        .bind::<Text, _>(state)
        .bind::<Integer, _>(user_id)
        .bind::<Text, _>(service.as_str())
        .bind::<BigInt, _>(now - AUTH_STATE_TTL_SECONDS)
        .get_result::<AuthStateRow>(connection)
        .optional()?;
        diesel::sql_query("DELETE FROM import_auth_state WHERE state = ?")
            .bind::<Text, _>(state)
            .execute(connection)?;
        Ok(row)
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    Matched,
    Review,
    Unmatched,
    /// Accepted or chosen by the person.
    Confirmed,
    /// Excluded by the person.
    Skipped,
}

impl From<MatchStatus> for ItemStatus {
    fn from(status: MatchStatus) -> Self {
        match status {
            MatchStatus::Matched => ItemStatus::Matched,
            MatchStatus::Review => ItemStatus::Review,
            MatchStatus::Unmatched => ItemStatus::Unmatched,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobItem {
    pub position: usize,
    pub source: ImportedTrack,
    pub status: ItemStatus,
    pub song_id: Option<String>,
    pub confidence: f32,
    #[serde(default)]
    pub reasons: Vec<MatchReason>,
    #[serde(default)]
    pub candidates: Vec<MatchCandidate>,
}

impl JobItem {
    fn included(&self) -> bool {
        matches!(self.status, ItemStatus::Matched | ItemStatus::Confirmed) && self.song_id.is_some()
    }
}

#[derive(QueryableByName, Debug)]
struct JobRow {
    #[diesel(sql_type = Text)]
    id: String,
    #[diesel(sql_type = Text)]
    service: String,
    #[diesel(sql_type = Text)]
    source_kind: String,
    #[diesel(sql_type = Text)]
    source_ref: String,
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = Text)]
    status: String,
    #[diesel(sql_type = Nullable<Text>)]
    error: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    source_url: Option<String>,
    #[diesel(sql_type = Nullable<Integer>)]
    playlist_id: Option<i32>,
    #[diesel(sql_type = BigInt)]
    created_at: i64,
    #[diesel(sql_type = BigInt)]
    updated_at: i64,
    #[diesel(sql_type = BigInt)]
    total: i64,
    #[diesel(sql_type = BigInt)]
    matched: i64,
    #[diesel(sql_type = BigInt)]
    review: i64,
    #[diesel(sql_type = BigInt)]
    unmatched: i64,
    #[diesel(sql_type = BigInt)]
    skipped: i64,
}

#[derive(QueryableByName)]
struct ItemsRow {
    #[diesel(sql_type = Text)]
    items: String,
}

const JOB_COLUMNS: &str = "id, service, source_kind, source_ref, name, status, error, source_url, \
     playlist_id, created_at, updated_at, json_array_length(items) AS total, \
     (SELECT COUNT(*) FROM json_each(import_job.items) \
       WHERE json_extract(value, '$.status') IN ('matched', 'confirmed')) AS matched, \
     (SELECT COUNT(*) FROM json_each(import_job.items) \
       WHERE json_extract(value, '$.status') = 'review') AS review, \
     (SELECT COUNT(*) FROM json_each(import_job.items) \
       WHERE json_extract(value, '$.status') = 'unmatched') AS unmatched, \
     (SELECT COUNT(*) FROM json_each(import_job.items) \
       WHERE json_extract(value, '$.status') = 'skipped') AS skipped";

#[derive(Clone, Debug)]
struct NewJob<'a> {
    id: &'a str,
    user_id: i32,
    service: ImportService,
    source_kind: &'a str,
    source_ref: &'a str,
    name: &'a str,
    status: &'a str,
    source_url: Option<&'a str>,
    now: i64,
}

fn insert_job(connection: &mut SqliteConnection, job: &NewJob<'_>) -> QueryResult<()> {
    // Keep the newest jobs; old ones are history, not data.
    diesel::sql_query(
        "DELETE FROM import_job WHERE user_id = ? AND id NOT IN ( \
           SELECT id FROM import_job WHERE user_id = ? ORDER BY updated_at DESC LIMIT ?)",
    )
    .bind::<Integer, _>(job.user_id)
    .bind::<Integer, _>(job.user_id)
    .bind::<BigInt, _>(MAX_JOBS_PER_USER - 1)
    .execute(connection)?;
    diesel::sql_query(
        "INSERT INTO import_job (id, user_id, service, source_kind, source_ref, name, status, \
         source_url, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Text, _>(job.id)
    .bind::<Integer, _>(job.user_id)
    .bind::<Text, _>(job.service.as_str())
    .bind::<Text, _>(job.source_kind)
    .bind::<Text, _>(job.source_ref)
    .bind::<Text, _>(job.name)
    .bind::<Text, _>(job.status)
    .bind::<Nullable<Text>, _>(job.source_url)
    .bind::<BigInt, _>(job.now)
    .bind::<BigInt, _>(job.now)
    .execute(connection)?;
    Ok(())
}

fn load_job(
    connection: &mut SqliteConnection,
    user_id: i32,
    job_id: &str,
) -> QueryResult<Option<(JobRow, Vec<JobItem>)>> {
    let Some(row) = diesel::sql_query(format!(
        "SELECT {JOB_COLUMNS} FROM import_job WHERE id = ? AND user_id = ?"
    ))
    .bind::<Text, _>(job_id)
    .bind::<Integer, _>(user_id)
    .get_result::<JobRow>(connection)
    .optional()?
    else {
        return Ok(None);
    };
    let items = diesel::sql_query("SELECT items FROM import_job WHERE id = ?")
        .bind::<Text, _>(job_id)
        .get_result::<ItemsRow>(connection)?;
    let items = serde_json::from_str(&items.items).unwrap_or_default();
    Ok(Some((row, items)))
}

fn list_jobs(connection: &mut SqliteConnection, user_id: i32) -> QueryResult<Vec<JobRow>> {
    diesel::sql_query(format!(
        "SELECT {JOB_COLUMNS} FROM import_job WHERE user_id = ? \
         ORDER BY updated_at DESC LIMIT ?"
    ))
    .bind::<Integer, _>(user_id)
    .bind::<BigInt, _>(MAX_JOBS_LISTED)
    .load(connection)
}

#[derive(Default)]
struct JobUpdate<'a> {
    status: Option<&'a str>,
    name: Option<&'a str>,
    error: Option<&'a str>,
    source_url: Option<&'a str>,
    items: Option<&'a [JobItem]>,
    playlist_id: Option<i32>,
}

fn update_job(
    connection: &mut SqliteConnection,
    job_id: &str,
    update: &JobUpdate<'_>,
    now: i64,
) -> QueryResult<usize> {
    let items = update
        .items
        .map(|items| serde_json::to_string(items).unwrap_or_else(|_| "[]".to_string()));
    diesel::sql_query(
        "UPDATE import_job SET \
           status = COALESCE(?, status), name = COALESCE(?, name), \
           error = CASE WHEN ? IS NULL THEN error ELSE ? END, \
           source_url = COALESCE(?, source_url), items = COALESCE(?, items), \
           playlist_id = COALESCE(?, playlist_id), updated_at = ? \
         WHERE id = ?",
    )
    .bind::<Nullable<Text>, _>(update.status)
    .bind::<Nullable<Text>, _>(update.name)
    .bind::<Nullable<Text>, _>(update.error)
    .bind::<Nullable<Text>, _>(update.error)
    .bind::<Nullable<Text>, _>(update.source_url)
    .bind::<Nullable<Text>, _>(items.as_deref())
    .bind::<Nullable<Integer>, _>(update.playlist_id)
    .bind::<BigInt, _>(now)
    .bind::<Text, _>(job_id)
    .execute(connection)
}

fn delete_job(connection: &mut SqliteConnection, user_id: i32, job_id: &str) -> QueryResult<usize> {
    diesel::sql_query("DELETE FROM import_job WHERE id = ? AND user_id = ?")
        .bind::<Text, _>(job_id)
        .bind::<Integer, _>(user_id)
        .execute(connection)
}

#[derive(Debug, PartialEq)]
enum SavePlaylistOutcome {
    Created { playlist_id: i32, added: usize },
    Updated { playlist_id: i32, added: usize },
    CapacityReached,
}

/// Creates the job's playlist, or adds newly matched songs to the playlist an
/// earlier save created. Unknown song ids are dropped.
fn save_playlist(
    connection: &mut SqliteConnection,
    user_id: i32,
    existing_playlist: Option<i32>,
    name: &str,
    description: &str,
    song_ids: &[String],
) -> QueryResult<SavePlaylistOutcome> {
    #[derive(QueryableByName)]
    struct IdRow {
        #[diesel(sql_type = Text)]
        id: String,
    }
    #[derive(QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = BigInt)]
        count: i64,
    }
    #[derive(QueryableByName)]
    struct PlaylistIdRow {
        #[diesel(sql_type = Integer)]
        id: i32,
    }
    connection.transaction(|connection| {
        let mut known = HashSet::new();
        for chunk in song_ids.chunks(400) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let mut query =
                diesel::sql_query(format!("SELECT id FROM song WHERE id IN ({placeholders})"))
                    .into_boxed();
            for id in chunk {
                query = query.bind::<Text, _>(id.clone());
            }
            known.extend(
                query
                    .load::<IdRow>(connection)?
                    .into_iter()
                    .map(|row| row.id),
            );
        }
        let mut seen = HashSet::new();
        let wanted: Vec<&String> = song_ids
            .iter()
            .filter(|id| known.contains(*id) && seen.insert((*id).clone()))
            .collect();

        let owned = match existing_playlist {
            Some(playlist_id) => {
                diesel::sql_query("SELECT a AS id FROM _playlist_to_user WHERE a = ? AND b = ?")
                    .bind::<Integer, _>(playlist_id)
                    .bind::<Integer, _>(user_id)
                    .get_result::<PlaylistIdRow>(connection)
                    .optional()?
                    .map(|row| row.id)
            }
            None => None,
        };
        let (playlist_id, created) = match owned {
            Some(id) => (id, false),
            None => {
                let count = diesel::sql_query(
                    "SELECT COUNT(*) AS count FROM _playlist_to_user WHERE b = ?",
                )
                .bind::<Integer, _>(user_id)
                .get_result::<CountRow>(connection)?
                .count;
                if count >= MAX_PLAYLISTS {
                    return Ok(SavePlaylistOutcome::CapacityReached);
                }
                let id = diesel::sql_query(
                    "INSERT INTO playlist (name, description) VALUES (?, ?) RETURNING id",
                )
                .bind::<Text, _>(name)
                .bind::<Text, _>(description)
                .get_result::<PlaylistIdRow>(connection)?
                .id;
                diesel::sql_query("INSERT INTO _playlist_to_user (a, b) VALUES (?, ?)")
                    .bind::<Integer, _>(id)
                    .bind::<Integer, _>(user_id)
                    .execute(connection)?;
                (id, true)
            }
        };
        let present: HashSet<String> =
            diesel::sql_query("SELECT b AS id FROM _playlist_to_song WHERE a = ?")
                .bind::<Integer, _>(playlist_id)
                .load::<IdRow>(connection)?
                .into_iter()
                .map(|row| row.id)
                .collect();
        let room = (MAX_TRACKS_PER_PLAYLIST as usize).saturating_sub(present.len());
        let mut added = 0;
        for song_id in wanted
            .into_iter()
            .filter(|id| !present.contains(*id))
            .take(room)
        {
            diesel::sql_query("INSERT INTO _playlist_to_song (a, b, added_by) VALUES (?, ?, ?)")
                .bind::<Integer, _>(playlist_id)
                .bind::<Text, _>(song_id)
                .bind::<Integer, _>(user_id)
                .execute(connection)?;
            added += 1;
        }
        if !created && added > 0 {
            diesel::sql_query("UPDATE playlist SET updated_at = CURRENT_TIMESTAMP WHERE id = ?")
                .bind::<Integer, _>(playlist_id)
                .execute(connection)?;
        }
        Ok(if created {
            SavePlaylistOutcome::Created { playlist_id, added }
        } else {
            SavePlaylistOutcome::Updated { playlist_id, added }
        })
    })
}

// ---------------------------------------------------------------------------
// Matching
// ---------------------------------------------------------------------------

fn match_items(index: &MatchIndex, tracks: Vec<ImportedTrack>) -> Vec<JobItem> {
    tracks
        .into_iter()
        .enumerate()
        .map(|(position, source)| {
            let result = index.match_track(&source.track);
            JobItem {
                position,
                status: result.status.into(),
                song_id: result.song_id,
                confidence: result.confidence,
                reasons: result.reasons,
                candidates: result.candidates,
                source,
            }
        })
        .collect()
}

/// Re-runs matching for items the person has not decided on.
fn rematch_items(index: &MatchIndex, items: &mut [JobItem]) -> usize {
    let mut changed = 0;
    for item in items
        .iter_mut()
        .filter(|item| !matches!(item.status, ItemStatus::Confirmed | ItemStatus::Skipped))
    {
        let result = index.match_track(&item.source.track);
        let status = ItemStatus::from(result.status);
        if status != item.status || result.song_id != item.song_id {
            changed += 1;
        }
        item.status = status;
        item.song_id = result.song_id;
        item.confidence = result.confidence;
        item.reasons = result.reasons;
        item.candidates = result.candidates;
    }
    changed
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case", tag = "action")]
enum ItemAction {
    /// Keep the proposed match.
    Accept { position: usize },
    /// Use a specific library song.
    Choose { position: usize, song_id: String },
    /// Leave the track out of the playlist.
    Skip { position: usize },
    /// Forget the person's decision and match again.
    Reset { position: usize },
}

impl ItemAction {
    fn position(&self) -> usize {
        match self {
            ItemAction::Accept { position }
            | ItemAction::Choose { position, .. }
            | ItemAction::Skip { position }
            | ItemAction::Reset { position } => *position,
        }
    }
}

#[derive(Debug, PartialEq)]
enum ApplyError {
    UnknownPosition(usize),
    NothingToAccept(usize),
    UnknownSong(String),
}

fn apply_actions(
    items: &mut [JobItem],
    actions: &[ItemAction],
    song_exists: impl Fn(&str) -> bool,
    index: Option<&MatchIndex>,
) -> Result<(), ApplyError> {
    for action in actions {
        let position = action.position();
        let item = items
            .get_mut(position)
            .ok_or(ApplyError::UnknownPosition(position))?;
        match action {
            ItemAction::Accept { .. } => {
                if item.song_id.is_none() {
                    return Err(ApplyError::NothingToAccept(position));
                }
                item.status = ItemStatus::Confirmed;
            }
            ItemAction::Choose { song_id, .. } => {
                if !valid_song_id(song_id) || !song_exists(song_id) {
                    return Err(ApplyError::UnknownSong(song_id.clone()));
                }
                item.song_id = Some(song_id.clone());
                item.status = ItemStatus::Confirmed;
                if let Some(candidate) = item
                    .candidates
                    .iter()
                    .find(|candidate| &candidate.song_id == song_id)
                {
                    item.confidence = candidate.confidence;
                    item.reasons = candidate.reasons.clone();
                } else {
                    item.reasons = Vec::new();
                }
            }
            ItemAction::Skip { .. } => item.status = ItemStatus::Skipped,
            ItemAction::Reset { .. } => {
                item.status = ItemStatus::Unmatched;
                if let Some(index) = index {
                    rematch_items(index, std::slice::from_mut(item));
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Responses
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ConnectionView {
    connected_at: String,
    account_label: Option<String>,
    details: serde_json::Value,
}

#[derive(Serialize)]
struct ServiceView {
    #[serde(flatten)]
    descriptor: ServiceDescriptor,
    connection: Option<ConnectionView>,
}

fn hint(secret: &str) -> String {
    let tail: String = secret
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("…{tail}")
}

/// Public details of a saved connection; never includes a secret.
fn connection_details(service: ImportService, credentials: &str) -> serde_json::Value {
    match service {
        ImportService::Spotify => {
            let credentials: SpotifyCredentials =
                serde_json::from_str(credentials).unwrap_or_default();
            serde_json::json!({
                "client_id": hint(&credentials.client_id),
                "premium": credentials.product.as_deref().map(|product| product == "premium"),
                "can_handoff": credentials.scope.contains("user-modify-playback-state"),
            })
        }
        ImportService::AppleMusic => {
            let credentials: AppleCredentials =
                serde_json::from_str(credentials).unwrap_or_default();
            serde_json::json!({
                "storefront": credentials.storefront,
                "has_user_token": credentials.music_user_token.is_some(),
            })
        }
        ImportService::YoutubeMusic => {
            let credentials: serde_json::Value =
                serde_json::from_str(credentials).unwrap_or_default();
            serde_json::json!({
                "api_key": hint(credentials["api_key"].as_str().unwrap_or_default()),
            })
        }
        ImportService::Deezer => {
            let credentials: serde_json::Value =
                serde_json::from_str(credentials).unwrap_or_default();
            serde_json::json!({ "profile_id": credentials["profile_id"] })
        }
        ImportService::File => serde_json::json!({}),
    }
}

#[derive(Serialize)]
struct JobSummary {
    id: String,
    service: String,
    source_kind: String,
    source_ref: String,
    name: String,
    status: String,
    error: Option<String>,
    source_url: Option<String>,
    playlist_id: Option<i32>,
    created_at: i64,
    updated_at: i64,
    /// Tracks read so far while the job is fetching.
    fetched: Option<usize>,
    total: i64,
    matched: i64,
    review: i64,
    unmatched: i64,
    skipped: i64,
}

fn progress_map() -> &'static Mutex<HashMap<String, usize>> {
    static PROGRESS: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();
    PROGRESS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn set_progress(job_id: &str, fetched: Option<usize>) {
    if let Ok(mut progress) = progress_map().lock() {
        match fetched {
            Some(count) => {
                progress.insert(job_id.to_string(), count);
            }
            None => {
                progress.remove(job_id);
            }
        }
    }
}

fn summary(row: JobRow, now: i64) -> JobSummary {
    let in_flight = matches!(row.status.as_str(), "fetching" | "matching");
    let fetched = in_flight
        .then(|| progress_map().lock().ok()?.get(&row.id).copied())
        .flatten();
    // A job that stopped reporting was interrupted (e.g. the server restarted).
    let (status, error) =
        if in_flight && fetched.is_none() && now - row.updated_at > STALE_JOB_SECONDS {
            (
                "failed".to_string(),
                Some("The import was interrupted. Start it again.".to_string()),
            )
        } else {
            (row.status, row.error)
        };
    JobSummary {
        id: row.id,
        service: row.service,
        source_kind: row.source_kind,
        source_ref: row.source_ref,
        name: row.name,
        status,
        error,
        source_url: row.source_url,
        playlist_id: row.playlist_id,
        created_at: row.created_at,
        updated_at: row.updated_at,
        fetched,
        total: row.total,
        matched: row.matched,
        review: row.review,
        unmatched: row.unmatched,
        skipped: row.skipped,
    }
}

#[derive(Serialize, Clone)]
struct SongBrief {
    id: String,
    name: String,
    artist: String,
    album: String,
    album_id: String,
    cover_url: String,
    duration: f64,
    path: String,
}

fn song_brief(cache: &LibraryCache, song_id: &str) -> Option<SongBrief> {
    let song = cache.song(song_id)?;
    let (_, album_id) = cache.song_map.get(song_id)?;
    let album = cache.album(album_id)?;
    Some(SongBrief {
        id: song.id.clone(),
        name: song.name.clone(),
        artist: song.artist.clone(),
        album: album.name.clone(),
        album_id: album.id.clone(),
        cover_url: album.cover_url.clone(),
        duration: song.duration,
        path: song.path.clone(),
    })
}

#[derive(Serialize)]
struct JobDetail {
    #[serde(flatten)]
    summary: JobSummary,
    items: Vec<JobItem>,
    songs: HashMap<String, SongBrief>,
}

fn job_detail(row: JobRow, items: Vec<JobItem>, cache: Option<&LibraryCache>) -> JobDetail {
    let mut songs = HashMap::new();
    if let Some(cache) = cache {
        for item in &items {
            for id in item
                .song_id
                .iter()
                .chain(item.candidates.iter().map(|candidate| &candidate.song_id))
            {
                if !songs.contains_key(id)
                    && let Some(brief) = song_brief(cache, id)
                {
                    songs.insert(id.clone(), brief);
                }
            }
        }
    }
    JobDetail {
        summary: summary(row, unix_now()),
        items,
        songs,
    }
}

// ---------------------------------------------------------------------------
// Services and connections
// ---------------------------------------------------------------------------

fn parse_service(value: &str) -> Result<ImportService, HttpResponse> {
    ImportService::parse(value)
        .filter(|service| *service != ImportService::File)
        .ok_or_else(|| not_found("Unknown music service.", "import_service_unknown"))
}

#[get("/services")]
async fn list_services(request: HttpRequest, pool: web::Data<DbPool>) -> HttpResponse {
    let user_id = match crate::api::auth::authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let rows = match blocking(&pool, move |connection| {
        load_connections(connection, user_id)
    })
    .await
    {
        Ok(rows) => rows,
        Err(error) => return worker_failed("list services", error),
    };
    let views: Vec<ServiceView> = ImportService::ALL
        .into_iter()
        .map(|service| ServiceView {
            descriptor: service_descriptor(service),
            connection: rows
                .iter()
                .find(|row| row.service == service.as_str())
                .map(|row| ConnectionView {
                    connected_at: row.connected_at.clone(),
                    account_label: row.account_label.clone(),
                    details: connection_details(service, &row.credentials),
                }),
        })
        .collect();
    HttpResponse::Ok().json(views)
}

#[derive(Deserialize)]
struct SpotifyAuthorize {
    client_id: String,
    redirect_uri: String,
}

#[post("/services/spotify/authorize")]
async fn spotify_authorize(
    request: HttpRequest,
    body: web::Json<SpotifyAuthorize>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match crate::api::auth::authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let client_id = body.client_id.trim().to_ascii_lowercase();
    let redirect_uri = body.redirect_uri.trim().to_string();
    if !spotify::valid_client_id(&client_id) {
        return bad_request(
            "Paste the 32-character Client ID from your Spotify app.",
            "spotify_client_id_invalid",
        );
    }
    if !spotify::valid_redirect_uri(&redirect_uri) {
        return bad_request(
            "Spotify only accepts https or http://127.0.0.1 redirect URIs. Open Parson at http://127.0.0.1 or over https to connect.",
            "spotify_redirect_uri_invalid",
        );
    }
    let state = spotify::random_state();
    let verifier = spotify::code_verifier();
    let authorize_url = spotify::authorize_url(
        spotify::ACCOUNTS_BASE,
        &client_id,
        &redirect_uri,
        &state,
        &verifier,
    );
    let stored_state = state.clone();
    match blocking(&pool, move |connection| {
        insert_auth_state(
            connection,
            &PendingAuth {
                state: &stored_state,
                user_id,
                service: ImportService::Spotify,
                verifier: &verifier,
                client_id: &client_id,
                redirect_uri: &redirect_uri,
                now: unix_now(),
            },
        )
    })
    .await
    {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({
            "authorize_url": authorize_url,
            "state": state,
        })),
        Err(error) => worker_failed("spotify authorize", error),
    }
}

#[derive(Deserialize)]
struct SpotifyCallback {
    code: String,
    state: String,
}

#[post("/services/spotify/callback")]
async fn spotify_callback(
    request: HttpRequest,
    body: web::Json<SpotifyCallback>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match crate::api::auth::authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    if body.code.is_empty() || body.code.len() > 2048 || body.state.len() > 128 {
        return bad_request(
            "The Spotify sign-in reply is invalid.",
            "spotify_callback_invalid",
        );
    }
    let state = body.state.clone();
    let pending = match blocking(&pool, move |connection| {
        take_auth_state(
            connection,
            &state,
            user_id,
            ImportService::Spotify,
            unix_now(),
        )
    })
    .await
    {
        Ok(Some(pending)) => pending,
        Ok(None) => {
            return bad_request(
                "This Spotify sign-in expired or was already used. Connect again.",
                "spotify_state_invalid",
            );
        }
        Err(error) => return worker_failed("spotify callback", error),
    };
    let http = match client() {
        Ok(http) => http,
        Err(error) => return provider_error(error),
    };
    let mut credentials = match spotify::exchange_code(
        &http,
        spotify::ACCOUNTS_BASE,
        &pending.client_id,
        &body.code,
        &pending.redirect_uri,
        &pending.code_verifier,
        unix_now(),
    )
    .await
    {
        Ok(credentials) => credentials,
        Err(error) => return provider_error(error),
    };
    let profile = spotify::profile(&http, spotify::API_BASE, &credentials.access_token).await;
    let label = match profile {
        Ok(profile) => {
            credentials.product = profile.product;
            Some(profile.display_name.unwrap_or(profile.id))
        }
        Err(error) => {
            tracing::warn!(%error, "could not read Spotify profile");
            None
        }
    };
    let serialized = serde_json::to_string(&credentials).unwrap_or_default();
    match blocking(&pool, move |connection| {
        save_connection(
            connection,
            user_id,
            ImportService::Spotify,
            &serialized,
            label.as_deref(),
        )
    })
    .await
    {
        Ok(_) => HttpResponse::NoContent().finish(),
        Err(error) => worker_failed("spotify save", error),
    }
}

#[derive(Deserialize)]
struct ConfigureService {
    developer_token: Option<String>,
    music_user_token: Option<String>,
    storefront: Option<String>,
    api_key: Option<String>,
    profile_id: Option<String>,
}

#[derive(Deserialize)]
struct DeezerUser {
    name: Option<String>,
}

#[put("/services/{service}")]
async fn configure_service(
    request: HttpRequest,
    service: web::Path<String>,
    body: web::Json<ConfigureService>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match crate::api::auth::authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let service = match parse_service(&service) {
        Ok(service) => service,
        Err(response) => return response,
    };
    let http = match client() {
        Ok(http) => http,
        Err(error) => return provider_error(error),
    };
    let body = body.into_inner();
    let (credentials, label) = match service {
        ImportService::Spotify | ImportService::File => {
            return bad_request(
                "Connect Spotify with the sign-in button.",
                "import_service_method",
            );
        }
        ImportService::AppleMusic => {
            let developer_token = body.developer_token.unwrap_or_default().trim().to_string();
            let music_user_token = body
                .music_user_token
                .map(|token| token.trim().to_string())
                .filter(|token| !token.is_empty());
            let storefront = body
                .storefront
                .unwrap_or_else(|| "us".to_string())
                .trim()
                .to_ascii_lowercase();
            if !apple::plausible_jwt(&developer_token) {
                return bad_request(
                    "Paste a MusicKit developer token (a JWT signed with your MusicKit key).",
                    "apple_developer_token_invalid",
                );
            }
            if music_user_token
                .as_ref()
                .is_some_and(|token| token.len() > 4096)
            {
                return bad_request(
                    "The Music User Token is too long.",
                    "apple_user_token_invalid",
                );
            }
            if !apple::valid_storefront(&storefront) {
                return bad_request(
                    "Use a two-letter storefront such as us or gb.",
                    "apple_storefront_invalid",
                );
            }
            let credentials = AppleCredentials {
                developer_token,
                music_user_token,
                storefront,
            };
            if let Err(error) = apple::verify(&http, apple::API_BASE, &credentials).await {
                return provider_error(error);
            }
            let label = if credentials.music_user_token.is_some() {
                format!("Library access · {}", credentials.storefront.to_uppercase())
            } else {
                format!("Catalog only · {}", credentials.storefront.to_uppercase())
            };
            (
                serde_json::to_string(&credentials).unwrap_or_default(),
                Some(label),
            )
        }
        ImportService::YoutubeMusic => {
            let api_key = body.api_key.unwrap_or_default().trim().to_string();
            if api_key.len() < 20
                || api_key.len() > 100
                || !api_key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return bad_request(
                    "Paste a YouTube Data API v3 key.",
                    "youtube_api_key_invalid",
                );
            }
            let verified: Result<serde_json::Value, _> = crate::imports::http::send_json(
                http.get(format!("{}/i18nRegions", youtube::API_BASE))
                    .query(&[("part", "snippet"), ("hl", "en"), ("key", api_key.as_str())]),
            )
            .await;
            if let Err(error) = verified {
                return provider_error(match error {
                    ProviderError::Invalid(_) | ProviderError::Unauthorized => {
                        ProviderError::Invalid("Google rejected that API key.".to_string())
                    }
                    other => other,
                });
            }
            (
                serde_json::json!({ "api_key": api_key }).to_string(),
                Some("API key".to_string()),
            )
        }
        ImportService::Deezer => {
            let entered = body.profile_id.unwrap_or_default();
            let entered = entered.trim();
            // A Deezer profile id is a plain number; only digits reach the URL.
            let profile_id = if entered.is_empty() {
                None
            } else if entered.chars().all(|c| c.is_ascii_digit())
                && let Ok(id) = entered.parse::<u64>()
            {
                Some(id)
            } else {
                return bad_request(
                    "Use the number from your Deezer profile link.",
                    "deezer_profile_invalid",
                );
            };
            let label = match profile_id {
                None => "Public playlists".to_string(),
                Some(id) => match crate::imports::http::send_json::<DeezerUser>(
                    http.get(format!("{}/user/{id}", deezer::API_BASE)),
                )
                .await
                {
                    Ok(DeezerUser { name: Some(name) }) => name,
                    Ok(_) => return provider_error(ProviderError::NotFound),
                    Err(error) => return provider_error(error),
                },
            };
            (
                serde_json::json!({ "profile_id": profile_id.map(|id| id.to_string()) })
                    .to_string(),
                Some(label),
            )
        }
    };
    match blocking(&pool, move |connection| {
        save_connection(connection, user_id, service, &credentials, label.as_deref())
    })
    .await
    {
        Ok(_) => HttpResponse::NoContent().finish(),
        Err(error) => worker_failed("configure service", error),
    }
}

#[delete("/services/{service}")]
async fn disconnect_service(
    request: HttpRequest,
    service: web::Path<String>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match crate::api::auth::authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let service = match parse_service(&service) {
        Ok(service) => service,
        Err(response) => return response,
    };
    match blocking(&pool, move |connection| {
        delete_connection(connection, user_id, service)
    })
    .await
    {
        Ok(_) => HttpResponse::NoContent().finish(),
        Err(error) => worker_failed("disconnect", error),
    }
}

enum Credentials {
    Spotify(SpotifyCredentials),
    Apple(AppleCredentials),
    Youtube(String),
    Deezer(Option<String>),
}

/// Loads a connection, refreshing Spotify tokens when needed.
async fn credentials_for(
    pool: &web::Data<DbPool>,
    http: &reqwest::Client,
    user_id: i32,
    service: ImportService,
) -> Result<Credentials, HttpResponse> {
    let row = blocking(pool, move |connection| {
        load_connection(connection, user_id, service)
    })
    .await
    .map_err(|error| worker_failed("load connection", error))?;
    let not_connected = || {
        json_error(
            StatusCode::CONFLICT,
            "conflict",
            format!("Connect {} first.", service_descriptor(service).name),
            "import_service_not_connected",
        )
    };
    let Some(row) = row else {
        // Deezer works without any saved connection.
        return if service == ImportService::Deezer {
            Ok(Credentials::Deezer(None))
        } else {
            Err(not_connected())
        };
    };
    match service {
        ImportService::Spotify => {
            let mut credentials: SpotifyCredentials =
                serde_json::from_str(&row.credentials).map_err(|_| not_connected())?;
            match spotify::ensure_fresh(http, spotify::ACCOUNTS_BASE, &mut credentials, unix_now())
                .await
            {
                Ok(true) => {
                    let serialized = serde_json::to_string(&credentials).unwrap_or_default();
                    blocking(pool, move |connection| {
                        save_connection(connection, user_id, service, &serialized, None)
                    })
                    .await
                    .map_err(|error| worker_failed("save refreshed token", error))?;
                }
                Ok(false) => {}
                Err(error) => return Err(provider_error(error)),
            }
            Ok(Credentials::Spotify(credentials))
        }
        ImportService::AppleMusic => serde_json::from_str(&row.credentials)
            .map(Credentials::Apple)
            .map_err(|_| not_connected()),
        ImportService::YoutubeMusic => {
            let value: serde_json::Value =
                serde_json::from_str(&row.credentials).map_err(|_| not_connected())?;
            value["api_key"]
                .as_str()
                .map(|key| Credentials::Youtube(key.to_string()))
                .ok_or_else(not_connected)
        }
        ImportService::Deezer => {
            let value: serde_json::Value =
                serde_json::from_str(&row.credentials).unwrap_or_default();
            Ok(Credentials::Deezer(
                value["profile_id"].as_str().map(str::to_string),
            ))
        }
        ImportService::File => Err(not_connected()),
    }
}

#[get("/services/{service}/playlists")]
async fn service_playlists(
    request: HttpRequest,
    service: web::Path<String>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match crate::api::auth::authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let service = match parse_service(&service) {
        Ok(service) => service,
        Err(response) => return response,
    };
    let http = match client() {
        Ok(http) => http,
        Err(error) => return provider_error(error),
    };
    let credentials = match credentials_for(&pool, &http, user_id, service).await {
        Ok(credentials) => credentials,
        Err(response) => return response,
    };
    let playlists: Result<Vec<RemotePlaylist>, ProviderError> = match credentials {
        Credentials::Spotify(credentials) => {
            spotify::list_playlists(&http, spotify::API_BASE, &credentials.access_token).await
        }
        Credentials::Apple(credentials) if credentials.music_user_token.is_some() => {
            apple::list_library_playlists(&http, apple::API_BASE, &credentials).await
        }
        Credentials::Deezer(Some(profile_id)) => {
            deezer::list_profile_playlists(&http, deezer::API_BASE, &profile_id).await
        }
        // Key-only and public connections cannot enumerate private playlists.
        Credentials::Apple(_) | Credentials::Youtube(_) | Credentials::Deezer(None) => {
            Ok(Vec::new())
        }
    };
    match playlists {
        Ok(playlists) => HttpResponse::Ok().json(playlists),
        Err(error) => provider_error(error),
    }
}

// ---------------------------------------------------------------------------
// Spotify Connect handoff
// ---------------------------------------------------------------------------

#[get("/services/spotify/devices")]
async fn spotify_devices(request: HttpRequest, pool: web::Data<DbPool>) -> HttpResponse {
    let user_id = match crate::api::auth::authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let http = match client() {
        Ok(http) => http,
        Err(error) => return provider_error(error),
    };
    let Credentials::Spotify(credentials) =
        (match credentials_for(&pool, &http, user_id, ImportService::Spotify).await {
            Ok(credentials) => credentials,
            Err(response) => return response,
        })
    else {
        return worker_failed("spotify devices", "unexpected credentials");
    };
    match spotify::devices(&http, spotify::API_BASE, &credentials.access_token).await {
        Ok(devices) => HttpResponse::Ok().json(devices),
        Err(error) => provider_error(error),
    }
}

#[derive(Deserialize)]
struct HandoffRequest {
    song_ids: Vec<String>,
    device_id: Option<String>,
    #[serde(default)]
    position_ms: u64,
}

#[derive(Serialize)]
struct HandoffResult {
    started: bool,
    resolved: usize,
    unresolved_song_ids: Vec<String>,
}

#[post("/services/spotify/handoff")]
async fn spotify_handoff(
    request: HttpRequest,
    body: web::Json<HandoffRequest>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let user_id = match crate::api::auth::authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let body = body.into_inner();
    if body.song_ids.is_empty()
        || body.song_ids.len() > spotify::MAX_HANDOFF_TRACKS
        || body.song_ids.iter().any(|id| !valid_song_id(id))
        || body
            .device_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 128)
    {
        return bad_request(
            "Hand off between 1 and 50 tracks.",
            "spotify_handoff_invalid",
        );
    }
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    let http = match client() {
        Ok(http) => http,
        Err(error) => return provider_error(error),
    };
    let Credentials::Spotify(credentials) =
        (match credentials_for(&pool, &http, user_id, ImportService::Spotify).await {
            Ok(credentials) => credentials,
            Err(response) => return response,
        })
    else {
        return worker_failed("spotify handoff", "unexpected credentials");
    };
    let lookups: Vec<(String, HandoffTrack)> = body
        .song_ids
        .iter()
        .filter_map(|id| {
            let song = cache.song(id)?;
            Some((
                id.clone(),
                HandoffTrack {
                    title: song.name.clone(),
                    artist: song.artist.clone(),
                    isrc: song.isrc.clone(),
                },
            ))
        })
        .collect();
    let token = credentials.access_token.clone();
    let results: Vec<(String, Result<Option<String>, ProviderError>)> =
        futures::stream::iter(lookups.into_iter().map(|(id, track)| {
            let http = http.clone();
            let token = token.clone();
            async move {
                let uri = spotify::find_track_uri(&http, spotify::API_BASE, &token, &track).await;
                (id, uri)
            }
        }))
        .buffered(HANDOFF_SEARCH_CONCURRENCY)
        .collect()
        .await;
    let mut uris = Vec::new();
    let mut unresolved = Vec::new();
    for (id, result) in results {
        match result {
            Ok(Some(uri)) => uris.push(uri),
            Ok(None) => unresolved.push(id),
            Err(error) => return provider_error(error),
        }
    }
    let known: HashSet<&String> = body.song_ids.iter().collect();
    unresolved.extend(
        known
            .into_iter()
            .filter(|id| cache.song(id).is_none())
            .cloned(),
    );
    if uris.is_empty() {
        return HttpResponse::Ok().json(HandoffResult {
            started: false,
            resolved: 0,
            unresolved_song_ids: unresolved,
        });
    }
    match spotify::start_playback(
        &http,
        spotify::API_BASE,
        &credentials.access_token,
        body.device_id.as_deref(),
        &uris,
        body.position_ms,
    )
    .await
    {
        Ok(()) => HttpResponse::Ok().json(HandoffResult {
            started: true,
            resolved: uris.len(),
            unresolved_song_ids: unresolved,
        }),
        Err(error) => provider_error(error),
    }
}

// ---------------------------------------------------------------------------
// Import jobs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CreateJob {
    url: Option<String>,
    service: Option<String>,
    kind: Option<String>,
    id: Option<String>,
    storefront: Option<String>,
    file_name: Option<String>,
    content: Option<String>,
}

enum JobInput {
    Remote(ParsedSource),
    File(String, ImportedPlaylist),
}

fn resolve_job_input(body: CreateJob) -> Result<JobInput, HttpResponse> {
    if let Some(content) = body.content {
        let file_name = body
            .file_name
            .unwrap_or_else(|| "playlist.m3u".to_string())
            .chars()
            .take(255)
            .collect::<String>();
        return parse_playlist_file(&file_name, &content)
            .map(|playlist| JobInput::File(file_name, playlist))
            .map_err(|message| bad_request(message, "import_file_invalid"));
    }
    if let Some(url) = body.url {
        return parse_source_url(&url).map(JobInput::Remote).ok_or_else(|| {
            bad_request(
                "That link is not a Spotify, Apple Music, YouTube Music or Deezer playlist.",
                "import_url_unrecognised",
            )
        });
    }
    let service = body
        .service
        .as_deref()
        .and_then(ImportService::parse)
        .filter(|service| *service != ImportService::File);
    let kind = body.kind.as_deref().and_then(SourceKind::parse);
    let id = body.id.unwrap_or_default();
    match (service, kind) {
        (Some(service), Some(kind))
            if !id.is_empty()
                && id.len() <= 128
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                && !id.starts_with('.') =>
        {
            Ok(JobInput::Remote(ParsedSource {
                service,
                kind,
                id,
                storefront: body
                    .storefront
                    .filter(|storefront| apple::valid_storefront(storefront)),
            }))
        }
        _ => Err(bad_request(
            "Choose a playlist, paste a playlist link, or upload a playlist file.",
            "import_source_invalid",
        )),
    }
}

async fn fetch_remote(
    http: &reqwest::Client,
    credentials: Credentials,
    source: &ParsedSource,
    job_id: &str,
) -> Result<ImportedPlaylist, ProviderError> {
    let progress = |count: usize| set_progress(job_id, Some(count));
    match (credentials, &source.kind) {
        (Credentials::Spotify(credentials), SourceKind::SavedTracks) => {
            spotify::fetch_saved_tracks(
                http,
                spotify::API_BASE,
                &credentials.access_token,
                progress,
            )
            .await
        }
        (Credentials::Spotify(credentials), SourceKind::Playlist) => {
            spotify::fetch_playlist(
                http,
                spotify::API_BASE,
                &credentials.access_token,
                &source.id,
                progress,
            )
            .await
        }
        (Credentials::Apple(credentials), SourceKind::LibraryPlaylist) => {
            apple::fetch_library_playlist(http, apple::API_BASE, &credentials, &source.id, progress)
                .await
        }
        (Credentials::Apple(credentials), SourceKind::Playlist) => {
            let storefront = source
                .storefront
                .clone()
                .unwrap_or_else(|| credentials.storefront.clone());
            apple::fetch_catalog_playlist(
                http,
                apple::API_BASE,
                &credentials,
                &storefront,
                &source.id,
                progress,
            )
            .await
        }
        (Credentials::Youtube(api_key), SourceKind::Playlist) => {
            youtube::fetch_playlist(http, youtube::API_BASE, &api_key, &source.id, progress).await
        }
        (Credentials::Deezer(_), SourceKind::Playlist) => {
            deezer::fetch_playlist(http, deezer::API_BASE, &source.id, progress).await
        }
        _ => Err(ProviderError::Invalid(
            "That kind of source is not supported for this service.".to_string(),
        )),
    }
}

async fn finish_job(
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
    job_id: String,
    playlist: ImportedPlaylist,
) {
    let fail = |message: String| {
        let pool = pool.clone();
        let job_id = job_id.clone();
        async move {
            set_progress(&job_id, None);
            let id = job_id.clone();
            if let Err(error) = blocking(&pool, move |connection| {
                update_job(
                    connection,
                    &id,
                    &JobUpdate {
                        status: Some("failed"),
                        error: Some(&message),
                        ..JobUpdate::default()
                    },
                    unix_now(),
                )
            })
            .await
            {
                tracing::warn!(%error, job_id, "could not record failed import");
            }
        }
    };
    let name = playlist.name.clone();
    let source_url = playlist.source_url.clone();
    {
        let id = job_id.clone();
        let _ = blocking(&pool, move |connection| {
            update_job(
                connection,
                &id,
                &JobUpdate {
                    status: Some("matching"),
                    name: Some(&name),
                    source_url: source_url.as_deref(),
                    ..JobUpdate::default()
                },
                unix_now(),
            )
        })
        .await;
    }
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(_) => {
            fail("Your library is not ready yet. Try again after it finishes loading.".to_string())
                .await;
            return;
        }
    };
    let tracks = playlist.tracks;
    let items = match web::block(move || {
        let index = MatchIndex::from_artists(&cache.artists);
        match_items(&index, tracks)
    })
    .await
    {
        Ok(items) => items,
        Err(error) => {
            fail(format!("Matching failed: {error}")).await;
            return;
        }
    };
    let id = job_id.clone();
    let result = blocking(&pool, move |connection| {
        update_job(
            connection,
            &id,
            &JobUpdate {
                status: Some("ready"),
                items: Some(&items),
                ..JobUpdate::default()
            },
            unix_now(),
        )
    })
    .await;
    set_progress(&job_id, None);
    if let Err(error) = result {
        tracing::error!(%error, job_id, "could not store import results");
        fail("Could not save the import results.".to_string()).await;
    }
}

#[post("/jobs")]
async fn create_job(
    request: HttpRequest,
    body: web::Json<CreateJob>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let user_id = match crate::api::auth::authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let input = match resolve_job_input(body.into_inner()) {
        Ok(input) => input,
        Err(response) => return response,
    };
    if let Err(readiness) = lifecycle.cache().await {
        return library_unavailable_response(readiness);
    }
    let job_id = uuid::Uuid::new_v4().to_string();
    let now = unix_now();
    let (service, kind, source_ref, name, remote) = match &input {
        JobInput::File(file_name, playlist) => (
            ImportService::File,
            SourceKind::File,
            file_name.clone(),
            playlist.name.clone(),
            None,
        ),
        JobInput::Remote(source) => (
            source.service,
            source.kind.clone(),
            source.id.clone(),
            format!("{} playlist", service_descriptor(source.service).name),
            Some(source.clone()),
        ),
    };
    // Resolve credentials up front so "not connected" is reported now.
    let http = match client() {
        Ok(http) => http,
        Err(error) => return provider_error(error),
    };
    let credentials = match &remote {
        Some(source) => match credentials_for(&pool, &http, user_id, source.service).await {
            Ok(credentials) => Some(credentials),
            Err(response) => return response,
        },
        None => None,
    };
    {
        let job_id = job_id.clone();
        let kind = kind.as_str().to_string();
        let source_ref = source_ref.clone();
        let name = name.clone();
        if let Err(error) = blocking(&pool, move |connection| {
            insert_job(
                connection,
                &NewJob {
                    id: &job_id,
                    user_id,
                    service,
                    source_kind: &kind,
                    source_ref: &source_ref,
                    name: &name,
                    status: "fetching",
                    source_url: None,
                    now,
                },
            )
        })
        .await
        {
            return worker_failed("create job", error);
        }
    }
    set_progress(&job_id, Some(0));
    let task_pool = pool.clone();
    let task_lifecycle = lifecycle.clone();
    let task_job_id = job_id.clone();
    actix_web::rt::spawn(async move {
        let playlist = match (input, credentials) {
            (JobInput::File(_, playlist), _) => Ok(playlist),
            (JobInput::Remote(source), Some(credentials)) => {
                fetch_remote(&http, credentials, &source, &task_job_id).await
            }
            (JobInput::Remote(_), None) => Err(ProviderError::Invalid("Not connected.".into())),
        };
        match playlist {
            Ok(playlist) => finish_job(task_pool, task_lifecycle, task_job_id, playlist).await,
            Err(error) => {
                set_progress(&task_job_id, None);
                let message = error.to_string();
                let id = task_job_id.clone();
                if let Err(error) = blocking(&task_pool, move |connection| {
                    update_job(
                        connection,
                        &id,
                        &JobUpdate {
                            status: Some("failed"),
                            error: Some(&message),
                            ..JobUpdate::default()
                        },
                        unix_now(),
                    )
                })
                .await
                {
                    tracing::warn!(%error, job_id = task_job_id, "could not record failed import");
                }
            }
        }
    });
    match blocking(&pool, move |connection| {
        load_job(connection, user_id, &job_id)
    })
    .await
    {
        Ok(Some((row, _))) => HttpResponse::Accepted().json(summary(row, unix_now())),
        Ok(None) => not_found("Import not found.", "import_not_found"),
        Err(error) => worker_failed("load created job", error),
    }
}

#[get("/jobs")]
async fn jobs(request: HttpRequest, pool: web::Data<DbPool>) -> HttpResponse {
    let user_id = match crate::api::auth::authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    match blocking(&pool, move |connection| list_jobs(connection, user_id)).await {
        Ok(rows) => {
            let now = unix_now();
            HttpResponse::Ok().json(
                rows.into_iter()
                    .map(|row| summary(row, now))
                    .collect::<Vec<_>>(),
            )
        }
        Err(error) => worker_failed("list jobs", error),
    }
}

fn valid_job_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok()
}

#[get("/jobs/{id}")]
async fn job_route(
    request: HttpRequest,
    id: web::Path<String>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let user_id = match crate::api::auth::authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let job_id = id.into_inner();
    if !valid_job_id(&job_id) {
        return not_found("Import not found.", "import_not_found");
    }
    let cache = lifecycle.cache().await.ok();
    match blocking(&pool, move |connection| {
        load_job(connection, user_id, &job_id)
    })
    .await
    {
        Ok(Some((row, items))) => HttpResponse::Ok().json(job_detail(row, items, cache.as_deref())),
        Ok(None) => not_found("Import not found.", "import_not_found"),
        Err(error) => worker_failed("load job", error),
    }
}

/// Why a job request was refused inside a database worker (responses are not `Send`).
enum JobRefusal {
    NotFound,
    NotReady,
    Invalid(String, &'static str),
}

fn refusal_response(refusal: JobRefusal) -> HttpResponse {
    match refusal {
        JobRefusal::NotFound => not_found("Import not found.", "import_not_found"),
        JobRefusal::NotReady => json_error(
            StatusCode::CONFLICT,
            "conflict",
            "This import is still running.",
            "import_not_ready",
        ),
        JobRefusal::Invalid(message, code) => bad_request(message, code),
    }
}

#[derive(Deserialize)]
struct UpdateItems {
    actions: Vec<ItemAction>,
}

#[patch("/jobs/{id}/items")]
async fn update_items(
    request: HttpRequest,
    id: web::Path<String>,
    body: web::Json<UpdateItems>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let user_id = match crate::api::auth::authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let job_id = id.into_inner();
    if !valid_job_id(&job_id) {
        return not_found("Import not found.", "import_not_found");
    }
    let actions = body.into_inner().actions;
    if actions.is_empty() || actions.len() > MAX_ITEM_UPDATES {
        return bad_request("Send between 1 and 2000 changes.", "import_actions_invalid");
    }
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    let needs_index = actions
        .iter()
        .any(|action| matches!(action, ItemAction::Reset { .. }));
    let block_cache = cache.clone();
    let result = blocking(&pool, move |connection| {
        connection.transaction(|connection| {
            let Some((row, mut items)) = load_job(connection, user_id, &job_id)? else {
                return Ok(Err(JobRefusal::NotFound));
            };
            if row.status != "ready" {
                return Ok(Err(JobRefusal::NotReady));
            }
            let index = needs_index.then(|| MatchIndex::from_artists(&block_cache.artists));
            if let Err(error) = apply_actions(
                &mut items,
                &actions,
                |song_id| block_cache.song(song_id).is_some(),
                index.as_ref(),
            ) {
                let message = match error {
                    ApplyError::UnknownPosition(position) => {
                        format!("Track {} is not part of this import.", position + 1)
                    }
                    ApplyError::NothingToAccept(position) => {
                        format!("Track {} has no match to accept.", position + 1)
                    }
                    ApplyError::UnknownSong(_) => "That song is not in your library.".to_string(),
                };
                return Ok(Err(JobRefusal::Invalid(message, "import_action_invalid")));
            }
            update_job(
                connection,
                &job_id,
                &JobUpdate {
                    items: Some(&items),
                    ..JobUpdate::default()
                },
                unix_now(),
            )?;
            let (row, items) = load_job(connection, user_id, &job_id)?.expect("job just updated");
            Ok(Ok((row, items)))
        })
    })
    .await;
    match result {
        Ok(Ok((row, items))) => HttpResponse::Ok().json(job_detail(row, items, Some(&cache))),
        Ok(Err(refusal)) => refusal_response(refusal),
        Err(error) => worker_failed("update items", error),
    }
}

#[post("/jobs/{id}/rematch")]
async fn rematch(
    request: HttpRequest,
    id: web::Path<String>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let user_id = match crate::api::auth::authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let job_id = id.into_inner();
    if !valid_job_id(&job_id) {
        return not_found("Import not found.", "import_not_found");
    }
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    let block_cache = cache.clone();
    let result = blocking(&pool, move |connection| {
        let Some((row, mut items)) = load_job(connection, user_id, &job_id)? else {
            return Ok(None);
        };
        if row.status != "ready" {
            return Ok(Some((row, items)));
        }
        let index = MatchIndex::from_artists(&block_cache.artists);
        rematch_items(&index, &mut items);
        update_job(
            connection,
            &job_id,
            &JobUpdate {
                items: Some(&items),
                ..JobUpdate::default()
            },
            unix_now(),
        )?;
        load_job(connection, user_id, &job_id)
    })
    .await;
    match result {
        Ok(Some((row, items))) => HttpResponse::Ok().json(job_detail(row, items, Some(&cache))),
        Ok(None) => not_found("Import not found.", "import_not_found"),
        Err(error) => worker_failed("rematch", error),
    }
}

#[derive(Deserialize)]
struct SavePlaylist {
    name: Option<String>,
}

fn import_description(service: &str, included: usize, total: usize) -> String {
    match ImportService::parse(service).filter(|service| *service != ImportService::File) {
        Some(service) => {
            let name = service_descriptor(service).name;
            format!(
                "Imported from {name}: {included} of {total} tracks matched to your library. No audio is streamed from {name}."
            )
        }
        None => format!(
            "Imported from a playlist file: {included} of {total} tracks matched to your library."
        ),
    }
}

#[post("/jobs/{id}/playlist")]
async fn save_job_playlist(
    request: HttpRequest,
    id: web::Path<String>,
    body: web::Json<SavePlaylist>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match crate::api::auth::authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let job_id = id.into_inner();
    if !valid_job_id(&job_id) {
        return not_found("Import not found.", "import_not_found");
    }
    let requested_name = body
        .into_inner()
        .name
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty());
    if requested_name
        .as_ref()
        .is_some_and(|name| name.chars().count() > MAX_PLAYLIST_NAME_CHARACTERS)
    {
        return bad_request(
            "Playlist name must contain 1 to 200 characters.",
            "playlist_name_invalid",
        );
    }
    let result = blocking(&pool, move |connection| {
        let Some((row, items)) = load_job(connection, user_id, &job_id)? else {
            return Ok(Err(JobRefusal::NotFound));
        };
        if row.status != "ready" {
            return Ok(Err(JobRefusal::NotReady));
        }
        let song_ids: Vec<String> = items
            .iter()
            .filter(|item| item.included())
            .filter_map(|item| item.song_id.clone())
            .collect();
        if song_ids.is_empty() {
            return Ok(Err(JobRefusal::Invalid(
                "No tracks are matched yet. Review the import first.".to_string(),
                "import_nothing_matched",
            )));
        }
        let name: String = requested_name
            .unwrap_or_else(|| row.name.clone())
            .chars()
            .take(MAX_PLAYLIST_NAME_CHARACTERS)
            .collect();
        let description = import_description(&row.service, song_ids.len(), items.len());
        let outcome = save_playlist(
            connection,
            user_id,
            row.playlist_id,
            &name,
            &description,
            &song_ids,
        )?;
        if let SavePlaylistOutcome::Created { playlist_id, .. } = outcome {
            update_job(
                connection,
                &job_id,
                &JobUpdate {
                    playlist_id: Some(playlist_id),
                    ..JobUpdate::default()
                },
                unix_now(),
            )?;
        }
        Ok(Ok(outcome))
    })
    .await;
    match result {
        Ok(Ok(SavePlaylistOutcome::Created { playlist_id, added })) => HttpResponse::Created()
            .json(
                serde_json::json!({ "playlist_id": playlist_id, "added": added, "created": true }),
            ),
        Ok(Ok(SavePlaylistOutcome::Updated { playlist_id, added })) => HttpResponse::Ok().json(
            serde_json::json!({ "playlist_id": playlist_id, "added": added, "created": false }),
        ),
        Ok(Ok(SavePlaylistOutcome::CapacityReached)) => {
            crate::api::error::conflict("Playlist capacity reached.", "playlist_capacity_reached")
        }
        Ok(Err(refusal)) => refusal_response(refusal),
        Err(error) => worker_failed("save playlist", error),
    }
}

#[delete("/jobs/{id}")]
async fn remove_job(
    request: HttpRequest,
    id: web::Path<String>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match crate::api::auth::authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let job_id = id.into_inner();
    if !valid_job_id(&job_id) {
        return not_found("Import not found.", "import_not_found");
    }
    match blocking(&pool, move |connection| {
        delete_job(connection, user_id, &job_id)
    })
    .await
    {
        Ok(1) => HttpResponse::NoContent().finish(),
        Ok(_) => not_found("Import not found.", "import_not_found"),
        Err(error) => worker_failed("delete job", error),
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/imports")
            .app_data(web::JsonConfig::default().limit(MAX_IMPORT_JSON_BYTES))
            .service(list_services)
            .service(spotify_authorize)
            .service(spotify_callback)
            .service(spotify_devices)
            .service(spotify_handoff)
            .service(service_playlists)
            .service(configure_service)
            .service(disconnect_service)
            .service(create_job)
            .service(jobs)
            .service(job_route)
            .service(update_items)
            .service(rematch)
            .service(save_job_playlist)
            .service(remove_job),
    );
}

#[cfg(test)]
mod tests {
    use diesel::connection::SimpleConnection;

    use super::*;
    use crate::library::matching::{LibraryTrack, SourceTrack};

    const MIGRATION: &str =
        include_str!("../../migrations/2026-10-24-120000_streaming_imports/up.sql");

    fn database() -> SqliteConnection {
        let mut connection =
            SqliteConnection::establish(":memory:").expect("in-memory sqlite connection");
        connection
            .batch_execute(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE user (id INTEGER PRIMARY KEY);
                 INSERT INTO user (id) VALUES (1), (2);
                 CREATE TABLE song (id TEXT PRIMARY KEY);
                 INSERT INTO song (id) VALUES ('s1'), ('s2'), ('s3');
                 CREATE TABLE playlist (
                   id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL,
                   description TEXT, updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP);
                 CREATE TABLE _playlist_to_user (
                   rowid INTEGER PRIMARY KEY, a INTEGER NOT NULL, b INTEGER NOT NULL);
                 CREATE TABLE _playlist_to_song (
                   rowid INTEGER PRIMARY KEY AUTOINCREMENT, a INTEGER NOT NULL, b TEXT NOT NULL,
                   added_by INTEGER, position INTEGER);",
            )
            .expect("fixture schema");
        connection
            .batch_execute(MIGRATION)
            .expect("import migration");
        connection
    }

    fn item(position: usize, status: ItemStatus, song_id: Option<&str>) -> JobItem {
        JobItem {
            position,
            source: ImportedTrack {
                track: SourceTrack {
                    title: format!("Track {position}"),
                    ..SourceTrack::default()
                },
                ..ImportedTrack::default()
            },
            status,
            song_id: song_id.map(str::to_string),
            confidence: 0.7,
            reasons: Vec::new(),
            candidates: vec![MatchCandidate {
                song_id: "s3".into(),
                confidence: 0.6,
                reasons: vec![MatchReason::SimilarTitle],
            }],
        }
    }

    fn new_job<'a>(id: &'a str, user_id: i32, now: i64) -> NewJob<'a> {
        NewJob {
            id,
            user_id,
            service: ImportService::Spotify,
            source_kind: "playlist",
            source_ref: "abc",
            name: "Road trip",
            status: "fetching",
            source_url: None,
            now,
        }
    }

    #[test]
    fn connections_are_per_user_and_never_expose_secrets() {
        let mut connection = database();
        let credentials = serde_json::to_string(&SpotifyCredentials {
            client_id: "0123456789abcdef0123456789abcdef".into(),
            access_token: "secret-access".into(),
            refresh_token: "secret-refresh".into(),
            expires_at: 10,
            scope: "user-modify-playback-state".into(),
            product: Some("premium".into()),
        })
        .unwrap();
        save_connection(
            &mut connection,
            1,
            ImportService::Spotify,
            &credentials,
            Some("Ann"),
        )
        .unwrap();
        // A refresh without a label keeps the previous label.
        save_connection(
            &mut connection,
            1,
            ImportService::Spotify,
            &credentials,
            None,
        )
        .unwrap();
        let rows = load_connections(&mut connection, 1).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].account_label.as_deref(), Some("Ann"));
        assert!(load_connections(&mut connection, 2).unwrap().is_empty());

        let details = connection_details(ImportService::Spotify, &rows[0].credentials).to_string();
        assert!(!details.contains("secret"));
        assert!(details.contains("…cdef"));
        assert!(details.contains("\"premium\":true"));

        delete_connection(&mut connection, 1, ImportService::Spotify).unwrap();
        assert!(
            load_connection(&mut connection, 1, ImportService::Spotify)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn oauth_state_is_single_use_owner_bound_and_expires() {
        let mut connection = database();
        let now = 1_000_000;
        insert_auth_state(
            &mut connection,
            &PendingAuth {
                state: "state-a",
                user_id: 1,
                service: ImportService::Spotify,
                verifier: "verifier",
                client_id: "client",
                redirect_uri: "https://x/imports",
                now,
            },
        )
        .unwrap();
        assert!(
            take_auth_state(&mut connection, "state-a", 2, ImportService::Spotify, now)
                .unwrap()
                .is_none(),
            "another user cannot redeem the state"
        );
        insert_auth_state(
            &mut connection,
            &PendingAuth {
                state: "state-b",
                user_id: 1,
                service: ImportService::Spotify,
                verifier: "verifier-b",
                client_id: "client",
                redirect_uri: "https://x/imports",
                now,
            },
        )
        .unwrap();
        let taken = take_auth_state(&mut connection, "state-b", 1, ImportService::Spotify, now)
            .unwrap()
            .expect("owner redeems state");
        assert_eq!(taken.code_verifier, "verifier-b");
        assert!(
            take_auth_state(&mut connection, "state-b", 1, ImportService::Spotify, now)
                .unwrap()
                .is_none(),
            "a state is single-use"
        );
        insert_auth_state(
            &mut connection,
            &PendingAuth {
                state: "state-c",
                user_id: 1,
                service: ImportService::Spotify,
                verifier: "v",
                client_id: "c",
                redirect_uri: "https://x",
                now,
            },
        )
        .unwrap();
        assert!(
            take_auth_state(
                &mut connection,
                "state-c",
                1,
                ImportService::Spotify,
                now + AUTH_STATE_TTL_SECONDS + 1
            )
            .unwrap()
            .is_none(),
            "expired states are rejected"
        );
    }

    #[test]
    fn jobs_report_counts_and_are_scoped_to_their_owner() {
        let mut connection = database();
        let id = "6f1c1d1e-6b8e-4a59-9b6f-1d7f6f0a0001";
        insert_job(&mut connection, &new_job(id, 1, 100)).unwrap();
        let items = vec![
            item(0, ItemStatus::Matched, Some("s1")),
            item(1, ItemStatus::Review, Some("s2")),
            item(2, ItemStatus::Unmatched, None),
            item(3, ItemStatus::Confirmed, Some("s3")),
            item(4, ItemStatus::Skipped, Some("s1")),
        ];
        update_job(
            &mut connection,
            id,
            &JobUpdate {
                status: Some("ready"),
                items: Some(&items),
                ..JobUpdate::default()
            },
            200,
        )
        .unwrap();
        let (row, loaded) = load_job(&mut connection, 1, id).unwrap().expect("job");
        assert_eq!(loaded, items);
        assert_eq!(
            (
                row.total,
                row.matched,
                row.review,
                row.unmatched,
                row.skipped
            ),
            (5, 2, 1, 1, 1)
        );
        assert_eq!(row.updated_at, 200);
        assert!(load_job(&mut connection, 2, id).unwrap().is_none());
        assert_eq!(list_jobs(&mut connection, 1).unwrap().len(), 1);
        assert_eq!(delete_job(&mut connection, 2, id).unwrap(), 0);
        assert_eq!(delete_job(&mut connection, 1, id).unwrap(), 1);
    }

    #[test]
    fn interrupted_jobs_are_reported_as_failed() {
        let mut connection = database();
        let id = "6f1c1d1e-6b8e-4a59-9b6f-1d7f6f0a0002";
        insert_job(&mut connection, &new_job(id, 1, 100)).unwrap();
        let row = list_jobs(&mut connection, 1).unwrap().remove(0);
        let view = summary(row, 100 + STALE_JOB_SECONDS + 1);
        assert_eq!(view.status, "failed");
        assert!(view.error.unwrap().contains("interrupted"));
    }

    #[test]
    fn review_actions_confirm_choose_skip_and_validate() {
        let mut items = vec![
            item(0, ItemStatus::Review, Some("s2")),
            item(1, ItemStatus::Unmatched, None),
            item(2, ItemStatus::Matched, Some("s1")),
        ];
        let exists = |id: &str| matches!(id, "s1" | "s2" | "s3");
        apply_actions(
            &mut items,
            &[
                ItemAction::Accept { position: 0 },
                ItemAction::Choose {
                    position: 1,
                    song_id: "s3".into(),
                },
                ItemAction::Skip { position: 2 },
            ],
            exists,
            None,
        )
        .unwrap();
        assert_eq!(items[0].status, ItemStatus::Confirmed);
        assert_eq!(items[1].status, ItemStatus::Confirmed);
        assert_eq!(items[1].song_id.as_deref(), Some("s3"));
        assert_eq!(
            items[1].confidence, 0.6,
            "uses the chosen candidate's score"
        );
        assert_eq!(items[2].status, ItemStatus::Skipped);
        assert!(!items[2].included());

        let mut fresh = vec![item(0, ItemStatus::Unmatched, None)];
        assert_eq!(
            apply_actions(
                &mut fresh,
                &[ItemAction::Accept { position: 0 }],
                exists,
                None
            ),
            Err(ApplyError::NothingToAccept(0))
        );
        assert_eq!(
            apply_actions(
                &mut fresh,
                &[ItemAction::Skip { position: 9 }],
                exists,
                None
            ),
            Err(ApplyError::UnknownPosition(9))
        );
        assert_eq!(
            apply_actions(
                &mut fresh,
                &[ItemAction::Choose {
                    position: 0,
                    song_id: "nope".into()
                }],
                exists,
                None
            ),
            Err(ApplyError::UnknownSong("nope".into()))
        );
    }

    #[test]
    fn rematching_respects_decisions_and_picks_up_new_library_tracks() {
        let tracks = vec![
            ImportedTrack {
                track: SourceTrack {
                    title: "Blue Monday".into(),
                    artists: vec!["New Order".into()],
                    duration_ms: Some(448_000),
                    ..SourceTrack::default()
                },
                ..ImportedTrack::default()
            },
            ImportedTrack {
                track: SourceTrack {
                    title: "Ceremony".into(),
                    artists: vec!["New Order".into()],
                    ..SourceTrack::default()
                },
                ..ImportedTrack::default()
            },
        ];
        let empty = MatchIndex::from_tracks(Vec::<LibraryTrack>::new());
        let mut items = match_items(&empty, tracks);
        assert!(
            items
                .iter()
                .all(|item| item.status == ItemStatus::Unmatched)
        );
        items[1].status = ItemStatus::Skipped;

        let library = MatchIndex::from_tracks([
            LibraryTrack {
                id: "bm".into(),
                title: "Blue Monday".into(),
                artist: "New Order".into(),
                album: "Power, Corruption & Lies".into(),
                duration_seconds: 448.0,
                ..LibraryTrack::default()
            },
            LibraryTrack {
                id: "ce".into(),
                title: "Ceremony".into(),
                artist: "New Order".into(),
                duration_seconds: 260.0,
                ..LibraryTrack::default()
            },
        ]);
        assert_eq!(rematch_items(&library, &mut items), 1);
        assert_eq!(items[0].status, ItemStatus::Matched);
        assert_eq!(items[0].song_id.as_deref(), Some("bm"));
        assert_eq!(items[1].status, ItemStatus::Skipped, "decisions are kept");
    }

    #[test]
    fn saving_creates_once_then_appends_only_new_known_songs() {
        let mut connection = database();
        let first = save_playlist(
            &mut connection,
            1,
            None,
            "Imported",
            "desc",
            &["s1".into(), "missing".into(), "s1".into(), "s2".into()],
        )
        .unwrap();
        let SavePlaylistOutcome::Created { playlist_id, added } = first else {
            panic!("expected a new playlist, got {first:?}");
        };
        assert_eq!(added, 2);
        let second = save_playlist(
            &mut connection,
            1,
            Some(playlist_id),
            "Imported",
            "desc",
            &["s1".into(), "s2".into(), "s3".into()],
        )
        .unwrap();
        assert_eq!(
            second,
            SavePlaylistOutcome::Updated {
                playlist_id,
                added: 1
            }
        );
        // Another user's playlist id is never appended to.
        let other = save_playlist(
            &mut connection,
            2,
            Some(playlist_id),
            "Mine",
            "desc",
            &["s1".into()],
        )
        .unwrap();
        assert!(
            matches!(other, SavePlaylistOutcome::Created { playlist_id: id, .. } if id != playlist_id)
        );
    }

    #[test]
    fn job_input_accepts_links_files_and_explicit_sources() {
        let link = resolve_job_input(CreateJob {
            url: Some("https://www.deezer.com/en/playlist/908622995".into()),
            service: None,
            kind: None,
            id: None,
            storefront: None,
            file_name: None,
            content: None,
        });
        assert!(matches!(
            link,
            Ok(JobInput::Remote(ParsedSource {
                service: ImportService::Deezer,
                ..
            }))
        ));
        let file = resolve_job_input(CreateJob {
            url: None,
            service: None,
            kind: None,
            id: None,
            storefront: None,
            file_name: Some("mix.m3u".into()),
            content: Some("#EXTM3U\n#EXTINF:1,A - B\nb.mp3\n".into()),
        });
        assert!(matches!(file, Ok(JobInput::File(_, _))));
        let liked = resolve_job_input(CreateJob {
            url: None,
            service: Some("spotify".into()),
            kind: Some("saved_tracks".into()),
            id: Some("liked".into()),
            storefront: None,
            file_name: None,
            content: None,
        });
        assert!(matches!(
            liked,
            Ok(JobInput::Remote(ParsedSource {
                kind: SourceKind::SavedTracks,
                ..
            }))
        ));
        for bad in [
            ("file", "playlist", "x"),
            ("spotify", "playlist", "../x"),
            ("spotify", "bogus", "x"),
        ] {
            assert!(
                resolve_job_input(CreateJob {
                    url: None,
                    service: Some(bad.0.into()),
                    kind: Some(bad.1.into()),
                    id: Some(bad.2.into()),
                    storefront: None,
                    file_name: None,
                    content: None,
                })
                .is_err(),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn provider_errors_never_use_401() {
        for error in [
            ProviderError::Unauthorized,
            ProviderError::NotFound,
            ProviderError::RateLimited,
            ProviderError::Forbidden("x".into()),
            ProviderError::Invalid("x".into()),
            ProviderError::Upstream("x".into()),
        ] {
            assert_ne!(provider_error(error).status(), StatusCode::UNAUTHORIZED);
        }
    }

    #[test]
    fn descriptions_are_honest_about_audio() {
        let text = import_description("spotify", 40, 50);
        assert!(text.contains("40 of 50"));
        assert!(text.contains("No audio is streamed from Spotify"));
        assert_eq!(
            import_description("file", 3, 4),
            "Imported from a playlist file: 3 of 4 tracks matched to your library."
        );
    }
}
