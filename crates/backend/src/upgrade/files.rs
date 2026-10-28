//! Track facts from the catalog tables, for matching what FETCH delivered
//! to the album it should replace.

use std::path::Path;

use diesel::QueryResult;
use diesel::prelude::*;
use diesel::sql_types::{Double, Integer, Nullable, Text};
use diesel::sqlite::SqliteConnection;

use super::matching::TrackFact;
use crate::library::identity::{normalize_album_identity, normalize_artist_identity};
use crate::library::tools::versions::split_edition;

#[derive(QueryableByName)]
struct FactRow {
    #[diesel(sql_type = Text)]
    path: String,
    #[diesel(sql_type = Text)]
    track_id: String,
    #[diesel(sql_type = Text)]
    album_id: String,
    #[diesel(sql_type = Text)]
    album_title: String,
    #[diesel(sql_type = Text)]
    album_artists_json: String,
    #[diesel(sql_type = Text)]
    track_artists_json: String,
    #[diesel(sql_type = Nullable<Text>)]
    release_mbid: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    recording_mbid: Option<String>,
    #[diesel(sql_type = Integer)]
    disc: i32,
    #[diesel(sql_type = Integer)]
    number: i32,
    #[diesel(sql_type = Double)]
    duration: f64,
}

const FACT_QUERY: &str = "SELECT fe.path, tf.track_id, COALESCE(te.album_id, '') AS album_id,
        COALESCE(ae.title, rfm.album, '') AS album_title,
        COALESCE(rfm.album_artists_json, '[]') AS album_artists_json,
        COALESCE(rfm.track_artists_json, '[]') AS track_artists_json,
        COALESCE(NULLIF(rfm.musicbrainz_release_id, ''), ae.musicbrainz_id) AS release_mbid,
        COALESCE(NULLIF(rfm.musicbrainz_recording_id, ''), te.musicbrainz_recording_id) AS recording_mbid,
        COALESCE(rfm.disc_number, te.disc_number, 1) AS disc,
        COALESCE(rfm.track_number, te.track_number, 0) AS number,
        COALESCE(NULLIF(rfm.duration_seconds, 0), te.duration_seconds, 0) AS duration
    FROM file_entry fe
    JOIN track_file tf ON tf.file_id = fe.id
    JOIN track_entity te ON te.id = tf.track_id
    LEFT JOIN album_entity ae ON ae.id = te.album_id
    LEFT JOIN raw_file_metadata rfm ON rfm.file_id = fe.id
    WHERE fe.availability = 'available'";

fn first_name(json: &str) -> String {
    serde_json::from_str::<Vec<String>>(json)
        .ok()
        .and_then(|names| names.into_iter().find(|name| !name.trim().is_empty()))
        .unwrap_or_default()
}

fn facts(rows: Vec<FactRow>) -> Vec<TrackFact> {
    rows.into_iter()
        .map(|row| {
            let album_artist = match first_name(&row.album_artists_json) {
                name if !name.is_empty() => name,
                _ => first_name(&row.track_artists_json),
            };
            TrackFact {
                path: row.path,
                track_id: row.track_id,
                album_id: row.album_id,
                album_title: row.album_title,
                album_artist,
                release_mbid: row.release_mbid.filter(|id| !id.is_empty()),
                recording_mbid: row.recording_mbid.filter(|id| !id.is_empty()),
                disc: u32::try_from(row.disc).unwrap_or(1),
                number: u32::try_from(row.number).unwrap_or(0),
                duration: row.duration,
            }
        })
        .collect()
}

/// Every available file of an album's tracks.
pub fn album_track_facts(
    connection: &mut SqliteConnection,
    album_id: &str,
) -> QueryResult<Vec<TrackFact>> {
    // The playing copy first, so matching pairs the original with the new.
    let rows = diesel::sql_query(format!(
        "{FACT_QUERY} AND te.album_id = ? ORDER BY tf.is_primary DESC, fe.path"
    ))
    .bind::<Text, _>(album_id)
    .load::<FactRow>(connection)?;
    let mut seen = std::collections::HashSet::new();
    Ok(facts(rows)
        .into_iter()
        .filter(|fact| seen.insert(fact.track_id.clone()))
        .collect())
}

/// Every available file of an album's tracks, every copy.
pub fn album_copy_facts(
    connection: &mut SqliteConnection,
    album_id: &str,
) -> QueryResult<Vec<TrackFact>> {
    let rows = diesel::sql_query(format!(
        "{FACT_QUERY} AND te.album_id = ? ORDER BY tf.is_primary DESC, fe.path"
    ))
    .bind::<Text, _>(album_id)
    .load::<FactRow>(connection)?;
    Ok(facts(rows))
}

/// Whether `path` lies inside `folder`.
pub fn path_is_under(path: &str, folder: &str) -> bool {
    let folder = folder.trim_end_matches(['/', '\\']);
    path.len() > folder.len()
        && path.starts_with(folder)
        && matches!(path.as_bytes()[folder.len()], b'/' | b'\\')
}

/// One original copy per track: never one `excluded` (where FETCH saved),
/// a lossy copy before a lossless one, as an upgrade replaces the lossy.
pub fn original_copies(copies: &[TrackFact], excluded: impl Fn(&str) -> bool) -> Vec<TrackFact> {
    let mut sorted: Vec<&TrackFact> = copies.iter().filter(|fact| !excluded(&fact.path)).collect();
    sorted.sort_by_key(|fact| is_lossless_path(&fact.path));
    let mut seen = std::collections::HashSet::new();
    sorted
        .into_iter()
        .filter(|fact| seen.insert(fact.track_id.clone()))
        .cloned()
        .collect()
}

/// Escapes `%`, `_` and `\` for a `LIKE ... ESCAPE '\'` prefix.
fn like_prefix(folder: &str) -> String {
    let mut escaped = String::with_capacity(folder.len() + 2);
    for character in folder.trim_end_matches(['/', '\\']).chars() {
        if matches!(character, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped.push_str(std::path::MAIN_SEPARATOR_STR);
    escaped.push('%');
    escaped
}

/// Files under a folder (where FETCH saved the album).
pub fn facts_under(connection: &mut SqliteConnection, folder: &str) -> QueryResult<Vec<TrackFact>> {
    let rows = diesel::sql_query(format!("{FACT_QUERY} AND fe.path LIKE ? ESCAPE '\\'"))
        .bind::<Text, _>(like_prefix(folder))
        .load::<FactRow>(connection)?;
    Ok(facts(rows))
}

/// Files of any album with the same artist and base title, wherever they
/// are (when FETCH's folder is not known).
pub fn facts_like_album(
    connection: &mut SqliteConnection,
    artist: &str,
    title: &str,
) -> QueryResult<Vec<TrackFact>> {
    let base = normalize_album_identity(&split_edition(title).0);
    let artist = normalize_artist_identity(artist);
    let rows = diesel::sql_query(format!(
        "{FACT_QUERY} AND te.album_id IN (SELECT id FROM album_entity)"
    ))
    .load::<FactRow>(connection)?;
    Ok(facts(rows)
        .into_iter()
        .filter(|fact| {
            normalize_album_identity(&split_edition(&fact.album_title).0) == base
                && normalize_artist_identity(&fact.album_artist) == artist
        })
        .collect())
}

#[derive(QueryableByName, Clone, Debug)]
pub struct LosslessFile {
    #[diesel(sql_type = Text)]
    pub path: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub codec: Option<String>,
    #[diesel(sql_type = Text)]
    pub extension: String,
    #[diesel(sql_type = Nullable<Integer>)]
    pub sample_rate: Option<i32>,
}

/// Every available file worth the fake-lossless check, optionally only
/// the files of `album_ids`.
pub fn lossless_files(
    connection: &mut SqliteConnection,
    album_ids: Option<&[String]>,
) -> QueryResult<Vec<LosslessFile>> {
    let rows = diesel::sql_query(
        "SELECT fe.path, rfm.codec, fe.extension, rfm.sample_rate,
                COALESCE((SELECT te.album_id FROM track_file tf JOIN track_entity te ON te.id = tf.track_id
                          WHERE tf.file_id = fe.id LIMIT 1), '') AS album_id
         FROM file_entry fe LEFT JOIN raw_file_metadata rfm ON rfm.file_id = fe.id
         WHERE fe.availability = 'available'
         ORDER BY fe.path",
    )
    .load::<LosslessFileRow>(connection)?;
    let wanted: Option<std::collections::HashSet<&str>> =
        album_ids.map(|ids| ids.iter().map(String::as_str).collect());
    Ok(rows
        .into_iter()
        .filter(|row| {
            wanted
                .as_ref()
                .is_none_or(|wanted| wanted.contains(row.album_id.as_str()))
        })
        .filter(|row| {
            crate::library::tools::spectral::worth_checking(row.codec.as_deref(), &row.extension)
        })
        .map(|row| LosslessFile {
            path: row.path,
            codec: row.codec,
            extension: row.extension,
            sample_rate: row.sample_rate,
        })
        .collect())
}

#[derive(QueryableByName)]
struct LosslessFileRow {
    #[diesel(sql_type = Text)]
    path: String,
    #[diesel(sql_type = Nullable<Text>)]
    codec: Option<String>,
    #[diesel(sql_type = Text)]
    extension: String,
    #[diesel(sql_type = Nullable<Integer>)]
    sample_rate: Option<i32>,
    #[diesel(sql_type = Text)]
    album_id: String,
}

pub fn is_lossless_path(path: &str) -> bool {
    let extension = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "flac" | "wav" | "aif" | "aiff" | "alac" | "ape" | "wv" | "tta" | "dsf" | "dff"
    )
}

/// `size:mtime_ns` of a file, to notice when it changes.
pub fn file_version(path: &Path) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    Some(format!("{}:{modified}", metadata.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use diesel::connection::SimpleConnection;

    #[test]
    fn facts_come_from_the_catalog_tables() {
        let pool = crate::persistence::connection::migrated_memory_pool();
        let mut connection = pool.get().unwrap();
        connection
            .batch_execute(
                "INSERT INTO library_root (id, path) VALUES (1, '/m');
                 INSERT INTO file_entry (id, root_id, path, directory, file_name, extension, size_bytes, modified_at_ns, availability) VALUES
                    (1, 1, '/m/mp3/1.mp3', '/m/mp3', '1.mp3', 'mp3', 1, 1, 'available'),
                    (2, 1, '/m/Rise Against/Endgame/01.flac', '/m/Rise Against/Endgame', '01.flac', 'flac', 1, 1, 'available'),
                    (3, 1, '/m/Rise_Against/x.flac', '/m/Rise_Against', 'x.flac', 'flac', 1, 1, 'available');
                 INSERT INTO raw_file_metadata (file_id, title, album, album_artists_json, track_number, disc_number, duration_seconds, codec) VALUES
                    (1, 'Architects', 'Endgame', '[\"Rise Against\"]', 1, 1, 200, 'mp3'),
                    (2, 'Architects', 'Endgame', '[\"Rise Against\"]', 1, 1, 201, 'flac'),
                    (3, 'Other', 'Other', '[\"Someone\"]', 1, 1, 100, 'flac');
                 INSERT INTO album_entity (id, title, normalized_title) VALUES ('old', 'Endgame', 'endgame'), ('o2', 'Other', 'other');
                 INSERT INTO track_entity (id, title, normalized_title, album_id) VALUES
                    ('t1', 'Architects', 'architects', 'old'), ('t3', 'Other', 'other', 'o2');
                 INSERT INTO track_file (track_id, file_id, quality_rank, is_primary) VALUES
                    ('t1', 1, 3000, false), ('t1', 2, 10000, true), ('t3', 3, 10000, true);",
            )
            .unwrap();
        let old = album_track_facts(&mut connection, "old").unwrap();
        assert_eq!(old.len(), 1, "one fact per track");
        assert_eq!(old[0].album_artist, "Rise Against");
        let under = facts_under(&mut connection, "/m/Rise Against/Endgame").unwrap();
        assert_eq!(under.len(), 1);
        assert_eq!(under[0].duration, 201.0);
        // `_` in the folder name is literal, not a wildcard.
        assert!(
            facts_under(&mut connection, "/m/Rise Against")
                .unwrap()
                .len()
                == 1
        );
        let like = facts_like_album(&mut connection, "Rise Against", "Endgame (Deluxe)").unwrap();
        assert_eq!(like.len(), 2);
        assert!(is_lossless_path("/a/b.FLAC"));
        // The FETCH folder's FLAC is a copy of the same track: the
        // original is the MP3 outside it.
        let copies = album_copy_facts(&mut connection, "old").unwrap();
        assert_eq!(copies.len(), 2);
        let originals = original_copies(&copies, |path| {
            path_is_under(path, "/m/Rise Against/Endgame")
        });
        assert_eq!(originals.len(), 1);
        assert_eq!(originals[0].path, "/m/mp3/1.mp3");
        assert!(path_is_under("/m/a/b.flac", "/m/a/"));
        assert!(!path_is_under("/m/ab/c.flac", "/m/a"));
        assert!(!is_lossless_path("/a/b.mp3"));
    }
}
