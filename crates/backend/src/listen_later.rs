//! Listen Later: albums, tracks and artists a profile set aside.
//!
//! Items live in `listen_later_item`, one row per profile and entity. With
//! "remove once played" on, a qualified play takes its track off the list,
//! and an album leaves once most of its tracks have played since it was
//! added (a single track sampled from a long album does not count as
//! having listened to it). Artists are open-ended and stay until removed.

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Text};
use diesel::sqlite::SqliteConnection;
use serde::{Deserialize, Serialize};

use crate::library::state::LibraryCache;

/// Items one profile may keep on the list.
pub const MAX_LISTEN_LATER_ITEMS: i64 = 5_000;
/// Ids accepted in one add or remove call.
pub const MAX_LISTEN_LATER_BATCH: usize = 500;
/// Share of an album's tracks that must play before it counts as played.
pub const ALBUM_PLAYED_SHARE: f64 = 0.8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListenLaterEntity {
    Album,
    Track,
    Artist,
}

impl ListenLaterEntity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Album => "album",
            Self::Track => "track",
            Self::Artist => "artist",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "album" => Some(Self::Album),
            "track" => Some(Self::Track),
            "artist" => Some(Self::Artist),
            _ => None,
        }
    }

    /// Whether the library currently has this entity.
    pub fn exists(self, cache: &LibraryCache, id: &str) -> bool {
        match self {
            Self::Album => cache.album(id).is_some(),
            Self::Track => cache.song(id).is_some(),
            Self::Artist => cache.artist(id).is_some(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, QueryableByName)]
pub struct ListenLaterRow {
    #[diesel(sql_type = Text)]
    pub entity_type: String,
    #[diesel(sql_type = Text)]
    pub entity_id: String,
    #[diesel(sql_type = Text)]
    pub added_at: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct AddOutcome {
    pub added: usize,
    /// Ids that were not added because the list is full.
    pub capacity_reached: bool,
}

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

/// Every item on a profile's list, newest first.
pub fn list(
    connection: &mut SqliteConnection,
    user_id: i32,
) -> Result<Vec<ListenLaterRow>, diesel::result::Error> {
    diesel::sql_query(
        "SELECT entity_type, entity_id, \
         strftime('%Y-%m-%dT%H:%M:%SZ', added_at) AS added_at \
         FROM listen_later_item WHERE user_id = ? \
         ORDER BY added_at DESC, entity_type, entity_id LIMIT ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<BigInt, _>(MAX_LISTEN_LATER_ITEMS)
    .load(connection)
}

/// Adds entities the library has; ids already on the list keep their date.
pub fn add(
    connection: &mut SqliteConnection,
    user_id: i32,
    entity: ListenLaterEntity,
    ids: &[String],
    cache: &LibraryCache,
) -> Result<AddOutcome, diesel::result::Error> {
    connection.immediate_transaction(|connection| {
        let mut count =
            diesel::sql_query("SELECT COUNT(*) AS count FROM listen_later_item WHERE user_id = ?")
                .bind::<Integer, _>(user_id)
                .get_result::<CountRow>(connection)?
                .count;
        let mut outcome = AddOutcome {
            added: 0,
            capacity_reached: false,
        };
        for id in ids {
            if !entity.exists(cache, id) {
                continue;
            }
            if count >= MAX_LISTEN_LATER_ITEMS {
                outcome.capacity_reached = true;
                break;
            }
            let inserted = diesel::sql_query(
                "INSERT INTO listen_later_item(user_id, entity_type, entity_id) VALUES (?, ?, ?) \
                 ON CONFLICT(user_id, entity_type, entity_id) DO NOTHING",
            )
            .bind::<Integer, _>(user_id)
            .bind::<Text, _>(entity.as_str())
            .bind::<Text, _>(id)
            .execute(connection)?;
            outcome.added += inserted;
            count += inserted as i64;
        }
        Ok(outcome)
    })
}

pub fn remove(
    connection: &mut SqliteConnection,
    user_id: i32,
    entity: ListenLaterEntity,
    ids: &[String],
) -> Result<usize, diesel::result::Error> {
    connection.immediate_transaction(|connection| {
        let mut removed = 0;
        for id in ids {
            removed += diesel::sql_query(
                "DELETE FROM listen_later_item \
                 WHERE user_id = ? AND entity_type = ? AND entity_id = ?",
            )
            .bind::<Integer, _>(user_id)
            .bind::<Text, _>(entity.as_str())
            .bind::<Text, _>(id)
            .execute(connection)?;
        }
        Ok(removed)
    })
}

/// Which of the given ids are on the list.
pub fn saved_ids(
    connection: &mut SqliteConnection,
    user_id: i32,
    entity: ListenLaterEntity,
    ids: &[String],
) -> Result<Vec<String>, diesel::result::Error> {
    #[derive(QueryableByName)]
    struct IdRow {
        #[diesel(sql_type = Text)]
        entity_id: String,
    }
    let mut saved = Vec::new();
    for chunk in ids.chunks(200) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let mut query = diesel::sql_query(format!(
            "SELECT entity_id FROM listen_later_item \
             WHERE user_id = ? AND entity_type = ? AND entity_id IN ({placeholders})"
        ))
        .into_boxed()
        .bind::<Integer, _>(user_id)
        .bind::<Text, _>(entity.as_str());
        for id in chunk {
            query = query.bind::<Text, _>(id.clone());
        }
        saved.extend(
            query
                .load::<IdRow>(connection)?
                .into_iter()
                .map(|row| row.entity_id),
        );
    }
    Ok(saved)
}

pub fn remove_when_played(
    connection: &mut SqliteConnection,
    user_id: i32,
) -> Result<bool, diesel::result::Error> {
    #[derive(QueryableByName)]
    struct SettingRow {
        #[diesel(sql_type = Integer)]
        remove_when_played: i32,
    }
    Ok(
        diesel::sql_query("SELECT remove_when_played FROM listen_later_setting WHERE user_id = ?")
            .bind::<Integer, _>(user_id)
            .get_result::<SettingRow>(connection)
            .optional()?
            .is_some_and(|row| row.remove_when_played != 0),
    )
}

pub fn set_remove_when_played(
    connection: &mut SqliteConnection,
    user_id: i32,
    enabled: bool,
) -> Result<(), diesel::result::Error> {
    diesel::sql_query(
        "INSERT INTO listen_later_setting(user_id, remove_when_played) VALUES (?, ?) \
         ON CONFLICT(user_id) DO UPDATE SET remove_when_played = excluded.remove_when_played",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Integer, _>(i32::from(enabled))
    .execute(connection)?;
    Ok(())
}

/// Plays of an album needed before it counts as played.
pub fn album_plays_needed(track_count: usize) -> i64 {
    ((track_count as f64 * ALBUM_PLAYED_SHARE).ceil() as i64).max(1)
}

/// Takes played music off the list when the profile asked for that. Runs
/// after the play is in `listen_history_item`.
pub fn remove_played(
    connection: &mut SqliteConnection,
    user_id: i32,
    song_id: &str,
    cache: &LibraryCache,
) -> Result<(), diesel::result::Error> {
    if !remove_when_played(connection, user_id)? {
        return Ok(());
    }
    diesel::sql_query(
        "DELETE FROM listen_later_item WHERE user_id = ? AND entity_type = 'track' AND entity_id = ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(song_id)
    .execute(connection)?;
    let Some((_, album_id)) = cache.song_map.get(song_id) else {
        return Ok(());
    };
    let Some(album) = cache.album(album_id) else {
        return Ok(());
    };
    let played = diesel::sql_query(
        "SELECT COUNT(DISTINCT h.song_id) AS count FROM listen_later_item l \
         JOIN listen_history_item h ON h.user_id = l.user_id AND h.album_id = l.entity_id \
           AND h.listened_at >= l.added_at \
         WHERE l.user_id = ? AND l.entity_type = 'album' AND l.entity_id = ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(album_id)
    .get_result::<CountRow>(connection)?
    .count;
    if played >= album_plays_needed(album.songs.len()) {
        diesel::sql_query(
            "DELETE FROM listen_later_item WHERE user_id = ? AND entity_type = 'album' AND entity_id = ?",
        )
        .bind::<Integer, _>(user_id)
        .bind::<Text, _>(album_id)
        .execute(connection)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recommendation::test_support::{radio_pool, radio_test_cache};
    use crate::recommendation::{PlaybackEventRequest, record_playback_event};

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn play(pool: &crate::persistence::connection::DbPool, cache: &LibraryCache, song: &str) {
        record_playback_event(
            1,
            &PlaybackEventRequest {
                event_key: format!("play-{song}-{}", uuid::Uuid::new_v4()),
                song_id: song.into(),
                event_type: "qualified_play".into(),
                ..Default::default()
            },
            cache,
            pool,
        )
        .expect("play");
    }

    fn saved(connection: &mut SqliteConnection) -> Vec<(String, String)> {
        list(connection, 1)
            .expect("list")
            .into_iter()
            .map(|row| (row.entity_type, row.entity_id))
            .collect()
    }

    #[test]
    fn adding_keeps_library_entities_once_per_profile() {
        let pool = radio_pool();
        let cache = radio_test_cache();
        let mut connection = pool.get().expect("connection");
        let track = cache.songs_flat[0].clone();
        let outcome = add(
            &mut connection,
            1,
            ListenLaterEntity::Track,
            &[track.clone(), "missing".into(), track.clone()],
            &cache,
        )
        .expect("add");
        assert_eq!(
            outcome,
            AddOutcome {
                added: 1,
                capacity_reached: false
            }
        );
        assert_eq!(saved(&mut connection), [("track".into(), track.clone())]);
        assert!(list(&mut connection, 2).expect("other").is_empty());
        assert_eq!(
            saved_ids(
                &mut connection,
                1,
                ListenLaterEntity::Track,
                &[track.clone(), "x".into()]
            )
            .expect("saved"),
            std::slice::from_ref(&track)
        );
        assert_eq!(
            remove(
                &mut connection,
                1,
                ListenLaterEntity::Track,
                &ids(&[&track])
            )
            .expect("rm"),
            1
        );
        assert!(saved(&mut connection).is_empty());
    }

    #[test]
    fn played_music_leaves_the_list_only_when_asked() {
        let pool = radio_pool();
        let cache = radio_test_cache();
        let (_, album_id) = cache.song_map[&cache.songs_flat[0]].clone();
        let album_songs = cache
            .album(&album_id)
            .expect("album")
            .songs
            .iter()
            .map(|song| song.id.clone())
            .collect::<Vec<_>>();
        let other = cache
            .songs_flat
            .iter()
            .find(|song| !album_songs.contains(song))
            .expect("a track on another album")
            .clone();
        {
            let mut connection = pool.get().expect("connection");
            add(
                &mut connection,
                1,
                ListenLaterEntity::Album,
                std::slice::from_ref(&album_id),
                &cache,
            )
            .expect("album");
            add(
                &mut connection,
                1,
                ListenLaterEntity::Track,
                std::slice::from_ref(&other),
                &cache,
            )
            .expect("track");
        }
        play(&pool, &cache, &other);
        assert_eq!(saved(&mut pool.get().unwrap()).len(), 2, "setting is off");

        set_remove_when_played(&mut pool.get().unwrap(), 1, true).expect("setting");
        play(&pool, &cache, &other);
        let remaining = saved(&mut pool.get().unwrap());
        assert_eq!(remaining, [("album".into(), album_id.clone())]);

        let needed = album_plays_needed(album_songs.len()) as usize;
        for song in album_songs.iter().take(needed.saturating_sub(1)) {
            play(&pool, &cache, song);
        }
        if needed > 1 {
            assert_eq!(
                saved(&mut pool.get().unwrap()).len(),
                1,
                "album not done yet"
            );
        }
        play(&pool, &cache, &album_songs[needed - 1]);
        assert!(saved(&mut pool.get().unwrap()).is_empty());
    }

    #[test]
    fn album_needs_most_of_its_tracks() {
        assert_eq!(album_plays_needed(0), 1);
        assert_eq!(album_plays_needed(1), 1);
        assert_eq!(album_plays_needed(5), 4);
        assert_eq!(album_plays_needed(12), 10);
    }
}
