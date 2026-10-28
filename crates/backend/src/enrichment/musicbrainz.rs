//! MusicBrainz web service (`/ws/2`) requests and response parsing.
//!
//! Only the fields enrichment uses are read. Parsing is lenient: a missing
//! field yields `None` or an empty list instead of failing the whole task.

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::Value;

use crate::library::identity::{normalize_album_identity, normalize_artist_identity};

pub const API_ROOT: &str = "https://musicbrainz.org/ws/2";
/// Search scores below this are never trusted, even when names match.
pub const MIN_SEARCH_SCORE: i64 = 90;

fn encode(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

/// Lucene phrase escaping for MusicBrainz search queries.
fn phrase(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        if matches!(character, '"' | '\\') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped.push('"');
    escaped
}

pub fn is_mbid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok() && value.len() == 36
}

pub fn artist_search_url(name: &str) -> String {
    format!(
        "{API_ROOT}/artist?query={}&limit=8&fmt=json",
        encode(&format!("artist:{}", phrase(name)))
    )
}

/// A search on other names: an artist renamed on MusicBrainz ("Kimiko
/// Douglass-Ishizaka") keeps the name your files use ("Kimiko Ishizaka") as
/// an alias, which the name search does not look at.
pub fn artist_alias_search_url(name: &str) -> String {
    format!(
        "{API_ROOT}/artist?query={}&limit=8&fmt=json",
        encode(&format!("alias:{}", phrase(name)))
    )
}

pub fn artist_lookup_url(mbid: &str) -> String {
    format!("{API_ROOT}/artist/{mbid}?inc=url-rels+genres+aliases&fmt=json")
}

/// A title and, for classical albums titled "Beethoven: Symphony No. 9",
/// the title without the composer, which is how MusicBrainz names most
/// such releases.
pub fn title_variants(title: &str) -> Vec<String> {
    let title = title.trim();
    let mut variants = vec![title.to_string()];
    if let Some((prefix, rest)) = title.split_once(": ") {
        let rest = rest.trim();
        if !prefix.trim().is_empty()
            && prefix.chars().count() <= 40
            && !prefix.chars().any(|c| c.is_ascii_digit())
            && rest.chars().count() >= 3
        {
            variants.push(rest.to_string());
        }
    }
    variants
}

/// The words of a title or name as search terms: letters and digits only,
/// so no Lucene syntax survives, lower-cased so `AND`/`OR` stay words.
pub fn search_words(value: &str) -> Vec<String> {
    let mut words = Vec::new();
    for word in value
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
    {
        if words.len() < 12 && !words.iter().any(|known| known == word) {
            words.push(word.to_string());
        }
    }
    words
}

/// Matches a title as typed, without its composer prefix, or by its
/// words: "Symphonien Nr. 5 & 7" then also finds "Symphonien nos. 5 & 7".
/// Exact phrases are boosted so they rank first.
fn title_clause(title: &str) -> String {
    let mut parts = title_variants(title)
        .iter()
        .map(|variant| format!("{}^3", phrase(variant)))
        .collect::<Vec<_>>();
    let words = search_words(title);
    if words.len() > 1 {
        parts.push(format!("({})", words.join(" ")));
    }
    format!("release:({})", parts.join(" OR "))
}

/// The artist as a phrase, or with every word required in any order
/// ("Kleiber, Carlos" or "Wiener Philharmoniker, Carlos Kleiber").
fn artist_clause(artist: &str) -> String {
    let words = search_words(artist);
    if words.len() > 1 {
        format!(
            "artist:({} OR ({}))",
            phrase(artist.trim()),
            words
                .iter()
                .map(|word| format!("+{word}"))
                .collect::<Vec<_>>()
                .join(" ")
        )
    } else {
        format!("artist:{}", phrase(artist.trim()))
    }
}

pub fn release_search_url(title: &str, artist: &str) -> String {
    format!(
        "{API_ROOT}/release?query={}&limit=15&fmt=json",
        encode(&format!(
            "{} AND {}",
            title_clause(title),
            artist_clause(artist)
        ))
    )
}

pub fn release_lookup_url(mbid: &str) -> String {
    format!(
        "{API_ROOT}/release/{mbid}?inc=artist-credits+labels+recordings+release-groups+artist-rels+recording-level-rels+work-rels+work-level-rels&fmt=json"
    )
}

pub fn release_group_lookup_url(mbid: &str) -> String {
    format!("{API_ROOT}/release-group/{mbid}?inc=url-rels+genres&fmt=json")
}

/// A work with its URL relationships (Wikidata and Wikipedia links).
/// A work with its composer (the relation dates it), the piece it is part
/// of, and its Wikipedia and Wikidata links.
pub fn work_lookup_url(mbid: &str) -> String {
    format!("{API_ROOT}/work/{mbid}?inc=artist-rels+work-rels+url-rels&fmt=json")
}

/// Works MusicBrainz lists for a composer (browse works by artist), with
/// the composer relations that carry the years, the parent of each
/// movement and the Wikidata link. MusicBrainz serves 100 per page.
pub fn work_browse_url(artist_mbid: &str, offset: usize) -> String {
    format!(
        "{API_ROOT}/work?artist={artist_mbid}&inc=artist-rels+work-rels+url-rels&limit={WORK_PAGE_SIZE}&offset={offset}&fmt=json"
    )
}

pub const WORK_PAGE_SIZE: usize = 100;

/// The total number of works and this page's works from a browse answer.
pub fn parse_work_browse(body: &Value) -> (usize, &[Value]) {
    let total = body
        .get("work-count")
        .and_then(Value::as_u64)
        .unwrap_or_default() as usize;
    (total, array(body, "works"))
}

/// The page listing a composer's works on MusicBrainz.
pub fn artist_works_page_url(mbid: &str) -> String {
    format!("https://musicbrainz.org/artist/{mbid}/works")
}

pub fn artist_page_url(mbid: &str) -> String {
    format!("https://musicbrainz.org/artist/{mbid}")
}

pub fn release_page_url(mbid: &str) -> String {
    format!("https://musicbrainz.org/release/{mbid}")
}

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtistCandidate {
    pub mbid: String,
    pub name: String,
    pub score: i64,
    pub disambiguation: Option<String>,
    /// Other names MusicBrainz lists for the artist.
    pub aliases: Vec<String>,
}

pub fn parse_artist_search(body: &Value) -> Vec<ArtistCandidate> {
    array(body, "artists")
        .iter()
        .filter_map(|artist| {
            Some(ArtistCandidate {
                mbid: text(artist, "id").filter(|id| is_mbid(id))?,
                name: text(artist, "name")?,
                score: artist.get("score").and_then(Value::as_i64).unwrap_or(0),
                disambiguation: text(artist, "disambiguation"),
                aliases: array(artist, "aliases")
                    .iter()
                    .filter_map(|alias| text(alias, "name"))
                    .collect(),
            })
        })
        .collect()
}

/// Whether any search result carries exactly this name (so a namesake
/// question, not a renamed artist).
pub fn has_exact_name(name: &str, candidates: &[ArtistCandidate]) -> bool {
    let wanted = normalize_artist_identity(name);
    candidates
        .iter()
        .any(|candidate| normalize_artist_identity(&candidate.name) == wanted)
}

/// Picks the single result that lists the name as an alias, by the same
/// rules as [`choose_artist`]: a strong search score, and two artists with
/// that alias are ambiguous unless `known` settles it.
pub fn choose_artist_by_alias(
    name: &str,
    candidates: &[ArtistCandidate],
    known: &[String],
) -> Option<ArtistCandidate> {
    let wanted = normalize_artist_identity(name);
    let matching = candidates
        .iter()
        .filter(|candidate| {
            candidate.score >= MIN_SEARCH_SCORE
                && candidate
                    .aliases
                    .iter()
                    .any(|alias| normalize_artist_identity(alias) == wanted)
        })
        .collect::<Vec<_>>();
    if let Some(known) = matching
        .iter()
        .find(|candidate| known.contains(&candidate.mbid))
    {
        return Some((*known).clone());
    }
    match matching.as_slice() {
        [only] => Some((*only).clone()),
        _ => None,
    }
}

/// Picks the single search result whose name matches exactly. Two equally
/// good namesakes are ambiguous, and a wrong biography is worse than none,
/// so ambiguity yields `None` unless `known` (MBIDs seen in this library's
/// release credits) settles it.
pub fn choose_artist(
    name: &str,
    candidates: &[ArtistCandidate],
    known: &[String],
) -> Option<ArtistCandidate> {
    let wanted = normalize_artist_identity(name);
    let matching = candidates
        .iter()
        .filter(|candidate| {
            candidate.score >= MIN_SEARCH_SCORE
                && normalize_artist_identity(&candidate.name) == wanted
        })
        .collect::<Vec<_>>();
    if let Some(known) = matching
        .iter()
        .find(|candidate| known.contains(&candidate.mbid))
    {
        return Some((*known).clone());
    }
    match matching.as_slice() {
        [only] => Some((*only).clone()),
        _ => None,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtistProfile {
    pub mbid: String,
    pub name: String,
    /// `Person`, `Group`, `Orchestra`, `Choir`, `Character` or `Other`.
    pub kind: Option<String>,
    pub area: Option<String>,
    pub begin_area: Option<String>,
    pub begin: Option<String>,
    pub end: Option<String>,
    pub disambiguation: Option<String>,
    pub wikidata_id: Option<String>,
    /// A Wikipedia article linked directly, as `(language, title)`.
    pub wikipedia: Option<(String, String)>,
    /// Genres voted on MusicBrainz, most votes first.
    pub genres: Vec<MbGenre>,
    /// A Commons file linked as the artist's image, a fallback when the
    /// Wikidata item has no P18 image.
    pub image_file: Option<String>,
    /// Other names: legal names, stage names and spellings.
    pub aliases: Vec<String>,
}

pub fn parse_artist(body: &Value) -> Option<ArtistProfile> {
    let life_span = body.get("life-span").cloned().unwrap_or(Value::Null);
    Some(ArtistProfile {
        mbid: text(body, "id").filter(|id| is_mbid(id))?,
        name: text(body, "name")?,
        kind: text(body, "type"),
        area: body.get("area").and_then(|area| text(area, "name")),
        begin_area: body.get("begin-area").and_then(|area| text(area, "name")),
        begin: text(&life_span, "begin"),
        end: text(&life_span, "end"),
        disambiguation: text(body, "disambiguation"),
        wikidata_id: wikidata_relation(body),
        wikipedia: wikipedia_relation(body),
        genres: parse_genres(body),
        image_file: image_relation(body),
        aliases: parse_aliases(body),
    })
}

/// At most this many aliases are kept per person.
pub const MAX_ALIASES: usize = 24;

/// The names in an artist's `aliases` (looked up with `inc=aliases`):
/// legal names, stage names and spellings, but not search hints.
pub fn parse_aliases(body: &Value) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for alias in array(body, "aliases") {
        if alias.get("type").and_then(Value::as_str) == Some("Search hint") {
            continue;
        }
        let Some(name) = text(alias, "name") else {
            continue;
        };
        if !names.contains(&name) {
            names.push(name);
        }
        if names.len() >= MAX_ALIASES {
            break;
        }
    }
    names
}

/// A genre as MusicBrainz names it, with its vote count.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MbGenre {
    pub name: String,
    pub mbid: Option<String>,
    pub count: i64,
}

/// At most this many genres are kept per release group or artist.
pub const MAX_GENRES: usize = 8;

/// The `genres` of an entity looked up with `inc=genres`: voted genres
/// only, most votes first.
pub fn parse_genres(body: &Value) -> Vec<MbGenre> {
    let mut genres = array(body, "genres")
        .iter()
        .filter_map(|genre| {
            let name = text(genre, "name")?;
            let count = genre.get("count").and_then(Value::as_i64).unwrap_or(0);
            (count > 0 && name.chars().count() <= 64).then(|| MbGenre {
                name,
                mbid: text(genre, "id").filter(|id| is_mbid(id)),
                count,
            })
        })
        .collect::<Vec<_>>();
    genres.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.name.cmp(&right.name))
    });
    genres.truncate(MAX_GENRES);
    genres
}

/// The Commons file named by an `image` URL relationship
/// (`https://commons.wikimedia.org/wiki/File:Name.jpg`).
pub fn image_relation(body: &Value) -> Option<String> {
    array(body, "relations").iter().find_map(|relation| {
        if relation.get("type").and_then(Value::as_str) != Some("image") {
            return None;
        }
        let resource = relation.get("url").and_then(|url| text(url, "resource"))?;
        let file = resource
            .strip_prefix("https://commons.wikimedia.org/wiki/File:")
            .or_else(|| resource.strip_prefix("http://commons.wikimedia.org/wiki/File:"))?;
        let file = percent_encoding::percent_decode_str(file)
            .decode_utf8()
            .ok()?
            .replace('_', " ");
        let file = file.trim();
        (!file.is_empty() && file.len() <= 240 && !file.contains(['/', '\\', '|', '#', '?']))
            .then(|| file.to_string())
    })
}

/// The Wikidata item linked from an entity's URL relationships.
pub fn wikidata_relation(body: &Value) -> Option<String> {
    array(body, "relations").iter().find_map(|relation| {
        if relation.get("type").and_then(Value::as_str) != Some("wikidata") {
            return None;
        }
        let resource = relation.get("url").and_then(|url| text(url, "resource"))?;
        let id = resource.rsplit('/').next()?;
        is_wikidata_id(id).then(|| id.to_string())
    })
}

/// The Wikipedia article linked from an entity's URL relationships, as
/// `(language, title)`. English wins when several languages are linked.
pub fn wikipedia_relation(body: &Value) -> Option<(String, String)> {
    let mut links = array(body, "relations")
        .iter()
        .filter(|relation| relation.get("type").and_then(Value::as_str) == Some("wikipedia"))
        .filter_map(|relation| {
            let resource = relation.get("url").and_then(|url| text(url, "resource"))?;
            super::wikimedia::parse_wikipedia_url(&resource)
        })
        .collect::<Vec<_>>();
    links.sort_by_key(|(language, _)| language != "en");
    links.into_iter().next()
}

/// Wikipedia and Wikidata links of a work looked up with [`work_lookup_url`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkLinks {
    pub mbid: String,
    pub title: String,
    pub wikidata_id: Option<String>,
    pub wikipedia: Option<(String, String)>,
}

pub fn parse_work_links(body: &Value) -> Option<WorkLinks> {
    Some(WorkLinks {
        mbid: text(body, "id").filter(|id| is_mbid(id))?,
        title: text(body, "title").unwrap_or_default(),
        wikidata_id: wikidata_relation(body),
        wikipedia: wikipedia_relation(body),
    })
}

pub fn is_wikidata_id(value: &str) -> bool {
    value.len() > 1
        && value.len() <= 16
        && value.starts_with('Q')
        && value[1..].bytes().all(|byte| byte.is_ascii_digit())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseCandidate {
    pub mbid: String,
    pub title: String,
    pub score: i64,
    pub artist: String,
    /// Every credited artist name, for matching an album artist that is
    /// one of several (a conductor on a classical release).
    pub artist_names: Vec<String>,
    pub track_count: Option<i64>,
    pub date: Option<String>,
    pub country: Option<String>,
    pub disambiguation: Option<String>,
    pub labels: Vec<String>,
    pub catalog_numbers: Vec<String>,
    /// Medium formats, e.g. `CD` or `2×Vinyl`.
    pub format: Option<String>,
    pub release_type: Option<String>,
}

pub fn parse_release_search(body: &Value) -> Vec<ReleaseCandidate> {
    array(body, "releases")
        .iter()
        .filter_map(|release| {
            Some(ReleaseCandidate {
                mbid: text(release, "id").filter(|id| is_mbid(id))?,
                title: text(release, "title")?,
                score: release.get("score").and_then(Value::as_i64).unwrap_or(0),
                artist: artist_credit_phrase(release),
                artist_names: array(release, "artist-credit")
                    .iter()
                    .filter_map(|credit| {
                        text(credit, "name").or_else(|| {
                            credit.get("artist").and_then(|artist| text(artist, "name"))
                        })
                    })
                    .collect(),
                track_count: release.get("track-count").and_then(Value::as_i64),
                date: text(release, "date"),
                country: text(release, "country"),
                disambiguation: text(release, "disambiguation"),
                labels: array(release, "label-info")
                    .iter()
                    .filter_map(|info| info.get("label").and_then(|label| text(label, "name")))
                    .filter(|label| label != "[no label]")
                    .fold(Vec::new(), |mut labels, label| {
                        if !labels.contains(&label) {
                            labels.push(label);
                        }
                        labels
                    }),
                catalog_numbers: array(release, "label-info")
                    .iter()
                    .filter_map(|info| text(info, "catalog-number"))
                    .filter(|number| number != "[none]")
                    .collect(),
                format: media_format(release),
                release_type: release
                    .get("release-group")
                    .and_then(|group| text(group, "primary-type")),
            })
        })
        .collect()
}

/// "CD", "2×CD" or "CD + DVD" from a release's media.
fn media_format(release: &Value) -> Option<String> {
    let mut formats: Vec<(String, usize)> = Vec::new();
    for medium in array(release, "media") {
        let format = text(medium, "format").unwrap_or_else(|| "Unknown".to_string());
        match formats.iter_mut().find(|(known, _)| *known == format) {
            Some((_, count)) => *count += 1,
            None => formats.push((format, 1)),
        }
    }
    let parts = formats
        .into_iter()
        .map(|(format, count)| {
            if count > 1 {
                format!("{count}×{format}")
            } else {
                format
            }
        })
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join(" + "))
}

/// A free-text release search for manual identification. The text is
/// searched as typed (MusicBrainz' default fields: release, artist, label,
/// catalogue number, barcode).
pub fn release_query_url(query: &str) -> String {
    format!(
        "{API_ROOT}/release?query={}&limit=15&fmt=json",
        encode(query)
    )
}

/// Release search by title and, when known, artist; the default query for
/// the identify dialog.
pub fn identify_query(title: &str, artist: Option<&str>) -> String {
    let title = title_clause(title);
    match artist.filter(|artist| !artist.trim().is_empty()) {
        Some(artist) => format!("{title} AND {}", artist_clause(artist)),
        None => title,
    }
}

/// [`identify_query`] that also ranks releases crediting the tagged
/// composers and conductors higher. The extra clause can always match (it
/// repeats the title), so it only reorders results and never hides one.
pub fn identify_query_with_people(title: &str, artist: Option<&str>, people: &[String]) -> String {
    let base = identify_query(title, artist);
    let artist = artist.map(normalize_artist_identity).unwrap_or_default();
    let mut boosts = Vec::new();
    for person in people {
        let person = person.trim();
        if person.is_empty()
            || normalize_artist_identity(person) == artist
            || boosts.len() >= 4
            || is_placeholder_artist(person)
        {
            continue;
        }
        let clause = format!("artist:{}^2", phrase(person));
        if !boosts.contains(&clause) {
            boosts.push(clause);
        }
    }
    if boosts.is_empty() {
        return base;
    }
    format!(
        "{base} AND ({} OR {})",
        title_clause(title),
        boosts.join(" OR ")
    )
}

/// Names that stand for "no artist" in tags. They are never matched to a
/// MusicBrainz artist (MusicBrainz has real artists with these names).
pub fn is_placeholder_artist(name: &str) -> bool {
    let normalized = normalize_artist_identity(name);
    let trimmed = normalized.trim_matches(|c: char| !c.is_alphanumeric());
    matches!(
        trimmed,
        "" | "unknown"
            | "unknown artist"
            | "unknown artists"
            | "artist"
            | "various"
            | "various artists"
            | "va"
            | "v a"
            | "no artist"
            | "anonymous"
            | "traditional"
            | "untitled artist"
            | "unknown album artist"
            | "album artist"
            | "soundtrack"
            | "original soundtrack"
            | "cast"
            | "original cast"
    )
}

fn artist_credit_phrase(value: &Value) -> String {
    array(value, "artist-credit")
        .iter()
        .map(|credit| {
            let name = text(credit, "name")
                .or_else(|| credit.get("artist").and_then(|artist| text(artist, "name")))
                .unwrap_or_default();
            let join = credit
                .get("joinphrase")
                .and_then(Value::as_str)
                .unwrap_or_default();
            format!("{name}{join}")
        })
        .collect()
}

/// Picks the best release for a local album: the title and album artist must
/// match, and a release with the same number of tracks wins.
pub fn choose_release(
    title: &str,
    artist: &str,
    track_count: usize,
    candidates: &[ReleaseCandidate],
) -> Option<ReleaseCandidate> {
    choose_release_in_year(title, artist, track_count, None, candidates)
}

/// [`choose_release`], preferring releases from the album's own year when
/// its tags have one: a conductor often recorded a work several times.
pub fn choose_release_in_year(
    title: &str,
    artist: &str,
    track_count: usize,
    year: Option<&str>,
    candidates: &[ReleaseCandidate],
) -> Option<ReleaseCandidate> {
    let wanted_titles = title_variants(title)
        .iter()
        .map(|variant| normalize_album_identity(variant))
        .collect::<Vec<_>>();
    let wanted_artist = normalize_artist_identity(artist);
    let matching = candidates
        .iter()
        .filter(|candidate| {
            candidate.score >= MIN_SEARCH_SCORE
                && wanted_titles.contains(&normalize_album_identity(&candidate.title))
                && (normalize_artist_identity(&candidate.artist) == wanted_artist
                    || candidate
                        .artist_names
                        .iter()
                        .any(|name| normalize_artist_identity(name) == wanted_artist))
        })
        .collect::<Vec<_>>();
    let same_year = matching
        .iter()
        .filter(|candidate| {
            year.is_some_and(|year| {
                year.len() == 4
                    && candidate
                        .date
                        .as_deref()
                        .is_some_and(|date| date.starts_with(year))
            })
        })
        .copied()
        .collect::<Vec<_>>();
    let matching = if same_year.is_empty() {
        matching
    } else {
        same_year
    };
    // Same track count first; otherwise the earliest dated release, which is
    // most often the original.
    matching
        .iter()
        .find(|candidate| candidate.track_count == Some(track_count as i64))
        .or_else(|| {
            matching
                .iter()
                .filter(|candidate| candidate.date.is_some())
                .min_by(|left, right| left.date.cmp(&right.date))
        })
        .or_else(|| matching.first())
        .map(|candidate| (*candidate).clone())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalCredit {
    /// The name the release credits, which may differ from the person's
    /// own (MusicBrainz "credited as").
    pub name: String,
    /// The person's own MusicBrainz name.
    pub artist_name: Option<String>,
    pub artist_mbid: Option<String>,
    pub role: String,
    pub instrument: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseTrack {
    pub disc: i64,
    pub position: i64,
    pub title: String,
    pub recording_mbid: Option<String>,
    /// The track's length in milliseconds, for the match check.
    pub length_ms: Option<i64>,
    pub credits: Vec<ExternalCredit>,
    /// Works the recording performs (with their parent work).
    pub works: Vec<super::works::WorkInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReleaseDetails {
    pub mbid: String,
    pub title: String,
    pub date: Option<String>,
    pub country: Option<String>,
    pub barcode: Option<String>,
    pub labels: Vec<(String, Option<String>)>,
    pub release_group_mbid: Option<String>,
    pub release_group_type: Option<String>,
    pub first_release_date: Option<String>,
    pub artists: Vec<(String, String)>,
    pub credits: Vec<ExternalCredit>,
    pub tracks: Vec<ReleaseTrack>,
}

/// Lower-cases an instrument, drops MusicBrainz qualifiers such as
/// "drums (drum set)" and folds common spellings together.
pub fn normalize_instrument(value: &str) -> String {
    let lower = value.to_lowercase();
    let base = lower.split('(').next().unwrap_or_default().trim();
    match base {
        "drum set" | "drumset" | "drum kit" | "kit" => "drums",
        "upright bass" | "acoustic bass" | "contrabass" | "string bass" => "double bass",
        "acoustic piano" | "grand piano" => "piano",
        "vocal" | "voice" => "vocals",
        other => other,
    }
    .to_string()
}

/// Maps a MusicBrainz relationship type and its attributes to a Parson
/// credit role and instrument.
pub fn relationship_role(kind: &str, attributes: &[String]) -> Option<(String, Option<String>)> {
    let first_attribute = attributes
        .iter()
        .find(|attribute| {
            !matches!(
                attribute.as_str(),
                "additional" | "guest" | "solo" | "minor" | "co" | "executive" | "assistant"
            )
        })
        .cloned();
    let role = match kind {
        "composer" | "writer" => "composer",
        "lyricist" | "librettist" => "lyricist",
        "arranger" | "orchestrator" | "instrument arranger" | "vocal arranger" => "arranger",
        "conductor" | "chorus master" => "conductor",
        "performing orchestra" => "ensemble",
        "producer" => "producer",
        "remixer" => "remixer",
        "mix" | "mix-DJ" => "mixer",
        "engineer" | "recording" | "mastering" | "sound" | "audio" | "programming" => "engineer",
        "instrument" => {
            return Some((
                "performer".to_string(),
                first_attribute.map(|instrument| normalize_instrument(&instrument)),
            ));
        }
        "vocal" => {
            let instrument = first_attribute
                .map(|attribute| attribute.to_lowercase())
                .filter(|attribute| attribute.contains("vocal"))
                .unwrap_or_else(|| "vocals".to_string());
            return Some(("performer".to_string(), Some(instrument)));
        }
        "performer" => "performer",
        _ => return None,
    };
    Some((role.to_string(), None))
}

fn relation_credits(value: &Value, credits: &mut Vec<ExternalCredit>) {
    for relation in array(value, "relations") {
        let kind = relation
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match relation.get("target-type").and_then(Value::as_str) {
            Some("artist") => {
                let attributes = array(relation, "attributes")
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                let Some((role, instrument)) = relationship_role(kind, &attributes) else {
                    continue;
                };
                let Some(artist) = relation.get("artist") else {
                    continue;
                };
                let Some(name) = text(relation, "target-credit").or_else(|| text(artist, "name"))
                else {
                    continue;
                };
                let credit = ExternalCredit {
                    name,
                    artist_name: text(artist, "name"),
                    artist_mbid: text(artist, "id").filter(|id| is_mbid(id)),
                    role,
                    instrument,
                };
                if !credits.contains(&credit) {
                    credits.push(credit);
                }
            }
            // A recording is a performance of a work; the work carries the
            // composer and lyricist.
            Some("work") if kind == "performance" => {
                if let Some(work) = relation.get("work") {
                    relation_credits(work, credits);
                }
            }
            _ => {}
        }
    }
}

pub fn parse_release(body: &Value) -> Option<ReleaseDetails> {
    let mbid = text(body, "id").filter(|id| is_mbid(id))?;
    let release_group = body.get("release-group").cloned().unwrap_or(Value::Null);
    let mut credits = Vec::new();
    relation_credits(body, &mut credits);
    let mut tracks = Vec::new();
    for (medium_index, medium) in array(body, "media").iter().enumerate() {
        let disc = medium
            .get("position")
            .and_then(Value::as_i64)
            .unwrap_or(medium_index as i64 + 1);
        for (track_index, track) in array(medium, "tracks").iter().enumerate() {
            let recording = track.get("recording").cloned().unwrap_or(Value::Null);
            let mut track_credits = Vec::new();
            relation_credits(&recording, &mut track_credits);
            tracks.push(ReleaseTrack {
                disc,
                position: track
                    .get("position")
                    .and_then(Value::as_i64)
                    .unwrap_or(track_index as i64 + 1),
                title: text(track, "title")
                    .or_else(|| text(&recording, "title"))
                    .unwrap_or_default(),
                recording_mbid: text(&recording, "id").filter(|id| is_mbid(id)),
                length_ms: track
                    .get("length")
                    .and_then(Value::as_i64)
                    .or_else(|| recording.get("length").and_then(Value::as_i64))
                    .filter(|length| *length > 0),
                credits: track_credits,
                works: super::works::recording_works(&recording),
            });
        }
    }
    Some(ReleaseDetails {
        mbid,
        title: text(body, "title").unwrap_or_default(),
        date: text(body, "date"),
        country: text(body, "country"),
        barcode: text(body, "barcode"),
        labels: array(body, "label-info")
            .iter()
            .filter_map(|info| {
                let name = info.get("label").and_then(|label| text(label, "name"))?;
                Some((name, text(info, "catalog-number")))
            })
            .collect(),
        release_group_mbid: text(&release_group, "id").filter(|id| is_mbid(id)),
        release_group_type: text(&release_group, "primary-type"),
        first_release_date: text(&release_group, "first-release-date"),
        artists: array(body, "artist-credit")
            .iter()
            .filter_map(|credit| {
                let artist = credit.get("artist")?;
                Some((
                    text(artist, "id").filter(|id| is_mbid(id))?,
                    text(artist, "name")?,
                ))
            })
            .collect(),
        credits,
        tracks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const BEATLES: &str = "b10bbbfc-cf9e-42e0-be17-e2c3e1d2600d";
    const OTHER: &str = "11111111-1111-4111-8111-111111111111";

    #[test]
    fn search_urls_quote_and_encode_names() {
        assert_eq!(
            artist_search_url("AC/DC \"live\""),
            "https://musicbrainz.org/ws/2/artist?query=artist%3A%22AC%2FDC%20%5C%22live%5C%22%22&limit=8&fmt=json"
        );
        assert!(release_search_url("Kind of Blue", "Miles Davis").contains(&encode(
            "release:(\"Kind of Blue\"^3 OR (kind of blue)) AND artist:(\"Miles Davis\" OR (+miles +davis))"
        )));
        // Words keep no search syntax: quotes, colons and operators go.
        assert_eq!(
            search_words("AC/DC: \"Live\" AND (1992) Live"),
            ["ac", "dc", "live", "and", "1992"]
        );
        // A forgiving search: the words of "Symphonien Nr. 5 & 7" also find
        // "Symphonien nos. 5 & 7".
        assert!(
            release_search_url("Symphonien Nr. 5 & 7", "Carlos Kleiber").contains(&encode(
                "release:(\"Symphonien Nr. 5 & 7\"^3 OR (symphonien nr 5 7)) AND artist:(\"Carlos Kleiber\" OR (+carlos +kleiber))"
            ))
        );
        assert!(release_lookup_url(BEATLES).contains("work-level-rels"));
    }

    #[test]
    fn artist_choice_requires_an_exact_unambiguous_name() {
        let candidates = parse_artist_search(&json!({"artists": [
            {"id": BEATLES, "name": "The Beatles", "score": 100},
            {"id": OTHER, "name": "Beatles Revival Band", "score": 92},
            {"id": "not-an-id", "name": "The Beatles", "score": 100}
        ]}));
        assert_eq!(candidates.len(), 2);
        assert_eq!(
            choose_artist("the beatles", &candidates, &[]).map(|artist| artist.mbid),
            Some(BEATLES.to_string())
        );
        assert_eq!(choose_artist("Beatles", &candidates, &[]), None);

        let namesakes = parse_artist_search(&json!({"artists": [
            {"id": BEATLES, "name": "Nirvana", "score": 100, "disambiguation": "US grunge"},
            {"id": OTHER, "name": "Nirvana", "score": 100, "disambiguation": "UK 60s"}
        ]}));
        assert_eq!(choose_artist("Nirvana", &namesakes, &[]), None);
        assert_eq!(
            choose_artist("Nirvana", &namesakes, &[OTHER.to_string()]).map(|artist| artist.mbid),
            Some(OTHER.to_string())
        );
        let weak = parse_artist_search(&json!({"artists": [
            {"id": BEATLES, "name": "Nirvana", "score": 60}
        ]}));
        assert_eq!(choose_artist("Nirvana", &weak, &[]), None);
    }

    #[test]
    fn a_renamed_artist_is_found_by_the_name_the_files_use() {
        assert!(
            artist_alias_search_url("Kimiko Ishizaka")
                .contains(&encode("alias:\"Kimiko Ishizaka\""))
        );
        let renamed = parse_artist_search(&json!({"artists": [
            {"id": BEATLES, "name": "Kimiko Douglass-Ishizaka", "score": 100,
             "aliases": [{"name": "Kimiko Ishizaka"}, {"name": "石坂紀美子"}]},
            {"id": OTHER, "name": "Kimiko", "score": 70}
        ]}));
        assert_eq!(renamed[0].aliases.len(), 2);
        assert!(!has_exact_name("Kimiko Ishizaka", &renamed));
        assert_eq!(choose_artist("Kimiko Ishizaka", &renamed, &[]), None);
        assert_eq!(
            choose_artist_by_alias("kimiko ishizaka", &renamed, &[]).map(|artist| artist.mbid),
            Some(BEATLES.to_string())
        );
        // Two artists sharing an alias stay ambiguous, unless the library
        // already knows one of them.
        let shared = parse_artist_search(&json!({"artists": [
            {"id": BEATLES, "name": "A", "score": 100, "aliases": [{"name": "Prince"}]},
            {"id": OTHER, "name": "B", "score": 100, "aliases": [{"name": "Prince"}]}
        ]}));
        assert_eq!(choose_artist_by_alias("Prince", &shared, &[]), None);
        assert_eq!(
            choose_artist_by_alias("Prince", &shared, &[OTHER.to_string()]).map(|a| a.mbid),
            Some(OTHER.to_string())
        );
        assert!(has_exact_name("a", &shared));
    }

    #[test]
    fn artist_lookup_reads_life_span_area_and_wikidata() {
        let profile = parse_artist(&json!({
            "id": BEATLES, "name": "The Beatles", "type": "Group",
            "area": {"name": "United Kingdom"}, "begin-area": {"name": "Liverpool"},
            "life-span": {"begin": "1960", "end": "1970-04-10", "ended": true},
            "disambiguation": "",
            "relations": [
                {"type": "discogs", "url": {"resource": "https://www.discogs.com/artist/82730"}},
                {"type": "wikidata", "url": {"resource": "https://www.wikidata.org/wiki/Q1299"}}
            ]
        }))
        .unwrap();
        assert_eq!(profile.kind.as_deref(), Some("Group"));
        assert_eq!(profile.begin_area.as_deref(), Some("Liverpool"));
        assert_eq!(profile.begin.as_deref(), Some("1960"));
        assert_eq!(profile.end.as_deref(), Some("1970-04-10"));
        assert_eq!(profile.disambiguation, None);
        assert_eq!(profile.wikidata_id.as_deref(), Some("Q1299"));
        assert!(!is_wikidata_id("Q12x"));
        assert!(!is_wikidata_id("P18"));
    }

    #[test]
    fn release_choice_matches_title_artist_and_prefers_track_count() {
        let candidates = parse_release_search(&json!({"releases": [
            {"id": BEATLES, "title": "Abbey Road", "score": 100, "track-count": 18,
             "date": "1987-10-19", "artist-credit": [{"name": "The Beatles", "joinphrase": ""}]},
            {"id": OTHER, "title": "Abbey Road", "score": 100, "track-count": 17,
             "date": "1969-09-26", "artist-credit": [{"name": "The Beatles"}]},
            {"id": "22222222-2222-4222-8222-222222222222", "title": "Abbey Road", "score": 100,
             "track-count": 17, "artist-credit": [{"name": "George Benson"}]}
        ]}));
        assert_eq!(
            choose_release("Abbey Road", "The Beatles", 17, &candidates)
                .map(|release| release.mbid),
            Some(OTHER.to_string())
        );
        // No track-count match: the earliest release, usually the original.
        assert_eq!(
            choose_release("Abbey Road", "The Beatles", 3, &candidates).map(|release| release.mbid),
            Some(OTHER.to_string())
        );
        assert_eq!(
            choose_release("Abbey Road", "The Beatles", 18, &candidates)
                .map(|release| release.mbid),
            Some(BEATLES.to_string())
        );
        assert_eq!(
            choose_release("Let It Be", "The Beatles", 12, &candidates),
            None
        );
    }

    #[test]
    fn a_conductor_album_artist_matches_a_release_credited_to_several_artists() {
        let candidates = parse_release_search(&json!({"releases": [
            {"id": BEATLES, "title": "Symphonie Nr. 9", "score": 100, "track-count": 4,
             "artist-credit": [
                {"name": "Ludwig van Beethoven", "joinphrase": "; "},
                {"name": "Berliner Philharmoniker", "joinphrase": ", "},
                {"name": "Herbert von Karajan"}
             ]}
        ]}));
        assert_eq!(
            choose_release("Symphonie Nr. 9", "Herbert von Karajan", 4, &candidates)
                .map(|release| release.mbid),
            Some(BEATLES.to_string())
        );
        assert_eq!(
            choose_release("Symphonie Nr. 9", "Karl Böhm", 4, &candidates),
            None
        );
    }

    #[test]
    fn classical_titles_also_match_without_the_composer_prefix() {
        assert_eq!(
            title_variants("Beethoven: Symphony No. 9"),
            ["Beethoven: Symphony No. 9", "Symphony No. 9"]
        );
        assert_eq!(title_variants("Kind of Blue"), ["Kind of Blue"]);
        // A colon after a number is part of the title, not a composer.
        assert_eq!(title_variants("1999: Remastered"), ["1999: Remastered"]);
        assert!(
            release_search_url("Beethoven: Symphony No. 9", "Herbert von Karajan").contains(
                &encode("release:(\"Beethoven: Symphony No. 9\"^3 OR \"Symphony No. 9\"^3 OR (beethoven symphony no 9))")
            )
        );
        let candidates = parse_release_search(&json!({"releases": [
            {"id": BEATLES, "title": "Symphony no. 9", "score": 100, "track-count": 5,
             "artist-credit": [{"name": "Beethoven", "joinphrase": "; "},
                               {"name": "Berliner Philharmoniker", "joinphrase": ", "},
                               {"name": "Herbert von Karajan"}]}
        ]}));
        assert_eq!(
            choose_release(
                "Beethoven: Symphony No. 9",
                "Herbert von Karajan",
                4,
                &candidates
            )
            .map(|release| release.mbid),
            Some(BEATLES.to_string())
        );
    }

    #[test]
    fn the_album_year_picks_between_recordings_of_one_title() {
        let candidates = parse_release_search(&json!({"releases": [
            {"id": BEATLES, "title": "Symphony no. 9", "score": 100, "track-count": 5, "date": "1977",
             "artist-credit": [{"name": "Herbert von Karajan"}],
             "label-info": [{"catalog-number": "[none]", "label": {"name": "[no label]"}}]},
            {"id": OTHER, "title": "Symphony no. 9", "score": 100, "track-count": 5, "date": "1963-05",
             "artist-credit": [{"name": "Herbert von Karajan"}]}
        ]}));
        assert!(candidates[0].labels.is_empty() && candidates[0].catalog_numbers.is_empty());
        let pick = |year| {
            choose_release_in_year(
                "Symphony No. 9",
                "Herbert von Karajan",
                4,
                year,
                &candidates,
            )
            .map(|release| release.mbid)
        };
        assert_eq!(pick(Some("1977")), Some(BEATLES.to_string()));
        assert_eq!(pick(Some("1963")), Some(OTHER.to_string()));
        // No year, or none matching: the earliest release.
        assert_eq!(pick(None), Some(OTHER.to_string()));
        assert_eq!(pick(Some("2001")), Some(OTHER.to_string()));
    }

    #[test]
    fn placeholder_names_are_recognised() {
        for name in [
            "Unknown Artist",
            "unknown",
            "[unknown]",
            "Various Artists",
            "VA",
            "V.A.",
            "",
            "Anonymous",
        ] {
            assert!(is_placeholder_artist(name), "{name}");
        }
        for name in ["Miles Davis", "Unknown Mortal Orchestra", "The Unknowns"] {
            assert!(!is_placeholder_artist(name), "{name}");
        }
    }

    #[test]
    fn artist_aliases_and_credited_names_are_read() {
        let body = json!({
            "id": BEATLES, "name": "Q-Tip",
            "aliases": [
                {"name": "Jonathan Davis", "type": "Legal name"},
                {"name": "Kamaal Ibn John Fareed", "type": "Legal name"},
                {"name": "Q Tip", "type": "Search hint"},
                {"name": "Jonathan Davis", "type": "Artist name"}
            ]
        });
        let profile = parse_artist(&body).unwrap();
        assert_eq!(
            profile.aliases,
            vec!["Jonathan Davis", "Kamaal Ibn John Fareed"]
        );
        assert!(artist_lookup_url(BEATLES).contains("aliases"));
        let release = json!({
            "id": BEATLES,
            "relations": [{
                "type": "producer", "target-type": "artist",
                "target-credit": "Jonathan Davis",
                "artist": {"id": OTHER, "name": "Q-Tip"}
            }]
        });
        let details = parse_release(&release).unwrap();
        assert_eq!(details.credits[0].name, "Jonathan Davis");
        assert_eq!(details.credits[0].artist_name.as_deref(), Some("Q-Tip"));
    }

    #[test]
    fn relationship_types_map_to_credit_roles() {
        assert_eq!(
            relationship_role("producer", &["executive".into()]),
            Some(("producer".into(), None))
        );
        assert_eq!(
            relationship_role("instrument", &["additional".into(), "Piano".into()]),
            Some(("performer".into(), Some("piano".into())))
        );
        assert_eq!(
            relationship_role("vocal", &[]),
            Some(("performer".into(), Some("vocals".into())))
        );
        assert_eq!(
            relationship_role("vocal", &["lead vocals".into()]),
            Some(("performer".into(), Some("lead vocals".into())))
        );
        assert_eq!(relationship_role("mix", &[]), Some(("mixer".into(), None)));
        assert_eq!(
            relationship_role("mastering", &[]),
            Some(("engineer".into(), None))
        );
        assert_eq!(
            relationship_role("performing orchestra", &[]),
            Some(("ensemble".into(), None))
        );
        assert_eq!(relationship_role("cover art", &[]), None);
        assert_eq!(
            relationship_role("instrument", &["drums (drum set)".into()]),
            Some(("performer".into(), Some("drums".into())))
        );
        assert_eq!(normalize_instrument("Upright Bass"), "double bass");
    }

    #[test]
    fn release_lookup_reads_labels_tracks_and_work_credits() {
        let release = parse_release(&json!({
            "id": BEATLES, "title": "Kind of Blue", "date": "1959-08-17", "country": "US",
            "barcode": "", "label-info": [{"catalog-number": "CL 1355", "label": {"name": "Columbia"}}],
            "release-group": {"id": OTHER, "primary-type": "Album", "first-release-date": "1959-08-17"},
            "artist-credit": [{"name": "Miles Davis", "artist": {"id": OTHER, "name": "Miles Davis"}}],
            "relations": [
                {"type": "producer", "target-type": "artist", "attributes": [],
                 "artist": {"id": OTHER, "name": "Irving Townsend"}},
                {"type": "cover art", "target-type": "artist", "attributes": [],
                 "artist": {"id": OTHER, "name": "Someone"}}
            ],
            "media": [{"position": 1, "tracks": [{
                "position": 3, "title": "Blue in Green",
                "recording": {"id": BEATLES, "title": "Blue in Green", "relations": [
                    {"type": "instrument", "target-type": "artist", "attributes": ["piano"],
                     "target-credit": "Bill Evans", "artist": {"id": OTHER, "name": "Bill Evans (2)"}},
                    {"type": "performance", "target-type": "work", "work": {"title": "Blue in Green", "relations": [
                        {"type": "composer", "target-type": "artist", "attributes": [],
                         "artist": {"id": OTHER, "name": "Miles Davis"}},
                        {"type": "composer", "target-type": "artist", "attributes": [],
                         "artist": {"id": BEATLES, "name": "Bill Evans"}}
                    ]}}
                ]}
            }]}]
        }))
        .unwrap();
        assert_eq!(
            release.labels,
            [("Columbia".to_string(), Some("CL 1355".to_string()))]
        );
        assert_eq!(release.barcode, None);
        assert_eq!(release.release_group_mbid.as_deref(), Some(OTHER));
        assert_eq!(release.credits.len(), 1);
        assert_eq!(release.credits[0].name, "Irving Townsend");
        let track = &release.tracks[0];
        assert_eq!((track.disc, track.position), (1, 3));
        let summary = track
            .credits
            .iter()
            .map(|credit| {
                (
                    credit.name.as_str(),
                    credit.role.as_str(),
                    credit.instrument.as_deref(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            summary,
            [
                ("Bill Evans", "performer", Some("piano")),
                ("Miles Davis", "composer", None),
                ("Bill Evans", "composer", None)
            ]
        );
    }

    #[test]
    fn genres_keep_voted_names_most_votes_first_and_image_links_name_a_commons_file() {
        let body = json!({
            "id": BEATLES, "name": "Miles Davis",
            "genres": [
                {"id": OTHER, "name": "jazz", "count": 4},
                {"name": "hard bop", "count": 9},
                {"name": "unvoted", "count": 0}
            ],
            "relations": [
                {"type": "image", "url": {"resource": "https://commons.wikimedia.org/wiki/File:Miles_Davis_by_Palumbo%C3%A9.jpg"}},
            ]
        });
        let profile = parse_artist(&body).expect("artist");
        assert_eq!(
            profile
                .genres
                .iter()
                .map(|genre| (genre.name.as_str(), genre.count))
                .collect::<Vec<_>>(),
            [("hard bop", 9), ("jazz", 4)]
        );
        assert_eq!(profile.genres[1].mbid.as_deref(), Some(OTHER));
        assert_eq!(
            profile.image_file.as_deref(),
            Some("Miles Davis by Palumboé.jpg")
        );
        assert!(artist_lookup_url(BEATLES).contains("inc=url-rels+genres"));
        assert!(release_group_lookup_url(BEATLES).contains("inc=url-rels+genres"));
        let elsewhere = json!({"relations": [
            {"type": "image", "url": {"resource": "https://example.com/wiki/File:x.jpg"}}
        ]});
        assert_eq!(image_relation(&elsewhere), None);
    }

    #[test]
    fn wikipedia_and_work_links_are_read_from_url_relations() {
        let body = serde_json::json!({
            "id": "22222222-2222-4222-8222-222222222222",
            "title": "Symphony no. 9 in D minor, op. 125",
            "relations": [
                {"type": "wikipedia", "url": {"resource": "https://de.wikipedia.org/wiki/9._Sinfonie_(Beethoven)"}},
                {"type": "wikipedia", "url": {"resource": "https://en.wikipedia.org/wiki/Symphony_No._9_(Beethoven)"}},
                {"type": "wikidata", "url": {"resource": "https://www.wikidata.org/wiki/Q190125"}}
            ]
        });
        assert_eq!(
            wikipedia_relation(&body),
            Some(("en".into(), "Symphony No. 9 (Beethoven)".into()))
        );
        let links = parse_work_links(&body).unwrap();
        assert_eq!(links.wikidata_id.as_deref(), Some("Q190125"));
        assert_eq!(links.title, "Symphony no. 9 in D minor, op. 125");
        assert!(
            work_lookup_url(&links.mbid).ends_with("?inc=artist-rels+work-rels+url-rels&fmt=json")
        );
        assert!(parse_work_links(&serde_json::json!({"id": "nope"})).is_none());
        assert_eq!(
            wikipedia_relation(&serde_json::json!({"relations": []})),
            None
        );
    }
}
