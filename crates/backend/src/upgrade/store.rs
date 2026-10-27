//! Database rows for Better versions: requests to FETCH and their jobs,
//! spectral checks, enhanced files and the enhance queue.

use diesel::QueryResult;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Integer, Nullable, Text};
use diesel::sqlite::SqliteConnection;
use serde::Serialize;

use super::fetch::FetchJob;
use crate::library::tools::spectral::SpectralResult;

/// Where a request to FETCH stands, from Mujin Music's side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestState {
    /// Sent; FETCH is planning or queueing it.
    Requested,
    /// FETCH is downloading.
    Downloading,
    /// Another app (Lidarr and the like) owns the job now.
    HandedOff,
    /// FETCH finished; Mujin Music is rescanning and checking the copy.
    Importing,
    /// The new copy is verified lossless and plays from now on.
    Upgraded,
    /// The new copy is a transcode of a lossy file: the original stays.
    FakeLossless,
    /// The new files did not match the album: the original stays.
    NoMatch,
    /// FETCH found nothing to download.
    NotFound,
    Failed,
    Cancelled,
}

impl RequestState {
    pub const ALL: [Self; 10] = [
        Self::Requested,
        Self::Downloading,
        Self::HandedOff,
        Self::Importing,
        Self::Upgraded,
        Self::FakeLossless,
        Self::NoMatch,
        Self::NotFound,
        Self::Failed,
        Self::Cancelled,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Downloading => "downloading",
            Self::HandedOff => "handed_off",
            Self::Importing => "importing",
            Self::Upgraded => "upgraded",
            Self::FakeLossless => "fake_lossless",
            Self::NoMatch => "no_match",
            Self::NotFound => "not_found",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == value)
    }

    /// Still waiting on FETCH or on the import.
    pub fn is_open(self) -> bool {
        matches!(
            self,
            Self::Requested | Self::Downloading | Self::HandedOff | Self::Importing
        )
    }
}

#[derive(QueryableByName, Clone, Debug, PartialEq)]
pub struct RequestRow {
    #[diesel(sql_type = BigInt)]
    pub id: i64,
    #[diesel(sql_type = Text)]
    pub album_id: String,
    #[diesel(sql_type = Text)]
    pub album_title: String,
    #[diesel(sql_type = Text)]
    pub artist_name: String,
    #[diesel(sql_type = Text)]
    pub query: String,
    #[diesel(sql_type = Text)]
    pub reason: String,
    #[diesel(sql_type = Text)]
    pub origin: String,
    #[diesel(sql_type = Text)]
    pub state: String,
    #[diesel(sql_type = Text)]
    pub message: String,
    #[diesel(sql_type = BigInt)]
    pub next_poll_at: i64,
    #[diesel(sql_type = Integer)]
    pub poll_failures: i32,
    #[diesel(sql_type = Integer)]
    pub quiet_polls: i32,
    #[diesel(sql_type = Nullable<Text>)]
    pub result_album_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub result_folder: Option<String>,
    #[diesel(sql_type = Text)]
    pub created_at: String,
    #[diesel(sql_type = Text)]
    pub updated_at: String,
}

impl RequestRow {
    pub fn state(&self) -> RequestState {
        RequestState::parse(&self.state).unwrap_or(RequestState::Failed)
    }
}

const REQUEST_COLUMNS: &str =
    "id, album_id, album_title, artist_name, query, reason, origin, state,
    message, next_poll_at, poll_failures, quiet_polls, result_album_id, result_folder,
    CAST(created_at AS TEXT) AS created_at, CAST(updated_at AS TEXT) AS updated_at";

#[derive(QueryableByName, Clone, Debug, PartialEq, Serialize)]
pub struct JobRow {
    #[diesel(sql_type = BigInt)]
    #[serde(skip)]
    pub request_id: i64,
    #[diesel(sql_type = Text)]
    pub job_id: String,
    #[diesel(sql_type = Text)]
    pub status: String,
    #[diesel(sql_type = Nullable<Double>)]
    pub progress: Option<f64>,
    #[diesel(sql_type = Text)]
    pub speed: String,
    #[diesel(sql_type = Text)]
    pub eta: String,
    #[diesel(sql_type = Text)]
    pub title: String,
    #[diesel(sql_type = Text)]
    pub engine: String,
    #[diesel(sql_type = Text)]
    pub dest: String,
    #[diesel(sql_type = Text)]
    pub error: String,
    #[diesel(sql_type = Text)]
    pub note: String,
    #[diesel(sql_type = Text)]
    pub size: String,
}

pub struct NewRequest<'a> {
    pub album_id: &'a str,
    pub album_title: &'a str,
    pub artist_name: &'a str,
    pub query: &'a str,
    pub reason: &'a str,
    pub automatic: bool,
    pub requested_by: Option<i32>,
}

#[derive(QueryableByName)]
struct IdRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
}

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

pub fn insert_request(
    connection: &mut SqliteConnection,
    request: &NewRequest<'_>,
    state: RequestState,
    message: &str,
) -> QueryResult<i64> {
    diesel::sql_query(
        "INSERT INTO upgrade_request
            (album_id, album_title, artist_name, query, reason, origin, requested_by, state, message)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Text, _>(request.album_id)
    .bind::<Text, _>(request.album_title)
    .bind::<Text, _>(request.artist_name)
    .bind::<Text, _>(request.query)
    .bind::<Text, _>(request.reason)
    .bind::<Text, _>(if request.automatic {
        "automatic"
    } else {
        "manual"
    })
    .bind::<Nullable<Integer>, _>(request.requested_by)
    .bind::<Text, _>(state.as_str())
    .bind::<Text, _>(message)
    .execute(connection)?;
    Ok(diesel::sql_query("SELECT last_insert_rowid() AS id")
        .get_result::<IdRow>(connection)?
        .id)
}

pub fn request(connection: &mut SqliteConnection, id: i64) -> QueryResult<Option<RequestRow>> {
    Ok(diesel::sql_query(format!(
        "SELECT {REQUEST_COLUMNS} FROM upgrade_request WHERE id = ?"
    ))
    .bind::<BigInt, _>(id)
    .load::<RequestRow>(connection)?
    .into_iter()
    .next())
}

/// The newest requests first (at most `limit`).
pub fn requests(connection: &mut SqliteConnection, limit: i64) -> QueryResult<Vec<RequestRow>> {
    diesel::sql_query(format!(
        "SELECT {REQUEST_COLUMNS} FROM upgrade_request ORDER BY id DESC LIMIT ?"
    ))
    .bind::<BigInt, _>(limit)
    .load(connection)
}

/// The newest request for each album in `album_ids`, or for every album.
pub fn latest_for_album(
    connection: &mut SqliteConnection,
    album_id: &str,
) -> QueryResult<Option<RequestRow>> {
    Ok(diesel::sql_query(format!(
        "SELECT {REQUEST_COLUMNS} FROM upgrade_request WHERE album_id = ? ORDER BY id DESC LIMIT 1"
    ))
    .bind::<Text, _>(album_id)
    .load::<RequestRow>(connection)?
    .into_iter()
    .next())
}

/// Open requests whose next poll is due.
pub fn due_requests(connection: &mut SqliteConnection, now: i64) -> QueryResult<Vec<RequestRow>> {
    diesel::sql_query(format!(
        "SELECT {REQUEST_COLUMNS} FROM upgrade_request
         WHERE state IN ('requested', 'downloading', 'handed_off', 'importing') AND next_poll_at <= ?
         ORDER BY next_poll_at, id"
    ))
    .bind::<BigInt, _>(now)
    .load(connection)
}

pub fn open_request_for_album(
    connection: &mut SqliteConnection,
    album_id: &str,
) -> QueryResult<Option<RequestRow>> {
    Ok(diesel::sql_query(format!(
        "SELECT {REQUEST_COLUMNS} FROM upgrade_request
         WHERE album_id = ? AND state IN ('requested', 'downloading', 'handed_off', 'importing')
         ORDER BY id DESC LIMIT 1"
    ))
    .bind::<Text, _>(album_id)
    .load::<RequestRow>(connection)?
    .into_iter()
    .next())
}

/// Automatic requests made since `since` (unix seconds).
pub fn automatic_since(connection: &mut SqliteConnection, since: i64) -> QueryResult<i64> {
    Ok(diesel::sql_query(
        "SELECT COUNT(*) AS count FROM upgrade_request
         WHERE origin = 'automatic' AND CAST(strftime('%s', created_at) AS INTEGER) >= ?",
    )
    .bind::<BigInt, _>(since)
    .get_result::<CountRow>(connection)?
    .count)
}

/// Albums that ever had a request (so automatic requests skip them).
pub fn requested_album_ids(connection: &mut SqliteConnection) -> QueryResult<Vec<String>> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        album_id: String,
    }
    Ok(
        diesel::sql_query("SELECT DISTINCT album_id FROM upgrade_request")
            .load::<Row>(connection)?
            .into_iter()
            .map(|row| row.album_id)
            .collect(),
    )
}

pub fn set_request_state(
    connection: &mut SqliteConnection,
    id: i64,
    state: RequestState,
    message: &str,
) -> QueryResult<()> {
    diesel::sql_query(
        "UPDATE upgrade_request SET state = ?, message = ?, updated_at = CURRENT_TIMESTAMP,
            finished_at = CASE WHEN ? THEN COALESCE(finished_at, CURRENT_TIMESTAMP) ELSE NULL END
         WHERE id = ?",
    )
    .bind::<Text, _>(state.as_str())
    .bind::<Text, _>(message)
    .bind::<diesel::sql_types::Bool, _>(!state.is_open())
    .bind::<BigInt, _>(id)
    .execute(connection)?;
    Ok(())
}

pub fn schedule_poll(
    connection: &mut SqliteConnection,
    id: i64,
    next_poll_at: i64,
    poll_failures: i32,
    quiet_polls: i32,
) -> QueryResult<()> {
    diesel::sql_query(
        "UPDATE upgrade_request SET next_poll_at = ?, poll_failures = ?, quiet_polls = ? WHERE id = ?",
    )
    .bind::<BigInt, _>(next_poll_at)
    .bind::<Integer, _>(poll_failures)
    .bind::<Integer, _>(quiet_polls)
    .bind::<BigInt, _>(id)
    .execute(connection)?;
    Ok(())
}

pub fn set_request_result(
    connection: &mut SqliteConnection,
    id: i64,
    album_id: Option<&str>,
    folder: Option<&str>,
) -> QueryResult<()> {
    diesel::sql_query(
        "UPDATE upgrade_request SET result_album_id = ?, result_folder = ? WHERE id = ?",
    )
    .bind::<Nullable<Text>, _>(album_id)
    .bind::<Nullable<Text>, _>(folder)
    .bind::<BigInt, _>(id)
    .execute(connection)?;
    Ok(())
}

pub fn delete_request(connection: &mut SqliteConnection, id: i64) -> QueryResult<usize> {
    diesel::sql_query("DELETE FROM upgrade_request_job WHERE request_id = ?")
        .bind::<BigInt, _>(id)
        .execute(connection)?;
    diesel::sql_query("DELETE FROM upgrade_request WHERE id = ?")
        .bind::<BigInt, _>(id)
        .execute(connection)
}

pub fn upsert_job(
    connection: &mut SqliteConnection,
    request_id: i64,
    job: &FetchJob,
) -> QueryResult<()> {
    diesel::sql_query(
        "INSERT INTO upgrade_request_job
            (request_id, job_id, status, progress, speed, eta, title, engine, dest, error, note, size)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(request_id, job_id) DO UPDATE SET
            status = excluded.status, progress = excluded.progress, speed = excluded.speed,
            eta = excluded.eta, title = excluded.title, engine = excluded.engine,
            dest = excluded.dest, error = excluded.error, note = excluded.note,
            size = excluded.size, updated_at = CURRENT_TIMESTAMP",
    )
    .bind::<BigInt, _>(request_id)
    .bind::<Text, _>(&job.id)
    .bind::<Text, _>(&job.status)
    .bind::<Nullable<Double>, _>(job.progress)
    .bind::<Text, _>(&job.speed)
    .bind::<Text, _>(&job.eta)
    .bind::<Text, _>(&job.title)
    .bind::<Text, _>(&job.engine)
    .bind::<Text, _>(&job.dest)
    .bind::<Text, _>(&job.error)
    .bind::<Text, _>(&job.note)
    .bind::<Text, _>(&job.size)
    .execute(connection)?;
    Ok(())
}

pub fn jobs_for(connection: &mut SqliteConnection, request_id: i64) -> QueryResult<Vec<JobRow>> {
    diesel::sql_query(
        "SELECT request_id, job_id, status, progress, speed, eta, title, engine, dest, error, note, size
         FROM upgrade_request_job WHERE request_id = ? ORDER BY job_id",
    )
    .bind::<BigInt, _>(request_id)
    .load(connection)
}

/* -------------------------------------------------------------- */
/* Spectral checks                                                 */
/* -------------------------------------------------------------- */

#[derive(QueryableByName, Clone, Debug)]
pub struct SpectralRow {
    #[diesel(sql_type = Text)]
    pub path: String,
    #[diesel(sql_type = Text)]
    pub file_version: String,
    #[diesel(sql_type = Text)]
    pub verdict: String,
    #[diesel(sql_type = Nullable<Integer>)]
    pub cutoff_hz: Option<i32>,
    #[diesel(sql_type = Double)]
    pub confidence: f64,
    #[diesel(sql_type = Text)]
    pub detail: String,
}

pub fn save_spectral(
    connection: &mut SqliteConnection,
    path: &str,
    file_version: &str,
    result: &SpectralResult,
) -> QueryResult<()> {
    diesel::sql_query(
        "INSERT INTO upgrade_spectral_check (path, file_version, verdict, cutoff_hz, confidence, detail)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(path) DO UPDATE SET file_version = excluded.file_version,
            verdict = excluded.verdict, cutoff_hz = excluded.cutoff_hz,
            confidence = excluded.confidence, detail = excluded.detail,
            analyzed_at = CURRENT_TIMESTAMP",
    )
    .bind::<Text, _>(path)
    .bind::<Text, _>(file_version)
    .bind::<Text, _>(result.verdict.as_str())
    .bind::<Nullable<Integer>, _>(result.cutoff_hz.map(|hz| hz as i32))
    .bind::<Double, _>(f64::from(result.confidence))
    .bind::<Text, _>(&result.detail)
    .execute(connection)?;
    Ok(())
}

pub fn spectral_for(
    connection: &mut SqliteConnection,
    path: &str,
) -> QueryResult<Option<SpectralRow>> {
    Ok(diesel::sql_query(
        "SELECT path, file_version, verdict, cutoff_hz, confidence, detail
         FROM upgrade_spectral_check WHERE path = ?",
    )
    .bind::<Text, _>(path)
    .load::<SpectralRow>(connection)?
    .into_iter()
    .next())
}

/// `path -> file_version` of every checked file.
pub fn spectral_versions(
    connection: &mut SqliteConnection,
) -> QueryResult<std::collections::HashMap<String, String>> {
    Ok(diesel::sql_query(
        "SELECT path, file_version, verdict, cutoff_hz, confidence, detail FROM upgrade_spectral_check",
    )
    .load::<SpectralRow>(connection)?
    .into_iter()
    .map(|row| (row.path, row.file_version))
    .collect())
}

/* -------------------------------------------------------------- */
/* Enhanced files and the enhance queue                            */
/* -------------------------------------------------------------- */

pub fn record_enhanced(
    connection: &mut SqliteConnection,
    path: &str,
    source_path: &str,
    model: &str,
) -> QueryResult<()> {
    diesel::sql_query(
        "INSERT INTO upgrade_enhanced_file (path, source_path, model) VALUES (?, ?, ?)
         ON CONFLICT(path) DO UPDATE SET source_path = excluded.source_path, model = excluded.model",
    )
    .bind::<Text, _>(path)
    .bind::<Text, _>(source_path)
    .bind::<Text, _>(model)
    .execute(connection)?;
    Ok(())
}

/// The enhanced copy made from `source_path`, if any.
pub fn enhanced_from(
    connection: &mut SqliteConnection,
    source_path: &str,
) -> QueryResult<Option<String>> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        path: String,
    }
    Ok(diesel::sql_query(
        "SELECT path FROM upgrade_enhanced_file WHERE source_path = ? ORDER BY created_at DESC LIMIT 1",
    )
    .bind::<Text, _>(source_path)
    .load::<Row>(connection)?
    .into_iter()
    .next()
    .map(|row| row.path))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnhanceState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
    Skipped,
}

impl EnhanceState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(QueryableByName, Clone, Debug, PartialEq, Serialize)]
pub struct EnhanceJobRow {
    #[diesel(sql_type = BigInt)]
    pub id: i64,
    #[diesel(sql_type = Text)]
    pub song_id: String,
    #[diesel(sql_type = Text)]
    pub album_id: String,
    #[diesel(sql_type = Text)]
    #[serde(skip)]
    pub source_path: String,
    #[diesel(sql_type = Text)]
    pub title: String,
    #[diesel(sql_type = Text)]
    pub state: String,
    #[diesel(sql_type = Nullable<Text>)]
    #[serde(skip)]
    pub worker_job_id: Option<String>,
    #[diesel(sql_type = Double)]
    pub progress: f64,
    #[diesel(sql_type = Nullable<Text>)]
    #[serde(skip)]
    pub output_path: Option<String>,
    #[diesel(sql_type = Text)]
    pub message: String,
    #[diesel(sql_type = Text)]
    pub batch: String,
    #[diesel(sql_type = Integer)]
    #[serde(skip)]
    pub attempts: i32,
}

const ENHANCE_COLUMNS: &str = "id, song_id, album_id, source_path, title, state, worker_job_id,
    progress, output_path, message, batch, attempts";

/// Queues tracks; a track already queued or running is not added twice.
/// Returns how many were added.
pub fn queue_enhance(
    connection: &mut SqliteConnection,
    tracks: &[(String, String, String, String)],
    batch: &str,
) -> QueryResult<usize> {
    let mut added = 0;
    for (song_id, album_id, path, title) in tracks {
        added += diesel::sql_query(
            "INSERT INTO upgrade_enhance_job (song_id, album_id, source_path, title, batch)
             SELECT ?, ?, ?, ?, ?
             WHERE NOT EXISTS (SELECT 1 FROM upgrade_enhance_job
                 WHERE source_path = ? AND state IN ('queued', 'running'))",
        )
        .bind::<Text, _>(song_id)
        .bind::<Text, _>(album_id)
        .bind::<Text, _>(path)
        .bind::<Text, _>(title)
        .bind::<Text, _>(batch)
        .bind::<Text, _>(path)
        .execute(connection)?;
    }
    Ok(added)
}

pub fn next_enhance_job(connection: &mut SqliteConnection) -> QueryResult<Option<EnhanceJobRow>> {
    Ok(diesel::sql_query(format!(
        "SELECT {ENHANCE_COLUMNS} FROM upgrade_enhance_job
         WHERE state IN ('running', 'queued') ORDER BY state = 'running' DESC, id LIMIT 1"
    ))
    .load::<EnhanceJobRow>(connection)?
    .into_iter()
    .next())
}

pub fn enhance_jobs(
    connection: &mut SqliteConnection,
    limit: i64,
) -> QueryResult<Vec<EnhanceJobRow>> {
    diesel::sql_query(format!(
        "SELECT {ENHANCE_COLUMNS} FROM upgrade_enhance_job
         ORDER BY CASE state WHEN 'running' THEN 0 WHEN 'queued' THEN 1 ELSE 2 END, id DESC LIMIT ?"
    ))
    .bind::<BigInt, _>(limit)
    .load(connection)
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct EnhanceCounts {
    pub queued: i64,
    pub running: i64,
    pub done: i64,
    pub failed: i64,
}

pub fn enhance_counts(connection: &mut SqliteConnection) -> QueryResult<EnhanceCounts> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        state: String,
        #[diesel(sql_type = BigInt)]
        count: i64,
    }
    let mut counts = EnhanceCounts::default();
    for row in
        diesel::sql_query("SELECT state, COUNT(*) AS count FROM upgrade_enhance_job GROUP BY state")
            .load::<Row>(connection)?
    {
        match row.state.as_str() {
            "queued" => counts.queued = row.count,
            "running" => counts.running = row.count,
            "done" => counts.done = row.count,
            "failed" => counts.failed = row.count,
            _ => {}
        }
    }
    Ok(counts)
}

pub fn update_enhance_job(
    connection: &mut SqliteConnection,
    id: i64,
    state: EnhanceState,
    worker_job_id: Option<&str>,
    progress: f64,
    output_path: Option<&str>,
    message: &str,
) -> QueryResult<()> {
    diesel::sql_query(
        "UPDATE upgrade_enhance_job SET state = ?, worker_job_id = COALESCE(?, worker_job_id),
            progress = ?, output_path = COALESCE(?, output_path), message = ?,
            attempts = attempts + CASE WHEN ? = 'failed' THEN 1 ELSE 0 END,
            updated_at = CURRENT_TIMESTAMP
         WHERE id = ?",
    )
    .bind::<Text, _>(state.as_str())
    .bind::<Nullable<Text>, _>(worker_job_id)
    .bind::<Double, _>(progress)
    .bind::<Nullable<Text>, _>(output_path)
    .bind::<Text, _>(message)
    .bind::<Text, _>(state.as_str())
    .bind::<BigInt, _>(id)
    .execute(connection)?;
    Ok(())
}

/// Cancels every queued job (running ones stop at their next check).
pub fn cancel_enhance_queue(connection: &mut SqliteConnection) -> QueryResult<usize> {
    diesel::sql_query(
        "UPDATE upgrade_enhance_job SET state = 'cancelled', message = 'Stopped.',
            updated_at = CURRENT_TIMESTAMP WHERE state IN ('queued', 'running')",
    )
    .execute(connection)
}

pub fn enhance_job_state(
    connection: &mut SqliteConnection,
    id: i64,
) -> QueryResult<Option<String>> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        state: String,
    }
    Ok(
        diesel::sql_query("SELECT state FROM upgrade_enhance_job WHERE id = ?")
            .bind::<BigInt, _>(id)
            .load::<Row>(connection)?
            .into_iter()
            .next()
            .map(|row| row.state),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tools::spectral::SpectralVerdict;

    #[test]
    fn requests_jobs_and_the_enhance_queue_round_trip() {
        let pool = crate::persistence::connection::migrated_memory_pool();
        let mut connection = pool.get().unwrap();
        let id = insert_request(
            &mut connection,
            &NewRequest {
                album_id: "a1",
                album_title: "Endgame",
                artist_name: "Rise Against",
                query: "the album Endgame by Rise Against as flac",
                reason: "lossy",
                automatic: true,
                requested_by: None,
            },
            RequestState::Requested,
            "",
        )
        .unwrap();
        assert_eq!(automatic_since(&mut connection, 0).unwrap(), 1);
        assert_eq!(due_requests(&mut connection, 0).unwrap().len(), 1);
        assert!(
            open_request_for_album(&mut connection, "a1")
                .unwrap()
                .is_some()
        );
        let job = super::super::fetch::parse_job(&serde_json::json!({
            "id": 5, "status": "running", "progress": 40, "engine": "soulseek"
        }))
        .unwrap();
        upsert_job(&mut connection, id, &job).unwrap();
        upsert_job(&mut connection, id, &job).unwrap();
        let jobs = jobs_for(&mut connection, id).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].progress, Some(40.0));
        set_request_state(&mut connection, id, RequestState::Upgraded, "Done").unwrap();
        assert!(
            open_request_for_album(&mut connection, "a1")
                .unwrap()
                .is_none()
        );
        assert_eq!(
            latest_for_album(&mut connection, "a1")
                .unwrap()
                .unwrap()
                .state(),
            RequestState::Upgraded
        );
        assert_eq!(requested_album_ids(&mut connection).unwrap(), vec!["a1"]);

        let tracks = vec![
            (
                "s1".to_string(),
                "a1".to_string(),
                "/m/1.mp3".to_string(),
                "One".to_string(),
            ),
            (
                "s2".to_string(),
                "a1".to_string(),
                "/m/2.mp3".to_string(),
                "Two".to_string(),
            ),
        ];
        assert_eq!(queue_enhance(&mut connection, &tracks, "b").unwrap(), 2);
        assert_eq!(
            queue_enhance(&mut connection, &tracks, "b").unwrap(),
            0,
            "no doubles"
        );
        let next = next_enhance_job(&mut connection).unwrap().unwrap();
        assert_eq!(next.song_id, "s1");
        update_enhance_job(
            &mut connection,
            next.id,
            EnhanceState::Running,
            Some("w1"),
            0.5,
            None,
            "",
        )
        .unwrap();
        // A running job resumes first after a restart.
        assert_eq!(
            next_enhance_job(&mut connection).unwrap().unwrap().id,
            next.id
        );
        assert_eq!(enhance_counts(&mut connection).unwrap().running, 1);
        assert_eq!(cancel_enhance_queue(&mut connection).unwrap(), 2);
        assert!(next_enhance_job(&mut connection).unwrap().is_none());

        record_enhanced(&mut connection, "/e/1.flac", "/m/1.mp3", "apollo-1.0").unwrap();
        assert_eq!(
            enhanced_from(&mut connection, "/m/1.mp3")
                .unwrap()
                .as_deref(),
            Some("/e/1.flac")
        );
        save_spectral(
            &mut connection,
            "/m/x.flac",
            "1:2",
            &SpectralResult {
                verdict: SpectralVerdict::Transcode,
                cutoff_hz: Some(16_000),
                confidence: 0.9,
                detail: "d".into(),
            },
        )
        .unwrap();
        let row = spectral_for(&mut connection, "/m/x.flac").unwrap().unwrap();
        assert_eq!(row.verdict, "transcode");
        assert_eq!(
            spectral_versions(&mut connection).unwrap()["/m/x.flac"],
            "1:2"
        );
        assert_eq!(delete_request(&mut connection, id).unwrap(), 1);
    }
}
