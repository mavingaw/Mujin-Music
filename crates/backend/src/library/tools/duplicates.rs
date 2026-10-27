//! Duplicate detection: the same recording held as more than one track.
//!
//! Candidates come from tags (title and artist, ignoring remaster suffixes)
//! with matching lengths, from shared ISRCs, and from matching acoustic
//! fingerprints even when the tags disagree. When both files of a tag
//! match have fingerprints, the fingerprint has the last word: a live take
//! and the studio take with the same title and length are kept apart.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::OnceLock;

use regex::Regex;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::analysis::Fingerprint;
use super::quality::Quality;
use crate::library::identity::{normalize_artist_identity, normalize_song_identity};

/// What the detector knows about one track in the catalog.
#[derive(Clone, Debug)]
pub struct TrackFacts {
    pub song_id: String,
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album_id: String,
    pub album_title: String,
    /// Tracks on the album this track sits on.
    pub album_track_count: usize,
    pub duration: f64,
    pub isrc: Option<String>,
    pub quality: Quality,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateKind {
    /// Copies on albums with the same title (often a rip in two formats).
    SameAlbum,
    /// At least one copy is a loose track outside a real album.
    Loose,
    /// The recording appears on different albums (e.g. a compilation).
    CrossAlbum,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Fingerprints or ISRCs agree.
    High,
    /// Tags and lengths agree.
    Medium,
}

#[derive(Clone, Debug, Serialize)]
pub struct DuplicateEvidence {
    pub tags: bool,
    pub isrc: bool,
    /// Lowest fingerprint similarity with the kept copy, when measured.
    pub fingerprint_similarity: Option<f32>,
    /// Largest length difference in the group, in seconds.
    pub duration_spread: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct DuplicateGroup {
    /// Hash of the member paths; changes when a copy is added or removed.
    pub key: String,
    pub kind: DuplicateKind,
    pub confidence: Confidence,
    /// Song ids, best copy first.
    pub members: Vec<String>,
    /// The copy "prefer highest quality" keeps.
    pub best: String,
    /// Whether keeping only the best copy is safe to do in bulk.
    pub suggested: bool,
    pub evidence: DuplicateEvidence,
}

static VERSION_SUFFIX: OnceLock<Regex> = OnceLock::new();

/// Title without suffixes that do not change the recording, such as
/// "(2011 Remaster)", "- Remastered" or "[Album Version]". Live, acoustic,
/// demo, mono and edit suffixes stay: those are different recordings.
pub fn recording_title(title: &str) -> String {
    let pattern = VERSION_SUFFIX.get_or_init(|| {
        Regex::new(
            r"(?ix)
            \s*(?:
                [\(\[]\s*[^\)\]]*?\b(?:remaster(?:ed)?|album\s+version|lp\s+version|explicit|hi[-\s]?res|24[-\s]?bit)\b[^\)\]]*[\)\]]
              | \s-\s[^-]*?\b(?:remaster(?:ed)?|album\s+version|lp\s+version)\b.*
            )\s*$",
        )
        .expect("version suffix pattern")
    });
    let mut current = title.trim().to_string();
    loop {
        let next = pattern.replace(&current, "").trim().to_string();
        if next == current || next.is_empty() {
            break;
        }
        current = next;
    }
    normalize_song_identity(&current)
}

struct UnionFind(Vec<usize>);

impl UnionFind {
    fn new(size: usize) -> Self {
        Self((0..size).collect())
    }
    fn find(&mut self, index: usize) -> usize {
        let mut root = index;
        while self.0[root] != root {
            root = self.0[root];
        }
        let mut current = index;
        while self.0[current] != root {
            let next = self.0[current];
            self.0[current] = root;
            current = next;
        }
        root
    }
    fn union(&mut self, left: usize, right: usize) {
        let (left, right) = (self.find(left), self.find(right));
        if left != right {
            self.0[left.max(right)] = left.min(right);
        }
    }
}

fn duration_close(left: f64, right: f64) -> bool {
    left > 0.0 && right > 0.0 && (left - right).abs() <= (left.max(right) * 0.01).max(2.0)
}

/// Stable key for a set of paths.
pub fn group_key(paths: &[&str]) -> String {
    let mut sorted: Vec<&str> = paths.to_vec();
    sorted.sort_unstable();
    let mut hasher = Sha256::new();
    for path in sorted {
        hasher.update(path.as_bytes());
        hasher.update([0]);
    }
    hasher
        .finalize()
        .iter()
        .take(12)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_loose(track: &TrackFacts) -> bool {
    track.album_track_count <= 1
}

/// Finds duplicate groups. `fingerprints` is keyed by path.
///
/// `album_clusters` maps an album to its album-versions group: copies on
/// versions of one album (a remaster and the original) are the versions
/// page's business, so a group keeps only the best copy of each cluster and
/// is dropped when one cluster is all that remains.
pub fn find_duplicates(
    tracks: &[TrackFacts],
    fingerprints: &HashMap<String, Fingerprint>,
    album_clusters: &HashMap<String, String>,
) -> Vec<DuplicateGroup> {
    let mut sets = UnionFind::new(tracks.len());
    let mut tag_linked = vec![false; tracks.len()];
    let mut isrc_linked = vec![false; tracks.len()];

    // Tags: same recording title and artist, close lengths.
    let mut by_tags: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (index, track) in tracks.iter().enumerate() {
        let title = recording_title(&track.title);
        if title.is_empty() {
            continue;
        }
        by_tags
            .entry((title, normalize_artist_identity(&track.artist)))
            .or_default()
            .push(index);
    }
    for indices in by_tags.values_mut() {
        if indices.len() < 2 {
            continue;
        }
        indices.sort_by(|a, b| tracks[*a].duration.total_cmp(&tracks[*b].duration));
        for pair in indices.windows(2) {
            if duration_close(tracks[pair[0]].duration, tracks[pair[1]].duration) {
                sets.union(pair[0], pair[1]);
                tag_linked[pair[0]] = true;
                tag_linked[pair[1]] = true;
            }
        }
    }

    // ISRC: one recording code, whatever the tags say.
    let mut by_isrc: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, track) in tracks.iter().enumerate() {
        if let Some(isrc) = track
            .isrc
            .as_deref()
            .map(|isrc| isrc.trim().to_ascii_uppercase())
            .filter(|isrc| isrc.len() >= 12)
        {
            by_isrc.entry(isrc).or_default().push(index);
        }
    }
    for indices in by_isrc.values() {
        for pair in indices.windows(2) {
            if duration_close(tracks[pair[0]].duration, tracks[pair[1]].duration) {
                sets.union(pair[0], pair[1]);
                isrc_linked[pair[0]] = true;
                isrc_linked[pair[1]] = true;
            }
        }
    }

    // Fingerprints: same audio under different tags.
    let mut printed: Vec<(usize, &Fingerprint)> = tracks
        .iter()
        .enumerate()
        .filter_map(|(index, track)| fingerprints.get(&track.path).map(|print| (index, print)))
        .filter(|(_, print)| print.is_informative())
        .collect();
    printed.sort_by(|a, b| tracks[a.0].duration.total_cmp(&tracks[b.0].duration));
    let mut acoustic_pairs: HashMap<(usize, usize), f32> = HashMap::new();
    for (position, (left, left_print)) in printed.iter().enumerate() {
        for (right, right_print) in printed[position + 1..].iter() {
            if tracks[*right].duration - tracks[*left].duration > 2.0 {
                break;
            }
            if tracks[*left].path == tracks[*right].path {
                continue;
            }
            let Some(similarity) = left_print.similarity(right_print) else {
                continue;
            };
            acoustic_pairs.insert(((*left).min(*right), (*left).max(*right)), similarity);
            if similarity >= left_print.kind().match_threshold() {
                sets.union(*left, *right);
            }
        }
    }
    let pair_similarity = |a: usize, b: usize| -> Option<f32> {
        if let Some(value) = acoustic_pairs.get(&(a.min(b), a.max(b))) {
            return Some(*value);
        }
        let left = fingerprints
            .get(&tracks[a].path)
            .filter(|print| print.is_informative())?;
        let right = fingerprints
            .get(&tracks[b].path)
            .filter(|print| print.is_informative())?;
        left.similarity(right)
    };

    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for index in 0..tracks.len() {
        groups.entry(sets.find(index)).or_default().push(index);
    }

    let mut result = Vec::new();
    for mut members in groups.into_values() {
        if members.len() < 2 {
            continue;
        }
        members.sort_by(|a, b| {
            tracks[*b]
                .quality
                .score
                .cmp(&tracks[*a].quality.score)
                // Prefer the copy that sits on a real album.
                .then(
                    tracks[*b]
                        .album_track_count
                        .cmp(&tracks[*a].album_track_count),
                )
                .then(tracks[*a].path.cmp(&tracks[*b].path))
        });
        let mut clusters_seen = HashSet::new();
        members.retain(|member| {
            let album = &tracks[*member].album_id;
            clusters_seen.insert(album_clusters.get(album).unwrap_or(album).clone())
        });
        if members.len() < 2 {
            continue;
        }
        let best = members[0];
        // A measured fingerprint mismatch with the kept copy wins over tags.
        let mut min_similarity: Option<f32> = None;
        members.retain(|member| {
            if *member == best {
                return true;
            }
            match pair_similarity(best, *member) {
                Some(similarity) => {
                    let kind = fingerprints
                        .get(&tracks[best].path)
                        .map(Fingerprint::kind)
                        .expect("a similarity needs a fingerprint");
                    if similarity < kind.mismatch_threshold() {
                        return false;
                    }
                    min_similarity = Some(min_similarity.map_or(similarity, |m| m.min(similarity)));
                    true
                }
                None => true,
            }
        });
        if members.len() < 2 {
            continue;
        }
        let paths: Vec<&str> = members.iter().map(|m| tracks[*m].path.as_str()).collect();
        let durations: Vec<f64> = members.iter().map(|m| tracks[*m].duration).collect();
        let spread = durations.iter().cloned().fold(f64::MIN, f64::max)
            - durations.iter().cloned().fold(f64::MAX, f64::min);
        let isrc = members.iter().all(|m| isrc_linked[*m]);
        let fingerprint_match = min_similarity.is_some_and(|similarity| {
            fingerprints
                .get(&tracks[best].path)
                .is_some_and(|print| similarity >= print.kind().match_threshold())
        });
        let confidence = if isrc || fingerprint_match {
            Confidence::High
        } else {
            Confidence::Medium
        };
        let album_titles: Vec<String> = members
            .iter()
            .map(|m| normalize_song_identity(&tracks[*m].album_title))
            .collect();
        let kind = if members.iter().any(|m| is_loose(&tracks[*m])) {
            DuplicateKind::Loose
        } else if album_titles.iter().all(|title| *title == album_titles[0]) {
            DuplicateKind::SameAlbum
        } else {
            DuplicateKind::CrossAlbum
        };
        // Bulk "keep the best copy" is offered only where it cannot break an
        // album: copies of one album, or loose tracks that copy an album track.
        let suggested = match kind {
            DuplicateKind::SameAlbum => true,
            DuplicateKind::Loose => {
                confidence == Confidence::High || members.iter().any(|m| tag_linked[*m])
            }
            DuplicateKind::CrossAlbum => false,
        };
        result.push(DuplicateGroup {
            key: group_key(&paths),
            kind,
            confidence,
            best: tracks[best].song_id.clone(),
            members: members.iter().map(|m| tracks[*m].song_id.clone()).collect(),
            suggested,
            evidence: DuplicateEvidence {
                tags: members.iter().all(|m| tag_linked[*m]),
                isrc,
                fingerprint_similarity: min_similarity,
                duration_spread: (spread * 10.0).round() / 10.0,
            },
        });
    }
    result.sort_by(|a, b| {
        b.suggested
            .cmp(&a.suggested)
            .then_with(|| a.members[0].cmp(&b.members[0]))
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::AudioProperties;
    use crate::library::tools::quality::grade;

    fn track(
        id: &str,
        title: &str,
        album: &str,
        count: usize,
        duration: f64,
        codec: &str,
    ) -> TrackFacts {
        let lossless = codec == "flac";
        TrackFacts {
            song_id: id.into(),
            path: format!("/music/{id}.{codec}"),
            title: title.into(),
            artist: "Example Artist".into(),
            album_id: album.into(),
            album_title: album.into(),
            album_track_count: count,
            duration,
            isrc: None,
            quality: grade(
                Some(&AudioProperties {
                    codec: Some(codec.into()),
                    sample_rate: Some(if lossless { 96_000 } else { 44_100 }),
                    bit_depth: lossless.then_some(24),
                    bitrate_kbps: Some(if lossless { 3000 } else { 320 }),
                    lossless: Some(lossless),
                    ..Default::default()
                }),
                codec,
            ),
        }
    }

    #[test]
    fn remaster_suffixes_are_ignored_but_live_takes_are_not() {
        assert_eq!(recording_title("Help! (2009 Remaster)"), "help");
        assert_eq!(recording_title("Help! - Remastered 2009"), "help");
        assert_eq!(recording_title("Help! [Album Version]"), "help");
        assert_ne!(recording_title("Help! (Live)"), "help");
    }

    #[test]
    fn a_loose_mp3_copy_of_an_album_track_is_grouped_with_the_flac_kept() {
        let tracks = vec![
            track("flac", "Opening Signal", "Fixture Album", 10, 241.0, "flac"),
            track(
                "mp3",
                "Opening Signal (2011 Remaster)",
                "Singles",
                1,
                240.2,
                "mp3",
            ),
            track(
                "other",
                "Closing Signal",
                "Fixture Album",
                10,
                241.0,
                "flac",
            ),
        ];
        let groups = find_duplicates(&tracks, &HashMap::new(), &HashMap::new());
        assert_eq!(groups.len(), 1);
        let group = &groups[0];
        assert_eq!(group.best, "flac");
        assert_eq!(group.members, vec!["flac", "mp3"]);
        assert_eq!(group.kind, DuplicateKind::Loose);
        assert_eq!(group.confidence, Confidence::Medium);
        assert!(group.suggested);
    }

    #[test]
    fn compilation_appearances_are_only_offered_for_review() {
        let tracks = vec![
            track(
                "studio",
                "Opening Signal",
                "Fixture Album",
                10,
                241.0,
                "flac",
            ),
            track("hits", "Opening Signal", "Greatest Hits", 18, 241.4, "mp3"),
        ];
        let groups = find_duplicates(&tracks, &HashMap::new(), &HashMap::new());
        assert_eq!(groups[0].kind, DuplicateKind::CrossAlbum);
        assert!(!groups[0].suggested);
    }

    #[test]
    fn fingerprints_join_mistagged_copies_and_split_different_takes() {
        let base: Vec<u32> = (0..300u32).map(|i| i.wrapping_mul(2_654_435_761)).collect();
        let other: Vec<u32> = (0..300u32)
            .map(|i| i.wrapping_mul(40_503).rotate_left(i % 31) ^ 0x5555_5555)
            .collect();
        let tracks = vec![
            track("a", "Opening Signal", "Fixture Album", 10, 241.0, "flac"),
            // Untagged copy of the same audio.
            track("b", "Track 01", "Unknown Album", 1, 241.5, "mp3"),
            // Same title and length, but a different performance.
            track("c", "Opening Signal", "Live Album", 12, 241.8, "mp3"),
        ];
        let mut prints = HashMap::new();
        prints.insert(
            tracks[0].path.clone(),
            Fingerprint::Chromaprint(base.clone()),
        );
        prints.insert(tracks[1].path.clone(), Fingerprint::Chromaprint(base));
        prints.insert(tracks[2].path.clone(), Fingerprint::Chromaprint(other));
        let groups = find_duplicates(&tracks, &prints, &HashMap::new());
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members, vec!["a", "b"]);
        assert_eq!(groups[0].confidence, Confidence::High);
    }

    #[test]
    fn copies_on_versions_of_one_album_are_left_to_the_versions_page() {
        let tracks = vec![
            track(
                "remaster",
                "Opening Signal",
                "Album (Remaster)",
                10,
                241.0,
                "flac",
            ),
            track("original", "Opening Signal", "Album", 10, 241.2, "flac"),
            track("single", "Opening Signal", "Single", 1, 241.4, "mp3"),
        ];
        let clusters = HashMap::from([
            ("Album (Remaster)".to_string(), "versions-1".to_string()),
            ("Album".to_string(), "versions-1".to_string()),
        ]);
        let groups = find_duplicates(&tracks, &HashMap::new(), &clusters);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members.len(), 2);
        assert_eq!(groups[0].kind, DuplicateKind::Loose);
        // Without the loose single, nothing is left to report.
        let groups = find_duplicates(&tracks[..2], &HashMap::new(), &clusters);
        assert!(groups.is_empty());
    }

    #[test]
    fn group_keys_ignore_order_and_change_with_membership() {
        assert_eq!(group_key(&["/a", "/b"]), group_key(&["/b", "/a"]));
        assert_ne!(group_key(&["/a", "/b"]), group_key(&["/a", "/b", "/c"]));
    }
}
