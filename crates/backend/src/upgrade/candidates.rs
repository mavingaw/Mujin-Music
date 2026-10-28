//! Upgrade candidates: albums whose best copy is lossy, low bitrate, a
//! fake lossless file, or (optionally) only CD quality.

use std::collections::HashMap;

use serde::Serialize;

use crate::domain::{Album, Artist};
use crate::library::identity::{normalize_album_identity, normalize_artist_identity};
use crate::library::tools::layers::{album_quality_with, song_quality_with, year_of};
use crate::library::tools::marks::{FileMark, FileMarks};
use crate::library::tools::quality::{Quality, QualityTier};
use crate::library::tools::spectral::{format_khz, likely_source_kbps};
use crate::library::tools::versions::split_edition;

/// Lossy copies below this bitrate get the "Low bitrate" badge.
pub const LOW_BITRATE_KBPS: u32 = 192;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonKind {
    /// MP3, AAC, Vorbis, Opus and the like.
    Lossy,
    LowBitrate,
    /// A lossless container made from a lossy copy.
    FakeLossless,
    /// Only AI-enhanced copies: plausible, not the master.
    AiEnhancedOnly,
    /// Genuine CD-quality lossless, listed when hi-res candidates are on.
    CdQuality,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Reason {
    pub kind: ReasonKind,
    /// Short badge text, e.g. `MP3 128`, `Fake lossless · 16 kHz`.
    pub label: String,
    /// One sentence for the tooltip.
    pub detail: String,
    /// Tracks this applies to.
    pub tracks: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Candidate {
    pub album_id: String,
    pub title: String,
    pub artist_id: String,
    pub artist_name: String,
    pub year: String,
    pub cover_url: String,
    pub musicbrainz_id: String,
    pub track_count: usize,
    pub quality: Quality,
    pub reasons: Vec<Reason>,
}

fn group_key(artist: &Artist, album: &Album) -> (String, String) {
    (
        normalize_artist_identity(&artist.name),
        normalize_album_identity(&split_edition(&album.name).0),
    )
}

fn genuinely_lossless(tier: QualityTier) -> bool {
    matches!(
        tier,
        QualityTier::Dsd | QualityTier::HiRes | QualityTier::Lossless
    )
}

/// Every album that would be better as a real lossless copy, worst first.
/// Albums with a genuine lossless version elsewhere in the library (a
/// remaster, a deluxe edition) are left out.
pub fn find_candidates(
    library: &[Artist],
    marks: &FileMarks,
    include_cd_quality: bool,
) -> Vec<Candidate> {
    // The best tier any version of an album has, by artist and base title.
    let mut best_version: HashMap<(String, String), u64> = HashMap::new();
    for artist in library {
        for album in &artist.albums {
            let (quality, _) = album_quality_with(album, marks);
            let entry = best_version.entry(group_key(artist, album)).or_default();
            *entry = (*entry).max(quality.tier.rank());
        }
    }
    let mut candidates = Vec::new();
    for artist in library {
        for album in &artist.albums {
            if album.songs.is_empty() || album.id.starts_with("ua-") {
                continue;
            }
            let mut lossy = 0usize;
            let mut low: Option<(usize, u32, String)> = None;
            let mut fake: Option<(usize, Option<u32>, f32)> = None;
            let mut enhanced = 0usize;
            let mut cd = 0usize;
            let mut lossy_label = String::new();
            for song in &album.songs {
                let quality = song_quality_with(song, marks);
                match (quality.tier, marks.get(&song.path)) {
                    (
                        _,
                        Some(FileMark::FakeLossless {
                            cutoff_hz,
                            confidence,
                        }),
                    ) => {
                        let entry = fake.get_or_insert((0, *cutoff_hz, *confidence));
                        entry.0 += 1;
                        entry.2 = entry.2.max(*confidence);
                    }
                    (QualityTier::AiEnhanced, _) => enhanced += 1,
                    (QualityTier::Lossy, _) => {
                        lossy += 1;
                        if lossy_label.is_empty() {
                            lossy_label = quality.label.clone();
                        }
                        let kbps = song
                            .audio
                            .as_ref()
                            .and_then(|audio| audio.bitrate_kbps)
                            .unwrap_or(0);
                        if kbps > 0 && kbps < LOW_BITRATE_KBPS {
                            let entry = low.get_or_insert((0, kbps, quality.label.clone()));
                            entry.0 += 1;
                            if kbps < entry.1 {
                                entry.1 = kbps;
                                entry.2 = quality.label.clone();
                            }
                        }
                    }
                    (QualityTier::Lossless, _) => cd += 1,
                    _ => {}
                }
            }
            let (quality, _) = album_quality_with(album, marks);
            let mut reasons = Vec::new();
            if let Some((tracks, cutoff, confidence)) = fake {
                reasons.push(Reason {
                    kind: ReasonKind::FakeLossless,
                    label: match cutoff {
                        Some(hz) => format!("Fake lossless · {}", format_khz(hz)),
                        None => "Fake lossless".into(),
                    },
                    detail: format!(
                        "{tracks} lossless file(s) stop at {} like a {} kbps encode ({}% sure): made from a lossy copy.",
                        cutoff.map_or_else(|| "a lossy cut-off".to_string(), format_khz),
                        cutoff.map_or(128, likely_source_kbps),
                        (confidence * 100.0).round()
                    ),
                    tracks,
                });
            }
            if let Some((tracks, kbps, label)) = low {
                reasons.push(Reason {
                    kind: ReasonKind::LowBitrate,
                    label: "Low bitrate".into(),
                    detail: format!(
                        "{tracks} track(s) at {kbps} kbps or less ({label}): clearly audible compression."
                    ),
                    tracks,
                });
            } else if lossy > 0 {
                reasons.push(Reason {
                    kind: ReasonKind::Lossy,
                    label: "Lossy".into(),
                    detail: format!("{lossy} track(s) only in a lossy format ({lossy_label})."),
                    tracks: lossy,
                });
            }
            if enhanced > 0 {
                reasons.push(Reason {
                    kind: ReasonKind::AiEnhancedOnly,
                    label: "AI-enhanced only".into(),
                    detail: format!(
                        "{enhanced} track(s) play an AI-enhanced copy: restored treble is plausible, not the original master. A real lossless copy is still better."
                    ),
                    tracks: enhanced,
                });
            }
            if reasons.is_empty()
                && include_cd_quality
                && cd > 0
                && quality.tier == QualityTier::Lossless
            {
                reasons.push(Reason {
                    kind: ReasonKind::CdQuality,
                    label: "CD quality".into(),
                    detail: "Lossless at 16-bit/44.1 kHz; a hi-res release may exist.".into(),
                    tracks: cd,
                });
            }
            if reasons.is_empty() {
                continue;
            }
            // A genuine lossless version of the same album already exists.
            let best = best_version
                .get(&group_key(artist, album))
                .copied()
                .unwrap_or(0);
            let only_cd = reasons.iter().all(|r| r.kind == ReasonKind::CdQuality);
            if !only_cd && best >= QualityTier::Lossless.rank() && !genuinely_lossless(quality.tier)
            {
                continue;
            }
            if only_cd && best > QualityTier::Lossless.rank() {
                continue;
            }
            candidates.push(Candidate {
                album_id: album.id.clone(),
                title: album.name.clone(),
                artist_id: artist.id.clone(),
                artist_name: artist.name.clone(),
                year: year_of(&album.first_release_date),
                cover_url: album.cover_url.clone(),
                musicbrainz_id: album.musicbrainz_id.clone(),
                track_count: album.songs.len(),
                quality,
                reasons,
            });
        }
    }
    candidates.sort_by(|a, b| {
        a.quality
            .score
            .cmp(&b.quality.score)
            .then(
                a.artist_name
                    .to_lowercase()
                    .cmp(&b.artist_name.to_lowercase()),
            )
            .then(a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AudioProperties, Song};

    fn song(id: &str, path: &str, codec: &str, kbps: u32, bits: Option<u8>) -> Song {
        Song {
            id: id.into(),
            name: id.into(),
            path: path.into(),
            track_number: 1,
            duration: 200.0,
            audio: Some(AudioProperties {
                codec: Some(codec.into()),
                sample_rate: Some(44_100),
                bit_depth: bits,
                channels: Some(2),
                bitrate_kbps: Some(kbps),
                lossless: crate::domain::codec_is_lossless(codec),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn album(id: &str, title: &str, songs: Vec<Song>) -> Album {
        Album {
            id: id.into(),
            name: title.into(),
            songs,
            first_release_date: "2011".into(),
            ..Default::default()
        }
    }

    fn library() -> Vec<Artist> {
        vec![Artist {
            id: "ra".into(),
            name: "Rise Against".into(),
            albums: vec![
                album(
                    "mp3-320",
                    "Endgame",
                    vec![song("a", "/m/a.mp3", "mp3", 320, None)],
                ),
                album(
                    "mp3-128",
                    "Siren Song",
                    vec![song("b", "/m/b.mp3", "mp3", 128, None)],
                ),
                album(
                    "fake",
                    "Appeal to Reason",
                    vec![song("c", "/m/c.flac", "flac", 900, Some(16))],
                ),
                album(
                    "real",
                    "The Sufferer",
                    vec![song("d", "/m/d.flac", "flac", 900, Some(16))],
                ),
                album(
                    "hires",
                    "Nowhere Generation",
                    vec![song("e", "/m/e.flac", "flac", 3000, Some(24))],
                ),
                // A lossy album with a genuine lossless version elsewhere.
                album(
                    "dupe-mp3",
                    "Wolves (Deluxe)",
                    vec![song("f", "/m/f.mp3", "mp3", 256, None)],
                ),
                album(
                    "dupe-flac",
                    "Wolves",
                    vec![song("g", "/m/g.flac", "flac", 900, Some(16))],
                ),
            ],
            ..Default::default()
        }]
    }

    fn marks() -> FileMarks {
        let mut marks = FileMarks::default();
        marks.by_path.insert(
            "/m/c.flac".into(),
            FileMark::FakeLossless {
                cutoff_hz: Some(16_000),
                confidence: 0.92,
            },
        );
        marks
    }

    #[test]
    fn lossy_low_bitrate_and_fake_lossless_albums_are_candidates() {
        let marks = marks();
        let found = find_candidates(&library(), &marks, false);
        let ids: Vec<&str> = found.iter().map(|c| c.album_id.as_str()).collect();
        assert!(ids.contains(&"mp3-320"));
        assert!(ids.contains(&"mp3-128"));
        assert!(ids.contains(&"fake"), "{ids:?}");
        assert!(!ids.contains(&"real"));
        assert!(!ids.contains(&"hires"));
        assert!(!ids.contains(&"dupe-mp3"), "a lossless version exists");
        let low = found.iter().find(|c| c.album_id == "mp3-128").unwrap();
        assert_eq!(low.reasons[0].kind, ReasonKind::LowBitrate);
        let lossy = found.iter().find(|c| c.album_id == "mp3-320").unwrap();
        assert_eq!(lossy.reasons[0].kind, ReasonKind::Lossy);
        assert_eq!(lossy.reasons[0].label, "Lossy");
        assert!(lossy.reasons[0].detail.contains("MP3 320"));
        assert_eq!(low.reasons[0].label, "Low bitrate");
        let fake = found.iter().find(|c| c.album_id == "fake").unwrap();
        assert_eq!(fake.reasons[0].kind, ReasonKind::FakeLossless);
        assert_eq!(fake.reasons[0].label, "Fake lossless · 16 kHz");
        assert!(fake.reasons[0].detail.contains("92% sure"));
        // Worst first: the fake made from a 128 kbps file, then the MP3 128.
        assert_eq!(found[0].album_id, "fake");
        assert_eq!(found[1].album_id, "mp3-128");
    }

    #[test]
    fn cd_quality_albums_are_listed_only_when_asked() {
        let marks = FileMarks::default();
        let without = find_candidates(&library(), &marks, false);
        assert!(!without.iter().any(|c| c.album_id == "real"));
        let with = find_candidates(&library(), &marks, true);
        let real = with.iter().find(|c| c.album_id == "real").expect("listed");
        assert_eq!(real.reasons[0].kind, ReasonKind::CdQuality);
        assert!(!with.iter().any(|c| c.album_id == "hires"));
    }
}
