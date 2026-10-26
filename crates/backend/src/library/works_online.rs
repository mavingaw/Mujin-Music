//! MusicBrainz enrichment for compositions.
//!
//! Files tagged by MusicBrainz Picard carry `MUSICBRAINZ_WORKID`. For each
//! such id this worker looks the work up once (type, composition date from
//! the composer relationship, parent work for movement-level ids) and each
//! composer once (life dates and country). Results land in
//! `musicbrainz_work` / `musicbrainz_composer`; the next work rebuild
//! ([`crate::library::works::ensure_works`]) picks them up.
//!
//! Requests follow the MusicBrainz API rules: an identifying User-Agent and
//! at most one request per second. Set `PARSON_MUSICBRAINZ=off` to disable
//! lookups.

use std::time::Duration;

use diesel::prelude::*;
use diesel::sql_types::{Nullable, Text};
use reqwest::Client;
use serde_json::Value;

use crate::library::credits::normalize_musicbrainz_id;
use crate::library::identity::normalize_artist_identity;
use crate::persistence::connection::DbPool;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

const API_BASE: &str = "https://musicbrainz.org/ws/2";
const REQUEST_SPACING: Duration = Duration::from_millis(1_100);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_BODY_BYTES: usize = 512 * 1024;
/// Lookups per pass, so a large library is enriched over several passes.
const LOOKUPS_PER_PASS: usize = 120;
const PASS_INTERVAL: Duration = Duration::from_secs(15 * 60);
/// Failed lookups are retried after this many days.
const RETRY_DAYS: i64 = 30;

/// What a MusicBrainz work lookup says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkLookup {
    pub title: Option<String>,
    pub work_type: Option<String>,
    pub composed_year: Option<i32>,
    pub parent_mbid: Option<String>,
    pub composer_mbids: Vec<String>,
}

/// What a MusicBrainz artist lookup says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArtistLookup {
    pub name: Option<String>,
    pub birth_year: Option<i32>,
    pub death_year: Option<i32>,
    pub country: Option<String>,
}

fn year_of(value: Option<&Value>) -> Option<i32> {
    value
        .and_then(Value::as_str)
        .and_then(|date| date.get(..4))
        .and_then(|year| year.parse::<i32>().ok())
        .filter(|year| (500..=2100).contains(year))
}

fn text_of(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|text| text.chars().take(300).collect())
}

/// Reads `GET /work/{id}?inc=artist-rels+work-rels`.
pub fn parse_work(json: &Value) -> WorkLookup {
    let mut lookup = WorkLookup {
        title: text_of(json.get("title")),
        work_type: text_of(json.get("type")),
        ..WorkLookup::default()
    };
    let relations = json
        .get("relations")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    for relation in relations {
        let kind = relation.get("type").and_then(Value::as_str).unwrap_or("");
        let direction = relation
            .get("direction")
            .and_then(Value::as_str)
            .unwrap_or("");
        match kind {
            "composer" => {
                // The composer relationship dates the composition.
                let year = year_of(relation.get("begin")).or_else(|| year_of(relation.get("end")));
                if let Some(year) = year {
                    lookup.composed_year =
                        Some(lookup.composed_year.map_or(year, |known| known.min(year)));
                }
                if let Some(id) = relation
                    .get("artist")
                    .and_then(|artist| artist.get("id"))
                    .and_then(Value::as_str)
                    .and_then(normalize_musicbrainz_id)
                    && !lookup.composer_mbids.contains(&id)
                {
                    lookup.composer_mbids.push(id);
                }
            }
            // "parts" backward: this work is a part of the related work.
            "parts" if direction == "backward" && lookup.parent_mbid.is_none() => {
                lookup.parent_mbid = relation
                    .get("work")
                    .and_then(|work| work.get("id"))
                    .and_then(Value::as_str)
                    .and_then(normalize_musicbrainz_id);
            }
            _ => {}
        }
    }
    lookup
}

/// Reads `GET /artist/{id}`.
pub fn parse_artist(json: &Value) -> ArtistLookup {
    let life = json.get("life-span");
    ArtistLookup {
        name: text_of(json.get("name")),
        birth_year: year_of(life.and_then(|life| life.get("begin"))),
        death_year: year_of(life.and_then(|life| life.get("end"))),
        country: text_of(json.get("country")).or_else(|| {
            json.get("area")
                .and_then(|area| area.get("iso-3166-1-codes"))
                .and_then(Value::as_array)
                .and_then(|codes| codes.first())
                .and_then(Value::as_str)
                .map(str::to_string)
        }),
    }
}

#[derive(QueryableByName)]
struct MbidRow {
    #[diesel(sql_type = Text)]
    mbid: String,
}

/// Work ids tagged in the library that have not been looked up (or failed
/// long enough ago), plus parents named by earlier lookups.
pub fn pending_works(connection: &mut SqliteConnection, limit: usize) -> QueryResult<Vec<String>> {
    diesel::sql_query(format!(
        "SELECT mbid FROM ( \
           SELECT DISTINCT work_mbid AS mbid FROM raw_file_metadata WHERE work_mbid IS NOT NULL \
           UNION SELECT parent_mbid FROM musicbrainz_work WHERE parent_mbid IS NOT NULL) \
         WHERE mbid NOT IN (SELECT mbid FROM musicbrainz_work \
           WHERE status = 'ok' OR fetched_at > datetime('now', '-{RETRY_DAYS} days')) \
         LIMIT {limit}"
    ))
    .load::<MbidRow>(connection)
    .map(|rows| rows.into_iter().map(|row| row.mbid).collect())
}

/// Composer ids named by work lookups that have no composer row yet.
pub fn pending_composers(
    connection: &mut SqliteConnection,
    limit: usize,
) -> QueryResult<Vec<String>> {
    diesel::sql_query(format!(
        "SELECT DISTINCT value AS mbid FROM musicbrainz_work, json_each(composer_mbids_json) \
         WHERE value NOT IN (SELECT mbid FROM musicbrainz_composer \
           WHERE status = 'ok' OR fetched_at > datetime('now', '-{RETRY_DAYS} days')) \
         LIMIT {limit}"
    ))
    .load::<MbidRow>(connection)
    .map(|rows| rows.into_iter().map(|row| row.mbid).collect())
}

pub fn store_work(
    connection: &mut SqliteConnection,
    mbid: &str,
    lookup: Option<&WorkLookup>,
) -> QueryResult<()> {
    let composers = lookup
        .map(|lookup| serde_json::to_string(&lookup.composer_mbids).unwrap_or_else(|_| "[]".into()))
        .unwrap_or_else(|| "[]".into());
    diesel::sql_query(
        "INSERT INTO musicbrainz_work(mbid, title, work_type, composed_year, parent_mbid, \
           composer_mbids_json, status, fetched_at) VALUES (?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP) \
         ON CONFLICT(mbid) DO UPDATE SET title = excluded.title, work_type = excluded.work_type, \
           composed_year = excluded.composed_year, parent_mbid = excluded.parent_mbid, \
           composer_mbids_json = excluded.composer_mbids_json, status = excluded.status, \
           fetched_at = excluded.fetched_at",
    )
    .bind::<Text, _>(mbid)
    .bind::<Nullable<Text>, _>(lookup.and_then(|lookup| lookup.title.as_deref()))
    .bind::<Nullable<Text>, _>(lookup.and_then(|lookup| lookup.work_type.as_deref()))
    .bind::<Nullable<diesel::sql_types::Integer>, _>(lookup.and_then(|lookup| lookup.composed_year))
    .bind::<Nullable<Text>, _>(lookup.and_then(|lookup| lookup.parent_mbid.as_deref()))
    .bind::<Text, _>(composers)
    .bind::<Text, _>(if lookup.is_some() { "ok" } else { "missing" })
    .execute(connection)
    .map(|_| ())
}

pub fn store_composer(
    connection: &mut SqliteConnection,
    mbid: &str,
    lookup: Option<&ArtistLookup>,
) -> QueryResult<()> {
    let normalized = lookup
        .and_then(|lookup| lookup.name.as_deref())
        .map(normalize_artist_identity);
    diesel::sql_query(
        "INSERT INTO musicbrainz_composer(mbid, name, normalized_name, birth_year, death_year, \
           country, status, fetched_at) VALUES (?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP) \
         ON CONFLICT(mbid) DO UPDATE SET name = excluded.name, \
           normalized_name = excluded.normalized_name, birth_year = excluded.birth_year, \
           death_year = excluded.death_year, country = excluded.country, \
           status = excluded.status, fetched_at = excluded.fetched_at",
    )
    .bind::<Text, _>(mbid)
    .bind::<Nullable<Text>, _>(lookup.and_then(|lookup| lookup.name.as_deref()))
    .bind::<Nullable<Text>, _>(normalized.as_deref())
    .bind::<Nullable<diesel::sql_types::Integer>, _>(lookup.and_then(|lookup| lookup.birth_year))
    .bind::<Nullable<diesel::sql_types::Integer>, _>(lookup.and_then(|lookup| lookup.death_year))
    .bind::<Nullable<Text>, _>(lookup.and_then(|lookup| lookup.country.as_deref()))
    .bind::<Text, _>(if lookup.is_some() { "ok" } else { "missing" })
    .execute(connection)
    .map(|_| ())
}

fn enabled() -> bool {
    !std::env::var("PARSON_MUSICBRAINZ").is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "off" | "false" | "no"
        )
    })
}

async fn fetch_json(client: &Client, url: &str) -> Result<Option<Value>, BoxError> {
    // Shares MusicBrainz' one request per second with the enrichment worker.
    crate::enrichment::fetch::shared_limiter()
        .acquire(crate::enrichment::fetch::url_host(url))
        .await;
    let response = client.get(url).send().await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let response = response.error_for_status()?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BODY_BYTES as u64)
    {
        return Err("MusicBrainz response is too large".into());
    }
    let body = response.bytes().await?;
    if body.len() > MAX_BODY_BYTES {
        return Err("MusicBrainz response is too large".into());
    }
    Ok(Some(serde_json::from_slice(&body)?))
}

async fn blocking<T: Send + 'static>(
    pool: &DbPool,
    task: impl FnOnce(&mut SqliteConnection) -> QueryResult<T> + Send + 'static,
) -> Result<T, BoxError> {
    let pool = pool.clone();
    tokio::task::spawn_blocking(move || {
        let mut connection = pool.get()?;
        task(&mut connection).map_err(BoxError::from)
    })
    .await?
}

/// Looks up pending works and composers. Returns how many lookups ran.
pub async fn enrich_once(pool: &DbPool, client: &Client) -> Result<usize, BoxError> {
    let mut done = 0;
    while done < LOOKUPS_PER_PASS {
        let works = blocking(pool, |connection| pending_works(connection, 20)).await?;
        let composers = blocking(pool, |connection| pending_composers(connection, 20)).await?;
        if works.is_empty() && composers.is_empty() {
            break;
        }
        for mbid in works {
            if done >= LOOKUPS_PER_PASS {
                break;
            }
            let url = format!("{API_BASE}/work/{mbid}?inc=artist-rels+work-rels&fmt=json");
            let lookup = fetch_json(client, &url)
                .await?
                .map(|json| parse_work(&json));
            blocking(pool, move |connection| {
                store_work(connection, &mbid, lookup.as_ref())
            })
            .await?;
            done += 1;
            tokio::time::sleep(REQUEST_SPACING).await;
        }
        for mbid in composers {
            if done >= LOOKUPS_PER_PASS {
                break;
            }
            let url = format!("{API_BASE}/artist/{mbid}?fmt=json");
            let lookup = fetch_json(client, &url)
                .await?
                .map(|json| parse_artist(&json));
            blocking(pool, move |connection| {
                store_composer(connection, &mbid, lookup.as_ref())
            })
            .await?;
            done += 1;
            tokio::time::sleep(REQUEST_SPACING).await;
        }
    }
    Ok(done)
}

/// Runs enrichment passes in the background while the server runs.
pub fn start_musicbrainz_enrichment(pool: DbPool) {
    if !enabled() {
        tracing::info!("MusicBrainz work lookups are disabled");
        return;
    }
    let client = match Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(REQUEST_TIMEOUT)
        .user_agent(concat!(
            "Parson/",
            env!("CARGO_PKG_VERSION"),
            " ( https://github.com/parson-music )"
        ))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!(%error, "MusicBrainz client could not start");
            return;
        }
    };
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(PASS_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            match enrich_once(&pool, &client).await {
                Ok(0) => {}
                Ok(lookups) => tracing::info!(lookups, "MusicBrainz work lookups completed"),
                Err(error) => tracing::debug!(%error, "MusicBrainz work lookups stopped"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use diesel::Connection;
    use diesel_migrations::MigrationHarness;
    use serde_json::json;

    use super::*;

    #[test]
    fn work_lookups_read_type_date_parent_and_composers() {
        let work = parse_work(&json!({
            "id": "c3ee2ae0-31dc-3c2f-9ee6-7d4a8e6a2fb2",
            "title": "Symphony no. 7 in A major, op. 92: II. Allegretto",
            "type": null,
            "relations": [
                {
                    "type": "composer", "direction": "backward", "begin": "1811", "end": "1812",
                    "artist": { "id": "1F9DF192-A621-4E4D-9A53-9A0E8B8A4B1E", "name": "Ludwig van Beethoven" }
                },
                {
                    "type": "parts", "direction": "backward",
                    "work": { "id": "3c37b9fa-a6c1-37d2-9e90-657a116d337c", "title": "Symphony no. 7" }
                },
                { "type": "parts", "direction": "forward", "work": { "id": "00000000-0000-0000-0000-000000000000" } }
            ]
        }));
        assert_eq!(work.work_type, None);
        assert_eq!(work.composed_year, Some(1811));
        assert_eq!(
            work.parent_mbid.as_deref(),
            Some("3c37b9fa-a6c1-37d2-9e90-657a116d337c")
        );
        assert_eq!(
            work.composer_mbids,
            ["1f9df192-a621-4e4d-9a53-9a0e8b8a4b1e"]
        );
    }

    #[test]
    fn artist_lookups_read_life_span_and_country() {
        let artist = parse_artist(&json!({
            "name": "Wolfgang Amadeus Mozart",
            "country": "AT",
            "life-span": { "begin": "1756-01-27", "end": "1791-12-05", "ended": true }
        }));
        assert_eq!(artist.birth_year, Some(1756));
        assert_eq!(artist.death_year, Some(1791));
        assert_eq!(artist.country.as_deref(), Some("AT"));
        let area_only = parse_artist(&json!({
            "name": "Unknown", "area": { "iso-3166-1-codes": ["FI"] }, "life-span": {}
        }));
        assert_eq!(area_only.country.as_deref(), Some("FI"));
        assert_eq!(area_only.birth_year, None);
    }

    #[test]
    fn lookups_are_queued_once_and_feed_the_work_rebuild() {
        let mut connection = SqliteConnection::establish(":memory:").expect("database");
        connection
            .run_pending_migrations(crate::persistence::connection::MIGRATIONS)
            .expect("migrations");
        use diesel::connection::SimpleConnection;
        connection
            .batch_execute(
                "INSERT INTO library_root(id, path) VALUES (1, '/m');
                 INSERT INTO file_entry(id, root_id, path, directory, file_name, extension, size_bytes, modified_at_ns)
                   VALUES (1, 1, '/m/a.flac', '/m', 'a.flac', 'flac', 1, 1);
                 INSERT INTO raw_file_metadata(file_id, work_mbid) VALUES (1, 'child');",
            )
            .expect("fixture");
        assert_eq!(
            pending_works(&mut connection, 10).expect("pending"),
            ["child"]
        );
        store_work(
            &mut connection,
            "child",
            Some(&WorkLookup {
                parent_mbid: Some("parent".into()),
                composer_mbids: vec!["composer".into()],
                ..WorkLookup::default()
            }),
        )
        .expect("store child");
        assert_eq!(
            pending_works(&mut connection, 10).expect("parent pending"),
            ["parent"]
        );
        // A missing work is not retried until the retry window passes.
        store_work(&mut connection, "parent", None).expect("store missing");
        assert!(pending_works(&mut connection, 10).expect("none").is_empty());
        assert_eq!(
            pending_composers(&mut connection, 10).expect("composer pending"),
            ["composer"]
        );
        store_composer(
            &mut connection,
            "composer",
            Some(&ArtistLookup {
                name: Some("Ludwig van Beethoven".into()),
                birth_year: Some(1770),
                death_year: Some(1827),
                country: Some("DE".into()),
            }),
        )
        .expect("store composer");
        assert!(
            pending_composers(&mut connection, 10)
                .expect("composers done")
                .is_empty()
        );
    }
}
