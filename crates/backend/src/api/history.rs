//! Listening history: a per-profile timeline of qualified plays with the
//! zone, device and source each one came from, listening statistics over
//! any period, and a CSV export. Mounted under `/users`.
//!
//! Every play carries a snapshot of its title, artist and album (see
//! `insert_listen_history` in [`crate::recommendation`]), so history stays
//! readable after files are renamed or leave the library; rows are joined
//! to the live library only to add artwork and current names.

use actix_web::{HttpRequest, HttpResponse, delete, get, web};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use diesel::prelude::*;
use diesel::query_builder::BoxedSqlQuery;
use diesel::sql_types::{BigInt, Double, Integer, Nullable, Text};
use diesel::sqlite::{Sqlite, SqliteConnection};
use serde::{Deserialize, Serialize};

use crate::api::auth::authenticated_user_id;
use crate::api::error::{bad_request, internal_server_error, not_found};
use crate::app::audio_quality;
use crate::library::state::{LibraryCache, LibraryLifecycle};
use crate::persistence::connection::DbPool;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

const MAX_TIMELINE_PAGE: i64 = 200;
const MAX_TOP: i64 = 50;
const MAX_TEXT_CHARACTERS: usize = 200;
const MAX_ID_CHARACTERS: usize = 256;
/// Largest offset from UTC in minutes (UTC+14:00).
const MAX_TZ_MINUTES: i32 = 14 * 60;
/// Zone filter value for plays recorded before zones were tracked.
pub const UNKNOWN_ZONE: &str = "none";
const DATETIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

fn worker_failed(context: &str, error: impl std::fmt::Display) -> HttpResponse {
    tracing::error!(%error, "{context}");
    internal_server_error("Could not load history.", "history_request_failed")
}

#[derive(Clone, Debug)]
enum Bind {
    Text(String),
    Int(i64),
}

/// A `WHERE` clause over `listen_history_item h` with its parameters.
#[derive(Clone, Debug, Default)]
struct Clause {
    sql: String,
    binds: Vec<Bind>,
}

impl Clause {
    fn and(&mut self, sql: &str, binds: impl IntoIterator<Item = Bind>) {
        self.sql.push_str(" AND ");
        self.sql.push_str(sql);
        self.binds.extend(binds);
    }

    fn query(
        &self,
        prefix: &str,
        suffix: &str,
    ) -> BoxedSqlQuery<'static, Sqlite, diesel::query_builder::SqlQuery> {
        self.query_with(prefix, suffix, Vec::new())
    }

    fn query_with(
        &self,
        prefix: &str,
        suffix: &str,
        trailing: Vec<Bind>,
    ) -> BoxedSqlQuery<'static, Sqlite, diesel::query_builder::SqlQuery> {
        let mut query = diesel::sql_query(format!("{prefix} WHERE {} {suffix}", self.sql))
            .into_boxed::<Sqlite>();
        for bind in self.binds.iter().cloned().chain(trailing) {
            query = match bind {
                Bind::Text(value) => query.bind::<Text, _>(value),
                Bind::Int(value) => query.bind::<BigInt, _>(value),
            };
        }
        query
    }
}

/// Filters shared by the timeline, statistics and export.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct HistoryFilter {
    /// Start of the period (RFC 3339 instant or `YYYY-MM-DD`, UTC).
    #[serde(default)]
    pub from: Option<String>,
    /// End of the period, exclusive.
    #[serde(default)]
    pub to: Option<String>,
    /// A zone id, or `none` for plays with no zone recorded.
    #[serde(default)]
    pub zone: Option<String>,
    /// `manual` (chosen by the listener), `generated` (Radio), ...
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub artist_id: Option<String>,
    #[serde(default)]
    pub album_id: Option<String>,
    #[serde(default)]
    pub song_id: Option<String>,
    /// Text matched against title, artist and album.
    #[serde(default)]
    pub q: Option<String>,
    /// Viewer's offset from UTC in minutes, for days and hours.
    #[serde(default)]
    pub tz: Option<i32>,
    // Paging and shape options, kept here because query strings cannot be
    // flattened into nested structs with typed numbers.
    /// Timeline cursor: the last entry already shown.
    #[serde(default)]
    pub before_id: Option<i32>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub bucket: Option<Bucket>,
    #[serde(default)]
    pub top: Option<i64>,
}

/// Parses an instant into the stored `YYYY-MM-DD HH:MM:SS` UTC form.
pub fn parse_instant(value: &str) -> Option<String> {
    let value = value.trim();
    if let Ok(instant) = DateTime::parse_from_rfc3339(value) {
        return Some(
            instant
                .with_timezone(&Utc)
                .naive_utc()
                .format(DATETIME_FORMAT)
                .to_string(),
        );
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(value, DATETIME_FORMAT) {
        return Some(naive.format(DATETIME_FORMAT).to_string());
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|naive| naive.format(DATETIME_FORMAT).to_string())
}

fn like_pattern(text: &str) -> String {
    let mut pattern = String::from("%");
    for character in text.trim().chars() {
        if matches!(character, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(character);
    }
    pattern.push('%');
    pattern
}

impl HistoryFilter {
    fn tz_minutes(&self) -> i32 {
        self.tz.unwrap_or(0).clamp(-MAX_TZ_MINUTES, MAX_TZ_MINUTES)
    }

    /// SQLite date modifier that moves UTC into the viewer's time zone.
    fn tz_modifier(&self) -> String {
        format!("{:+} minutes", self.tz_minutes())
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        for value in [&self.from, &self.to].into_iter().flatten() {
            if parse_instant(value).is_none() {
                return Err("history_period_invalid");
            }
        }
        if let (Some(from), Some(to)) = (
            self.from.as_deref().and_then(parse_instant),
            self.to.as_deref().and_then(parse_instant),
        ) && from > to
        {
            return Err("history_period_invalid");
        }
        if self
            .q
            .as_deref()
            .is_some_and(|text| text.chars().count() > MAX_TEXT_CHARACTERS)
        {
            return Err("history_query_invalid");
        }
        if [
            &self.zone,
            &self.source,
            &self.device,
            &self.artist_id,
            &self.album_id,
            &self.song_id,
        ]
        .into_iter()
        .flatten()
        .any(|value| value.is_empty() || value.chars().count() > MAX_ID_CHARACTERS)
        {
            return Err("history_filter_invalid");
        }
        if self.tz.is_some_and(|tz| tz.abs() > MAX_TZ_MINUTES) {
            return Err("history_tz_invalid");
        }
        Ok(())
    }

    fn clause(&self, user_id: i32) -> Clause {
        let mut clause = Clause {
            sql: "h.user_id = ?".into(),
            binds: vec![Bind::Int(i64::from(user_id))],
        };
        if let Some(from) = self.from.as_deref().and_then(parse_instant) {
            clause.and("h.listened_at >= ?", [Bind::Text(from)]);
        }
        if let Some(to) = self.to.as_deref().and_then(parse_instant) {
            clause.and("h.listened_at < ?", [Bind::Text(to)]);
        }
        match self.zone.as_deref() {
            Some(UNKNOWN_ZONE) => clause.and("h.zone_id IS NULL", []),
            Some(zone) => clause.and("h.zone_id = ?", [Bind::Text(zone.into())]),
            None => {}
        }
        if let Some(source) = &self.source {
            clause.and(
                "COALESCE(h.source, 'unknown') = ?",
                [Bind::Text(source.clone())],
            );
        }
        if let Some(device) = &self.device {
            clause.and("h.device = ?", [Bind::Text(device.clone())]);
        }
        if let Some(artist) = &self.artist_id {
            clause.and("h.artist_id = ?", [Bind::Text(artist.clone())]);
        }
        if let Some(album) = &self.album_id {
            clause.and("h.album_id = ?", [Bind::Text(album.clone())]);
        }
        if let Some(song) = &self.song_id {
            clause.and("h.song_id = ?", [Bind::Text(song.clone())]);
        }
        if let Some(text) = self.q.as_deref().filter(|text| !text.trim().is_empty()) {
            let pattern = like_pattern(text);
            clause.and(
                "(h.title LIKE ? ESCAPE '\\' OR h.artist_name LIKE ? ESCAPE '\\' \
                 OR h.album_name LIKE ? ESCAPE '\\')",
                [
                    Bind::Text(pattern.clone()),
                    Bind::Text(pattern.clone()),
                    Bind::Text(pattern),
                ],
            );
        }
        clause
    }
}

// ---------------------------------------------------------------------------
// Timeline
// ---------------------------------------------------------------------------

#[derive(Debug, QueryableByName)]
struct HistoryRow {
    #[diesel(sql_type = Integer)]
    id: i32,
    #[diesel(sql_type = Text)]
    song_id: String,
    #[diesel(sql_type = Text)]
    listened_at: String,
    #[diesel(sql_type = Nullable<Text>)]
    title: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    artist_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    artist_name: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    album_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    album_name: Option<String>,
    #[diesel(sql_type = Nullable<Double>)]
    duration_seconds: Option<f64>,
    #[diesel(sql_type = Nullable<Text>)]
    zone_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    zone_name: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    source: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    device: Option<String>,
}

const ROW_COLUMNS: &str = "h.id, h.song_id, \
    strftime('%Y-%m-%dT%H:%M:%SZ', h.listened_at) AS listened_at, h.title, h.artist_id, \
    h.artist_name, h.album_id, h.album_name, h.duration_seconds, h.zone_id, h.zone_name, \
    h.source, h.device";

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: i32,
    pub song_id: String,
    pub listened_at: String,
    pub title: String,
    pub artist_id: Option<String>,
    pub artist_name: String,
    pub album_id: Option<String>,
    pub album_name: Option<String>,
    pub cover_path: Option<String>,
    pub duration_seconds: f64,
    pub zone_id: Option<String>,
    pub zone_name: Option<String>,
    pub source: Option<String>,
    pub device: Option<String>,
    /// The track is still in the library and can be played.
    pub available: bool,
    pub quality: Option<&'static str>,
}

fn entry(row: HistoryRow, cache: Option<&LibraryCache>) -> HistoryEntry {
    let song = cache.and_then(|cache| cache.song(&row.song_id));
    let ids = cache.and_then(|cache| cache.song_map.get(&row.song_id));
    let album_id = ids.map(|(_, album)| album.clone()).or(row.album_id);
    let artist_id = ids.map(|(artist, _)| artist.clone()).or(row.artist_id);
    let album = cache
        .zip(album_id.as_deref())
        .and_then(|(cache, id)| cache.album(id));
    let artist = cache
        .zip(artist_id.as_deref())
        .and_then(|(cache, id)| cache.artist(id));
    HistoryEntry {
        id: row.id,
        title: song
            .map(|song| song.name.clone())
            .or(row.title)
            .unwrap_or_else(|| "Unknown track".into()),
        artist_name: artist
            .map(|artist| artist.name.clone())
            .or(row.artist_name)
            .or_else(|| song.map(|song| song.artist.clone()))
            .unwrap_or_else(|| "Unknown artist".into()),
        album_name: album.map(|album| album.name.clone()).or(row.album_name),
        cover_path: album
            .map(|album| album.cover_url.clone())
            .filter(|cover| !cover.trim().is_empty()),
        duration_seconds: song
            .map(|song| song.duration)
            .or(row.duration_seconds)
            .filter(|duration| duration.is_finite())
            .unwrap_or(0.0)
            .max(0.0),
        quality: song.and_then(|song| audio_quality(song.audio.as_ref())),
        available: song.is_some(),
        song_id: row.song_id,
        listened_at: row.listened_at,
        artist_id,
        album_id,
        zone_id: row.zone_id,
        zone_name: row.zone_name,
        source: row.source,
        device: row.device,
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryTimeline {
    pub items: Vec<HistoryEntry>,
    /// Cursor for the next (older) page, absent at the end.
    pub next_before_id: Option<i32>,
}

fn load_timeline(
    connection: &mut SqliteConnection,
    user_id: i32,
    filter: &HistoryFilter,
    before_id: Option<i32>,
    limit: i64,
) -> Result<Vec<HistoryRow>, diesel::result::Error> {
    let mut clause = filter.clause(user_id);
    // Newest play first. The cursor is the last row seen; rows imported
    // out of order still page by when they were heard.
    if let Some(before) = before_id {
        let before = i64::from(before);
        clause.and(
            "(h.listened_at < (SELECT c.listened_at FROM listen_history_item c WHERE c.id = ?) \
             OR (h.listened_at = (SELECT c.listened_at FROM listen_history_item c WHERE c.id = ?) \
                 AND h.id < ?))",
            [Bind::Int(before), Bind::Int(before), Bind::Int(before)],
        );
    }
    clause
        .query_with(
            &format!("SELECT {ROW_COLUMNS} FROM listen_history_item h"),
            "ORDER BY h.listened_at DESC, h.id DESC LIMIT ?",
            vec![Bind::Int(limit)],
        )
        .load::<HistoryRow>(connection)
}

async fn optional_cache(lifecycle: &LibraryLifecycle) -> Option<std::sync::Arc<LibraryCache>> {
    lifecycle.cache().await.ok()
}

fn invalid(code: &'static str) -> HttpResponse {
    bad_request("The history filter is invalid.", code)
}

#[get("/me/history/timeline")]
async fn history_timeline(
    request: HttpRequest,
    query: web::Query<HistoryFilter>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let filter = query.into_inner();
    if let Err(code) = filter.validate() {
        return invalid(code);
    }
    let limit = filter.limit.unwrap_or(60).clamp(1, MAX_TIMELINE_PAGE);
    let cache = optional_cache(&lifecycle).await;
    let pool = pool.get_ref().clone();
    match web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        let rows = load_timeline(
            &mut connection,
            user_id,
            &filter,
            filter.before_id,
            limit + 1,
        )?;
        let more = rows.len() as i64 > limit;
        let items = rows
            .into_iter()
            .take(limit as usize)
            .map(|row| entry(row, cache.as_deref()))
            .collect::<Vec<_>>();
        let next_before_id = more.then(|| items.last().map(|item| item.id)).flatten();
        Ok(HistoryTimeline {
            items,
            next_before_id,
        })
    })
    .await
    {
        Ok(Ok(timeline)) => HttpResponse::Ok().json(timeline),
        Ok(Err(error)) => worker_failed("history timeline failed", error),
        Err(error) => worker_failed("history timeline worker failed", error),
    }
}

// ---------------------------------------------------------------------------
// Statistics
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Bucket {
    #[default]
    Day,
    Week,
    Month,
    Year,
}

impl Bucket {
    /// SQL expression over `h.listened_at` naming the bucket a play falls in,
    /// in the viewer's time zone.
    fn expression(self) -> &'static str {
        match self {
            Self::Day => "date(h.listened_at, ?)",
            // Weeks start on Monday.
            Self::Week => "date(h.listened_at, ?, '-6 days', 'weekday 1')",
            Self::Month => "strftime('%Y-%m', h.listened_at, ?)",
            Self::Year => "strftime('%Y', h.listened_at, ?)",
        }
    }
}

#[derive(Debug, Default, Serialize, PartialEq, QueryableByName)]
#[serde(rename_all = "camelCase")]
pub struct HistoryTotals {
    #[diesel(sql_type = BigInt)]
    pub plays: i64,
    #[diesel(sql_type = Double)]
    pub listened_seconds: f64,
    #[diesel(sql_type = BigInt)]
    pub tracks: i64,
    #[diesel(sql_type = BigInt)]
    pub artists: i64,
    #[diesel(sql_type = BigInt)]
    pub albums: i64,
    #[diesel(sql_type = BigInt)]
    pub active_days: i64,
    #[diesel(sql_type = Nullable<Text>)]
    pub first_play: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub last_play: Option<String>,
}

#[derive(Debug, QueryableByName)]
struct RankRow {
    #[diesel(sql_type = Text)]
    id: String,
    #[diesel(sql_type = Nullable<Text>)]
    name: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    detail: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    detail_id: Option<String>,
    #[diesel(sql_type = BigInt)]
    plays: i64,
    #[diesel(sql_type = Double)]
    seconds: f64,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RankedItem {
    pub id: String,
    pub name: String,
    /// Artist name for albums and tracks.
    pub detail: Option<String>,
    pub detail_id: Option<String>,
    pub image_path: Option<String>,
    pub plays: i64,
    pub listened_seconds: f64,
    pub available: bool,
}

#[derive(Debug, Serialize, PartialEq, QueryableByName)]
#[serde(rename_all = "camelCase")]
pub struct BucketCount {
    #[diesel(sql_type = Text)]
    pub bucket: String,
    #[diesel(sql_type = BigInt)]
    pub plays: i64,
    #[diesel(sql_type = Double)]
    pub listened_seconds: f64,
}

#[derive(Debug, Serialize, PartialEq, QueryableByName)]
#[serde(rename_all = "camelCase")]
pub struct GroupCount {
    #[diesel(sql_type = Text)]
    pub value: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub label: Option<String>,
    #[diesel(sql_type = BigInt)]
    pub plays: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryStats {
    pub totals: HistoryTotals,
    /// Totals for the period of the same length just before, when the
    /// request names both ends of its period.
    pub previous: Option<HistoryTotals>,
    pub bucket: Bucket,
    pub timeline: Vec<BucketCount>,
    /// Plays per hour of day (0-23) in the viewer's time zone.
    pub hours: Vec<i64>,
    /// Plays per weekday, Monday first.
    pub weekdays: Vec<i64>,
    pub top_artists: Vec<RankedItem>,
    pub top_albums: Vec<RankedItem>,
    pub top_tracks: Vec<RankedItem>,
    pub zones: Vec<GroupCount>,
    pub sources: Vec<GroupCount>,
}

fn totals(
    connection: &mut SqliteConnection,
    clause: &Clause,
    tz: &str,
) -> Result<HistoryTotals, diesel::result::Error> {
    let mut with_tz = Clause {
        sql: clause.sql.clone(),
        binds: vec![Bind::Text(tz.into())],
    };
    with_tz.binds.extend(clause.binds.iter().cloned());
    with_tz
        .query(
            "SELECT COUNT(*) AS plays, \
             CAST(COALESCE(SUM(COALESCE(h.duration_seconds, 0)), 0) AS REAL) AS listened_seconds, \
             COUNT(DISTINCT h.song_id) AS tracks, COUNT(DISTINCT h.artist_id) AS artists, \
             COUNT(DISTINCT h.album_id) AS albums, \
             COUNT(DISTINCT date(h.listened_at, ?)) AS active_days, \
             strftime('%Y-%m-%dT%H:%M:%SZ', MIN(h.listened_at)) AS first_play, \
             strftime('%Y-%m-%dT%H:%M:%SZ', MAX(h.listened_at)) AS last_play \
             FROM listen_history_item h",
            "",
        )
        .get_result(connection)
}

fn ranked(
    connection: &mut SqliteConnection,
    clause: &Clause,
    kind: &str,
    limit: i64,
) -> Result<Vec<RankRow>, diesel::result::Error> {
    let (id, name, detail, detail_id) = match kind {
        "artist" => ("h.artist_id", "h.artist_name", "NULL", "NULL"),
        "album" => ("h.album_id", "h.album_name", "h.artist_name", "h.artist_id"),
        _ => ("h.song_id", "h.title", "h.artist_name", "h.artist_id"),
    };
    clause
        .query_with(
            &format!(
                "SELECT {id} AS id, MAX({name}) AS name, MAX({detail}) AS detail, \
                 MAX({detail_id}) AS detail_id, COUNT(*) AS plays, \
                 CAST(COALESCE(SUM(COALESCE(h.duration_seconds, 0)), 0) AS REAL) AS seconds \
                 FROM listen_history_item h"
            ),
            &format!(
                "AND {id} IS NOT NULL GROUP BY {id} \
                 ORDER BY plays DESC, seconds DESC, MAX(h.listened_at) DESC LIMIT ?"
            ),
            vec![Bind::Int(limit)],
        )
        .load(connection)
}

fn hydrate_ranked(rows: Vec<RankRow>, kind: &str, cache: Option<&LibraryCache>) -> Vec<RankedItem> {
    rows.into_iter()
        .map(|row| {
            let (name, image, available) = match (kind, cache) {
                ("artist", Some(cache)) => match cache.artist(&row.id) {
                    Some(artist) => (
                        Some(artist.name.clone()),
                        Some(artist.icon_url.clone()),
                        true,
                    ),
                    None => (None, None, false),
                },
                ("album", Some(cache)) => match cache.album(&row.id) {
                    Some(album) => (
                        Some(album.name.clone()),
                        Some(album.cover_url.clone()),
                        true,
                    ),
                    None => (None, None, false),
                },
                (_, Some(cache)) => match cache.song(&row.id) {
                    Some(song) => {
                        let cover = cache
                            .song_map
                            .get(&row.id)
                            .and_then(|(_, album)| cache.album(album))
                            .map(|album| album.cover_url.clone());
                        (Some(song.name.clone()), cover, true)
                    }
                    None => (None, None, false),
                },
                _ => (None, None, false),
            };
            RankedItem {
                name: name.or(row.name).unwrap_or_else(|| "Unknown".into()),
                image_path: image.filter(|path| !path.trim().is_empty()),
                available,
                id: row.id,
                detail: row.detail,
                detail_id: row.detail_id,
                plays: row.plays,
                listened_seconds: row.seconds,
            }
        })
        .collect()
}

/// The period of the same length ending where `filter` starts.
fn previous_filter(filter: &HistoryFilter) -> Option<HistoryFilter> {
    let parse = |value: &Option<String>| {
        value
            .as_deref()
            .and_then(parse_instant)
            .and_then(|text| NaiveDateTime::parse_from_str(&text, DATETIME_FORMAT).ok())
    };
    let (from, to) = (parse(&filter.from)?, parse(&filter.to)?);
    let length = to - from;
    if length <= chrono::Duration::zero() {
        return None;
    }
    let format = |instant: NaiveDateTime| instant.format(DATETIME_FORMAT).to_string();
    Some(HistoryFilter {
        from: Some(format(from - length)),
        to: Some(format(from)),
        ..filter.clone()
    })
}

pub fn history_stats(
    connection: &mut SqliteConnection,
    user_id: i32,
    filter: &HistoryFilter,
    bucket: Bucket,
    top: i64,
    cache: Option<&LibraryCache>,
) -> Result<HistoryStats, diesel::result::Error> {
    let clause = filter.clause(user_id);
    let tz = filter.tz_modifier();
    let totals_now = totals(connection, &clause, &tz)?;
    let previous = previous_filter(filter)
        .map(|previous| totals(connection, &previous.clause(user_id), &tz))
        .transpose()?;

    let with_leading_tz = |clause: &Clause| {
        let mut next = Clause {
            sql: clause.sql.clone(),
            binds: vec![Bind::Text(tz.clone())],
        };
        next.binds.extend(clause.binds.iter().cloned());
        next
    };
    let expression = bucket.expression();
    let timeline = with_leading_tz(&clause)
        .query(
            &format!(
                "SELECT {expression} AS bucket, COUNT(*) AS plays, \
                 CAST(COALESCE(SUM(COALESCE(h.duration_seconds, 0)), 0) AS REAL) AS listened_seconds \
                 FROM listen_history_item h"
            ),
            "GROUP BY bucket ORDER BY bucket",
        )
        .load::<BucketCount>(connection)?;

    #[derive(QueryableByName)]
    struct SlotRow {
        #[diesel(sql_type = BigInt)]
        slot: i64,
        #[diesel(sql_type = BigInt)]
        plays: i64,
    }
    let mut hours = vec![0; 24];
    for row in with_leading_tz(&clause)
        .query(
            "SELECT CAST(strftime('%H', h.listened_at, ?) AS INTEGER) AS slot, COUNT(*) AS plays \
             FROM listen_history_item h",
            "GROUP BY slot",
        )
        .load::<SlotRow>(connection)?
    {
        if let Some(hour) = hours.get_mut(row.slot as usize) {
            *hour = row.plays;
        }
    }
    let mut weekdays = vec![0; 7];
    for row in with_leading_tz(&clause)
        .query(
            "SELECT CAST(strftime('%w', h.listened_at, ?) AS INTEGER) AS slot, COUNT(*) AS plays \
             FROM listen_history_item h",
            "GROUP BY slot",
        )
        .load::<SlotRow>(connection)?
    {
        // SQLite counts Sunday as 0; the response starts on Monday.
        let index = (row.slot as usize + 6) % 7;
        weekdays[index] = row.plays;
    }

    let top_artists = hydrate_ranked(ranked(connection, &clause, "artist", top)?, "artist", cache);
    let top_albums = hydrate_ranked(ranked(connection, &clause, "album", top)?, "album", cache);
    let top_tracks = hydrate_ranked(ranked(connection, &clause, "track", top)?, "track", cache);
    let zones = clause
        .query(
            &format!(
                "SELECT COALESCE(h.zone_id, '{UNKNOWN_ZONE}') AS value, \
                 MAX(h.zone_name) AS label, COUNT(*) AS plays FROM listen_history_item h"
            ),
            "GROUP BY value ORDER BY plays DESC LIMIT 50",
        )
        .load::<GroupCount>(connection)?;
    let sources = clause
        .query(
            "SELECT COALESCE(h.source, 'unknown') AS value, NULL AS label, COUNT(*) AS plays \
             FROM listen_history_item h",
            "GROUP BY value ORDER BY plays DESC LIMIT 20",
        )
        .load::<GroupCount>(connection)?;

    Ok(HistoryStats {
        totals: totals_now,
        previous,
        bucket,
        timeline,
        hours,
        weekdays,
        top_artists,
        top_albums,
        top_tracks,
        zones,
        sources,
    })
}

#[get("/me/history/stats")]
async fn history_stats_route(
    request: HttpRequest,
    query: web::Query<HistoryFilter>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let filter = query.into_inner();
    if let Err(code) = filter.validate() {
        return invalid(code);
    }
    let top = filter.top.unwrap_or(10).clamp(1, MAX_TOP);
    let cache = optional_cache(&lifecycle).await;
    let pool = pool.get_ref().clone();
    match web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        Ok(history_stats(
            &mut connection,
            user_id,
            &filter,
            filter.bucket.unwrap_or_default(),
            top,
            cache.as_deref(),
        )?)
    })
    .await
    {
        Ok(Ok(stats)) => HttpResponse::Ok().json(stats),
        Ok(Err(error)) => worker_failed("history stats failed", error),
        Err(error) => worker_failed("history stats worker failed", error),
    }
}

// ---------------------------------------------------------------------------
// Filter options
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryFilters {
    pub zones: Vec<GroupCount>,
    pub sources: Vec<GroupCount>,
    pub devices: Vec<GroupCount>,
    pub first_play: Option<String>,
    pub total: i64,
}

#[get("/me/history/filters")]
async fn history_filters(request: HttpRequest, pool: web::Data<DbPool>) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let pool = pool.get_ref().clone();
    match web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        let clause = HistoryFilter::default().clause(user_id);
        let zones = clause
            .query(
                &format!(
                    "SELECT COALESCE(h.zone_id, '{UNKNOWN_ZONE}') AS value, \
                     MAX(h.zone_name) AS label, COUNT(*) AS plays FROM listen_history_item h"
                ),
                "GROUP BY value ORDER BY plays DESC LIMIT 50",
            )
            .load::<GroupCount>(&mut connection)?;
        let sources = clause
            .query(
                "SELECT COALESCE(h.source, 'unknown') AS value, NULL AS label, COUNT(*) AS plays \
                 FROM listen_history_item h",
                "GROUP BY value ORDER BY plays DESC LIMIT 20",
            )
            .load::<GroupCount>(&mut connection)?;
        let devices = clause
            .query(
                "SELECT h.device AS value, NULL AS label, COUNT(*) AS plays \
                 FROM listen_history_item h",
                "AND h.device IS NOT NULL GROUP BY value ORDER BY plays DESC LIMIT 20",
            )
            .load::<GroupCount>(&mut connection)?;
        let totals = totals(&mut connection, &clause, "+0 minutes")?;
        Ok(HistoryFilters {
            zones,
            sources,
            devices,
            first_play: totals.first_play,
            total: totals.plays,
        })
    })
    .await
    {
        Ok(Ok(filters)) => HttpResponse::Ok().json(filters),
        Ok(Err(error)) => worker_failed("history filters failed", error),
        Err(error) => worker_failed("history filters worker failed", error),
    }
}

// ---------------------------------------------------------------------------
// Export and removal
// ---------------------------------------------------------------------------

/// One CSV field, quoted when needed, with spreadsheet formulas defused.
pub fn csv_field(value: &str) -> String {
    let defused = if value.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{value}")
    } else {
        value.to_string()
    };
    if defused.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", defused.replace('"', "\"\""))
    } else {
        defused
    }
}

pub const CSV_HEADER: &str = "played_at_utc,title,artist,album,duration_seconds,zone,device,source,song_id,artist_id,album_id";

pub fn history_csv(entries: &[HistoryEntry]) -> String {
    let mut csv = String::with_capacity(64 + entries.len() * 160);
    csv.push_str(CSV_HEADER);
    csv.push_str("\r\n");
    for entry in entries {
        let fields = [
            entry.listened_at.clone(),
            entry.title.clone(),
            entry.artist_name.clone(),
            entry.album_name.clone().unwrap_or_default(),
            format!("{:.0}", entry.duration_seconds),
            entry.zone_name.clone().unwrap_or_default(),
            entry.device.clone().unwrap_or_default(),
            entry.source.clone().unwrap_or_default(),
            entry.song_id.clone(),
            entry.artist_id.clone().unwrap_or_default(),
            entry.album_id.clone().unwrap_or_default(),
        ];
        csv.push_str(
            &fields
                .iter()
                .map(|field| csv_field(field))
                .collect::<Vec<_>>()
                .join(","),
        );
        csv.push_str("\r\n");
    }
    csv
}

#[get("/me/history/export.csv")]
async fn export_csv(
    request: HttpRequest,
    query: web::Query<HistoryFilter>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let filter = query.into_inner();
    if let Err(code) = filter.validate() {
        return invalid(code);
    }
    let cache = optional_cache(&lifecycle).await;
    let pool = pool.get_ref().clone();
    match web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        let rows = load_timeline(&mut connection, user_id, &filter, None, i64::MAX)?;
        let entries = rows
            .into_iter()
            .map(|row| entry(row, cache.as_deref()))
            .collect::<Vec<_>>();
        Ok(history_csv(&entries))
    })
    .await
    {
        Ok(Ok(csv)) => HttpResponse::Ok()
            .content_type("text/csv; charset=utf-8")
            .insert_header((
                "Content-Disposition",
                format!(
                    "attachment; filename=\"parson-history-{}.csv\"",
                    Utc::now().format("%Y-%m-%d")
                ),
            ))
            .insert_header(("Cache-Control", "no-store"))
            .body(csv),
        Ok(Err(error)) => worker_failed("history export failed", error),
        Err(error) => worker_failed("history export worker failed", error),
    }
}

#[delete("/me/history/entries/{id}")]
async fn delete_entry(
    request: HttpRequest,
    id: web::Path<i32>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let id = id.into_inner();
    let pool = pool.get_ref().clone();
    match web::block(move || -> Result<usize, BoxError> {
        let mut connection = pool.get()?;
        Ok(
            diesel::sql_query("DELETE FROM listen_history_item WHERE user_id = ? AND id = ?")
                .bind::<Integer, _>(user_id)
                .bind::<Integer, _>(id)
                .execute(&mut connection)?,
        )
    })
    .await
    {
        Ok(Ok(0)) => not_found("History entry not found.", "history_entry_not_found"),
        Ok(Ok(_)) => HttpResponse::NoContent().finish(),
        Ok(Err(error)) => worker_failed("history delete failed", error),
        Err(error) => worker_failed("history delete worker failed", error),
    }
}

/// History routes inside the `/users` scope.
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(history_timeline)
        .service(history_stats_route)
        .service(history_filters)
        .service(export_csv)
        .service(delete_entry);
}

#[cfg(test)]
mod tests {
    use super::*;
    use diesel::connection::SimpleConnection;
    use diesel_migrations::MigrationHarness;

    fn database() -> SqliteConnection {
        let mut connection = SqliteConnection::establish(":memory:").expect("history database");
        connection
            .run_pending_migrations(crate::persistence::connection::MIGRATIONS)
            .expect("migrations");
        connection
            .batch_execute(
                "INSERT INTO user(id, username, password) VALUES (1, 'one', 'x'), (2, 'two', 'x');
                 INSERT INTO listen_history_item
                   (user_id, song_id, listened_at, zone_id, zone_name, source, device,
                    title, artist_id, artist_name, album_id, album_name, duration_seconds) VALUES
                   (1, 's1', '2026-10-01 22:30:00', 'client:a', 'Office', 'manual', 'Desktop app',
                    'Blue', 'ar1', 'Miles', 'al1', 'Kind of Blue', 300),
                   (1, 's2', '2026-10-02 08:00:00', 'client:a', 'Office', 'generated', 'Desktop app',
                    'So What', 'ar1', 'Miles', 'al1', 'Kind of Blue', 540),
                   (1, 's3', '2026-10-05 12:00:00', 'sonos:x', 'Kitchen', 'manual', NULL,
                    '=cmd()', 'ar2', 'Nina, \"High\" Priestess', 'al2', 'Pastel', 200),
                   (1, 's1', '2026-10-06 09:00:00', NULL, NULL, NULL, NULL,
                    'Blue', 'ar1', 'Miles', 'al1', 'Kind of Blue', 300),
                   (2, 's9', '2026-10-03 09:00:00', 'client:b', 'Theirs', 'manual', NULL,
                    'Other', 'ar9', 'Other', 'al9', 'Other', 100);",
            )
            .expect("fixture");
        connection
    }

    fn filter() -> HistoryFilter {
        HistoryFilter::default()
    }

    #[test]
    fn query_strings_parse_typed_numbers() {
        let parsed = web::Query::<HistoryFilter>::from_query(
            "from=2026-10-01T00:00:00.000Z&tz=-300&bucket=week&top=5&before_id=9&limit=20&zone=none",
        )
        .expect("query")
        .into_inner();
        assert_eq!(parsed.tz, Some(-300));
        assert_eq!(parsed.bucket, Some(Bucket::Week));
        assert_eq!(parsed.top, Some(5));
        assert_eq!(parsed.before_id, Some(9));
        assert_eq!(parsed.limit, Some(20));
        assert!(parsed.validate().is_ok());
    }

    #[test]
    fn instants_parse_into_the_stored_form() {
        assert_eq!(
            parse_instant("2026-10-01T23:30:00+01:00").as_deref(),
            Some("2026-10-01 22:30:00")
        );
        assert_eq!(
            parse_instant("2026-10-01").as_deref(),
            Some("2026-10-01 00:00:00")
        );
        assert!(parse_instant("yesterday").is_none());
        let reversed = HistoryFilter {
            from: Some("2026-10-02".into()),
            to: Some("2026-10-01".into()),
            ..filter()
        };
        assert!(reversed.validate().is_err());
    }

    #[test]
    fn timeline_is_newest_first_filtered_and_per_profile() {
        let mut connection = database();
        let rows = load_timeline(&mut connection, 1, &filter(), None, 10).expect("timeline");
        assert_eq!(
            rows.iter()
                .map(|row| row.song_id.as_str())
                .collect::<Vec<_>>(),
            ["s1", "s3", "s2", "s1"]
        );
        let office = HistoryFilter {
            zone: Some("client:a".into()),
            ..filter()
        };
        assert_eq!(
            load_timeline(&mut connection, 1, &office, None, 10)
                .expect("zone")
                .len(),
            2
        );
        let unknown = HistoryFilter {
            zone: Some(UNKNOWN_ZONE.into()),
            ..filter()
        };
        assert_eq!(
            load_timeline(&mut connection, 1, &unknown, None, 10)
                .expect("no zone")
                .len(),
            1
        );
        let radio = HistoryFilter {
            source: Some("generated".into()),
            ..filter()
        };
        assert_eq!(
            load_timeline(&mut connection, 1, &radio, None, 10).expect("radio")[0].song_id,
            "s2"
        );
        let october_first = HistoryFilter {
            from: Some("2026-10-01T00:00:00Z".into()),
            to: Some("2026-10-02T00:00:00Z".into()),
            ..filter()
        };
        assert_eq!(
            load_timeline(&mut connection, 1, &october_first, None, 10)
                .expect("day")
                .len(),
            1
        );
        let text = HistoryFilter {
            q: Some("pastel".into()),
            ..filter()
        };
        assert_eq!(
            load_timeline(&mut connection, 1, &text, None, 10).expect("text")[0].song_id,
            "s3"
        );
        // A play imported later but heard earlier sorts by when it was heard.
        connection
            .batch_execute(
                "INSERT INTO listen_history_item (user_id, song_id, listened_at)
                 VALUES (1, 'old', '2026-09-01 10:00:00')",
            )
            .expect("imported play");
        let all = load_timeline(&mut connection, 1, &filter(), None, 10).expect("all");
        assert_eq!(all.last().map(|row| row.song_id.as_str()), Some("old"));
        let first = load_timeline(&mut connection, 1, &filter(), None, 2).expect("page");
        let older =
            load_timeline(&mut connection, 1, &filter(), Some(first[1].id), 10).expect("older");
        assert_eq!(
            older
                .iter()
                .map(|row| row.song_id.as_str())
                .collect::<Vec<_>>(),
            ["s2", "s1", "old"]
        );
    }

    #[test]
    fn stats_rank_bucket_and_compare_periods() {
        let mut connection = database();
        let stats =
            history_stats(&mut connection, 1, &filter(), Bucket::Day, 10, None).expect("stats");
        assert_eq!(stats.totals.plays, 4);
        assert_eq!(stats.totals.listened_seconds, 1340.0);
        assert_eq!(stats.totals.artists, 2);
        assert_eq!(stats.totals.tracks, 3);
        assert_eq!(stats.totals.active_days, 4);
        assert_eq!(stats.top_artists[0].id, "ar1");
        assert_eq!(stats.top_artists[0].plays, 3);
        assert_eq!(stats.top_albums[0].detail.as_deref(), Some("Miles"));
        assert_eq!(stats.top_tracks[0].id, "s1");
        assert_eq!(stats.timeline.len(), 4);
        assert_eq!(stats.hours.iter().sum::<i64>(), 4);
        assert_eq!(stats.hours[22], 1);
        assert_eq!(stats.zones[0].value, "client:a");
        assert!(stats.previous.is_none());

        // The 22:30 UTC play falls on the next day two hours east of UTC.
        let east = HistoryFilter {
            tz: Some(120),
            ..filter()
        };
        let shifted = history_stats(&mut connection, 1, &east, Bucket::Day, 10, None).expect("tz");
        assert_eq!(shifted.timeline[0].bucket, "2026-10-02");
        assert_eq!(shifted.timeline[0].plays, 2);
        assert_eq!(shifted.hours[0], 1);

        let week = HistoryFilter {
            from: Some("2026-10-05T00:00:00Z".into()),
            to: Some("2026-10-12T00:00:00Z".into()),
            ..filter()
        };
        let weekly =
            history_stats(&mut connection, 1, &week, Bucket::Week, 10, None).expect("week");
        assert_eq!(weekly.totals.plays, 2);
        assert_eq!(
            weekly.previous.as_ref().map(|previous| previous.plays),
            Some(2)
        );
        assert_eq!(weekly.timeline.len(), 1);
        assert_eq!(weekly.timeline[0].bucket, "2026-10-05");
        // Monday 2026-10-05 and Tuesday 2026-10-06.
        assert_eq!(weekly.weekdays[0], 1);
        assert_eq!(weekly.weekdays[1], 1);
    }

    #[test]
    fn csv_escapes_and_defuses_formulas() {
        let mut connection = database();
        let entries = load_timeline(&mut connection, 1, &filter(), None, 10)
            .expect("rows")
            .into_iter()
            .map(|row| entry(row, None))
            .collect::<Vec<_>>();
        let csv = history_csv(&entries);
        let lines = csv.split("\r\n").collect::<Vec<_>>();
        assert_eq!(lines[0], CSV_HEADER);
        assert_eq!(lines.len(), 6);
        assert!(lines[2].starts_with("2026-10-05T12:00:00Z,'=cmd(),\"Nina, \"\"High\"\" Priestess\",Pastel,200,Kitchen,,manual,s3,"));
        assert_eq!(csv_field("-1"), "'-1");
        assert_eq!(csv_field("plain"), "plain");
    }
}
