//! Database access for library tools. Every write here is to Parson's own
//! tables; user files are never opened for writing.

use std::collections::{HashMap, HashSet};

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Integer, Nullable, Text};
use diesel::sqlite::SqliteConnection;

use super::analysis::{Fingerprint, FingerprintKind, Waveform};
use super::skipped::{ImportedFile, SkippedRecord};
use super::versions::VersionChoice;
use crate::domain::AudioProperties;

#[derive(QueryableByName)]
struct PathRow {
    #[diesel(sql_type = Text)]
    path: String,
}

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

/// Library root folders, in the order they were added.
pub fn root_paths(connection: &mut SqliteConnection) -> QueryResult<Vec<String>> {
    Ok(
        diesel::sql_query("SELECT path FROM library_root ORDER BY id")
            .load::<PathRow>(connection)?
            .into_iter()
            .map(|row| row.path)
            .collect(),
    )
}

#[derive(QueryableByName)]
struct ImportedRow {
    #[diesel(sql_type = Text)]
    path: String,
    #[diesel(sql_type = Nullable<Text>)]
    error: Option<String>,
    #[diesel(sql_type = Double)]
    duration_seconds: f64,
}

/// Every available file the catalog holds, for the skipped-files audit.
pub fn imported_files(
    connection: &mut SqliteConnection,
) -> QueryResult<HashMap<String, ImportedFile>> {
    Ok(diesel::sql_query(
        "SELECT fe.path, rfm.error, COALESCE(rfm.duration_seconds, 0) AS duration_seconds
         FROM file_entry fe LEFT JOIN raw_file_metadata rfm ON rfm.file_id = fe.id
         WHERE fe.availability = 'available'",
    )
    .load::<ImportedRow>(connection)?
    .into_iter()
    .map(|row| {
        (
            row.path,
            ImportedFile {
                error: row.error.filter(|error| !error.trim().is_empty()),
                duration_seconds: row.duration_seconds,
            },
        )
    })
    .collect())
}

/// Replaces the findings for `root`, keeping each file's "ignored" choice.
pub fn replace_skipped(
    connection: &mut SqliteConnection,
    root: &str,
    records: &[SkippedRecord],
) -> QueryResult<()> {
    connection.transaction(|connection| {
        let current: HashSet<&str> = records.iter().map(|record| record.path.as_str()).collect();
        let previous = diesel::sql_query("SELECT path FROM library_skipped_file WHERE root_path = ?")
            .bind::<Text, _>(root)
            .load::<PathRow>(connection)?;
        for row in previous {
            if !current.contains(row.path.as_str()) {
                diesel::sql_query("DELETE FROM library_skipped_file WHERE path = ?")
                    .bind::<Text, _>(&row.path)
                    .execute(connection)?;
            }
        }
        for record in records {
            diesel::sql_query(
                "INSERT INTO library_skipped_file
                    (path, root_path, reason, detail, extension, size_bytes, modified_at_ns)
                 VALUES (?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(path) DO UPDATE SET
                    root_path = excluded.root_path, reason = excluded.reason,
                    detail = excluded.detail, extension = excluded.extension,
                    size_bytes = excluded.size_bytes, modified_at_ns = excluded.modified_at_ns,
                    -- A changed file deserves a fresh look.
                    ignored = CASE WHEN library_skipped_file.modified_at_ns = excluded.modified_at_ns
                                    AND library_skipped_file.reason = excluded.reason
                                   THEN library_skipped_file.ignored ELSE 0 END,
                    checked_at = CURRENT_TIMESTAMP",
            )
            .bind::<Text, _>(&record.path)
            .bind::<Text, _>(&record.root_path)
            .bind::<Text, _>(record.reason.as_str())
            .bind::<Text, _>(&record.detail)
            .bind::<Text, _>(&record.extension)
            .bind::<BigInt, _>(record.size_bytes)
            .bind::<BigInt, _>(record.modified_at_ns)
            .execute(connection)?;
        }
        Ok(())
    })
}

/// Removes findings for roots that are no longer in the library.
pub fn prune_skipped_roots(connection: &mut SqliteConnection, roots: &[String]) -> QueryResult<()> {
    let rows = diesel::sql_query("SELECT DISTINCT root_path AS path FROM library_skipped_file")
        .load::<PathRow>(connection)?;
    for row in rows {
        if !roots.contains(&row.path) {
            diesel::sql_query("DELETE FROM library_skipped_file WHERE root_path = ?")
                .bind::<Text, _>(&row.path)
                .execute(connection)?;
        }
    }
    Ok(())
}

#[derive(QueryableByName, Clone, Debug)]
pub struct SkippedRow {
    #[diesel(sql_type = Text)]
    pub path: String,
    #[diesel(sql_type = Text)]
    pub root_path: String,
    #[diesel(sql_type = Text)]
    pub reason: String,
    #[diesel(sql_type = Text)]
    pub detail: String,
    #[diesel(sql_type = Text)]
    pub extension: String,
    #[diesel(sql_type = BigInt)]
    pub size_bytes: i64,
    #[diesel(sql_type = Integer)]
    pub ignored: i32,
    #[diesel(sql_type = Text)]
    pub first_seen_at: String,
    #[diesel(sql_type = Text)]
    pub checked_at: String,
}

pub fn skipped_rows(connection: &mut SqliteConnection) -> QueryResult<Vec<SkippedRow>> {
    diesel::sql_query(
        "SELECT path, root_path, reason, detail, extension, size_bytes, ignored,
                CAST(first_seen_at AS TEXT) AS first_seen_at, CAST(checked_at AS TEXT) AS checked_at
         FROM library_skipped_file ORDER BY path",
    )
    .load::<SkippedRow>(connection)
}

pub fn skipped_count(connection: &mut SqliteConnection) -> QueryResult<i64> {
    Ok(
        diesel::sql_query("SELECT COUNT(*) AS count FROM library_skipped_file WHERE ignored = 0")
            .get_result::<CountRow>(connection)?
            .count,
    )
}

pub fn set_skipped_ignored(
    connection: &mut SqliteConnection,
    paths: &[String],
    ignored: bool,
) -> QueryResult<usize> {
    let mut changed = 0;
    for path in paths {
        changed += diesel::sql_query("UPDATE library_skipped_file SET ignored = ? WHERE path = ?")
            .bind::<Integer, _>(i32::from(ignored))
            .bind::<Text, _>(path)
            .execute(connection)?;
    }
    Ok(changed)
}

/// Last time the skipped-files check ran, if ever.
pub fn last_skipped_check(connection: &mut SqliteConnection) -> QueryResult<Option<String>> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Nullable<Text>)]
        value: Option<String>,
    }
    Ok(diesel::sql_query(
        "SELECT CAST(updated_at AS TEXT) AS value FROM server_setting
         WHERE key = 'library_tools.skipped_checked_at'",
    )
    .load::<Row>(connection)?
    .into_iter()
    .next()
    .and_then(|row| row.value))
}

pub fn record_skipped_check(connection: &mut SqliteConnection) -> QueryResult<()> {
    diesel::sql_query(
        "INSERT INTO server_setting (key, value_json) VALUES ('library_tools.skipped_checked_at', 'true')
         ON CONFLICT(key) DO UPDATE SET updated_at = CURRENT_TIMESTAMP",
    )
    .execute(connection)?;
    Ok(())
}

/// Makes the next scan read these files again from disk (tags, stream
/// headers and folder artwork), without changing the files.
pub fn mark_files_for_reread(
    connection: &mut SqliteConnection,
    paths: &[String],
) -> QueryResult<usize> {
    let mut changed = 0;
    for chunk in paths.chunks(400) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let sql = format!(
            "UPDATE raw_file_metadata SET parser_version = NULL, cover_resolver_version = NULL,
                    classification_version = NULL
             WHERE file_id IN (SELECT id FROM file_entry WHERE path IN ({placeholders}))"
        );
        let mut query = diesel::sql_query(sql).into_boxed();
        for path in chunk {
            query = query.bind::<Text, _>(path.clone());
        }
        changed += query.execute(connection)?;
    }
    Ok(changed)
}

/// Makes the next scan read every file again.
pub fn mark_library_for_reread(connection: &mut SqliteConnection) -> QueryResult<usize> {
    diesel::sql_query(
        "UPDATE raw_file_metadata SET parser_version = NULL, cover_resolver_version = NULL,
                classification_version = NULL",
    )
    .execute(connection)
}

// ---------------------------------------------------------------------------
// Visibility, copies and preferences
// ---------------------------------------------------------------------------

#[derive(QueryableByName, Clone, Debug)]
pub struct VisibilityRow {
    #[diesel(sql_type = Text)]
    pub path: String,
    #[diesel(sql_type = Text)]
    pub state: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub kept_path: Option<String>,
    #[diesel(sql_type = Text)]
    pub song_id: String,
    #[diesel(sql_type = Text)]
    pub updated_at: String,
}

pub fn visibility_rows(connection: &mut SqliteConnection) -> QueryResult<Vec<VisibilityRow>> {
    diesel::sql_query(
        "SELECT path, state, kept_path, song_id, CAST(updated_at AS TEXT) AS updated_at
         FROM library_file_visibility ORDER BY updated_at DESC",
    )
    .load::<VisibilityRow>(connection)
}

pub fn set_visibility(
    connection: &mut SqliteConnection,
    path: &str,
    state: &str,
    kept_path: Option<&str>,
    song_id: &str,
) -> QueryResult<()> {
    diesel::sql_query(
        "INSERT INTO library_file_visibility (path, state, kept_path, song_id) VALUES (?, ?, ?, ?)
         ON CONFLICT(path) DO UPDATE SET state = excluded.state, kept_path = excluded.kept_path,
             song_id = excluded.song_id, updated_at = CURRENT_TIMESTAMP",
    )
    .bind::<Text, _>(path)
    .bind::<Text, _>(state)
    .bind::<Nullable<Text>, _>(kept_path)
    .bind::<Text, _>(song_id)
    .execute(connection)?;
    Ok(())
}

pub fn clear_visibility(connection: &mut SqliteConnection, path: &str) -> QueryResult<usize> {
    diesel::sql_query("DELETE FROM library_file_visibility WHERE path = ?")
        .bind::<Text, _>(path)
        .execute(connection)
}

/// Moves likes and playlist entries from a merged copy to the kept copy.
/// The merged copy's own rows stay, so they return if it is restored.
pub fn carry_references(
    connection: &mut SqliteConnection,
    from_song: &str,
    to_song: &str,
) -> QueryResult<()> {
    if from_song == to_song || from_song.is_empty() || to_song.is_empty() {
        return Ok(());
    }
    diesel::sql_query("INSERT OR IGNORE INTO song (id) VALUES (?)")
        .bind::<Text, _>(to_song)
        .execute(connection)?;
    diesel::sql_query(
        "INSERT OR IGNORE INTO favorite_song (user_id, song_id, added_at)
         SELECT user_id, ?, added_at FROM favorite_song WHERE song_id = ?",
    )
    .bind::<Text, _>(to_song)
    .bind::<Text, _>(from_song)
    .execute(connection)?;
    diesel::sql_query(
        "INSERT INTO _playlist_to_song (a, b, date_added, added_by, position)
         SELECT a, ?, date_added, added_by, position FROM _playlist_to_song source
         WHERE b = ? AND NOT EXISTS (
             SELECT 1 FROM _playlist_to_song existing WHERE existing.a = source.a AND existing.b = ?)",
    )
    .bind::<Text, _>(to_song)
    .bind::<Text, _>(from_song)
    .bind::<Text, _>(to_song)
    .execute(connection)?;
    diesel::sql_query(
        "INSERT INTO playlist_track (playlist_id, track_id, date_added, added_by, position)
         SELECT playlist_id, ?, date_added, added_by, position FROM playlist_track
         WHERE track_id = ? AND EXISTS (SELECT 1 FROM track_entity WHERE id = ?)
         ON CONFLICT(playlist_id, track_id) DO NOTHING",
    )
    .bind::<Text, _>(to_song)
    .bind::<Text, _>(from_song)
    .bind::<Text, _>(to_song)
    .execute(connection)?;
    Ok(())
}

pub fn dismissed_duplicates(connection: &mut SqliteConnection) -> QueryResult<HashSet<String>> {
    Ok(
        diesel::sql_query("SELECT group_key AS path FROM library_duplicate_dismissal")
            .load::<PathRow>(connection)?
            .into_iter()
            .map(|row| row.path)
            .collect(),
    )
}

pub fn set_duplicate_dismissed(
    connection: &mut SqliteConnection,
    key: &str,
    dismissed: bool,
) -> QueryResult<()> {
    if dismissed {
        diesel::sql_query(
            "INSERT OR IGNORE INTO library_duplicate_dismissal (group_key) VALUES (?)",
        )
        .bind::<Text, _>(key)
        .execute(connection)?;
    } else {
        diesel::sql_query("DELETE FROM library_duplicate_dismissal WHERE group_key = ?")
            .bind::<Text, _>(key)
            .execute(connection)?;
    }
    Ok(())
}

/// One stored file of a track that has several.
#[derive(QueryableByName, Clone, Debug)]
pub struct CopyRow {
    #[diesel(sql_type = Text)]
    pub track_id: String,
    #[diesel(sql_type = Text)]
    pub path: String,
    #[diesel(sql_type = Text)]
    pub extension: String,
    #[diesel(sql_type = BigInt)]
    pub size_bytes: i64,
    #[diesel(sql_type = Integer)]
    pub is_primary: i32,
    #[diesel(sql_type = Nullable<Text>)]
    pub codec: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub container: Option<String>,
    #[diesel(sql_type = Nullable<Integer>)]
    pub sample_rate: Option<i32>,
    #[diesel(sql_type = Nullable<Integer>)]
    pub bit_depth: Option<i32>,
    #[diesel(sql_type = Nullable<Integer>)]
    pub channels: Option<i32>,
    #[diesel(sql_type = Nullable<Integer>)]
    pub bitrate: Option<i32>,
    #[diesel(sql_type = Double)]
    pub duration_seconds: f64,
}

impl CopyRow {
    pub fn audio(&self) -> Option<AudioProperties> {
        let positive = |value: Option<i32>| value.filter(|value| *value > 0);
        let audio = AudioProperties {
            codec: self.codec.clone().filter(|codec| !codec.is_empty()),
            container: self.container.clone().filter(|value| !value.is_empty()),
            sample_rate: positive(self.sample_rate).map(|value| value as u32),
            bit_depth: positive(self.bit_depth).and_then(|value| u8::try_from(value).ok()),
            channels: positive(self.channels).and_then(|value| u8::try_from(value).ok()),
            bitrate_kbps: positive(self.bitrate).map(|value| value as u32),
            lossless: self
                .codec
                .as_deref()
                .and_then(crate::domain::codec_is_lossless),
            enhanced_by: None,
        };
        (!audio.is_empty()).then_some(audio)
    }
}

const COPY_COLUMNS: &str = "tf.track_id, fe.path, fe.extension, fe.size_bytes,
    CAST(tf.is_primary AS INTEGER) AS is_primary, rfm.codec, rfm.container, rfm.sample_rate,
    rfm.bit_depth, rfm.channels, rfm.bitrate, COALESCE(rfm.duration_seconds, 0) AS duration_seconds";

/// Files of every track that has more than one available copy.
pub fn multi_copy_tracks(connection: &mut SqliteConnection) -> QueryResult<Vec<CopyRow>> {
    diesel::sql_query(format!(
        "SELECT {COPY_COLUMNS}
         FROM track_file tf
         JOIN file_entry fe ON fe.id = tf.file_id AND fe.availability = 'available'
         LEFT JOIN raw_file_metadata rfm ON rfm.file_id = fe.id
         WHERE tf.track_id IN (
             SELECT tf2.track_id FROM track_file tf2
             JOIN file_entry fe2 ON fe2.id = tf2.file_id AND fe2.availability = 'available'
             GROUP BY tf2.track_id HAVING COUNT(*) > 1)
         ORDER BY tf.track_id, tf.quality_rank DESC"
    ))
    .load::<CopyRow>(connection)
}

/// Files of one track.
pub fn track_copies(
    connection: &mut SqliteConnection,
    track_id: &str,
) -> QueryResult<Vec<CopyRow>> {
    diesel::sql_query(format!(
        "SELECT {COPY_COLUMNS}
         FROM track_file tf
         JOIN file_entry fe ON fe.id = tf.file_id AND fe.availability = 'available'
         LEFT JOIN raw_file_metadata rfm ON rfm.file_id = fe.id
         WHERE tf.track_id = ?
         ORDER BY tf.quality_rank DESC"
    ))
    .bind::<Text, _>(track_id)
    .load::<CopyRow>(connection)
}

#[derive(QueryableByName, Clone, Debug)]
pub struct PreferenceRow {
    #[diesel(sql_type = Text)]
    pub track_id: String,
    #[diesel(sql_type = Text)]
    pub path: String,
}

pub fn file_preferences(connection: &mut SqliteConnection) -> QueryResult<Vec<PreferenceRow>> {
    diesel::sql_query("SELECT track_id, path FROM library_file_preference")
        .load::<PreferenceRow>(connection)
}

pub fn set_file_preference(
    connection: &mut SqliteConnection,
    track_id: &str,
    path: Option<&str>,
) -> QueryResult<()> {
    match path {
        Some(path) => diesel::sql_query(
            "INSERT INTO library_file_preference (track_id, path) VALUES (?, ?)
             ON CONFLICT(track_id) DO UPDATE SET path = excluded.path, updated_at = CURRENT_TIMESTAMP",
        )
        .bind::<Text, _>(track_id)
        .bind::<Text, _>(path)
        .execute(connection)?,
        None => diesel::sql_query("DELETE FROM library_file_preference WHERE track_id = ?")
            .bind::<Text, _>(track_id)
            .execute(connection)?,
    };
    Ok(())
}

// ---------------------------------------------------------------------------
// Album versions
// ---------------------------------------------------------------------------

pub fn version_members(
    connection: &mut SqliteConnection,
) -> QueryResult<HashMap<String, Option<String>>> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        album_id: String,
        #[diesel(sql_type = Nullable<Text>)]
        group_key: Option<String>,
    }
    Ok(
        diesel::sql_query("SELECT album_id, group_key FROM library_album_version_member")
            .load::<Row>(connection)?
            .into_iter()
            .map(|row| (row.album_id, row.group_key))
            .collect(),
    )
}

pub fn set_version_member(
    connection: &mut SqliteConnection,
    album_id: &str,
    group_key: Option<&str>,
) -> QueryResult<()> {
    diesel::sql_query(
        "INSERT INTO library_album_version_member (album_id, group_key) VALUES (?, ?)
         ON CONFLICT(album_id) DO UPDATE SET group_key = excluded.group_key, updated_at = CURRENT_TIMESTAMP",
    )
    .bind::<Text, _>(album_id)
    .bind::<Nullable<Text>, _>(group_key)
    .execute(connection)?;
    Ok(())
}

pub fn clear_version_member(connection: &mut SqliteConnection, album_id: &str) -> QueryResult<()> {
    diesel::sql_query("DELETE FROM library_album_version_member WHERE album_id = ?")
        .bind::<Text, _>(album_id)
        .execute(connection)?;
    Ok(())
}

pub fn version_choices(
    connection: &mut SqliteConnection,
) -> QueryResult<HashMap<String, VersionChoice>> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        group_key: String,
        #[diesel(sql_type = Text)]
        preferred_album_id: String,
        #[diesel(sql_type = Integer)]
        hide_others: i32,
    }
    Ok(diesel::sql_query(
        "SELECT group_key, preferred_album_id, hide_others FROM library_album_version_choice",
    )
    .load::<Row>(connection)?
    .into_iter()
    .map(|row| {
        (
            row.group_key,
            VersionChoice {
                preferred_album_id: row.preferred_album_id,
                hide_others: row.hide_others != 0,
            },
        )
    })
    .collect())
}

pub fn set_version_choice(
    connection: &mut SqliteConnection,
    group_key: &str,
    choice: Option<&VersionChoice>,
) -> QueryResult<()> {
    match choice {
        Some(choice) => diesel::sql_query(
            "INSERT INTO library_album_version_choice (group_key, preferred_album_id, hide_others)
             VALUES (?, ?, ?)
             ON CONFLICT(group_key) DO UPDATE SET preferred_album_id = excluded.preferred_album_id,
                 hide_others = excluded.hide_others, updated_at = CURRENT_TIMESTAMP",
        )
        .bind::<Text, _>(group_key)
        .bind::<Text, _>(&choice.preferred_album_id)
        .bind::<Integer, _>(i32::from(choice.hide_others))
        .execute(connection)?,
        None => diesel::sql_query("DELETE FROM library_album_version_choice WHERE group_key = ?")
            .bind::<Text, _>(group_key)
            .execute(connection)?,
    };
    Ok(())
}

/// MusicBrainz release group per album, where known.
pub fn release_groups(connection: &mut SqliteConnection) -> QueryResult<HashMap<String, String>> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        id: String,
        #[diesel(sql_type = Text)]
        release_group_id: String,
    }
    Ok(diesel::sql_query(
        "SELECT id, release_group_id FROM album_entity
         WHERE release_group_id IS NOT NULL AND release_group_id != ''",
    )
    .load::<Row>(connection)?
    .into_iter()
    .map(|row| (row.id, row.release_group_id))
    .collect())
}

// ---------------------------------------------------------------------------
// Albumizer
// ---------------------------------------------------------------------------

#[derive(QueryableByName, Clone, Debug, PartialEq, Eq)]
pub struct UserAlbumRow {
    #[diesel(sql_type = Text)]
    pub id: String,
    #[diesel(sql_type = Text)]
    pub title: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub artist_id: Option<String>,
    #[diesel(sql_type = Text)]
    pub artist_name: String,
    #[diesel(sql_type = Text)]
    pub year: String,
    #[diesel(sql_type = Text)]
    pub primary_type: String,
    #[diesel(sql_type = Text)]
    pub cover_path: String,
}

#[derive(QueryableByName, Clone, Debug, PartialEq, Eq)]
pub struct UserAlbumTrackRow {
    #[diesel(sql_type = Text)]
    pub path: String,
    #[diesel(sql_type = Text)]
    pub album_id: String,
    #[diesel(sql_type = Integer)]
    pub position: i32,
    #[diesel(sql_type = Integer)]
    pub disc: i32,
}

pub fn user_albums(
    connection: &mut SqliteConnection,
) -> QueryResult<(Vec<UserAlbumRow>, Vec<UserAlbumTrackRow>)> {
    let albums = diesel::sql_query(
        "SELECT id, title, artist_id, artist_name, year, primary_type, cover_path
         FROM library_user_album ORDER BY created_at, id",
    )
    .load::<UserAlbumRow>(connection)?;
    let tracks = diesel::sql_query(
        "SELECT path, album_id, position, disc FROM library_user_album_track
         ORDER BY album_id, disc, position",
    )
    .load::<UserAlbumTrackRow>(connection)?;
    Ok((albums, tracks))
}

/// Creates or replaces a user album and its track list. A track can be in
/// one user album at a time; adding it here takes it out of any other.
pub fn save_user_album(
    connection: &mut SqliteConnection,
    album: &UserAlbumRow,
    tracks: &[(String, i32)],
) -> QueryResult<()> {
    connection.transaction(|connection| {
        diesel::sql_query(
            "INSERT INTO library_user_album (id, title, artist_id, artist_name, year, primary_type, cover_path)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET title = excluded.title, artist_id = excluded.artist_id,
                 artist_name = excluded.artist_name, year = excluded.year,
                 primary_type = excluded.primary_type, cover_path = excluded.cover_path,
                 updated_at = CURRENT_TIMESTAMP",
        )
        .bind::<Text, _>(&album.id)
        .bind::<Text, _>(&album.title)
        .bind::<Nullable<Text>, _>(album.artist_id.as_deref())
        .bind::<Text, _>(&album.artist_name)
        .bind::<Text, _>(&album.year)
        .bind::<Text, _>(&album.primary_type)
        .bind::<Text, _>(&album.cover_path)
        .execute(connection)?;
        diesel::sql_query("DELETE FROM library_user_album_track WHERE album_id = ?")
            .bind::<Text, _>(&album.id)
            .execute(connection)?;
        for (index, (path, disc)) in tracks.iter().enumerate() {
            diesel::sql_query(
                "INSERT INTO library_user_album_track (path, album_id, position, disc) VALUES (?, ?, ?, ?)
                 ON CONFLICT(path) DO UPDATE SET album_id = excluded.album_id,
                     position = excluded.position, disc = excluded.disc",
            )
            .bind::<Text, _>(path)
            .bind::<Text, _>(&album.id)
            .bind::<Integer, _>(index as i32 + 1)
            .bind::<Integer, _>((*disc).max(1))
            .execute(connection)?;
        }
        // Albums emptied by moving their last track go away.
        diesel::sql_query(
            "DELETE FROM library_user_album WHERE id != ? AND NOT EXISTS (
                 SELECT 1 FROM library_user_album_track t WHERE t.album_id = library_user_album.id)",
        )
        .bind::<Text, _>(&album.id)
        .execute(connection)?;
        Ok(())
    })
}

pub fn delete_user_album(connection: &mut SqliteConnection, id: &str) -> QueryResult<usize> {
    connection.transaction(|connection| {
        diesel::sql_query("DELETE FROM library_user_album_track WHERE album_id = ?")
            .bind::<Text, _>(id)
            .execute(connection)?;
        diesel::sql_query("DELETE FROM library_user_album WHERE id = ?")
            .bind::<Text, _>(id)
            .execute(connection)
    })
}

// ---------------------------------------------------------------------------
// Analysis
// ---------------------------------------------------------------------------

#[derive(QueryableByName, Clone, Debug)]
pub struct AnalysisRow {
    #[diesel(sql_type = Text)]
    pub path: String,
    #[diesel(sql_type = Text)]
    pub file_version: String,
    #[diesel(sql_type = Text)]
    pub waveform: String,
    #[diesel(sql_type = Text)]
    pub fingerprint: String,
    #[diesel(sql_type = Text)]
    pub fingerprint_kind: String,
    #[diesel(sql_type = Double)]
    pub duration_seconds: f64,
    #[diesel(sql_type = Nullable<Text>)]
    pub error: Option<String>,
    #[diesel(sql_type = Text)]
    pub analyzed_at: String,
}

impl AnalysisRow {
    pub fn waveform(&self) -> Option<Waveform> {
        Waveform::decode(&self.waveform)
    }

    pub fn fingerprint(&self) -> Option<Fingerprint> {
        Fingerprint::decode(
            FingerprintKind::parse(&self.fingerprint_kind)?,
            &self.fingerprint,
        )
    }
}

const ANALYSIS_COLUMNS: &str = "path, file_version, waveform, fingerprint, fingerprint_kind,
    duration_seconds, error, CAST(analyzed_at AS TEXT) AS analyzed_at";

pub fn analysis_for(
    connection: &mut SqliteConnection,
    path: &str,
) -> QueryResult<Option<AnalysisRow>> {
    Ok(diesel::sql_query(format!(
        "SELECT {ANALYSIS_COLUMNS} FROM library_audio_analysis WHERE path = ?"
    ))
    .bind::<Text, _>(path)
    .load::<AnalysisRow>(connection)?
    .into_iter()
    .next())
}

/// Every stored fingerprint, by path.
pub fn fingerprints(
    connection: &mut SqliteConnection,
) -> QueryResult<HashMap<String, Fingerprint>> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        path: String,
        #[diesel(sql_type = Text)]
        fingerprint: String,
        #[diesel(sql_type = Text)]
        fingerprint_kind: String,
    }
    Ok(diesel::sql_query(
        "SELECT path, fingerprint, fingerprint_kind FROM library_audio_analysis
         WHERE fingerprint != '' AND error IS NULL",
    )
    .load::<Row>(connection)?
    .into_iter()
    .filter_map(|row| {
        let kind = FingerprintKind::parse(&row.fingerprint_kind)?;
        Some((row.path, Fingerprint::decode(kind, &row.fingerprint)?))
    })
    .collect())
}

/// Paths with a stored analysis (successful or not).
pub fn analyzed_paths(connection: &mut SqliteConnection) -> QueryResult<HashSet<String>> {
    Ok(
        diesel::sql_query("SELECT path FROM library_audio_analysis WHERE error IS NULL")
            .load::<PathRow>(connection)?
            .into_iter()
            .map(|row| row.path)
            .collect(),
    )
}

pub fn save_analysis(
    connection: &mut SqliteConnection,
    path: &str,
    file_version: &str,
    waveform: Option<&Waveform>,
    fingerprint: Option<&Fingerprint>,
    duration_seconds: f64,
    error: Option<&str>,
) -> QueryResult<()> {
    diesel::sql_query(
        "INSERT INTO library_audio_analysis
            (path, file_version, waveform, fingerprint, fingerprint_kind, duration_seconds, error, analyzed_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
         ON CONFLICT(path) DO UPDATE SET file_version = excluded.file_version,
             waveform = excluded.waveform, fingerprint = excluded.fingerprint,
             fingerprint_kind = excluded.fingerprint_kind, duration_seconds = excluded.duration_seconds,
             error = excluded.error, analyzed_at = CURRENT_TIMESTAMP",
    )
    .bind::<Text, _>(path)
    .bind::<Text, _>(file_version)
    .bind::<Text, _>(waveform.map(Waveform::encode).unwrap_or_default())
    .bind::<Text, _>(fingerprint.map(Fingerprint::encode).unwrap_or_default())
    .bind::<Text, _>(fingerprint.map(|print| print.kind().as_str()).unwrap_or_default())
    .bind::<Double, _>(duration_seconds)
    .bind::<Nullable<Text>, _>(error)
    .execute(connection)?;
    Ok(())
}

/// Song ids with a stored loudness measurement.
pub fn measured_song_ids(connection: &mut SqliteConnection) -> QueryResult<HashSet<String>> {
    Ok(
        diesel::sql_query("SELECT song_id AS path FROM track_loudness")
            .load::<PathRow>(connection)?
            .into_iter()
            .map(|row| row.path)
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// Missing files
// ---------------------------------------------------------------------------

#[derive(QueryableByName, Clone, Debug)]
pub struct MissingRow {
    #[diesel(sql_type = Text)]
    pub path: String,
    #[diesel(sql_type = Text)]
    pub root_path: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub title: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub album: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub artists_json: Option<String>,
    #[diesel(sql_type = BigInt)]
    pub size_bytes: i64,
    #[diesel(sql_type = Text)]
    pub updated_at: String,
}

pub fn missing_files(connection: &mut SqliteConnection) -> QueryResult<Vec<MissingRow>> {
    diesel::sql_query(
        "SELECT fe.path, lr.path AS root_path, rfm.title, rfm.album,
                rfm.track_artists_json AS artists_json, fe.size_bytes,
                CAST(fe.updated_at AS TEXT) AS updated_at
         FROM file_entry fe
         JOIN library_root lr ON lr.id = fe.root_id
         LEFT JOIN raw_file_metadata rfm ON rfm.file_id = fe.id
         WHERE fe.availability = 'missing'
         ORDER BY fe.path",
    )
    .load::<MissingRow>(connection)
}

/// Forgets missing files: their catalog rows, nothing on disk. Returns how
/// many file records were removed.
pub fn forget_missing(connection: &mut SqliteConnection, paths: &[String]) -> QueryResult<usize> {
    connection.transaction(|connection| {
        let mut removed = 0;
        for path in paths {
            removed += diesel::sql_query(
                "DELETE FROM file_entry WHERE path = ? AND availability = 'missing'",
            )
            .bind::<Text, _>(path)
            .execute(connection)?;
            for table in [
                "library_file_visibility",
                "library_audio_analysis",
                "library_user_album_track",
                "library_skipped_file",
            ] {
                diesel::sql_query(format!("DELETE FROM {table} WHERE path = ?"))
                    .bind::<Text, _>(path)
                    .execute(connection)?;
            }
        }
        // Stale per-file rows whose file the catalog no longer knows.
        diesel::sql_query(
            "DELETE FROM library_audio_analysis
             WHERE NOT EXISTS (SELECT 1 FROM file_entry fe WHERE fe.path = library_audio_analysis.path)",
        )
        .execute(connection)?;
        diesel::sql_query(
            "DELETE FROM library_user_album WHERE NOT EXISTS (
                 SELECT 1 FROM library_user_album_track t WHERE t.album_id = library_user_album.id)",
        )
        .execute(connection)?;
        Ok(removed)
    })
}

#[derive(QueryableByName, Clone, Debug)]
pub struct IdentificationRow {
    #[diesel(sql_type = Text)]
    pub album_id: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub task_status: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub match_status: Option<String>,
}

/// MusicBrainz lookup state and release match per album.
pub fn identification(connection: &mut SqliteConnection) -> QueryResult<Vec<IdentificationRow>> {
    diesel::sql_query(
        "SELECT al.id AS album_id,
                (SELECT status FROM metadata_task mt
                 WHERE mt.provider = 'musicbrainz' AND mt.entity_type = 'album' AND mt.entity_id = al.id)
                    AS task_status,
                (SELECT status FROM album_release_match arm WHERE arm.album_id = al.id) AS match_status
         FROM album_entity al",
    )
    .load::<IdentificationRow>(connection)
}

/// Total bytes of every available file.
pub fn library_bytes(connection: &mut SqliteConnection) -> QueryResult<i64> {
    Ok(diesel::sql_query(
        "SELECT COALESCE(SUM(size_bytes), 0) AS count FROM file_entry WHERE availability = 'available'",
    )
    .get_result::<CountRow>(connection)?
    .count)
}

/// When the last scan finished.
pub fn last_scan_at(connection: &mut SqliteConnection) -> QueryResult<Option<String>> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Nullable<Text>)]
        value: Option<String>,
    }
    Ok(diesel::sql_query(
        "SELECT CAST(MAX(finished_at) AS TEXT) AS value FROM library_scan_job WHERE status != 'failed'",
    )
    .load::<Row>(connection)?
    .into_iter()
    .next()
    .and_then(|row| row.value))
}

/// Paths of every available file on the given albums' tracks.
pub fn paths_for_tracks(
    connection: &mut SqliteConnection,
    track_ids: &[String],
) -> QueryResult<Vec<String>> {
    let mut paths = Vec::new();
    for chunk in track_ids.chunks(400) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let mut query = diesel::sql_query(format!(
            "SELECT fe.path FROM track_file tf JOIN file_entry fe ON fe.id = tf.file_id
             WHERE tf.track_id IN ({placeholders})"
        ))
        .into_boxed();
        for id in chunk {
            query = query.bind::<Text, _>(id.clone());
        }
        paths.extend(
            query
                .load::<PathRow>(connection)?
                .into_iter()
                .map(|row| row.path),
        );
    }
    Ok(paths)
}

pub fn identification_enabled(connection: &mut SqliteConnection) -> bool {
    crate::enrichment::store::load_settings(connection).is_ok_and(|settings| settings.enabled)
}

#[cfg(test)]
mod tests {
    use diesel::connection::SimpleConnection;

    use super::*;

    fn connection()
    -> diesel::r2d2::PooledConnection<diesel::r2d2::ConnectionManager<SqliteConnection>> {
        let pool = crate::persistence::connection::migrated_memory_pool();
        let mut connection = pool.get().expect("connection");
        connection
            .batch_execute(
                "INSERT INTO library_root (id, path) VALUES (1, '/music');
                 INSERT INTO file_entry (id, root_id, path, directory, file_name, extension, size_bytes, modified_at_ns, availability) VALUES
                    (1, 1, '/music/a.flac', '/music', 'a.flac', 'flac', 100, 1, 'available'),
                    (2, 1, '/music/a.mp3', '/music', 'a.mp3', 'mp3', 50, 1, 'available'),
                    (3, 1, '/music/gone.flac', '/music', 'gone.flac', 'flac', 70, 1, 'missing');
                 INSERT INTO raw_file_metadata (file_id, title, duration_seconds, parser_version, codec, sample_rate, bit_depth) VALUES
                    (1, 'A', 200, '21', 'flac', 96000, 24),
                    (2, 'A', 200, '21', 'mp3', 44100, NULL),
                    (3, 'Gone', 100, '21', 'flac', 44100, 16);
                 INSERT INTO track_entity (id, title, normalized_title) VALUES ('t-a', 'A', 'a'), ('t-b', 'B', 'b');
                 INSERT INTO track_file (track_id, file_id, quality_rank, is_primary) VALUES
                    ('t-a', 1, 10, true), ('t-a', 2, 5, false);
                 INSERT INTO \"user\" (id, username, password) VALUES (1, 'listener', 'x');
                 INSERT INTO song (id) VALUES ('t-b');
                 INSERT INTO favorite_song (user_id, song_id) VALUES (1, 't-b');
                 INSERT INTO playlist (id, name) VALUES (1, 'Mix');
                 INSERT INTO _playlist_to_song (a, b, position) VALUES (1, 't-b', 3);",
            )
            .expect("fixture");
        connection
    }

    #[test]
    fn copies_preferences_and_rereads_touch_only_parson_rows() {
        let mut connection = connection();
        let copies = multi_copy_tracks(&mut connection).unwrap();
        assert_eq!(copies.len(), 2);
        assert_eq!(copies[0].path, "/music/a.flac");
        assert_eq!(copies[0].audio().unwrap().bit_depth, Some(24));
        set_file_preference(&mut connection, "t-a", Some("/music/a.mp3")).unwrap();
        assert_eq!(
            file_preferences(&mut connection).unwrap()[0].path,
            "/music/a.mp3"
        );
        set_file_preference(&mut connection, "t-a", None).unwrap();
        assert!(file_preferences(&mut connection).unwrap().is_empty());

        assert_eq!(
            mark_files_for_reread(&mut connection, &["/music/a.mp3".into()]).unwrap(),
            1
        );
        #[derive(QueryableByName)]
        struct Version {
            #[diesel(sql_type = Nullable<Text>)]
            parser_version: Option<String>,
        }
        let versions =
            diesel::sql_query("SELECT parser_version FROM raw_file_metadata ORDER BY file_id")
                .load::<Version>(&mut connection)
                .unwrap();
        assert_eq!(versions[0].parser_version.as_deref(), Some("21"));
        assert_eq!(versions[1].parser_version, None);
        assert_eq!(
            paths_for_tracks(&mut connection, &["t-a".into()])
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn merging_carries_likes_and_playlist_entries_and_restoring_keeps_them() {
        let mut connection = connection();
        set_visibility(
            &mut connection,
            "/music/b.flac",
            "merged",
            Some("/music/a.flac"),
            "t-b",
        )
        .unwrap();
        carry_references(&mut connection, "t-b", "t-a").unwrap();
        // Carrying twice changes nothing more.
        carry_references(&mut connection, "t-b", "t-a").unwrap();
        let favorites =
            diesel::sql_query("SELECT song_id AS path FROM favorite_song ORDER BY song_id")
                .load::<PathRow>(&mut connection)
                .unwrap();
        assert_eq!(
            favorites
                .iter()
                .map(|row| row.path.as_str())
                .collect::<Vec<_>>(),
            vec!["t-a", "t-b"]
        );
        let entries = diesel::sql_query("SELECT b AS path FROM _playlist_to_song ORDER BY b")
            .load::<PathRow>(&mut connection)
            .unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(visibility_rows(&mut connection).unwrap()[0].state, "merged");
        assert_eq!(
            clear_visibility(&mut connection, "/music/b.flac").unwrap(),
            1
        );
        assert!(visibility_rows(&mut connection).unwrap().is_empty());
    }

    #[test]
    fn missing_files_are_forgotten_without_touching_available_ones() {
        let mut connection = connection();
        save_analysis(
            &mut connection,
            "/music/gone.flac",
            "70:1",
            None,
            None,
            0.0,
            None,
        )
        .unwrap();
        let missing = missing_files(&mut connection).unwrap();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].title.as_deref(), Some("Gone"));
        assert_eq!(missing[0].root_path, "/music");
        // An available file is never removed, even when asked.
        let removed = forget_missing(
            &mut connection,
            &["/music/gone.flac".into(), "/music/a.flac".into()],
        )
        .unwrap();
        assert_eq!(removed, 1);
        assert!(missing_files(&mut connection).unwrap().is_empty());
        assert_eq!(imported_files(&mut connection).unwrap().len(), 2);
        assert!(
            analysis_for(&mut connection, "/music/gone.flac")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn user_albums_versions_and_skipped_findings_round_trip() {
        let mut connection = connection();
        let album = UserAlbumRow {
            id: "ua-1".into(),
            title: "Made".into(),
            artist_id: None,
            artist_name: "Someone".into(),
            year: "2020".into(),
            primary_type: "album".into(),
            cover_path: String::new(),
        };
        save_user_album(
            &mut connection,
            &album,
            &[("/music/b.flac".into(), 1), ("/music/a.flac".into(), 1)],
        )
        .unwrap();
        let second = UserAlbumRow {
            id: "ua-2".into(),
            ..album.clone()
        };
        // Moving every track of ua-1 into ua-2 removes the emptied ua-1.
        save_user_album(
            &mut connection,
            &second,
            &[("/music/a.flac".into(), 1), ("/music/b.flac".into(), 2)],
        )
        .unwrap();
        let (albums, tracks) = user_albums(&mut connection).unwrap();
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].id, "ua-2");
        assert_eq!(tracks[0].path, "/music/a.flac");
        assert_eq!((tracks[1].position, tracks[1].disc), (2, 2));
        assert_eq!(delete_user_album(&mut connection, "ua-2").unwrap(), 1);

        set_version_member(&mut connection, "album-1", Some("manual-x")).unwrap();
        set_version_member(&mut connection, "album-2", None).unwrap();
        let members = version_members(&mut connection).unwrap();
        assert_eq!(members.get("album-1"), Some(&Some("manual-x".to_string())));
        assert_eq!(members.get("album-2"), Some(&None));
        let choice = VersionChoice {
            preferred_album_id: "album-1".into(),
            hide_others: true,
        };
        set_version_choice(&mut connection, "manual-x", Some(&choice)).unwrap();
        assert_eq!(
            version_choices(&mut connection).unwrap().get("manual-x"),
            Some(&choice)
        );

        let record = SkippedRecord {
            path: "/music/v.mkv".into(),
            root_path: "/music".into(),
            reason: super::super::skipped::SkipReason::VideoFile,
            detail: "MKV video.".into(),
            extension: "mkv".into(),
            size_bytes: 10,
            modified_at_ns: 5,
        };
        replace_skipped(&mut connection, "/music", std::slice::from_ref(&record)).unwrap();
        set_skipped_ignored(&mut connection, &["/music/v.mkv".into()], true).unwrap();
        assert_eq!(skipped_count(&mut connection).unwrap(), 0);
        // The same finding keeps its "ignored" choice; a changed file does not.
        replace_skipped(&mut connection, "/music", std::slice::from_ref(&record)).unwrap();
        assert_eq!(skipped_count(&mut connection).unwrap(), 0);
        let changed = SkippedRecord {
            modified_at_ns: 6,
            ..record
        };
        replace_skipped(&mut connection, "/music", &[changed]).unwrap();
        assert_eq!(skipped_count(&mut connection).unwrap(), 1);
        replace_skipped(&mut connection, "/music", &[]).unwrap();
        assert!(skipped_rows(&mut connection).unwrap().is_empty());
        record_skipped_check(&mut connection).unwrap();
        assert!(last_skipped_check(&mut connection).unwrap().is_some());
    }
}
