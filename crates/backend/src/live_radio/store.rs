//! My Live Radio and recently played stations, per listener.

use diesel::deserialize::QueryableByName;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Text};
use diesel::sqlite::SqliteConnection;

use super::station::Station;

/// Most stations one listener can save.
pub const MAX_SAVED: i64 = 1_000;
/// Recently played stations kept per listener.
pub const MAX_RECENT: i64 = 24;

#[derive(QueryableByName)]
struct StationRow {
    #[diesel(sql_type = Text)]
    station_json: String,
}

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

#[derive(Debug, PartialEq)]
pub enum SaveError {
    TooMany,
    Database(String),
}

impl From<diesel::result::Error> for SaveError {
    fn from(error: diesel::result::Error) -> Self {
        Self::Database(error.to_string())
    }
}

fn decode(rows: Vec<StationRow>) -> Vec<Station> {
    rows.into_iter()
        .filter_map(|row| serde_json::from_str(&row.station_json).ok())
        .collect()
}

/// Saves (or refreshes) a station in the listener's My Live Radio. New
/// stations go to the top.
pub fn save(
    connection: &mut SqliteConnection,
    user_id: i32,
    station: &Station,
) -> Result<(), SaveError> {
    let json =
        serde_json::to_string(station).map_err(|error| SaveError::Database(error.to_string()))?;
    connection.immediate_transaction(|connection| {
        let count = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM live_radio_station WHERE user_id = ? AND station_id <> ?",
        )
        .bind::<Integer, _>(user_id)
        .bind::<Text, _>(&station.id)
        .get_result::<CountRow>(connection)?
        .count;
        if count >= MAX_SAVED {
            return Err(SaveError::TooMany);
        }
        diesel::sql_query(
            "INSERT INTO live_radio_station (user_id, station_id, source, station_json, position)
             VALUES (?, ?, ?, ?, (SELECT COALESCE(MIN(position), 0) - 1 FROM live_radio_station WHERE user_id = ?))
             ON CONFLICT(user_id, station_id) DO UPDATE SET
               station_json = excluded.station_json,
               source = excluded.source,
               updated_at = CURRENT_TIMESTAMP",
        )
        .bind::<Integer, _>(user_id)
        .bind::<Text, _>(&station.id)
        .bind::<Text, _>(station.source.as_str())
        .bind::<Text, _>(&json)
        .bind::<Integer, _>(user_id)
        .execute(connection)?;
        Ok(())
    })
}

pub fn list_saved(connection: &mut SqliteConnection, user_id: i32) -> QueryResult<Vec<Station>> {
    diesel::sql_query(
        "SELECT station_json FROM live_radio_station WHERE user_id = ?
         ORDER BY position ASC, created_at DESC LIMIT ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<BigInt, _>(MAX_SAVED)
    .load::<StationRow>(connection)
    .map(decode)
}

#[derive(QueryableByName)]
struct IdRow {
    #[diesel(sql_type = Text)]
    station_id: String,
}

/// Ids of the listener's saved stations.
pub fn saved_ids(
    connection: &mut SqliteConnection,
    user_id: i32,
) -> QueryResult<std::collections::HashSet<String>> {
    diesel::sql_query("SELECT station_id FROM live_radio_station WHERE user_id = ?")
        .bind::<Integer, _>(user_id)
        .load::<IdRow>(connection)
        .map(|rows| rows.into_iter().map(|row| row.station_id).collect())
}

/// A saved station: the listener's own, or (for `None`) anyone's. Signed
/// zone URLs carry no listener, only proof that Parson queued the station.
pub fn find_saved(
    connection: &mut SqliteConnection,
    station_id: &str,
    user_id: Option<i32>,
) -> QueryResult<Option<Station>> {
    let rows = match user_id {
        Some(user_id) => diesel::sql_query(
            "SELECT station_json FROM live_radio_station WHERE user_id = ? AND station_id = ?",
        )
        .bind::<Integer, _>(user_id)
        .bind::<Text, _>(station_id)
        .load::<StationRow>(connection)?,
        None => diesel::sql_query(
            "SELECT station_json FROM live_radio_station WHERE station_id = ?
             ORDER BY source = 'custom' DESC, updated_at DESC LIMIT 1",
        )
        .bind::<Text, _>(station_id)
        .load::<StationRow>(connection)?,
    };
    Ok(decode(rows).into_iter().next())
}

pub fn remove(
    connection: &mut SqliteConnection,
    user_id: i32,
    station_id: &str,
) -> QueryResult<bool> {
    diesel::sql_query("DELETE FROM live_radio_station WHERE user_id = ? AND station_id = ?")
        .bind::<Integer, _>(user_id)
        .bind::<Text, _>(station_id)
        .execute(connection)
        .map(|deleted| deleted > 0)
}

/// Puts saved stations in the given order; ids not listed keep their
/// relative order after the listed ones.
pub fn reorder(
    connection: &mut SqliteConnection,
    user_id: i32,
    station_ids: &[String],
) -> QueryResult<()> {
    connection.immediate_transaction(|connection| {
        diesel::sql_query(
            "UPDATE live_radio_station SET position = position + 1000000 WHERE user_id = ?",
        )
        .bind::<Integer, _>(user_id)
        .execute(connection)?;
        for (index, id) in station_ids.iter().enumerate() {
            diesel::sql_query(
                "UPDATE live_radio_station SET position = ? WHERE user_id = ? AND station_id = ?",
            )
            .bind::<Integer, _>(index as i32)
            .bind::<Integer, _>(user_id)
            .bind::<Text, _>(id)
            .execute(connection)?;
        }
        Ok(())
    })
}

/// Records a play, keeping the newest [`MAX_RECENT`].
pub fn record_recent(
    connection: &mut SqliteConnection,
    user_id: i32,
    station: &Station,
) -> QueryResult<()> {
    let json = serde_json::to_string(station).unwrap_or_default();
    connection.immediate_transaction(|connection| {
        diesel::sql_query(
            "INSERT INTO live_radio_recent (user_id, station_id, station_json, played_at, play_order)
             VALUES (?, ?, ?, CURRENT_TIMESTAMP,
               (SELECT COALESCE(MAX(play_order), 0) + 1 FROM live_radio_recent WHERE user_id = ?))
             ON CONFLICT(user_id, station_id) DO UPDATE SET
               station_json = excluded.station_json,
               played_at = excluded.played_at,
               play_order = excluded.play_order",
        )
        .bind::<Integer, _>(user_id)
        .bind::<Text, _>(&station.id)
        .bind::<Text, _>(&json)
        .bind::<Integer, _>(user_id)
        .execute(connection)?;
        diesel::sql_query(
            "DELETE FROM live_radio_recent WHERE user_id = ? AND station_id NOT IN (
               SELECT station_id FROM live_radio_recent WHERE user_id = ?
               ORDER BY play_order DESC LIMIT ?)",
        )
        .bind::<Integer, _>(user_id)
        .bind::<Integer, _>(user_id)
        .bind::<BigInt, _>(MAX_RECENT)
        .execute(connection)?;
        Ok(())
    })
}

pub fn list_recent(connection: &mut SqliteConnection, user_id: i32) -> QueryResult<Vec<Station>> {
    diesel::sql_query(
        "SELECT station_json FROM live_radio_recent WHERE user_id = ?
         ORDER BY play_order DESC LIMIT ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<BigInt, _>(MAX_RECENT)
    .load::<StationRow>(connection)
    .map(decode)
}

pub fn clear_recent(connection: &mut SqliteConnection, user_id: i32) -> QueryResult<()> {
    diesel::sql_query("DELETE FROM live_radio_recent WHERE user_id = ?")
        .bind::<Integer, _>(user_id)
        .execute(connection)
        .map(|_| ())
}

#[derive(QueryableByName)]
struct VerifiedRow {
    #[diesel(sql_type = Text)]
    station_id: String,
    #[diesel(sql_type = Text)]
    format_json: String,
}

/// Remembers the format Parson read from a station's audio.
pub fn save_verified(
    connection: &mut SqliteConnection,
    station_id: &str,
    format: &super::relay::SourceFormat,
) -> QueryResult<()> {
    let json = serde_json::to_string(format).unwrap_or_default();
    diesel::sql_query(
        "INSERT INTO live_radio_verified (station_id, format_json, verified_at)
         VALUES (?, ?, CURRENT_TIMESTAMP)
         ON CONFLICT(station_id) DO UPDATE SET
           format_json = excluded.format_json,
           verified_at = excluded.verified_at",
    )
    .bind::<Text, _>(station_id)
    .bind::<Text, _>(&json)
    .execute(connection)
    .map(|_| ())
}

/// Formats Parson heard in the last 30 days, by station.
pub fn load_verified(
    connection: &mut SqliteConnection,
) -> QueryResult<std::collections::HashMap<String, super::relay::SourceFormat>> {
    diesel::sql_query(
        "SELECT station_id, format_json FROM live_radio_verified
         WHERE verified_at >= datetime('now', '-30 days')
         ORDER BY verified_at DESC LIMIT 20000",
    )
    .load::<VerifiedRow>(connection)
    .map(|rows| {
        rows.into_iter()
            .filter_map(|row| Some((row.station_id, serde_json::from_str(&row.format_json).ok()?)))
            .collect()
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use diesel::connection::SimpleConnection;
    use diesel::r2d2::{ConnectionManager, Pool};
    use diesel::sqlite::SqliteConnection;
    use diesel_migrations::MigrationHarness;

    use super::*;
    use crate::live_radio::station::StationSource;
    use crate::persistence::connection::DbPool;

    pub(crate) fn pool() -> DbPool {
        let pool = Arc::new(
            Pool::builder()
                .max_size(1)
                .build(ConnectionManager::<SqliteConnection>::new(":memory:"))
                .expect("pool"),
        );
        let mut connection = pool.get().expect("connection");
        connection
            .run_pending_migrations(crate::persistence::connection::MIGRATIONS)
            .expect("migrations");
        connection
            .batch_execute(
                "PRAGMA foreign_keys = ON;
                 INSERT INTO user(id, username, password, role) VALUES (1, 'ada', 'x', 'admin'), (2, 'grace', 'x', 'user');",
            )
            .expect("users");
        drop(connection);
        pool
    }

    pub(crate) fn station(id: &str, name: &str) -> Station {
        Station {
            id: id.into(),
            source: StationSource::Directory,
            name: name.into(),
            stream_url: format!("https://radio.example/{name}"),
            homepage: None,
            favicon: None,
            country: None,
            country_code: None,
            region: None,
            languages: Vec::new(),
            tags: Vec::new(),
            codec: Some("mp3".into()),
            bitrate_kbps: Some(128),
            hls: false,
            votes: None,
            clicks: None,
            click_trend: None,
            online: None,
            description: None,
            allow_lan: false,
        }
    }

    const A: &str = "d1a54d2e-623e-4970-ab11-35f7b56c5ec3";
    const B: &str = "e8f9ffdc-22a8-4c8b-82dd-ec63685feee6";
    const C: &str = "98adecf7-2683-4408-9be7-02d3f9098eb8";

    #[test]
    fn saved_stations_are_per_listener_newest_first_and_reorderable() {
        let pool = pool();
        let mut connection = pool.get().unwrap();
        save(&mut connection, 1, &station(A, "a")).unwrap();
        save(&mut connection, 1, &station(B, "b")).unwrap();
        save(&mut connection, 2, &station(C, "c")).unwrap();
        let names = |list: Vec<Station>| list.into_iter().map(|s| s.name).collect::<Vec<_>>();
        assert_eq!(names(list_saved(&mut connection, 1).unwrap()), ["b", "a"]);
        assert_eq!(names(list_saved(&mut connection, 2).unwrap()), ["c"]);

        // Saving again refreshes the snapshot without moving it.
        save(&mut connection, 1, &station(A, "a2")).unwrap();
        assert_eq!(names(list_saved(&mut connection, 1).unwrap()), ["b", "a2"]);

        reorder(&mut connection, 1, &[A.to_string(), B.to_string()]).unwrap();
        assert_eq!(names(list_saved(&mut connection, 1).unwrap()), ["a2", "b"]);

        assert!(saved_ids(&mut connection, 1).unwrap().contains(B));
        assert!(find_saved(&mut connection, C, Some(1)).unwrap().is_none());
        assert_eq!(
            find_saved(&mut connection, C, None).unwrap().unwrap().name,
            "c"
        );

        assert!(remove(&mut connection, 1, B).unwrap());
        assert!(!remove(&mut connection, 1, B).unwrap());
        assert_eq!(names(list_saved(&mut connection, 1).unwrap()), ["a2"]);
    }

    #[test]
    fn verified_formats_are_shared_and_updated() {
        use crate::live_radio::format::Container;
        use crate::live_radio::relay::SourceFormat;
        let pool = pool();
        let mut connection = pool.get().unwrap();
        let mut heard = SourceFormat {
            container: Container::Ogg,
            codec: Some("flac".into()),
            sample_rate: Some(44_100),
            bit_depth: Some(16),
            channels: Some(2),
            bitrate_kbps: None,
            verified: true,
        };
        save_verified(&mut connection, A, &heard).unwrap();
        heard.sample_rate = Some(48_000);
        save_verified(&mut connection, A, &heard).unwrap();
        let loaded = load_verified(&mut connection).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[A].sample_rate, Some(48_000));
    }

    #[test]
    fn recents_keep_the_latest_plays() {
        let pool = pool();
        let mut connection = pool.get().unwrap();
        for index in 0..(MAX_RECENT + 3) {
            let id = format!("00000000-0000-4000-8000-{index:012}");
            record_recent(&mut connection, 1, &station(&id, &format!("s{index}"))).unwrap();
        }
        record_recent(&mut connection, 1, &station(A, "again")).unwrap();
        let recent = list_recent(&mut connection, 1).unwrap();
        assert_eq!(recent.len() as i64, MAX_RECENT);
        assert_eq!(recent[0].name, "again");
        assert!(list_recent(&mut connection, 2).unwrap().is_empty());
        clear_recent(&mut connection, 1).unwrap();
        assert!(list_recent(&mut connection, 1).unwrap().is_empty());
    }
}
