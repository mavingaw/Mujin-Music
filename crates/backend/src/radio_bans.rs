//! Per-listener bans: artists, albums and tracks that Radio, Shuffle and
//! recommendations must never pick. Bans are a hard exclusion that applies
//! on the very next pick; explicit plays (an album, a playlist, a search
//! result) still work, as in Roon.

use std::collections::HashSet;

use diesel::deserialize::QueryableByName;
use diesel::prelude::*;
use diesel::sql_types::{Integer, Text};
use diesel::sqlite::SqliteConnection;
use serde::{Deserialize, Serialize};

use crate::library::state::LibraryCache;

/// Most bans a listener can hold; keeps the per-pick lookup cheap.
pub const MAX_BANS_PER_USER: i64 = 5_000;

/// What a ban applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BanKind {
    Song,
    Album,
    Artist,
}

impl BanKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BanKind::Song => "song",
            BanKind::Album => "album",
            BanKind::Artist => "artist",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "song" | "track" => Some(BanKind::Song),
            "album" => Some(BanKind::Album),
            "artist" => Some(BanKind::Artist),
            _ => None,
        }
    }

    /// Whether `id` names an entity of this kind in the current catalog.
    pub fn exists(self, id: &str, cache: &LibraryCache) -> bool {
        match self {
            BanKind::Song => cache.song(id).is_some(),
            BanKind::Album => cache.album(id).is_some(),
            BanKind::Artist => cache.artist(id).is_some(),
        }
    }
}

/// One stored ban.
#[derive(Clone, Debug, PartialEq, QueryableByName)]
pub struct BanRow {
    #[diesel(sql_type = Text)]
    pub kind: String,
    #[diesel(sql_type = Text)]
    pub entity_id: String,
    #[diesel(sql_type = Text)]
    pub created_at: String,
}

/// A listener's bans, ready for per-song checks.
#[derive(Clone, Debug, Default)]
pub struct RadioBans {
    pub songs: HashSet<String>,
    pub albums: HashSet<String>,
    pub artists: HashSet<String>,
}

impl RadioBans {
    pub fn load(connection: &mut SqliteConnection, user_id: i32) -> QueryResult<Self> {
        let mut bans = RadioBans::default();
        for row in list_bans(connection, user_id)? {
            bans.insert(&row.kind, row.entity_id);
        }
        Ok(bans)
    }

    fn insert(&mut self, kind: &str, id: String) {
        match BanKind::parse(kind) {
            Some(BanKind::Song) => {
                self.songs.insert(id);
            }
            Some(BanKind::Album) => {
                self.albums.insert(id);
            }
            Some(BanKind::Artist) => {
                self.artists.insert(id);
            }
            None => {}
        }
    }

    pub fn is_empty(&self) -> bool {
        self.songs.is_empty() && self.albums.is_empty() && self.artists.is_empty()
    }

    pub fn len(&self) -> usize {
        self.songs.len() + self.albums.len() + self.artists.len()
    }

    /// True when the song itself, its album, its album artist or any
    /// credited artist is banned.
    pub fn excludes_song(&self, song_id: &str, cache: &LibraryCache) -> bool {
        if self.is_empty() {
            return false;
        }
        if self.songs.contains(song_id) {
            return true;
        }
        if let Some((artist_id, album_id)) = cache.song_map.get(song_id)
            && (self.artists.contains(artist_id) || self.albums.contains(album_id))
        {
            return true;
        }
        if self.artists.is_empty() {
            return false;
        }
        cache.song(song_id).is_some_and(|song| {
            song.contributing_artist_ids
                .iter()
                .any(|id| self.artists.contains(id))
        })
    }
}

pub fn list_bans(connection: &mut SqliteConnection, user_id: i32) -> QueryResult<Vec<BanRow>> {
    diesel::sql_query(
        "SELECT kind, entity_id, CAST(created_at AS TEXT) AS created_at
         FROM user_radio_ban WHERE user_id = ?
         ORDER BY created_at DESC, kind ASC, entity_id ASC LIMIT ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<diesel::sql_types::BigInt, _>(MAX_BANS_PER_USER)
    .load::<BanRow>(connection)
}

#[derive(Debug, PartialEq)]
pub enum AddBanError {
    TooMany,
    Database(diesel::result::Error),
}

impl From<diesel::result::Error> for AddBanError {
    fn from(error: diesel::result::Error) -> Self {
        AddBanError::Database(error)
    }
}

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    count: i64,
}

/// Adds a ban (idempotent) and returns the stored row.
pub fn add_ban(
    connection: &mut SqliteConnection,
    user_id: i32,
    kind: BanKind,
    entity_id: &str,
) -> Result<BanRow, AddBanError> {
    connection.immediate_transaction(|connection| {
        let count = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM user_radio_ban
             WHERE user_id = ? AND NOT (kind = ? AND entity_id = ?)",
        )
        .bind::<Integer, _>(user_id)
        .bind::<Text, _>(kind.as_str())
        .bind::<Text, _>(entity_id)
        .get_result::<CountRow>(connection)?
        .count;
        if count >= MAX_BANS_PER_USER {
            return Err(AddBanError::TooMany);
        }
        diesel::sql_query(
            "INSERT INTO user_radio_ban (user_id, kind, entity_id) VALUES (?, ?, ?)
             ON CONFLICT(user_id, kind, entity_id) DO NOTHING",
        )
        .bind::<Integer, _>(user_id)
        .bind::<Text, _>(kind.as_str())
        .bind::<Text, _>(entity_id)
        .execute(connection)?;
        Ok(diesel::sql_query(
            "SELECT kind, entity_id, CAST(created_at AS TEXT) AS created_at
             FROM user_radio_ban WHERE user_id = ? AND kind = ? AND entity_id = ?",
        )
        .bind::<Integer, _>(user_id)
        .bind::<Text, _>(kind.as_str())
        .bind::<Text, _>(entity_id)
        .get_result::<BanRow>(connection)?)
    })
}

/// Removes a ban. Returns whether one existed.
pub fn remove_ban(
    connection: &mut SqliteConnection,
    user_id: i32,
    kind: BanKind,
    entity_id: &str,
) -> QueryResult<bool> {
    diesel::sql_query("DELETE FROM user_radio_ban WHERE user_id = ? AND kind = ? AND entity_id = ?")
        .bind::<Integer, _>(user_id)
        .bind::<Text, _>(kind.as_str())
        .bind::<Text, _>(entity_id)
        .execute(connection)
        .map(|deleted| deleted > 0)
}

#[cfg(test)]
mod tests {
    use super::{AddBanError, BanKind, RadioBans, add_ban, list_bans, remove_ban};
    use crate::recommendation::test_support::{radio_pool, radio_test_cache};

    #[test]
    fn bans_are_per_listener_idempotent_and_removable() {
        let pool = radio_pool();
        let mut connection = pool.get().unwrap();
        let first = add_ban(&mut connection, 1, BanKind::Artist, "artist-jazz").unwrap();
        let again = add_ban(&mut connection, 1, BanKind::Artist, "artist-jazz").unwrap();
        assert_eq!(first, again);
        add_ban(&mut connection, 1, BanKind::Song, "r1").unwrap();
        add_ban(&mut connection, 2, BanKind::Album, "album-rock-2").unwrap();

        let mine = list_bans(&mut connection, 1).unwrap();
        assert_eq!(mine.len(), 2);
        assert_eq!(list_bans(&mut connection, 2).unwrap().len(), 1);

        assert!(remove_ban(&mut connection, 1, BanKind::Song, "r1").unwrap());
        assert!(!remove_ban(&mut connection, 1, BanKind::Song, "r1").unwrap());
        assert_eq!(list_bans(&mut connection, 1).unwrap().len(), 1);
        assert_eq!(list_bans(&mut connection, 2).unwrap().len(), 1);
    }

    #[test]
    fn a_ban_covers_the_song_its_album_and_its_artists() {
        let pool = radio_pool();
        let cache = radio_test_cache();
        let mut connection = pool.get().unwrap();
        add_ban(&mut connection, 1, BanKind::Song, "r1").unwrap();
        add_ban(&mut connection, 1, BanKind::Album, "album-rock-2").unwrap();
        add_ban(&mut connection, 1, BanKind::Artist, "artist-jazz").unwrap();
        let bans = RadioBans::load(&mut connection, 1).unwrap();
        assert_eq!(bans.len(), 3);
        assert!(bans.excludes_song("r1", &cache));
        assert!(!bans.excludes_song("r2", &cache));
        assert!(bans.excludes_song("r4", &cache));
        assert!(bans.excludes_song("r5", &cache));
        assert!(bans.excludes_song("j2", &cache));
        assert!(!bans.excludes_song("r3", &cache));

        let other = RadioBans::load(&mut connection, 2).unwrap();
        assert!(other.is_empty());
        assert!(!other.excludes_song("r1", &cache));
    }

    #[test]
    fn ban_kinds_parse_track_as_song() {
        assert_eq!(BanKind::parse("track"), Some(BanKind::Song));
        assert_eq!(BanKind::parse("song"), Some(BanKind::Song));
        assert_eq!(BanKind::parse("genre"), None);
        let cache = radio_test_cache();
        assert!(BanKind::Album.exists("album-jazz", &cache));
        assert!(!BanKind::Album.exists("r1", &cache));
        assert!(matches!(
            AddBanError::from(diesel::result::Error::NotFound),
            AddBanError::Database(_)
        ));
    }
}
