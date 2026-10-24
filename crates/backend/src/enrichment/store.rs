//! SQLite storage for enrichment settings, tasks, editorial content, external
//! credits and user notes.

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Nullable, Text};
use diesel::sqlite::SqliteConnection;
use serde::{Deserialize, Serialize};

pub const SETTINGS_KEY: &str = "enrichment";
pub const PROVIDER: &str = "musicbrainz";
pub const MAX_ATTEMPTS: i32 = 5;
pub const MAX_NOTE_CHARS: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrichmentSettings {
    /// Off by default: enrichment contacts MusicBrainz and Wikimedia.
    #[serde(default)]
    pub enabled: bool,
    /// Preferred Wikipedia language; English is the fallback.
    #[serde(default = "default_language")]
    pub language: String,
    /// Download freely licensed artist photos from Wikimedia Commons.
    #[serde(default = "default_true")]
    pub artist_images: bool,
}

fn default_language() -> String {
    "en".to_string()
}

fn default_true() -> bool {
    true
}

impl Default for EnrichmentSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            language: default_language(),
            artist_images: true,
        }
    }
}

#[derive(QueryableByName)]
struct ValueRow {
    #[diesel(sql_type = Text)]
    value: String,
}

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = Text)]
    status: String,
    #[diesel(sql_type = BigInt)]
    count: i64,
}

pub fn load_settings(connection: &mut SqliteConnection) -> QueryResult<EnrichmentSettings> {
    let rows = diesel::sql_query("SELECT value_json AS value FROM server_setting WHERE key = ?")
        .bind::<Text, _>(SETTINGS_KEY)
        .load::<ValueRow>(connection)?;
    Ok(rows
        .first()
        .and_then(|row| serde_json::from_str(&row.value).ok())
        .unwrap_or_default())
}

pub fn save_settings(
    connection: &mut SqliteConnection,
    settings: &EnrichmentSettings,
) -> QueryResult<()> {
    let value = serde_json::to_string(settings).unwrap_or_else(|_| "{}".to_string());
    diesel::sql_query(
        "INSERT INTO server_setting (key, value_json) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = CURRENT_TIMESTAMP",
    )
    .bind::<Text, _>(SETTINGS_KEY)
    .bind::<Text, _>(value)
    .execute(connection)?;
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TaskCounts {
    pub pending: i64,
    pub done: i64,
    pub skipped: i64,
    pub failed: i64,
}

pub fn task_counts(connection: &mut SqliteConnection) -> QueryResult<TaskCounts> {
    let rows = diesel::sql_query(
        "SELECT status, COUNT(*) AS count FROM metadata_task
         WHERE provider = ? AND entity_type IN ('album', 'artist') GROUP BY status",
    )
    .bind::<Text, _>(PROVIDER)
    .load::<CountRow>(connection)?;
    let mut counts = TaskCounts::default();
    for row in rows {
        match row.status.as_str() {
            "pending" => counts.pending += row.count,
            "done" => counts.done += row.count,
            "skipped" => counts.skipped += row.count,
            _ => counts.failed += row.count,
        }
    }
    Ok(counts)
}

/// Queues every album and artist that has no task yet. The indexer queues
/// tasks after a full scan; this covers libraries indexed before enrichment
/// existed and artists only known from credits.
pub fn seed_tasks(connection: &mut SqliteConnection) -> QueryResult<usize> {
    diesel::sql_query(
        "INSERT INTO metadata_task (provider, entity_type, entity_id)
         SELECT 'musicbrainz', 'album', id FROM album_entity
         UNION SELECT 'musicbrainz', 'artist', id FROM artist_entity
         ON CONFLICT(provider, entity_type, entity_id) DO NOTHING",
    )
    .execute(connection)
}

#[derive(Debug, Clone, PartialEq, Eq, QueryableByName)]
pub struct Task {
    #[diesel(sql_type = Integer)]
    pub id: i32,
    #[diesel(sql_type = Text)]
    pub entity_type: String,
    #[diesel(sql_type = Text)]
    pub entity_id: String,
    #[diesel(sql_type = Integer)]
    pub attempts: i32,
}

/// The next due task: albums first (their releases name the artists'
/// MusicBrainz ids), then artists. Track tasks are covered by their album.
pub fn next_task(connection: &mut SqliteConnection) -> QueryResult<Option<Task>> {
    diesel::sql_query(
        "UPDATE metadata_task SET status = 'skipped', last_error = 'covered by the album',
                updated_at = CURRENT_TIMESTAMP
         WHERE provider = 'musicbrainz' AND entity_type = 'track' AND status = 'pending'",
    )
    .execute(connection)?;
    let mut tasks = diesel::sql_query(
        "SELECT id, entity_type, entity_id, attempts FROM metadata_task
         WHERE provider = 'musicbrainz' AND status = 'pending'
           AND entity_type IN ('album', 'artist')
           AND (not_before IS NULL OR not_before <= CURRENT_TIMESTAMP)
         ORDER BY CASE entity_type WHEN 'album' THEN 0 ELSE 1 END, attempts, id
         LIMIT 1",
    )
    .load::<Task>(connection)?;
    Ok(tasks.pop())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// No confident match; not retried until the entity is refreshed.
    Skipped(String),
    /// A transient failure (network, rate limit); retried with backoff.
    Retry(String),
}

pub fn finish_task(
    connection: &mut SqliteConnection,
    task: &Task,
    outcome: &Outcome,
) -> QueryResult<()> {
    let (status, error, delay_minutes) = match outcome {
        Outcome::Done => ("done", None, 0),
        Outcome::Skipped(reason) => ("skipped", Some(reason.as_str()), 0),
        Outcome::Retry(error) if task.attempts + 1 >= MAX_ATTEMPTS => {
            ("failed", Some(error.as_str()), 0)
        }
        Outcome::Retry(error) => {
            let attempt = task.attempts + 1;
            ("pending", Some(error.as_str()), 10 * attempt * attempt)
        }
    };
    let error = error.map(|error| error.chars().take(500).collect::<String>());
    diesel::sql_query(
        "UPDATE metadata_task SET status = ?, attempts = attempts + 1, last_error = ?,
                not_before = CASE WHEN ? > 0 THEN datetime('now', '+' || ? || ' minutes') ELSE NULL END,
                updated_at = CURRENT_TIMESTAMP
         WHERE id = ?",
    )
    .bind::<Text, _>(status)
    .bind::<Nullable<Text>, _>(error)
    .bind::<Integer, _>(delay_minutes)
    .bind::<Integer, _>(delay_minutes)
    .bind::<Integer, _>(task.id)
    .execute(connection)?;
    Ok(())
}

/// Queues an entity to be looked up again.
pub fn refresh_entity(
    connection: &mut SqliteConnection,
    entity_type: &str,
    entity_id: &str,
) -> QueryResult<()> {
    diesel::sql_query(
        "INSERT INTO metadata_task (provider, entity_type, entity_id) VALUES ('musicbrainz', ?, ?)
         ON CONFLICT(provider, entity_type, entity_id) DO UPDATE SET
             status = 'pending', attempts = 0, last_error = NULL, not_before = NULL,
             updated_at = CURRENT_TIMESTAMP",
    )
    .bind::<Text, _>(entity_type)
    .bind::<Text, _>(entity_id)
    .execute(connection)?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, QueryableByName)]
pub struct TaskState {
    #[diesel(sql_type = Text)]
    pub status: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub last_error: Option<String>,
    #[diesel(sql_type = Text)]
    pub updated_at: String,
}

pub fn task_state(
    connection: &mut SqliteConnection,
    entity_type: &str,
    entity_id: &str,
) -> QueryResult<Option<TaskState>> {
    let mut rows = diesel::sql_query(
        "SELECT status, last_error, CAST(updated_at AS TEXT) AS updated_at FROM metadata_task
         WHERE provider = 'musicbrainz' AND entity_type = ? AND entity_id = ?",
    )
    .bind::<Text, _>(entity_type)
    .bind::<Text, _>(entity_id)
    .load::<TaskState>(connection)?;
    Ok(rows.pop())
}

#[derive(Debug, Clone, Default, PartialEq, Eq, QueryableByName)]
pub struct ProfileRow {
    #[diesel(sql_type = Nullable<Text>)]
    pub musicbrainz_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub release_group_mbid: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub wikidata_id: Option<String>,
    #[diesel(sql_type = Text)]
    pub profile_json: String,
}

pub fn save_profile(
    connection: &mut SqliteConnection,
    entity_type: &str,
    entity_id: &str,
    profile: &ProfileRow,
) -> QueryResult<()> {
    diesel::sql_query(
        "INSERT INTO editorial_profile
             (entity_type, entity_id, musicbrainz_id, release_group_mbid, wikidata_id, profile_json)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(entity_type, entity_id) DO UPDATE SET
             musicbrainz_id = excluded.musicbrainz_id,
             release_group_mbid = excluded.release_group_mbid,
             wikidata_id = excluded.wikidata_id,
             profile_json = excluded.profile_json,
             updated_at = CURRENT_TIMESTAMP",
    )
    .bind::<Text, _>(entity_type)
    .bind::<Text, _>(entity_id)
    .bind::<Nullable<Text>, _>(profile.musicbrainz_id.as_deref())
    .bind::<Nullable<Text>, _>(profile.release_group_mbid.as_deref())
    .bind::<Nullable<Text>, _>(profile.wikidata_id.as_deref())
    .bind::<Text, _>(&profile.profile_json)
    .execute(connection)?;
    Ok(())
}

pub fn load_profile(
    connection: &mut SqliteConnection,
    entity_type: &str,
    entity_id: &str,
) -> QueryResult<Option<ProfileRow>> {
    let mut rows = diesel::sql_query(
        "SELECT musicbrainz_id, release_group_mbid, wikidata_id, profile_json
         FROM editorial_profile WHERE entity_type = ? AND entity_id = ?",
    )
    .bind::<Text, _>(entity_type)
    .bind::<Text, _>(entity_id)
    .load::<ProfileRow>(connection)?;
    Ok(rows.pop())
}

/// Attributed text from a licensed source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, QueryableByName)]
pub struct EditorialText {
    #[diesel(sql_type = Text)]
    pub kind: String,
    #[diesel(sql_type = Text)]
    pub source: String,
    #[diesel(sql_type = Text)]
    pub language: String,
    #[diesel(sql_type = Text)]
    pub title: String,
    #[diesel(sql_type = Text)]
    pub body: String,
    #[diesel(sql_type = Text)]
    pub url: String,
    #[diesel(sql_type = Text)]
    pub license: String,
    #[diesel(sql_type = Text)]
    pub license_url: String,
}

pub fn replace_texts(
    connection: &mut SqliteConnection,
    entity_type: &str,
    entity_id: &str,
    texts: &[EditorialText],
) -> QueryResult<()> {
    connection.transaction(|connection| {
        diesel::sql_query("DELETE FROM editorial_text WHERE entity_type = ? AND entity_id = ?")
            .bind::<Text, _>(entity_type)
            .bind::<Text, _>(entity_id)
            .execute(connection)?;
        for text in texts {
            diesel::sql_query(
                "INSERT INTO editorial_text
                     (entity_type, entity_id, kind, source, language, title, body, url, license, license_url)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind::<Text, _>(entity_type)
            .bind::<Text, _>(entity_id)
            .bind::<Text, _>(&text.kind)
            .bind::<Text, _>(&text.source)
            .bind::<Text, _>(&text.language)
            .bind::<Text, _>(&text.title)
            .bind::<Text, _>(&text.body)
            .bind::<Text, _>(&text.url)
            .bind::<Text, _>(&text.license)
            .bind::<Text, _>(&text.license_url)
            .execute(connection)?;
        }
        Ok(())
    })
}

pub fn load_texts(
    connection: &mut SqliteConnection,
    entity_type: &str,
    entity_id: &str,
) -> QueryResult<Vec<EditorialText>> {
    diesel::sql_query(
        "SELECT kind, source, language, title, body, url, license, license_url
         FROM editorial_text WHERE entity_type = ? AND entity_id = ? ORDER BY kind",
    )
    .bind::<Text, _>(entity_type)
    .bind::<Text, _>(entity_id)
    .load::<EditorialText>(connection)
}

#[derive(Debug, Clone, PartialEq, Eq, QueryableByName)]
pub struct ImageRow {
    #[diesel(sql_type = Text)]
    pub file_name: String,
    #[diesel(sql_type = Text)]
    pub mime_type: String,
    #[diesel(sql_type = Text)]
    pub source_url: String,
    #[diesel(sql_type = Text)]
    pub page_url: String,
    #[diesel(sql_type = Text)]
    pub author: String,
    #[diesel(sql_type = Text)]
    pub license: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub license_url: Option<String>,
}

pub fn save_image(
    connection: &mut SqliteConnection,
    entity_type: &str,
    entity_id: &str,
    image: &ImageRow,
) -> QueryResult<()> {
    diesel::sql_query(
        "INSERT INTO editorial_image
             (entity_type, entity_id, file_name, mime_type, source_url, page_url, author, license, license_url)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(entity_type, entity_id) DO UPDATE SET
             file_name = excluded.file_name, mime_type = excluded.mime_type,
             source_url = excluded.source_url, page_url = excluded.page_url,
             author = excluded.author, license = excluded.license,
             license_url = excluded.license_url, updated_at = CURRENT_TIMESTAMP",
    )
    .bind::<Text, _>(entity_type)
    .bind::<Text, _>(entity_id)
    .bind::<Text, _>(&image.file_name)
    .bind::<Text, _>(&image.mime_type)
    .bind::<Text, _>(&image.source_url)
    .bind::<Text, _>(&image.page_url)
    .bind::<Text, _>(&image.author)
    .bind::<Text, _>(&image.license)
    .bind::<Nullable<Text>, _>(image.license_url.as_deref())
    .execute(connection)?;
    Ok(())
}

pub fn load_image(
    connection: &mut SqliteConnection,
    entity_type: &str,
    entity_id: &str,
) -> QueryResult<Option<ImageRow>> {
    let mut rows = diesel::sql_query(
        "SELECT file_name, mime_type, source_url, page_url, author, license, license_url
         FROM editorial_image WHERE entity_type = ? AND entity_id = ?",
    )
    .bind::<Text, _>(entity_type)
    .bind::<Text, _>(entity_id)
    .load::<ImageRow>(connection)?;
    Ok(rows.pop())
}

#[derive(Debug, Clone, PartialEq, Eq, QueryableByName)]
pub struct CreditRow {
    #[diesel(sql_type = Text)]
    pub album_id: String,
    #[diesel(sql_type = Text)]
    pub track_id: String,
    #[diesel(sql_type = Text)]
    pub name: String,
    #[diesel(sql_type = Text)]
    pub normalized_name: String,
    #[diesel(sql_type = Text)]
    pub role: String,
    #[diesel(sql_type = Text)]
    pub instrument: String,
    #[diesel(sql_type = Text)]
    pub source: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub source_artist_id: Option<String>,
}

pub fn replace_album_credits(
    connection: &mut SqliteConnection,
    album_id: &str,
    source: &str,
    credits: &[CreditRow],
) -> QueryResult<()> {
    connection.transaction(|connection| {
        diesel::sql_query("DELETE FROM enriched_credit WHERE album_id = ? AND source = ?")
            .bind::<Text, _>(album_id)
            .bind::<Text, _>(source)
            .execute(connection)?;
        for credit in credits {
            diesel::sql_query(
                "INSERT INTO enriched_credit
                     (album_id, track_id, name, normalized_name, role, instrument, source, source_artist_id)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT DO NOTHING",
            )
            .bind::<Text, _>(&credit.album_id)
            .bind::<Text, _>(&credit.track_id)
            .bind::<Text, _>(&credit.name)
            .bind::<Text, _>(&credit.normalized_name)
            .bind::<Text, _>(&credit.role)
            .bind::<Text, _>(&credit.instrument)
            .bind::<Text, _>(&credit.source)
            .bind::<Nullable<Text>, _>(credit.source_artist_id.as_deref())
            .execute(connection)?;
        }
        Ok(())
    })
}

const CREDIT_COLUMNS: &str =
    "album_id, track_id, name, normalized_name, role, instrument, source, source_artist_id";

pub fn album_credits(
    connection: &mut SqliteConnection,
    album_id: &str,
) -> QueryResult<Vec<CreditRow>> {
    diesel::sql_query(format!(
        "SELECT {CREDIT_COLUMNS} FROM enriched_credit WHERE album_id = ? ORDER BY rowid"
    ))
    .bind::<Text, _>(album_id)
    .load::<CreditRow>(connection)
}

/// External credits for a person, by normalized name or source artist id.
pub fn person_credits(
    connection: &mut SqliteConnection,
    normalized_name: Option<&str>,
    source_artist_id: Option<&str>,
) -> QueryResult<Vec<CreditRow>> {
    diesel::sql_query(format!(
        "SELECT {CREDIT_COLUMNS} FROM enriched_credit
         WHERE normalized_name = ? OR source_artist_id = ? LIMIT 5000"
    ))
    .bind::<Text, _>(normalized_name.unwrap_or("\u{0}"))
    .bind::<Text, _>(source_artist_id.unwrap_or("\u{0}"))
    .load::<CreditRow>(connection)
}

/// Remembers a MusicBrainz id for a local artist seen on a release.
pub fn link_artist_mbid(
    connection: &mut SqliteConnection,
    artist_id: &str,
    mbid: &str,
) -> QueryResult<()> {
    diesel::sql_query(
        "INSERT INTO external_id (entity_type, entity_id, provider, external_id, url)
         VALUES ('artist', ?, 'musicbrainz', ?, 'https://musicbrainz.org/artist/' || ?)
         ON CONFLICT DO NOTHING",
    )
    .bind::<Text, _>(artist_id)
    .bind::<Text, _>(mbid)
    .bind::<Text, _>(mbid)
    .execute(connection)?;
    Ok(())
}

pub fn linked_artist_mbids(
    connection: &mut SqliteConnection,
    artist_id: &str,
) -> QueryResult<Vec<String>> {
    Ok(diesel::sql_query(
        "SELECT external_id AS value FROM external_id
         WHERE entity_type = 'artist' AND entity_id = ? AND provider = 'musicbrainz'
         UNION
         SELECT DISTINCT rfm.musicbrainz_artist_id AS value
         FROM track_artist ta
         JOIN track_file tf ON tf.track_id = ta.track_id
         JOIN raw_file_metadata rfm ON rfm.file_id = tf.file_id
         WHERE ta.artist_id = ? AND ta.role = 'primary'
           AND length(rfm.musicbrainz_artist_id) = 36
         LIMIT 4",
    )
    .bind::<Text, _>(artist_id)
    .bind::<Text, _>(artist_id)
    .load::<ValueRow>(connection)?
    .into_iter()
    .map(|row| row.value)
    .collect())
}

pub fn artist_name(
    connection: &mut SqliteConnection,
    artist_id: &str,
) -> QueryResult<Option<String>> {
    let mut rows = diesel::sql_query("SELECT name AS value FROM artist_entity WHERE id = ?")
        .bind::<Text, _>(artist_id)
        .load::<ValueRow>(connection)?;
    Ok(rows.pop().map(|row| row.value))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, QueryableByName)]
pub struct NoteRow {
    #[diesel(sql_type = Text)]
    pub body: String,
    #[diesel(sql_type = Text)]
    pub updated_at: String,
}

pub fn load_note(
    connection: &mut SqliteConnection,
    user_id: i32,
    entity_type: &str,
    entity_id: &str,
) -> QueryResult<Option<NoteRow>> {
    let mut rows = diesel::sql_query(
        "SELECT body, CAST(updated_at AS TEXT) AS updated_at FROM user_note
         WHERE user_id = ? AND entity_type = ? AND entity_id = ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(entity_type)
    .bind::<Text, _>(entity_id)
    .load::<NoteRow>(connection)?;
    Ok(rows.pop())
}

/// Saves a note, or deletes it when `body` is blank.
pub fn save_note(
    connection: &mut SqliteConnection,
    user_id: i32,
    entity_type: &str,
    entity_id: &str,
    body: &str,
) -> QueryResult<Option<NoteRow>> {
    let body = body.trim();
    if body.is_empty() {
        diesel::sql_query(
            "DELETE FROM user_note WHERE user_id = ? AND entity_type = ? AND entity_id = ?",
        )
        .bind::<Integer, _>(user_id)
        .bind::<Text, _>(entity_type)
        .bind::<Text, _>(entity_id)
        .execute(connection)?;
        return Ok(None);
    }
    diesel::sql_query(
        "INSERT INTO user_note (user_id, entity_type, entity_id, body) VALUES (?, ?, ?, ?)
         ON CONFLICT(user_id, entity_type, entity_id) DO UPDATE SET
             body = excluded.body, updated_at = CURRENT_TIMESTAMP",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(entity_type)
    .bind::<Text, _>(entity_id)
    .bind::<Text, _>(body)
    .execute(connection)?;
    load_note(connection, user_id, entity_type, entity_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::connection::migrated_memory_pool;
    use diesel::connection::SimpleConnection;

    #[test]
    fn settings_default_to_off_and_round_trip() {
        let pool = migrated_memory_pool();
        let mut connection = pool.get().unwrap();
        let settings = load_settings(&mut connection).unwrap();
        assert!(!settings.enabled);
        assert_eq!(settings.language, "en");
        let enabled = EnrichmentSettings {
            enabled: true,
            language: "de".into(),
            artist_images: false,
        };
        save_settings(&mut connection, &enabled).unwrap();
        assert_eq!(load_settings(&mut connection).unwrap(), enabled);
    }

    #[test]
    fn tasks_run_albums_first_back_off_and_eventually_fail() {
        let pool = migrated_memory_pool();
        let mut connection = pool.get().unwrap();
        connection
            .batch_execute(
                "INSERT INTO metadata_task (provider, entity_type, entity_id) VALUES
                   ('musicbrainz', 'track', 't1'), ('musicbrainz', 'artist', 'a1'),
                   ('musicbrainz', 'album', 'b1');",
            )
            .unwrap();
        let task = next_task(&mut connection).unwrap().unwrap();
        assert_eq!(
            (task.entity_type.as_str(), task.entity_id.as_str()),
            ("album", "b1")
        );
        finish_task(&mut connection, &task, &Outcome::Retry("timeout".into())).unwrap();
        // The album now waits; the artist is next.
        let task = next_task(&mut connection).unwrap().unwrap();
        assert_eq!(task.entity_id, "a1");
        finish_task(&mut connection, &task, &Outcome::Done).unwrap();
        assert_eq!(next_task(&mut connection).unwrap(), None);
        let counts = task_counts(&mut connection).unwrap();
        assert_eq!((counts.pending, counts.done), (1, 1));

        let exhausted = Task {
            id: 3,
            entity_type: "album".into(),
            entity_id: "b1".into(),
            attempts: MAX_ATTEMPTS - 1,
        };
        finish_task(&mut connection, &exhausted, &Outcome::Retry("again".into())).unwrap();
        assert_eq!(task_counts(&mut connection).unwrap().failed, 1);
        refresh_entity(&mut connection, "album", "b1").unwrap();
        assert_eq!(
            next_task(&mut connection)
                .unwrap()
                .map(|task| task.entity_id),
            Some("b1".into())
        );
        assert_eq!(
            task_state(&mut connection, "track", "t1")
                .unwrap()
                .unwrap()
                .status,
            "skipped"
        );
    }

    #[test]
    fn notes_are_per_user_and_blank_notes_delete() {
        let pool = migrated_memory_pool();
        let mut connection = pool.get().unwrap();
        connection
            .batch_execute(
                "INSERT INTO user (id, username, password) VALUES (1, 'a', 'x'), (2, 'b', 'x');",
            )
            .unwrap();
        let note = save_note(&mut connection, 1, "album", "x", "  First pressing  ").unwrap();
        assert_eq!(note.unwrap().body, "First pressing");
        assert_eq!(load_note(&mut connection, 2, "album", "x").unwrap(), None);
        assert_eq!(
            save_note(&mut connection, 1, "album", "x", " ").unwrap(),
            None
        );
        assert_eq!(load_note(&mut connection, 1, "album", "x").unwrap(), None);
    }
}
