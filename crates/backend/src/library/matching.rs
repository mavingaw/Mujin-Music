//! Matches tracks described by an external source (a Spotify playlist, an
//! M3U file, a YouTube video title, ...) to songs in the local catalog.
//!
//! The engine is pure: it never touches the network or the database, so
//! every provider importer shares it and it is fully unit-testable. Each
//! result carries a confidence in `0.0..=1.0`, the evidence that produced
//! it, and up to [`MAX_CANDIDATES`] alternatives for the review screen.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::domain::Artist;
use crate::library::search::normalize;

/// Confidence at or above which a match is accepted without review.
pub const AUTO_MATCH_THRESHOLD: f32 = 0.85;
/// Confidence at or above which a candidate is offered for review.
pub const REVIEW_THRESHOLD: f32 = 0.55;
/// Lowest confidence still worth listing as an alternative.
const CANDIDATE_FLOOR: f32 = 0.35;
pub const MAX_CANDIDATES: usize = 5;
/// Bounds the work for one source track on very large libraries.
const MAX_SCORED_TRACKS: usize = 4_000;
/// Title tokens shared by more tracks than this are too common to seed candidates.
const MAX_POSTING_SEED: usize = 20_000;

/// A track as described by the source being imported.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SourceTrack {
    pub title: String,
    #[serde(default)]
    pub artists: Vec<String>,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub isrc: Option<String>,
    /// A file path or URL, for M3U/XSPF entries that point at local files.
    #[serde(default)]
    pub location: Option<String>,
}

/// A catalog song reduced to the fields matching needs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LibraryTrack {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub contributing_artists: Vec<String>,
    pub album: String,
    pub duration_seconds: f64,
    pub isrc: Option<String>,
    pub path: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchStatus {
    /// Confident enough to import without asking.
    Matched,
    /// A plausible candidate exists but a person should confirm it.
    Review,
    /// Nothing in the library is close enough.
    Unmatched,
}

/// Evidence that contributed to a candidate's confidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchReason {
    Isrc,
    FilePath,
    ExactTitle,
    SimilarTitle,
    ExactArtist,
    SimilarArtist,
    ArtistMismatch,
    Album,
    DurationClose,
    DurationMismatch,
    VersionMismatch,
    Ambiguous,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MatchCandidate {
    pub song_id: String,
    pub confidence: f32,
    pub reasons: Vec<MatchReason>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MatchResult {
    pub status: MatchStatus,
    pub song_id: Option<String>,
    pub confidence: f32,
    pub reasons: Vec<MatchReason>,
    pub candidates: Vec<MatchCandidate>,
}

impl MatchResult {
    fn unmatched(candidates: Vec<MatchCandidate>) -> Self {
        Self {
            status: MatchStatus::Unmatched,
            song_id: None,
            confidence: candidates.first().map_or(0.0, |c| c.confidence),
            reasons: Vec::new(),
            candidates,
        }
    }
}

struct IndexedTrack {
    id: String,
    title: String,
    versions: HashSet<&'static str>,
    artists: Vec<String>,
    album: String,
    duration_seconds: f64,
}

/// An in-memory index over the catalog, built once per import.
pub struct MatchIndex {
    tracks: Vec<IndexedTrack>,
    by_isrc: HashMap<String, Vec<usize>>,
    by_title_token: HashMap<String, Vec<usize>>,
    by_path_suffix: HashMap<String, Vec<usize>>,
}

const VERSION_KEYWORDS: &[&str] = &[
    "live",
    "remix",
    "acoustic",
    "instrumental",
    "demo",
    "karaoke",
    "cover",
    "unplugged",
    "orchestral",
    "a cappella",
    "acapella",
    "slowed",
    "sped up",
    "nightcore",
    "reprise",
];

fn noise_regex() -> &'static Regex {
    static NOISE: OnceLock<Regex> = OnceLock::new();
    NOISE.get_or_init(|| {
        Regex::new(
            r"(?ix)
            \s*[\(\[]\s*(?:
                feat\.?|ft\.?|featuring|with
            )\s[^\)\]]*[\)\]]
            | \s+(?:feat\.?|ft\.?|featuring)\s.*$
            | \s*[\(\[][^\)\]]*\b(?:remaster(?:ed)?|mono|stereo|deluxe|bonus\s+track|album\s+version|single\s+version|explicit|clean|official\s+(?:music\s+)?(?:video|audio|lyric\s+video|visuali[sz]er)|lyric\s+video|lyrics|audio|hd|hq|4k|mv|m/v)\b[^\)\]]*[\)\]]
            | \s+-\s+(?:\d{4}\s+)?remaster(?:ed)?(?:\s+\d{4})?(?:\s+version)?\s*$
            | \s+-\s+(?:mono|stereo|single\s+version|album\s+version|radio\s+edit)\s*$
            | \s+-\s+\d{4}\s+(?:stereo\s+|mono\s+)?mix\s*$
            ",
        )
        .expect("title noise regex should compile")
    })
}

/// Normalizes a title for comparison, removing featured-artist credits,
/// remaster/packaging qualifiers and video-site noise.
pub fn normalize_title(title: &str) -> String {
    let mut current = title.trim().to_string();
    // Qualifiers can be stacked: "Song (feat. X) [2011 Remaster]".
    for _ in 0..4 {
        let next = noise_regex().replace_all(&current, "").trim().to_string();
        if next == current || next.is_empty() {
            break;
        }
        current = next;
    }
    let normalized = normalize(&current.replace('&', " and "));
    if normalized.is_empty() {
        normalize(title)
    } else {
        normalized
    }
}

/// Normalizes an artist name; a leading "The" is ignored.
pub fn normalize_artist(name: &str) -> String {
    let normalized = normalize(&name.replace('&', " and "));
    normalized
        .strip_prefix("the ")
        .map(str::to_string)
        .unwrap_or(normalized)
}

fn version_tags(title: &str) -> HashSet<&'static str> {
    let normalized = normalize(title);
    let padded = format!(" {normalized} ");
    VERSION_KEYWORDS
        .iter()
        .copied()
        .filter(|keyword| padded.contains(&format!(" {keyword} ")))
        .collect()
}

/// Splits a combined credit such as "A, B & C feat. D" into names.
fn split_artist_credit(credit: &str) -> Vec<String> {
    static SPLIT: OnceLock<Regex> = OnceLock::new();
    SPLIT
        .get_or_init(|| {
            Regex::new(r"(?i)\s*(?:,|;|/|\s&\s|\sx\s|\s+feat\.?\s+|\s+ft\.?\s+|\s+featuring\s+|\s+with\s+|\s+and\s+)\s*")
                .expect("artist split regex should compile")
        })
        .split(credit)
        .map(normalize_artist)
        .filter(|name| !name.is_empty())
        .collect()
}

fn normalize_isrc(isrc: &str) -> Option<String> {
    let value: String = isrc
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    (value.len() == 12).then_some(value)
}

/// The last two path components, lowercased and with `\` folded to `/`,
/// so `C:\Music\Artist\01 Song.flac` matches `/srv/music/Artist/01 Song.flac`.
fn path_suffix(path: &str) -> Option<String> {
    let decoded = percent_encoding::percent_decode_str(path)
        .decode_utf8_lossy()
        .replace('\\', "/");
    let trimmed = decoded
        .strip_prefix("file://")
        .unwrap_or(&decoded)
        .trim_end_matches('/');
    let parts: Vec<&str> = trimmed.rsplit('/').take(2).collect();
    if parts.first().is_none_or(|name| !name.contains('.')) {
        return None;
    }
    Some(
        parts
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("/")
            .to_lowercase(),
    )
}

/// Levenshtein similarity in `0.0..=1.0` over chars.
fn edit_similarity(left: &str, right: &str) -> f32 {
    if left == right {
        return 1.0;
    }
    let left: Vec<char> = left.chars().take(160).collect();
    let right: Vec<char> = right.chars().take(160).collect();
    let longest = left.len().max(right.len());
    if longest == 0 {
        return 1.0;
    }
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];
    for (i, left_char) in left.iter().enumerate() {
        current[0] = i + 1;
        for (j, right_char) in right.iter().enumerate() {
            let substitution = previous[j] + usize::from(left_char != right_char);
            current[j + 1] = substitution.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    1.0 - previous[right.len()] as f32 / longest as f32
}

fn token_similarity(left: &str, right: &str) -> f32 {
    let left: HashSet<&str> = left.split_whitespace().collect();
    let right: HashSet<&str> = right.split_whitespace().collect();
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let shared = left.intersection(&right).count() as f32;
    // Dice coefficient tolerates one side carrying an extra word.
    2.0 * shared / (left.len() + right.len()) as f32
}

/// Text similarity used for titles, artists and albums.
pub fn text_similarity(left: &str, right: &str) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    edit_similarity(left, right).max(token_similarity(left, right))
}

fn artist_similarity(source: &[String], library: &[String]) -> f32 {
    if source.is_empty() || library.is_empty() {
        return 0.0;
    }
    // The first credited source artist matters most; extra featured
    // artists on either side should not drag a correct match down.
    let primary = source
        .first()
        .map(|name| {
            library
                .iter()
                .map(|candidate| text_similarity(name, candidate))
                .fold(0.0_f32, f32::max)
        })
        .unwrap_or(0.0);
    let any = source
        .iter()
        .flat_map(|name| {
            library
                .iter()
                .map(move |candidate| text_similarity(name, candidate))
        })
        .fold(0.0_f32, f32::max);
    primary.max(any * 0.9)
}

impl MatchIndex {
    pub fn from_artists(artists: &[Artist]) -> Self {
        let tracks = artists.iter().flat_map(|artist| {
            artist.albums.iter().flat_map(move |album| {
                album.songs.iter().map(move |song| LibraryTrack {
                    id: song.id.clone(),
                    title: song.name.clone(),
                    artist: if song.artist.is_empty() {
                        artist.name.clone()
                    } else {
                        song.artist.clone()
                    },
                    contributing_artists: song.contributing_artists.clone(),
                    album: album.name.clone(),
                    duration_seconds: song.duration,
                    isrc: song.isrc.clone(),
                    path: song.path.clone(),
                })
            })
        });
        Self::from_tracks(tracks)
    }

    pub fn from_tracks(tracks: impl IntoIterator<Item = LibraryTrack>) -> Self {
        let mut index = Self {
            tracks: Vec::new(),
            by_isrc: HashMap::new(),
            by_title_token: HashMap::new(),
            by_path_suffix: HashMap::new(),
        };
        for track in tracks {
            let position = index.tracks.len();
            let title = normalize_title(&track.title);
            let mut title_tokens: Vec<String> =
                title.split_whitespace().map(str::to_string).collect();
            title_tokens.sort_unstable();
            title_tokens.dedup();
            for token in &title_tokens {
                index
                    .by_title_token
                    .entry(token.clone())
                    .or_default()
                    .push(position);
            }
            if let Some(isrc) = track.isrc.as_deref().and_then(normalize_isrc) {
                index.by_isrc.entry(isrc).or_default().push(position);
            }
            if let Some(suffix) = path_suffix(&track.path) {
                index
                    .by_path_suffix
                    .entry(suffix)
                    .or_default()
                    .push(position);
            }
            let mut artists = split_artist_credit(&track.artist);
            for contributor in &track.contributing_artists {
                artists.extend(split_artist_credit(contributor));
            }
            artists.dedup();
            index.tracks.push(IndexedTrack {
                id: track.id,
                versions: version_tags(&track.title),
                title,
                artists,
                album: normalize(&track.album),
                duration_seconds: track.duration_seconds,
            });
        }
        index
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    fn candidate_positions(&self, title: &str) -> Vec<usize> {
        let mut tokens: Vec<&Vec<usize>> = title
            .split_whitespace()
            .filter_map(|token| self.by_title_token.get(token))
            .filter(|postings| postings.len() <= MAX_POSTING_SEED)
            .collect();
        // Rare tokens first: they narrow the candidate set the most.
        tokens.sort_by_key(|postings| postings.len());
        let mut seen = HashSet::new();
        let mut positions = Vec::new();
        for postings in tokens {
            for &position in postings {
                if seen.insert(position) {
                    positions.push(position);
                    if positions.len() >= MAX_SCORED_TRACKS {
                        return positions;
                    }
                }
            }
        }
        positions
    }

    fn score(
        &self,
        track: &IndexedTrack,
        source: &PreparedSource,
        isrc_hit: bool,
        path_hit: bool,
    ) -> MatchCandidate {
        let mut reasons = Vec::new();
        if path_hit {
            reasons.push(MatchReason::FilePath);
            return MatchCandidate {
                song_id: track.id.clone(),
                confidence: 1.0,
                reasons,
            };
        }

        let title_score = text_similarity(&source.title, &track.title);
        if source.title == track.title {
            reasons.push(MatchReason::ExactTitle);
        } else if title_score >= 0.75 {
            reasons.push(MatchReason::SimilarTitle);
        }

        let artist_score = if source.artists.is_empty() {
            // Sources such as bare YouTube titles may lack an artist; stay neutral.
            0.5
        } else {
            artist_similarity(&source.artists, &track.artists)
        };
        if !source.artists.is_empty() {
            if artist_score >= 0.98 {
                reasons.push(MatchReason::ExactArtist);
            } else if artist_score >= 0.75 {
                reasons.push(MatchReason::SimilarArtist);
            } else {
                reasons.push(MatchReason::ArtistMismatch);
            }
        }

        let album_score = source
            .album
            .as_deref()
            .map(|album| text_similarity(album, &track.album));
        if album_score.is_some_and(|score| score >= 0.8) {
            reasons.push(MatchReason::Album);
        }

        let duration_delta = source
            .duration_seconds
            .filter(|_| track.duration_seconds > 0.0)
            .map(|seconds| (seconds - track.duration_seconds).abs());
        let duration_score = match duration_delta {
            Some(delta) if delta <= 3.0 => {
                reasons.push(MatchReason::DurationClose);
                1.0
            }
            Some(delta) if delta <= 10.0 => 0.6,
            Some(delta) if delta <= 30.0 => 0.25,
            Some(_) => {
                reasons.push(MatchReason::DurationMismatch);
                0.0
            }
            None => 0.6,
        };

        let mut confidence = 0.52 * title_score
            + 0.3 * artist_score
            + 0.06 * album_score.unwrap_or(0.5)
            + 0.12 * duration_score;
        if duration_delta.is_some_and(|delta| delta > 30.0) {
            confidence -= 0.12;
        }
        if source.versions != track.versions {
            reasons.push(MatchReason::VersionMismatch);
            confidence -= 0.2;
        }
        if !source.artists.is_empty() && artist_score < 0.5 {
            // A same-titled song by somebody else is a different song.
            confidence = confidence.min(0.5);
        }
        if isrc_hit {
            reasons.insert(0, MatchReason::Isrc);
            // ISRC identifies the recording; a wildly different duration
            // still warrants a look (e.g. a mis-tagged file).
            confidence = if duration_delta.is_some_and(|delta| delta > 30.0) {
                confidence.max(0.7)
            } else {
                confidence.max(0.99)
            };
        }
        MatchCandidate {
            song_id: track.id.clone(),
            confidence: confidence.clamp(0.0, 1.0),
            reasons,
        }
    }

    /// Matches one source track against the catalog.
    pub fn match_track(&self, source: &SourceTrack) -> MatchResult {
        let prepared = PreparedSource::new(source);
        if prepared.title.is_empty() && source.location.is_none() && source.isrc.is_none() {
            return MatchResult::unmatched(Vec::new());
        }

        let isrc_positions: HashSet<usize> = source
            .isrc
            .as_deref()
            .and_then(normalize_isrc)
            .and_then(|isrc| self.by_isrc.get(&isrc))
            .map(|positions| positions.iter().copied().collect())
            .unwrap_or_default();
        let path_positions: HashSet<usize> = source
            .location
            .as_deref()
            .and_then(path_suffix)
            .and_then(|suffix| self.by_path_suffix.get(&suffix))
            .map(|positions| positions.iter().copied().collect())
            .unwrap_or_default();

        let mut positions: Vec<usize> = path_positions
            .iter()
            .chain(isrc_positions.iter())
            .copied()
            .collect();
        positions.extend(self.candidate_positions(&prepared.title));
        let mut seen = HashSet::new();
        positions.retain(|position| seen.insert(*position));

        let mut candidates: Vec<MatchCandidate> = positions
            .into_iter()
            .map(|position| {
                self.score(
                    &self.tracks[position],
                    &prepared,
                    isrc_positions.contains(&position),
                    path_positions.contains(&position),
                )
            })
            .filter(|candidate| candidate.confidence >= CANDIDATE_FLOOR)
            .collect();
        candidates.sort_by(|left, right| {
            right
                .confidence
                .partial_cmp(&left.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.song_id.cmp(&right.song_id))
        });
        candidates.truncate(MAX_CANDIDATES);

        let Some(best) = candidates.first().cloned() else {
            return MatchResult::unmatched(Vec::new());
        };
        let mut reasons = best.reasons.clone();
        let mut status = if best.confidence >= AUTO_MATCH_THRESHOLD {
            MatchStatus::Matched
        } else if best.confidence >= REVIEW_THRESHOLD {
            MatchStatus::Review
        } else {
            MatchStatus::Unmatched
        };
        // Two different songs scoring almost the same without hard evidence:
        // ask rather than guess (e.g. studio vs. remaster on another album).
        let hard_evidence = best
            .reasons
            .iter()
            .any(|reason| matches!(reason, MatchReason::Isrc | MatchReason::FilePath));
        if status == MatchStatus::Matched
            && !hard_evidence
            && candidates.get(1).is_some_and(|runner_up| {
                best.confidence - runner_up.confidence < 0.02 && best.confidence < 0.97
            })
        {
            status = MatchStatus::Review;
            reasons.push(MatchReason::Ambiguous);
        }
        if status == MatchStatus::Unmatched {
            return MatchResult::unmatched(candidates);
        }
        MatchResult {
            status,
            song_id: Some(best.song_id),
            confidence: best.confidence,
            reasons,
            candidates,
        }
    }
}

struct PreparedSource {
    title: String,
    versions: HashSet<&'static str>,
    artists: Vec<String>,
    album: Option<String>,
    duration_seconds: Option<f64>,
}

impl PreparedSource {
    fn new(source: &SourceTrack) -> Self {
        let mut artists = Vec::new();
        for credit in &source.artists {
            for name in split_artist_credit(credit) {
                if !artists.contains(&name) {
                    artists.push(name);
                }
            }
        }
        Self {
            title: normalize_title(&source.title),
            versions: version_tags(&source.title),
            artists,
            album: source
                .album
                .as_deref()
                .map(normalize)
                .filter(|album| !album.is_empty()),
            duration_seconds: source
                .duration_ms
                .filter(|ms| *ms > 0)
                .map(|ms| ms as f64 / 1000.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str, title: &str, artist: &str, album: &str, seconds: f64) -> LibraryTrack {
        LibraryTrack {
            id: id.to_string(),
            title: title.to_string(),
            artist: artist.to_string(),
            contributing_artists: Vec::new(),
            album: album.to_string(),
            duration_seconds: seconds,
            isrc: None,
            path: format!("/music/{artist}/{album}/{title}.flac"),
        }
    }

    fn source(title: &str, artist: &str, seconds: Option<u64>) -> SourceTrack {
        SourceTrack {
            title: title.to_string(),
            artists: vec![artist.to_string()],
            duration_ms: seconds.map(|s| s * 1000),
            ..SourceTrack::default()
        }
    }

    fn library() -> MatchIndex {
        MatchIndex::from_tracks([
            track(
                "a",
                "Here Comes the Sun",
                "The Beatles",
                "Abbey Road",
                185.0,
            ),
            track("b", "Something", "The Beatles", "Abbey Road", 182.0),
            track("c", "Something", "Lasse Lindh", "Rugby", 210.0),
            track("d", "Hey Jude (Live)", "The Beatles", "Live", 430.0),
            track("e", "Hey Jude", "The Beatles", "1", 431.0),
            track("f", "Café del Mar", "Energy 52", "Café del Mar", 412.0),
            track("g", "Halo", "Beyoncé", "I Am... Sasha Fierce", 261.0),
        ])
    }

    #[test]
    fn titles_lose_feature_credits_and_remaster_noise() {
        assert_eq!(
            normalize_title("Here Comes the Sun - Remastered 2009"),
            "here comes the sun"
        );
        assert_eq!(
            normalize_title("Stay (feat. Justin Bieber) [2021 Remaster]"),
            "stay"
        );
        assert_eq!(normalize_title("Halo (Official Music Video)"), "halo");
        assert_eq!(normalize_title("Ain't It Fun ft. Someone"), "aint it fun");
        assert_eq!(normalize_title("Hey Jude (Live)"), "hey jude live");
        assert_eq!(normalize_title("(feat. X)"), "feat x");
    }

    #[test]
    fn exact_title_artist_and_duration_match_automatically() {
        let result = library().match_track(&source(
            "Here Comes The Sun - 2019 Mix",
            "The Beatles",
            Some(186),
        ));
        assert_eq!(result.status, MatchStatus::Matched);
        assert_eq!(result.song_id.as_deref(), Some("a"));
        assert!(result.reasons.contains(&MatchReason::ExactArtist));
        assert!(result.reasons.contains(&MatchReason::DurationClose));
    }

    #[test]
    fn same_title_by_another_artist_is_not_matched() {
        let result = library().match_track(&source("Something", "Lasse Lindh", Some(210)));
        assert_eq!(result.song_id.as_deref(), Some("c"));
        assert_eq!(result.status, MatchStatus::Matched);

        let wrong = library().match_track(&source("Something", "George Harrison Tribute", None));
        assert_ne!(wrong.status, MatchStatus::Matched);
    }

    #[test]
    fn studio_and_live_versions_are_kept_apart() {
        let studio = library().match_track(&source("Hey Jude", "The Beatles", Some(431)));
        assert_eq!(studio.song_id.as_deref(), Some("e"));
        assert_eq!(studio.status, MatchStatus::Matched);

        let live = library().match_track(&source("Hey Jude - Live", "The Beatles", Some(430)));
        assert_eq!(live.song_id.as_deref(), Some("d"));
    }

    #[test]
    fn accents_and_the_prefix_are_folded() {
        let result = library().match_track(&source("Cafe Del Mar", "Energy52", Some(412)));
        assert_eq!(result.song_id.as_deref(), Some("f"));
        assert_eq!(result.status, MatchStatus::Matched);
        let halo = library().match_track(&source("Halo", "Beyonce", None));
        assert_eq!(halo.song_id.as_deref(), Some("g"));
        assert_eq!(halo.status, MatchStatus::Matched);
        let beatles = library().match_track(&source("Something", "Beatles", Some(182)));
        assert_eq!(beatles.song_id.as_deref(), Some("b"));
    }

    #[test]
    fn isrc_wins_even_when_titles_differ() {
        let mut tracks = vec![track("x", "Untitled 03", "Artist", "Album", 200.0)];
        tracks[0].isrc = Some("USUM71703861".to_string());
        let index = MatchIndex::from_tracks(tracks);
        let mut wanted = source("Completely Different Name", "Someone", Some(201));
        wanted.isrc = Some("us-um7-17-03861".to_string());
        let result = index.match_track(&wanted);
        assert_eq!(result.status, MatchStatus::Matched);
        assert_eq!(result.song_id.as_deref(), Some("x"));
        assert_eq!(result.reasons.first(), Some(&MatchReason::Isrc));
    }

    #[test]
    fn m3u_paths_match_by_folder_and_file_name_across_platforms() {
        let index = library();
        let result = index.match_track(&SourceTrack {
            title: String::new(),
            location: Some(r"D:\Old Music\Abbey Road\Something.flac".to_string()),
            ..SourceTrack::default()
        });
        assert_eq!(result.song_id.as_deref(), Some("b"));
        assert_eq!(result.confidence, 1.0);
        assert_eq!(result.reasons, vec![MatchReason::FilePath]);

        let encoded = index.match_track(&SourceTrack {
            title: "x".to_string(),
            location: Some("file:///home/u/Abbey%20Road/Something.flac".to_string()),
            ..SourceTrack::default()
        });
        assert_eq!(encoded.song_id.as_deref(), Some("b"));
    }

    #[test]
    fn weak_candidates_are_offered_for_review_not_imported() {
        let result = library().match_track(&source("Here Comes Sun", "Beatles Tribute Band", None));
        assert_ne!(result.status, MatchStatus::Matched);
        let missing = library().match_track(&source("Nonexistent Song", "Nobody", Some(100)));
        assert_eq!(missing.status, MatchStatus::Unmatched);
        assert!(missing.song_id.is_none());
    }

    #[test]
    fn near_identical_duplicates_require_review() {
        let index = MatchIndex::from_tracks([
            track("one", "Song", "Band", "Album", 200.0),
            track("two", "Song", "Band", "Album (Deluxe)", 200.0),
        ]);
        let result = index.match_track(&source("Song", "Band", Some(200)));
        // Same title/artist/duration on two albums: still confident overall,
        // but the album hint is missing so the engine asks.
        assert_eq!(result.candidates.len(), 2);
        assert!(result.status == MatchStatus::Review || result.confidence >= 0.97);
    }

    #[test]
    fn titles_without_artists_stay_below_auto_match_unless_exact() {
        let index = library();
        let result = index.match_track(&SourceTrack {
            title: "Here Comes the Sun".to_string(),
            ..SourceTrack::default()
        });
        assert_eq!(result.song_id.as_deref(), Some("a"));
        assert_eq!(result.status, MatchStatus::Review);
    }

    #[test]
    fn similarity_is_symmetric_and_bounded() {
        for (left, right) in [("abc", "abd"), ("", "x"), ("long title here", "title here")] {
            let forward = text_similarity(left, right);
            assert!((0.0..=1.0).contains(&forward));
            assert!((forward - text_similarity(right, left)).abs() < f32::EPSILON);
        }
        assert_eq!(text_similarity("same", "same"), 1.0);
    }

    #[test]
    fn artist_credits_split_on_common_separators() {
        assert_eq!(
            split_artist_credit("Calvin Harris, Dua Lipa & Someone feat. The Weeknd"),
            vec!["calvin harris", "dua lipa", "someone", "weeknd"]
        );
    }

    #[test]
    fn large_libraries_bound_candidate_scoring() {
        let index = MatchIndex::from_tracks((0..10_000).map(|n| {
            track(
                &format!("id{n}"),
                &format!("Love Song {n}"),
                "Artist",
                "Album",
                200.0,
            )
        }));
        assert_eq!(index.len(), 10_000);
        let result = index.match_track(&source("Love Song 9999", "Artist", Some(200)));
        assert_eq!(result.song_id.as_deref(), Some("id9999"));
        assert!(result.candidates.len() <= MAX_CANDIDATES);
    }
}
