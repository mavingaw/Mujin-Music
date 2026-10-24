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

pub fn artist_lookup_url(mbid: &str) -> String {
    format!("{API_ROOT}/artist/{mbid}?inc=url-rels&fmt=json")
}

pub fn release_search_url(title: &str, artist: &str) -> String {
    format!(
        "{API_ROOT}/release?query={}&limit=10&fmt=json",
        encode(&format!(
            "release:{} AND artist:{}",
            phrase(title),
            phrase(artist)
        ))
    )
}

pub fn release_lookup_url(mbid: &str) -> String {
    format!(
        "{API_ROOT}/release/{mbid}?inc=artist-credits+labels+recordings+release-groups+artist-rels+recording-level-rels+work-rels+work-level-rels&fmt=json"
    )
}

pub fn release_group_lookup_url(mbid: &str) -> String {
    format!("{API_ROOT}/release-group/{mbid}?inc=url-rels&fmt=json")
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
            })
        })
        .collect()
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
    pub track_count: Option<i64>,
    pub date: Option<String>,
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
                track_count: release.get("track-count").and_then(Value::as_i64),
                date: text(release, "date"),
            })
        })
        .collect()
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
    let wanted_title = normalize_album_identity(title);
    let wanted_artist = normalize_artist_identity(artist);
    let matching = candidates
        .iter()
        .filter(|candidate| {
            candidate.score >= MIN_SEARCH_SCORE
                && normalize_album_identity(&candidate.title) == wanted_title
                && normalize_artist_identity(&candidate.artist) == wanted_artist
        })
        .collect::<Vec<_>>();
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
    pub name: String,
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
    pub credits: Vec<ExternalCredit>,
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
                credits: track_credits,
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
        assert!(
            release_search_url("Kind of Blue", "Miles Davis")
                .contains("release%3A%22Kind%20of%20Blue%22%20AND%20artist%3A%22Miles%20Davis%22")
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
}
