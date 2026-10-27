//! Database access for scrobbling connections, the offline queue, server
//! credentials and history imports.

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Binary, Integer, Nullable, Text};
use diesel::sqlite::SqliteConnection;
use serde::Serialize;

use super::{Listen, Service};

/// Sent and ignored listens are kept this long as the activity log.
pub const LOG_RETENTION_SECONDS: i64 = 30 * 24 * 60 * 60;
/// And at most this many per profile and service.
pub const MAX_LOG_ROWS: i64 = 500;

#[derive(Clone, Debug, QueryableByName)]
pub struct ConnectionRow {
    #[diesel(sql_type = Integer)]
    pub user_id: i32,
    #[diesel(sql_type = Text)]
    pub service: String,
    #[diesel(sql_type = Text)]
    pub account_name: String,
    #[diesel(sql_type = Binary)]
    pub sealed_secret: Vec<u8>,
    #[diesel(sql_type = Integer)]
    pub enabled: i32,
    #[diesel(sql_type = Integer)]
    pub send_now_playing: i32,
    #[diesel(sql_type = Integer)]
    pub scrobble_radio: i32,
    #[diesel(sql_type = Text)]
    pub status: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub last_error: Option<String>,
    #[diesel(sql_type = Nullable<BigInt>)]
    pub last_scrobbled_at: Option<i64>,
    #[diesel(sql_type = BigInt)]
    pub connected_at: i64,
}

impl ConnectionRow {
    pub fn service(&self) -> Option<Service> {
        Service::parse(&self.service)
    }

    pub fn active(&self) -> bool {
        self.enabled != 0 && self.status == "ok"
    }
}

const CONNECTION_COLUMNS: &str = "user_id, service, account_name, sealed_secret, enabled, \
     send_now_playing, scrobble_radio, status, last_error, last_scrobbled_at, connected_at";

pub fn connections(
    connection: &mut SqliteConnection,
    user_id: i32,
) -> QueryResult<Vec<ConnectionRow>> {
    diesel::sql_query(format!(
        "SELECT {CONNECTION_COLUMNS} FROM scrobble_connection WHERE user_id = ? ORDER BY service"
    ))
    .bind::<Integer, _>(user_id)
    .load(connection)
}

pub fn connection(
    connection: &mut SqliteConnection,
    user_id: i32,
    service: Service,
) -> QueryResult<Option<ConnectionRow>> {
    diesel::sql_query(format!(
        "SELECT {CONNECTION_COLUMNS} FROM scrobble_connection WHERE user_id = ? AND service = ?"
    ))
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(service.as_str())
    .get_result(connection)
    .optional()
}

/// Stores a new or renewed connection. Reconnecting keeps the listener's
/// toggles, clears the error and makes waiting listens due immediately.
pub fn upsert_connection(
    connection: &mut SqliteConnection,
    user_id: i32,
    service: Service,
    account_name: &str,
    sealed_secret: &[u8],
    now: i64,
) -> QueryResult<()> {
    connection.immediate_transaction(|connection| {
        diesel::sql_query(
            "INSERT INTO scrobble_connection
               (user_id, service, account_name, sealed_secret, connected_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(user_id, service) DO UPDATE SET
               account_name = excluded.account_name,
               sealed_secret = excluded.sealed_secret,
               status = 'ok',
               last_error = NULL,
               connected_at = excluded.connected_at",
        )
        .bind::<Integer, _>(user_id)
        .bind::<Text, _>(service.as_str())
        .bind::<Text, _>(account_name)
        .bind::<Binary, _>(sealed_secret)
        .bind::<BigInt, _>(now)
        .execute(connection)?;
        diesel::sql_query(
            "UPDATE scrobble_queue SET next_attempt_at = ?
             WHERE user_id = ? AND service = ? AND status = 'pending'",
        )
        .bind::<BigInt, _>(now)
        .bind::<Integer, _>(user_id)
        .bind::<Text, _>(service.as_str())
        .execute(connection)?;
        Ok(())
    })
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SettingsPatch {
    pub enabled: Option<bool>,
    pub send_now_playing: Option<bool>,
    pub scrobble_radio: Option<bool>,
}

pub fn update_settings(
    connection: &mut SqliteConnection,
    user_id: i32,
    service: Service,
    patch: SettingsPatch,
) -> QueryResult<bool> {
    let flag = |value: Option<bool>| value.map(i32::from);
    diesel::sql_query(
        "UPDATE scrobble_connection SET
           enabled = COALESCE(?, enabled),
           send_now_playing = COALESCE(?, send_now_playing),
           scrobble_radio = COALESCE(?, scrobble_radio)
         WHERE user_id = ? AND service = ?",
    )
    .bind::<Nullable<Integer>, _>(flag(patch.enabled))
    .bind::<Nullable<Integer>, _>(flag(patch.send_now_playing))
    .bind::<Nullable<Integer>, _>(flag(patch.scrobble_radio))
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(service.as_str())
    .execute(connection)
    .map(|updated| updated > 0)
}

/// Removes a connection and every listen still waiting for it.
pub fn delete_connection(
    connection: &mut SqliteConnection,
    user_id: i32,
    service: Service,
) -> QueryResult<bool> {
    connection.immediate_transaction(|connection| {
        diesel::sql_query("DELETE FROM scrobble_queue WHERE user_id = ? AND service = ?")
            .bind::<Integer, _>(user_id)
            .bind::<Text, _>(service.as_str())
            .execute(connection)?;
        diesel::sql_query("DELETE FROM scrobble_history_import WHERE user_id = ? AND service = ?")
            .bind::<Integer, _>(user_id)
            .bind::<Text, _>(service.as_str())
            .execute(connection)?;
        diesel::sql_query("DELETE FROM scrobble_connection WHERE user_id = ? AND service = ?")
            .bind::<Integer, _>(user_id)
            .bind::<Text, _>(service.as_str())
            .execute(connection)
            .map(|deleted| deleted > 0)
    })
}

pub fn mark_reauth_required(
    connection: &mut SqliteConnection,
    user_id: i32,
    service: Service,
    message: &str,
) -> QueryResult<()> {
    diesel::sql_query(
        "UPDATE scrobble_connection SET status = 'reauth_required', last_error = ?
         WHERE user_id = ? AND service = ?",
    )
    .bind::<Text, _>(message)
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(service.as_str())
    .execute(connection)
    .map(|_| ())
}

pub fn record_connection_result(
    connection: &mut SqliteConnection,
    user_id: i32,
    service: Service,
    error: Option<&str>,
    scrobbled_at: Option<i64>,
) -> QueryResult<()> {
    diesel::sql_query(
        "UPDATE scrobble_connection SET
           last_error = ?,
           last_scrobbled_at = COALESCE(?, last_scrobbled_at)
         WHERE user_id = ? AND service = ?",
    )
    .bind::<Nullable<Text>, _>(error)
    .bind::<Nullable<BigInt>, _>(scrobbled_at)
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(service.as_str())
    .execute(connection)
    .map(|_| ())
}

/// Queues a listen; returns false when the same listen was queued before.
pub fn enqueue(
    connection: &mut SqliteConnection,
    user_id: i32,
    service: Service,
    song_id: Option<&str>,
    listen: &Listen,
    now: i64,
) -> QueryResult<bool> {
    let dedupe_key = format!(
        "{}:{}",
        song_id.unwrap_or(&format!("{}|{}", listen.artist, listen.track)),
        listen.listened_at
    );
    diesel::sql_query(
        "INSERT OR IGNORE INTO scrobble_queue
           (user_id, service, dedupe_key, song_id, artist, track, album, album_artist,
            duration_seconds, track_number, release_mbid, isrc, listened_at, chosen_by_user,
            next_attempt_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(service.as_str())
    .bind::<Text, _>(dedupe_key)
    .bind::<Nullable<Text>, _>(song_id)
    .bind::<Text, _>(&listen.artist)
    .bind::<Text, _>(&listen.track)
    .bind::<Nullable<Text>, _>(listen.album.as_deref())
    .bind::<Nullable<Text>, _>(listen.album_artist.as_deref())
    .bind::<Nullable<Integer>, _>(listen.duration_seconds.map(|value| value as i32))
    .bind::<Nullable<Integer>, _>(listen.track_number.map(|value| value as i32))
    .bind::<Nullable<Text>, _>(listen.release_mbid.as_deref())
    .bind::<Nullable<Text>, _>(listen.isrc.as_deref())
    .bind::<BigInt, _>(listen.listened_at)
    .bind::<Integer, _>(i32::from(listen.chosen_by_user))
    .bind::<BigInt, _>(now)
    .bind::<BigInt, _>(now)
    .execute(connection)
    .map(|inserted| inserted > 0)
}

#[derive(QueryableByName)]
struct DueGroup {
    #[diesel(sql_type = Integer)]
    user_id: i32,
    #[diesel(sql_type = Text)]
    service: String,
}

/// Profiles and services with listens due for delivery.
pub fn due_groups(
    connection: &mut SqliteConnection,
    now: i64,
    limit: i64,
) -> QueryResult<Vec<(i32, Service)>> {
    diesel::sql_query(
        "SELECT DISTINCT q.user_id, q.service FROM scrobble_queue q
         JOIN scrobble_connection c ON c.user_id = q.user_id AND c.service = q.service
         WHERE q.status = 'pending' AND q.next_attempt_at <= ?
           AND c.enabled = 1 AND c.status = 'ok'
         LIMIT ?",
    )
    .bind::<BigInt, _>(now)
    .bind::<BigInt, _>(limit)
    .load::<DueGroup>(connection)
    .map(|rows| {
        rows.into_iter()
            .filter_map(|row| Some((row.user_id, Service::parse(&row.service)?)))
            .collect()
    })
}

/// The earliest moment any pending listen becomes due.
pub fn next_due_at(connection: &mut SqliteConnection) -> QueryResult<Option<i64>> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Nullable<BigInt>)]
        due: Option<i64>,
    }
    diesel::sql_query(
        "SELECT MIN(q.next_attempt_at) AS due FROM scrobble_queue q
         JOIN scrobble_connection c ON c.user_id = q.user_id AND c.service = q.service
         WHERE q.status = 'pending' AND c.enabled = 1 AND c.status = 'ok'",
    )
    .get_result::<Row>(connection)
    .map(|row| row.due)
}

#[derive(Clone, Debug, QueryableByName)]
pub struct QueueRow {
    #[diesel(sql_type = BigInt)]
    pub id: i64,
    #[diesel(sql_type = Nullable<Text>)]
    pub song_id: Option<String>,
    #[diesel(sql_type = Text)]
    pub artist: String,
    #[diesel(sql_type = Text)]
    pub track: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub album: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub album_artist: Option<String>,
    #[diesel(sql_type = Nullable<Integer>)]
    pub duration_seconds: Option<i32>,
    #[diesel(sql_type = Nullable<Integer>)]
    pub track_number: Option<i32>,
    #[diesel(sql_type = Nullable<Text>)]
    pub release_mbid: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub isrc: Option<String>,
    #[diesel(sql_type = BigInt)]
    pub listened_at: i64,
    #[diesel(sql_type = Integer)]
    pub chosen_by_user: i32,
    #[diesel(sql_type = Integer)]
    pub attempts: i32,
}

impl QueueRow {
    pub fn listen(&self) -> Listen {
        Listen {
            artist: self.artist.clone(),
            track: self.track.clone(),
            album: self.album.clone(),
            album_artist: self.album_artist.clone(),
            duration_seconds: self
                .duration_seconds
                .and_then(|value| u32::try_from(value).ok()),
            track_number: self
                .track_number
                .and_then(|value| u32::try_from(value).ok()),
            release_mbid: self.release_mbid.clone(),
            isrc: self.isrc.clone(),
            listened_at: self.listened_at,
            chosen_by_user: self.chosen_by_user != 0,
        }
    }
}

pub fn due_rows(
    connection: &mut SqliteConnection,
    user_id: i32,
    service: Service,
    now: i64,
    limit: usize,
) -> QueryResult<Vec<QueueRow>> {
    diesel::sql_query(
        "SELECT id, song_id, artist, track, album, album_artist, duration_seconds, track_number,
                release_mbid, isrc, listened_at, chosen_by_user, attempts
         FROM scrobble_queue
         WHERE user_id = ? AND service = ? AND status = 'pending' AND next_attempt_at <= ?
         ORDER BY listened_at, id LIMIT ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(service.as_str())
    .bind::<BigInt, _>(now)
    .bind::<BigInt, _>(limit as i64)
    .load(connection)
}

pub fn mark_delivered(
    connection: &mut SqliteConnection,
    id: i64,
    status: &str,
    reason: Option<&str>,
    now: i64,
) -> QueryResult<()> {
    diesel::sql_query(
        "UPDATE scrobble_queue SET status = ?, last_error = ?, sent_at = ?, attempts = attempts + 1
         WHERE id = ?",
    )
    .bind::<Text, _>(status)
    .bind::<Nullable<Text>, _>(reason)
    .bind::<BigInt, _>(now)
    .bind::<BigInt, _>(id)
    .execute(connection)
    .map(|_| ())
}

/// Schedules another attempt, or gives up after too many.
pub fn defer(
    connection: &mut SqliteConnection,
    row: &QueueRow,
    error: &str,
    next_attempt_at: i64,
    give_up: bool,
) -> QueryResult<()> {
    diesel::sql_query(
        "UPDATE scrobble_queue SET
           attempts = attempts + 1,
           last_error = ?,
           next_attempt_at = ?,
           status = CASE WHEN ? THEN 'failed' ELSE status END
         WHERE id = ?",
    )
    .bind::<Text, _>(error)
    .bind::<BigInt, _>(next_attempt_at)
    .bind::<diesel::sql_types::Bool, _>(give_up)
    .bind::<BigInt, _>(row.id)
    .execute(connection)
    .map(|_| ())
}

/// Makes every waiting or failed listen of a profile due now.
pub fn retry_now(connection: &mut SqliteConnection, user_id: i32, now: i64) -> QueryResult<usize> {
    diesel::sql_query(
        "UPDATE scrobble_queue SET
           status = 'pending',
           next_attempt_at = ?,
           attempts = CASE WHEN status = 'failed' THEN 0 ELSE attempts END
         WHERE user_id = ? AND status IN ('pending', 'failed')",
    )
    .bind::<BigInt, _>(now)
    .bind::<Integer, _>(user_id)
    .execute(connection)
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueCounts {
    pub pending: i64,
    pub failed: i64,
    pub sent: i64,
    pub ignored: i64,
    pub next_attempt_at: Option<i64>,
}

pub fn queue_counts(
    connection: &mut SqliteConnection,
    user_id: i32,
    service: Service,
) -> QueryResult<QueueCounts> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = BigInt)]
        pending: i64,
        #[diesel(sql_type = BigInt)]
        failed: i64,
        #[diesel(sql_type = BigInt)]
        sent: i64,
        #[diesel(sql_type = BigInt)]
        ignored: i64,
        #[diesel(sql_type = Nullable<BigInt>)]
        next_attempt_at: Option<i64>,
    }
    diesel::sql_query(
        "SELECT
           CAST(COALESCE(SUM(status = 'pending'), 0) AS BIGINT) AS pending,
           CAST(COALESCE(SUM(status = 'failed'), 0) AS BIGINT) AS failed,
           CAST(COALESCE(SUM(status = 'sent'), 0) AS BIGINT) AS sent,
           CAST(COALESCE(SUM(status = 'ignored'), 0) AS BIGINT) AS ignored,
           MIN(CASE WHEN status = 'pending' THEN next_attempt_at END) AS next_attempt_at
         FROM scrobble_queue WHERE user_id = ? AND service = ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(service.as_str())
    .get_result::<Row>(connection)
    .map(|row| QueueCounts {
        pending: row.pending,
        failed: row.failed,
        sent: row.sent,
        ignored: row.ignored,
        next_attempt_at: row.next_attempt_at,
    })
}

#[derive(Clone, Debug, QueryableByName, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    #[diesel(sql_type = BigInt)]
    pub id: i64,
    #[diesel(sql_type = Text)]
    pub service: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub song_id: Option<String>,
    #[diesel(sql_type = Text)]
    pub artist: String,
    #[diesel(sql_type = Text)]
    pub track: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub album: Option<String>,
    #[diesel(sql_type = BigInt)]
    pub listened_at: i64,
    #[diesel(sql_type = Text)]
    pub status: String,
    #[diesel(sql_type = Integer)]
    pub attempts: i32,
    #[diesel(sql_type = Nullable<Text>)]
    pub last_error: Option<String>,
    #[diesel(sql_type = BigInt)]
    pub next_attempt_at: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    pub sent_at: Option<i64>,
}

pub fn recent_log(
    connection: &mut SqliteConnection,
    user_id: i32,
    limit: i64,
) -> QueryResult<Vec<LogEntry>> {
    diesel::sql_query(
        "SELECT id, service, song_id, artist, track, album, listened_at, status, attempts,
                last_error, next_attempt_at, sent_at
         FROM scrobble_queue WHERE user_id = ?
         ORDER BY listened_at DESC, id DESC LIMIT ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<BigInt, _>(limit)
    .load(connection)
}

/// Trims the activity log: delivered listens older than the retention
/// window, and anything beyond the per-profile cap.
pub fn prune_log(connection: &mut SqliteConnection, now: i64) -> QueryResult<usize> {
    let aged = diesel::sql_query(
        "DELETE FROM scrobble_queue
         WHERE status IN ('sent', 'ignored') AND COALESCE(sent_at, created_at) < ?",
    )
    .bind::<BigInt, _>(now - LOG_RETENTION_SECONDS)
    .execute(connection)?;
    let capped = diesel::sql_query(
        "DELETE FROM scrobble_queue WHERE id IN (
           SELECT id FROM (
             SELECT id, ROW_NUMBER() OVER (
               PARTITION BY user_id, service ORDER BY listened_at DESC, id DESC
             ) AS position
             FROM scrobble_queue WHERE status IN ('sent', 'ignored')
           ) WHERE position > ?
         )",
    )
    .bind::<BigInt, _>(MAX_LOG_ROWS)
    .execute(connection)?;
    Ok(aged + capped)
}

pub fn server_config(
    connection: &mut SqliteConnection,
    service: Service,
) -> QueryResult<Option<Vec<u8>>> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Binary)]
        sealed_config: Vec<u8>,
    }
    diesel::sql_query("SELECT sealed_config FROM scrobble_service_config WHERE service = ?")
        .bind::<Text, _>(service.as_str())
        .get_result::<Row>(connection)
        .optional()
        .map(|row| row.map(|row| row.sealed_config))
}

pub fn set_server_config(
    connection: &mut SqliteConnection,
    service: Service,
    sealed: &[u8],
    updated_by: i32,
    now: i64,
) -> QueryResult<()> {
    diesel::sql_query(
        "INSERT INTO scrobble_service_config (service, sealed_config, updated_by, updated_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(service) DO UPDATE SET
           sealed_config = excluded.sealed_config,
           updated_by = excluded.updated_by,
           updated_at = excluded.updated_at",
    )
    .bind::<Text, _>(service.as_str())
    .bind::<Binary, _>(sealed)
    .bind::<Integer, _>(updated_by)
    .bind::<BigInt, _>(now)
    .execute(connection)
    .map(|_| ())
}

pub fn delete_server_config(
    connection: &mut SqliteConnection,
    service: Service,
) -> QueryResult<bool> {
    diesel::sql_query("DELETE FROM scrobble_service_config WHERE service = ?")
        .bind::<Text, _>(service.as_str())
        .execute(connection)
        .map(|deleted| deleted > 0)
}

#[derive(Clone, Debug, QueryableByName, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportStatus {
    #[diesel(sql_type = Text)]
    pub status: String,
    #[diesel(sql_type = BigInt)]
    pub fetched: i64,
    #[diesel(sql_type = BigInt)]
    pub matched: i64,
    #[diesel(sql_type = BigInt)]
    pub imported: i64,
    #[diesel(sql_type = Nullable<Text>)]
    pub error: Option<String>,
    #[diesel(sql_type = BigInt)]
    pub started_at: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    pub finished_at: Option<i64>,
}

pub fn import_status(
    connection: &mut SqliteConnection,
    user_id: i32,
    service: Service,
) -> QueryResult<Option<ImportStatus>> {
    diesel::sql_query(
        "SELECT status, CAST(fetched AS BIGINT) AS fetched, CAST(matched AS BIGINT) AS matched,
                CAST(imported AS BIGINT) AS imported, error, started_at, finished_at
         FROM scrobble_history_import WHERE user_id = ? AND service = ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(service.as_str())
    .get_result(connection)
    .optional()
}

pub fn write_import_status(
    connection: &mut SqliteConnection,
    user_id: i32,
    service: Service,
    status: &ImportStatus,
) -> QueryResult<()> {
    diesel::sql_query(
        "INSERT INTO scrobble_history_import
           (user_id, service, status, fetched, matched, imported, error, started_at, finished_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(user_id, service) DO UPDATE SET
           status = excluded.status, fetched = excluded.fetched, matched = excluded.matched,
           imported = excluded.imported, error = excluded.error,
           started_at = excluded.started_at, finished_at = excluded.finished_at",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(service.as_str())
    .bind::<Text, _>(&status.status)
    .bind::<BigInt, _>(status.fetched)
    .bind::<BigInt, _>(status.matched)
    .bind::<BigInt, _>(status.imported)
    .bind::<Nullable<Text>, _>(status.error.as_deref())
    .bind::<BigInt, _>(status.started_at)
    .bind::<Nullable<BigInt>, _>(status.finished_at)
    .execute(connection)
    .map(|_| ())
}

/// Imports that were running when the server stopped did not finish.
pub fn fail_interrupted_imports(connection: &mut SqliteConnection, now: i64) -> QueryResult<usize> {
    diesel::sql_query(
        "UPDATE scrobble_history_import
         SET status = 'failed', error = 'The server restarted during the import.', finished_at = ?
         WHERE status = 'running'",
    )
    .bind::<BigInt, _>(now)
    .execute(connection)
}

/// When playback of `song_id` started in this player session, from the
/// `play_started` / `manual_selection` event the client sent.
pub fn playback_started_at(
    connection: &mut SqliteConnection,
    user_id: i32,
    session_id: &str,
    song_id: &str,
    not_before: i64,
) -> QueryResult<Option<i64>> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = BigInt)]
        started: i64,
    }
    diesel::sql_query(
        "SELECT CAST(strftime('%s', created_at) AS BIGINT) AS started FROM playback_event
         WHERE user_id = ? AND session_id = ? AND song_id = ?
           AND event_type IN ('play_started', 'manual_selection')
           AND created_at >= datetime(?, 'unixepoch')
         ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(session_id)
    .bind::<Text, _>(song_id)
    .bind::<BigInt, _>(not_before)
    .get_result::<Row>(connection)
    .optional()
    .map(|row| row.map(|row| row.started))
}
