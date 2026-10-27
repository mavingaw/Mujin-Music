//! Matching the files FETCH delivered to the album they should replace:
//! MusicBrainz ids first, else normalised album artist and album title,
//! then each track by recording id or by disc, track number and a length
//! within two seconds.

use std::collections::HashMap;

use crate::library::identity::{normalize_album_identity, normalize_artist_identity};
use crate::library::tools::versions::split_edition;

/// Seconds two copies of one track may differ by.
pub const DURATION_TOLERANCE: f64 = 2.0;
/// Share of the old album's tracks the new copy must cover.
pub const MIN_COVERAGE: f64 = 0.6;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackFact {
    pub path: String,
    pub track_id: String,
    pub album_id: String,
    pub album_title: String,
    pub album_artist: String,
    pub release_mbid: Option<String>,
    pub recording_mbid: Option<String>,
    pub disc: u32,
    pub number: u32,
    pub duration: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MatchedTrack {
    pub old_track_id: String,
    pub old_path: String,
    pub new_track_id: String,
    pub new_path: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MatchOutcome {
    /// The album the new files were indexed as (may be the old album's
    /// id when the copies merged into the same tracks).
    pub album_id: String,
    pub by_release_id: bool,
    pub tracks: Vec<MatchedTrack>,
    /// Matched share of the old album's tracks.
    pub coverage: f64,
}

fn mbid(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| uuid::Uuid::parse_str(value).is_ok())
}

fn album_key(fact: &TrackFact) -> (String, String) {
    (
        normalize_artist_identity(&fact.album_artist),
        normalize_album_identity(&split_edition(&fact.album_title).0),
    )
}

fn disc(value: u32) -> u32 {
    value.max(1)
}

/// Pairs `old` (the album's current tracks) with `new` (files found where
/// FETCH saved the album). `None` when they are not the same album or too
/// few tracks match.
pub fn match_album(old: &[TrackFact], new: &[TrackFact]) -> Option<MatchOutcome> {
    if old.is_empty() || new.is_empty() {
        return None;
    }
    let old_release = old.iter().find_map(|fact| mbid(&fact.release_mbid));
    let old_key = album_key(&old[0]);
    // Group the new files by the album they were indexed as.
    let mut by_album: HashMap<&str, Vec<&TrackFact>> = HashMap::new();
    for fact in new {
        by_album
            .entry(fact.album_id.as_str())
            .or_default()
            .push(fact);
    }
    let mut best: Option<MatchOutcome> = None;
    for (album_id, files) in by_album {
        let by_release_id = old_release.as_ref().is_some_and(|release| {
            files
                .iter()
                .any(|f| mbid(&f.release_mbid).as_ref() == Some(release))
        });
        let same_names =
            files.iter().any(|fact| album_key(fact) == old_key) && !old_key.1.is_empty();
        if !by_release_id && !same_names {
            continue;
        }
        let mut used = vec![false; files.len()];
        let mut tracks = Vec::new();
        for old_track in old {
            let recording = mbid(&old_track.recording_mbid);
            let position = files
                .iter()
                .enumerate()
                .position(|(index, candidate)| {
                    !used[index]
                        && recording.is_some()
                        && mbid(&candidate.recording_mbid) == recording
                })
                .or_else(|| {
                    files.iter().enumerate().position(|(index, candidate)| {
                        !used[index]
                            && old_track.number > 0
                            && candidate.number == old_track.number
                            && disc(candidate.disc) == disc(old_track.disc)
                            && (candidate.duration - old_track.duration).abs() <= DURATION_TOLERANCE
                    })
                });
            if let Some(index) = position {
                used[index] = true;
                tracks.push(MatchedTrack {
                    old_track_id: old_track.track_id.clone(),
                    old_path: old_track.path.clone(),
                    new_track_id: files[index].track_id.clone(),
                    new_path: files[index].path.clone(),
                });
            }
        }
        let coverage = tracks.len() as f64 / old.len() as f64;
        if tracks.is_empty() || coverage < MIN_COVERAGE {
            continue;
        }
        let outcome = MatchOutcome {
            album_id: album_id.to_string(),
            by_release_id,
            tracks,
            coverage,
        };
        if best
            .as_ref()
            .is_none_or(|current| outcome.tracks.len() > current.tracks.len())
        {
            best = Some(outcome);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(path: &str, album: &str, title: &str, number: u32, duration: f64) -> TrackFact {
        TrackFact {
            path: path.into(),
            track_id: format!("t-{path}"),
            album_id: album.into(),
            album_title: title.into(),
            album_artist: "Rise Against".into(),
            disc: 1,
            number,
            duration,
            ..Default::default()
        }
    }

    fn old() -> Vec<TrackFact> {
        (1..=4)
            .map(|n| {
                fact(
                    &format!("/m/mp3/{n}.mp3"),
                    "old",
                    "Endgame",
                    n,
                    200.0 + f64::from(n),
                )
            })
            .collect()
    }

    #[test]
    fn same_names_and_track_numbers_within_two_seconds_match() {
        let new: Vec<TrackFact> = (1..=4)
            .map(|n| {
                fact(
                    &format!("/m/flac/{n}.flac"),
                    "new",
                    "Endgame (Deluxe Edition)",
                    n,
                    201.5 + f64::from(n),
                )
            })
            .collect();
        let outcome = match_album(&old(), &new).expect("matches");
        assert_eq!(outcome.album_id, "new");
        assert_eq!(outcome.tracks.len(), 4);
        assert!(!outcome.by_release_id);
        assert_eq!(outcome.tracks[0].new_path, "/m/flac/1.flac");
    }

    #[test]
    fn lengths_off_by_more_than_two_seconds_or_other_albums_do_not_match() {
        let longer: Vec<TrackFact> = (1..=4)
            .map(|n| fact(&format!("/m/flac/{n}.flac"), "new", "Endgame", n, 230.0))
            .collect();
        assert!(match_album(&old(), &longer).is_none());
        let other: Vec<TrackFact> = (1..=4)
            .map(|n| {
                fact(
                    &format!("/m/flac/{n}.flac"),
                    "new",
                    "Siren Song",
                    n,
                    200.0 + f64::from(n),
                )
            })
            .collect();
        assert!(match_album(&old(), &other).is_none());
        // Too few tracks: 1 of 4.
        let partial = vec![fact("/m/flac/1.flac", "new", "Endgame", 1, 201.0)];
        assert!(match_album(&old(), &partial).is_none());
    }

    #[test]
    fn musicbrainz_ids_win_over_names() {
        let release = "6f1d0c3d-3f0e-4c37-9c8a-6c3bd1b1d1a7".to_string();
        let mut old = old();
        for (index, track) in old.iter_mut().enumerate() {
            track.release_mbid = Some(release.clone());
            track.recording_mbid = Some(format!("00000000-0000-4000-8000-00000000000{index}"));
        }
        // Retitled and renumbered, but the ids agree.
        let new: Vec<TrackFact> = old
            .iter()
            .enumerate()
            .map(|(index, track)| TrackFact {
                path: format!("/m/flac/{index}.flac"),
                track_id: format!("n{index}"),
                album_id: "new".into(),
                album_title: "エンドゲーム".into(),
                album_artist: "ライズ・アゲインスト".into(),
                release_mbid: Some(release.to_uppercase()),
                recording_mbid: track.recording_mbid.clone(),
                disc: 1,
                number: 10 + index as u32,
                duration: 300.0,
            })
            .collect();
        let outcome = match_album(&old, &new).expect("ids match");
        assert!(outcome.by_release_id);
        assert_eq!(outcome.coverage, 1.0);
    }
}
