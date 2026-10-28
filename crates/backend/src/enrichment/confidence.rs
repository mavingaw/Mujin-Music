//! The match check that stands between a MusicBrainz search result and the
//! library: before the cover, performers, works or release type of an
//! automatically found release are applied to a local album, the release is
//! compared with what the files themselves say.
//!
//! Each piece of evidence is a [`Signal`] worth a few points for or against:
//!
//! - track count (the same number of tracks, or the local tracks being a
//!   part of the release),
//! - per-track lengths (the same performance, a different performance of
//!   the same programme, or lengths that cannot be compared),
//! - the year (and record label) in the tags against the release and its
//!   first release,
//! - the conductor and ensemble named in the tags against the people the
//!   release credits.
//!
//! A release needs [`VERIFY_THRESHOLD`] points. One that falls short is kept
//! only as a suggestion; the album keeps its file-tag data and asks a person
//! to identify it.

use serde::{Deserialize, Serialize};

use super::musicbrainz::{ReleaseCandidate, ReleaseDetails, title_variants};
use crate::library::identity::{normalize_album_identity, normalize_artist_identity};

/// Points a release needs before its data is applied automatically.
pub const VERIFY_THRESHOLD: i32 = 3;

/// Two lengths are the same recording when they differ by less than this
/// many seconds or [`SAME_LENGTH_SHARE`] of the longer one.
const SAME_LENGTH_SECONDS: f64 = 3.0;
const SAME_LENGTH_SHARE: f64 = 0.02;
/// Lengths within this share of each other are another performance of the
/// same music; further apart, the files are trimmed or something else.
const OTHER_PERFORMANCE_SHARE: f64 = 0.4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalOutcome {
    Match,
    Partial,
    Mismatch,
    Unknown,
}

/// One piece of evidence for or against a release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signal {
    /// `tracks`, `lengths`, `year`, `label`, `conductor` or `ensemble`.
    pub key: String,
    pub outcome: SignalOutcome,
    pub points: i32,
    /// A sentence a person can check, e.g. "The release has 2 tracks; your
    /// album has 5."
    pub detail: String,
}

/// The result of checking one release against a local album.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchCheck {
    pub score: i32,
    pub threshold: i32,
    pub verified: bool,
    pub signals: Vec<Signal>,
}

/// What the files say about an album, for the check.
#[derive(Debug, Clone, Default)]
pub struct LocalAlbum<'a> {
    pub track_count: usize,
    /// Pairs of (local length, release length) in seconds for the tracks
    /// that were paired by title or position.
    pub paired_lengths: Vec<(f64, f64)>,
    /// How many local tracks were paired with a release track.
    pub paired_tracks: usize,
    pub year: Option<&'a str>,
    pub label: Option<&'a str>,
    pub conductors: &'a [String],
    pub ensembles: &'a [String],
    pub composers: &'a [String],
}

fn plural(count: usize, word: &str) -> String {
    if count == 1 {
        format!("1 {word}")
    } else {
        format!("{count} {word}s")
    }
}

fn signal(key: &str, outcome: SignalOutcome, points: i32, detail: String) -> Signal {
    Signal {
        key: key.to_string(),
        outcome,
        points,
        detail,
    }
}

fn year_of(date: Option<&str>) -> Option<&str> {
    date.and_then(|date| date.get(..4))
        .filter(|year| year.bytes().all(|byte| byte.is_ascii_digit()))
}

fn clock(seconds: f64) -> String {
    let total = seconds.round().max(0.0) as u64;
    format!("{}:{:02}", total / 60, total % 60)
}

/// Title words with spelling variants folded together, so "Symphonien
/// Nr. 5 & 7" and "Symphonies Nos. 5 and 7" read alike. Joining words are
/// dropped.
pub fn title_tokens(title: &str) -> Vec<String> {
    normalize_artist_identity(title)
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .filter_map(|token| {
            let folded = match token {
                "and" | "und" | "et" | "e" | "y" | "plus" => return None,
                "no" | "nos" | "nr" | "nrn" | "num" | "number" | "numbers" | "n" => "no",
                "symphony" | "symphonies" | "symphonie" | "symphonien" | "sinfonie"
                | "sinfonien" | "sinfonia" | "sinfonias" => "symphony",
                "concerto" | "concertos" | "concerti" | "konzert" | "konzerte" | "concert" => {
                    "concerto"
                }
                "sonata" | "sonatas" | "sonate" | "sonaten" => "sonata",
                "quartet" | "quartets" | "quartett" | "quartette" | "quatuor" | "quatuors" => {
                    "quartet"
                }
                "op" | "opus" => "op",
                "vol" | "volume" => "vol",
                other => other,
            };
            Some(folded.to_string())
        })
        .collect()
}

/// How alike two titles are, from 0 to 1: the share of words they share
/// once spelling variants are folded.
pub fn title_similarity(left: &str, right: &str) -> f64 {
    let left = title_tokens(left);
    let right = title_tokens(right);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let shared = left.iter().filter(|token| right.contains(token)).count();
    let union = left.len() + right.len() - shared;
    shared as f64 / union as f64
}

/// How well a release title matches a local album title.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TitleMatch {
    /// Shares too few words to be the same album.
    No,
    /// Most words match (a subtitle or edition note differs).
    Similar,
    /// The same words once spellings are folded ("Nr." and "nos.").
    Folded,
    /// The same title, or the same title without the composer prefix.
    Exact,
}

/// A title without its parenthesised or bracketed parts.
fn strip_bracketed(title: &str) -> String {
    let mut depth = 0usize;
    let mut kept = String::with_capacity(title.len());
    for character in title.chars() {
        match character {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => kept.push(character),
            _ => {}
        }
    }
    kept.trim().to_string()
}

/// Every word of the local title (or of its variant without a composer
/// prefix) is in the release title.
fn title_words_within(local: &str, release: &str) -> bool {
    let release = title_tokens(release);
    title_variants(local).iter().any(|variant| {
        let words = title_tokens(variant);
        !words.is_empty() && words.iter().all(|word| release.contains(word))
    })
}

pub fn title_match(local: &str, release: &str) -> TitleMatch {
    let release_normalized = normalize_album_identity(release);
    let variants = title_variants(local);
    if variants
        .iter()
        .any(|variant| normalize_album_identity(variant) == release_normalized)
    {
        return TitleMatch::Exact;
    }
    let best = variants
        .iter()
        .map(|variant| title_similarity(variant, release))
        .fold(0.0, f64::max);
    // The same title once an edition note in brackets is set aside ("Blue
    // Train (Rudy Van Gelder edition)"). A quoted nickname is not an
    // edition note: "Symphony No. 9 \"From the New World\"" is another
    // symphony.
    let without_note = strip_bracketed(release);
    let edition_note = without_note != release
        && variants
            .iter()
            .any(|variant| title_similarity(variant, &without_note) >= 0.999);
    if best >= 0.999 {
        TitleMatch::Folded
    } else if best >= 0.6 || edition_note {
        TitleMatch::Similar
    } else {
        TitleMatch::No
    }
}

fn name_tokens(name: &str) -> Vec<String> {
    normalize_artist_identity(name)
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect()
}

/// Whether two credited names are the same person or group: the same
/// normalized name, or every word of the shorter one in the longer one
/// ("Karajan" and "Herbert von Karajan").
pub fn same_person(left: &str, right: &str) -> bool {
    let (left_normalized, right_normalized) = (
        normalize_artist_identity(left),
        normalize_artist_identity(right),
    );
    if left_normalized.is_empty() || right_normalized.is_empty() {
        return false;
    }
    if left_normalized == right_normalized {
        return true;
    }
    let (left, right) = (name_tokens(left), name_tokens(right));
    let (short, long) = if left.len() <= right.len() {
        (left, right)
    } else {
        (right, left)
    };
    !short.is_empty()
        && short.iter().all(|token| token.chars().count() >= 3)
        && short.iter().all(|token| long.contains(token))
}

/// Whether an artist credit names an orchestra, choir or other ensemble
/// rather than a person.
pub fn looks_like_ensemble(name: &str) -> bool {
    let name = normalize_artist_identity(name);
    [
        "orchest",
        "philharmon",
        "symphon",
        "sinfoni",
        "ensemble",
        "choir",
        "chor",
        "chorus",
        "kammer",
        "chamber",
        "consort",
        "staatskapelle",
        "gewandhaus",
        "concertgebouw",
        "camerata",
        "academy of",
        "akademie",
        "quartet",
        "quartett",
        "players",
        "solisten",
        "soloists",
    ]
    .iter()
    .any(|needle| name.contains(needle))
}

/// Everyone the release names: its artists and every credit.
fn release_people(release: &ReleaseDetails) -> Vec<(String, String)> {
    let mut people = Vec::new();
    for (_, name) in &release.artists {
        let role = if looks_like_ensemble(name) {
            "ensemble"
        } else {
            "artist"
        };
        people.push((name.clone(), role.to_string()));
    }
    for credit in release
        .credits
        .iter()
        .chain(release.tracks.iter().flat_map(|track| &track.credits))
    {
        let role = if credit.role == "performer" && looks_like_ensemble(&credit.name) {
            "ensemble".to_string()
        } else {
            credit.role.clone()
        };
        people.push((credit.name.clone(), role.clone()));
        if let Some(own) = &credit.artist_name {
            people.push((own.clone(), role));
        }
    }
    people
}

fn people_signal(
    key: &str,
    label: &str,
    tagged: &[String],
    people: &[(String, String)],
) -> Option<Signal> {
    if tagged.is_empty() {
        return None;
    }
    let found = tagged
        .iter()
        .find(|name| people.iter().any(|(person, _)| same_person(name, person)));
    if let Some(name) = found {
        return Some(signal(
            key,
            SignalOutcome::Match,
            2,
            format!("{name} is credited on the release."),
        ));
    }
    let mut others = people
        .iter()
        .filter(|(_, role)| role == key)
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    others.dedup();
    let tagged = tagged.join(", ");
    Some(if others.is_empty() {
        signal(
            key,
            SignalOutcome::Unknown,
            0,
            format!("Your tags name {tagged} as {label}; the release does not list one."),
        )
    } else {
        signal(
            key,
            SignalOutcome::Mismatch,
            -4,
            format!(
                "Your tags name {tagged} as {label}; the release credits {}.",
                others.into_iter().take(2).collect::<Vec<_>>().join(" and ")
            ),
        )
    })
}

fn lengths_signal(pairs: &[(f64, f64)]) -> Signal {
    let compared = pairs
        .iter()
        .filter(|(local, release)| *local > 0.0 && *release > 0.0)
        .collect::<Vec<_>>();
    if compared.is_empty() {
        return signal(
            "lengths",
            SignalOutcome::Unknown,
            0,
            "Track lengths could not be compared.".to_string(),
        );
    }
    let same = compared
        .iter()
        .filter(|(local, release)| {
            (local - release).abs()
                <= SAME_LENGTH_SECONDS.max(local.max(*release) * SAME_LENGTH_SHARE)
        })
        .count();
    let share = same as f64 / compared.len() as f64;
    if share >= 0.75 {
        return signal(
            "lengths",
            SignalOutcome::Match,
            3,
            format!("{same} of {} track lengths match.", compared.len()),
        );
    }
    let (local, release) = compared
        .iter()
        .copied()
        .copied()
        .max_by(|left, right| {
            (left.0 - left.1)
                .abs()
                .total_cmp(&(right.0 - right.1).abs())
        })
        .unwrap_or_default();
    let example = format!("{} here, {} on the release", clock(local), clock(release));
    if share > 0.25 {
        return signal(
            "lengths",
            SignalOutcome::Partial,
            0,
            format!(
                "Only {same} of {} track lengths match ({example}).",
                compared.len()
            ),
        );
    }
    let close = compared.iter().all(|(local, release)| {
        (local - release).abs() <= local.max(*release) * OTHER_PERFORMANCE_SHARE
    });
    if close {
        signal(
            "lengths",
            SignalOutcome::Mismatch,
            -3,
            format!("The track lengths are those of another performance ({example})."),
        )
    } else {
        signal(
            "lengths",
            SignalOutcome::Mismatch,
            -1,
            format!("The track lengths do not match ({example}); the files may be shortened."),
        )
    }
}

/// Checks a looked-up release against a local album.
pub fn check_release(local: &LocalAlbum<'_>, release: &ReleaseDetails) -> MatchCheck {
    let mut signals = Vec::new();
    let release_tracks = release.tracks.len();
    signals.push(if release_tracks == local.track_count {
        signal(
            "tracks",
            SignalOutcome::Match,
            3,
            format!("{}, like your album.", plural(release_tracks, "track")),
        )
    } else if local.track_count < release_tracks && local.paired_tracks == local.track_count {
        signal(
            "tracks",
            SignalOutcome::Partial,
            0,
            format!(
                "Your {} are {} of the release's {}.",
                plural(local.track_count, "track"),
                local.track_count,
                release_tracks
            ),
        )
    } else {
        signal(
            "tracks",
            SignalOutcome::Mismatch,
            -3,
            format!(
                "The release has {}; your album has {}.",
                plural(release_tracks, "track"),
                local.track_count
            ),
        )
    });
    signals.push(lengths_signal(&local.paired_lengths));
    if let Some(year) = local.year.filter(|year| year.len() == 4) {
        let edition = year_of(release.date.as_deref());
        let first = year_of(release.first_release_date.as_deref());
        signals.push(if edition == Some(year) || first == Some(year) {
            signal(
                "year",
                SignalOutcome::Match,
                1,
                format!("Released in {year}, as your tags say."),
            )
        } else if let Some(near) = [first, edition].into_iter().flatten().find(|other| {
            matches!(
                (other.parse::<i32>(), year.parse::<i32>()),
                (Ok(other), Ok(year)) if (other - year).abs() == 1
            )
        }) {
            // Tags often hold the recording year; the record came out the
            // year after (Blue Train: recorded 1957, released 1958).
            signal(
                "year",
                SignalOutcome::Partial,
                0,
                format!("Released in {near}, a year from the {year} in your tags."),
            )
        } else if edition.is_none() && first.is_none() {
            signal(
                "year",
                SignalOutcome::Unknown,
                0,
                "The release has no date.".to_string(),
            )
        } else {
            let mut when = format!("this release is from {}", edition.or(first).unwrap_or("?"));
            if let (Some(edition), Some(first)) = (edition, first)
                && edition != first
            {
                when = format!("this release is from {edition} (first released {first})");
            }
            signal(
                "year",
                SignalOutcome::Mismatch,
                -2,
                format!("Your tags say {year}; {when}."),
            )
        });
    }
    if let Some(label) = local.label.filter(|label| !label.trim().is_empty())
        && !release.labels.is_empty()
    {
        let names = release
            .labels
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>();
        signals.push(if names.iter().any(|name| same_person(label, name)) {
            signal(
                "label",
                SignalOutcome::Match,
                1,
                format!("On {label}, as your tags say."),
            )
        } else {
            signal(
                "label",
                SignalOutcome::Mismatch,
                -1,
                format!("Your tags say {label}; the release is on {}.", names[0]),
            )
        });
    }
    let people = release_people(release);
    signals.extend(people_signal(
        "conductor",
        "conductor",
        local.conductors,
        &people,
    ));
    signals.extend(people_signal(
        "ensemble",
        "ensemble",
        local.ensembles,
        &people,
    ));
    signals.extend(people_signal(
        "composer",
        "composer",
        local.composers,
        &people,
    ));
    let score = signals.iter().map(|signal| signal.points).sum();
    MatchCheck {
        score,
        threshold: VERIFY_THRESHOLD,
        verified: score >= VERIFY_THRESHOLD,
        signals,
    }
}

/// Whether a release that failed the check is still clearly the same
/// *album*: the same title, every local track found on it, and nothing but
/// the track lengths disagreeing (a partial or trimmed copy). Another year,
/// label, conductor, ensemble or composer means another recording, and a
/// single that shares the album's name is not the album. The edition stays
/// unconfirmed, so nothing from the release itself is used, but the release
/// group's own article (background, reception) describes this album as much
/// as any edition of it.
pub fn same_album_group(local_title: &str, release: &ReleaseDetails, check: &MatchCheck) -> bool {
    let tracks_fit = check.signals.iter().any(|signal| {
        signal.key == "tracks"
            && matches!(
                signal.outcome,
                SignalOutcome::Match | SignalOutcome::Partial
            )
    });
    let only_lengths_differ = check
        .signals
        .iter()
        .all(|signal| signal.key == "lengths" || signal.outcome != SignalOutcome::Mismatch);
    let single = release
        .release_group_type
        .as_deref()
        .is_some_and(|kind| kind.eq_ignore_ascii_case("single"));
    release.release_group_mbid.is_some()
        && !single
        && tracks_fit
        && only_lengths_differ
        && title_match(local_title, &release.title) == TitleMatch::Exact
}

/// A first ranking of search results from what the search returns (no
/// lengths or credits yet). `None` when the title or artist rules the
/// release out.
pub fn prescore(
    title: &str,
    artist: &str,
    local: &LocalAlbum<'_>,
    candidate: &ReleaseCandidate,
) -> Option<i32> {
    candidate_points(title, artist, local, candidate, true)
}

/// How likely a release a person searched for is the album, for ordering
/// and labelling "Identify album" results. Unlike [`prescore`] nothing is
/// ruled out: a different title or artist only costs points.
pub fn identify_fit(
    title: &str,
    artist: &str,
    local: &LocalAlbum<'_>,
    candidate: &ReleaseCandidate,
) -> i32 {
    candidate_points(title, artist, local, candidate, false).unwrap_or_default()
}

fn candidate_points(
    title: &str,
    artist: &str,
    local: &LocalAlbum<'_>,
    candidate: &ReleaseCandidate,
    strict: bool,
) -> Option<i32> {
    // A classical release credits its composer; with a tagged composer
    // credited, "Symphony No. 9 in D minor, op. 125 \"Choral\"" is the same
    // piece as "Symphony No. 9" (and Dvořák's ninth is not).
    let composer_credited = !local.composers.is_empty()
        && candidate.artist_names.iter().any(|name| {
            local
                .composers
                .iter()
                .any(|composer| same_person(composer, name))
        });
    let title_points = match title_match(title, &candidate.title) {
        TitleMatch::Exact => 2,
        TitleMatch::Folded => 1,
        TitleMatch::Similar => 0,
        TitleMatch::No if composer_credited && title_words_within(title, &candidate.title) => 0,
        TitleMatch::No if strict => return None,
        TitleMatch::No => -3,
    };
    let artist_matches = same_person(artist, &candidate.artist)
        || candidate
            .artist_names
            .iter()
            .any(|name| same_person(artist, name));
    if strict && (!artist_matches || candidate.score < 50) {
        return None;
    }
    let mut points = title_points;
    if !artist_matches {
        points -= 2;
    }
    if let Some(count) = candidate.track_count {
        let count = count.max(0) as usize;
        points += match count.cmp(&local.track_count) {
            std::cmp::Ordering::Equal => 3,
            std::cmp::Ordering::Greater => 0,
            std::cmp::Ordering::Less => -3,
        };
    }
    if let (Some(year), Some(date)) = (
        local.year.filter(|year| year.len() == 4),
        year_of(candidate.date.as_deref()),
    ) {
        points += if year == date { 1 } else { -1 };
    }
    if let Some(label) = local.label
        && candidate.labels.iter().any(|name| same_person(label, name))
    {
        points += 1;
    }
    // On a classical album (a conductor is tagged) the composer is part
    // of the artist credit.
    if !local.conductors.is_empty() && !local.composers.is_empty() {
        points += if composer_credited { 2 } else { -2 };
    }
    // The orchestra in the artist credit: the tagged one, or another.
    let credited_ensembles = candidate
        .artist_names
        .iter()
        .filter(|name| looks_like_ensemble(name))
        .collect::<Vec<_>>();
    if !local.ensembles.is_empty() && !credited_ensembles.is_empty() {
        let tagged = credited_ensembles.iter().any(|name| {
            local
                .ensembles
                .iter()
                .any(|ensemble| same_person(ensemble, name))
        });
        points += if tagged { 2 } else { -2 };
    }
    Some(points)
}

/// Search results that may be the album, best first: by [`prescore`], then
/// the earlier release (most often the original), then MusicBrainz' score.
pub fn rank_candidates<'a>(
    title: &str,
    artist: &str,
    local: &LocalAlbum<'_>,
    candidates: &'a [ReleaseCandidate],
) -> Vec<(i32, &'a ReleaseCandidate)> {
    let mut ranked = candidates
        .iter()
        .filter_map(|candidate| {
            prescore(title, artist, local, candidate).map(|points| (points, candidate))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|(left_points, left), (right_points, right)| {
        right_points
            .cmp(left_points)
            .then_with(|| match (&left.date, &right.date) {
                (Some(left), Some(right)) => left.cmp(right),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| right.score.cmp(&left.score))
    });
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enrichment::musicbrainz::{ExternalCredit, ReleaseTrack};

    fn track(position: i64, title: &str, seconds: f64) -> ReleaseTrack {
        ReleaseTrack {
            disc: 1,
            position,
            title: title.into(),
            recording_mbid: None,
            credits: Vec::new(),
            works: Vec::new(),
            length_ms: Some((seconds * 1000.0) as i64),
        }
    }

    fn credit(name: &str, role: &str) -> ExternalCredit {
        ExternalCredit {
            name: name.into(),
            artist_name: Some(name.into()),
            artist_mbid: None,
            role: role.into(),
            instrument: None,
        }
    }

    fn candidate(title: &str, artist: &str, tracks: i64, date: &str) -> ReleaseCandidate {
        ReleaseCandidate {
            mbid: "00000000-0000-4000-8000-000000000000".into(),
            title: title.into(),
            score: 100,
            artist: artist.into(),
            artist_names: vec![artist.into()],
            track_count: Some(tracks),
            date: Some(date.into()),
            country: None,
            disambiguation: None,
            labels: Vec::new(),
            catalog_numbers: Vec::new(),
            format: None,
            release_type: None,
        }
    }

    #[test]
    fn titles_match_across_languages_and_number_spellings() {
        assert_eq!(
            title_match("Symphonien Nr. 5 & 7", "Symphonien nos. 5 & 7"),
            TitleMatch::Folded
        );
        assert_eq!(
            title_match("Beethoven: Symphony No. 9", "Symphony No. 9"),
            TitleMatch::Exact
        );
        assert_eq!(
            title_match("Blue Train", "Blue Train (Rudy Van Gelder edition)"),
            TitleMatch::Similar
        );
        assert_eq!(title_match("Blue Train", "Giant Steps"), TitleMatch::No);
        // Another composer's ninth symphony is not a near match.
        assert_eq!(
            title_match(
                "Beethoven: Symphony No. 9",
                "Symphony no. 9 in E minor, \"From the New World\""
            ),
            TitleMatch::No
        );
        assert_eq!(
            title_tokens("Sinfonien Nrn. 5 und 7"),
            ["symphony", "no", "5", "7"]
        );
    }

    #[test]
    fn people_match_by_name_or_surname_but_not_by_a_shared_word() {
        assert!(same_person("Karajan", "Herbert von Karajan"));
        assert!(same_person(
            "Philharmonia Orchestra",
            "The Philharmonia Orchestra"
        ));
        assert!(!same_person(
            "Philharmonia Orchestra",
            "Berliner Philharmoniker"
        ));
        assert!(!same_person(
            "Wiener Philharmoniker",
            "Berliner Philharmoniker"
        ));
        assert!(looks_like_ensemble("Berliner Philharmoniker"));
        assert!(!looks_like_ensemble("Herbert von Karajan"));
    }

    /// The 1955 Philharmonia Ninth against a 1999 reissue of a Berlin
    /// recording: same conductor, same four movements, another orchestra,
    /// another year and another performance.
    #[test]
    fn a_reissue_of_another_recording_fails_the_check() {
        let release = ReleaseDetails {
            title: "Symphonie Nr. 9".into(),
            date: Some("1999-03-01".into()),
            first_release_date: Some("1977".into()),
            artists: vec![
                ("b".into(), "Berliner Philharmoniker".into()),
                ("k".into(), "Herbert von Karajan".into()),
            ],
            credits: vec![
                credit("Berliner Philharmoniker", "ensemble"),
                credit("Herbert von Karajan", "conductor"),
            ],
            tracks: vec![
                track(1, "I.", 940.0),
                track(2, "II.", 620.0),
                track(3, "III.", 1010.0),
                track(4, "IV.", 1460.0),
            ],
            ..ReleaseDetails::default()
        };
        let conductors = vec!["Herbert von Karajan".to_string()];
        let ensembles = vec!["Philharmonia Orchestra".to_string()];
        let check = check_release(
            &LocalAlbum {
                track_count: 4,
                paired_tracks: 4,
                paired_lengths: vec![
                    (1000.0, 940.0),
                    (660.0, 620.0),
                    (1080.0, 1010.0),
                    (1500.0, 1460.0),
                ],
                year: Some("1955"),
                label: None,
                conductors: &conductors,
                ensembles: &ensembles,
                composers: &[],
            },
            &release,
        );
        assert!(!check.verified, "{check:?}");
        let outcomes = check
            .signals
            .iter()
            .map(|signal| (signal.key.as_str(), signal.outcome, signal.points))
            .collect::<Vec<_>>();
        assert_eq!(
            outcomes,
            [
                ("tracks", SignalOutcome::Match, 3),
                ("lengths", SignalOutcome::Mismatch, -3),
                ("year", SignalOutcome::Mismatch, -2),
                ("conductor", SignalOutcome::Match, 2),
                ("ensemble", SignalOutcome::Mismatch, -4),
            ]
        );
        assert_eq!(check.score, -4);
        assert_eq!(
            check.signals[2].detail,
            "Your tags say 1955; this release is from 1999 (first released 1977)."
        );
        assert_eq!(
            check.signals[4].detail,
            "Your tags name Philharmonia Orchestra as ensemble; the release credits Berliner Philharmoniker."
        );
    }

    #[test]
    fn a_trimmed_partial_copy_is_the_same_album_but_another_recording_is_not() {
        let release = ReleaseDetails {
            title: "Ghosts I–IV".into(),
            release_group_mbid: Some("group".into()),
            release_group_type: Some("Album".into()),
            first_release_date: Some("2008-03-02".into()),
            tracks: (1..=36)
                .map(|position| track(position, &format!("{position} Ghosts I"), 180.0))
                .collect(),
            ..ReleaseDetails::default()
        };
        // Five 48-second clips of the album's first five tracks.
        let local = LocalAlbum {
            track_count: 5,
            paired_tracks: 5,
            paired_lengths: (0..5).map(|_| (48.0, 180.0)).collect(),
            year: Some("2008"),
            ..LocalAlbum::default()
        };
        let check = check_release(&local, &release);
        assert!(
            !check.verified,
            "a trimmed copy is not this edition: {check:?}"
        );
        assert!(same_album_group("Ghosts I-IV", &release, &check));
        // Another title, a single of the same name, or no release group.
        assert!(!same_album_group("Ghosts V: Together", &release, &check));
        let single = ReleaseDetails {
            release_group_type: Some("Single".into()),
            ..release.clone()
        };
        assert!(!same_album_group("Ghosts I-IV", &single, &check));
        let no_group = ReleaseDetails {
            release_group_mbid: None,
            ..release.clone()
        };
        assert!(!same_album_group("Ghosts I-IV", &no_group, &check));
        // Tracks the release does not have.
        let extra = check_release(
            &LocalAlbum {
                paired_tracks: 4,
                ..local.clone()
            },
            &release,
        );
        assert!(!same_album_group("Ghosts I-IV", &release, &extra));
        // Another year: another recording, whatever the title.
        let other_year = check_release(
            &LocalAlbum {
                year: Some("1990"),
                ..local.clone()
            },
            &release,
        );
        assert!(!same_album_group("Ghosts I-IV", &release, &other_year));
    }

    /// Another composer's work with the same title and conductor.
    #[test]
    fn a_release_of_another_composers_work_fails_the_check() {
        let release = ReleaseDetails {
            title: "Symphony no. 9".into(),
            artists: vec![
                ("d".into(), "Antonín Dvořák".into()),
                ("k".into(), "Herbert von Karajan".into()),
            ],
            credits: vec![credit("Antonín Dvořák", "composer")],
            tracks: (1..=4)
                .map(|position| track(position, "x", 600.0))
                .collect(),
            ..ReleaseDetails::default()
        };
        let conductors = vec!["Herbert von Karajan".to_string()];
        let composers = vec!["Ludwig van Beethoven".to_string()];
        let check = check_release(
            &LocalAlbum {
                track_count: 4,
                paired_tracks: 4,
                conductors: &conductors,
                composers: &composers,
                ..LocalAlbum::default()
            },
            &release,
        );
        assert!(!check.verified, "{check:?}");
        let composer = check.signals.last().unwrap();
        assert_eq!(composer.outcome, SignalOutcome::Mismatch);
        assert_eq!(
            composer.detail,
            "Your tags name Ludwig van Beethoven as composer; the release credits Antonín Dvořák."
        );
        // "Beethoven" in the artist credit is Ludwig van Beethoven.
        let beethoven = ReleaseDetails {
            artists: vec![("b".into(), "Beethoven".into())],
            credits: Vec::new(),
            ..release
        };
        let check = check_release(
            &LocalAlbum {
                track_count: 4,
                paired_tracks: 4,
                composers: &composers,
                ..LocalAlbum::default()
            },
            &beethoven,
        );
        assert_eq!(check.signals.last().unwrap().outcome, SignalOutcome::Match);
    }

    /// Blue Train's five tracks against the two-track 7-inch single.
    #[test]
    fn a_single_with_the_albums_title_fails_the_check() {
        let release = ReleaseDetails {
            title: "Blue Train".into(),
            date: Some("1957".into()),
            release_group_type: Some("Single".into()),
            tracks: vec![
                track(1, "Blue Train (part 1)", 300.0),
                track(2, "Blue Train (part 2)", 300.0),
            ],
            ..ReleaseDetails::default()
        };
        let check = check_release(
            &LocalAlbum {
                track_count: 5,
                paired_tracks: 0,
                year: Some("1957"),
                ..LocalAlbum::default()
            },
            &release,
        );
        assert!(!check.verified);
        assert_eq!(
            check.signals[0].detail,
            "The release has 2 tracks; your album has 5."
        );
    }

    #[test]
    fn a_record_released_the_year_after_its_recording_still_passes() {
        let release = ReleaseDetails {
            title: "Blue Train".into(),
            date: Some("1958".into()),
            first_release_date: Some("1958".into()),
            labels: vec![("Blue Note".into(), Some("BLP 1577".into()))],
            tracks: (1..=5)
                .map(|position| track(position, "x", 600.0))
                .collect(),
            ..ReleaseDetails::default()
        };
        let check = check_release(
            &LocalAlbum {
                track_count: 5,
                paired_tracks: 5,
                paired_lengths: vec![(2.0, 600.0); 5],
                year: Some("1957"),
                label: Some("Blue Note"),
                ..LocalAlbum::default()
            },
            &release,
        );
        assert!(check.verified, "{check:?}");
        assert_eq!(check.signals[2].outcome, SignalOutcome::Partial);
        assert_eq!(
            check.signals[2].detail,
            "Released in 1958, a year from the 1957 in your tags."
        );
    }

    #[test]
    fn the_right_release_passes_even_with_shortened_files() {
        let release = ReleaseDetails {
            title: "Kind of Blue".into(),
            date: Some("1959-08-17".into()),
            labels: vec![("Columbia".into(), Some("CL 1355".into()))],
            tracks: (1..=5)
                .map(|position| track(position, "x", 540.0))
                .collect(),
            ..ReleaseDetails::default()
        };
        let lengths = vec![(4.0, 540.0); 5];
        let shortened = check_release(
            &LocalAlbum {
                track_count: 5,
                paired_tracks: 5,
                paired_lengths: lengths,
                year: Some("1959"),
                label: Some("Columbia"),
                ..LocalAlbum::default()
            },
            &release,
        );
        assert!(shortened.verified, "{shortened:?}");
        assert_eq!(shortened.signals[1].points, -1);
        let full = check_release(
            &LocalAlbum {
                track_count: 5,
                paired_tracks: 5,
                paired_lengths: vec![(541.0, 540.0); 5],
                ..LocalAlbum::default()
            },
            &release,
        );
        assert_eq!(full.score, 6);
        assert_eq!(full.signals[1].detail, "5 of 5 track lengths match.");
    }

    #[test]
    fn the_album_is_preferred_over_a_single_and_the_tagged_orchestra_wins() {
        let candidates = vec![
            candidate("Blue Train", "John Coltrane", 2, "1957"),
            candidate("Blue Train", "John Coltrane", 5, "1957-09"),
            candidate("Giant Steps", "John Coltrane", 7, "1960"),
        ];
        let local = |track_count| LocalAlbum {
            track_count,
            year: Some("1957"),
            ..LocalAlbum::default()
        };
        let ranked = rank_candidates("Blue Train", "John Coltrane", &local(5), &candidates);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].1.track_count, Some(5));
        // Four of the five tracks: the full album still beats the single.
        let ranked = rank_candidates("Blue Train", "John Coltrane", &local(4), &candidates);
        assert_eq!(ranked[0].1.track_count, Some(5));

        let conducted = |orchestra: &str, date: &str| ReleaseCandidate {
            artist: format!("Beethoven; {orchestra}, Herbert von Karajan"),
            artist_names: vec![
                "Beethoven".into(),
                orchestra.into(),
                "Herbert von Karajan".into(),
            ],
            ..candidate("Symphony no. 9", "", 4, date)
        };
        let candidates = vec![
            conducted("Berliner Philharmoniker", "1999-07-01"),
            conducted("Philharmonia Orchestra", "2001"),
        ];
        let ensembles = vec!["Philharmonia Orchestra".to_string()];
        let ranked = rank_candidates(
            "Beethoven: Symphony No. 9",
            "Herbert von Karajan",
            &LocalAlbum {
                track_count: 4,
                year: Some("1955"),
                ensembles: &ensembles,
                ..LocalAlbum::default()
            },
            &candidates,
        );
        assert_eq!(ranked[0].1.date.as_deref(), Some("2001"));

        // With the composer tagged, a title that adds key and opus is the
        // same piece; another composer's ninth is ruled out.
        let composers = vec!["Ludwig van Beethoven".to_string()];
        let conductors = vec!["Herbert von Karajan".to_string()];
        let by = |composer: &str, title: &str| ReleaseCandidate {
            artist: format!("{composer}; Herbert von Karajan"),
            artist_names: vec![composer.into(), "Herbert von Karajan".into()],
            ..candidate(title, "", 4, "2001")
        };
        let candidates = vec![
            by(
                "Antonín Dvořák",
                "Symphony no. 9 in E minor, \"From the New World\"",
            ),
            by(
                "Ludwig van Beethoven",
                "Symphony no. 9 in D minor, op. 125 \"Choral\"",
            ),
        ];
        let ranked = rank_candidates(
            "Beethoven: Symphony No. 9",
            "Herbert von Karajan",
            &LocalAlbum {
                track_count: 4,
                conductors: &conductors,
                composers: &composers,
                ..LocalAlbum::default()
            },
            &candidates,
        );
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].1.artist_names[0], "Ludwig van Beethoven");

        let kleiber = vec![ReleaseCandidate {
            artist: "Beethoven; Wiener Philharmoniker, Carlos Kleiber".into(),
            artist_names: vec![
                "Beethoven".into(),
                "Wiener Philharmoniker".into(),
                "Carlos Kleiber".into(),
            ],
            ..candidate("Symphonien nos. 5 & 7", "", 8, "1995-02-20")
        }];
        let ranked = rank_candidates(
            "Symphonien Nr. 5 & 7",
            "Carlos Kleiber",
            &local(8),
            &kleiber,
        );
        assert_eq!(ranked.len(), 1);
    }
}
