//! Library Focus: multi-facet filtering over the catalog tables.
//!
//! A [`FocusQuery`] is a list of filters (genre, release year, format,
//! quality, sample rate, bit depth, credits, label, release type, user tags,
//! storage location, liked, played, recently added), each of which can be
//! inverted into an exclusion, combined with "match all" or "match any".
//! The query is compiled to SQL here so it can run against the catalog tables
//! the indexer writes (`track_entity`, `album_entity`, `track_artist`,
//! `raw_file_metadata`, ...) and use the indexes added by the
//! `library_focus` migration.
//!
//! Every filter is compiled to an uncorrelated set of track ids (and, for
//! album-level properties such as the release year, a set of album ids).
//! SQLite evaluates each set once, so a query costs one pass per filter
//! instead of one correlated lookup per catalog row. Albums and artists
//! match a filter when at least one of their tracks does; an exclusion keeps
//! only entities with no matching track, which is how Roon treats "click a
//! second time to exclude".

use std::collections::BTreeMap;

use diesel::prelude::*;
use diesel::query_builder::BoxedSqlQuery;
use diesel::sql_types::{BigInt, Nullable, Text};
use diesel::sqlite::{Sqlite, SqliteConnection};
use serde::{Deserialize, Serialize};

use crate::library::credits::CREDIT_ROLES;

pub const MAX_FILTERS: usize = 32;
pub const MAX_TEXT_CHARS: usize = 200;
pub const MAX_PAGE_SIZE: usize = 200;
pub const DEFAULT_FACET_VALUES: usize = 12;
pub const MAX_FACET_VALUES: usize = 200;
const MAX_YEAR_VALUES: usize = 400;
/// Largest id page a caller may resolve at once (a smart playlist's tracks).
pub const MAX_RESOLVED_IDS: usize = crate::playlist_rules::MAX_TRACKS_PER_PLAYLIST as usize;
const MAX_ADDED_WITHIN_DAYS: i64 = 36_500;

/// The browser a focus applies to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusEntity {
    #[default]
    Albums,
    Songs,
    Artists,
    /// People credited as composer on at least one track.
    Composers,
}

impl FocusEntity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Albums => "albums",
            Self::Songs => "songs",
            Self::Artists => "artists",
            Self::Composers => "composers",
        }
    }
}

/// Credit facet columns that merge several stored roles, as Roon's
/// "Conductor / Ensemble" and "Production" columns do.
pub const CREDIT_GROUPS: [(&str, &[&str]); 2] = [
    ("conductor_ensemble", &["conductor", "ensemble"]),
    ("production", &["producer", "engineer", "mixer", "remixer"]),
];

/// Stored roles a credit criterion's role matches (`None` for `any`).
pub fn credit_role_members(role: &str) -> Option<Vec<&str>> {
    if role == "any" {
        return None;
    }
    Some(
        CREDIT_GROUPS
            .iter()
            .find(|(group, _)| *group == role)
            .map(|(_, members)| members.to_vec())
            .unwrap_or_else(|| vec![role]),
    )
}

fn valid_credit_role(role: &str) -> bool {
    role == "any"
        || CREDIT_ROLES.contains(&role)
        || CREDIT_GROUPS.iter().any(|(group, _)| *group == role)
}

/// The facet column a stored credit role is counted in.
const CREDIT_COLUMN_SQL: &str = "CASE \
    WHEN base_role IN ('conductor', 'ensemble') THEN 'conductor_ensemble' \
    WHEN base_role IN ('producer', 'engineer', 'mixer', 'remixer') THEN 'production' \
    ELSE base_role END";

/// Display order of credit facet columns.
pub fn credit_column_rank(column: &str) -> usize {
    const ORDER: [&str; 6] = [
        "performer",
        "conductor_ensemble",
        "composer",
        "lyricist",
        "arranger",
        "production",
    ];
    ORDER
        .iter()
        .position(|candidate| *candidate == column)
        .unwrap_or(ORDER.len())
}

/// Largest number of scope criteria a focus may carry.
pub const MAX_SCOPE: usize = 4;

/// How included filters combine. Exclusions always apply.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode {
    #[default]
    All,
    Any,
}

/// Audio quality tiers derived from codec, sample rate and bit depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityTier {
    /// Lossless above 48 kHz or deeper than 16 bits.
    Hires,
    /// Any lossless codec, including high resolution files.
    Lossless,
    /// Lossless at 16 bits and at most 48 kHz.
    Cd,
    Lossy,
}

/// One focus criterion. Labels (`name`) are optional display hints so that a
/// saved focus can render its chips without another lookup; they never
/// affect matching.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "facet", rename_all = "snake_case")]
pub enum Criterion {
    Genre {
        value: String,
    },
    Year {
        #[serde(default)]
        from: Option<i64>,
        #[serde(default)]
        to: Option<i64>,
    },
    Format {
        value: String,
    },
    Quality {
        value: QualityTier,
    },
    SampleRate {
        #[serde(default)]
        min: Option<i64>,
        #[serde(default)]
        max: Option<i64>,
    },
    BitDepth {
        #[serde(default)]
        min: Option<i64>,
        #[serde(default)]
        max: Option<i64>,
    },
    Credit {
        /// A canonical credit role (`composer`, `performer`, ...) or `any`.
        role: String,
        artist_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Artist {
        artist_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// The tracks of one album, used to scope a focus to an album's
    /// track list.
    Album {
        album_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Label {
        value: String,
    },
    ReleaseType {
        value: String,
    },
    Tag {
        tag_id: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Source {
        root_id: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Liked,
    Played,
    AddedWithin {
        days: i64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FocusFilter {
    #[serde(flatten)]
    pub criterion: Criterion,
    #[serde(default)]
    pub exclude: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusSort {
    Artist,
    Title,
    Album,
    ReleaseDate,
    DateAdded,
    MostPlayed,
    LastPlayed,
    Duration,
    TrackCount,
    Quality,
    Random,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FocusQuery {
    #[serde(default)]
    pub entity: FocusEntity,
    #[serde(default, rename = "match")]
    pub match_mode: MatchMode,
    #[serde(default)]
    pub filters: Vec<FocusFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<FocusSort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<SortDirection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u32>,
    /// Criteria that fix the subset a focus browses (a composer's works, an
    /// album's tracks). They always apply, are never shown as chips and
    /// also bound the "of N" library total.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scope: Vec<Criterion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusError(pub String);

impl std::fmt::Display for FocusError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for FocusError {}

fn invalid(message: impl Into<String>) -> FocusError {
    FocusError(message.into())
}

fn valid_text(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty() && value.chars().count() <= maximum
}

fn valid_range(min: Option<i64>, max: Option<i64>, bound: i64) -> bool {
    let in_bounds = |value: Option<i64>| value.is_none_or(|value| (0..=bound).contains(&value));
    (min.is_some() || max.is_some())
        && in_bounds(min)
        && in_bounds(max)
        && match (min, max) {
            (Some(min), Some(max)) => min <= max,
            _ => true,
        }
}

impl FocusQuery {
    /// Rejects queries that are malformed or larger than the server accepts.
    pub fn validate(&self) -> Result<(), FocusError> {
        if self.filters.len() > MAX_FILTERS {
            return Err(invalid(format!(
                "A focus can combine at most {MAX_FILTERS} filters."
            )));
        }
        if self
            .text
            .as_deref()
            .is_some_and(|text| text.chars().count() > MAX_TEXT_CHARS)
        {
            return Err(invalid("The focus text filter is too long."));
        }
        if self.scope.len() > MAX_SCOPE {
            return Err(invalid(format!(
                "A focus can be scoped by at most {MAX_SCOPE} criteria."
            )));
        }
        let criteria = self
            .filters
            .iter()
            .map(|filter| &filter.criterion)
            .chain(self.scope.iter());
        for criterion in criteria {
            let valid = match criterion {
                Criterion::Genre { value }
                | Criterion::Format { value }
                | Criterion::Label { value } => valid_text(value, MAX_TEXT_CHARS),
                Criterion::ReleaseType { value } => valid_text(value, 64),
                Criterion::Year { from, to } => valid_range(*from, *to, 9_999),
                Criterion::SampleRate { min, max } => valid_range(*min, *max, 10_000_000),
                Criterion::BitDepth { min, max } => valid_range(*min, *max, 64),
                Criterion::Credit {
                    role, artist_id, ..
                } => valid_credit_role(role) && valid_text(artist_id, 256),
                Criterion::Artist { artist_id, .. } => valid_text(artist_id, 256),
                Criterion::Album { album_id, .. } => valid_text(album_id, 256),
                Criterion::Tag { tag_id, .. } => *tag_id > 0,
                Criterion::Source { root_id, .. } => *root_id > 0,
                Criterion::AddedWithin { days } => (1..=MAX_ADDED_WITHIN_DAYS).contains(days),
                Criterion::Quality { .. } | Criterion::Liked | Criterion::Played => true,
            };
            if !valid {
                return Err(invalid("A focus filter is invalid."));
            }
        }
        Ok(())
    }

    /// The sort applied when the requested one does not fit the entity.
    pub fn effective_sort(&self) -> FocusSort {
        let requested = self.sort.unwrap_or(match self.entity {
            FocusEntity::Albums | FocusEntity::Songs => FocusSort::Artist,
            FocusEntity::Artists | FocusEntity::Composers => FocusSort::Title,
        });
        let supported = match self.entity {
            FocusEntity::Albums => !matches!(requested, FocusSort::Album),
            FocusEntity::Songs => !matches!(requested, FocusSort::TrackCount),
            FocusEntity::Artists | FocusEntity::Composers => !matches!(
                requested,
                FocusSort::Album
                    | FocusSort::ReleaseDate
                    | FocusSort::Duration
                    | FocusSort::Quality
            ),
        };
        if supported {
            requested
        } else if matches!(self.entity, FocusEntity::Artists | FocusEntity::Composers) {
            FocusSort::Title
        } else {
            FocusSort::Artist
        }
    }

    pub fn effective_direction(&self) -> SortDirection {
        self.direction.unwrap_or(match self.effective_sort() {
            FocusSort::DateAdded
            | FocusSort::MostPlayed
            | FocusSort::LastPlayed
            | FocusSort::TrackCount
            | FocusSort::Quality => SortDirection::Desc,
            _ => SortDirection::Asc,
        })
    }

    /// The same filters applied to individual tracks, as used when a focus
    /// becomes a smart playlist.
    pub fn as_songs(&self) -> FocusQuery {
        let mut songs = self.clone();
        songs.entity = FocusEntity::Songs;
        if songs.sort == Some(FocusSort::TrackCount) {
            songs.sort = None;
        }
        songs
    }
}

/// A bound SQL parameter.
#[derive(Clone, Debug, PartialEq)]
pub enum Bind {
    Text(String),
    Int(i64),
}

/// A piece of SQL with its positional parameters, in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fragment {
    pub sql: String,
    pub binds: Vec<Bind>,
}

impl Fragment {
    fn new(sql: impl Into<String>) -> Self {
        Self {
            sql: sql.into(),
            binds: Vec::new(),
        }
    }

    fn text(mut self, value: impl Into<String>) -> Self {
        self.binds.push(Bind::Text(value.into()));
        self
    }

    fn int(mut self, value: i64) -> Self {
        self.binds.push(Bind::Int(value));
        self
    }

    fn push(&mut self, sql: &str) -> &mut Self {
        self.sql.push_str(sql);
        self
    }

    fn append(&mut self, other: Fragment) -> &mut Self {
        self.sql.push_str(&other.sql);
        self.binds.extend(other.binds);
        self
    }

    /// Prepares the statement with every parameter bound.
    pub fn into_query(self) -> BoxedSqlQuery<'static, Sqlite, diesel::query_builder::SqlQuery> {
        let mut query = diesel::sql_query(self.sql).into_boxed::<Sqlite>();
        for bind in self.binds {
            query = match bind {
                Bind::Text(value) => query.bind::<Text, _>(value),
                Bind::Int(value) => query.bind::<BigInt, _>(value),
            };
        }
        query
    }
}

/// Format of a track's primary file: the codec, with PCM reported as its
/// container (WAV or AIFF), falling back to the file extension.
const FORMAT_EXPR: &str = "CASE WHEN pm.codec IN ('pcm', 'pcm_float') \
     THEN COALESCE(NULLIF(pm.container, ''), 'pcm') \
     ELSE COALESCE(NULLIF(pm.codec, ''), lower(pfe.extension)) END";
const LOSSLESS_FORMATS: &str =
    "('flac', 'alac', 'pcm', 'pcm_float', 'wavpack', 'ape', 'tta', 'wav', 'aiff', 'aif')";
const PRIMARY_FILES: &str = "SELECT pf.track_id FROM track_file pf \
     JOIN file_entry pfe ON pfe.id = pf.file_id \
     LEFT JOIN raw_file_metadata pm ON pm.file_id = pf.file_id \
     WHERE pf.is_primary = 1";
const RELEASE_YEAR: &str = "CAST(substr(first_release_date, 1, 4) AS INTEGER)";
const ARTIST_ROLES_SQL: &str = "('primary', 'featured')";

fn lossless_condition() -> String {
    format!("({FORMAT_EXPR}) IN {LOSSLESS_FORMATS}")
}

fn hires_condition() -> String {
    format!(
        "({} AND (COALESCE(pm.sample_rate, 0) > 48000 OR COALESCE(pm.bit_depth, 0) > 16))",
        lossless_condition()
    )
}

fn quality_tier_expr() -> String {
    format!(
        "CASE WHEN {hires} THEN 'hires' WHEN {lossless} THEN 'cd' \
         WHEN pm.codec IS NOT NULL OR pfe.extension IS NOT NULL THEN 'lossy' ELSE 'unknown' END",
        hires = hires_condition(),
        lossless = lossless_condition(),
    )
}

/// The ids a criterion matches.
struct CriterionSets {
    /// Track ids (never NULL).
    tracks: Fragment,
    /// Album ids, when the criterion is a property of the album itself.
    albums: Option<Fragment>,
    /// Artist ids, when the criterion names artists directly.
    artists: Option<Fragment>,
}

fn primary_file_tracks(condition: Fragment) -> Fragment {
    let mut fragment = Fragment::new(format!("{PRIMARY_FILES} AND ("));
    fragment.append(condition).push(")");
    fragment
}

fn range_condition(column: &str, min: Option<i64>, max: Option<i64>) -> Fragment {
    match (min, max) {
        (Some(min), Some(max)) => Fragment::new(format!("{column} BETWEEN ? AND ?"))
            .int(min)
            .int(max),
        (Some(min), None) => Fragment::new(format!("{column} >= ?")).int(min),
        (None, Some(max)) => Fragment::new(format!("{column} <= ?")).int(max),
        (None, None) => Fragment::new("1"),
    }
}

fn album_tracks(albums: &Fragment) -> Fragment {
    let mut tracks = Fragment::new("SELECT id FROM track_entity WHERE album_id IN (");
    tracks.append(albums.clone()).push(")");
    tracks
}

fn criterion_sets(criterion: &Criterion, user_id: i32) -> CriterionSets {
    let user = i64::from(user_id);
    let tracks_only = |tracks: Fragment| CriterionSets {
        tracks,
        albums: None,
        artists: None,
    };
    let album_level = |albums: Fragment| CriterionSets {
        tracks: album_tracks(&albums),
        albums: Some(albums),
        artists: None,
    };
    match criterion {
        Criterion::Genre { value } => {
            let key = crate::library::indexer::normalize_genre_key(value);
            // Genres are album genres (the union of their tracks' genres),
            // as in Roon; tracks without an album keep their own genres.
            let albums = Fragment::new(
                "SELECT ag.album_id FROM album_genre ag \
                 JOIN genre_entity ge ON ge.id = ag.genre_id WHERE ge.normalized_name = ?",
            )
            .text(key.clone());
            let mut tracks = Fragment::new(
                "SELECT tg.track_id FROM track_genre tg \
                 JOIN genre_entity ge ON ge.id = tg.genre_id WHERE ge.normalized_name = ? \
                 UNION SELECT id FROM track_entity WHERE album_id IN (",
            )
            .text(key);
            tracks.append(albums.clone()).push(")");
            CriterionSets {
                tracks,
                albums: Some(albums),
                artists: None,
            }
        }
        Criterion::Year { from, to } => {
            let mut albums = Fragment::new(format!(
                "SELECT id FROM album_entity WHERE {RELEASE_YEAR} > 0 AND "
            ));
            albums.append(range_condition(RELEASE_YEAR, *from, *to));
            album_level(albums)
        }
        // Release types come only from RELEASETYPE / MusicBrainz Album Type
        // tags; untagged albums have no release type rather than a guess.
        Criterion::ReleaseType { value } => tracks_only(primary_file_tracks(
            Fragment::new("pm.release_type = lower(?)").text(value.trim()),
        )),
        Criterion::Album { album_id, .. } => {
            album_level(Fragment::new("SELECT ?").text(album_id.clone()))
        }
        Criterion::Format { value } => tracks_only(primary_file_tracks(
            Fragment::new(format!("({FORMAT_EXPR}) = lower(?)")).text(value.trim()),
        )),
        Criterion::Quality { value } => {
            let condition = match value {
                QualityTier::Hires => hires_condition(),
                QualityTier::Lossless => lossless_condition(),
                QualityTier::Cd => {
                    format!("({} AND NOT {})", lossless_condition(), hires_condition())
                }
                QualityTier::Lossy => format!(
                    "(NOT {} AND (pm.codec IS NOT NULL OR pfe.extension IS NOT NULL))",
                    lossless_condition()
                ),
            };
            tracks_only(primary_file_tracks(Fragment::new(condition)))
        }
        Criterion::SampleRate { min, max } => tracks_only(primary_file_tracks(range_condition(
            "pm.sample_rate",
            *min,
            *max,
        ))),
        Criterion::BitDepth { min, max } => tracks_only(primary_file_tracks(range_condition(
            "pm.bit_depth",
            *min,
            *max,
        ))),
        Criterion::Label { value } => tracks_only(primary_file_tracks(
            Fragment::new("trim(pm.label) = ? COLLATE NOCASE").text(value.trim()),
        )),
        Criterion::Credit {
            role, artist_id, ..
        } => {
            let tracks = match credit_role_members(role) {
                None => Fragment::new("SELECT track_id FROM track_artist WHERE artist_id = ?")
                    .text(artist_id.clone()),
                Some(members) => {
                    let mut tracks = Fragment::new(
                        "SELECT track_id FROM track_artist WHERE artist_id = ? AND (",
                    )
                    .text(artist_id.clone());
                    for (index, member) in members.iter().enumerate() {
                        if index > 0 {
                            tracks.push(" OR ");
                        }
                        tracks.append(
                            Fragment::new("role = ? OR role LIKE ?")
                                .text(*member)
                                .text(format!("{member}:%")),
                        );
                    }
                    tracks.push(")");
                    tracks
                }
            };
            tracks_only(tracks)
        }
        Criterion::Artist { artist_id, .. } => CriterionSets {
            tracks: Fragment::new(format!(
                "SELECT track_id FROM track_artist WHERE artist_id = ? AND role IN {ARTIST_ROLES_SQL} \
                 UNION SELECT t.id FROM track_entity t \
                 JOIN album_artist aa ON aa.album_id = t.album_id \
                 WHERE aa.artist_id = ? AND aa.role = 'primary'"
            ))
            .text(artist_id.clone())
            .text(artist_id.clone()),
            albums: None,
            artists: Some(Fragment::new("SELECT ?").text(artist_id.clone())),
        },
        Criterion::Tag { tag_id, .. } => {
            let tag = |entity: &str| {
                Fragment::new(format!(
                    "SELECT i.entity_id FROM user_tag_item i JOIN user_tag g ON g.id = i.tag_id \
                     WHERE g.user_id = ? AND i.tag_id = ? AND i.entity_type = '{entity}'"
                ))
                .int(user)
                .int(*tag_id)
            };
            let mut tracks = tag("track");
            tracks.push(" UNION SELECT id FROM track_entity WHERE album_id IN (");
            tracks.append(tag("album")).push(")");
            tracks.push(format!(
                " UNION SELECT track_id FROM track_artist WHERE role IN {ARTIST_ROLES_SQL} AND artist_id IN ("
            ).as_str());
            tracks.append(tag("artist")).push(")");
            tracks.push(
                " UNION SELECT t.id FROM track_entity t JOIN album_artist aa ON aa.album_id = t.album_id \
                 WHERE aa.role = 'primary' AND aa.artist_id IN (",
            );
            tracks.append(tag("artist")).push(")");
            CriterionSets {
                tracks,
                albums: None,
                artists: Some(tag("artist")),
            }
        }
        Criterion::Source { root_id, .. } => tracks_only(primary_file_tracks(
            Fragment::new("pfe.root_id = ?").int(*root_id),
        )),
        Criterion::Liked => tracks_only(
            Fragment::new("SELECT song_id FROM favorite_song WHERE user_id = ?").int(user),
        ),
        Criterion::Played => tracks_only(
            Fragment::new(
                "SELECT song_id FROM listen_history_item WHERE user_id = ? \
                 UNION SELECT song_id FROM user_track_preference \
                 WHERE user_id = ? AND (qualified_plays > 0 OR completions > 0)",
            )
            .int(user)
            .int(user),
        ),
        Criterion::AddedWithin { days } => tracks_only(
            Fragment::new("SELECT id FROM track_entity WHERE created_at >= datetime('now', ?)")
                .text(format!("-{days} days")),
        ),
    }
}

/// Entity ids that match a criterion, as an `IN (...)` set over `e.id`.
fn entity_set(entity: FocusEntity, sets: CriterionSets) -> Fragment {
    match entity {
        FocusEntity::Songs => sets.tracks,
        FocusEntity::Albums => match sets.albums {
            Some(albums) => albums,
            None => {
                let mut albums = Fragment::new(
                    "SELECT album_id FROM track_entity WHERE album_id IS NOT NULL AND id IN (",
                );
                albums.append(sets.tracks).push(")");
                albums
            }
        },
        FocusEntity::Artists => {
            let mut artists = Fragment::new(format!(
                "SELECT artist_id FROM track_artist WHERE role IN {ARTIST_ROLES_SQL} AND track_id IN ("
            ));
            artists.append(sets.tracks.clone()).push(")");
            artists.push(
                " UNION SELECT aa.artist_id FROM album_artist aa \
                 JOIN track_entity at ON at.album_id = aa.album_id \
                 WHERE aa.role = 'primary' AND at.id IN (",
            );
            artists.append(sets.tracks).push(")");
            if let Some(direct) = sets.artists {
                artists.push(" UNION ");
                artists.append(direct);
            }
            artists
        }
        FocusEntity::Composers => {
            let mut composers = Fragment::new(
                "SELECT artist_id FROM track_artist WHERE role = 'composer' AND track_id IN (",
            );
            composers.append(sets.tracks).push(")");
            if let Some(direct) = sets.artists {
                composers.push(" UNION ");
                composers.append(direct);
            }
            composers
        }
    }
}

fn entity_table(entity: FocusEntity) -> &'static str {
    match entity {
        FocusEntity::Albums => "album_entity",
        FocusEntity::Songs => "track_entity",
        FocusEntity::Artists | FocusEntity::Composers => "artist_entity",
    }
}

/// Entities that belong to the browsable catalog.
fn universe(entity: FocusEntity) -> &'static str {
    match entity {
        FocusEntity::Songs => "1",
        FocusEntity::Albums => {
            "e.id IN (SELECT album_id FROM track_entity WHERE album_id IS NOT NULL)"
        }
        FocusEntity::Artists => {
            "e.id IN (SELECT artist_id FROM track_artist WHERE role IN ('primary', 'featured') \
             UNION SELECT artist_id FROM album_artist WHERE role = 'primary')"
        }
        FocusEntity::Composers => {
            "e.id IN (SELECT artist_id FROM track_artist WHERE role = 'composer')"
        }
    }
}

fn like_pattern(text: &str) -> String {
    let mut pattern = String::with_capacity(text.len() + 2);
    pattern.push('%');
    for character in text.trim().chars() {
        if matches!(character, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(character);
    }
    pattern.push('%');
    pattern
}

fn text_condition(entity: FocusEntity, text: &str) -> Fragment {
    let pattern = like_pattern(text);
    match entity {
        FocusEntity::Songs => Fragment::new(
            "(e.title LIKE ? ESCAPE '\\' \
             OR e.album_id IN (SELECT id FROM album_entity WHERE title LIKE ? ESCAPE '\\') \
             OR e.id IN (SELECT ta.track_id FROM track_artist ta \
               JOIN artist_entity ar ON ar.id = ta.artist_id \
               WHERE ta.role IN ('primary', 'featured') AND ar.name LIKE ? ESCAPE '\\'))",
        )
        .text(pattern.clone())
        .text(pattern.clone())
        .text(pattern),
        FocusEntity::Albums => Fragment::new(
            "(e.title LIKE ? ESCAPE '\\' \
             OR e.id IN (SELECT aa.album_id FROM album_artist aa \
               JOIN artist_entity ar ON ar.id = aa.artist_id \
               WHERE aa.role = 'primary' AND ar.name LIKE ? ESCAPE '\\'))",
        )
        .text(pattern.clone())
        .text(pattern),
        FocusEntity::Artists | FocusEntity::Composers => {
            Fragment::new("e.name LIKE ? ESCAPE '\\'").text(pattern)
        }
    }
}

/// The `WHERE` condition over alias `e` for a focus.
pub fn where_condition(query: &FocusQuery, user_id: i32) -> Fragment {
    let entity = query.entity;
    let mut condition = Fragment::new(universe(entity));
    for criterion in &query.scope {
        condition.push(" AND e.id IN (");
        condition
            .append(entity_set(entity, criterion_sets(criterion, user_id)))
            .push(")");
    }
    if let Some(text) = query.text.as_deref().filter(|text| !text.trim().is_empty()) {
        condition.push(" AND ");
        condition.append(text_condition(entity, text));
    }
    let mut includes = Vec::new();
    for filter in &query.filters {
        let set = entity_set(entity, criterion_sets(&filter.criterion, user_id));
        let mut membership = Fragment::new(if filter.exclude {
            "e.id NOT IN ("
        } else {
            "e.id IN ("
        });
        membership.append(set).push(")");
        if filter.exclude || query.match_mode == MatchMode::All {
            condition.push(" AND ");
            condition.append(membership);
        } else {
            includes.push(membership);
        }
    }
    if !includes.is_empty() {
        condition.push(" AND (");
        for (index, membership) in includes.into_iter().enumerate() {
            if index > 0 {
                condition.push(" OR ");
            }
            condition.append(membership);
        }
        condition.push(")");
    }
    condition
}

fn direction_sql(direction: SortDirection) -> &'static str {
    match direction {
        SortDirection::Asc => "ASC",
        SortDirection::Desc => "DESC",
    }
}

/// Joins needed by a sort and the `ORDER BY` list.
struct SortPlan {
    joins: Fragment,
    order: Fragment,
}

fn quality_score(track: &str) -> String {
    format!(
        "(SELECT MAX(CASE WHEN {lossless} THEN 1000000000 ELSE 0 END \
           + COALESCE(pm.sample_rate, 0) * 100 + COALESCE(pm.bit_depth, 0) * 10000000 \
           + COALESCE(pm.bitrate, 0)) \
          FROM track_file pf JOIN file_entry pfe ON pfe.id = pf.file_id \
          LEFT JOIN raw_file_metadata pm ON pm.file_id = pf.file_id \
          WHERE pf.is_primary = 1 AND pf.track_id {track})",
        lossless = lossless_condition(),
    )
}

fn sort_plan(query: &FocusQuery, user_id: i32) -> SortPlan {
    let user = i64::from(user_id);
    let entity = query.entity;
    let sort = query.effective_sort();
    let dir = direction_sql(query.effective_direction());
    let mut joins = Fragment::default();
    let order = match (entity, sort) {
        (_, FocusSort::Random) => {
            let seed = i64::from(query.seed.unwrap_or(1));
            Fragment::new("((e.rowid + ?) * 2654435761) % 4294967311").int(seed)
        }
        (FocusEntity::Albums, FocusSort::Artist) => Fragment::new(format!(
            "(SELECT COALESCE(ar.sort_name, ar.name) FROM album_artist aa \
               JOIN artist_entity ar ON ar.id = aa.artist_id \
               WHERE aa.album_id = e.id AND aa.role = 'primary' ORDER BY aa.position LIMIT 1) \
             COLLATE NOCASE {dir}, e.first_release_date {dir}, \
             COALESCE(e.sort_title, e.title) COLLATE NOCASE {dir}"
        )),
        (FocusEntity::Albums, FocusSort::Title) => Fragment::new(format!(
            "COALESCE(e.sort_title, e.title) COLLATE NOCASE {dir}"
        )),
        (FocusEntity::Albums, FocusSort::ReleaseDate) => Fragment::new(format!(
            "(COALESCE(e.first_release_date, '') = '') ASC, e.first_release_date {dir}, \
             COALESCE(e.sort_title, e.title) COLLATE NOCASE ASC"
        )),
        (FocusEntity::Albums, FocusSort::TrackCount) => Fragment::new(format!(
            "(SELECT COUNT(*) FROM track_entity WHERE album_id = e.id) {dir}"
        )),
        (FocusEntity::Albums, FocusSort::Duration) => Fragment::new(format!(
            "(SELECT SUM(duration_seconds) FROM track_entity WHERE album_id = e.id) {dir}"
        )),
        (FocusEntity::Albums, FocusSort::Quality) => Fragment::new(format!(
            "{} {dir}",
            quality_score("IN (SELECT id FROM track_entity WHERE album_id = e.id)")
        )),
        (FocusEntity::Albums, FocusSort::MostPlayed | FocusSort::LastPlayed) => {
            joins = Fragment::new(
                " LEFT JOIN (SELECT pt.album_id AS id, COUNT(*) AS plays, MAX(h.listened_at) AS last \
                   FROM listen_history_item h JOIN track_entity pt ON pt.id = h.song_id \
                   WHERE h.user_id = ? AND pt.album_id IS NOT NULL GROUP BY pt.album_id) sp \
                 ON sp.id = e.id",
            )
            .int(user);
            let column = if sort == FocusSort::MostPlayed {
                "COALESCE(sp.plays, 0)"
            } else {
                "COALESCE(sp.last, '')"
            };
            Fragment::new(format!("{column} {dir}"))
        }
        (FocusEntity::Albums | FocusEntity::Artists, FocusSort::DateAdded) => {
            Fragment::new(format!("e.created_at {dir}"))
        }
        (FocusEntity::Songs, FocusSort::Artist) => Fragment::new(format!(
            "(SELECT COALESCE(ar.sort_name, ar.name) FROM track_artist ta \
               JOIN artist_entity ar ON ar.id = ta.artist_id \
               WHERE ta.track_id = e.id AND ta.role = 'primary' ORDER BY ta.position LIMIT 1) \
             COLLATE NOCASE {dir}, \
             (SELECT first_release_date FROM album_entity WHERE id = e.album_id) {dir}, \
             (SELECT COALESCE(sort_title, title) FROM album_entity WHERE id = e.album_id) \
             COLLATE NOCASE {dir}, e.disc_number {dir}, e.track_number {dir}"
        )),
        (FocusEntity::Songs, FocusSort::Title) => Fragment::new(format!(
            "COALESCE(e.sort_title, e.title) COLLATE NOCASE {dir}"
        )),
        (FocusEntity::Songs, FocusSort::Album) => Fragment::new(format!(
            "(e.album_id IS NULL) ASC, \
             (SELECT COALESCE(sort_title, title) FROM album_entity WHERE id = e.album_id) \
             COLLATE NOCASE {dir}, e.album_id, e.disc_number ASC, e.track_number ASC"
        )),
        (FocusEntity::Songs, FocusSort::ReleaseDate) => Fragment::new(format!(
            "(SELECT COALESCE(first_release_date, '') FROM album_entity WHERE id = e.album_id) {dir}, \
             e.album_id, e.disc_number ASC, e.track_number ASC"
        )),
        (FocusEntity::Songs, FocusSort::DateAdded) => Fragment::new(format!(
            "e.created_at {dir}, e.album_id, e.track_number ASC"
        )),
        (FocusEntity::Songs, FocusSort::Duration) => {
            Fragment::new(format!("e.duration_seconds {dir}"))
        }
        (FocusEntity::Songs, FocusSort::Quality) => {
            Fragment::new(format!("{} {dir}", quality_score("= e.id")))
        }
        (FocusEntity::Songs, FocusSort::MostPlayed | FocusSort::LastPlayed) => {
            joins = Fragment::new(
                " LEFT JOIN (SELECT song_id AS id, COUNT(*) AS plays, MAX(listened_at) AS last \
                   FROM listen_history_item WHERE user_id = ? GROUP BY song_id) sp ON sp.id = e.id",
            )
            .int(user);
            let column = if sort == FocusSort::MostPlayed {
                "COALESCE(sp.plays, 0)"
            } else {
                "COALESCE(sp.last, '')"
            };
            Fragment::new(format!("{column} {dir}"))
        }
        (FocusEntity::Artists | FocusEntity::Composers, FocusSort::Title | FocusSort::Artist) => {
            Fragment::new(format!(
                "COALESCE(e.sort_name, e.name) COLLATE NOCASE {dir}"
            ))
        }
        (FocusEntity::Composers, FocusSort::TrackCount) => Fragment::new(format!(
            "(SELECT COUNT(DISTINCT track_id) FROM track_artist \
               WHERE artist_id = e.id AND role = 'composer') {dir}"
        )),
        (FocusEntity::Composers, FocusSort::DateAdded) => Fragment::new(format!(
            "(SELECT MAX(ct.created_at) FROM track_artist ca \
               JOIN track_entity ct ON ct.id = ca.track_id \
               WHERE ca.artist_id = e.id AND ca.role = 'composer') {dir}"
        )),
        (FocusEntity::Composers, FocusSort::MostPlayed | FocusSort::LastPlayed) => {
            joins = Fragment::new(
                " LEFT JOIN (SELECT ta.artist_id AS id, COUNT(*) AS plays, MAX(h.listened_at) AS last \
                   FROM listen_history_item h JOIN track_artist ta \
                     ON ta.track_id = h.song_id AND ta.role = 'composer' \
                   WHERE h.user_id = ? GROUP BY ta.artist_id) sp ON sp.id = e.id",
            )
            .int(user);
            let column = if sort == FocusSort::MostPlayed {
                "COALESCE(sp.plays, 0)"
            } else {
                "COALESCE(sp.last, '')"
            };
            Fragment::new(format!("{column} {dir}"))
        }
        (FocusEntity::Artists, FocusSort::TrackCount) => Fragment::new(format!(
            "(SELECT COUNT(DISTINCT track_id) FROM track_artist \
               WHERE artist_id = e.id AND role IN ('primary', 'featured')) {dir}"
        )),
        (FocusEntity::Artists, FocusSort::MostPlayed | FocusSort::LastPlayed) => {
            joins = Fragment::new(
                " LEFT JOIN (SELECT ta.artist_id AS id, COUNT(*) AS plays, MAX(h.listened_at) AS last \
                   FROM listen_history_item h JOIN track_artist ta \
                     ON ta.track_id = h.song_id AND ta.role = 'primary' \
                   WHERE h.user_id = ? GROUP BY ta.artist_id) sp ON sp.id = e.id",
            )
            .int(user);
            let column = if sort == FocusSort::MostPlayed {
                "COALESCE(sp.plays, 0)"
            } else {
                "COALESCE(sp.last, '')"
            };
            Fragment::new(format!("{column} {dir}"))
        }
        // effective_sort() never yields the remaining combinations.
        (FocusEntity::Albums, FocusSort::Album)
        | (FocusEntity::Songs, FocusSort::TrackCount)
        | (
            FocusEntity::Artists | FocusEntity::Composers,
            FocusSort::Album | FocusSort::ReleaseDate | FocusSort::Duration | FocusSort::Quality,
        ) => Fragment::new("1"),
    };
    let mut order_by = order;
    order_by.push(", e.id ASC");
    SortPlan {
        joins,
        order: order_by,
    }
}

#[derive(QueryableByName)]
struct IdRow {
    #[diesel(sql_type = Text)]
    id: String,
}

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

/// One page of focused entity ids, in sort order, with the match count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusIds {
    pub ids: Vec<String>,
    pub total: i64,
}

pub fn select_sql(query: &FocusQuery, user_id: i32, offset: usize, limit: usize) -> Fragment {
    let plan = sort_plan(query, user_id);
    let mut sql = Fragment::new(format!(
        "SELECT e.id AS id FROM {} e",
        entity_table(query.entity)
    ));
    sql.append(plan.joins);
    sql.push(" WHERE ");
    sql.append(where_condition(query, user_id));
    sql.push(" ORDER BY ");
    sql.append(plan.order);
    sql.push(" LIMIT ? OFFSET ?");
    sql.int(limit as i64).int(offset as i64)
}

pub fn count_sql(query: &FocusQuery, user_id: i32) -> Fragment {
    let mut sql = Fragment::new(format!(
        "SELECT COUNT(*) AS count FROM {} e WHERE ",
        entity_table(query.entity)
    ));
    sql.append(where_condition(query, user_id));
    sql
}

/// Runs a focus and returns one page of ids plus the total match count.
pub fn focus_ids(
    connection: &mut SqliteConnection,
    query: &FocusQuery,
    user_id: i32,
    offset: usize,
    limit: usize,
) -> Result<FocusIds, Box<dyn std::error::Error + Send + Sync>> {
    query.validate()?;
    let limit = limit.clamp(1, MAX_RESOLVED_IDS);
    let ids = select_sql(query, user_id, offset, limit)
        .into_query()
        .load::<IdRow>(connection)?
        .into_iter()
        .map(|row| row.id)
        .collect();
    let total = count_sql(query, user_id)
        .into_query()
        .get_result::<CountRow>(connection)?
        .count;
    Ok(FocusIds { ids, total })
}

/// Number of entities a focus browses before its filters apply: the whole
/// catalog, or the scoped subset (a composer's works, an album's tracks).
pub fn library_total(
    connection: &mut SqliteConnection,
    focus: &FocusQuery,
) -> Result<i64, diesel::result::Error> {
    let query = FocusQuery {
        entity: focus.entity,
        scope: focus.scope.clone(),
        ..FocusQuery::default()
    };
    count_sql(&query, 0)
        .into_query()
        .get_result::<CountRow>(connection)
        .map(|row| row.count)
}

/// Facet columns a focus panel can show.
pub const FACET_KEYS: [&str; 13] = [
    "genre",
    "year",
    "artist",
    "credit",
    "quality",
    "format",
    "sample_rate",
    "bit_depth",
    "label",
    "release_type",
    "tag",
    "source",
    "status",
];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FacetValue {
    pub value: String,
    pub label: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Facet {
    pub key: String,
    /// Credit role for `credit` facets (one facet per role).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Distinct values before the per-facet limit.
    pub total: i64,
    pub values: Vec<FacetValue>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FocusFacets {
    /// Entities matching the focus.
    pub total: i64,
    pub facets: Vec<Facet>,
}

/// Which facets to compute and how many values each returns.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FacetRequest {
    /// Facet keys to compute; all of them when empty.
    #[serde(default)]
    pub only: Vec<String>,
    /// Case-insensitive substring filter over facet labels.
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(QueryableByName)]
struct FacetRow {
    #[diesel(sql_type = Text)]
    facet: String,
    #[diesel(sql_type = Nullable<Text>)]
    role: Option<String>,
    #[diesel(sql_type = Text)]
    value: String,
    #[diesel(sql_type = Text)]
    label: String,
    #[diesel(sql_type = BigInt)]
    count: i64,
    #[diesel(sql_type = BigInt)]
    total: i64,
}

/// `(focused entity, track)` pairs for the focused set.
fn scope_pairs(entity: FocusEntity) -> &'static str {
    match entity {
        FocusEntity::Songs => "SELECT id AS entity_id, id AS track_id FROM focused",
        FocusEntity::Albums => {
            "SELECT album_id AS entity_id, id AS track_id FROM track_entity \
             WHERE album_id IN (SELECT id FROM focused)"
        }
        FocusEntity::Artists => {
            "SELECT artist_id AS entity_id, track_id FROM track_artist \
             WHERE role IN ('primary', 'featured') AND artist_id IN (SELECT id FROM focused) \
             UNION SELECT aa.artist_id, t.id FROM album_artist aa \
             JOIN track_entity t ON t.album_id = aa.album_id \
             WHERE aa.role = 'primary' AND aa.artist_id IN (SELECT id FROM focused)"
        }
        FocusEntity::Composers => {
            "SELECT artist_id AS entity_id, track_id FROM track_artist \
             WHERE role = 'composer' AND artist_id IN (SELECT id FROM focused)"
        }
    }
}

/// A grouped facet branch: `source` yields `(entity_id, value, label)` rows.
fn grouped_branch(
    key: &str,
    source: Fragment,
    search: Option<&str>,
    limit: usize,
    order: &str,
) -> Fragment {
    let mut branch = Fragment::new(format!(
        "SELECT * FROM (SELECT '{key}' AS facet, NULL AS role, CAST(value AS TEXT) AS value, \
         CAST(label AS TEXT) AS label, COUNT(DISTINCT entity_id) AS count, \
         COUNT(*) OVER () AS total FROM ("
    ));
    branch.append(source);
    branch.push(") WHERE value IS NOT NULL AND label IS NOT NULL AND label <> ''");
    if let Some(search) = search {
        branch.push(" AND label LIKE ? ESCAPE '\\'");
        branch.binds.push(Bind::Text(like_pattern(search)));
    }
    branch.push(&format!(" GROUP BY value ORDER BY {order} LIMIT {limit})"));
    branch
}

fn primary_file_pairs(columns: &str, condition: &str) -> Fragment {
    Fragment::new(format!(
        "SELECT ft.entity_id AS entity_id, {columns} FROM ft \
         JOIN track_file pf ON pf.track_id = ft.track_id AND pf.is_primary = 1 \
         JOIN file_entry pfe ON pfe.id = pf.file_id \
         LEFT JOIN raw_file_metadata pm ON pm.file_id = pf.file_id WHERE {condition}"
    ))
}

fn facet_branch(
    key: &str,
    entity: FocusEntity,
    user_id: i32,
    search: Option<&str>,
    limit: usize,
) -> Option<Fragment> {
    let user = i64::from(user_id);
    let by_count = "count DESC, label COLLATE NOCASE ASC";
    let branch = match key {
        "genre" => {
            let source = if entity == FocusEntity::Albums {
                Fragment::new(
                    "SELECT ag.album_id AS entity_id, ge.name AS value, ge.name AS label \
                     FROM album_genre ag JOIN genre_entity ge ON ge.id = ag.genre_id \
                     WHERE ag.album_id IN (SELECT id FROM focused)",
                )
            } else {
                Fragment::new(
                    "SELECT ft.entity_id AS entity_id, ge.name AS value, ge.name AS label FROM ft \
                     JOIN track_genre tg ON tg.track_id = ft.track_id \
                     JOIN genre_entity ge ON ge.id = tg.genre_id \
                     UNION SELECT ft.entity_id, ge.name, ge.name FROM ft \
                     JOIN track_entity gt ON gt.id = ft.track_id \
                     JOIN album_genre ag ON ag.album_id = gt.album_id \
                     JOIN genre_entity ge ON ge.id = ag.genre_id",
                )
            };
            grouped_branch(key, source, search, limit, by_count)
        }
        "year" => {
            let source = if entity == FocusEntity::Albums {
                Fragment::new(format!(
                    "SELECT id AS entity_id, {RELEASE_YEAR} AS value, {RELEASE_YEAR} AS label \
                     FROM album_entity WHERE id IN (SELECT id FROM focused) AND {RELEASE_YEAR} > 0"
                ))
            } else {
                Fragment::new(format!(
                    "SELECT ft.entity_id AS entity_id, {RELEASE_YEAR} AS value, \
                     {RELEASE_YEAR} AS label FROM ft \
                     JOIN track_entity yt ON yt.id = ft.track_id \
                     JOIN album_entity ON album_entity.id = yt.album_id WHERE {RELEASE_YEAR} > 0"
                ))
            };
            grouped_branch(
                key,
                source,
                search,
                MAX_YEAR_VALUES,
                "CAST(value AS INTEGER) ASC",
            )
        }
        "format" => grouped_branch(
            key,
            primary_file_pairs(
                &format!("({FORMAT_EXPR}) AS value, ({FORMAT_EXPR}) AS label"),
                "1",
            ),
            search,
            limit,
            by_count,
        ),
        "quality" => {
            let tier = quality_tier_expr();
            grouped_branch(
                key,
                primary_file_pairs(&format!("({tier}) AS value, ({tier}) AS label"), "1"),
                None,
                limit,
                by_count,
            )
        }
        "sample_rate" => grouped_branch(
            key,
            primary_file_pairs(
                "pm.sample_rate AS value, pm.sample_rate AS label",
                "pm.sample_rate > 0",
            ),
            None,
            limit,
            "CAST(value AS INTEGER) DESC",
        ),
        "bit_depth" => grouped_branch(
            key,
            primary_file_pairs(
                "pm.bit_depth AS value, pm.bit_depth AS label",
                "pm.bit_depth > 0",
            ),
            None,
            limit,
            "CAST(value AS INTEGER) DESC",
        ),
        "label" => grouped_branch(
            key,
            primary_file_pairs(
                "trim(pm.label) AS value, trim(pm.label) AS label",
                "pm.label IS NOT NULL",
            ),
            search,
            limit,
            by_count,
        ),
        "artist" => grouped_branch(
            key,
            Fragment::new(
                "SELECT ft.entity_id AS entity_id, ar.id AS value, ar.name AS label FROM ft \
                 JOIN track_artist ta ON ta.track_id = ft.track_id AND ta.role = 'primary' \
                 JOIN artist_entity ar ON ar.id = ta.artist_id",
            ),
            search,
            limit,
            by_count,
        ),
        "release_type" => grouped_branch(
            key,
            primary_file_pairs(
                "pm.release_type AS value, pm.release_type AS label",
                "pm.release_type IS NOT NULL",
            ),
            None,
            limit,
            by_count,
        ),
        "tag" => grouped_branch(
            key,
            Fragment::new(format!(
                "SELECT entity_id, g.id AS value, g.name AS label FROM ( \
                   SELECT ft.entity_id AS entity_id, i.tag_id AS tag_id FROM ft \
                   JOIN user_tag_item i ON i.entity_type = 'track' AND i.entity_id = ft.track_id \
                   UNION SELECT ft.entity_id, i.tag_id FROM ft \
                   JOIN track_entity it ON it.id = ft.track_id \
                   JOIN user_tag_item i ON i.entity_type = 'album' AND i.entity_id = it.album_id \
                   UNION SELECT ft.entity_id, i.tag_id FROM ft \
                   JOIN track_artist ita ON ita.track_id = ft.track_id AND ita.role IN {ARTIST_ROLES_SQL} \
                   JOIN user_tag_item i ON i.entity_type = 'artist' AND i.entity_id = ita.artist_id \
                 ) tagged JOIN user_tag g ON g.id = tagged.tag_id WHERE g.user_id = ?"
            ))
            .int(user),
            search,
            limit,
            by_count,
        ),
        "source" => grouped_branch(
            key,
            Fragment::new(
                "SELECT ft.entity_id AS entity_id, lr.id AS value, \
                 COALESCE(NULLIF(lr.display_name, ''), lr.path) AS label FROM ft \
                 JOIN track_file pf ON pf.track_id = ft.track_id AND pf.is_primary = 1 \
                 JOIN file_entry pfe ON pfe.id = pf.file_id \
                 JOIN library_root lr ON lr.id = pfe.root_id",
            ),
            None,
            limit,
            by_count,
        ),
        "status" => Fragment::new(
            "SELECT 'status' AS facet, NULL AS role, 'liked' AS value, 'liked' AS label, \
               COUNT(DISTINCT entity_id) AS count, 2 AS total FROM ft \
               WHERE track_id IN (SELECT song_id FROM favorite_song WHERE user_id = ?) \
             UNION ALL SELECT 'status', NULL, 'played', 'played', COUNT(DISTINCT entity_id), 2 \
               FROM ft WHERE track_id IN (SELECT song_id FROM listen_history_item WHERE user_id = ? \
                 UNION SELECT song_id FROM user_track_preference \
                 WHERE user_id = ? AND (qualified_plays > 0 OR completions > 0))",
        )
        .int(user)
        .int(user)
        .int(user),
        "credit" => {
            let mut branch = Fragment::new(format!(
                "SELECT facet, role, value, label, count, total FROM ( \
                   SELECT 'credit' AS facet, role, value, label, \
                     COUNT(DISTINCT entity_id) AS count, \
                     COUNT(*) OVER (PARTITION BY role) AS total, \
                     ROW_NUMBER() OVER (PARTITION BY role \
                       ORDER BY COUNT(DISTINCT entity_id) DESC, label COLLATE NOCASE) AS rn \
                   FROM (SELECT entity_id, {CREDIT_COLUMN_SQL} AS role, value, label FROM ( \
                     SELECT ft.entity_id AS entity_id, \
                     CASE WHEN instr(ta.role, ':') > 0 \
                       THEN substr(ta.role, 1, instr(ta.role, ':') - 1) ELSE ta.role END AS base_role, \
                     ar.id AS value, ar.name AS label FROM ft \
                     JOIN track_artist ta ON ta.track_id = ft.track_id \
                       AND ta.role NOT IN ('primary', 'featured') \
                     JOIN artist_entity ar ON ar.id = ta.artist_id))"
            ));
            if let Some(search) = search {
                branch.push(" WHERE label LIKE ? ESCAPE '\\'");
                branch.binds.push(Bind::Text(like_pattern(search)));
            }
            branch.push(&format!(
                " GROUP BY role, value) WHERE rn <= {limit}"
            ));
            branch
        }
        _ => return None,
    };
    Some(branch)
}

pub fn facets_sql(query: &FocusQuery, user_id: i32, request: &FacetRequest) -> Fragment {
    let entity = query.entity;
    let limit = request
        .limit
        .unwrap_or(DEFAULT_FACET_VALUES)
        .clamp(1, MAX_FACET_VALUES);
    let search = request
        .search
        .as_deref()
        .map(str::trim)
        .filter(|search| !search.is_empty());
    let mut sql = Fragment::new(format!(
        "WITH focused AS (SELECT e.id AS id FROM {} e WHERE ",
        entity_table(entity)
    ));
    sql.append(where_condition(query, user_id));
    sql.push(&format!("), ft AS ({}) ", scope_pairs(entity)));
    sql.push(
        "SELECT 'total' AS facet, NULL AS role, '' AS value, '' AS label, \
         COUNT(*) AS count, 1 AS total FROM focused",
    );
    for key in FACET_KEYS {
        if !request.only.is_empty() && !request.only.iter().any(|only| only == key) {
            continue;
        }
        if let Some(branch) = facet_branch(key, entity, user_id, search, limit) {
            sql.push(" UNION ALL SELECT * FROM (");
            sql.append(branch);
            sql.push(")");
        }
    }
    sql
}

fn format_label(value: &str) -> String {
    match value {
        "flac" => "FLAC".into(),
        "alac" => "ALAC".into(),
        "aac" => "AAC".into(),
        "mp3" => "MP3".into(),
        "vorbis" => "Ogg Vorbis".into(),
        "opus" => "Opus".into(),
        "wav" => "WAV".into(),
        "aiff" | "aif" => "AIFF".into(),
        "pcm" | "pcm_float" => "PCM".into(),
        "wavpack" => "WavPack".into(),
        "ape" => "Monkey's Audio".into(),
        other => other.to_uppercase(),
    }
}

fn sample_rate_label(value: &str) -> String {
    match value.parse::<f64>() {
        Ok(hz) => {
            let khz = hz / 1000.0;
            if (khz - khz.round()).abs() < f64::EPSILON {
                format!("{khz:.0} kHz")
            } else {
                format!("{khz:.1} kHz")
            }
        }
        Err(_) => value.to_string(),
    }
}

fn title_case(value: &str) -> String {
    let mut characters = value.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}

/// Display name of a canonical MusicBrainz release type.
pub fn release_type_label(value: &str) -> String {
    match value {
        "album" => "Album".into(),
        "single" => "Single".into(),
        "ep" => "EP".into(),
        "broadcast" => "Broadcast".into(),
        "other" => "Other".into(),
        "compilation" => "Compilation".into(),
        "soundtrack" => "Soundtrack".into(),
        "live" => "Live".into(),
        "remix" => "Remix".into(),
        "spokenword" => "Spoken word".into(),
        "interview" => "Interview".into(),
        "audiobook" => "Audiobook".into(),
        "audio drama" => "Audio drama".into(),
        "demo" => "Demo".into(),
        "dj-mix" => "DJ-mix".into(),
        "mixtape/street" => "Mixtape/Street".into(),
        other => title_case(other),
    }
}

fn source_label(value: &str) -> String {
    std::path::Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(value)
        .to_string()
}

/// Human labels for facet values whose stored form is a code.
pub fn display_label(key: &str, value: &str, label: &str) -> String {
    match key {
        "format" => format_label(value),
        "quality" => match value {
            "hires" => "Hi-Res".into(),
            "cd" => "CD quality".into(),
            "lossy" => "Lossy".into(),
            _ => "Unknown".into(),
        },
        "sample_rate" => sample_rate_label(value),
        "bit_depth" => format!("{value}-bit"),
        "release_type" => release_type_label(value),
        "source" => source_label(label),
        "status" => match value {
            "liked" => "Liked".into(),
            "played" => "Played".into(),
            "unplayed" => "Unplayed".into(),
            _ => label.into(),
        },
        _ => label.into(),
    }
}

/// Computes facet counts over the entities a focus matches.
pub fn focus_facets(
    connection: &mut SqliteConnection,
    query: &FocusQuery,
    user_id: i32,
    request: &FacetRequest,
) -> Result<FocusFacets, Box<dyn std::error::Error + Send + Sync>> {
    query.validate()?;
    if request
        .search
        .as_deref()
        .is_some_and(|search| search.chars().count() > MAX_TEXT_CHARS)
    {
        return Err(invalid("The facet search is too long.").into());
    }
    let rows = facets_sql(query, user_id, request)
        .into_query()
        .load::<FacetRow>(connection)?;
    let mut total = 0;
    let mut facets = BTreeMap::<(usize, String), Facet>::new();
    for row in rows {
        if row.facet == "total" {
            total = row.count;
            continue;
        }
        let order = FACET_KEYS
            .iter()
            .position(|key| *key == row.facet)
            .unwrap_or(FACET_KEYS.len());
        let role_order = row.role.as_deref().map(credit_column_rank).unwrap_or(0);
        let facet = facets
            .entry((
                order * 100 + role_order,
                row.role.clone().unwrap_or_default(),
            ))
            .or_insert_with(|| Facet {
                key: row.facet.clone(),
                role: row.role.clone(),
                total: row.total,
                values: Vec::new(),
            });
        facet.values.push(FacetValue {
            label: display_label(&row.facet, &row.value, &row.label),
            value: row.value,
            count: row.count,
        });
    }
    let mut facets = facets.into_values().collect::<Vec<_>>();
    if let Some(status) = facets.iter_mut().find(|facet| facet.key == "status") {
        let played = status
            .values
            .iter()
            .find(|value| value.value == "played")
            .map_or(0, |value| value.count);
        status.values.push(FacetValue {
            value: "unplayed".into(),
            label: "Unplayed".into(),
            count: (total - played).max(0),
        });
        status.total = 3;
    }
    Ok(FocusFacets { total, facets })
}

/// A person shown in the Composers browser.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FocusComposer {
    pub id: String,
    pub name: String,
    /// Tracks the person is credited as composer on.
    pub track_count: i64,
    /// Distinct compositions (tagged works, or track titles without one).
    pub work_count: i64,
    /// Albums with at least one of their compositions.
    pub album_count: i64,
}

#[derive(QueryableByName)]
struct ComposerRow {
    #[diesel(sql_type = Text)]
    id: String,
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = BigInt)]
    track_count: i64,
    #[diesel(sql_type = BigInt)]
    work_count: i64,
    #[diesel(sql_type = BigInt)]
    album_count: i64,
}

const COMPOSER_STATS: &str = "SELECT ar.id AS id, ar.name AS name, \
     COUNT(DISTINCT ta.track_id) AS track_count, \
     COUNT(DISTINCT COALESCE(lower(replace(replace(trim(pm.work), '.', ''), ',', '')), 'track:' || t.normalized_title)) AS work_count, \
     COUNT(DISTINCT t.album_id) AS album_count \
     FROM artist_entity ar \
     JOIN track_artist ta ON ta.artist_id = ar.id AND ta.role = 'composer' \
     JOIN track_entity t ON t.id = ta.track_id \
     LEFT JOIN track_file pf ON pf.track_id = t.id AND pf.is_primary = 1 \
     LEFT JOIN raw_file_metadata pm ON pm.file_id = pf.file_id";

/// Composer rows for focused composer ids, in the given order.
pub fn composer_rows(
    connection: &mut SqliteConnection,
    ids: &[String],
) -> Result<Vec<FocusComposer>, diesel::result::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut sql = Fragment::new(format!("{COMPOSER_STATS} WHERE ar.id IN ("));
    for (index, id) in ids.iter().enumerate() {
        if index > 0 {
            sql.push(", ");
        }
        sql.push("?");
        sql.binds.push(Bind::Text(id.clone()));
    }
    sql.push(") GROUP BY ar.id");
    let mut rows = sql
        .into_query()
        .load::<ComposerRow>(connection)?
        .into_iter()
        .map(|row| {
            (
                row.id.clone(),
                FocusComposer {
                    id: row.id,
                    name: row.name,
                    track_count: row.track_count,
                    work_count: row.work_count,
                    album_count: row.album_count,
                },
            )
        })
        .collect::<std::collections::HashMap<_, _>>();
    Ok(ids.iter().filter_map(|id| rows.remove(id)).collect())
}

/// How many tracks a person is credited on, per credit facet column.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PersonRole {
    pub role: String,
    pub track_count: i64,
}

/// A credited person, for the header of a composer or performer page.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FocusPerson {
    pub id: String,
    pub name: String,
    /// Roles in display order; `primary` covers main and featured artists.
    pub roles: Vec<PersonRole>,
    pub composer: Option<FocusComposer>,
}

#[derive(QueryableByName)]
struct PersonRoleRow {
    #[diesel(sql_type = Text)]
    role: String,
    #[diesel(sql_type = BigInt)]
    track_count: i64,
}

#[derive(QueryableByName)]
struct NameRow {
    #[diesel(sql_type = Text)]
    name: String,
}

/// Looks up a person by artist id with their credit roles.
pub fn focus_person(
    connection: &mut SqliteConnection,
    id: &str,
) -> Result<Option<FocusPerson>, diesel::result::Error> {
    let name = Fragment::new("SELECT name FROM artist_entity WHERE id = ?")
        .text(id)
        .into_query()
        .get_result::<NameRow>(connection)
        .optional()?;
    let Some(name) = name else {
        return Ok(None);
    };
    let mut roles = Fragment::new(format!(
        "SELECT role, COUNT(DISTINCT track_id) AS track_count FROM ( \
           SELECT track_id, CASE WHEN base_role IN ('primary', 'featured') THEN 'primary' \
             ELSE {CREDIT_COLUMN_SQL} END AS role FROM ( \
             SELECT track_id, CASE WHEN instr(role, ':') > 0 \
               THEN substr(role, 1, instr(role, ':') - 1) ELSE role END AS base_role \
             FROM track_artist WHERE artist_id = ?)) GROUP BY role"
    ))
    .text(id)
    .into_query()
    .load::<PersonRoleRow>(connection)?
    .into_iter()
    .map(|row| PersonRole {
        role: row.role,
        track_count: row.track_count,
    })
    .collect::<Vec<_>>();
    roles.sort_by_key(|role| {
        if role.role == "primary" {
            0
        } else {
            credit_column_rank(&role.role) + 1
        }
    });
    let composer = if roles.iter().any(|role| role.role == "composer") {
        composer_rows(connection, &[id.to_string()])?.pop()
    } else {
        None
    };
    Ok(Some(FocusPerson {
        id: id.to_string(),
        name: name.name,
        roles,
        composer,
    }))
}

/// Largest number of focused tracks grouped into compositions at once.
pub const MAX_COMPOSITION_TRACKS: usize = 5_000;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NamedRef {
    pub id: String,
    pub name: String,
}

/// One track of a recording, labelled with its movement.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompositionTrack {
    pub id: String,
    pub title: String,
    /// Movement name, from the MOVEMENTNAME tag or the title after the work.
    pub movement: Option<String>,
    pub movement_number: Option<i64>,
    pub duration_seconds: f64,
}

/// One album's performance of a composition.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompositionRecording {
    pub album_id: Option<String>,
    pub album_name: Option<String>,
    pub artist_id: Option<String>,
    pub artist_name: Option<String>,
    /// Album artwork path, filled in from the library cache.
    pub cover_path: Option<String>,
    pub release_year: Option<String>,
    /// Conductors, ensembles and soloists credited on the recording.
    pub performers: Vec<String>,
    pub duration_seconds: f64,
    pub tracks: Vec<CompositionTrack>,
}

/// A composition with every recording of it in the focused set.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Composition {
    pub key: String,
    pub title: String,
    /// Whether the title comes from a WORK tag (otherwise it is derived).
    pub tagged: bool,
    pub composers: Vec<NamedRef>,
    pub movement_count: usize,
    pub recordings: Vec<CompositionRecording>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FocusCompositions {
    /// Compositions in the focused set.
    pub total: usize,
    /// Focused tracks the compositions were built from.
    pub track_total: i64,
    /// Whether more tracks matched than were grouped.
    pub truncated: bool,
    pub compositions: Vec<Composition>,
}

#[derive(QueryableByName, Debug, Clone)]
struct CompositionRow {
    #[diesel(sql_type = Text)]
    id: String,
    #[diesel(sql_type = Text)]
    title: String,
    #[diesel(sql_type = Nullable<Text>)]
    album_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    album_name: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    release_date: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    artist_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    artist_name: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Double)]
    duration_seconds: f64,
    #[diesel(sql_type = Nullable<Text>)]
    work: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    movement: Option<String>,
    #[diesel(sql_type = Nullable<BigInt>)]
    movement_number: Option<i64>,
}

#[derive(QueryableByName)]
struct TrackCreditRow {
    #[diesel(sql_type = Text)]
    track_id: String,
    #[diesel(sql_type = Text)]
    role: String,
    #[diesel(sql_type = Text)]
    artist_id: String,
    #[diesel(sql_type = Text)]
    name: String,
}

/// Case- and punctuation-insensitive key for grouping titles.
pub fn work_key(title: &str) -> String {
    title
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Splits `Work: Movement` titles, e.g. "Symphony No. 5: I. Allegro".
fn split_work_title(title: &str) -> Option<(&str, &str)> {
    let (work, movement) = title.split_once(": ")?;
    let (work, movement) = (work.trim(), movement.trim());
    (work.chars().count() >= 3 && !movement.is_empty()).then_some((work, movement))
}

/// The movement label once the work title is known.
fn movement_label(title: &str, work: &str, tagged: Option<&str>) -> Option<String> {
    if let Some(tagged) = tagged.map(str::trim).filter(|value| !value.is_empty()) {
        return Some(tagged.to_string());
    }
    let (prefix, rest) = split_work_title(title)?;
    (work_key(prefix) == work_key(work)).then(|| rest.to_string())
}

/// Groups focused tracks into compositions, Roon style: tracks that share
/// a WORK tag are one composition and each album holding them is one
/// recording of it, with MOVEMENTNAME / MOVEMENT ordering its tracks.
/// Untagged tracks titled `Work: Movement` are grouped when at least two
/// tracks of the same album share the work; any other track is a
/// composition of its own.
pub fn focus_compositions(
    connection: &mut SqliteConnection,
    query: &FocusQuery,
    user_id: i32,
) -> Result<FocusCompositions, Box<dyn std::error::Error + Send + Sync>> {
    let mut songs = query.as_songs();
    songs.sort = None;
    songs.direction = None;
    songs.validate()?;
    let mut focused =
        Fragment::new("WITH focused AS (SELECT e.id AS id FROM track_entity e WHERE ");
    focused.append(where_condition(&songs, user_id));
    focused.push(&format!(" LIMIT {MAX_COMPOSITION_TRACKS}) "));

    let mut rows_sql = focused.clone();
    rows_sql.push(
        "SELECT t.id AS id, t.title AS title, t.album_id AS album_id, a.title AS album_name, \
           a.first_release_date AS release_date, aa.artist_id AS artist_id, aar.name AS artist_name, \
           CAST(COALESCE(t.duration_seconds, 0) AS REAL) AS duration_seconds, \
           NULLIF(trim(pm.work), '') AS work, NULLIF(trim(pm.movement), '') AS movement, \
           CAST(pm.movement_number AS BIGINT) AS movement_number \
         FROM focused f JOIN track_entity t ON t.id = f.id \
         LEFT JOIN album_entity a ON a.id = t.album_id \
         LEFT JOIN album_artist aa ON aa.album_id = t.album_id AND aa.role = 'primary' \
           AND aa.artist_id = (SELECT artist_id FROM album_artist WHERE album_id = t.album_id \
             AND role = 'primary' ORDER BY position LIMIT 1) \
         LEFT JOIN artist_entity aar ON aar.id = aa.artist_id \
         LEFT JOIN track_file pf ON pf.track_id = t.id AND pf.is_primary = 1 \
         LEFT JOIN raw_file_metadata pm ON pm.file_id = pf.file_id \
         ORDER BY t.album_id, t.disc_number, t.track_number, t.id",
    );
    let rows = rows_sql.into_query().load::<CompositionRow>(connection)?;

    let mut credits_sql = focused;
    credits_sql.push(
        "SELECT ta.track_id AS track_id, \
           CASE WHEN instr(ta.role, ':') > 0 THEN substr(ta.role, 1, instr(ta.role, ':') - 1) \
             ELSE ta.role END AS role, ar.id AS artist_id, ar.name AS name \
         FROM track_artist ta JOIN artist_entity ar ON ar.id = ta.artist_id \
         WHERE ta.track_id IN (SELECT id FROM focused) \
           AND (ta.role IN ('composer', 'conductor', 'ensemble') OR ta.role = 'performer' \
             OR ta.role LIKE 'performer:%') \
         ORDER BY ta.position, ar.name",
    );
    let mut composers_by_track = std::collections::HashMap::<String, Vec<NamedRef>>::new();
    let mut performers_by_track = std::collections::HashMap::<String, Vec<(usize, String)>>::new();
    for row in credits_sql
        .into_query()
        .load::<TrackCreditRow>(connection)?
    {
        if row.role == "composer" {
            composers_by_track
                .entry(row.track_id)
                .or_default()
                .push(NamedRef {
                    id: row.artist_id,
                    name: row.name,
                });
        } else {
            let rank = match row.role.as_str() {
                "conductor" => 0,
                "ensemble" => 1,
                _ => 2,
            };
            performers_by_track
                .entry(row.track_id)
                .or_default()
                .push((rank, row.name));
        }
    }

    let track_total = count_sql(&songs, user_id)
        .into_query()
        .get_result::<CountRow>(connection)?
        .count;

    // Untagged `Work: Movement` prefixes shared by two tracks of one album.
    let mut prefix_counts = std::collections::HashMap::<(Option<String>, String), usize>::new();
    for row in rows.iter().filter(|row| row.work.is_none()) {
        if let Some((prefix, _)) = split_work_title(&row.title) {
            *prefix_counts
                .entry((row.album_id.clone(), work_key(prefix)))
                .or_default() += 1;
        }
    }

    struct Building {
        title: String,
        tagged: bool,
        composers: Vec<NamedRef>,
        recordings: Vec<CompositionRecording>,
    }
    let mut order = Vec::<String>::new();
    let mut compositions = std::collections::HashMap::<String, Building>::new();
    for row in rows {
        let (title, tagged) = match row.work.as_deref() {
            Some(work) => (work.to_string(), true),
            None => match split_work_title(&row.title) {
                Some((prefix, _))
                    if prefix_counts
                        .get(&(row.album_id.clone(), work_key(prefix)))
                        .is_some_and(|count| *count >= 2) =>
                {
                    (prefix.to_string(), false)
                }
                _ => (row.title.clone(), false),
            },
        };
        let key = work_key(&title);
        let key = if key.is_empty() {
            format!("track:{}", row.id)
        } else {
            key
        };
        let movement = movement_label(&row.title, &title, row.movement.as_deref())
            .filter(|movement| work_key(movement) != key);
        let building = compositions.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            Building {
                title: title.clone(),
                tagged,
                composers: Vec::new(),
                recordings: Vec::new(),
            }
        });
        building.tagged |= tagged;
        for composer in composers_by_track.remove(&row.id).unwrap_or_default() {
            if !building
                .composers
                .iter()
                .any(|known| known.id == composer.id)
            {
                building.composers.push(composer);
            }
        }
        let recording = match building
            .recordings
            .iter_mut()
            .position(|recording| recording.album_id == row.album_id && row.album_id.is_some())
        {
            Some(index) => &mut building.recordings[index],
            None => {
                building.recordings.push(CompositionRecording {
                    album_id: row.album_id.clone(),
                    album_name: row.album_name.clone(),
                    artist_id: row.artist_id.clone(),
                    artist_name: row.artist_name.clone(),
                    cover_path: None,
                    release_year: row
                        .release_date
                        .as_deref()
                        .map(|date| date.chars().take(4).collect::<String>())
                        .filter(|year| year.len() == 4),
                    performers: Vec::new(),
                    duration_seconds: 0.0,
                    tracks: Vec::new(),
                });
                building
                    .recordings
                    .last_mut()
                    .expect("recording just added")
            }
        };
        let mut performers = performers_by_track.remove(&row.id).unwrap_or_default();
        performers.sort_by_key(|(rank, _)| *rank);
        for (_, name) in performers {
            if recording.performers.len() < 4 && !recording.performers.contains(&name) {
                recording.performers.push(name);
            }
        }
        recording.duration_seconds += row.duration_seconds.max(0.0);
        recording.tracks.push(CompositionTrack {
            id: row.id,
            title: row.title,
            movement,
            movement_number: row.movement_number.filter(|number| *number > 0),
            duration_seconds: row.duration_seconds,
        });
    }

    let mut compositions = order
        .into_iter()
        .filter_map(|key| {
            let mut building = compositions.remove(&key)?;
            for recording in &mut building.recordings {
                // Tagged movement numbers order a recording; disc and track
                // order (already applied) breaks ties.
                recording
                    .tracks
                    .sort_by_key(|track| track.movement_number.unwrap_or(i64::MAX));
            }
            building.recordings.sort_by(|left, right| {
                left.release_year
                    .cmp(&right.release_year)
                    .then_with(|| left.album_name.cmp(&right.album_name))
            });
            let movement_count = building
                .recordings
                .iter()
                .map(|recording| recording.tracks.len())
                .max()
                .unwrap_or(0);
            Some(Composition {
                key,
                title: building.title,
                tagged: building.tagged,
                composers: building.composers,
                movement_count,
                recordings: building.recordings,
            })
        })
        .collect::<Vec<_>>();
    compositions.sort_by(|left, right| {
        left.title
            .to_lowercase()
            .cmp(&right.title.to_lowercase())
            .then_with(|| left.key.cmp(&right.key))
    });
    Ok(FocusCompositions {
        total: compositions.len(),
        track_total,
        truncated: track_total > MAX_COMPOSITION_TRACKS as i64,
        compositions,
    })
}

#[cfg(test)]
mod tests {
    use diesel::Connection;
    use diesel::connection::SimpleConnection;
    use diesel_migrations::MigrationHarness;

    use super::*;

    fn catalog() -> SqliteConnection {
        let mut connection = SqliteConnection::establish(":memory:").expect("focus database");
        connection
            .run_pending_migrations(crate::persistence::connection::MIGRATIONS)
            .expect("focus migrations");
        // Two roots, three artists, four albums, eight tracks with mixed
        // formats, genres, credits and years.
        connection
            .batch_execute(
                "INSERT INTO user(id, username, password) VALUES (1, 'listener', 'hash'), (2, 'other', 'hash');
                 INSERT INTO library_root(id, path) VALUES (1, '/music/Main'), (2, '/music/Qobuz Downloads');
                 INSERT INTO artist_entity(id, name, normalized_name) VALUES
                   ('a-nora', 'Nora Kelly', 'nora kelly'),
                   ('a-abe', 'Abe Rounds', 'abe rounds'),
                   ('a-cima', 'Cimafunk', 'cimafunk'),
                   ('a-comp', 'Anna Hamilton', 'anna hamilton');
                 INSERT INTO album_entity(id, title, normalized_title, primary_type, first_release_date, created_at) VALUES
                   ('al-pop', 'Pop Record', 'pop record', 'Album', '1994-05-01', '2026-01-01 00:00:00'),
                   ('al-jazz', 'Jazz Night', 'jazz night', 'Album', '2021', '2026-02-01 00:00:00'),
                   ('al-single', 'Lone Single', 'lone single', 'Single', '2022-01-01', '2026-03-01 00:00:00'),
                   ('al-hires', 'Studio Masters', 'studio masters', 'Album', '1978', '2026-04-01 00:00:00');
                 INSERT INTO album_artist(album_id, artist_id, role) VALUES
                   ('al-pop', 'a-nora', 'primary'), ('al-jazz', 'a-abe', 'primary'),
                   ('al-single', 'a-cima', 'primary'), ('al-hires', 'a-nora', 'primary');
                 INSERT INTO track_entity(id, title, normalized_title, album_id, track_number, duration_seconds, created_at) VALUES
                   ('t1', 'Sunrise', 'sunrise', 'al-pop', 1, 200, '2026-01-01 00:00:00'),
                   ('t2', 'Sundown', 'sundown', 'al-pop', 2, 210, '2026-01-01 00:00:00'),
                   ('t3', 'Blue Hour', 'blue hour', 'al-jazz', 1, 300, '2026-02-01 00:00:00'),
                   ('t4', 'Late Set', 'late set', 'al-jazz', 2, 320, '2026-02-01 00:00:00'),
                   ('t5', 'Only One', 'only one', 'al-single', 1, 180, '2026-03-01 00:00:00'),
                   ('t6', 'Master One', 'master one', 'al-hires', 1, 400, '2026-04-01 00:00:00'),
                   ('t7', 'Master Two', 'master two', 'al-hires', 2, 410, '2026-04-01 00:00:00'),
                   ('t8', 'Loose Track', 'loose track', NULL, 0, 100, '2026-04-02 00:00:00');
                 INSERT INTO song(id) VALUES ('t1'), ('t2'), ('t3'), ('t4'), ('t5'), ('t6'), ('t7'), ('t8');
                 INSERT INTO track_artist(track_id, artist_id, role) VALUES
                   ('t1', 'a-nora', 'primary'), ('t2', 'a-nora', 'primary'), ('t2', 'a-cima', 'featured'),
                   ('t3', 'a-abe', 'primary'), ('t4', 'a-abe', 'primary'),
                   ('t5', 'a-cima', 'primary'), ('t6', 'a-nora', 'primary'), ('t7', 'a-nora', 'primary'),
                   ('t8', 'a-cima', 'primary'),
                   ('t3', 'a-comp', 'composer'), ('t6', 'a-comp', 'composer'),
                   ('t4', 'a-cima', 'performer:piano'), ('t7', 'a-abe', 'producer');
                 INSERT INTO genre_entity(id, name, normalized_name) VALUES (1, 'Pop/Rock', 'poprock'), (2, 'Jazz', 'jazz'), (3, 'Latin', 'latin');
                 INSERT INTO track_genre(track_id, genre_id) VALUES
                   ('t1', 1), ('t2', 1), ('t3', 2), ('t4', 2), ('t5', 3), ('t6', 1), ('t7', 2), ('t8', 3);
                 INSERT INTO album_genre(album_id, genre_id) VALUES
                   ('al-pop', 1), ('al-jazz', 2), ('al-single', 3), ('al-hires', 1), ('al-hires', 2);
                 INSERT INTO file_entry(id, root_id, path, directory, file_name, extension, size_bytes, modified_at_ns) VALUES
                   (1, 1, '/music/Main/1.flac', '/music/Main', '1.flac', 'flac', 1, 1),
                   (2, 1, '/music/Main/2.mp3', '/music/Main', '2.mp3', 'mp3', 1, 1),
                   (3, 1, '/music/Main/3.flac', '/music/Main', '3.flac', 'flac', 1, 1),
                   (4, 1, '/music/Main/4.m4a', '/music/Main', '4.m4a', 'm4a', 1, 1),
                   (5, 2, '/music/Qobuz Downloads/5.mp3', '/music/Qobuz Downloads', '5.mp3', 'mp3', 1, 1),
                   (6, 2, '/music/Qobuz Downloads/6.flac', '/music/Qobuz Downloads', '6.flac', 'flac', 1, 1),
                   (7, 2, '/music/Qobuz Downloads/7.wav', '/music/Qobuz Downloads', '7.wav', 'wav', 1, 1),
                   (8, 1, '/music/Main/8.ogg', '/music/Main', '8.ogg', 'ogg', 1, 1);
                 INSERT INTO track_file(track_id, file_id) VALUES
                   ('t1', 1), ('t2', 2), ('t3', 3), ('t4', 4), ('t5', 5), ('t6', 6), ('t7', 7), ('t8', 8);
                 INSERT INTO raw_file_metadata(file_id, codec, container, sample_rate, bit_depth, bitrate, label, release_type) VALUES
                   (1, 'flac', 'flac', 44100, 16, 900, 'Sunset Records', 'album'),
                   (2, 'mp3', 'mp3', 44100, NULL, 320, 'Sunset Records', 'album'),
                   (3, 'flac', 'flac', 48000, 16, 1000, 'Blue Note', NULL),
                   (4, 'aac', 'mp4', 44100, NULL, 256, 'Blue Note', NULL),
                   (5, 'mp3', 'mp3', 48000, NULL, 192, NULL, 'single'),
                   (6, 'flac', 'flac', 96000, 24, 3000, 'Masters', 'album'),
                   (7, 'pcm', 'wav', 192000, 24, 9000, 'Masters', 'album'),
                   (8, 'vorbis', 'ogg', 44100, NULL, 160, NULL, NULL);
                 INSERT INTO favorite_song(user_id, song_id) VALUES (1, 't3'), (1, 't6'), (2, 't1');
                 INSERT INTO listen_history_item(user_id, song_id, listened_at) VALUES
                   (1, 't3', '2026-05-01 00:00:00'), (1, 't3', '2026-05-02 00:00:00'),
                   (1, 't3', '2026-05-03 00:00:00'), (1, 't6', '2026-05-04 00:00:00'),
                   (2, 't1', '2026-05-05 00:00:00');
                 INSERT INTO user_tag(id, user_id, name, normalized_name) VALUES (1, 1, 'Late night', 'late night'), (2, 2, 'Theirs', 'theirs');
                 INSERT INTO user_tag_item(tag_id, entity_type, entity_id) VALUES
                   (1, 'album', 'al-jazz'), (1, 'track', 't5'), (2, 'track', 't1');",
            )
            .expect("focus fixture");
        connection
    }

    fn query(entity: FocusEntity, filters: Vec<FocusFilter>) -> FocusQuery {
        FocusQuery {
            entity,
            filters,
            ..FocusQuery::default()
        }
    }

    fn include(criterion: Criterion) -> FocusFilter {
        FocusFilter {
            criterion,
            exclude: false,
        }
    }

    fn exclude(criterion: Criterion) -> FocusFilter {
        FocusFilter {
            criterion,
            exclude: true,
        }
    }

    fn ids(connection: &mut SqliteConnection, query: &FocusQuery) -> Vec<String> {
        let mut ids = focus_ids(connection, query, 1, 0, 100)
            .expect("focus query")
            .ids;
        ids.sort();
        ids
    }

    fn genre(value: &str) -> Criterion {
        Criterion::Genre {
            value: value.into(),
        }
    }

    #[test]
    fn json_filters_round_trip_with_include_and_exclude() {
        let json = serde_json::json!({
            "entity": "albums",
            "match": "any",
            "filters": [
                {"facet": "genre", "value": "Pop/Rock"},
                {"facet": "year", "from": 1990, "to": 1999},
                {"facet": "format", "value": "mp3", "exclude": true},
                {"facet": "credit", "role": "composer", "artist_id": "a-comp", "name": "Anna Hamilton"},
                {"facet": "liked"}
            ],
            "sort": "date_added"
        });
        let query: FocusQuery = serde_json::from_value(json).expect("focus json");
        assert_eq!(query.match_mode, MatchMode::Any);
        assert_eq!(query.filters.len(), 5);
        assert!(query.filters[2].exclude);
        assert_eq!(query.filters[4].criterion, Criterion::Liked);
        assert_eq!(query.effective_direction(), SortDirection::Desc);
        let encoded = serde_json::to_value(&query).expect("encode focus");
        assert_eq!(encoded["filters"][3]["name"], "Anna Hamilton");
        assert_eq!(encoded["filters"][2]["exclude"], true);
    }

    #[test]
    fn validation_bounds_filters_and_values() {
        let mut too_many = query(FocusEntity::Albums, Vec::new());
        too_many.filters = (0..=MAX_FILTERS)
            .map(|_| include(Criterion::Liked))
            .collect();
        assert!(too_many.validate().is_err());
        for criterion in [
            genre("  "),
            Criterion::Year {
                from: Some(2000),
                to: Some(1990),
            },
            Criterion::Year {
                from: None,
                to: None,
            },
            Criterion::Credit {
                role: "drummer".into(),
                artist_id: "a".into(),
                name: None,
            },
            Criterion::Tag {
                tag_id: 0,
                name: None,
            },
            Criterion::AddedWithin { days: 0 },
        ] {
            assert!(
                query(FocusEntity::Albums, vec![include(criterion.clone())])
                    .validate()
                    .is_err(),
                "{criterion:?} should be rejected"
            );
        }
        assert!(
            query(
                FocusEntity::Albums,
                vec![include(Criterion::Credit {
                    role: "any".into(),
                    artist_id: "a".into(),
                    name: None
                })]
            )
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn album_focus_combines_genre_decade_and_format_exclusion() {
        let mut connection = catalog();
        assert_eq!(
            ids(&mut connection, &query(FocusEntity::Albums, Vec::new())),
            ["al-hires", "al-jazz", "al-pop", "al-single"]
        );
        // Pop/Rock albums (Pop Record, Studio Masters).
        assert_eq!(
            ids(
                &mut connection,
                &query(FocusEntity::Albums, vec![include(genre("pop/rock"))])
            ),
            ["al-hires", "al-pop"]
        );
        // ... from the 1990s.
        assert_eq!(
            ids(
                &mut connection,
                &query(
                    FocusEntity::Albums,
                    vec![
                        include(genre("Pop/Rock")),
                        include(Criterion::Year {
                            from: Some(1990),
                            to: Some(1999)
                        })
                    ]
                )
            ),
            ["al-pop"]
        );
        // Excluding MP3 drops albums that contain any MP3 track.
        assert_eq!(
            ids(
                &mut connection,
                &query(
                    FocusEntity::Albums,
                    vec![exclude(Criterion::Format {
                        value: "MP3".into()
                    })]
                )
            ),
            ["al-hires", "al-jazz"]
        );
        // Excluding singles keeps everything else.
        assert_eq!(
            ids(
                &mut connection,
                &query(
                    FocusEntity::Albums,
                    vec![exclude(Criterion::ReleaseType {
                        value: "single".into()
                    })]
                )
            ),
            ["al-hires", "al-jazz", "al-pop"]
        );
    }

    #[test]
    fn songs_match_audio_properties_quality_tiers_and_sources() {
        let mut connection = catalog();
        let songs = |filters| query(FocusEntity::Songs, filters);
        assert_eq!(
            ids(
                &mut connection,
                &songs(vec![include(Criterion::Quality {
                    value: QualityTier::Hires
                })])
            ),
            ["t6", "t7"]
        );
        assert_eq!(
            ids(
                &mut connection,
                &songs(vec![include(Criterion::Quality {
                    value: QualityTier::Cd
                })])
            ),
            ["t1", "t3"]
        );
        assert_eq!(
            ids(
                &mut connection,
                &songs(vec![include(Criterion::Quality {
                    value: QualityTier::Lossy
                })])
            ),
            ["t2", "t4", "t5", "t8"]
        );
        assert_eq!(
            ids(
                &mut connection,
                &songs(vec![include(Criterion::SampleRate {
                    min: Some(88_200),
                    max: None
                })])
            ),
            ["t6", "t7"]
        );
        assert_eq!(
            ids(
                &mut connection,
                &songs(vec![include(Criterion::BitDepth {
                    min: Some(24),
                    max: None
                })])
            ),
            ["t6", "t7"]
        );
        // PCM is reported by container.
        assert_eq!(
            ids(
                &mut connection,
                &songs(vec![include(Criterion::Format {
                    value: "wav".into()
                })])
            ),
            ["t7"]
        );
        assert_eq!(
            ids(
                &mut connection,
                &songs(vec![include(Criterion::Source {
                    root_id: 2,
                    name: None
                })])
            ),
            ["t5", "t6", "t7"]
        );
        assert_eq!(
            ids(
                &mut connection,
                &songs(vec![include(Criterion::Label {
                    value: "blue note".into()
                })])
            ),
            ["t3", "t4"]
        );
        // Songs inherit album genres: Studio Masters is Pop/Rock and Jazz.
        assert_eq!(
            ids(&mut connection, &songs(vec![include(genre("Jazz"))])),
            ["t3", "t4", "t6", "t7"]
        );
    }

    #[test]
    fn credits_tags_liked_and_played_are_scoped_to_the_user() {
        let mut connection = catalog();
        let composer = Criterion::Credit {
            role: "composer".into(),
            artist_id: "a-comp".into(),
            name: None,
        };
        assert_eq!(
            ids(
                &mut connection,
                &query(FocusEntity::Albums, vec![include(composer)])
            ),
            ["al-hires", "al-jazz"]
        );
        let performer = Criterion::Credit {
            role: "performer".into(),
            artist_id: "a-cima".into(),
            name: None,
        };
        assert_eq!(
            ids(
                &mut connection,
                &query(FocusEntity::Songs, vec![include(performer)])
            ),
            ["t4"]
        );
        assert_eq!(
            ids(
                &mut connection,
                &query(
                    FocusEntity::Songs,
                    vec![include(Criterion::Tag {
                        tag_id: 1,
                        name: None
                    })]
                )
            ),
            ["t3", "t4", "t5"]
        );
        // Another user's tag matches nothing for this user.
        assert!(
            ids(
                &mut connection,
                &query(
                    FocusEntity::Songs,
                    vec![include(Criterion::Tag {
                        tag_id: 2,
                        name: None
                    })]
                )
            )
            .is_empty()
        );
        assert_eq!(
            ids(
                &mut connection,
                &query(FocusEntity::Songs, vec![include(Criterion::Liked)])
            ),
            ["t3", "t6"]
        );
        // Unplayed albums: no track has been played by this user.
        assert_eq!(
            ids(
                &mut connection,
                &query(FocusEntity::Albums, vec![exclude(Criterion::Played)])
            ),
            ["al-pop", "al-single"]
        );
    }

    #[test]
    fn match_any_ors_includes_but_keeps_exclusions() {
        let mut connection = catalog();
        let mut any = query(
            FocusEntity::Albums,
            vec![
                include(genre("Latin")),
                include(Criterion::Year {
                    from: Some(1970),
                    to: Some(1979),
                }),
                exclude(Criterion::Liked),
            ],
        );
        any.match_mode = MatchMode::Any;
        assert_eq!(ids(&mut connection, &any), ["al-single"]);
        any.filters.pop();
        assert_eq!(ids(&mut connection, &any), ["al-hires", "al-single"]);
    }

    #[test]
    fn artist_focus_uses_primary_and_featured_appearances() {
        let mut connection = catalog();
        assert_eq!(
            ids(&mut connection, &query(FocusEntity::Artists, Vec::new())),
            ["a-abe", "a-cima", "a-nora"]
        );
        // Contributor-only artists stay out; Cimafunk is featured on a Pop/Rock track.
        assert_eq!(
            ids(
                &mut connection,
                &query(FocusEntity::Artists, vec![include(genre("Pop/Rock"))])
            ),
            ["a-cima", "a-nora"]
        );
        assert_eq!(
            ids(
                &mut connection,
                &query(
                    FocusEntity::Artists,
                    vec![include(Criterion::Quality {
                        value: QualityTier::Hires
                    })]
                )
            ),
            ["a-nora"]
        );
        // Every artist has at least one lossy track.
        assert!(
            ids(
                &mut connection,
                &query(
                    FocusEntity::Artists,
                    vec![exclude(Criterion::Quality {
                        value: QualityTier::Lossy
                    })]
                )
            )
            .is_empty()
        );
    }

    #[test]
    fn sorts_order_pages_and_report_totals() {
        let mut connection = catalog();
        let mut albums = query(FocusEntity::Albums, Vec::new());
        albums.sort = Some(FocusSort::ReleaseDate);
        let page = focus_ids(&mut connection, &albums, 1, 0, 2).expect("first page");
        assert_eq!(page.total, 4);
        assert_eq!(page.ids, ["al-hires", "al-pop"]);
        let page = focus_ids(&mut connection, &albums, 1, 2, 2).expect("second page");
        assert_eq!(page.ids, ["al-jazz", "al-single"]);

        albums.sort = Some(FocusSort::MostPlayed);
        let page = focus_ids(&mut connection, &albums, 1, 0, 4).expect("most played");
        assert_eq!(page.ids[..2], ["al-jazz", "al-hires"]);

        albums.sort = Some(FocusSort::Quality);
        let page = focus_ids(&mut connection, &albums, 1, 0, 1).expect("quality");
        assert_eq!(page.ids, ["al-hires"]);

        albums.sort = Some(FocusSort::DateAdded);
        let page = focus_ids(&mut connection, &albums, 1, 0, 1).expect("date added");
        assert_eq!(page.ids, ["al-hires"]);

        let mut songs = query(FocusEntity::Songs, Vec::new());
        songs.sort = Some(FocusSort::Album);
        let page = focus_ids(&mut connection, &songs, 1, 0, 3).expect("song album sort");
        assert_eq!(page.ids, ["t3", "t4", "t5"]);
        songs.text = Some("master".into());
        assert_eq!(ids(&mut connection, &songs), ["t6", "t7"]);
        songs.text = Some("abe".into());
        assert_eq!(ids(&mut connection, &songs), ["t3", "t4"]);

        let mut random = query(FocusEntity::Songs, Vec::new());
        random.sort = Some(FocusSort::Random);
        random.seed = Some(7);
        let first = focus_ids(&mut connection, &random, 1, 0, 8)
            .expect("random")
            .ids;
        let again = focus_ids(&mut connection, &random, 1, 0, 8)
            .expect("random")
            .ids;
        assert_eq!(first, again, "a seed gives a stable order for paging");
        assert_eq!(first.len(), 8);

        assert_eq!(
            library_total(&mut connection, &query(FocusEntity::Songs, Vec::new())).expect("total"),
            8
        );
    }

    #[test]
    fn facets_count_entities_in_the_focused_set() {
        let mut connection = catalog();
        let facets = focus_facets(
            &mut connection,
            &query(FocusEntity::Albums, Vec::new()),
            1,
            &FacetRequest::default(),
        )
        .expect("facets");
        assert_eq!(facets.total, 4);
        let facet = |key: &str| {
            facets
                .facets
                .iter()
                .find(|facet| facet.key == key)
                .unwrap_or_else(|| panic!("{key} facet"))
        };
        let genres = facet("genre");
        // Ties are broken alphabetically.
        assert_eq!(genres.values[0].label, "Jazz");
        assert_eq!(genres.values[0].count, 2);
        assert_eq!(genres.values[1].label, "Pop/Rock");
        assert_eq!(genres.values[1].count, 2);
        assert_eq!(genres.values[2].label, "Latin");
        let years = facet("year")
            .values
            .iter()
            .map(|value| value.value.as_str())
            .collect::<Vec<_>>();
        assert_eq!(years, ["1978", "1994", "2021", "2022"]);
        let formats = facet("format");
        assert!(
            formats
                .values
                .iter()
                .any(|value| value.value == "flac" && value.label == "FLAC" && value.count == 3)
        );
        let quality = facet("quality");
        assert!(
            quality
                .values
                .iter()
                .any(|value| value.value == "hires" && value.count == 1)
        );
        let rates = facet("sample_rate");
        assert_eq!(rates.values[0].label, "192 kHz");
        assert!(rates.values.iter().any(|value| value.label == "44.1 kHz"));
        let composers = facets
            .facets
            .iter()
            .find(|facet| facet.key == "credit" && facet.role.as_deref() == Some("composer"))
            .expect("composer facet");
        assert_eq!(composers.values[0].label, "Anna Hamilton");
        assert_eq!(composers.values[0].count, 2);
        assert!(
            facets
                .facets
                .iter()
                .any(|facet| facet.key == "credit" && facet.role.as_deref() == Some("performer"))
        );
        let tags = facet("tag");
        assert_eq!(tags.values.len(), 1, "only this user's tags are counted");
        assert_eq!(tags.values[0].label, "Late night");
        assert_eq!(tags.values[0].count, 2);
        let sources = facet("source");
        assert!(
            sources
                .values
                .iter()
                .any(|value| value.label == "Qobuz Downloads" && value.count == 2)
        );
        let status = facet("status");
        let count = |value: &str| {
            status
                .values
                .iter()
                .find(|candidate| candidate.value == value)
                .map(|candidate| candidate.count)
        };
        assert_eq!(count("liked"), Some(2));
        assert_eq!(count("played"), Some(2));
        assert_eq!(count("unplayed"), Some(2));

        // Counts follow the focus.
        let focused = focus_facets(
            &mut connection,
            &query(FocusEntity::Albums, vec![include(genre("Jazz"))]),
            1,
            &FacetRequest {
                only: vec!["genre".into(), "release_type".into()],
                ..FacetRequest::default()
            },
        )
        .expect("focused facets");
        assert_eq!(focused.total, 2);
        assert_eq!(focused.facets.len(), 2);
        let pop = focused.facets[0]
            .values
            .iter()
            .find(|value| value.label == "Pop/Rock")
            .expect("pop count");
        assert_eq!(pop.count, 1);
    }

    #[test]
    fn facet_search_filters_values_and_limits_apply_per_credit_role() {
        let mut connection = catalog();
        let facets = focus_facets(
            &mut connection,
            &query(FocusEntity::Songs, Vec::new()),
            1,
            &FacetRequest {
                only: vec!["artist".into(), "credit".into()],
                search: Some("abe".into()),
                limit: Some(1),
            },
        )
        .expect("searched facets");
        let artist = facets
            .facets
            .iter()
            .find(|facet| facet.key == "artist")
            .expect("artist facet");
        assert_eq!(artist.values.len(), 1);
        assert_eq!(artist.values[0].label, "Abe Rounds");
        assert_eq!(artist.values[0].count, 2);
        let producers = facets
            .facets
            .iter()
            .find(|facet| facet.role.as_deref() == Some("production"))
            .expect("production facet");
        assert_eq!(producers.values[0].label, "Abe Rounds");
        assert!(
            facets
                .facets
                .iter()
                .all(|facet| facet.role.as_deref() != Some("composer"))
        );
    }

    /// Adds conductor, ensemble and soloist credits plus WORK / MOVEMENT
    /// tags to the shared fixture.
    fn classical(connection: &mut SqliteConnection) {
        connection
            .batch_execute(
                "INSERT INTO artist_entity(id, name, normalized_name) VALUES
                   ('a-cond', 'Claudio Abbado', 'claudio abbado'),
                   ('a-ens', 'Berliner Philharmoniker', 'berliner philharmoniker'),
                   ('a-sol', 'Martha Argerich', 'martha argerich');
                 INSERT INTO track_artist(track_id, artist_id, role) VALUES
                   ('t6', 'a-cond', 'conductor'), ('t7', 'a-cond', 'conductor'),
                   ('t6', 'a-ens', 'ensemble'), ('t7', 'a-ens', 'ensemble'),
                   ('t6', 'a-sol', 'performer:piano'), ('t7', 'a-comp', 'composer');
                 UPDATE raw_file_metadata SET work = 'Symphony No. 1', movement = 'Allegro', movement_number = 2 WHERE file_id = 6;
                 UPDATE raw_file_metadata SET work = 'symphony no 1', movement = 'Adagio', movement_number = 1 WHERE file_id = 7;
                 UPDATE track_entity SET title = 'Suite: I. Blue' WHERE id = 't3';
                 UPDATE track_entity SET title = 'Suite: II. Late' WHERE id = 't4';
                 INSERT INTO track_artist(track_id, artist_id, role) VALUES ('t4', 'a-comp', 'composer');",
            )
            .expect("classical fixture");
    }

    fn scoped(entity: FocusEntity, scope: Vec<Criterion>, filters: Vec<FocusFilter>) -> FocusQuery {
        FocusQuery {
            entity,
            filters,
            scope,
            ..FocusQuery::default()
        }
    }

    fn composer(artist_id: &str) -> Criterion {
        Criterion::Credit {
            role: "composer".into(),
            artist_id: artist_id.into(),
            name: None,
        }
    }

    #[test]
    fn composers_browser_lists_credited_composers_with_focus_filters() {
        let mut connection = catalog();
        classical(&mut connection);
        assert_eq!(
            ids(&mut connection, &query(FocusEntity::Composers, Vec::new())),
            ["a-comp"]
        );
        // Composers of hi-res tracks, and composers with no MP3 tracks.
        assert_eq!(
            ids(
                &mut connection,
                &query(
                    FocusEntity::Composers,
                    vec![include(Criterion::Quality {
                        value: QualityTier::Hires
                    })]
                )
            ),
            ["a-comp"]
        );
        assert!(
            ids(
                &mut connection,
                &query(
                    FocusEntity::Composers,
                    vec![exclude(Criterion::Format {
                        value: "aac".into()
                    })]
                )
            )
            .is_empty()
        );
        let rows = composer_rows(&mut connection, &["a-comp".into(), "missing".into()])
            .expect("composer rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Anna Hamilton");
        assert_eq!(rows[0].track_count, 4);
        assert_eq!(rows[0].album_count, 2);
        // t6/t7 share a work; t3 and t4 are their own titles.
        assert_eq!(rows[0].work_count, 3);

        let facets = focus_facets(
            &mut connection,
            &query(FocusEntity::Composers, Vec::new()),
            1,
            &FacetRequest {
                only: vec!["credit".into()],
                ..FacetRequest::default()
            },
        )
        .expect("composer facets");
        assert_eq!(facets.total, 1);
        assert!(
            facets
                .facets
                .iter()
                .any(|facet| facet.role.as_deref() == Some("conductor_ensemble"))
        );
    }

    #[test]
    fn performer_and_conductor_ensemble_columns_count_and_filter() {
        let mut connection = catalog();
        classical(&mut connection);
        let facets = focus_facets(
            &mut connection,
            &query(FocusEntity::Songs, Vec::new()),
            1,
            &FacetRequest {
                only: vec!["credit".into()],
                ..FacetRequest::default()
            },
        )
        .expect("credit facets");
        let roles = facets
            .facets
            .iter()
            .map(|facet| facet.role.clone().unwrap_or_default())
            .collect::<Vec<_>>();
        assert_eq!(
            roles,
            ["performer", "conductor_ensemble", "composer", "production"],
            "credit columns in Roon order"
        );
        let column = |role: &str| {
            facets
                .facets
                .iter()
                .find(|facet| facet.role.as_deref() == Some(role))
                .expect("credit column")
        };
        let conducting = column("conductor_ensemble");
        assert_eq!(conducting.total, 2);
        assert!(conducting.values.iter().all(|value| value.count == 2));
        let performers = column("performer")
            .values
            .iter()
            .map(|value| value.label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(performers, ["Cimafunk", "Martha Argerich"]);

        let group = |artist_id: &str| Criterion::Credit {
            role: "conductor_ensemble".into(),
            artist_id: artist_id.into(),
            name: None,
        };
        assert_eq!(
            ids(
                &mut connection,
                &query(FocusEntity::Songs, vec![include(group("a-ens"))])
            ),
            ["t6", "t7"]
        );
        assert_eq!(
            ids(
                &mut connection,
                &query(FocusEntity::Albums, vec![exclude(group("a-cond"))])
            ),
            ["al-jazz", "al-pop", "al-single"]
        );
        // A single stored role still works, and a group is not a free-form role.
        assert_eq!(
            ids(
                &mut connection,
                &query(
                    FocusEntity::Songs,
                    vec![include(Criterion::Credit {
                        role: "performer".into(),
                        artist_id: "a-sol".into(),
                        name: None,
                    })]
                )
            ),
            ["t6"]
        );
        let mut invalid = query(FocusEntity::Songs, vec![include(group("a-ens"))]);
        if let Criterion::Credit { role, .. } = &mut invalid.filters[0].criterion {
            *role = "conductor_ensemble_x".into();
        }
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn scope_bounds_results_totals_and_facets() {
        let mut connection = catalog();
        classical(&mut connection);
        let discography = scoped(FocusEntity::Albums, vec![composer("a-comp")], Vec::new());
        assert_eq!(ids(&mut connection, &discography), ["al-hires", "al-jazz"]);
        assert_eq!(
            library_total(&mut connection, &discography).expect("total"),
            2
        );
        let hires = scoped(
            FocusEntity::Albums,
            vec![composer("a-comp")],
            vec![include(Criterion::Quality {
                value: QualityTier::Hires,
            })],
        );
        assert_eq!(ids(&mut connection, &hires), ["al-hires"]);
        assert_eq!(
            library_total(&mut connection, &hires).expect("scoped total"),
            2
        );

        // An album's track list, focused on one conductor.
        let tracks = scoped(
            FocusEntity::Songs,
            vec![Criterion::Album {
                album_id: "al-hires".into(),
                name: None,
            }],
            vec![include(Criterion::Credit {
                role: "performer".into(),
                artist_id: "a-sol".into(),
                name: None,
            })],
        );
        assert_eq!(ids(&mut connection, &tracks), ["t6"]);
        assert_eq!(
            library_total(&mut connection, &tracks).expect("album total"),
            2
        );
        let facets = focus_facets(
            &mut connection,
            &scoped(
                FocusEntity::Songs,
                vec![Criterion::Album {
                    album_id: "al-hires".into(),
                    name: None,
                }],
                Vec::new(),
            ),
            1,
            &FacetRequest {
                only: vec!["format".into()],
                ..FacetRequest::default()
            },
        )
        .expect("scoped facets");
        assert_eq!(facets.total, 2);
        let formats = facets.facets[0]
            .values
            .iter()
            .map(|value| value.value.as_str())
            .collect::<Vec<_>>();
        assert_eq!(formats, ["flac", "wav"]);
        // Match any never widens past the scope.
        let mut any = scoped(
            FocusEntity::Songs,
            vec![composer("a-comp")],
            vec![include(genre("Latin")), include(genre("Jazz"))],
        );
        any.match_mode = MatchMode::Any;
        assert_eq!(ids(&mut connection, &any), ["t3", "t4", "t6", "t7"]);
        let too_many = scoped(
            FocusEntity::Songs,
            vec![composer("a-comp"); MAX_SCOPE + 1],
            Vec::new(),
        );
        assert!(too_many.validate().is_err());
    }

    #[test]
    fn release_types_come_from_tags_and_untagged_albums_stay_unknown() {
        let mut connection = catalog();
        let facets = focus_facets(
            &mut connection,
            &query(FocusEntity::Albums, Vec::new()),
            1,
            &FacetRequest {
                only: vec!["release_type".into()],
                ..FacetRequest::default()
            },
        )
        .expect("release type facet");
        let values = facets.facets[0]
            .values
            .iter()
            .map(|value| (value.label.as_str(), value.count))
            .collect::<Vec<_>>();
        // al-jazz has no RELEASETYPE tag and is not guessed.
        assert_eq!(values, [("Album", 2), ("Single", 1)]);
        assert_eq!(release_type_label("ep"), "EP");
        assert_eq!(
            ids(
                &mut connection,
                &query(
                    FocusEntity::Albums,
                    vec![include(Criterion::ReleaseType {
                        value: "Album".into()
                    })]
                )
            ),
            ["al-hires", "al-pop"]
        );
    }

    #[test]
    fn compositions_group_works_movements_and_recordings() {
        let mut connection = catalog();
        classical(&mut connection);
        let compositions = focus_compositions(
            &mut connection,
            &scoped(FocusEntity::Albums, vec![composer("a-comp")], Vec::new()),
            1,
        )
        .expect("compositions");
        assert_eq!(compositions.track_total, 4);
        assert!(!compositions.truncated);
        let titles = compositions
            .compositions
            .iter()
            .map(|composition| composition.title.as_str())
            .collect::<Vec<_>>();
        assert_eq!(titles, ["Suite", "Symphony No. 1"]);
        let suite = &compositions.compositions[0];
        assert!(!suite.tagged, "derived from shared Work: Movement titles");
        assert_eq!(suite.movement_count, 2);
        let movements = suite.recordings[0]
            .tracks
            .iter()
            .map(|track| track.movement.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(movements, [Some("I. Blue"), Some("II. Late")]);

        let symphony = &compositions.compositions[1];
        assert!(symphony.tagged);
        assert_eq!(symphony.composers[0].name, "Anna Hamilton");
        assert_eq!(symphony.recordings.len(), 1);
        let recording = &symphony.recordings[0];
        assert_eq!(recording.album_id.as_deref(), Some("al-hires"));
        assert_eq!(recording.release_year.as_deref(), Some("1978"));
        assert_eq!(
            recording.performers,
            [
                "Claudio Abbado",
                "Berliner Philharmoniker",
                "Martha Argerich"
            ]
        );
        // MOVEMENT numbers order the tracks, not disc/track order.
        let order = recording
            .tracks
            .iter()
            .map(|track| (track.id.as_str(), track.movement.as_deref()))
            .collect::<Vec<_>>();
        assert_eq!(order, [("t7", Some("Adagio")), ("t6", Some("Allegro"))]);
        assert_eq!(recording.duration_seconds, 810.0);

        // Focus filters narrow the compositions too.
        let lossy = focus_compositions(
            &mut connection,
            &scoped(
                FocusEntity::Songs,
                vec![composer("a-comp")],
                vec![include(Criterion::Format {
                    value: "aac".into(),
                })],
            ),
            1,
        )
        .expect("focused compositions");
        assert_eq!(lossy.total, 1);
        assert_eq!(lossy.compositions[0].title, "Suite: II. Late");
    }

    #[test]
    fn people_report_their_credit_roles() {
        let mut connection = catalog();
        classical(&mut connection);
        let person = focus_person(&mut connection, "a-cond")
            .expect("person")
            .expect("conductor exists");
        assert_eq!(person.name, "Claudio Abbado");
        assert_eq!(
            person.roles,
            [PersonRole {
                role: "conductor_ensemble".into(),
                track_count: 2
            }]
        );
        assert!(person.composer.is_none());
        let composer = focus_person(&mut connection, "a-comp")
            .expect("person")
            .expect("composer exists");
        assert_eq!(composer.composer.expect("composer stats").track_count, 4);
        let cimafunk = focus_person(&mut connection, "a-cima")
            .expect("person")
            .expect("artist exists");
        assert_eq!(cimafunk.roles[0].role, "primary");
        assert_eq!(cimafunk.roles[1].role, "performer");
        assert!(
            focus_person(&mut connection, "nobody")
                .expect("lookup")
                .is_none()
        );
    }

    #[test]
    fn filter_values_cannot_inject_sql() {
        let mut connection = catalog();
        let hostile = "x') OR 1=1 --";
        for criterion in [
            genre(hostile),
            Criterion::Format {
                value: hostile.into(),
            },
            Criterion::Label {
                value: hostile.into(),
            },
            Criterion::ReleaseType {
                value: hostile.into(),
            },
            Criterion::Artist {
                artist_id: hostile.into(),
                name: None,
            },
        ] {
            assert!(
                ids(
                    &mut connection,
                    &query(FocusEntity::Songs, vec![include(criterion)])
                )
                .is_empty()
            );
        }
        let mut text = query(FocusEntity::Songs, Vec::new());
        text.text = Some("100%_".into());
        assert!(ids(&mut connection, &text).is_empty());
    }

    #[test]
    fn hot_filters_use_indexes() {
        #[derive(QueryableByName)]
        struct PlanRow {
            #[diesel(sql_type = Text)]
            detail: String,
        }
        let mut connection = catalog();
        let focus = query(
            FocusEntity::Albums,
            vec![
                include(genre("Jazz")),
                include(Criterion::Year {
                    from: Some(1990),
                    to: Some(1999),
                }),
                include(Criterion::Credit {
                    role: "composer".into(),
                    artist_id: "a-comp".into(),
                    name: None,
                }),
            ],
        );
        let mut select = select_sql(&focus, 1, 0, 50);
        select.sql = format!("EXPLAIN QUERY PLAN {}", select.sql);
        let plan = select
            .into_query()
            .load::<PlanRow>(&mut connection)
            .expect("query plan")
            .into_iter()
            .map(|row| row.detail)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(plan.contains("idx_album_entity_release_year"), "{plan}");
        assert!(plan.contains("idx_track_artist_artist_role"), "{plan}");
        assert!(
            plan.contains("idx_album_genre_genre") || plan.contains("genre_entity"),
            "{plan}"
        );
    }
}
