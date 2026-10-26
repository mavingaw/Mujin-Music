//! SQLite storage for enrichment settings, tasks, editorial content, external
//! credits and user notes.

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Nullable, Text};
use diesel::sqlite::SqliteConnection;
use serde::{Deserialize, Serialize};

pub const SETTINGS_KEY: &str = "enrichment";
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
    /// Whether albums and artists show MusicBrainz genres or the genres
    /// in the files' tags (each falls back to the other).
    #[serde(default)]
    pub genre_source: crate::enrichment::genres::GenreSource,
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
            genre_source: Default::default(),
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

/// Progress of the MusicBrainz stage, which every album and artist passes.
pub fn task_counts(connection: &mut SqliteConnection) -> QueryResult<TaskCounts> {
    stage_counts(connection, Stage::Musicbrainz)
}

pub fn stage_counts(connection: &mut SqliteConnection, stage: Stage) -> QueryResult<TaskCounts> {
    let rows = diesel::sql_query(
        "SELECT status, COUNT(*) AS count FROM metadata_task
         WHERE provider = ? AND entity_type IN ('album', 'artist') GROUP BY status",
    )
    .bind::<Text, _>(stage.provider())
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
///
/// SQLite reads `ON CONFLICT` after a bare `SELECT ... FROM table` as the
/// start of a join constraint ("near DO: syntax error"), so the compound
/// select is wrapped and given an explicit `WHERE true`.
pub fn seed_tasks(connection: &mut SqliteConnection) -> QueryResult<usize> {
    let queued = diesel::sql_query(
        "INSERT INTO metadata_task (provider, entity_type, entity_id)
         SELECT 'musicbrainz', entity_type, entity_id FROM (
             SELECT 'album' AS entity_type, id AS entity_id FROM album_entity
             UNION SELECT 'artist', id FROM artist_entity
         ) WHERE true
         ON CONFLICT(provider, entity_type, entity_id) DO NOTHING",
    )
    .execute(connection)?;
    // Items matched before lookups were split into stages still need their
    // Wikidata stage (and through it the biography and photo).
    let linked = diesel::sql_query(
        "INSERT INTO metadata_task (provider, entity_type, entity_id)
         SELECT 'wikidata', entity_type, entity_id FROM editorial_profile
         WHERE wikidata_id IS NOT NULL
         ON CONFLICT(provider, entity_type, entity_id) DO NOTHING",
    )
    .execute(connection)?;
    // Text no longer waits for Wikidata: anything matched gets a Wikipedia
    // stage of its own.
    let text = diesel::sql_query(
        "INSERT INTO metadata_task (provider, entity_type, entity_id)
         SELECT 'wikipedia', entity_type, entity_id FROM editorial_profile
         WHERE entity_type IN ('artist', 'album', 'work')
           AND (musicbrainz_id IS NOT NULL OR wikidata_id IS NOT NULL)
         ON CONFLICT(provider, entity_type, entity_id) DO NOTHING",
    )
    .execute(connection)?;
    // People MusicBrainz credits who are not library artists (producers,
    // engineers, session players) get a page of their own, with a
    // biography and photo, under their `mb:` id.
    let people = diesel::sql_query(
        "INSERT INTO metadata_task (provider, entity_type, entity_id)
         SELECT DISTINCT 'musicbrainz', 'artist', 'mb:' || credit.source_artist_id
         FROM enriched_credit credit
         WHERE credit.source = 'musicbrainz'
           AND length(credit.source_artist_id) = 36
           AND NOT EXISTS (
               SELECT 1 FROM external_id linked
               WHERE linked.entity_type = 'artist' AND linked.provider = 'musicbrainz'
                 AND linked.external_id = credit.source_artist_id
           )
         ON CONFLICT(provider, entity_type, entity_id) DO NOTHING",
    )
    .execute(connection)?;
    // Influences and followers for people with a Wikidata item, and covers
    // for matched albums, including those matched before these stages.
    let relations = diesel::sql_query(
        "INSERT INTO metadata_task (provider, entity_type, entity_id)
         SELECT 'wikidata-query', 'artist', entity_id FROM editorial_profile
         WHERE entity_type = 'artist' AND wikidata_id IS NOT NULL
         ON CONFLICT(provider, entity_type, entity_id) DO NOTHING",
    )
    .execute(connection)?;
    let covers = diesel::sql_query(
        "INSERT INTO metadata_task (provider, entity_type, entity_id)
         SELECT 'coverart', 'album', entity_id FROM editorial_profile
         WHERE entity_type = 'album' AND musicbrainz_id IS NOT NULL
         ON CONFLICT(provider, entity_type, entity_id) DO NOTHING",
    )
    .execute(connection)?;
    Ok(queued + linked + text + people + relations + covers)
}

/// The enrichment stages. Each one talks to a single host and has its own
/// queue (`metadata_task.provider`) and its own back-off, so a busy
/// Wikimedia Commons never holds back MusicBrainz or Wikipedia.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    /// Resolve the album or artist and its credits on MusicBrainz.
    Musicbrainz,
    /// Fetch the biography, background and reception text, from the
    /// Wikipedia link MusicBrainz gives or an article whose Wikidata item
    /// matches. It does not wait for the Wikidata stage.
    Wikipedia,
    /// Read sitelinks, life dates and the P18 image from Wikidata, which
    /// enrich what the Wikipedia stage found (an article in the preferred
    /// language, a photo).
    Wikidata,
    /// Download the licensed artist photo from Wikimedia Commons.
    Commons,
    /// Ask the Wikidata Query Service who influenced a person (P737) and
    /// whom they influenced, for the Influences / Followers sections.
    Relations,
    /// Download the front cover of the matched release from the Cover Art
    /// Archive for an album that has no artwork of its own.
    CoverArt,
}

impl Stage {
    pub const ALL: [Stage; 6] = [
        Stage::Musicbrainz,
        Stage::Wikipedia,
        Stage::Wikidata,
        Stage::Commons,
        Stage::Relations,
        Stage::CoverArt,
    ];

    /// Whether the stage fills an entity's page text or photo, so a page
    /// waiting on it says so. Relations and cover art arrive quietly.
    pub fn feeds_page_text(self) -> bool {
        !matches!(self, Stage::Relations | Stage::CoverArt)
    }

    /// The `metadata_task.provider` value for the stage's queue.
    pub fn provider(self) -> &'static str {
        match self {
            Stage::Musicbrainz => "musicbrainz",
            Stage::Wikidata => "wikidata",
            Stage::Wikipedia => "wikipedia",
            Stage::Commons => "commons",
            Stage::Relations => "wikidata-query",
            Stage::CoverArt => "coverart",
        }
    }

    /// The host whose back-off applies to the stage.
    pub fn host(self) -> &'static str {
        match self {
            Stage::Musicbrainz => "musicbrainz.org",
            Stage::Wikidata => "wikidata.org",
            Stage::Wikipedia => "wikipedia.org",
            Stage::Commons => "commons.wikimedia.org",
            Stage::Relations => "query.wikidata.org",
            Stage::CoverArt => "coverartarchive.org",
        }
    }

    pub fn from_provider(provider: &str) -> Option<Stage> {
        Stage::ALL
            .into_iter()
            .find(|stage| stage.provider() == provider)
    }
}

/// Queues (or re-queues) a later stage for an entity once the stage before
/// it produced what it needs.
pub fn queue_stage(
    connection: &mut SqliteConnection,
    stage: Stage,
    entity_type: &str,
    entity_id: &str,
) -> QueryResult<()> {
    diesel::sql_query(
        "INSERT INTO metadata_task (provider, entity_type, entity_id) VALUES (?, ?, ?)
         ON CONFLICT(provider, entity_type, entity_id) DO UPDATE SET
             status = 'pending', attempts = 0, last_error = NULL, not_before = NULL,
             updated_at = CURRENT_TIMESTAMP",
    )
    .bind::<Text, _>(stage.provider())
    .bind::<Text, _>(entity_type)
    .bind::<Text, _>(entity_id)
    .execute(connection)?;
    Ok(())
}

/// The status of one stage's task for an entity.
pub fn task_status(
    connection: &mut SqliteConnection,
    stage: Stage,
    entity_type: &str,
    entity_id: &str,
) -> QueryResult<Option<String>> {
    let mut rows = diesel::sql_query(
        "SELECT status AS value FROM metadata_task
         WHERE provider = ? AND entity_type = ? AND entity_id = ?",
    )
    .bind::<Text, _>(stage.provider())
    .bind::<Text, _>(entity_type)
    .bind::<Text, _>(entity_id)
    .load::<ValueRow>(connection)?;
    Ok(rows.pop().map(|row| row.value))
}

/// Removes a later stage that no longer applies (no Wikidata item, no
/// article, no photo).
pub fn drop_stage(
    connection: &mut SqliteConnection,
    stage: Stage,
    entity_type: &str,
    entity_id: &str,
) -> QueryResult<()> {
    diesel::sql_query(
        "DELETE FROM metadata_task WHERE provider = ? AND entity_type = ? AND entity_id = ?",
    )
    .bind::<Text, _>(stage.provider())
    .bind::<Text, _>(entity_type)
    .bind::<Text, _>(entity_id)
    .execute(connection)?;
    Ok(())
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

/// The next due MusicBrainz task: albums first (their releases name the
/// artists' MusicBrainz ids), then artists. Track tasks are covered by their
/// album.
pub fn next_task(connection: &mut SqliteConnection) -> QueryResult<Option<Task>> {
    next_stage_task(connection, Stage::Musicbrainz)
}

/// The next due task in one stage's queue.
pub fn next_stage_task(
    connection: &mut SqliteConnection,
    stage: Stage,
) -> QueryResult<Option<Task>> {
    if stage == Stage::Musicbrainz {
        diesel::sql_query(
            "UPDATE metadata_task SET status = 'skipped', last_error = 'covered by the album',
                    updated_at = CURRENT_TIMESTAMP
             WHERE provider = 'musicbrainz' AND entity_type = 'track' AND status = 'pending'",
        )
        .execute(connection)?;
    }
    let mut tasks = diesel::sql_query(
        "SELECT id, entity_type, entity_id, attempts FROM metadata_task
         WHERE provider = ? AND status = 'pending'
           AND entity_type IN ('album', 'artist', 'genre', 'work')
           AND (not_before IS NULL OR not_before <= CURRENT_TIMESTAMP)
         ORDER BY attempts,
                  CASE entity_type WHEN 'album' THEN 0 WHEN 'artist' THEN 1
                                   WHEN 'genre' THEN 2 ELSE 3 END,
                  id
         LIMIT 1",
    )
    .bind::<Text, _>(stage.provider())
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

/// Records the outcome of one stage for an entity, as [`finish_task`] does
/// for a task the queue handed out.
pub fn finish_stage(
    connection: &mut SqliteConnection,
    stage: Stage,
    entity_type: &str,
    entity_id: &str,
    outcome: &Outcome,
) -> QueryResult<()> {
    let mut tasks = diesel::sql_query(
        "SELECT id, entity_type, entity_id, attempts FROM metadata_task
         WHERE provider = ? AND entity_type = ? AND entity_id = ?",
    )
    .bind::<Text, _>(stage.provider())
    .bind::<Text, _>(entity_type)
    .bind::<Text, _>(entity_id)
    .load::<Task>(connection)?;
    match tasks.pop() {
        Some(task) => finish_task(connection, &task, outcome),
        None => Ok(()),
    }
}

/// Queues a stage for an entity that has never had it; a finished or
/// pending task is left alone.
pub fn queue_stage_once(
    connection: &mut SqliteConnection,
    stage: Stage,
    entity_type: &str,
    entity_id: &str,
) -> QueryResult<bool> {
    let added = diesel::sql_query(
        "INSERT INTO metadata_task (provider, entity_type, entity_id) VALUES (?, ?, ?)
         ON CONFLICT(provider, entity_type, entity_id) DO NOTHING",
    )
    .bind::<Text, _>(stage.provider())
    .bind::<Text, _>(entity_type)
    .bind::<Text, _>(entity_id)
    .execute(connection)?;
    Ok(added > 0)
}

#[derive(QueryableByName)]
struct NamesRow {
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = Nullable<Text>)]
    artist: Option<String>,
}

/// The catalog's name for an entity (and an album's artist), for profiles
/// stored before they recorded names.
pub fn entity_names(
    connection: &mut SqliteConnection,
    entity_type: &str,
    entity_id: &str,
) -> QueryResult<Option<(String, Option<String>)>> {
    let query = match entity_type {
        "album" => {
            "SELECT al.title AS name,
                    (SELECT ar.name FROM album_artist aa JOIN artist_entity ar ON ar.id = aa.artist_id
                     WHERE aa.album_id = al.id ORDER BY aa.position LIMIT 1) AS artist
             FROM album_entity al WHERE al.id = ?"
        }
        "artist" => "SELECT name, NULL AS artist FROM artist_entity WHERE id = ?",
        "work" => "SELECT title AS name, composer_name AS artist FROM work WHERE mbid = ?",
        _ => return Ok(None),
    };
    let mut rows = diesel::sql_query(query)
        .bind::<Text, _>(entity_id)
        .load::<NamesRow>(connection)?;
    Ok(rows.pop().map(|row| (row.name, row.artist)))
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
    /// The stage still at work (`musicbrainz`, `wikidata`, `wikipedia` or
    /// `commons`), so a page can say what it is waiting for.
    #[diesel(sql_type = Text)]
    pub stage: String,
}

/// Where an entity's lookup stands across all stages: the MusicBrainz state
/// until that finished, then the first later stage still pending or failed.
pub fn task_state(
    connection: &mut SqliteConnection,
    entity_type: &str,
    entity_id: &str,
) -> QueryResult<Option<TaskState>> {
    let rows = diesel::sql_query(
        "SELECT status, last_error, CAST(updated_at AS TEXT) AS updated_at, provider AS stage
         FROM metadata_task WHERE entity_type = ? AND entity_id = ?",
    )
    .bind::<Text, _>(entity_type)
    .bind::<Text, _>(entity_id)
    .load::<TaskState>(connection)?;
    let by_stage = |stage: Stage| rows.iter().find(|row| row.stage == stage.provider());
    // Genres start at Wikipedia; they have no MusicBrainz stage.
    let Some(first) = by_stage(Stage::Musicbrainz)
        .or_else(|| by_stage(Stage::Wikipedia))
        .or_else(|| by_stage(Stage::Wikidata))
    else {
        return Ok(None);
    };
    if first.status != "done" {
        return Ok(Some(first.clone()));
    }
    let text_stages = || {
        Stage::ALL[1..]
            .iter()
            .filter(|stage| stage.feeds_page_text())
            .filter_map(|stage| by_stage(*stage))
    };
    Ok(text_stages()
        .filter(|row| row.stage != first.stage)
        .find(|row| row.status == "pending")
        .or_else(|| text_stages().find(|row| row.status == "failed"))
        .unwrap_or(first)
        .clone()
        .into())
}

/// Artists among `ids` that have a stored photo.
pub fn artists_with_images(
    connection: &mut SqliteConnection,
    ids: &[String],
) -> QueryResult<std::collections::HashSet<String>> {
    let mut found = std::collections::HashSet::new();
    for chunk in ids.chunks(400) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let query = format!(
            "SELECT entity_id AS value FROM editorial_image
             WHERE entity_type = 'artist' AND entity_id IN ({placeholders})"
        );
        let mut statement = diesel::sql_query(query).into_boxed::<diesel::sqlite::Sqlite>();
        for id in chunk {
            statement = statement.bind::<Text, _>(id.clone());
        }
        found.extend(
            statement
                .load::<ValueRow>(connection)?
                .into_iter()
                .map(|row| row.value),
        );
    }
    Ok(found)
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

/// A work's About text (the Wikipedia lead), when one was found.
pub fn work_background(
    connection: &mut SqliteConnection,
    mbid: &str,
) -> QueryResult<Option<EditorialText>> {
    Ok(load_texts(connection, "work", mbid)?
        .into_iter()
        .find(|text| text.kind == "background"))
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

/// A name a MusicBrainz person is known by.
#[derive(Debug, Clone, PartialEq, Eq, QueryableByName)]
pub struct AliasRow {
    #[diesel(sql_type = Text)]
    pub mbid: String,
    #[diesel(sql_type = Text)]
    pub normalized_name: String,
    #[diesel(sql_type = Text)]
    pub name: String,
    /// `name` (their MusicBrainz name), `credit` (a release credits them so)
    /// or `alias` (a MusicBrainz alias).
    #[diesel(sql_type = Text)]
    pub kind: String,
}

/// Remembers names a MusicBrainz person is known by. A person's own name
/// replaces a weaker kind stored for the same spelling.
pub fn save_aliases(connection: &mut SqliteConnection, aliases: &[AliasRow]) -> QueryResult<()> {
    for alias in aliases {
        if alias.normalized_name.is_empty() || alias.mbid.len() != 36 {
            continue;
        }
        diesel::sql_query(
            "INSERT INTO person_alias (mbid, normalized_name, name, kind) VALUES (?, ?, ?, ?)
             ON CONFLICT(mbid, normalized_name) DO UPDATE SET
                 name = CASE WHEN excluded.kind = 'name' THEN excluded.name ELSE person_alias.name END,
                 kind = CASE WHEN excluded.kind = 'name' OR person_alias.kind = 'name'
                             THEN 'name' ELSE person_alias.kind END",
        )
        .bind::<Text, _>(&alias.mbid)
        .bind::<Text, _>(&alias.normalized_name)
        .bind::<Text, _>(&alias.name)
        .bind::<Text, _>(&alias.kind)
        .execute(connection)?;
    }
    Ok(())
}

pub fn load_aliases(connection: &mut SqliteConnection) -> QueryResult<Vec<AliasRow>> {
    diesel::sql_query(
        "SELECT mbid, normalized_name, name, kind FROM person_alias ORDER BY mbid, normalized_name",
    )
    .load::<AliasRow>(connection)
}

#[derive(Debug, Clone, PartialEq, Eq, QueryableByName)]
pub struct PersonLinkRow {
    #[diesel(sql_type = Text)]
    pub artist_id: String,
    #[diesel(sql_type = Text)]
    pub mbid: String,
}

/// Every local person with a MusicBrainz id: ids MusicBrainz releases
/// linked, ids resolved for an artist page, and ids tagged on tracks with a
/// single primary artist.
pub fn person_links(connection: &mut SqliteConnection) -> QueryResult<Vec<PersonLinkRow>> {
    diesel::sql_query(
        "SELECT entity_id AS artist_id, external_id AS mbid FROM external_id
         WHERE entity_type = 'artist' AND provider = 'musicbrainz' AND length(external_id) = 36
         UNION
         SELECT entity_id AS artist_id, musicbrainz_id AS mbid FROM editorial_profile
         WHERE entity_type = 'artist' AND entity_id NOT LIKE 'mb:%'
           AND length(musicbrainz_id) = 36
         UNION
         SELECT ta.artist_id AS artist_id, rfm.musicbrainz_artist_id AS mbid
         FROM track_artist ta
         JOIN track_file tf ON tf.track_id = ta.track_id
         JOIN raw_file_metadata rfm ON rfm.file_id = tf.file_id
         WHERE ta.role = 'primary' AND length(rfm.musicbrainz_artist_id) = 36
           AND (SELECT count(*) FROM track_artist other
                WHERE other.track_id = ta.track_id AND other.role = 'primary') = 1
         LIMIT 50000",
    )
    .load::<PersonLinkRow>(connection)
}

#[derive(Debug, Clone, PartialEq, Eq, QueryableByName)]
pub struct CreditNameRow {
    #[diesel(sql_type = Text)]
    pub album_id: String,
    #[diesel(sql_type = Text)]
    pub normalized_name: String,
    #[diesel(sql_type = Text)]
    pub mbid: String,
}

/// Which MusicBrainz person each name credited on an album is.
pub fn credited_names(connection: &mut SqliteConnection) -> QueryResult<Vec<CreditNameRow>> {
    diesel::sql_query(
        "SELECT DISTINCT album_id, normalized_name, source_artist_id AS mbid
         FROM enriched_credit WHERE length(source_artist_id) = 36 LIMIT 200000",
    )
    .load::<CreditNameRow>(connection)
}

/// External credits for any of several names or MusicBrainz ids.
pub fn people_credits(
    connection: &mut SqliteConnection,
    normalized_names: &[String],
    mbids: &[String],
) -> QueryResult<Vec<CreditRow>> {
    let mut rows: Vec<CreditRow> = Vec::new();
    for name in normalized_names.iter().take(16) {
        for row in person_credits(connection, Some(name), None)? {
            if !rows.contains(&row) {
                rows.push(row);
            }
        }
    }
    for mbid in mbids.iter().take(16) {
        for row in person_credits(connection, None, Some(mbid))? {
            if !rows.contains(&row) {
                rows.push(row);
            }
        }
    }
    Ok(rows)
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

/// A stored work (composition).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, QueryableByName)]
pub struct WorkRow {
    #[diesel(sql_type = Text)]
    pub mbid: String,
    #[diesel(sql_type = Text)]
    pub title: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub form: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub musical_key: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub catalogue: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub composed: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub composer_name: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub composer_normalized: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub composer_mbid: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub language: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub parent_mbid: Option<String>,
    #[diesel(sql_type = Nullable<BigInt>)]
    pub part_position: Option<i64>,
}

/// A track performing a work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, QueryableByName)]
pub struct TrackWorkRow {
    #[diesel(sql_type = Text)]
    pub track_id: String,
    #[diesel(sql_type = Text)]
    pub album_id: String,
    #[diesel(sql_type = Text)]
    pub work_mbid: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub recording_mbid: Option<String>,
}

const WORK_COLUMNS: &str = "mbid, title, form, musical_key, catalogue, composed, composer_name,
     composer_normalized, composer_mbid, language, parent_mbid, part_position";

fn upsert_work(connection: &mut SqliteConnection, work: &WorkRow) -> QueryResult<()> {
    // A work seen only as a parent has fewer facts than the same work seen
    // directly; never replace a known fact with nothing.
    diesel::sql_query(
        "INSERT INTO work (mbid, title, form, musical_key, catalogue, composed, composer_name,
             composer_normalized, composer_mbid, language, parent_mbid, part_position)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(mbid) DO UPDATE SET
             title = excluded.title,
             form = COALESCE(excluded.form, work.form),
             musical_key = COALESCE(excluded.musical_key, work.musical_key),
             catalogue = COALESCE(excluded.catalogue, work.catalogue),
             composed = COALESCE(excluded.composed, work.composed),
             composer_name = COALESCE(excluded.composer_name, work.composer_name),
             composer_normalized = COALESCE(excluded.composer_normalized, work.composer_normalized),
             composer_mbid = COALESCE(excluded.composer_mbid, work.composer_mbid),
             language = COALESCE(excluded.language, work.language),
             parent_mbid = COALESCE(excluded.parent_mbid, work.parent_mbid),
             part_position = COALESCE(excluded.part_position, work.part_position),
             updated_at = CURRENT_TIMESTAMP",
    )
    .bind::<Text, _>(&work.mbid)
    .bind::<Text, _>(&work.title)
    .bind::<Nullable<Text>, _>(work.form.as_deref())
    .bind::<Nullable<Text>, _>(work.musical_key.as_deref())
    .bind::<Nullable<Text>, _>(work.catalogue.as_deref())
    .bind::<Nullable<Text>, _>(work.composed.as_deref())
    .bind::<Nullable<Text>, _>(work.composer_name.as_deref())
    .bind::<Nullable<Text>, _>(work.composer_normalized.as_deref())
    .bind::<Nullable<Text>, _>(work.composer_mbid.as_deref())
    .bind::<Nullable<Text>, _>(work.language.as_deref())
    .bind::<Nullable<Text>, _>(work.parent_mbid.as_deref())
    .bind::<Nullable<BigInt>, _>(work.part_position)
    .execute(connection)?;
    Ok(())
}

/// Stores the works an album's tracks perform and replaces the album's
/// MusicBrainz track-to-work links.
pub fn replace_album_works(
    connection: &mut SqliteConnection,
    album_id: &str,
    works: &[WorkRow],
    links: &[TrackWorkRow],
) -> QueryResult<()> {
    connection.transaction(|connection| {
        for work in works {
            upsert_work(connection, work)?;
        }
        diesel::sql_query("DELETE FROM track_work WHERE album_id = ? AND source = 'musicbrainz'")
            .bind::<Text, _>(album_id)
            .execute(connection)?;
        for link in links {
            diesel::sql_query(
                "INSERT INTO track_work (track_id, album_id, work_mbid, recording_mbid, source)
                 VALUES (?, ?, ?, ?, 'musicbrainz') ON CONFLICT DO NOTHING",
            )
            .bind::<Text, _>(&link.track_id)
            .bind::<Text, _>(&link.album_id)
            .bind::<Text, _>(&link.work_mbid)
            .bind::<Nullable<Text>, _>(link.recording_mbid.as_deref())
            .execute(connection)?;
        }
        Ok(())
    })
}

pub fn load_work(connection: &mut SqliteConnection, mbid: &str) -> QueryResult<Option<WorkRow>> {
    let mut rows = diesel::sql_query(format!("SELECT {WORK_COLUMNS} FROM work WHERE mbid = ?"))
        .bind::<Text, _>(mbid)
        .load::<WorkRow>(connection)?;
    Ok(rows.pop())
}

/// The movements (parts) of a work.
pub fn work_parts(connection: &mut SqliteConnection, mbid: &str) -> QueryResult<Vec<WorkRow>> {
    diesel::sql_query(format!(
        "SELECT {WORK_COLUMNS} FROM work WHERE parent_mbid = ?
         ORDER BY part_position IS NULL, part_position, title"
    ))
    .bind::<Text, _>(mbid)
    .load::<WorkRow>(connection)
}

/// Tracks performing a work or any of its parts.
pub fn work_tracks(
    connection: &mut SqliteConnection,
    mbid: &str,
) -> QueryResult<Vec<TrackWorkRow>> {
    diesel::sql_query(
        "SELECT track_id, album_id, work_mbid, recording_mbid FROM track_work
         WHERE work_mbid = ? OR work_mbid IN (SELECT mbid FROM work WHERE parent_mbid = ?)
         ORDER BY album_id, rowid",
    )
    .bind::<Text, _>(mbid)
    .bind::<Text, _>(mbid)
    .load::<TrackWorkRow>(connection)
}

/// The works a track performs, each with its parent when there is one.
pub fn track_works(
    connection: &mut SqliteConnection,
    track_id: &str,
) -> QueryResult<Vec<(WorkRow, Option<WorkRow>)>> {
    let works = diesel::sql_query(format!(
        "SELECT {WORK_COLUMNS} FROM work
         WHERE mbid IN (SELECT work_mbid FROM track_work WHERE track_id = ?)"
    ))
    .bind::<Text, _>(track_id)
    .load::<WorkRow>(connection)?;
    let mut result = Vec::new();
    for work in works {
        let parent = match &work.parent_mbid {
            Some(parent) => load_work(connection, parent)?,
            None => None,
        };
        result.push((work, parent));
    }
    Ok(result)
}

/// Track-to-work links for one album.
pub fn album_track_works(
    connection: &mut SqliteConnection,
    album_id: &str,
) -> QueryResult<Vec<TrackWorkRow>> {
    diesel::sql_query(
        "SELECT track_id, album_id, work_mbid, recording_mbid FROM track_work
         WHERE album_id = ? ORDER BY rowid",
    )
    .bind::<Text, _>(album_id)
    .load::<TrackWorkRow>(connection)
}

/// Works by mbid, for a set of links.
pub fn works_by_ids(
    connection: &mut SqliteConnection,
    mbids: &[String],
) -> QueryResult<Vec<WorkRow>> {
    let mut rows = Vec::new();
    for chunk in mbids.chunks(200) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let mut query = diesel::sql_query(format!(
            "SELECT {WORK_COLUMNS} FROM work WHERE mbid IN ({placeholders})"
        ))
        .into_boxed::<diesel::sqlite::Sqlite>();
        for mbid in chunk {
            query = query.bind::<Text, _>(mbid.clone());
        }
        rows.extend(query.load::<WorkRow>(connection)?);
    }
    Ok(rows)
}

/// Works whose composer has one of these MusicBrainz ids or this normalized
/// name, with the number of library tracks performing each (parts count
/// toward their parent).
pub fn composer_works(
    connection: &mut SqliteConnection,
    normalized_name: Option<&str>,
    mbids: &[String],
) -> QueryResult<Vec<(WorkRow, i64, i64)>> {
    #[derive(QueryableByName)]
    struct CountedWork {
        #[diesel(embed)]
        work: WorkRow,
        #[diesel(sql_type = BigInt)]
        track_count: i64,
        #[diesel(sql_type = BigInt)]
        album_count: i64,
    }
    let ids = mbids
        .iter()
        .filter(|mbid| super::musicbrainz::is_mbid(mbid))
        .map(|mbid| format!("'{mbid}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let by_id = if ids.is_empty() {
        String::new()
    } else {
        format!(" OR w.composer_mbid IN ({ids})")
    };
    let rows = diesel::sql_query(format!(
        "SELECT w.mbid, w.title, w.form, w.musical_key, w.catalogue, w.composed, w.composer_name,
                w.composer_normalized, w.composer_mbid, w.language, w.parent_mbid, w.part_position,
                (SELECT COUNT(DISTINCT tw.track_id) FROM track_work tw
                 WHERE tw.work_mbid = w.mbid
                    OR tw.work_mbid IN (SELECT p.mbid FROM work p WHERE p.parent_mbid = w.mbid))
                  AS track_count,
                (SELECT COUNT(DISTINCT tw.album_id) FROM track_work tw
                 WHERE tw.work_mbid = w.mbid
                    OR tw.work_mbid IN (SELECT p.mbid FROM work p WHERE p.parent_mbid = w.mbid))
                  AS album_count
         FROM work w
         WHERE w.parent_mbid IS NULL AND (w.composer_normalized = ?{by_id})
         ORDER BY w.title LIMIT 2000"
    ))
    .bind::<Text, _>(normalized_name.unwrap_or("\u{0}"))
    .load::<CountedWork>(connection)?;
    Ok(rows
        .into_iter()
        .map(|row| (row.work, row.track_count, row.album_count))
        .collect())
}

/// The MusicBrainz release an administrator chose for an album.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, QueryableByName)]
pub struct Identification {
    #[diesel(sql_type = Text)]
    pub release_mbid: String,
    #[diesel(sql_type = Text)]
    pub release_title: String,
    #[diesel(sql_type = Text)]
    pub release_artist: String,
    #[diesel(sql_type = Text)]
    pub created_at: String,
}

pub fn load_identification(
    connection: &mut SqliteConnection,
    album_id: &str,
) -> QueryResult<Option<Identification>> {
    let mut rows = diesel::sql_query(
        "SELECT release_mbid, release_title, release_artist, CAST(created_at AS TEXT) AS created_at
         FROM album_identification WHERE album_id = ?",
    )
    .bind::<Text, _>(album_id)
    .load::<Identification>(connection)?;
    Ok(rows.pop())
}

/// Pins an album to a release (or clears the pin with `None`) and queues
/// the album to be looked up again.
pub fn set_identification(
    connection: &mut SqliteConnection,
    album_id: &str,
    release: Option<(&str, &str, &str)>,
    user_id: Option<i32>,
) -> QueryResult<()> {
    connection.transaction(|connection| {
        match release {
            Some((mbid, title, artist)) => {
                diesel::sql_query(
                    "INSERT INTO album_identification
                         (album_id, release_mbid, release_title, release_artist, identified_by)
                     VALUES (?, ?, ?, ?, ?)
                     ON CONFLICT(album_id) DO UPDATE SET
                         release_mbid = excluded.release_mbid,
                         release_title = excluded.release_title,
                         release_artist = excluded.release_artist,
                         identified_by = excluded.identified_by,
                         created_at = CURRENT_TIMESTAMP",
                )
                .bind::<Text, _>(album_id)
                .bind::<Text, _>(mbid)
                .bind::<Text, _>(title)
                .bind::<Text, _>(artist)
                .bind::<Nullable<Integer>, _>(user_id)
                .execute(connection)?;
            }
            None => {
                diesel::sql_query("DELETE FROM album_identification WHERE album_id = ?")
                    .bind::<Text, _>(album_id)
                    .execute(connection)?;
            }
        }
        refresh_entity(connection, "album", album_id)
    })
}

/// The match check stored for an album's MusicBrainz release.
#[derive(Debug, Clone, PartialEq, Eq, QueryableByName)]
pub struct ReleaseMatchRow {
    #[diesel(sql_type = Text)]
    pub status: String,
    #[diesel(sql_type = Text)]
    pub method: String,
    #[diesel(sql_type = Text)]
    pub release_mbid: String,
    #[diesel(sql_type = Text)]
    pub release_title: String,
    #[diesel(sql_type = Text)]
    pub release_artist: String,
    #[diesel(sql_type = Integer)]
    pub score: i32,
    #[diesel(sql_type = Integer)]
    pub threshold: i32,
    #[diesel(sql_type = Text)]
    pub details_json: String,
}

pub fn save_release_match(
    connection: &mut SqliteConnection,
    album_id: &str,
    row: &ReleaseMatchRow,
) -> QueryResult<()> {
    diesel::sql_query(
        "INSERT INTO album_release_match
             (album_id, status, method, release_mbid, release_title, release_artist,
              score, threshold, details_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(album_id) DO UPDATE SET
             status = excluded.status, method = excluded.method,
             release_mbid = excluded.release_mbid, release_title = excluded.release_title,
             release_artist = excluded.release_artist, score = excluded.score,
             threshold = excluded.threshold, details_json = excluded.details_json,
             updated_at = CURRENT_TIMESTAMP",
    )
    .bind::<Text, _>(album_id)
    .bind::<Text, _>(&row.status)
    .bind::<Text, _>(&row.method)
    .bind::<Text, _>(&row.release_mbid)
    .bind::<Text, _>(&row.release_title)
    .bind::<Text, _>(&row.release_artist)
    .bind::<Integer, _>(row.score)
    .bind::<Integer, _>(row.threshold)
    .bind::<Text, _>(&row.details_json)
    .execute(connection)?;
    Ok(())
}

pub fn load_release_match(
    connection: &mut SqliteConnection,
    album_id: &str,
) -> QueryResult<Option<ReleaseMatchRow>> {
    let mut rows = diesel::sql_query(
        "SELECT status, method, release_mbid, release_title, release_artist, score, threshold,
                details_json
         FROM album_release_match WHERE album_id = ?",
    )
    .bind::<Text, _>(album_id)
    .load::<ReleaseMatchRow>(connection)?;
    Ok(rows.pop())
}

pub fn delete_release_match(connection: &mut SqliteConnection, album_id: &str) -> QueryResult<()> {
    diesel::sql_query("DELETE FROM album_release_match WHERE album_id = ?")
        .bind::<Text, _>(album_id)
        .execute(connection)?;
    Ok(())
}

/// Removes everything enrichment stored for an entity (used when a match
/// turns out to be wrong, e.g. a placeholder artist name).
pub fn clear_editorial(
    connection: &mut SqliteConnection,
    entity_type: &str,
    entity_id: &str,
) -> QueryResult<()> {
    for table in ["editorial_profile", "editorial_text", "editorial_image"] {
        diesel::sql_query(format!(
            "DELETE FROM {table} WHERE entity_type = ? AND entity_id = ?"
        ))
        .bind::<Text, _>(entity_type)
        .bind::<Text, _>(entity_id)
        .execute(connection)?;
    }
    if entity_type == "artist" {
        diesel::sql_query("DELETE FROM person_influence WHERE artist_id = ?")
            .bind::<Text, _>(entity_id)
            .execute(connection)?;
    }
    if entity_type == "album" {
        diesel::sql_query("DELETE FROM album_cover_art WHERE album_id = ?")
            .bind::<Text, _>(entity_id)
            .execute(connection)?;
        diesel::sql_query(
            "DELETE FROM enriched_credit WHERE album_id = ? AND source = 'musicbrainz'",
        )
        .bind::<Text, _>(entity_id)
        .execute(connection)?;
        diesel::sql_query("DELETE FROM track_work WHERE album_id = ? AND source = 'musicbrainz'")
            .bind::<Text, _>(entity_id)
            .execute(connection)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::connection::migrated_memory_pool;
    use diesel::connection::SimpleConnection;

    #[test]
    fn aliases_keep_a_persons_own_name_over_a_credit() {
        let pool = migrated_memory_pool();
        let mut connection = pool.get().unwrap();
        let mbid = "7a9b2c5a-4e7e-4b8c-9c7a-1d2e3f4a5b6c";
        let alias = |name: &str, kind: &str| AliasRow {
            mbid: mbid.into(),
            normalized_name: crate::library::identity::normalize_artist_identity(name),
            name: name.into(),
            kind: kind.into(),
        };
        save_aliases(
            &mut connection,
            &[alias("Jonathan Davis", "credit"), alias("Q-Tip", "credit")],
        )
        .unwrap();
        save_aliases(&mut connection, &[alias("Q-Tip", "name")]).unwrap();
        // A later credit does not demote their own name.
        save_aliases(&mut connection, &[alias("Q-Tip", "credit")]).unwrap();
        let rows = load_aliases(&mut connection).unwrap();
        let kinds = rows
            .iter()
            .map(|row| (row.name.as_str(), row.kind.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(kinds, vec![("Jonathan Davis", "credit"), ("Q-Tip", "name")]);
        // Bad ids are ignored.
        save_aliases(
            &mut connection,
            &[AliasRow {
                mbid: "nope".into(),
                ..alias("X", "name")
            }],
        )
        .unwrap();
        assert_eq!(load_aliases(&mut connection).unwrap().len(), 2);
    }

    #[test]
    fn person_links_and_credited_names_come_from_every_source() {
        let pool = migrated_memory_pool();
        let mut connection = pool.get().unwrap();
        let mbid = "7a9b2c5a-4e7e-4b8c-9c7a-1d2e3f4a5b6c";
        link_artist_mbid(&mut connection, "artist-qtip", mbid).unwrap();
        save_profile(
            &mut connection,
            "artist",
            "tag-qtip",
            &ProfileRow {
                musicbrainz_id: Some(mbid.into()),
                ..ProfileRow::default()
            },
        )
        .unwrap();
        replace_album_credits(
            &mut connection,
            "low-end",
            "musicbrainz",
            &[CreditRow {
                album_id: "low-end".into(),
                track_id: String::new(),
                name: "Jonathan Davis".into(),
                normalized_name: "jonathan davis".into(),
                role: "producer".into(),
                instrument: String::new(),
                source: "musicbrainz".into(),
                source_artist_id: Some(mbid.into()),
            }],
        )
        .unwrap();
        let mut links = person_links(&mut connection)
            .unwrap()
            .into_iter()
            .map(|row| row.artist_id)
            .collect::<Vec<_>>();
        links.sort();
        assert_eq!(links, vec!["artist-qtip", "tag-qtip"]);
        let names = credited_names(&mut connection).unwrap();
        assert_eq!(names.len(), 1);
        assert_eq!(names[0].normalized_name, "jonathan davis");
        let found =
            people_credits(&mut connection, &["jonathan davis".into()], &[mbid.into()]).unwrap();
        assert_eq!(found.len(), 1, "one row found twice is listed once");
    }

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
            genre_source: crate::enrichment::genres::GenreSource::Tags,
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
    fn seeding_queues_people_credited_only_on_musicbrainz() {
        const MACERO: &str = "2a3b4c5d-6e7f-4a8b-9c0d-1e2f3a4b5c6d";
        const EVANS: &str = "c4b9e4a2-4f6f-4b39-9b8c-7a2e0f0a1b2c";
        let pool = migrated_memory_pool();
        let mut connection = pool.get().unwrap();
        connection
            .batch_execute(&format!(
                "INSERT INTO enriched_credit
                   (album_id, track_id, name, normalized_name, role, source, source_artist_id)
                 VALUES ('kob', '', 'Teo Macero', 'teo macero', 'producer', 'musicbrainz', '{MACERO}'),
                        ('kob', 't3', 'Teo Macero', 'teo macero', 'producer', 'musicbrainz', '{MACERO}'),
                        ('kob', 't3', 'Bill Evans', 'bill evans', 'performer', 'musicbrainz', '{EVANS}'),
                        ('kob', 't1', 'Nobody', 'nobody', 'engineer', 'musicbrainz', NULL);"
            ))
            .unwrap();
        // Bill Evans is a library artist, enriched under his own id.
        link_artist_mbid(&mut connection, "artist-evans", EVANS).unwrap();
        assert_eq!(seed_tasks(&mut connection).unwrap(), 1);
        assert_eq!(seed_tasks(&mut connection).unwrap(), 0);
        let task = next_stage_task(&mut connection, Stage::Musicbrainz)
            .unwrap()
            .unwrap();
        assert_eq!(task.entity_id, format!("mb:{MACERO}"));
    }

    #[test]
    fn seeding_queues_library_items_once_without_a_syntax_error() {
        let pool = migrated_memory_pool();
        let mut connection = pool.get().unwrap();
        connection
            .batch_execute(
                "INSERT INTO artist_entity (id, name, normalized_name) VALUES
                   ('beethoven', 'Ludwig van Beethoven', 'ludwig van beethoven'),
                   ('karajan', 'Herbert von Karajan', 'herbert von karajan');
                 INSERT INTO album_entity (id, title, normalized_title) VALUES
                   ('ninth', 'Symphony No. 9', 'symphony no 9');
                 INSERT INTO metadata_task (provider, entity_type, entity_id, status)
                   VALUES ('musicbrainz', 'artist', 'karajan', 'done');",
            )
            .unwrap();
        // Karajan already has a task; Beethoven and the album are new.
        assert_eq!(seed_tasks(&mut connection).unwrap(), 2);
        assert_eq!(seed_tasks(&mut connection).unwrap(), 0);
        let counts = task_counts(&mut connection).unwrap();
        assert_eq!((counts.pending, counts.done), (2, 1));
        assert_eq!(
            next_task(&mut connection)
                .unwrap()
                .map(|task| task.entity_id),
            Some("ninth".into())
        );
        // An artist matched by an earlier version gets its Wikidata stage.
        save_profile(
            &mut connection,
            "artist",
            "karajan",
            &ProfileRow {
                wikidata_id: Some("Q78491".into()),
                ..ProfileRow::default()
            },
        )
        .unwrap();
        // It also gets a Wikipedia stage of its own, which does not wait
        // for Wikidata, and its influences are looked up.
        assert_eq!(seed_tasks(&mut connection).unwrap(), 3);
        assert_eq!(
            next_stage_task(&mut connection, Stage::Relations)
                .unwrap()
                .map(|task| task.entity_id),
            Some("karajan".into())
        );
        assert_eq!(
            next_stage_task(&mut connection, Stage::Wikidata)
                .unwrap()
                .map(|task| task.entity_id),
            Some("karajan".into())
        );
        assert_eq!(
            next_stage_task(&mut connection, Stage::Wikipedia)
                .unwrap()
                .map(|task| task.entity_id),
            Some("karajan".into())
        );
    }

    #[test]
    fn each_stage_has_its_own_queue_and_the_state_names_the_waiting_stage() {
        let pool = migrated_memory_pool();
        let mut connection = pool.get().unwrap();
        refresh_entity(&mut connection, "artist", "a1").unwrap();
        let task = next_task(&mut connection).unwrap().unwrap();
        assert_eq!(
            task_state(&mut connection, "artist", "a1")
                .unwrap()
                .unwrap()
                .stage,
            "musicbrainz"
        );
        finish_task(&mut connection, &task, &Outcome::Done).unwrap();
        queue_stage(&mut connection, Stage::Wikipedia, "artist", "a1").unwrap();
        queue_stage(&mut connection, Stage::Commons, "artist", "a1").unwrap();
        assert_eq!(next_task(&mut connection).unwrap(), None);
        let state = task_state(&mut connection, "artist", "a1")
            .unwrap()
            .unwrap();
        assert_eq!(
            (state.status.as_str(), state.stage.as_str()),
            ("pending", "wikipedia")
        );

        // A busy Commons does not stop the Wikipedia queue.
        let photo = next_stage_task(&mut connection, Stage::Commons)
            .unwrap()
            .unwrap();
        finish_task(&mut connection, &photo, &Outcome::Retry("HTTP 429".into())).unwrap();
        assert_eq!(
            next_stage_task(&mut connection, Stage::Commons).unwrap(),
            None
        );
        let text = next_stage_task(&mut connection, Stage::Wikipedia)
            .unwrap()
            .unwrap();
        assert_eq!(text.entity_id, "a1");
        finish_task(&mut connection, &text, &Outcome::Done).unwrap();
        let state = task_state(&mut connection, "artist", "a1")
            .unwrap()
            .unwrap();
        assert_eq!(
            (state.status.as_str(), state.stage.as_str()),
            ("pending", "commons")
        );
        assert_eq!(
            stage_counts(&mut connection, Stage::Wikipedia)
                .unwrap()
                .done,
            1
        );
        drop_stage(&mut connection, Stage::Commons, "artist", "a1").unwrap();
        assert_eq!(
            task_state(&mut connection, "artist", "a1")
                .unwrap()
                .unwrap()
                .stage,
            "musicbrainz"
        );
        assert_eq!(Stage::from_provider("commons"), Some(Stage::Commons));
    }

    #[test]
    fn photos_are_found_for_credited_people() {
        let pool = migrated_memory_pool();
        let mut connection = pool.get().unwrap();
        save_image(
            &mut connection,
            "artist",
            "kleiber",
            &ImageRow {
                file_name: "artist-kleiber.jpg".into(),
                mime_type: "image/jpeg".into(),
                source_url: "https://upload.wikimedia.org/k.jpg".into(),
                page_url: "https://commons.wikimedia.org/wiki/File:K.jpg".into(),
                author: "Unknown".into(),
                license: "CC BY-SA 3.0".into(),
                license_url: None,
            },
        )
        .unwrap();
        let found =
            artists_with_images(&mut connection, &["kleiber".into(), "nobody".into()]).unwrap();
        assert_eq!(found.into_iter().collect::<Vec<_>>(), ["kleiber"]);
    }

    fn work(mbid: &str, title: &str, parent: Option<&str>, position: Option<i64>) -> WorkRow {
        WorkRow {
            mbid: mbid.into(),
            title: title.into(),
            composer_name: Some("Ludwig van Beethoven".into()),
            composer_normalized: Some("ludwig van beethoven".into()),
            composer_mbid: Some("1f9df192-a621-4f54-8850-2c5373b7eac9".into()),
            parent_mbid: parent.map(str::to_string),
            part_position: position,
            ..WorkRow::default()
        }
    }

    fn link(track: &str, album: &str, work: &str) -> TrackWorkRow {
        TrackWorkRow {
            track_id: track.into(),
            album_id: album.into(),
            work_mbid: work.into(),
            recording_mbid: None,
        }
    }

    #[test]
    fn works_link_tracks_to_movements_and_movements_to_the_piece() {
        let pool = migrated_memory_pool();
        let mut connection = pool.get().unwrap();
        let symphony = WorkRow {
            form: Some("Symphony".into()),
            musical_key: Some("D minor".into()),
            catalogue: Some("Op. 125".into()),
            ..work("sym", "Symphony No. 9", None, None)
        };
        replace_album_works(
            &mut connection,
            "karajan",
            &[
                symphony.clone(),
                work("m1", "Symphony No. 9: I.", Some("sym"), Some(1)),
                work("m4", "Symphony No. 9: IV.", Some("sym"), Some(4)),
            ],
            &[link("k1", "karajan", "m1"), link("k4", "karajan", "m4")],
        )
        .unwrap();
        // A second recording of the finale only; the parent is seen without
        // its facts, which must not erase them.
        replace_album_works(
            &mut connection,
            "bernstein",
            &[
                WorkRow {
                    form: None,
                    ..work("sym", "Symphony No. 9", None, None)
                },
                work("m4", "Symphony No. 9: IV.", Some("sym"), Some(4)),
            ],
            &[link("b4", "bernstein", "m4")],
        )
        .unwrap();
        let stored = load_work(&mut connection, "sym").unwrap().unwrap();
        assert_eq!(stored.form.as_deref(), Some("Symphony"));
        assert_eq!(stored.catalogue.as_deref(), Some("Op. 125"));
        let parts = work_parts(&mut connection, "sym").unwrap();
        assert_eq!(
            parts
                .iter()
                .map(|part| part.mbid.as_str())
                .collect::<Vec<_>>(),
            ["m1", "m4"]
        );
        let tracks = work_tracks(&mut connection, "sym").unwrap();
        let mut ids = tracks
            .iter()
            .map(|track| track.track_id.as_str())
            .collect::<Vec<_>>();
        ids.sort_unstable();
        assert_eq!(ids, ["b4", "k1", "k4"]);
        let works = track_works(&mut connection, "k4").unwrap();
        assert_eq!(works.len(), 1);
        assert_eq!(
            works[0].1.as_ref().map(|parent| parent.mbid.as_str()),
            Some("sym")
        );

        let by_name = composer_works(&mut connection, Some("ludwig van beethoven"), &[]).unwrap();
        assert_eq!(by_name.len(), 1);
        assert_eq!(
            (by_name[0].0.mbid.as_str(), by_name[0].1, by_name[0].2),
            ("sym", 3, 2)
        );
        let by_id = composer_works(
            &mut connection,
            None,
            &[
                "1f9df192-a621-4f54-8850-2c5373b7eac9".into(),
                "x'; DROP".into(),
            ],
        )
        .unwrap();
        assert_eq!(by_id.len(), 1);

        // Looking the album up again replaces its links but keeps others'.
        replace_album_works(&mut connection, "karajan", &[], &[]).unwrap();
        assert_eq!(work_tracks(&mut connection, "sym").unwrap().len(), 1);
    }

    #[test]
    fn identifying_an_album_pins_the_release_and_queues_a_lookup() {
        let pool = migrated_memory_pool();
        let mut connection = pool.get().unwrap();
        set_identification(
            &mut connection,
            "album-1",
            Some(("rel", "Symphonie Nr. 9", "Karajan")),
            Some(1),
        )
        .unwrap();
        let pinned = load_identification(&mut connection, "album-1")
            .unwrap()
            .unwrap();
        assert_eq!(pinned.release_mbid, "rel");
        assert_eq!(
            task_state(&mut connection, "album", "album-1")
                .unwrap()
                .unwrap()
                .status,
            "pending"
        );
        set_identification(&mut connection, "album-1", None, None).unwrap();
        assert_eq!(
            load_identification(&mut connection, "album-1").unwrap(),
            None
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

/// A genre an external source attached to an album or artist.
#[derive(Debug, Clone, PartialEq, Eq, QueryableByName)]
pub struct GenreLinkRow {
    #[diesel(sql_type = Text)]
    pub entity_type: String,
    #[diesel(sql_type = Text)]
    pub entity_id: String,
    #[diesel(sql_type = Text)]
    pub name: String,
    #[diesel(sql_type = Text)]
    pub source: String,
    #[diesel(sql_type = BigInt)]
    pub weight: i64,
    #[diesel(sql_type = Nullable<Text>)]
    pub mbid: Option<String>,
}

/// Replaces the genres one source gave an album or artist.
pub fn replace_genre_links(
    connection: &mut SqliteConnection,
    entity_type: &str,
    entity_id: &str,
    source: &str,
    genres: &[crate::enrichment::musicbrainz::MbGenre],
) -> QueryResult<()> {
    connection.transaction(|connection| {
        diesel::sql_query(
            "DELETE FROM genre_link WHERE entity_type = ? AND entity_id = ? AND source = ?",
        )
        .bind::<Text, _>(entity_type)
        .bind::<Text, _>(entity_id)
        .bind::<Text, _>(source)
        .execute(connection)?;
        for genre in genres {
            diesel::sql_query(
                "INSERT INTO genre_link (entity_type, entity_id, name, source, weight, mbid)
                 VALUES (?, ?, ?, ?, ?, ?)
                 ON CONFLICT(entity_type, entity_id, name, source) DO UPDATE SET
                     weight = MAX(weight, excluded.weight)",
            )
            .bind::<Text, _>(entity_type)
            .bind::<Text, _>(entity_id)
            .bind::<Text, _>(&genre.name)
            .bind::<Text, _>(source)
            .bind::<BigInt, _>(genre.count)
            .bind::<Nullable<Text>, _>(genre.mbid.as_deref())
            .execute(connection)?;
        }
        Ok(())
    })
}

/// Every external genre of one entity type, for building the genre index.
pub fn genre_links(
    connection: &mut SqliteConnection,
    entity_type: &str,
) -> QueryResult<Vec<GenreLinkRow>> {
    diesel::sql_query(
        "SELECT entity_type, entity_id, name, source, CAST(weight AS BIGINT) AS weight, mbid
         FROM genre_link WHERE entity_type = ? ORDER BY entity_id, weight DESC, name",
    )
    .bind::<Text, _>(entity_type)
    .load::<GenreLinkRow>(connection)
}

#[derive(QueryableByName)]
struct AlbumNameRow {
    #[diesel(sql_type = Text)]
    album_id: String,
    #[diesel(sql_type = Text)]
    name: String,
}

/// Genres read from the files' tags (GENRE, TCON, ©gen) for every album.
pub fn album_tag_genres(
    connection: &mut SqliteConnection,
) -> QueryResult<std::collections::HashMap<String, Vec<String>>> {
    let rows = diesel::sql_query(
        "SELECT ag.album_id AS album_id, g.name AS name
         FROM album_genre ag JOIN genre_entity g ON g.id = ag.genre_id
         ORDER BY ag.album_id, g.normalized_name",
    )
    .load::<AlbumNameRow>(connection)?;
    let mut genres = std::collections::HashMap::<String, Vec<String>>::new();
    for row in rows {
        genres.entry(row.album_id).or_default().push(row.name);
    }
    Ok(genres)
}

/// Albums whose genres an administrator chose by hand.
pub fn album_genre_edits(
    connection: &mut SqliteConnection,
) -> QueryResult<std::collections::HashMap<String, Vec<String>>> {
    let rows = diesel::sql_query("SELECT album_id, genres_json AS name FROM album_genre_edit")
        .load::<AlbumNameRow>(connection)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.album_id,
                serde_json::from_str::<Vec<String>>(&row.name).unwrap_or_default(),
            )
        })
        .collect())
}

/// Sets an album's genres by hand, or (with `None`) goes back to tag and
/// online genres.
pub fn set_album_genre_edit(
    connection: &mut SqliteConnection,
    album_id: &str,
    genres: Option<&[String]>,
    user_id: Option<i32>,
) -> QueryResult<()> {
    match genres {
        Some(genres) => {
            let value = serde_json::to_string(genres).unwrap_or_else(|_| "[]".to_string());
            diesel::sql_query(
                "INSERT INTO album_genre_edit (album_id, genres_json, edited_by) VALUES (?, ?, ?)
                 ON CONFLICT(album_id) DO UPDATE SET genres_json = excluded.genres_json,
                     edited_by = excluded.edited_by, updated_at = CURRENT_TIMESTAMP",
            )
            .bind::<Text, _>(album_id)
            .bind::<Text, _>(value)
            .bind::<Nullable<Integer>, _>(user_id)
            .execute(connection)?;
        }
        None => {
            diesel::sql_query("DELETE FROM album_genre_edit WHERE album_id = ?")
                .bind::<Text, _>(album_id)
                .execute(connection)?;
        }
    }
    Ok(())
}

/// Queues a genre's About text (the Wikipedia article the genre tree
/// names) unless it was already looked up. Returns whether a task was added.
pub fn queue_genre(connection: &mut SqliteConnection, slug: &str) -> QueryResult<bool> {
    let added = diesel::sql_query(
        "INSERT INTO metadata_task (provider, entity_type, entity_id) VALUES (?, 'genre', ?)
         ON CONFLICT(provider, entity_type, entity_id) DO NOTHING",
    )
    .bind::<Text, _>(Stage::Wikipedia.provider())
    .bind::<Text, _>(slug)
    .execute(connection)?;
    Ok(added > 0)
}

/// Queues About texts for every canonical genre (and its parents) the
/// library's tags or MusicBrainz name.
pub fn seed_genre_tasks(connection: &mut SqliteConnection) -> QueryResult<usize> {
    use crate::enrichment::genres;
    let mut names = album_tag_genres(connection)?
        .into_values()
        .flatten()
        .collect::<Vec<_>>();
    names.extend(
        genre_links(connection, "album")?
            .into_iter()
            .map(|row| row.name),
    );
    names.extend(
        genre_links(connection, "artist")?
            .into_iter()
            .map(|row| row.name),
    );
    let slugs = genres::with_ancestors(
        names
            .iter()
            .filter_map(|name| genres::canonical(name))
            .map(|node| node.slug),
    );
    let mut queued = 0;
    for slug in slugs {
        if queue_genre(connection, &slug)? {
            queued += 1;
        }
    }
    Ok(queued)
}

/// Every external credit with one role, for genre pages' composers.
pub fn credits_with_role(
    connection: &mut SqliteConnection,
    role: &str,
) -> QueryResult<Vec<CreditRow>> {
    diesel::sql_query(format!(
        "SELECT {CREDIT_COLUMNS} FROM enriched_credit WHERE role = ? LIMIT 50000"
    ))
    .bind::<Text, _>(role)
    .load::<CreditRow>(connection)
}

/// Someone Wikidata says influenced a person (`influence`) or was
/// influenced by them (`follower`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, QueryableByName)]
pub struct InfluenceRow {
    #[diesel(sql_type = Text)]
    pub artist_id: String,
    /// `influence` or `follower`.
    #[diesel(sql_type = Text)]
    pub direction: String,
    #[diesel(sql_type = Text)]
    pub wikidata_id: String,
    #[diesel(sql_type = Text)]
    pub name: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub description: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub musicbrainz_id: Option<String>,
    /// Commons file name of their Wikidata image (P18).
    #[diesel(sql_type = Nullable<Text>)]
    pub image: Option<String>,
    #[diesel(sql_type = Integer)]
    pub position: i32,
}

/// Replaces what Wikidata says about a person's influences and followers.
pub fn replace_influences(
    connection: &mut SqliteConnection,
    artist_id: &str,
    rows: &[InfluenceRow],
) -> QueryResult<()> {
    connection.transaction(|connection| {
        diesel::sql_query("DELETE FROM person_influence WHERE artist_id = ?")
            .bind::<Text, _>(artist_id)
            .execute(connection)?;
        for row in rows {
            diesel::sql_query(
                "INSERT INTO person_influence (artist_id, direction, wikidata_id, name,
                     description, musicbrainz_id, image, position)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING",
            )
            .bind::<Text, _>(artist_id)
            .bind::<Text, _>(&row.direction)
            .bind::<Text, _>(&row.wikidata_id)
            .bind::<Text, _>(&row.name)
            .bind::<Nullable<Text>, _>(&row.description)
            .bind::<Nullable<Text>, _>(&row.musicbrainz_id)
            .bind::<Nullable<Text>, _>(&row.image)
            .bind::<Integer, _>(row.position)
            .execute(connection)?;
        }
        Ok(())
    })
}

/// Influences and followers stored under any of a person's ids.
pub fn load_influences(
    connection: &mut SqliteConnection,
    artist_ids: &[String],
) -> QueryResult<Vec<InfluenceRow>> {
    let mut rows = Vec::new();
    for id in artist_ids.iter().take(16) {
        rows.extend(
            diesel::sql_query(
                "SELECT artist_id, direction, wikidata_id, name, description, musicbrainz_id,
                        image, position
                 FROM person_influence WHERE artist_id = ? ORDER BY direction, position",
            )
            .bind::<Text, _>(id)
            .load::<InfluenceRow>(connection)?,
        );
    }
    Ok(rows)
}

#[derive(Debug, Clone, PartialEq, Eq, QueryableByName)]
pub struct WikidataLinkRow {
    #[diesel(sql_type = Text)]
    pub entity_id: String,
    #[diesel(sql_type = Text)]
    pub wikidata_id: String,
}

/// People already resolved on this server, by their Wikidata items.
pub fn artists_by_wikidata(
    connection: &mut SqliteConnection,
    wikidata_ids: &[String],
) -> QueryResult<Vec<WikidataLinkRow>> {
    let mut found = Vec::new();
    for chunk in wikidata_ids.chunks(400) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let query = format!(
            "SELECT entity_id, wikidata_id FROM editorial_profile
             WHERE entity_type = 'artist' AND wikidata_id IN ({placeholders})"
        );
        let mut statement = diesel::sql_query(query).into_boxed::<diesel::sqlite::Sqlite>();
        for id in chunk {
            statement = statement.bind::<Text, _>(id.clone());
        }
        found.extend(statement.load::<WikidataLinkRow>(connection)?);
    }
    Ok(found)
}

/// A front cover fetched from the Cover Art Archive.
#[derive(Debug, Clone, PartialEq, Eq, QueryableByName)]
pub struct CoverArtRow {
    #[diesel(sql_type = Text)]
    pub album_id: String,
    /// Absolute path of the stored image.
    #[diesel(sql_type = Text)]
    pub file_path: String,
    #[diesel(sql_type = Text)]
    pub mime_type: String,
    /// The Cover Art Archive URL it was fetched from.
    #[diesel(sql_type = Text)]
    pub source_url: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub release_mbid: Option<String>,
}

pub fn save_cover_art(connection: &mut SqliteConnection, row: &CoverArtRow) -> QueryResult<()> {
    diesel::sql_query(
        "INSERT INTO album_cover_art (album_id, file_path, mime_type, source_url, release_mbid)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(album_id) DO UPDATE SET file_path = excluded.file_path,
             mime_type = excluded.mime_type, source_url = excluded.source_url,
             release_mbid = excluded.release_mbid, updated_at = CURRENT_TIMESTAMP",
    )
    .bind::<Text, _>(&row.album_id)
    .bind::<Text, _>(&row.file_path)
    .bind::<Text, _>(&row.mime_type)
    .bind::<Text, _>(&row.source_url)
    .bind::<Nullable<Text>, _>(&row.release_mbid)
    .execute(connection)?;
    Ok(())
}

pub fn load_cover_arts(connection: &mut SqliteConnection) -> QueryResult<Vec<CoverArtRow>> {
    diesel::sql_query(
        "SELECT album_id, file_path, mime_type, source_url, release_mbid FROM album_cover_art
         LIMIT 200000",
    )
    .load::<CoverArtRow>(connection)
}

pub fn load_cover_art(
    connection: &mut SqliteConnection,
    album_id: &str,
) -> QueryResult<Option<CoverArtRow>> {
    let mut rows = diesel::sql_query(
        "SELECT album_id, file_path, mime_type, source_url, release_mbid FROM album_cover_art
         WHERE album_id = ?",
    )
    .bind::<Text, _>(album_id)
    .load::<CoverArtRow>(connection)?;
    Ok(rows.pop())
}

/// Albums with a recording of a work by a composer (by MusicBrainz id or
/// normalized name), including works whose movements are linked.
pub fn composer_work_albums(
    connection: &mut SqliteConnection,
    normalized_names: &[String],
    mbids: &[String],
) -> QueryResult<Vec<String>> {
    let mut albums: Vec<String> = Vec::new();
    let mut add = |rows: Vec<ValueRow>| {
        for row in rows {
            if !albums.contains(&row.value) {
                albums.push(row.value);
            }
        }
    };
    const QUERY: &str = "SELECT DISTINCT tw.album_id AS value FROM track_work tw
         JOIN work w ON w.mbid = tw.work_mbid
         LEFT JOIN work p ON p.mbid = w.parent_mbid
         WHERE w.{column} = ? OR p.{column} = ? LIMIT 2000";
    for name in normalized_names.iter().take(16) {
        add(
            diesel::sql_query(QUERY.replace("{column}", "composer_normalized"))
                .bind::<Text, _>(name)
                .bind::<Text, _>(name)
                .load::<ValueRow>(connection)?,
        );
    }
    for mbid in mbids.iter().take(16) {
        add(
            diesel::sql_query(QUERY.replace("{column}", "composer_mbid"))
                .bind::<Text, _>(mbid)
                .bind::<Text, _>(mbid)
                .load::<ValueRow>(connection)?,
        );
    }
    Ok(albums)
}
