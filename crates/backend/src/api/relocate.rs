//! "Locate…" for a music folder that moved, typically after restoring a
//! backup from another computer onto a new server. Parson's track and album
//! ids include the folder a release lives in, so indexing the new location
//! creates new ids. This module notes where every old track sat inside the
//! folder, indexes the new folder, drops the old one (the catalog then
//! settles on its final ids), matches every old track to its new id by that
//! place in the folder and moves playlists, favourites, history,
//! preferences and metadata edits over to it. Playlists live in
//! `_playlist_to_song` (backed by the never-deleted `song` table), so they
//! survive the catalog rebuild in between.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use actix_web::{HttpResponse, post, web};
use diesel::connection::SimpleConnection;
use diesel::deserialize::QueryableByName;
use diesel::sql_types::{Nullable, Text};
use diesel::{RunQueryDsl, SqliteConnection};
use serde::{Deserialize, Serialize};

use crate::api::error::{bad_request, internal_server_error, not_found};
use crate::api::library::{
    LibraryIndexRequest, RemoveLibraryRootQuery, index_library, read_library_paths, remove_root,
};
use crate::library::state::LibraryLifecycle;
use crate::persistence::connection::DbPool;

/// Catalog tables the indexer owns and rebuilds; never rewritten here.
const CATALOG_TABLES: &[&str] = &[
    "track_entity",
    "track_file",
    "track_artist",
    "track_genre",
    "album_entity",
    "album_artist",
    "album_genre",
    "album_inference_cache",
    "library_search_document",
    "metadata_task",
    "raw_file_metadata",
    "file_entry",
    "recording_entity",
    "release_group_entity",
];
/// Columns that hold a track (song) id in user data.
const TRACK_COLUMNS: &[&str] = &[
    "song_id",
    "track_id",
    "from_song_id",
    "to_song_id",
    "seed_song_id",
];

#[derive(Deserialize)]
struct RelocateRequest {
    from: String,
    to: String,
}

#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub struct RelocationReport {
    /// Tracks found again at the new location.
    pub matched_tracks: usize,
    /// Tracks of the old folder that are not in the new one.
    pub missing_tracks: usize,
    /// User-data rows moved to the new ids (playlists, favourites, history…).
    pub moved_rows: usize,
}

#[derive(QueryableByName)]
struct TrackPathRow {
    #[diesel(sql_type = Text)]
    track_id: String,
    #[diesel(sql_type = Nullable<Text>)]
    album_id: Option<String>,
    #[diesel(sql_type = Text)]
    path: String,
}

/// `path` relative to `root`, with `/` separators, or `None` outside it.
fn relative_to(path: &str, root: &str) -> Option<String> {
    let root = root.trim_end_matches(['/', '\\']);
    let rest = path.strip_prefix(root)?;
    let rest = rest.strip_prefix(['/', '\\'])?;
    Some(rest.replace('\\', "/"))
}

fn tracks_under(
    connection: &mut SqliteConnection,
    root: &str,
) -> Result<Vec<TrackPathRow>, String> {
    let root = root.trim_end_matches(['/', '\\']);
    diesel::sql_query(
        "SELECT track_file.track_id AS track_id, track_entity.album_id AS album_id, file_entry.path AS path
         FROM track_file
         JOIN file_entry ON file_entry.id = track_file.file_id
         JOIN track_entity ON track_entity.id = track_file.track_id
         WHERE substr(file_entry.path, 1, length(?1) + 1) IN (?1 || '/', ?1 || '\\')",
    )
    .bind::<Text, _>(root)
    .load::<TrackPathRow>(connection)
    .map_err(|error| error.to_string())
}

/// Old id → new id, for tracks and for albums, matched by path in the folder.
pub fn relocation_maps(
    old: &[(String, Option<String>, String)],
    from: &str,
    new: &[(String, Option<String>, String)],
    to: &str,
) -> (HashMap<String, String>, HashMap<String, String>, usize) {
    let by_path = new
        .iter()
        .filter_map(|(track, album, path)| Some((relative_to(path, to)?, (track, album))))
        .collect::<HashMap<_, _>>();
    let mut tracks = HashMap::new();
    let mut albums = HashMap::new();
    let mut missing = HashSet::new();
    for (track, album, path) in old {
        let Some(relative) = relative_to(path, from) else {
            continue;
        };
        match by_path.get(&relative) {
            Some((new_track, new_album)) => {
                if *new_track != track {
                    tracks.insert(track.clone(), (*new_track).clone());
                }
                if let (Some(old_album), Some(new_album)) = (album, new_album)
                    && old_album != new_album
                {
                    albums.insert(old_album.clone(), new_album.clone());
                }
            }
            None => {
                missing.insert(track.clone());
            }
        }
    }
    missing.retain(|track| !tracks.contains_key(track));
    (tracks, albums, missing.len())
}

#[derive(QueryableByName)]
struct NameRow {
    #[diesel(sql_type = Text)]
    name: String,
}

fn quote(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

/// Moves every user-data reference from old ids to new ids in one
/// transaction. Rows that would collide (the same favourite twice) keep the
/// old id and are cleaned up with the old folder.
pub fn apply_relocation(
    connection: &mut SqliteConnection,
    tracks: &HashMap<String, String>,
    albums: &HashMap<String, String>,
) -> Result<usize, String> {
    // IMMEDIATE takes the write lock up front, so a concurrent indexer write
    // makes this wait (busy timeout) instead of failing on lock upgrade.
    connection
        .immediate_transaction::<usize, diesel::result::Error, _>(|connection| {
            connection.batch_execute(
                "DROP TABLE IF EXISTS temp.relocated_id;
                 CREATE TEMP TABLE relocated_id(old TEXT PRIMARY KEY, new TEXT NOT NULL, kind TEXT NOT NULL);",
            )?;
            for (kind, map) in [("track", tracks), ("album", albums)] {
                for (old, new) in map {
                    diesel::sql_query("INSERT OR REPLACE INTO temp.relocated_id(old, new, kind) VALUES (?, ?, ?)")
                        .bind::<Text, _>(old)
                        .bind::<Text, _>(new)
                        .bind::<Text, _>(kind)
                        .execute(connection)?;
                }
            }
            // The legacy `song` table is what favourites and playlists
            // reference; make sure every new id has its row.
            let has_song_table = !diesel::sql_query(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'song'",
            )
            .load::<NameRow>(connection)?
            .is_empty();
            if has_song_table {
                connection.batch_execute(
                    "INSERT OR IGNORE INTO song(id) SELECT new FROM temp.relocated_id WHERE kind = 'track';",
                )?;
            }
            let tables = diesel::sql_query(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            )
            .load::<NameRow>(connection)?;
            let mut moved = 0;
            for table in tables {
                if CATALOG_TABLES.contains(&table.name.as_str()) || table.name == "song" {
                    continue;
                }
                let columns = diesel::sql_query(format!(
                    "SELECT name FROM pragma_table_info('{}')",
                    table.name.replace('\'', "''")
                ))
                .load::<NameRow>(connection)?;
                let names = columns.iter().map(|column| column.name.as_str()).collect::<Vec<_>>();
                let mut targets = Vec::new();
                for column in &names {
                    if TRACK_COLUMNS.contains(column)
                        || (table.name == "_playlist_to_song" && *column == "b")
                    {
                        targets.push((column.to_string(), "kind = 'track'"));
                    } else if *column == "album_id" {
                        targets.push((column.to_string(), "kind = 'album'"));
                    }
                }
                // Metadata edits and external ids are keyed by entity id.
                if names.contains(&"entity_id") && names.contains(&"entity_type") {
                    targets.push(("entity_id".to_string(), "1 = 1"));
                }
                for (column, kind) in targets {
                    let table_name = quote(&table.name);
                    let column_name = quote(&column);
                    moved += diesel::sql_query(format!(
                        "UPDATE OR IGNORE {table_name} SET {column_name} =
                           (SELECT new FROM temp.relocated_id WHERE old = {table_name}.{column_name} AND {kind})
                         WHERE {column_name} IN (SELECT old FROM temp.relocated_id WHERE {kind})"
                    ))
                    .execute(connection)?;
                }
            }
            connection.batch_execute("DROP TABLE temp.relocated_id;")?;
            // `playlist_track` is derived from `_playlist_to_song` by the
            // indexer; bring it up to date for the moved entries now.
            let derived = diesel::sql_query(
                "SELECT name FROM sqlite_master WHERE type = 'table'
                 AND name IN ('playlist_track', '_playlist_to_song', 'track_entity')",
            )
            .load::<NameRow>(connection)?;
            if derived.len() == 3 {
                connection.batch_execute(
                    "INSERT INTO playlist_track (playlist_id, track_id, date_added, added_by, position)
                     SELECT pts.a, pts.b, pts.date_added, pts.added_by, pts.position FROM _playlist_to_song pts
                     JOIN track_entity t ON t.id = pts.b WHERE true
                     ON CONFLICT(playlist_id, track_id) DO NOTHING;",
                )?;
            }
            Ok(moved)
        })
        .map_err(|error| error.to_string())
}

fn as_triples(rows: Vec<TrackPathRow>) -> Vec<(String, Option<String>, String)> {
    rows.into_iter()
        .map(|row| (row.track_id, row.album_id, row.path))
        .collect()
}

#[post("/roots/relocate")]
pub async fn relocate_library_root(
    request: web::Json<RelocateRequest>,
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let from = request.from.trim().to_string();
    let requested = request.to.trim().to_string();
    if from.is_empty() || requested.is_empty() || requested.len() > 4096 {
        return bad_request(
            "Choose the folder that holds the music now.",
            "library_path_empty",
        );
    }
    if !read_library_paths().await.contains(&from) {
        return not_found("Library folder was not found.", "library_root_not_found");
    }
    let to = match tokio::fs::canonicalize(&requested).await {
        Ok(path) if path.is_dir() => path.to_string_lossy().into_owned(),
        _ => {
            return bad_request(
                "That folder isn’t available on this server.",
                "library_path_unavailable",
            );
        }
    };
    if Path::new(&to) == Path::new(&from) {
        return bad_request(
            "That is the folder Parson already looks in.",
            "library_path_same",
        );
    }

    let snapshot_pool = pool.get_ref().clone();
    let snapshot_from = from.clone();
    let old = match web::block(move || -> Result<_, String> {
        let mut connection = snapshot_pool.get().map_err(|error| error.to_string())?;
        tracks_under(&mut connection, &snapshot_from).map(as_triples)
    })
    .await
    {
        Ok(Ok(rows)) => rows,
        Ok(Err(error)) => return internal_server_error(error, "relocate_failed"),
        Err(error) => return internal_server_error(error.to_string(), "relocate_failed"),
    };

    let indexed = index_library(
        web::Json(LibraryIndexRequest::new(to.clone())),
        lifecycle.clone(),
    )
    .await;
    if !indexed.status().is_success() {
        return indexed;
    }

    // Dropping the old folder lets the catalog settle on its final ids. The
    // index above may still be publishing its results; wait for it.
    let mut waited = 0;
    loop {
        let removed = remove_root(
            web::Query(RemoveLibraryRootQuery::new(from.clone())),
            lifecycle.clone(),
        )
        .await;
        if removed.status().is_success() {
            break;
        }
        if removed.status() != actix_web::http::StatusCode::CONFLICT || waited >= 240 {
            return removed;
        }
        waited += 1;
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }

    let remap_pool = pool.get_ref().clone();
    let (remap_from, remap_to) = (from.clone(), to.clone());
    let report = match web::block(move || -> Result<RelocationReport, String> {
        let mut connection = remap_pool.get().map_err(|error| error.to_string())?;
        let new = as_triples(tracks_under(&mut connection, &remap_to)?);
        let (tracks, albums, missing_tracks) = relocation_maps(&old, &remap_from, &new, &remap_to);
        let mut attempt = 0;
        let moved_rows = loop {
            match apply_relocation(&mut connection, &tracks, &albums) {
                Ok(moved) => break moved,
                // The indexer may still be finishing its own writes.
                Err(error) if error.contains("locked") && attempt < 20 => {
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
                Err(error) => return Err(error),
            }
        };
        Ok(RelocationReport {
            matched_tracks: old.len().saturating_sub(missing_tracks),
            missing_tracks,
            moved_rows,
        })
    })
    .await
    {
        Ok(Ok(report)) => report,
        Ok(Err(error)) => {
            tracing::error!(%error, "moving user data to the relocated folder failed");
            return internal_server_error(
                "The music folder moved, but your playlists and history could not be moved with it. Nothing was deleted.",
                "relocate_failed",
            );
        }
        Err(error) => return internal_server_error(error.to_string(), "relocate_failed"),
    };
    HttpResponse::Ok().json(report)
}

#[cfg(test)]
mod tests {
    use super::{apply_relocation, relative_to, relocation_maps};
    use diesel::connection::SimpleConnection;
    use diesel::{Connection, RunQueryDsl, SqliteConnection};
    use std::collections::HashMap;

    #[test]
    fn paths_are_compared_inside_their_folder() {
        assert_eq!(
            relative_to("/home/ana/Music/A/01.flac", "/home/ana/Music/").as_deref(),
            Some("A/01.flac")
        );
        assert_eq!(
            relative_to(r"D:\Music\A\01.flac", r"D:\Music").as_deref(),
            Some("A/01.flac")
        );
        assert_eq!(
            relative_to("/home/ana/Music2/x.flac", "/home/ana/Music"),
            None
        );
    }

    #[test]
    fn tracks_and_albums_are_matched_by_their_place_in_the_folder() {
        let old = vec![
            (
                "t1".into(),
                Some("a1".into()),
                "/old/Artist/Album/01.flac".into(),
            ),
            (
                "t2".into(),
                Some("a1".into()),
                "/old/Artist/Album/02.flac".into(),
            ),
            ("t3".into(), Some("a2".into()), "/old/Gone/01.flac".into()),
        ];
        let new = vec![
            (
                "n1".into(),
                Some("b1".into()),
                "/mnt/nas/music/Artist/Album/01.flac".into(),
            ),
            (
                "n2".into(),
                Some("b1".into()),
                "/mnt/nas/music/Artist/Album/02.flac".into(),
            ),
        ];
        let (tracks, albums, missing) = relocation_maps(&old, "/old", &new, "/mnt/nas/music");
        assert_eq!(tracks.get("t1").map(String::as_str), Some("n1"));
        assert_eq!(tracks.get("t2").map(String::as_str), Some("n2"));
        assert_eq!(albums.get("a1").map(String::as_str), Some("b1"));
        assert_eq!(missing, 1);
    }

    #[test]
    fn user_data_follows_the_tracks_and_catalog_tables_are_left_alone() {
        let mut connection = SqliteConnection::establish(":memory:").unwrap();
        connection
            .batch_execute(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE song(id TEXT PRIMARY KEY);
                 CREATE TABLE favorite_song(user_id INTEGER, song_id TEXT REFERENCES song(id), PRIMARY KEY(user_id, song_id));
                 CREATE TABLE _playlist_to_song(a INTEGER, b TEXT REFERENCES song(id), position INTEGER);
                 CREATE TABLE listen_history_item(id INTEGER PRIMARY KEY, user_id INTEGER, song_id TEXT);
                 CREATE TABLE track_transition(from_song_id TEXT, to_song_id TEXT);
                 CREATE TABLE user_album_preference(user_id INTEGER, album_id TEXT);
                 CREATE TABLE metadata_override(entity_type TEXT, entity_id TEXT, field TEXT);
                 CREATE TABLE track_entity(id TEXT PRIMARY KEY, album_id TEXT);
                 INSERT INTO song VALUES ('t1'), ('t2'), ('n1');
                 INSERT INTO favorite_song VALUES (1, 't1'), (1, 'n1'), (2, 't2');
                 INSERT INTO _playlist_to_song VALUES (7, 't1', 0), (7, 't2', 1);
                 INSERT INTO listen_history_item(user_id, song_id) VALUES (1, 't1'), (1, 't1');
                 INSERT INTO track_transition VALUES ('t1', 't2');
                 INSERT INTO user_album_preference VALUES (1, 'a1');
                 INSERT INTO metadata_override VALUES ('album', 'a1', 'title'), ('track', 't2', 'title');
                 INSERT INTO track_entity VALUES ('t1', 'a1');",
            )
            .unwrap();
        let tracks = HashMap::from([
            ("t1".to_string(), "n1".to_string()),
            ("t2".to_string(), "n2".to_string()),
        ]);
        let albums = HashMap::from([("a1".to_string(), "b1".to_string())]);
        let moved = apply_relocation(&mut connection, &tracks, &albums).unwrap();
        assert!(moved >= 9, "{moved}");

        #[derive(diesel::deserialize::QueryableByName)]
        struct Row {
            #[diesel(sql_type = diesel::sql_types::Text)]
            value: String,
        }
        let mut values = |sql: &str| {
            diesel::sql_query(sql)
                .load::<Row>(&mut connection)
                .unwrap()
                .into_iter()
                .map(|row| row.value)
                .collect::<Vec<_>>()
        };
        // The duplicate favourite keeps its old id (cleaned up with the old
        // folder); the others follow.
        assert_eq!(
            values("SELECT song_id AS value FROM favorite_song ORDER BY user_id, song_id"),
            vec!["n1", "t1", "n2"]
        );
        assert_eq!(
            values("SELECT b AS value FROM _playlist_to_song ORDER BY position"),
            vec!["n1", "n2"]
        );
        assert_eq!(
            values("SELECT song_id AS value FROM listen_history_item"),
            vec!["n1", "n1"]
        );
        assert_eq!(
            values("SELECT from_song_id || '>' || to_song_id AS value FROM track_transition"),
            vec!["n1>n2"]
        );
        assert_eq!(
            values("SELECT album_id AS value FROM user_album_preference"),
            vec!["b1"]
        );
        assert_eq!(
            values("SELECT entity_id AS value FROM metadata_override ORDER BY entity_type"),
            vec!["b1", "n2"]
        );
        // The catalog is the indexer's business.
        assert_eq!(values("SELECT id AS value FROM track_entity"), vec!["t1"]);
    }
}
