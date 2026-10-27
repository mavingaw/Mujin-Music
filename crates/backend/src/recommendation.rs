use std::collections::{HashMap, HashSet};

use diesel::deserialize::QueryableByName;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Integer, Nullable, Text};
use serde::{Deserialize, Serialize};

use crate::library::state::LibraryCache;
use crate::persistence::connection::DbPool;
use crate::radio_bans::RadioBans;

const MAX_PROFILE_TRACKS: i64 = 250;
const MAX_PROFILE_ARTISTS: i64 = 24;
const MAX_TRANSITIONS: i64 = 80;
const RECENT_EXCLUSION_COUNT: i64 = 40;
const MAX_PLAYBACK_EVENTS_PER_USER: i64 = 20_000;
/// Plays kept per profile: about ten years of heavy daily listening.
const MAX_LISTEN_HISTORY_PER_USER: i64 = 100_000;
const MAX_SEARCH_PROFILE_TRACKS: i64 = 500;
const MAX_SEARCH_AFFINITY_BOOST: f64 = 10.0;
/// Maximum events accepted before ordered retention runs.
const RETENTION_BATCH_SIZE: i32 = 256;
const MAX_EVENT_REFERENCE_CHARACTERS: usize = 160;
const MAX_EVENT_SOURCE_CHARACTERS: usize = 64;
const MAX_EVENT_ZONE_CHARACTERS: usize = 120;
/// Tracks at or below this preference score are kept out of Radio.
const REJECTED_TRACK_SCORE: f64 = -15.0;
const ARTIST_PENALTY_GRACE: f64 = 20.0;
const MAX_ARTIST_PENALTY: f64 = 12.0;
/// Once the library is used up, Radio lets played songs back in, but never
/// one of the last this-many songs (or half the playable library, if that is
/// smaller) so a repeat is always a while away.
const RECYCLE_GAP_TRACKS: usize = 30;
/// Songs played within this many hours come back only after older ones.
const RECYCLE_GAP_HOURS: f64 = 3.0;
/// Played songs considered when ranking repeats by how long ago they played.
const MAX_RECYCLE_HISTORY: i64 = 5_000;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct PlaybackEventRequest {
    pub event_key: String,
    pub song_id: String,
    pub event_type: String,
    pub session_id: Option<String>,
    pub queue_id: Option<String>,
    pub source: Option<String>,
    pub position_seconds: Option<f64>,
    pub duration_seconds: Option<f64>,
    /// The zone that played the track (`client:<device>`, `sonos:...`), so
    /// history can say where something was heard.
    #[serde(default)]
    pub zone_id: Option<String>,
    /// The zone's name as the client showed it; a name set in zone
    /// settings wins.
    #[serde(default)]
    pub zone_name: Option<String>,
    /// The kind of device: "Desktop app", "Chrome on macOS", "Android", ...
    #[serde(default)]
    pub device: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlaybackEventResult {
    pub accepted: bool,
    pub qualified: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct RankedCandidate {
    pub song_id: String,
    pub score: f64,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct SearchAffinity {
    pub song_id: String,
    pub score: f32,
}

#[derive(QueryableByName)]
struct TrackPreferenceRow {
    #[diesel(sql_type = Text)]
    song_id: String,
    #[diesel(sql_type = Double)]
    preference_score: f64,
}

#[derive(QueryableByName)]
struct ArtistPreferenceRow {
    #[diesel(sql_type = Text)]
    artist_id: String,
    #[diesel(sql_type = Double)]
    score: f64,
}

#[derive(QueryableByName)]
struct GenrePreferenceRow {
    #[diesel(sql_type = Text)]
    genre: String,
    #[diesel(sql_type = Double)]
    score: f64,
}

#[derive(QueryableByName)]
struct AlbumPreferenceRow {
    #[diesel(sql_type = Text)]
    album_id: String,
    #[diesel(sql_type = Double)]
    score: f64,
}

#[derive(QueryableByName)]
struct TransitionRow {
    #[diesel(sql_type = Text)]
    song_id: String,
    #[diesel(sql_type = Double)]
    score: f64,
}

#[derive(QueryableByName)]
struct SongIdRow {
    #[diesel(sql_type = Text)]
    song_id: String,
}

#[derive(QueryableByName)]
struct PreviousSongRow {
    #[diesel(sql_type = Text)]
    song_id: String,
}

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

#[derive(QueryableByName)]
struct HistoryCountRow {
    #[diesel(sql_type = Text)]
    song_id: String,
    #[diesel(sql_type = BigInt)]
    count: i64,
}

#[derive(QueryableByName)]
struct RetentionCounterRow {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    playback_events_since_prune: i32,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    history_events_since_prune: i32,
}

fn valid_event_type(value: &str) -> bool {
    matches!(
        value,
        "play_started"
            | "manual_selection"
            | "qualified_play"
            | "completed"
            | "early_skip"
            | "manual_queue_add"
            | "playlist_add"
            | "recommendation_impression"
            | "recommendation_selected"
            | "disliked"
            | "thumbs_up"
            | "skip_not_now"
            | "skip_doesnt_fit"
    )
}

fn event_weight(event_type: &str) -> f64 {
    match event_type {
        "qualified_play" => 3.0,
        "completed" => 4.0,
        "manual_queue_add" => 6.0,
        "playlist_add" => 8.0,
        "recommendation_selected" => 2.0,
        "manual_selection" => 2.0,
        "early_skip" => -5.0,
        "disliked" => -20.0,
        // Radio thumbs up: a deliberate "more like this" from the Radio queue.
        "thumbs_up" => 10.0,
        // Skip reasons. "Not right now" is a mood, not a verdict, so it only
        // nudges the track and keeps it out of the next few stations through
        // the recent-play exclusion. "Doesn't fit" mostly teaches the
        // transition from the previous track that it was a poor follow-up.
        "skip_not_now" => -1.0,
        "skip_doesnt_fit" => -3.0,
        _ => 0.0,
    }
}

/// Events that count as a skip from the previously played track.
fn is_skip(event_type: &str) -> bool {
    matches!(event_type, "early_skip" | "skip_doesnt_fit")
}

fn is_positive(event_type: &str) -> bool {
    matches!(
        event_type,
        "qualified_play" | "completed" | "manual_queue_add" | "playlist_add"
    )
}

pub fn record_playback_event(
    user_id: i32,
    event: &PlaybackEventRequest,
    cache: &LibraryCache,
    pool: &DbPool,
) -> Result<PlaybackEventResult, Box<dyn std::error::Error>> {
    validate_playback_event(event, cache)?;
    let mut connection = pool.get()?;
    connection.transaction::<PlaybackEventResult, Box<dyn std::error::Error>, _>(|connection| {
        persist_playback_event(connection, user_id, event, cache)
    })
}

fn validate_playback_event(
    event: &PlaybackEventRequest,
    cache: &LibraryCache,
) -> Result<(), Box<dyn std::error::Error>> {
    if event.event_key.trim().is_empty() || event.event_key.len() > 160 {
        return Err("event_key must contain between 1 and 160 characters".into());
    }
    if !valid_event_type(&event.event_type) {
        return Err("unsupported playback event type".into());
    }
    if event.song_id.is_empty()
        || event.song_id.chars().count() > MAX_EVENT_REFERENCE_CHARACTERS
        || event.session_id.as_deref().is_some_and(|value| {
            value.is_empty() || value.chars().count() > MAX_EVENT_REFERENCE_CHARACTERS
        })
        || event
            .queue_id
            .as_deref()
            .is_some_and(|value| uuid::Uuid::parse_str(value).is_err())
        || event.source.as_deref().is_some_and(|value| {
            value.trim().is_empty() || value.chars().count() > MAX_EVENT_SOURCE_CHARACTERS
        })
        || event
            .position_seconds
            .is_some_and(|value| !value.is_finite() || value < 0.0)
        || event
            .duration_seconds
            .is_some_and(|value| !value.is_finite() || value < 0.0)
        || [&event.zone_id, &event.zone_name, &event.device]
            .into_iter()
            .flatten()
            .any(|value| {
                value.trim().is_empty() || value.chars().count() > MAX_EVENT_ZONE_CHARACTERS
            })
    {
        return Err("playback event contains an invalid or oversized field".into());
    }
    if cache.song(&event.song_id).is_none() {
        return Err("song is not present in the indexed library".into());
    }

    Ok(())
}

fn persist_playback_event(
    connection: &mut SqliteConnection,
    user_id: i32,
    event: &PlaybackEventRequest,
    cache: &LibraryCache,
) -> Result<PlaybackEventResult, Box<dyn std::error::Error>> {
    let previous = if is_positive(&event.event_type) || is_skip(&event.event_type) {
        event.session_id.as_ref().and_then(|session_id| {
            diesel::sql_query(
                "SELECT song_id FROM playback_event
                 WHERE user_id = ? AND session_id = ?
                   AND event_type IN ('qualified_play', 'completed') AND song_id <> ?
                 ORDER BY created_at DESC, id DESC LIMIT 1",
            )
            .bind::<diesel::sql_types::Integer, _>(user_id)
            .bind::<Text, _>(session_id)
            .bind::<Text, _>(&event.song_id)
            .get_result::<PreviousSongRow>(connection)
            .optional()
            .ok()
            .flatten()
            .map(|row| row.song_id)
        })
    } else {
        None
    };

    let inserted = diesel::sql_query(
        "INSERT OR IGNORE INTO playback_event
         (event_key, user_id, song_id, event_type, session_id, queue_id, source,
          position_seconds, duration_seconds)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Text, _>(event.event_key.trim())
    .bind::<diesel::sql_types::Integer, _>(user_id)
    .bind::<Text, _>(&event.song_id)
    .bind::<Text, _>(&event.event_type)
    .bind::<Nullable<Text>, _>(event.session_id.as_deref())
    .bind::<Nullable<Text>, _>(event.queue_id.as_deref())
    .bind::<Text, _>(event.source.as_deref().unwrap_or("unknown"))
    .bind::<Double, _>(event.position_seconds.unwrap_or(0.0).max(0.0))
    .bind::<Double, _>(event.duration_seconds.unwrap_or(0.0).max(0.0))
    .execute(connection)?;

    if inserted == 0 {
        return Ok(PlaybackEventResult {
            accepted: false,
            qualified: is_positive(&event.event_type),
        });
    }

    if matches!(
        event.event_type.as_str(),
        "play_started" | "manual_selection"
    ) {
        diesel::sql_query("UPDATE user SET now_playing = ? WHERE id = ?")
            .bind::<Text, _>(&event.song_id)
            .bind::<diesel::sql_types::Integer, _>(user_id)
            .execute(connection)?;

        if let Some(queue_id) = event.queue_id.as_deref() {
            diesel::sql_query(
                "UPDATE playback_queue
                 SET current_position = COALESCE((
                       SELECT position FROM playback_queue_item
                       WHERE queue_id = ? AND song_id = ? LIMIT 1
                     ), current_position),
                     revision = revision + 1,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE id = ? AND user_id = ?",
            )
            .bind::<Text, _>(queue_id)
            .bind::<Text, _>(&event.song_id)
            .bind::<Text, _>(queue_id)
            .bind::<diesel::sql_types::Integer, _>(user_id)
            .execute(connection)?;
        }
    }

    if event.event_type == "qualified_play" {
        insert_listen_history(connection, user_id, event, cache)?;
        crate::listen_later::remove_played(connection, user_id, &event.song_id, cache)?;
    }

    let weight = event_weight(&event.event_type);
    if weight != 0.0 {
        let qualified = i32::from(event.event_type == "qualified_play");
        let completed = i32::from(event.event_type == "completed");
        let skipped = i32::from(matches!(
            event.event_type.as_str(),
            "early_skip" | "disliked" | "skip_doesnt_fit"
        ));
        let queued = i32::from(event.event_type == "manual_queue_add");
        let playlist = i32::from(event.event_type == "playlist_add");
        diesel::sql_query(
            "INSERT INTO user_track_preference
             (user_id, song_id, qualified_plays, completions, early_skips,
              manual_queue_adds, playlist_adds, preference_score, last_positive_at, last_played_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?,
                     CASE WHEN ? > 0 THEN CURRENT_TIMESTAMP ELSE NULL END,
                     CASE WHEN ? IN ('manual_selection', 'qualified_play', 'completed', 'early_skip', 'skip_not_now', 'skip_doesnt_fit') THEN CURRENT_TIMESTAMP ELSE NULL END)
             ON CONFLICT(user_id, song_id) DO UPDATE SET
               qualified_plays = qualified_plays + excluded.qualified_plays,
               completions = completions + excluded.completions,
               early_skips = early_skips + excluded.early_skips,
               manual_queue_adds = manual_queue_adds + excluded.manual_queue_adds,
               playlist_adds = playlist_adds + excluded.playlist_adds,
               preference_score = preference_score + excluded.preference_score,
               last_positive_at = CASE WHEN ? > 0 THEN CURRENT_TIMESTAMP ELSE last_positive_at END,
               last_played_at = CASE WHEN ? IN ('manual_selection', 'qualified_play', 'completed', 'early_skip', 'skip_not_now', 'skip_doesnt_fit')
                                     THEN CURRENT_TIMESTAMP ELSE last_played_at END",
        )
        .bind::<diesel::sql_types::Integer, _>(user_id)
        .bind::<Text, _>(&event.song_id)
        .bind::<diesel::sql_types::Integer, _>(qualified)
        .bind::<diesel::sql_types::Integer, _>(completed)
        .bind::<diesel::sql_types::Integer, _>(skipped)
        .bind::<diesel::sql_types::Integer, _>(queued)
        .bind::<diesel::sql_types::Integer, _>(playlist)
        .bind::<Double, _>(weight)
        .bind::<Double, _>(weight)
        .bind::<Text, _>(&event.event_type)
        .bind::<Double, _>(weight)
        .bind::<Text, _>(&event.event_type)
        .execute(connection)?;

        if let Some((artist_id, album_id)) = cache.song_map.get(&event.song_id) {
            let positive = weight.max(0.0);
            let negative = (-weight).max(0.0);
            upsert_entity_preference(
                connection,
                "user_artist_preference",
                "artist_id",
                user_id,
                artist_id,
                positive,
                negative,
            )?;
            upsert_entity_preference(
                connection,
                "user_album_preference",
                "album_id",
                user_id,
                album_id,
                positive,
                negative,
            )?;
            for genre in cache.album_genres.get(album_id).into_iter().flatten() {
                upsert_entity_preference(
                    connection,
                    "user_genre_preference",
                    "genre",
                    user_id,
                    genre,
                    positive,
                    negative,
                )?;
            }
        }
    }

    if let Some(from_song) = previous.filter(|song| song != &event.song_id) {
        let (positive, skipped) = if is_skip(&event.event_type) {
            (0, 1)
        } else {
            (1, 0)
        };
        diesel::sql_query(
            "INSERT INTO track_transition
             (user_id, from_song_id, to_song_id, positive_count, skip_count, last_observed_at)
             VALUES (?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
             ON CONFLICT(user_id, from_song_id, to_song_id) DO UPDATE SET
               positive_count = positive_count + excluded.positive_count,
               skip_count = skip_count + excluded.skip_count,
               last_observed_at = CURRENT_TIMESTAMP",
        )
        .bind::<diesel::sql_types::Integer, _>(user_id)
        .bind::<Text, _>(&from_song)
        .bind::<Text, _>(&event.song_id)
        .bind::<diesel::sql_types::Integer, _>(positive)
        .bind::<diesel::sql_types::Integer, _>(skipped)
        .execute(connection)?;
        diesel::sql_query(
            "DELETE FROM track_transition
             WHERE user_id = ? AND from_song_id = ? AND to_song_id IN (
               SELECT to_song_id FROM track_transition
               WHERE user_id = ? AND from_song_id = ?
               ORDER BY (positive_count * 2 - skip_count) DESC, last_observed_at DESC
               LIMIT -1 OFFSET 50
             )",
        )
        .bind::<diesel::sql_types::Integer, _>(user_id)
        .bind::<Text, _>(&from_song)
        .bind::<diesel::sql_types::Integer, _>(user_id)
        .bind::<Text, _>(&from_song)
        .execute(connection)?;
    }

    schedule_playback_retention(connection, user_id, event.event_type == "qualified_play")?;

    Ok(PlaybackEventResult {
        accepted: true,
        qualified: is_positive(&event.event_type),
    })
}

/// Records one play with a snapshot of what the track was called, so the
/// history still reads right after files are renamed or removed.
fn insert_listen_history(
    connection: &mut SqliteConnection,
    user_id: i32,
    event: &PlaybackEventRequest,
    cache: &LibraryCache,
) -> QueryResult<usize> {
    let song = cache.song(&event.song_id);
    let (artist_id, album_id) = cache
        .song_map
        .get(&event.song_id)
        .map(|(artist, album)| (Some(artist.as_str()), Some(album.as_str())))
        .unwrap_or((None, None));
    let artist_name = artist_id
        .and_then(|id| cache.artist(id))
        .map(|artist| artist.name.as_str())
        .or_else(|| song.map(|song| song.artist.as_str()));
    let album_name = album_id
        .and_then(|id| cache.album(id))
        .map(|album| album.name.as_str());
    let duration = song
        .map(|song| song.duration)
        .filter(|duration| duration.is_finite() && *duration > 0.0)
        .or(event.duration_seconds);
    let clean = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let zone_id = clean(&event.zone_id);
    diesel::sql_query(
        "INSERT INTO listen_history_item
         (user_id, song_id, listened_at, zone_id, zone_name, source, device,
          title, artist_id, artist_name, album_id, album_name, duration_seconds)
         VALUES (?, ?, CURRENT_TIMESTAMP, ?,
                 COALESCE((SELECT NULLIF(TRIM(name), '') FROM zone_setting WHERE zone_id = ?), ?),
                 ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<diesel::sql_types::Integer, _>(user_id)
    .bind::<Text, _>(&event.song_id)
    .bind::<Nullable<Text>, _>(zone_id.as_deref())
    .bind::<Nullable<Text>, _>(zone_id.as_deref())
    .bind::<Nullable<Text>, _>(clean(&event.zone_name))
    .bind::<Nullable<Text>, _>(clean(&event.source))
    .bind::<Nullable<Text>, _>(clean(&event.device))
    .bind::<Nullable<Text>, _>(song.map(|song| song.name.as_str()))
    .bind::<Nullable<Text>, _>(artist_id)
    .bind::<Nullable<Text>, _>(artist_name)
    .bind::<Nullable<Text>, _>(album_id)
    .bind::<Nullable<Text>, _>(album_name)
    .bind::<Nullable<Double>, _>(duration)
    .execute(connection)
}

fn schedule_playback_retention(
    connection: &mut SqliteConnection,
    user_id: i32,
    include_listen_history: bool,
) -> QueryResult<()> {
    let counters = diesel::sql_query(
        "INSERT INTO user_data_retention
           (user_id, playback_events_since_prune, history_events_since_prune)
         VALUES (?, 1, ?)
         ON CONFLICT(user_id) DO UPDATE SET
           playback_events_since_prune = playback_events_since_prune + 1,
           history_events_since_prune = history_events_since_prune + excluded.history_events_since_prune
         RETURNING playback_events_since_prune, history_events_since_prune",
    )
    .bind::<diesel::sql_types::Integer, _>(user_id)
    .bind::<diesel::sql_types::Integer, _>(i32::from(include_listen_history))
    .get_result::<RetentionCounterRow>(connection)?;

    let prune_playback = counters.playback_events_since_prune >= RETENTION_BATCH_SIZE;
    let prune_history = counters.history_events_since_prune >= RETENTION_BATCH_SIZE;
    if prune_playback || prune_history {
        prune_playback_history(connection, user_id, prune_history, prune_playback)?;
        diesel::sql_query(
            "UPDATE user_data_retention SET
               playback_events_since_prune = CASE WHEN ? THEN 0 ELSE playback_events_since_prune END,
               history_events_since_prune = CASE WHEN ? THEN 0 ELSE history_events_since_prune END
             WHERE user_id = ?",
        )
        .bind::<diesel::sql_types::Bool, _>(prune_playback)
        .bind::<diesel::sql_types::Bool, _>(prune_history)
        .bind::<diesel::sql_types::Integer, _>(user_id)
        .execute(connection)?;
    }
    Ok(())
}

pub(crate) fn schedule_listen_history_retention(
    connection: &mut SqliteConnection,
    user_id: i32,
) -> QueryResult<()> {
    let counters = diesel::sql_query(
        "INSERT INTO user_data_retention
           (user_id, playback_events_since_prune, history_events_since_prune)
         VALUES (?, 0, 1)
         ON CONFLICT(user_id) DO UPDATE SET
           history_events_since_prune = history_events_since_prune + 1
         RETURNING playback_events_since_prune, history_events_since_prune",
    )
    .bind::<diesel::sql_types::Integer, _>(user_id)
    .get_result::<RetentionCounterRow>(connection)?;
    if counters.history_events_since_prune >= RETENTION_BATCH_SIZE {
        prune_listen_history(connection, user_id)?;
        diesel::sql_query(
            "UPDATE user_data_retention SET history_events_since_prune = 0 WHERE user_id = ?",
        )
        .bind::<diesel::sql_types::Integer, _>(user_id)
        .execute(connection)?;
    }
    Ok(())
}

fn prune_playback_history(
    connection: &mut SqliteConnection,
    user_id: i32,
    include_listen_history: bool,
    include_playback_events: bool,
) -> QueryResult<()> {
    if include_playback_events {
        diesel::sql_query(
            "DELETE FROM playback_event WHERE user_id = ? AND id IN (
               SELECT id FROM playback_event WHERE user_id = ?
               ORDER BY created_at DESC, id DESC LIMIT -1 OFFSET ?
             )",
        )
        .bind::<diesel::sql_types::Integer, _>(user_id)
        .bind::<diesel::sql_types::Integer, _>(user_id)
        .bind::<BigInt, _>(MAX_PLAYBACK_EVENTS_PER_USER)
        .execute(connection)?;
    }
    if include_listen_history {
        prune_listen_history(connection, user_id)?;
    }
    Ok(())
}

pub(crate) fn prune_listen_history(
    connection: &mut SqliteConnection,
    user_id: i32,
) -> QueryResult<usize> {
    diesel::sql_query(
        "DELETE FROM listen_history_item WHERE user_id = ? AND id IN (
           SELECT id FROM listen_history_item WHERE user_id = ?
           ORDER BY listened_at DESC, id DESC LIMIT -1 OFFSET ?
         )",
    )
    .bind::<diesel::sql_types::Integer, _>(user_id)
    .bind::<diesel::sql_types::Integer, _>(user_id)
    .bind::<BigInt, _>(MAX_LISTEN_HISTORY_PER_USER)
    .execute(connection)
}

fn upsert_entity_preference(
    connection: &mut SqliteConnection,
    table: &str,
    key_column: &str,
    user_id: i32,
    key: &str,
    positive: f64,
    negative: f64,
) -> QueryResult<usize> {
    let statement = format!(
        "INSERT INTO {table} (user_id, {key_column}, positive_weight, negative_weight, last_positive_at)
         VALUES (?, ?, ?, ?, CASE WHEN ? > 0 THEN CURRENT_TIMESTAMP ELSE NULL END)
         ON CONFLICT(user_id, {key_column}) DO UPDATE SET
           positive_weight = positive_weight + excluded.positive_weight,
           negative_weight = negative_weight + excluded.negative_weight,
           last_positive_at = CASE WHEN ? > 0 THEN CURRENT_TIMESTAMP ELSE last_positive_at END"
    );
    diesel::sql_query(statement)
        .bind::<diesel::sql_types::Integer, _>(user_id)
        .bind::<Text, _>(key)
        .bind::<Double, _>(positive)
        .bind::<Double, _>(negative)
        .bind::<Double, _>(positive)
        .bind::<Double, _>(positive)
        .execute(connection)
}

#[derive(Default)]
struct CandidateScore {
    score: f64,
    sources: HashSet<&'static str>,
}

fn add_candidate(
    candidates: &mut HashMap<String, CandidateScore>,
    song_id: &str,
    score: f64,
    source: &'static str,
) {
    let candidate = candidates.entry(song_id.to_owned()).or_default();
    candidate.score += score;
    candidate.sources.insert(source);
}

fn normalized_recording_key(value: &str) -> String {
    const VERSION_MARKERS: &[&str] = &[
        "remaster",
        "remastered",
        "deluxe",
        "expanded",
        "anniversary",
        "version",
    ];
    value
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty() && !VERSION_MARKERS.contains(token))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn search_affinities(
    user_id: i32,
    pool: &DbPool,
) -> Result<Vec<SearchAffinity>, Box<dyn std::error::Error>> {
    let mut connection = pool.get()?;
    let preferences = diesel::sql_query(
        "SELECT song_id,
                preference_score / (1.0 + MAX(0.0, julianday('now') - julianday(last_positive_at)) / 180.0)
                  AS preference_score
         FROM user_track_preference
         WHERE user_id = ? AND preference_score > 0
         ORDER BY preference_score DESC, last_positive_at DESC, song_id ASC LIMIT ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<BigInt, _>(MAX_SEARCH_PROFILE_TRACKS)
    .load::<TrackPreferenceRow>(&mut connection)?;
    let favorites = diesel::sql_query(
        "SELECT song_id FROM favorite_song
         WHERE user_id = ?
         ORDER BY added_at DESC, song_id ASC LIMIT ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<BigInt, _>(MAX_SEARCH_PROFILE_TRACKS)
    .load::<SongIdRow>(&mut connection)?;
    let playlist_tracks = diesel::sql_query(
        "SELECT track.b AS song_id
         FROM _playlist_to_song track
         JOIN _playlist_to_user membership ON membership.a = track.a
         WHERE membership.b = ?
         GROUP BY track.b
         ORDER BY MAX(track.date_added) DESC, track.b ASC LIMIT ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<BigInt, _>(MAX_SEARCH_PROFILE_TRACKS)
    .load::<SongIdRow>(&mut connection)?;
    // Imported catalogs may have listen history without derived preference rows.
    let history = diesel::sql_query(
        "SELECT song_id, COUNT(*) AS count
         FROM listen_history_item
         WHERE user_id = ?
         GROUP BY song_id
         ORDER BY count DESC, MAX(listened_at) DESC, song_id ASC LIMIT ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<BigInt, _>(MAX_SEARCH_PROFILE_TRACKS)
    .load::<HistoryCountRow>(&mut connection)?;

    let mut scores = HashMap::<String, f64>::new();
    let mut preference_ids = HashSet::new();
    for row in preferences {
        preference_ids.insert(row.song_id.clone());
        // One ordinary play is a nudge. Repeated or deliberate actions
        // approach, but never exceed, the five-point profile budget.
        let boost = 5.0 * (1.0 - (-row.preference_score.max(0.0) / 12.0).exp());
        scores.insert(row.song_id, boost);
    }
    for row in favorites {
        *scores.entry(row.song_id).or_default() += 5.0;
    }
    for row in playlist_tracks {
        *scores.entry(row.song_id).or_default() += 3.0;
    }
    for row in history {
        if !preference_ids.contains(&row.song_id) {
            let legacy_boost = ((row.count.max(0) as f64).ln_1p() * 0.8).min(2.0);
            *scores.entry(row.song_id).or_default() += legacy_boost;
        }
    }

    let mut affinities = scores
        .into_iter()
        .filter(|(_, score)| *score > 0.0)
        .map(|(song_id, score)| SearchAffinity {
            song_id,
            score: score.min(MAX_SEARCH_AFFINITY_BOOST) as f32,
        })
        .collect::<Vec<_>>();
    affinities.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.song_id.cmp(&right.song_id))
    });
    Ok(affinities)
}

/// What a Radio station is built around.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RadioSeed {
    Song(String),
    Album(String),
    Artist(String),
    Genre(String),
}

impl RadioSeed {
    pub fn kind(&self) -> &'static str {
        match self {
            RadioSeed::Song(_) => "song",
            RadioSeed::Album(_) => "album",
            RadioSeed::Artist(_) => "artist",
            RadioSeed::Genre(_) => "genre",
        }
    }

    pub fn id(&self) -> &str {
        match self {
            RadioSeed::Song(id)
            | RadioSeed::Album(id)
            | RadioSeed::Artist(id)
            | RadioSeed::Genre(id) => id,
        }
    }

    /// Builds a seed from a stored or requested kind/id pair.
    pub fn from_parts(kind: &str, id: &str) -> Option<Self> {
        let id = id.to_owned();
        match kind {
            "song" => Some(RadioSeed::Song(id)),
            "album" => Some(RadioSeed::Album(id)),
            "artist" => Some(RadioSeed::Artist(id)),
            "genre" => Some(RadioSeed::Genre(id)),
            _ => None,
        }
    }

    /// Display name of the seed in the current catalog, or `None` when the
    /// seed no longer exists.
    pub fn display_name(&self, cache: &LibraryCache) -> Option<String> {
        match self {
            RadioSeed::Song(id) => cache.song(id).map(|song| song.name.clone()),
            RadioSeed::Album(id) => cache.album(id).map(|album| album.name.clone()),
            RadioSeed::Artist(id) => cache.artist(id).map(|artist| artist.name.clone()),
            RadioSeed::Genre(name) => catalog_genre(cache, name).map(str::to_owned),
        }
    }
}

/// Finds the catalog spelling of a genre, ignoring case and surrounding space.
pub fn catalog_genre<'a>(cache: &'a LibraryCache, requested: &str) -> Option<&'a str> {
    let requested = requested.trim();
    if requested.is_empty() {
        return None;
    }
    cache
        .songs_by_genre
        .keys()
        .filter(|genre| genre.trim().eq_ignore_ascii_case(requested))
        .min()
        .map(String::as_str)
}

fn add_flat_candidates(
    candidates: &mut HashMap<String, CandidateScore>,
    cache: &LibraryCache,
    indices: Option<&Vec<crate::library::state::CatalogIndex>>,
    skip_song_id: Option<&str>,
    score: f64,
    source: &'static str,
) {
    for index in indices.into_iter().flatten().take(300) {
        if let Some(song_id) = cache.flat_song_id(*index)
            && Some(song_id) != skip_song_id
        {
            add_candidate(candidates, song_id, score, source);
        }
    }
}

fn add_seed_candidates(
    candidates: &mut HashMap<String, CandidateScore>,
    seed: &RadioSeed,
    cache: &LibraryCache,
) {
    match seed {
        RadioSeed::Song(seed_id) => {
            if let Some((artist_id, album_id)) = cache.song_map.get(seed_id) {
                add_flat_candidates(
                    candidates,
                    cache,
                    cache.songs_by_artist.get(artist_id),
                    Some(seed_id),
                    16.0,
                    "seed_artist",
                );
                for genre in cache.album_genres.get(album_id).into_iter().flatten() {
                    add_flat_candidates(
                        candidates,
                        cache,
                        cache.songs_by_genre.get(genre),
                        Some(seed_id),
                        10.0,
                        "seed_genre",
                    );
                }
            }
        }
        RadioSeed::Album(album_id) => {
            if let Some(album) = cache.album(album_id) {
                for song in album.songs.iter().take(100) {
                    add_candidate(candidates, &song.id, 14.0, "seed_album");
                }
            }
            if let Some(owner) = cache.album_owner(album_id) {
                add_flat_candidates(
                    candidates,
                    cache,
                    cache.songs_by_artist.get(&owner.id),
                    None,
                    12.0,
                    "seed_artist",
                );
            }
            for genre in cache.album_genres.get(album_id).into_iter().flatten() {
                add_flat_candidates(
                    candidates,
                    cache,
                    cache.songs_by_genre.get(genre),
                    None,
                    10.0,
                    "seed_genre",
                );
            }
        }
        RadioSeed::Artist(artist_id) => {
            add_flat_candidates(
                candidates,
                cache,
                cache.songs_by_artist.get(artist_id),
                None,
                20.0,
                "seed_artist",
            );
            let mut genres = HashSet::new();
            if let Some(artist) = cache.artist(artist_id) {
                for album in &artist.albums {
                    for genre in cache.album_genres.get(&album.id).into_iter().flatten() {
                        genres.insert(genre.as_str());
                    }
                }
            }
            let mut genres = genres.into_iter().collect::<Vec<_>>();
            genres.sort_unstable();
            for genre in genres.into_iter().take(8) {
                add_flat_candidates(
                    candidates,
                    cache,
                    cache.songs_by_genre.get(genre),
                    None,
                    10.0,
                    "seed_genre",
                );
            }
        }
        RadioSeed::Genre(requested) => {
            if let Some(genre) = catalog_genre(cache, requested) {
                add_flat_candidates(
                    candidates,
                    cache,
                    cache.songs_by_genre.get(genre),
                    None,
                    18.0,
                    "seed_genre",
                );
            }
        }
    }
}

pub fn recommend(
    user_id: i32,
    seed_song_id: Option<&str>,
    cache: &LibraryCache,
    pool: &DbPool,
    limit: usize,
) -> Result<Vec<RankedCandidate>, Box<dyn std::error::Error>> {
    let seed = seed_song_id.map(|id| RadioSeed::Song(id.to_owned()));
    recommend_radio(user_id, seed.as_ref(), cache, pool, limit, &HashSet::new())
}

/// Ranks Radio picks around `seed` (or the listener's profile without one).
/// Songs in `exclude` are never returned, and neither are songs the listener
/// disliked, keeps skipping or banned (directly, or through its album or
/// artist).
pub fn recommend_radio(
    user_id: i32,
    seed: Option<&RadioSeed>,
    cache: &LibraryCache,
    pool: &DbPool,
    limit: usize,
    exclude: &HashSet<String>,
) -> Result<Vec<RankedCandidate>, Box<dyn std::error::Error>> {
    let queued = exclude.iter().cloned().collect::<Vec<_>>();
    recommend_radio_with(
        user_id,
        seed,
        cache,
        pool,
        limit,
        RadioExclusions {
            queued: &queued,
            reject: &HashSet::new(),
            widen: false,
        },
    )
}

/// Radio that does not dead-end: like [`recommend_radio`], but when the
/// station's own neighbourhood runs dry it widens to nearby genres and then
/// to the whole library (letting recently played songs back in) and, once
/// even that is used up, recycles songs in `exclude` (see
/// [`recommend_radio_endless`]). Bans and dislikes apply at every step.
pub fn recommend_radio_widening(
    user_id: i32,
    seed: Option<&RadioSeed>,
    cache: &LibraryCache,
    pool: &DbPool,
    limit: usize,
    exclude: &HashSet<String>,
) -> Result<Vec<RankedCandidate>, Box<dyn std::error::Error>> {
    let mut queued = exclude.iter().cloned().collect::<Vec<_>>();
    queued.sort();
    recommend_radio_endless(user_id, seed, cache, pool, limit, &queued, &HashSet::new())
}

/// Endless Radio. Like [`recommend_radio_widening`], but `queued` is the
/// listener's queue in play order (most recent last) and only keeps songs out
/// while fresh ones remain: once the library is used up, played songs come
/// back, least recently played first, never one of the last few songs, and
/// still in station order. `reject` (thumbs down this session), the
/// listener's dislikes and bans are never returned. The result is only empty
/// when nothing in the library is playable for Radio.
pub fn recommend_radio_endless(
    user_id: i32,
    seed: Option<&RadioSeed>,
    cache: &LibraryCache,
    pool: &DbPool,
    limit: usize,
    queued: &[String],
    reject: &HashSet<String>,
) -> Result<Vec<RankedCandidate>, Box<dyn std::error::Error>> {
    recommend_radio_with(
        user_id,
        seed,
        cache,
        pool,
        limit,
        RadioExclusions {
            queued,
            reject,
            widen: true,
        },
    )
}

struct RadioExclusions<'a> {
    /// Songs already queued or played, oldest first.
    queued: &'a [String],
    /// Songs never to return.
    reject: &'a HashSet<String>,
    /// Widen past the station, and recycle played songs, instead of
    /// running dry.
    widen: bool,
}

fn recommend_radio_with(
    user_id: i32,
    seed: Option<&RadioSeed>,
    cache: &LibraryCache,
    pool: &DbPool,
    limit: usize,
    exclusions: RadioExclusions<'_>,
) -> Result<Vec<RankedCandidate>, Box<dyn std::error::Error>> {
    let RadioExclusions {
        queued,
        reject,
        widen,
    } = exclusions;
    let mut connection = pool.get()?;
    let bans = RadioBans::load(&mut connection, user_id)?;
    let mut candidates = HashMap::<String, CandidateScore>::new();
    let seed_song_id = match seed {
        Some(RadioSeed::Song(id)) => Some(id.as_str()),
        _ => None,
    };

    if let Some(seed) = seed {
        add_seed_candidates(&mut candidates, seed, cache);
    }

    if let Some(seed_id) = seed_song_id {
        let transitions = diesel::sql_query(
            "SELECT to_song_id AS song_id,
                    (positive_count * 30.0) - (skip_count * 12.0) AS score
             FROM track_transition
             WHERE user_id = ? AND from_song_id = ? AND positive_count > skip_count
             ORDER BY score DESC, to_song_id ASC LIMIT ?",
        )
        .bind::<diesel::sql_types::Integer, _>(user_id)
        .bind::<Text, _>(seed_id)
        .bind::<BigInt, _>(MAX_TRANSITIONS)
        .load::<TransitionRow>(&mut connection)?;
        for row in transitions {
            add_candidate(&mut candidates, &row.song_id, row.score, "transition");
        }
    }

    let track_preferences = diesel::sql_query(
        "SELECT song_id,
                preference_score / (1.0 + MAX(0.0, julianday('now') - julianday(last_positive_at)) / 90.0)
                  AS preference_score
         FROM user_track_preference
         WHERE user_id = ? AND preference_score > 0
         ORDER BY preference_score DESC, last_positive_at DESC, song_id ASC LIMIT ?",
    )
    .bind::<diesel::sql_types::Integer, _>(user_id)
    .bind::<BigInt, _>(MAX_PROFILE_TRACKS)
    .load::<TrackPreferenceRow>(&mut connection)?;
    for (preference_index, row) in track_preferences.into_iter().enumerate() {
        add_candidate(
            &mut candidates,
            &row.song_id,
            row.preference_score.min(25.0),
            "track_affinity",
        );
        if preference_index < 10 {
            let neighbours = diesel::sql_query(
                "SELECT DISTINCT neighbour.b AS song_id
                 FROM _playlist_to_song seed
                 JOIN _playlist_to_song neighbour ON neighbour.a = seed.a AND neighbour.b <> seed.b
                 JOIN _playlist_to_user membership ON membership.a = seed.a
                 WHERE seed.b = ? AND membership.b = ?
                 ORDER BY ABS(COALESCE(neighbour.position, 0) - COALESCE(seed.position, 0)), neighbour.b
                 LIMIT 30",
            )
            .bind::<Text, _>(&row.song_id)
            .bind::<diesel::sql_types::Integer, _>(user_id)
            .load::<SongIdRow>(&mut connection)?;
            for neighbour in neighbours {
                add_candidate(
                    &mut candidates,
                    &neighbour.song_id,
                    20.0,
                    "playlist_cooccurrence",
                );
            }
        }
    }

    let legacy_history = diesel::sql_query(
        "SELECT history.song_id, COUNT(*) AS count
         FROM listen_history_item history
         WHERE history.user_id = ?
           AND NOT EXISTS (
             SELECT 1 FROM user_track_preference preference
             WHERE preference.user_id = history.user_id
               AND preference.song_id = history.song_id
           )
         GROUP BY history.song_id
         ORDER BY count DESC, MAX(history.listened_at) DESC, history.song_id ASC LIMIT ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<BigInt, _>(MAX_PROFILE_TRACKS)
    .load::<HistoryCountRow>(&mut connection)?;
    for history in legacy_history {
        let score = (history.count.max(0) as f64).ln_1p() * 5.0;
        add_candidate(
            &mut candidates,
            &history.song_id,
            score.min(16.0),
            "listen_history",
        );
    }

    let favorites = diesel::sql_query(
        "SELECT song_id FROM favorite_song
         WHERE user_id = ?
         ORDER BY added_at DESC, song_id ASC LIMIT ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<BigInt, _>(MAX_PROFILE_TRACKS)
    .load::<SongIdRow>(&mut connection)?;
    for favorite in favorites {
        add_candidate(&mut candidates, &favorite.song_id, 18.0, "favorite");
    }

    let playlist_tracks = diesel::sql_query(
        "SELECT track.b AS song_id
         FROM _playlist_to_song track
         JOIN _playlist_to_user membership ON membership.a = track.a
         WHERE membership.b = ?
         GROUP BY track.b
         ORDER BY MAX(track.date_added) DESC, track.b ASC LIMIT ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<BigInt, _>(MAX_PROFILE_TRACKS)
    .load::<SongIdRow>(&mut connection)?;
    for track in playlist_tracks {
        add_candidate(&mut candidates, &track.song_id, 14.0, "playlist_membership");
    }

    let artist_preferences = diesel::sql_query(
        "SELECT artist_id,
                (positive_weight - negative_weight * 1.5)
                / 3.0
                / (1.0 + MAX(0.0, julianday('now') - julianday(last_positive_at)) / 90.0) AS score
         FROM user_artist_preference WHERE user_id = ? AND positive_weight > negative_weight
         ORDER BY score DESC, artist_id ASC LIMIT ?",
    )
    .bind::<diesel::sql_types::Integer, _>(user_id)
    .bind::<BigInt, _>(MAX_PROFILE_ARTISTS)
    .load::<ArtistPreferenceRow>(&mut connection)?;
    for artist in artist_preferences {
        if let Some(indices) = cache.songs_by_artist.get(&artist.artist_id) {
            for index in indices.iter().take(300) {
                if let Some(song_id) = cache.flat_song_id(*index) {
                    add_candidate(
                        &mut candidates,
                        song_id,
                        artist.score.clamp(0.0, 10.0),
                        "artist_affinity",
                    );
                }
            }
        }
    }

    let genre_preferences = diesel::sql_query(
        "SELECT genre,
                (positive_weight - negative_weight * 1.5)
                / 4.0
                / (1.0 + MAX(0.0, julianday('now') - julianday(last_positive_at)) / 90.0) AS score
         FROM user_genre_preference WHERE user_id = ? AND positive_weight > negative_weight
         ORDER BY score DESC, genre ASC LIMIT 12",
    )
    .bind::<diesel::sql_types::Integer, _>(user_id)
    .load::<GenrePreferenceRow>(&mut connection)?;
    for genre in genre_preferences {
        if let Some(indices) = cache.songs_by_genre.get(&genre.genre) {
            for index in indices.iter().take(300) {
                if let Some(song_id) = cache.flat_song_id(*index) {
                    add_candidate(
                        &mut candidates,
                        song_id,
                        genre.score.clamp(0.0, 8.0),
                        "genre_affinity",
                    );
                }
            }
        }
    }

    let album_preferences = diesel::sql_query(
        "SELECT album_id,
                (positive_weight - negative_weight * 1.5)
                / 3.0
                / (1.0 + MAX(0.0, julianday('now') - julianday(last_positive_at)) / 90.0) AS score
         FROM user_album_preference WHERE user_id = ? AND positive_weight > negative_weight
         ORDER BY score DESC, album_id ASC LIMIT 20",
    )
    .bind::<diesel::sql_types::Integer, _>(user_id)
    .load::<AlbumPreferenceRow>(&mut connection)?;
    for album in album_preferences {
        if let Some(library_album) = cache.album(&album.album_id) {
            for song in library_album.songs.iter().take(100) {
                add_candidate(
                    &mut candidates,
                    &song.id,
                    album.score.clamp(0.0, 8.0),
                    "album_affinity",
                );
            }
        }
    }

    let recent = diesel::sql_query(
        "SELECT song_id FROM user_track_preference
         WHERE user_id = ? AND last_played_at IS NOT NULL
         ORDER BY last_played_at DESC LIMIT ?",
    )
    .bind::<diesel::sql_types::Integer, _>(user_id)
    .bind::<BigInt, _>(RECENT_EXCLUSION_COUNT)
    .load::<SongIdRow>(&mut connection)?
    .into_iter()
    .map(|row| row.song_id)
    .collect::<Vec<_>>();

    // Disliked tracks (one thumbs down, or a run of early skips) never come
    // back through Radio; explicit plays still work.
    let rejected = diesel::sql_query(
        "SELECT song_id FROM user_track_preference
         WHERE user_id = ? AND preference_score <= ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Double, _>(REJECTED_TRACK_SCORE)
    .load::<SongIdRow>(&mut connection)?
    .into_iter()
    .map(|row| row.song_id)
    .chain(reject.iter().cloned())
    .collect::<HashSet<_>>();
    // Kept out while Radio widens: the caller's queue and rejections, and
    // the listener's dislikes. Only the queue is let back in by recycling.
    let hard_exclusions = queued
        .iter()
        .cloned()
        .chain(rejected.iter().cloned())
        .collect::<HashSet<_>>();
    let recent = recent
        .into_iter()
        .chain(hard_exclusions.iter().cloned())
        .collect::<HashSet<_>>();

    // Artists the listener keeps turning down sink in proportion. One thumbs
    // down is about the track, so the penalty starts after more than that.
    let artist_penalties = diesel::sql_query(
        "SELECT artist_id, negative_weight - positive_weight AS score
         FROM user_artist_preference
         WHERE user_id = ? AND negative_weight - positive_weight > ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Double, _>(ARTIST_PENALTY_GRACE)
    .load::<ArtistPreferenceRow>(&mut connection)?
    .into_iter()
    .map(|row| {
        let penalty = ((row.score - ARTIST_PENALTY_GRACE) / 4.0).min(MAX_ARTIST_PENALTY);
        (row.artist_id, penalty)
    })
    .collect::<HashMap<_, _>>();
    if !artist_penalties.is_empty() {
        for (song_id, candidate) in &mut candidates {
            if let Some(penalty) = cache
                .song_map
                .get(song_id)
                .and_then(|(artist_id, _)| artist_penalties.get(artist_id))
            {
                candidate.score -= penalty;
            }
        }
    }

    let event_count = diesel::sql_query(
        "SELECT
           (SELECT COUNT(*) FROM playback_event
            WHERE user_id = ? AND event_type IN
              ('manual_selection', 'qualified_play', 'completed', 'manual_queue_add', 'playlist_add', 'thumbs_up'))
           + (SELECT COUNT(*) FROM favorite_song WHERE user_id = ?)
           + (SELECT COUNT(*)
              FROM _playlist_to_song track
              JOIN _playlist_to_user membership ON membership.a = track.a
              WHERE membership.b = ?)
           + (SELECT COUNT(*) FROM listen_history_item WHERE user_id = ?) AS count",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Integer, _>(user_id)
    .bind::<Integer, _>(user_id)
    .bind::<Integer, _>(user_id)
    .get_result::<CountRow>(&mut connection)?
    .count;

    if seed.is_none() && event_count < 1 && !widen {
        return Ok(Vec::new());
    }

    // Bans are a hard exclusion that applies on the next pick.
    candidates.retain(|song_id, _| !bans.excludes_song(song_id, cache));
    let station = if widen {
        candidates
            .iter()
            .map(|(song_id, candidate)| (song_id.clone(), candidate.score))
            .collect::<HashMap<_, _>>()
    } else {
        HashMap::new()
    };
    let ranked = rank_candidates(candidates, seed_song_id, recent.clone(), cache);
    let mut selected = select_candidates(ranked, cache, limit);
    if widen && selected.len() < limit {
        widen_selection(
            &mut selected,
            WidenContext {
                seed,
                seed_song_id,
                cache,
                bans: &bans,
                recent: &recent,
                hard_exclusions: &hard_exclusions,
                limit,
            },
        );
    }
    if widen && selected.len() < limit {
        let played = played_history(&mut connection, user_id, queued)?;
        recycle_selection(
            &mut selected,
            RecycleContext {
                cache,
                bans: &bans,
                rejected: &rejected,
                station: &station,
                played: &played,
                limit,
            },
        );
    }
    Ok(selected)
}

/// A song the listener has queued or played, most recent first.
struct PlayedTrack {
    song_id: String,
    /// Hours since it last played, when the server recorded the play.
    hours_ago: Option<f64>,
}

#[derive(QueryableByName)]
struct PlayedRow {
    #[diesel(sql_type = Text)]
    song_id: String,
    #[diesel(sql_type = Double)]
    hours_ago: f64,
}

/// What the listener heard, most recent first: the caller's queue (in
/// reverse play order) and then the server's play history.
fn played_history(
    connection: &mut SqliteConnection,
    user_id: i32,
    queued: &[String],
) -> QueryResult<Vec<PlayedTrack>> {
    let rows = diesel::sql_query(
        "SELECT song_id,
                MAX(0.0, (julianday('now') - julianday(last_played_at)) * 24.0) AS hours_ago
         FROM user_track_preference
         WHERE user_id = ? AND last_played_at IS NOT NULL
         ORDER BY last_played_at DESC, song_id ASC LIMIT ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<BigInt, _>(MAX_RECYCLE_HISTORY)
    .load::<PlayedRow>(connection)?;
    let hours = rows
        .iter()
        .map(|row| (row.song_id.as_str(), row.hours_ago))
        .collect::<HashMap<_, _>>();
    let mut seen = HashSet::new();
    let mut played = Vec::with_capacity(queued.len() + rows.len());
    for song_id in queued
        .iter()
        .rev()
        .map(String::as_str)
        .chain(rows.iter().map(|row| row.song_id.as_str()))
    {
        if seen.insert(song_id) {
            played.push(PlayedTrack {
                song_id: song_id.to_owned(),
                hours_ago: hours.get(song_id).copied(),
            });
        }
    }
    Ok(played)
}

struct RecycleContext<'a> {
    cache: &'a LibraryCache,
    bans: &'a RadioBans,
    /// Dislikes and this session's rejections: never recycled.
    rejected: &'a HashSet<String>,
    /// The station's own affinity per song, used to keep station order.
    station: &'a HashMap<String, f64>,
    played: &'a [PlayedTrack],
    limit: usize,
}

/// Radio's last step before giving up: when every fresh song is used up,
/// played songs come back. Bans and dislikes are applied first; the most
/// recent songs (see [`RECYCLE_GAP_TRACKS`]) stay out, songs heard within
/// [`RECYCLE_GAP_HOURS`] come after older ones, and the rest are ranked by
/// how long ago they played with the station's affinity as a tiebreaker, then
/// spaced by artist and album as usual. Only when that leaves nothing does
/// Radio fall back to the whole playable library (still avoiding the song
/// that just played when there is any other choice).
fn recycle_selection(selected: &mut Vec<RankedCandidate>, context: RecycleContext<'_>) {
    let RecycleContext {
        cache,
        bans,
        rejected,
        station,
        played,
        limit,
    } = context;
    let playable = |song_id: &str| {
        cache.song(song_id).is_some()
            && !rejected.contains(song_id)
            && !bans.excludes_song(song_id, cache)
    };
    let eligible = cache
        .songs_flat
        .iter()
        .filter(|song_id| playable(song_id))
        .collect::<Vec<_>>();
    if eligible.is_empty() {
        return;
    }
    // Recency rank among playable songs: 0 is the one that just played.
    let mut rank = HashMap::<&str, usize>::new();
    let mut hours = HashMap::<&str, f64>::new();
    for track in played {
        if playable(&track.song_id) && !rank.contains_key(track.song_id.as_str()) {
            rank.insert(track.song_id.as_str(), rank.len());
            if let Some(hours_ago) = track.hours_ago {
                hours.insert(track.song_id.as_str(), hours_ago);
            }
        }
    }
    let gap = RECYCLE_GAP_TRACKS.min(eligible.len() / 2);
    let played_count = rank.len() as f64;
    let ranked_pool = |admit: &dyn Fn(&str) -> bool, taken: &HashSet<String>| {
        let mut pool = eligible
            .iter()
            .filter(|song_id| !taken.contains(song_id.as_str()) && admit(song_id))
            .map(|song_id| {
                let (staleness, reason) = match rank.get(song_id.as_str()) {
                    Some(position) => (
                        10.0 * (*position as f64 + 1.0) / (played_count + 1.0),
                        "played a while ago",
                    ),
                    None => (11.0, "from your library"),
                };
                let affinity = station
                    .get(song_id.as_str())
                    .map(|score| (score / 25.0).clamp(0.0, 1.0) * 4.0)
                    .unwrap_or(0.0);
                RankedCandidate {
                    song_id: (*song_id).clone(),
                    score: 10.0 + staleness + affinity,
                    reason: reason.to_owned(),
                }
            })
            .collect::<Vec<_>>();
        pool.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.song_id.cmp(&right.song_id))
        });
        pool
    };
    let outside_gap = |song_id: &str| rank.get(song_id).is_none_or(|position| *position >= gap);
    let outside_hours = |song_id: &str| {
        outside_gap(song_id)
            && hours
                .get(song_id)
                .is_none_or(|hours_ago| *hours_ago >= RECYCLE_GAP_HOURS)
    };
    let tiers: [&dyn Fn(&str) -> bool; 2] = [&outside_hours, &outside_gap];
    for admit in tiers {
        if selected.len() >= limit {
            return;
        }
        let taken = taken_ids(selected);
        let pool = ranked_pool(admit, &taken);
        fill_spaced(selected, pool, cache, limit);
    }
    if !selected.is_empty() {
        return;
    }
    // Nothing outside the gap (a tiny or mostly banned library): anything
    // playable, but not the song that just played unless it is all there is.
    let just_played = |song_id: &str| eligible.len() > 1 && rank.get(song_id) == Some(&0);
    let pool = ranked_pool(&|song_id| !just_played(song_id), &HashSet::new());
    fill_spaced(selected, pool, cache, limit);
}

fn taken_ids(selected: &[RankedCandidate]) -> HashSet<String> {
    selected
        .iter()
        .map(|candidate| candidate.song_id.clone())
        .collect()
}

/// Adds `pool` (already ranked) with the usual artist and album spacing,
/// then without it when a small library leaves no other way to fill up.
fn fill_spaced(
    selected: &mut Vec<RankedCandidate>,
    pool: Vec<RankedCandidate>,
    cache: &LibraryCache,
    limit: usize,
) {
    let wanted = limit.saturating_sub(selected.len());
    if wanted == 0 || pool.is_empty() {
        return;
    }
    let spaced = select_candidates(pool.clone(), cache, wanted);
    let mut taken = spaced
        .iter()
        .map(|candidate| candidate.song_id.clone())
        .collect::<HashSet<_>>();
    selected.extend(spaced);
    for candidate in pool {
        if selected.len() >= limit {
            break;
        }
        if taken.insert(candidate.song_id.clone()) {
            selected.push(candidate);
        }
    }
}

struct WidenContext<'a> {
    seed: Option<&'a RadioSeed>,
    seed_song_id: Option<&'a str>,
    cache: &'a LibraryCache,
    bans: &'a RadioBans,
    recent: &'a HashSet<String>,
    hard_exclusions: &'a HashSet<String>,
    limit: usize,
}

/// Tops `selected` up from nearby genres, then from the whole library.
fn widen_selection(selected: &mut Vec<RankedCandidate>, context: WidenContext<'_>) {
    let WidenContext {
        seed,
        seed_song_id,
        cache,
        bans,
        recent,
        hard_exclusions,
        limit,
    } = context;
    let extend = |candidates: HashMap<String, CandidateScore>,
                  blocked: &HashSet<String>,
                  selected: &mut Vec<RankedCandidate>| {
        let taken = selected
            .iter()
            .map(|candidate| candidate.song_id.clone())
            .collect::<HashSet<_>>();
        let candidates = candidates
            .into_iter()
            .filter(|(song_id, _)| !taken.contains(song_id) && !bans.excludes_song(song_id, cache))
            .collect::<HashMap<_, _>>();
        let ranked = rank_candidates(candidates, seed_song_id, blocked.clone(), cache);
        selected.extend(select_candidates(
            ranked,
            cache,
            limit.saturating_sub(selected.len()),
        ));
    };

    let seed_genres = seed
        .map(|seed| radio_seed_genres(seed, cache))
        .unwrap_or_default();
    let nearby = nearby_genres(&seed_genres, cache);
    if !nearby.is_empty() {
        let mut candidates = HashMap::new();
        for genre in &nearby {
            add_flat_candidates(
                &mut candidates,
                cache,
                cache.songs_by_genre.get(genre),
                seed_song_id,
                10.0,
                "nearby_genre",
            );
        }
        extend(candidates, recent, selected);
    }
    if selected.len() >= limit {
        return;
    }

    // Last resort: anything in the library the listener has not rejected,
    // including songs played recently. A stable per-station jitter keeps the
    // order from being alphabetical.
    let station_key = seed.map(RadioSeed::id).unwrap_or("");
    let mut candidates = HashMap::new();
    for song_id in &cache.songs_flat {
        if Some(song_id.as_str()) == seed_song_id {
            continue;
        }
        let jitter = (stable_hash(station_key, song_id) % 1000) as f64 / 1000.0;
        add_candidate(&mut candidates, song_id, 10.0 + jitter, "library");
    }
    extend(candidates, hard_exclusions, selected);
}

fn stable_hash(station_key: &str, value: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    station_key.hash(&mut hasher);
    value.hash(&mut hasher);
    hasher.finish()
}

/// Genres a station is built on.
fn radio_seed_genres(seed: &RadioSeed, cache: &LibraryCache) -> HashSet<String> {
    let album_genres = |album_id: &str| {
        cache
            .album_genres
            .get(album_id)
            .into_iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>()
    };
    match seed {
        RadioSeed::Song(id) => cache
            .song_map
            .get(id)
            .map(|(_, album_id)| album_genres(album_id))
            .unwrap_or_default()
            .into_iter()
            .collect(),
        RadioSeed::Album(id) => album_genres(id).into_iter().collect(),
        RadioSeed::Artist(id) => cache
            .artist(id)
            .map(|artist| {
                artist
                    .albums
                    .iter()
                    .flat_map(|album| album_genres(&album.id))
                    .collect()
            })
            .unwrap_or_default(),
        RadioSeed::Genre(name) => catalog_genre(cache, name)
            .map(|genre| HashSet::from([genre.to_owned()]))
            .unwrap_or_default(),
    }
}

/// Genres that share artists with `seed_genres`, most shared first.
fn nearby_genres(seed_genres: &HashSet<String>, cache: &LibraryCache) -> Vec<String> {
    if seed_genres.is_empty() {
        return Vec::new();
    }
    let mut shared = HashMap::<&str, usize>::new();
    for artist in cache.artists.iter() {
        let genres = artist
            .albums
            .iter()
            .flat_map(|album| cache.album_genres.get(&album.id).into_iter().flatten())
            .map(String::as_str)
            .collect::<HashSet<_>>();
        if !genres.iter().any(|genre| seed_genres.contains(*genre)) {
            continue;
        }
        for genre in genres {
            if !seed_genres.contains(genre) {
                *shared.entry(genre).or_default() += 1;
            }
        }
    }
    let mut nearby = shared.into_iter().collect::<Vec<_>>();
    nearby.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(right.0)));
    nearby
        .into_iter()
        .take(12)
        .map(|(genre, _)| genre.to_owned())
        .collect()
}

fn rank_candidates(
    candidates: HashMap<String, CandidateScore>,
    seed_song_id: Option<&str>,
    recent: HashSet<String>,
    cache: &LibraryCache,
) -> Vec<RankedCandidate> {
    let seed_key = seed_song_id
        .and_then(|id| cache.song(id))
        .map(|song| normalized_recording_key(&song.name));
    let mut ranked = candidates
        .into_iter()
        .filter(|(song_id, candidate)| {
            candidate.score >= 10.0
                && seed_song_id != Some(song_id.as_str())
                && !recent.contains(song_id)
                && cache.song(song_id).is_some()
        })
        .map(|(song_id, mut candidate)| {
            if candidate.sources.len() > 1 {
                candidate.score += ((candidate.sources.len() - 1) as f64) * 5.0;
            }
            if let Some((_, album_id)) = cache.song_map.get(&song_id)
                && let Some(album) = cache.album(album_id)
            {
                candidate.score *= recommendation_release_weight(&album.primary_type);
            }
            let reason = if candidate.sources.contains("transition") {
                "follows your listening"
            } else if candidate.sources.contains("playlist_cooccurrence") {
                "fits your playlists"
            } else if candidate.sources.contains("seed_album") {
                "from this album"
            } else if candidate.sources.contains("seed_artist") {
                "from this artist"
            } else if candidate.sources.contains("seed_genre") {
                "fits this station"
            } else if candidate.sources.contains("favorite") {
                "from your liked songs"
            } else if candidate.sources.contains("playlist_membership") {
                "from your playlists"
            } else if candidate.sources.contains("artist_affinity") {
                "from an artist you play"
            } else if candidate.sources.contains("nearby_genre") {
                "from a nearby genre"
            } else if candidate.sources.contains("library") {
                "from your library"
            } else {
                "played before"
            };
            RankedCandidate {
                song_id,
                score: candidate.score,
                reason: reason.to_owned(),
            }
        })
        .filter(|candidate| {
            seed_key.as_ref().is_none_or(|seed| {
                cache
                    .song(&candidate.song_id)
                    .map(|song| normalized_recording_key(&song.name) != *seed)
                    .unwrap_or(false)
            })
        })
        .collect::<Vec<_>>();

    ranked.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.song_id.cmp(&right.song_id))
    });

    ranked
}

fn recommendation_release_weight(primary_type: &str) -> f64 {
    match primary_type.trim().to_ascii_lowercase().as_str() {
        "album" => 1.15,
        "remix" => 0.7,
        _ => 1.0,
    }
}

fn select_candidates(
    ranked: Vec<RankedCandidate>,
    cache: &LibraryCache,
    limit: usize,
) -> Vec<RankedCandidate> {
    let mut selected = Vec::with_capacity(limit);
    let mut selected_ids = HashSet::new();
    let mut recent_artists = Vec::<String>::new();
    let mut album_counts = HashMap::<String, usize>::new();
    for candidate in ranked.iter().cloned() {
        let Some((artist_id, album_id)) = cache.song_map.get(&candidate.song_id) else {
            continue;
        };
        if recent_artists
            .iter()
            .rev()
            .take(3)
            .any(|id| id == artist_id)
        {
            continue;
        }
        if album_counts.get(album_id).copied().unwrap_or(0) >= 2 {
            continue;
        }
        recent_artists.push(artist_id.clone());
        *album_counts.entry(album_id.clone()).or_default() += 1;
        selected_ids.insert(candidate.song_id.clone());
        selected.push(candidate);
        if selected.len() >= limit {
            break;
        }
    }
    // Relax spacing deterministically for small libraries: first only
    // avoid the same artist twice in a row, then drop artist spacing.
    for keep_artists_apart in [true, false] {
        if selected.len() >= limit {
            break;
        }
        for candidate in &ranked {
            if selected_ids.contains(&candidate.song_id) {
                continue;
            }
            let Some((artist_id, album_id)) = cache.song_map.get(&candidate.song_id) else {
                continue;
            };
            if album_counts.get(album_id).copied().unwrap_or(0) >= 3 {
                continue;
            }
            if keep_artists_apart && recent_artists.last() == Some(artist_id) {
                continue;
            }
            recent_artists.push(artist_id.clone());
            *album_counts.entry(album_id.clone()).or_default() += 1;
            selected_ids.insert(candidate.song_id.clone());
            selected.push(candidate.clone());
            if selected.len() >= limit {
                break;
            }
        }
    }
    avoid_back_to_back_artists(selected, cache)
}

/// Reorders picks so the same artist does not play twice in a row when
/// another artist is available, otherwise keeping the ranked order.
fn avoid_back_to_back_artists(
    mut remaining: Vec<RankedCandidate>,
    cache: &LibraryCache,
) -> Vec<RankedCandidate> {
    let artist = |candidate: &RankedCandidate| {
        cache
            .song_map
            .get(&candidate.song_id)
            .map(|(artist_id, _)| artist_id.clone())
    };
    let mut ordered = Vec::with_capacity(remaining.len());
    let mut previous: Option<String> = None;
    while !remaining.is_empty() {
        let position = remaining
            .iter()
            .position(|candidate| previous.is_none() || artist(candidate) != previous)
            .unwrap_or(0);
        let candidate = remaining.remove(position);
        previous = artist(&candidate);
        ordered.push(candidate);
    }
    ordered
}

/// A track Radio keeps out because the listener disliked it (a thumbs down,
/// or a run of early skips).
#[derive(Clone, Debug, PartialEq, Serialize, QueryableByName)]
pub struct DislikedTrack {
    #[diesel(sql_type = Text)]
    pub song_id: String,
    /// When the latest thumbs down or skip was recorded, if still in history.
    #[diesel(sql_type = Nullable<Text>)]
    pub disliked_at: Option<String>,
}

/// The listener's disliked tracks, most recent first.
pub fn disliked_tracks(
    connection: &mut SqliteConnection,
    user_id: i32,
    limit: i64,
) -> QueryResult<Vec<DislikedTrack>> {
    diesel::sql_query(
        "SELECT preference.song_id,
                (SELECT CAST(MAX(event.created_at) AS TEXT) FROM playback_event event
                 WHERE event.user_id = preference.user_id
                   AND event.song_id = preference.song_id
                   AND event.event_type IN ('disliked', 'early_skip', 'skip_doesnt_fit')
                ) AS disliked_at
         FROM user_track_preference preference
         WHERE preference.user_id = ? AND preference.preference_score <= ?
         ORDER BY disliked_at IS NULL, disliked_at DESC, preference.song_id ASC
         LIMIT ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Double, _>(REJECTED_TRACK_SCORE)
    .bind::<BigInt, _>(limit)
    .load::<DislikedTrack>(connection)
}

/// Undoes a dislike: the track becomes neutral again (no longer kept out of
/// Radio), and the penalty it put on its artist, album and genres is lifted.
/// Returns whether the track was disliked.
pub fn clear_dislike(
    connection: &mut SqliteConnection,
    user_id: i32,
    song_id: &str,
    cache: &LibraryCache,
) -> QueryResult<bool> {
    connection.immediate_transaction(|connection| {
        let cleared = diesel::sql_query(
            "UPDATE user_track_preference
             SET preference_score = 0, early_skips = 0
             WHERE user_id = ? AND song_id = ? AND preference_score < 0",
        )
        .bind::<Integer, _>(user_id)
        .bind::<Text, _>(song_id)
        .execute(connection)?;
        if cleared == 0 {
            return Ok(false);
        }
        let lift = -event_weight("disliked");
        if let Some((artist_id, album_id)) = cache.song_map.get(song_id) {
            let mut entities = vec![
                ("user_artist_preference", "artist_id", artist_id.clone()),
                ("user_album_preference", "album_id", album_id.clone()),
            ];
            for genre in cache.album_genres.get(album_id).into_iter().flatten() {
                entities.push(("user_genre_preference", "genre", genre.clone()));
            }
            for (table, column, key) in entities {
                diesel::sql_query(format!(
                    "UPDATE {table} SET negative_weight = MAX(0, negative_weight - ?)
                     WHERE user_id = ? AND {column} = ?"
                ))
                .bind::<Double, _>(lift)
                .bind::<Integer, _>(user_id)
                .bind::<Text, _>(&key)
                .execute(connection)?;
            }
        }
        Ok(true)
    })
}

pub fn recent_playback_ids(
    user_id: i32,
    pool: &DbPool,
    limit: i64,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut connection = pool.get()?;
    let rows = diesel::sql_query(
        "SELECT song_id FROM playback_event
         WHERE id IN (
           SELECT MAX(id) FROM playback_event
           WHERE user_id = ? AND event_type IN
             ('manual_selection', 'qualified_play', 'completed')
           GROUP BY song_id
         )
         ORDER BY created_at DESC, id DESC LIMIT ?",
    )
    .bind::<diesel::sql_types::Integer, _>(user_id)
    .bind::<BigInt, _>(limit.clamp(1, 200))
    .load::<SongIdRow>(&mut connection)?;
    Ok(rows.into_iter().map(|row| row.song_id).collect())
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;

    use super::{
        CandidateScore, CountRow, MAX_LISTEN_HISTORY_PER_USER, MAX_PLAYBACK_EVENTS_PER_USER,
        PlaybackEventRequest, normalized_recording_key, prune_playback_history, rank_candidates,
        recommend, record_playback_event, search_affinities,
    };
    use crate::domain::{Album, Artist, Song};
    use crate::library::search::SearchIndex;
    use crate::library::state::LibraryCache;
    use diesel::connection::SimpleConnection;
    use diesel::{Connection, RunQueryDsl};

    fn one_song_cache() -> LibraryCache {
        let song = Song {
            id: "song-1".into(),
            name: "Song".into(),
            ..Song::default()
        };
        let album = Album {
            id: "album-1".into(),
            name: "Album".into(),
            songs: vec![song],
            ..Album::default()
        };
        let artist = Artist {
            id: "artist-1".into(),
            name: "Artist".into(),
            albums: vec![album],
            ..Artist::default()
        };
        let artists = Arc::new(vec![artist]);
        LibraryCache {
            search_index: SearchIndex::build(&artists).expect("test search index"),
            artists,
            song_map: HashMap::from([("song-1".into(), ("artist-1".into(), "album-1".into()))]),
            album_genres: HashMap::new(),
            artist_positions: HashMap::from([("artist-1".into(), 0)]),
            album_positions: HashMap::from([("album-1".into(), (0, 0))]),
            song_positions: HashMap::from([("song-1".into(), (0, 0, 0))]),
            songs_flat: vec!["song-1".into()],
            songs_by_artist: HashMap::new(),
            songs_by_genre: HashMap::new(),
            image_paths: HashSet::new(),
        }
    }

    fn release_ranking_cache() -> LibraryCache {
        let release_types = [
            ("album-song", "album-release", "Album"),
            ("ep-song", "ep-release", "EP"),
            ("remix-song", "remix-release", "Remix"),
            ("strong-remix-song", "strong-remix-release", "Remix"),
        ];
        let albums = release_types
            .iter()
            .map(|(song_id, album_id, release_type)| Album {
                id: (*album_id).into(),
                name: format!("{release_type} Release"),
                primary_type: (*release_type).into(),
                songs: vec![Song {
                    id: (*song_id).into(),
                    name: format!("{release_type} Recording"),
                    ..Song::default()
                }],
                ..Album::default()
            })
            .collect::<Vec<_>>();
        let artist = Artist {
            id: "artist".into(),
            name: "Artist".into(),
            albums,
            ..Artist::default()
        };
        let artists = Arc::new(vec![artist]);
        let song_map = release_types
            .iter()
            .map(|(song_id, album_id, _)| {
                (
                    (*song_id).to_string(),
                    ("artist".to_string(), (*album_id).to_string()),
                )
            })
            .collect::<HashMap<_, _>>();
        let album_positions = release_types
            .iter()
            .enumerate()
            .map(|(index, (_, album_id, _))| ((*album_id).to_string(), (0, index as u32)))
            .collect::<HashMap<_, _>>();
        let song_positions = release_types
            .iter()
            .enumerate()
            .map(|(index, (song_id, _, _))| ((*song_id).to_string(), (0, index as u32, 0)))
            .collect::<HashMap<_, _>>();
        LibraryCache {
            search_index: SearchIndex::build(&artists).expect("test search index"),
            artists,
            song_map,
            album_genres: HashMap::new(),
            artist_positions: HashMap::from([("artist".into(), 0)]),
            album_positions,
            song_positions,
            songs_flat: release_types
                .iter()
                .map(|(song_id, _, _)| (*song_id).to_string())
                .collect(),
            songs_by_artist: HashMap::new(),
            songs_by_genre: HashMap::new(),
            image_paths: HashSet::new(),
        }
    }

    fn candidate(score: f64) -> CandidateScore {
        CandidateScore {
            score,
            sources: HashSet::from(["artist_affinity"]),
        }
    }

    fn recommendation_pool() -> crate::persistence::connection::DbPool {
        use diesel::r2d2::{ConnectionManager, Pool};

        let manager = ConnectionManager::<diesel::sqlite::SqliteConnection>::new(":memory:");
        let pool = Arc::new(Pool::builder().max_size(1).build(manager).unwrap());
        pool.get()
            .expect("recommendation connection")
            .batch_execute(
                "CREATE TABLE track_transition (
                   user_id INTEGER, from_song_id TEXT, to_song_id TEXT,
                   positive_count INTEGER, skip_count INTEGER
                 );
                 CREATE TABLE user_track_preference (
                   user_id INTEGER, song_id TEXT, preference_score REAL,
                   last_positive_at DATETIME, last_played_at DATETIME
                 );
                 CREATE TABLE _playlist_to_song (
                   a TEXT, b TEXT, position INTEGER,
                   date_added DATETIME DEFAULT CURRENT_TIMESTAMP
                 );
                 CREATE TABLE _playlist_to_user (a TEXT, b INTEGER);
                 CREATE TABLE user_artist_preference (
                   user_id INTEGER, artist_id TEXT, positive_weight REAL,
                   negative_weight REAL, last_positive_at DATETIME
                 );
                 CREATE TABLE user_genre_preference (
                   user_id INTEGER, genre TEXT, positive_weight REAL,
                   negative_weight REAL, last_positive_at DATETIME
                 );
                 CREATE TABLE user_album_preference (
                   user_id INTEGER, album_id TEXT, positive_weight REAL,
                   negative_weight REAL, last_positive_at DATETIME
                 );
                 CREATE TABLE favorite_song (
                   user_id INTEGER, song_id TEXT, added_at DATETIME DEFAULT CURRENT_TIMESTAMP
                 );
                 CREATE TABLE listen_history_item (
                   user_id INTEGER, song_id TEXT, listened_at DATETIME DEFAULT CURRENT_TIMESTAMP
                 );
                 CREATE TABLE playback_event (user_id INTEGER, event_type TEXT);
                 CREATE TABLE user_radio_ban (
                   user_id INTEGER, kind TEXT, entity_id TEXT,
                   created_at DATETIME DEFAULT CURRENT_TIMESTAMP
                 );",
            )
            .expect("recommendation schema");
        pool
    }

    fn recommendation_cache() -> LibraryCache {
        let mut cache = release_ranking_cache();
        cache
            .songs_by_artist
            .insert("artist".into(), vec![0, 1, 2, 3]);
        cache
    }

    #[test]
    fn recommendations_handle_fresh_sparse_deleted_and_multi_user_profiles() {
        let pool = recommendation_pool();
        let cache = recommendation_cache();

        assert!(
            recommend(1, None, &cache, &pool, 10)
                .expect("fresh recommendation")
                .is_empty()
        );
        let seeded = recommend(1, Some("album-song"), &cache, &pool, 10)
            .expect("sparse seeded recommendation");
        assert!(!seeded.is_empty());
        assert!(
            seeded
                .iter()
                .all(|candidate| candidate.song_id != "album-song")
        );

        pool.get()
            .expect("profile fixture connection")
            .batch_execute(
                "INSERT INTO playback_event VALUES
                   (1, 'manual_selection'), (2, 'manual_selection');
                 INSERT INTO user_track_preference
                   (user_id, song_id, preference_score, last_positive_at)
                 VALUES
                   (1, 'deleted-song', 40, CURRENT_TIMESTAMP),
                   (1, 'album-song', 30, CURRENT_TIMESTAMP);",
            )
            .expect("profile fixtures");

        let first_user = recommend(1, None, &cache, &pool, 10).expect("first user recommendation");
        assert!(
            first_user
                .iter()
                .any(|candidate| candidate.song_id == "album-song")
        );
        assert!(
            first_user
                .iter()
                .all(|candidate| candidate.song_id != "deleted-song")
        );
        assert!(
            recommend(2, None, &cache, &pool, 10)
                .expect("second user recommendation")
                .is_empty(),
            "one user's affinity must not leak into another user's queue"
        );
    }

    #[test]
    fn search_affinity_uses_likes_and_repeat_listening_as_bounded_nudges() {
        let pool = recommendation_pool();
        pool.get()
            .expect("search affinity fixture connection")
            .batch_execute(
                "INSERT INTO user_track_preference
                   (user_id, song_id, preference_score, last_positive_at)
                 VALUES
                   (1, 'played-once', 3, CURRENT_TIMESTAMP),
                   (1, 'played-often', 36, CURRENT_TIMESTAMP),
                   (1, 'liked-and-played', 36, CURRENT_TIMESTAMP);
                 INSERT INTO favorite_song (user_id, song_id) VALUES
                   (1, 'liked-only'), (1, 'liked-and-played');
                 INSERT INTO _playlist_to_user (a, b) VALUES ('playlist-1', 1);
                 INSERT INTO _playlist_to_song (a, b) VALUES
                   ('playlist-1', 'playlist-only');
                 INSERT INTO listen_history_item (user_id, song_id) VALUES
                   (1, 'legacy-history'), (1, 'legacy-history'),
                   (1, 'legacy-history'), (1, 'legacy-history');",
            )
            .expect("search affinity fixtures");

        let affinities = search_affinities(1, &pool).expect("search affinities");
        let scores = affinities
            .into_iter()
            .map(|affinity| (affinity.song_id, affinity.score))
            .collect::<HashMap<_, _>>();

        assert!(scores["played-once"] > 0.5 && scores["played-once"] < 2.0);
        assert!(scores["played-often"] > scores["played-once"]);
        assert!(scores["played-often"] < 5.0);
        assert_eq!(scores["liked-only"], 5.0);
        assert_eq!(scores["playlist-only"], 3.0);
        assert!(scores["liked-and-played"] > scores["liked-only"]);
        assert!(scores["liked-and-played"] <= 10.0);
        assert!(scores["legacy-history"] > 0.0 && scores["legacy-history"] < 2.0);
        assert!(
            search_affinities(2, &pool)
                .expect("other user's search affinities")
                .is_empty(),
            "search personalization signals must remain scoped to their user"
        );
    }

    #[test]
    fn a_liked_song_can_seed_a_fresh_generated_queue() {
        let pool = recommendation_pool();
        let cache = recommendation_cache();
        pool.get()
            .expect("liked queue fixture connection")
            .batch_execute(
                "INSERT INTO favorite_song (user_id, song_id)
                 VALUES (3, 'album-song');",
            )
            .expect("liked queue fixture");

        let ranked = recommend(3, None, &cache, &pool, 10).expect("liked queue");
        assert_eq!(ranked[0].song_id, "album-song");
        assert_eq!(ranked[0].reason, "from your liked songs");
    }

    #[test]
    fn a_playlist_song_can_seed_a_fresh_generated_queue() {
        let pool = recommendation_pool();
        let cache = recommendation_cache();
        pool.get()
            .expect("playlist queue fixture connection")
            .batch_execute(
                "INSERT INTO _playlist_to_user (a, b) VALUES ('playlist-5', 5);
                 INSERT INTO _playlist_to_song (a, b)
                 VALUES ('playlist-5', 'album-song');",
            )
            .expect("playlist queue fixture");

        let ranked = recommend(5, None, &cache, &pool, 10).expect("playlist queue");
        assert_eq!(ranked[0].song_id, "album-song");
        assert_eq!(ranked[0].reason, "from your playlists");
    }

    #[test]
    fn one_completed_listen_does_not_fan_out_across_the_whole_artist() {
        let pool = recommendation_pool();
        let cache = recommendation_cache();
        pool.get()
            .expect("single listen fixture connection")
            .batch_execute(
                "INSERT INTO playback_event VALUES (4, 'completed');
                 INSERT INTO user_track_preference
                   (user_id, song_id, preference_score, last_positive_at, last_played_at)
                 VALUES (4, 'album-song', 7, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                 INSERT INTO user_artist_preference
                   (user_id, artist_id, positive_weight, negative_weight, last_positive_at)
                 VALUES (4, 'artist', 7, 0, CURRENT_TIMESTAMP);
                 INSERT INTO user_album_preference
                   (user_id, album_id, positive_weight, negative_weight, last_positive_at)
                 VALUES (4, 'album-release', 7, 0, CURRENT_TIMESTAMP);",
            )
            .expect("single listen fixtures");

        assert!(
            recommend(4, None, &cache, &pool, 10)
                .expect("single listen queue")
                .is_empty(),
            "one listen must not fill the queue with broad artist affinity"
        );
    }

    #[test]
    fn legacy_history_needs_repetition_before_it_seeds_a_queue() {
        let pool = recommendation_pool();
        let cache = recommendation_cache();
        pool.get()
            .expect("legacy history fixture connection")
            .batch_execute(
                "INSERT INTO listen_history_item (user_id, song_id) VALUES
                   (6, 'album-song'), (6, 'album-song'), (6, 'album-song'),
                   (6, 'album-song'), (6, 'album-song'), (6, 'album-song'),
                   (6, 'album-song'),
                   (7, 'album-song');",
            )
            .expect("legacy history fixtures");

        let repeated = recommend(6, None, &cache, &pool, 10).expect("repeat history queue");
        assert_eq!(repeated[0].song_id, "album-song");
        assert!(
            recommend(7, None, &cache, &pool, 10)
                .expect("single history queue")
                .is_empty(),
            "one imported listen must remain below the queue threshold"
        );
    }

    #[test]
    fn generated_queue_favours_albums_and_reduces_remix_emphasis() {
        let cache = release_ranking_cache();
        let ranked = rank_candidates(
            HashMap::from([
                ("album-song".into(), candidate(20.0)),
                ("ep-song".into(), candidate(20.0)),
                ("remix-song".into(), candidate(20.0)),
                ("strong-remix-song".into(), candidate(40.0)),
            ]),
            None,
            HashSet::new(),
            &cache,
        );

        assert_eq!(
            ranked
                .iter()
                .map(|candidate| candidate.song_id.as_str())
                .collect::<Vec<_>>(),
            ["strong-remix-song", "album-song", "ep-song", "remix-song"]
        );
        assert_eq!(ranked[1].score, 23.0);
        assert_eq!(ranked[2].score, 20.0);
        assert_eq!(ranked[3].score, 14.0);
    }

    #[test]
    fn normalizes_common_release_version_markers() {
        assert_eq!(
            normalized_recording_key("A Song (2011 Remastered Version)"),
            "a song 2011"
        );
        assert_eq!(normalized_recording_key("A Song - Deluxe"), "a song");
    }

    #[test]
    fn recent_playback_uses_manual_events_and_deduplicates_tracks() {
        use diesel::r2d2::{ConnectionManager, Pool};
        use std::sync::Arc;

        let manager = ConnectionManager::<diesel::sqlite::SqliteConnection>::new(":memory:");
        let pool = Arc::new(Pool::builder().max_size(1).build(manager).unwrap());
        {
            let mut connection = pool.get().unwrap();
            connection
                .batch_execute(
                    "CREATE TABLE playback_event (
                         id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
                         event_key TEXT NOT NULL UNIQUE,
                         user_id INTEGER NOT NULL,
                         song_id TEXT NOT NULL,
                         event_type TEXT NOT NULL,
                         created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
                     );
                     INSERT INTO playback_event
                       (event_key, user_id, song_id, event_type, created_at)
                     VALUES
                       ('a', 1, 'track-a', 'manual_selection', '2026-01-01 10:00:00'),
                       ('b', 1, 'track-b', 'qualified_play', '2026-01-01 10:01:00'),
                       ('c', 1, 'track-a', 'completed', '2026-01-01 10:02:00'),
                       ('d', 1, 'ignored', 'early_skip', '2026-01-01 10:03:00');",
                )
                .unwrap();
        }

        assert_eq!(
            super::recent_playback_ids(1, &pool, 10).unwrap(),
            vec!["track-a".to_string(), "track-b".to_string()]
        );
    }

    #[test]
    fn playback_event_failures_roll_back_the_idempotency_key() {
        use diesel::r2d2::{ConnectionManager, Pool};

        let manager = ConnectionManager::<diesel::sqlite::SqliteConnection>::new(":memory:");
        let pool = Arc::new(
            Pool::builder()
                .max_size(1)
                .build(manager)
                .expect("event pool"),
        );
        {
            let mut connection = pool.get().expect("event connection");
            connection
                .batch_execute(
                    "CREATE TABLE playback_event (
                       id INTEGER PRIMARY KEY AUTOINCREMENT,
                       event_key TEXT NOT NULL UNIQUE, user_id INTEGER NOT NULL,
                       song_id TEXT NOT NULL, event_type TEXT NOT NULL,
                       session_id TEXT, queue_id TEXT, source TEXT NOT NULL DEFAULT 'unknown',
                       position_seconds REAL NOT NULL DEFAULT 0,
                       duration_seconds REAL NOT NULL DEFAULT 0,
                       created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
                     );",
                )
                .expect("event fixture");
        }
        let event = PlaybackEventRequest {
            event_key: "event-1".into(),
            song_id: "song-1".into(),
            event_type: "manual_selection".into(),
            session_id: None,
            queue_id: None,
            source: Some("manual".into()),
            position_seconds: Some(0.0),
            duration_seconds: Some(180.0),
            ..Default::default()
        };
        assert!(record_playback_event(1, &event, &one_song_cache(), &pool).is_err());
        let mut connection = pool.get().expect("event count connection");
        let count = diesel::sql_query("SELECT COUNT(*) AS count FROM playback_event")
            .get_result::<CountRow>(&mut connection)
            .expect("event count")
            .count;
        assert_eq!(count, 0);
    }

    #[test]
    fn playback_start_updates_now_playing_and_queue_position_in_one_event() {
        use diesel::r2d2::{ConnectionManager, Pool};

        #[derive(diesel::QueryableByName)]
        struct PlaybackState {
            #[diesel(sql_type = diesel::sql_types::Text)]
            now_playing: String,
            #[diesel(sql_type = diesel::sql_types::Integer)]
            current_position: i32,
            #[diesel(sql_type = diesel::sql_types::Integer)]
            revision: i32,
        }

        let manager = ConnectionManager::<diesel::sqlite::SqliteConnection>::new(":memory:");
        let pool = Arc::new(
            Pool::builder()
                .max_size(1)
                .build(manager)
                .expect("event pool"),
        );
        let queue_id = "11111111-1111-4111-8111-111111111111";
        {
            let mut connection = pool.get().expect("event connection");
            connection
                .batch_execute(&format!(
                    "CREATE TABLE user (
                       id INTEGER PRIMARY KEY, now_playing TEXT
                     );
                     CREATE TABLE playback_event (
                       id INTEGER PRIMARY KEY AUTOINCREMENT,
                       event_key TEXT NOT NULL UNIQUE, user_id INTEGER NOT NULL,
                       song_id TEXT NOT NULL, event_type TEXT NOT NULL,
                       session_id TEXT, queue_id TEXT, source TEXT NOT NULL DEFAULT 'unknown',
                       position_seconds REAL NOT NULL DEFAULT 0,
                       duration_seconds REAL NOT NULL DEFAULT 0,
                       created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
                     );
                     CREATE TABLE listen_history_item (
                       id INTEGER PRIMARY KEY AUTOINCREMENT, user_id INTEGER NOT NULL,
                       song_id TEXT, listened_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
                     );
                     CREATE TABLE user_data_retention (
                       user_id INTEGER PRIMARY KEY,
                       playback_events_since_prune INTEGER NOT NULL DEFAULT 0,
                       history_events_since_prune INTEGER NOT NULL DEFAULT 0
                     );
                     CREATE TABLE playback_queue (
                       id TEXT PRIMARY KEY, user_id INTEGER NOT NULL,
                       current_position INTEGER NOT NULL DEFAULT 0,
                       revision INTEGER NOT NULL DEFAULT 1,
                       updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
                     );
                     CREATE TABLE playback_queue_item (
                       queue_id TEXT NOT NULL, position INTEGER NOT NULL, song_id TEXT NOT NULL
                     );
                     INSERT INTO user (id) VALUES (1);
                     INSERT INTO playback_queue (id, user_id) VALUES ('{queue_id}', 1);
                     INSERT INTO playback_queue_item (queue_id, position, song_id)
                     VALUES ('{queue_id}', 0, 'other-song'), ('{queue_id}', 1, 'song-1');"
                ))
                .expect("playback state fixture");
        }
        let event = PlaybackEventRequest {
            event_key: "event-start".into(),
            song_id: "song-1".into(),
            event_type: "play_started".into(),
            session_id: Some("session-1".into()),
            queue_id: Some(queue_id.into()),
            source: Some("generated".into()),
            position_seconds: Some(0.0),
            duration_seconds: Some(180.0),
            ..Default::default()
        };
        let result = record_playback_event(1, &event, &one_song_cache(), &pool)
            .expect("playback start event");
        assert!(result.accepted);

        let mut connection = pool.get().expect("state connection");
        let state = diesel::sql_query(
            "SELECT user.now_playing, playback_queue.current_position, playback_queue.revision
             FROM user JOIN playback_queue ON playback_queue.user_id = user.id
             WHERE user.id = 1",
        )
        .get_result::<PlaybackState>(&mut connection)
        .expect("playback state");
        assert_eq!(state.now_playing, "song-1");
        assert_eq!(state.current_position, 1);
        assert_eq!(state.revision, 2);
    }

    #[test]
    fn playback_history_retention_is_exact_and_user_scoped() {
        let mut connection = diesel::sqlite::SqliteConnection::establish(":memory:")
            .expect("history retention connection");
        connection
            .batch_execute(&format!(
                "CREATE TABLE playback_event (
                   id INTEGER PRIMARY KEY AUTOINCREMENT, user_id INTEGER NOT NULL,
                   created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
                 );
                 CREATE TABLE listen_history_item (
                   id INTEGER PRIMARY KEY AUTOINCREMENT, user_id INTEGER NOT NULL,
                   listened_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
                 );
                 WITH RECURSIVE sequence(value) AS (
                   SELECT 1 UNION ALL SELECT value + 1 FROM sequence WHERE value <= {}
                 )
                 INSERT INTO playback_event (user_id) SELECT 1 FROM sequence;
                 WITH RECURSIVE sequence(value) AS (
                   SELECT 1 UNION ALL SELECT value + 1 FROM sequence WHERE value <= {}
                 )
                 INSERT INTO listen_history_item (user_id) SELECT 1 FROM sequence;
                 INSERT INTO playback_event (user_id) VALUES (2);
                 INSERT INTO listen_history_item (user_id) VALUES (2);",
                MAX_PLAYBACK_EVENTS_PER_USER, MAX_LISTEN_HISTORY_PER_USER
            ))
            .expect("history retention fixture");
        prune_playback_history(&mut connection, 1, true, true).expect("history pruning");

        let event_count =
            diesel::sql_query("SELECT COUNT(*) AS count FROM playback_event WHERE user_id = 1")
                .get_result::<CountRow>(&mut connection)
                .expect("event retention count")
                .count;
        let history_count = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM listen_history_item WHERE user_id = 1",
        )
        .get_result::<CountRow>(&mut connection)
        .expect("listen retention count")
        .count;
        assert_eq!(event_count, MAX_PLAYBACK_EVENTS_PER_USER);
        assert_eq!(history_count, MAX_LISTEN_HISTORY_PER_USER);
        let other_user =
            diesel::sql_query("SELECT COUNT(*) AS count FROM playback_event WHERE user_id = 2")
                .get_result::<CountRow>(&mut connection)
                .expect("other user event count")
                .count;
        assert_eq!(other_user, 1);
    }

    use super::test_support::{radio_pool, radio_test_cache};
    use super::{RadioSeed, recommend_radio, valid_event_type};

    fn radio_event(key: &str, song_id: &str, event_type: &str) -> PlaybackEventRequest {
        PlaybackEventRequest {
            event_key: key.into(),
            song_id: song_id.into(),
            event_type: event_type.into(),
            session_id: Some("radio-session".into()),
            queue_id: None,
            source: Some("generated".into()),
            position_seconds: Some(4.0),
            duration_seconds: Some(200.0),
            ..Default::default()
        }
    }

    fn ids(items: &[super::RankedCandidate]) -> Vec<&str> {
        items.iter().map(|item| item.song_id.as_str()).collect()
    }

    #[test]
    fn bans_are_a_hard_exclusion_that_applies_at_once() {
        let pool = radio_pool();
        let cache = radio_test_cache();
        let rock = || {
            super::recommend_radio_widening(
                1,
                Some(&RadioSeed::Genre("Rock".into())),
                &cache,
                &pool,
                20,
                &HashSet::new(),
            )
            .expect("rock radio")
        };
        assert!(ids(&rock()).contains(&"r4"));
        {
            let mut connection = pool.get().unwrap();
            crate::radio_bans::add_ban(
                &mut connection,
                1,
                crate::radio_bans::BanKind::Album,
                "album-rock-2",
            )
            .unwrap();
            crate::radio_bans::add_ban(
                &mut connection,
                1,
                crate::radio_bans::BanKind::Artist,
                "artist-jazz",
            )
            .unwrap();
            crate::radio_bans::add_ban(&mut connection, 1, crate::radio_bans::BanKind::Song, "r1")
                .unwrap();
        }
        // Even widened to the whole library, nothing banned comes back.
        let picks = rock();
        assert_eq!(ids(&picks).len(), 2, "{:?}", ids(&picks));
        assert!(ids(&picks).iter().all(|id| matches!(*id, "r2" | "r3")));

        // Another listener is unaffected, and unbanning works at once.
        let other = super::recommend_radio_widening(
            2,
            Some(&RadioSeed::Genre("Rock".into())),
            &cache,
            &pool,
            20,
            &HashSet::new(),
        )
        .unwrap();
        assert!(ids(&other).contains(&"r4"));
        {
            let mut connection = pool.get().unwrap();
            crate::radio_bans::remove_ban(
                &mut connection,
                1,
                crate::radio_bans::BanKind::Album,
                "album-rock-2",
            )
            .unwrap();
        }
        assert!(ids(&rock()).contains(&"r4"));
    }

    #[test]
    fn widening_radio_reaches_nearby_genres_then_the_whole_library() {
        let pool = radio_pool();
        let mut cache = radio_test_cache();
        // artist-rock-1 also plays Blues, which makes Blues "near" Rock.
        cache
            .album_genres
            .insert("album-rock-1b".into(), vec!["Blues".into()]);
        let r3 = cache.songs_flat.iter().position(|id| id == "r3").unwrap() as u32;
        cache
            .songs_by_genre
            .get_mut("Rock")
            .unwrap()
            .retain(|index| *index != r3);
        cache.songs_by_genre.insert("Blues".into(), vec![r3]);
        let exhausted = ["r1", "r2", "r4", "r5"]
            .into_iter()
            .map(str::to_owned)
            .collect::<HashSet<_>>();
        let seed = RadioSeed::Genre("Rock".into());

        let strict = recommend_radio(1, Some(&seed), &cache, &pool, 5, &exhausted).unwrap();
        assert!(strict.is_empty(), "{:?}", ids(&strict));

        let wide =
            super::recommend_radio_widening(1, Some(&seed), &cache, &pool, 5, &exhausted).unwrap();
        assert_eq!(wide[0].song_id, "r3");
        assert_eq!(wide[0].reason, "from a nearby genre");
        assert_eq!(wide.len(), 4, "{:?}", ids(&wide));
        assert!(
            wide[1..]
                .iter()
                .all(|item| item.song_id.starts_with('j') && item.reason == "from your library")
        );
        assert!(wide.iter().all(|item| !exhausted.contains(&item.song_id)));

        // A listener with no history still gets profile Radio when widening.
        assert!(
            recommend_radio(2, None, &cache, &pool, 5, &HashSet::new())
                .unwrap()
                .is_empty()
        );
        assert!(
            !super::recommend_radio_widening(2, None, &cache, &pool, 5, &HashSet::new())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_single_dislike_can_be_undone() {
        let pool = radio_pool();
        let cache = radio_test_cache();
        record_playback_event(1, &radio_event("down-r2", "r2", "disliked"), &cache, &pool)
            .expect("dislike");
        let mut connection = pool.get().unwrap();
        let disliked = super::disliked_tracks(&mut connection, 1, 50).unwrap();
        assert_eq!(disliked.len(), 1);
        assert_eq!(disliked[0].song_id, "r2");
        assert!(disliked[0].disliked_at.is_some());
        assert!(
            super::disliked_tracks(&mut connection, 2, 50)
                .unwrap()
                .is_empty()
        );

        assert!(super::clear_dislike(&mut connection, 1, "r2", &cache).unwrap());
        assert!(!super::clear_dislike(&mut connection, 1, "r2", &cache).unwrap());
        assert!(
            super::disliked_tracks(&mut connection, 1, 50)
                .unwrap()
                .is_empty()
        );
        drop(connection);
        let rock = recommend_radio(
            1,
            Some(&RadioSeed::Genre("Rock".into())),
            &cache,
            &pool,
            10,
            &HashSet::new(),
        )
        .unwrap();
        assert!(ids(&rock).contains(&"r2"), "{:?}", ids(&rock));
    }

    #[test]
    fn endless_radio_keeps_a_43_track_library_going_for_220_picks() {
        let pool = radio_pool();
        let cache = super::test_support::endless_test_library();
        {
            let mut connection = pool.get().unwrap();
            crate::radio_bans::add_ban(
                &mut connection,
                1,
                crate::radio_bans::BanKind::Artist,
                "artist-f",
            )
            .unwrap();
        }
        record_playback_event(
            1,
            &radio_event("down-a1-2", "a1-2", "disliked"),
            &cache,
            &pool,
        )
        .expect("thumbs down");
        let reject = HashSet::from(["d1-1".to_owned()]);
        let blocked = |song_id: &str| {
            song_id.starts_with("f1-") || song_id == "a1-2" || reject.contains(song_id)
        };
        let eligible = cache.songs_flat.iter().filter(|id| !blocked(id)).count();
        assert_eq!(eligible, 32);
        let gap = super::RECYCLE_GAP_TRACKS.min(eligible / 2);
        let artist = |song_id: &str| cache.song_map[song_id].0.clone();
        let genre = |song_id: &str| cache.album_genres[&cache.song_map[song_id].1][0].clone();

        // Client-style: keep a few picks ahead, send the queue (oldest
        // first) as exclusions and play what was queued.
        let seed = RadioSeed::Genre("Rock".into());
        let mut queue = Vec::<String>::new();
        let mut played = 0;
        let mut request = 0;
        while queue.len() < 220 {
            request += 1;
            let window = &queue[queue.len().saturating_sub(500)..];
            let picks =
                super::recommend_radio_endless(1, Some(&seed), &cache, &pool, 6, window, &reject)
                    .expect("radio picks");
            assert!(
                !picks.is_empty(),
                "request {request} ran dry after {} picks",
                queue.len()
            );
            let recycling = queue.len() >= eligible;
            for (index, pick) in picks.iter().enumerate() {
                assert!(
                    !blocked(&pick.song_id),
                    "{} is banned or disliked",
                    pick.song_id
                );
                // Once recycling, the whole library is in play, so artists
                // can always be kept apart.
                if recycling && index > 0 {
                    assert_ne!(
                        artist(&picks[index - 1].song_id),
                        artist(&pick.song_id),
                        "same artist back to back in request {request}: {:?}",
                        ids(&picks)
                    );
                }
                let recent = &queue[queue.len().saturating_sub(gap)..];
                assert!(
                    !recent.contains(&pick.song_id),
                    "{} repeated within {gap} tracks at pick {}",
                    pick.song_id,
                    queue.len()
                );
                queue.push(pick.song_id.clone());
            }
            while played < queue.len().saturating_sub(2) {
                let song_id = queue[played].clone();
                record_playback_event(
                    1,
                    &radio_event(&format!("play-{played}"), &song_id, "completed"),
                    &cache,
                    &pool,
                )
                .expect("play");
                played += 1;
            }
        }

        // Rock first, as the station asks, and then the whole library.
        assert!(
            queue[..12].iter().all(|id| genre(id) == "Rock"),
            "{:?}",
            &queue[..12]
        );
        let mut plays = HashMap::<&str, usize>::new();
        for song_id in &queue {
            *plays.entry(song_id.as_str()).or_default() += 1;
        }
        assert_eq!(plays.len(), eligible, "every playable track comes round");
        assert!(
            plays.values().all(|count| *count >= 4),
            "least recently played come back first: {plays:?}"
        );
        // Once recycling, picks say why.
        let late =
            super::recommend_radio_endless(1, Some(&seed), &cache, &pool, 3, &queue, &reject)
                .unwrap();
        assert!(late.iter().all(|pick| pick.reason == "played a while ago"));
    }

    #[test]
    fn endless_radio_only_runs_dry_when_nothing_is_playable() {
        let pool = radio_pool();
        let cache = radio_test_cache();
        let seed = RadioSeed::Genre("Rock".into());
        let picks = |queued: &[&str], limit: usize| {
            let queued = queued.iter().map(|id| (*id).to_owned()).collect::<Vec<_>>();
            super::recommend_radio_endless(
                1,
                Some(&seed),
                &cache,
                &pool,
                limit,
                &queued,
                &HashSet::new(),
            )
            .unwrap()
        };
        {
            let mut connection = pool.get().unwrap();
            for (kind, id) in [
                (crate::radio_bans::BanKind::Artist, "artist-rock-2"),
                (crate::radio_bans::BanKind::Artist, "artist-jazz"),
                (crate::radio_bans::BanKind::Song, "r1"),
            ] {
                crate::radio_bans::add_ban(&mut connection, 1, kind, id).unwrap();
            }
        }
        // Two playable tracks, both just queued: the one heard longest ago
        // comes back, never the one that just played.
        assert_eq!(ids(&picks(&["r2", "r3"], 5)), ["r2"]);
        assert_eq!(ids(&picks(&["r3", "r2"], 5)), ["r3"]);

        // One playable track: it plays again rather than stopping.
        record_playback_event(1, &radio_event("down-r2", "r2", "disliked"), &cache, &pool).unwrap();
        assert_eq!(ids(&picks(&["r3"], 5)), ["r3"]);

        // Nothing playable at all: only now is Radio out of songs.
        let mut connection = pool.get().unwrap();
        crate::radio_bans::add_ban(
            &mut connection,
            1,
            crate::radio_bans::BanKind::Album,
            "album-rock-1b",
        )
        .unwrap();
        drop(connection);
        assert!(picks(&["r3"], 5).is_empty());
    }

    #[test]
    fn radio_feedback_events_are_accepted() {
        for event in ["thumbs_up", "disliked", "skip_not_now", "skip_doesnt_fit"] {
            assert!(valid_event_type(event), "{event}");
        }
        assert!(super::event_weight("thumbs_up") > super::event_weight("completed"));
        assert!(super::event_weight("skip_not_now") > super::event_weight("early_skip"));
        assert!(super::event_weight("disliked") <= super::REJECTED_TRACK_SCORE);
        assert!(super::is_skip("skip_doesnt_fit"));
        assert!(!super::is_skip("skip_not_now"));
    }

    #[test]
    fn radio_can_be_seeded_from_a_genre_album_or_artist() {
        let pool = radio_pool();
        let cache = radio_test_cache();

        let jazz = recommend_radio(
            1,
            Some(&RadioSeed::Genre("  jazz ".into())),
            &cache,
            &pool,
            10,
            &HashSet::new(),
        )
        .expect("genre radio");
        assert!(!jazz.is_empty());
        assert!(
            jazz.iter().all(|item| item.song_id.starts_with('j')),
            "{:?}",
            ids(&jazz)
        );

        let album = recommend_radio(
            1,
            Some(&RadioSeed::Album("album-rock-2".into())),
            &cache,
            &pool,
            10,
            &HashSet::new(),
        )
        .expect("album radio");
        assert!(matches!(album[0].song_id.as_str(), "r4" | "r5"));
        assert_eq!(album[0].reason, "from this album");
        assert!(album.iter().all(|item| !item.song_id.starts_with('j')));

        let artist = recommend_radio(
            1,
            Some(&RadioSeed::Artist("artist-rock-1".into())),
            &cache,
            &pool,
            10,
            &HashSet::from(["r1".to_owned()]),
        )
        .expect("artist radio");
        assert!(matches!(artist[0].song_id.as_str(), "r2" | "r3"));
        assert!(artist.iter().all(|item| item.song_id != "r1"));
        assert!(artist.iter().any(|item| item.song_id == "r4"));
    }

    #[test]
    fn thumbs_down_keeps_a_track_out_of_radio_and_thumbs_up_brings_it_in() {
        let pool = radio_pool();
        let cache = radio_test_cache();
        record_playback_event(1, &radio_event("down", "r2", "disliked"), &cache, &pool)
            .expect("dislike");
        record_playback_event(1, &radio_event("up", "j2", "thumbs_up"), &cache, &pool)
            .expect("thumbs up");

        let rock = recommend_radio(
            1,
            Some(&RadioSeed::Genre("Rock".into())),
            &cache,
            &pool,
            10,
            &HashSet::new(),
        )
        .expect("rock radio");
        assert!(!ids(&rock).contains(&"r2"), "{:?}", ids(&rock));
        assert!(ids(&rock).contains(&"r3"));

        // A thumbs up alone is enough history for profile Radio.
        let profile =
            recommend_radio(1, None, &cache, &pool, 10, &HashSet::new()).expect("profile radio");
        assert!(ids(&profile).contains(&"j2"), "{:?}", ids(&profile));
        assert!(!ids(&profile).contains(&"r2"));

        // Repeated thumbs down on one artist sink the rest of their catalogue.
        let score_of = |items: &[super::RankedCandidate], id: &str| {
            items
                .iter()
                .find(|item| item.song_id == id)
                .map(|item| item.score)
                .expect("scored candidate")
        };
        let before = score_of(&rock, "r3");
        record_playback_event(1, &radio_event("down-r1", "r1", "disliked"), &cache, &pool)
            .expect("repeat dislike");
        let rock = recommend_radio(
            1,
            Some(&RadioSeed::Genre("Rock".into())),
            &cache,
            &pool,
            10,
            &HashSet::new(),
        )
        .expect("rock radio after repeated dislikes");
        assert!(score_of(&rock, "r3") < before);
        assert!(score_of(&rock, "r3") < score_of(&rock, "r4"));
        assert!(
            rock.iter()
                .all(|item| !matches!(item.song_id.as_str(), "r1" | "r2"))
        );

        // Other listeners are unaffected by this listener's thumbs down.
        let other = recommend_radio(
            2,
            Some(&RadioSeed::Genre("Rock".into())),
            &cache,
            &pool,
            10,
            &HashSet::new(),
        )
        .expect("other listener radio");
        assert!(ids(&other).contains(&"r2"));
    }

    #[test]
    fn skip_reasons_teach_different_lessons() {
        #[derive(diesel::QueryableByName)]
        struct Transition {
            #[diesel(sql_type = diesel::sql_types::Integer)]
            positive_count: i32,
            #[diesel(sql_type = diesel::sql_types::Integer)]
            skip_count: i32,
        }
        let pool = radio_pool();
        let cache = radio_test_cache();
        record_playback_event(1, &radio_event("q", "r1", "qualified_play"), &cache, &pool)
            .expect("qualified play");
        record_playback_event(
            1,
            &radio_event("fit", "j1", "skip_doesnt_fit"),
            &cache,
            &pool,
        )
        .expect("doesn't fit");
        record_playback_event(1, &radio_event("now", "r3", "skip_not_now"), &cache, &pool)
            .expect("not now");

        let mut connection = pool.get().expect("transition connection");
        let transition = diesel::sql_query(
            "SELECT positive_count, skip_count FROM track_transition
             WHERE user_id = 1 AND from_song_id = 'r1' AND to_song_id = 'j1'",
        )
        .get_result::<Transition>(&mut connection)
        .expect("doesn't-fit transition");
        assert_eq!((transition.positive_count, transition.skip_count), (0, 1));
        let not_now_transitions = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM track_transition WHERE to_song_id = 'r3'",
        )
        .get_result::<CountRow>(&mut connection)
        .expect("not-now transitions")
        .count;
        assert_eq!(not_now_transitions, 0);
        drop(connection);

        // "Not right now" rests the track instead of banning it.
        let rock = recommend_radio(
            1,
            Some(&RadioSeed::Genre("Rock".into())),
            &cache,
            &pool,
            10,
            &HashSet::new(),
        )
        .expect("rock radio");
        assert!(!ids(&rock).contains(&"r3"));
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;

    use diesel::connection::SimpleConnection;

    use crate::domain::{Album, Artist, Song};
    use crate::library::search::SearchIndex;
    use crate::library::state::LibraryCache;
    use crate::persistence::connection::DbPool;

    /// Pool with the real migrated schema and two listeners.
    pub(crate) fn radio_pool() -> DbPool {
        use diesel::r2d2::{ConnectionManager, Pool};

        let manager = ConnectionManager::<diesel::sqlite::SqliteConnection>::new(":memory:");
        let pool = Arc::new(Pool::builder().max_size(1).build(manager).unwrap());
        let mut connection = pool.get().expect("radio schema connection");
        diesel_migrations::MigrationHarness::run_pending_migrations(
            &mut *connection,
            crate::persistence::connection::MIGRATIONS,
        )
        .expect("migrated schema");
        connection
            .batch_execute(
                "INSERT INTO user (id, username, password, bitrate, role)
                 VALUES (1, 'one', 'x', 0, 'user'), (2, 'two', 'x', 0, 'user');",
            )
            .expect("radio listeners");
        drop(connection);
        pool
    }

    /// Two rock artists (three albums) and a jazz artist.
    pub(crate) fn radio_test_cache() -> LibraryCache {
        library_cache(&[
            ("artist-rock-1", "album-rock-1", "Rock", &["r1", "r2"]),
            ("artist-rock-1", "album-rock-1b", "Rock", &["r3"]),
            ("artist-rock-2", "album-rock-2", "Rock", &["r4", "r5"]),
            ("artist-jazz", "album-jazz", "Jazz", &["j1", "j2", "j3"]),
        ])
    }

    /// The 43-track library the endless Radio regression runs on: three rock
    /// artists (20 tracks), two jazz artists (14) and a folk artist (9).
    pub(crate) fn endless_test_library() -> LibraryCache {
        let albums: [(&str, &str, &str, usize); 8] = [
            ("artist-a", "a1", "Rock", 4),
            ("artist-a", "a2", "Rock", 4),
            ("artist-b", "b1", "Rock", 4),
            ("artist-b", "b2", "Rock", 3),
            ("artist-c", "c1", "Rock", 5),
            ("artist-d", "d1", "Jazz", 5),
            ("artist-d", "d2", "Jazz", 4),
            ("artist-e", "e1", "Jazz", 5),
        ];
        let folk: (&str, &str, &str, usize) = ("artist-f", "f1", "Folk", 9);
        let songs = albums
            .iter()
            .chain(std::iter::once(&folk))
            .map(|(_, album, _, count)| {
                (1..=*count)
                    .map(|track| format!("{album}-{track}"))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let song_refs = songs
            .iter()
            .map(|ids| ids.iter().map(String::as_str).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        let layout = albums
            .iter()
            .chain(std::iter::once(&folk))
            .zip(&song_refs)
            .map(|((artist, album, genre, _), ids)| (*artist, *album, *genre, ids.as_slice()))
            .collect::<Vec<_>>();
        let cache = library_cache(&layout);
        assert_eq!(cache.songs_flat.len(), 43);
        cache
    }

    /// A library from (artist, album, genre, songs) rows.
    pub(crate) fn library_cache(layout: &[(&str, &str, &str, &[&str])]) -> LibraryCache {
        let mut artists = Vec::<Artist>::new();
        let mut song_map = HashMap::new();
        let mut album_genres = HashMap::new();
        let mut songs_flat = Vec::new();
        let mut songs_by_artist = HashMap::<String, Vec<u32>>::new();
        let mut songs_by_genre = HashMap::<String, Vec<u32>>::new();
        for &(artist_id, album_id, genre, song_ids) in layout {
            let album = Album {
                id: album_id.into(),
                name: format!("{album_id} name"),
                primary_type: "Album".into(),
                songs: song_ids
                    .iter()
                    .map(|id| Song {
                        id: (*id).into(),
                        name: format!("Song {id}"),
                        artist: artist_id.into(),
                        duration: 200.0,
                        ..Song::default()
                    })
                    .collect(),
                ..Album::default()
            };
            album_genres.insert(album_id.to_owned(), vec![genre.to_owned()]);
            for id in song_ids {
                let index = songs_flat.len() as u32;
                songs_flat.push((*id).to_owned());
                song_map.insert(
                    (*id).to_owned(),
                    (artist_id.to_owned(), album_id.to_owned()),
                );
                songs_by_artist
                    .entry(artist_id.to_owned())
                    .or_default()
                    .push(index);
                songs_by_genre
                    .entry(genre.to_owned())
                    .or_default()
                    .push(index);
            }
            match artists.iter_mut().find(|artist| artist.id == artist_id) {
                Some(artist) => artist.albums.push(album),
                None => artists.push(Artist {
                    id: artist_id.into(),
                    name: format!("{artist_id} name"),
                    albums: vec![album],
                    ..Artist::default()
                }),
            }
        }
        let mut artist_positions = HashMap::new();
        let mut album_positions = HashMap::new();
        let mut song_positions = HashMap::new();
        for (artist_index, artist) in artists.iter().enumerate() {
            artist_positions.insert(artist.id.clone(), artist_index as u32);
            for (album_index, album) in artist.albums.iter().enumerate() {
                album_positions.insert(album.id.clone(), (artist_index as u32, album_index as u32));
                for (song_index, song) in album.songs.iter().enumerate() {
                    song_positions.insert(
                        song.id.clone(),
                        (artist_index as u32, album_index as u32, song_index as u32),
                    );
                }
            }
        }
        let artists = Arc::new(artists);
        LibraryCache {
            search_index: SearchIndex::build(&artists).expect("radio search index"),
            artists,
            song_map,
            album_genres,
            artist_positions,
            album_positions,
            song_positions,
            songs_flat,
            songs_by_artist,
            songs_by_genre,
            image_paths: HashSet::new(),
        }
    }
}
