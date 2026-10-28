//! Library statistics: how much music, in which formats and resolutions,
//! and how much of it is identified and analyzed.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;

use super::layers::song_quality;
use super::quality::{QualityTier, dsd_rate_label, khz};
use crate::domain::Artist;

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct Breakdown {
    pub key: String,
    pub label: String,
    pub tracks: u64,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct Identification {
    /// Tracks on albums matched to a MusicBrainz release.
    pub identified: u64,
    /// Tracks on albums waiting for their lookup.
    pub awaiting: u64,
    /// Tracks on albums whose lookup failed or found no confident match.
    pub unidentified: u64,
    /// Whether online identification is turned on.
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct Analysis {
    /// Tracks with loudness measured and a waveform stored.
    pub analyzed: u64,
    /// Tracks with only one of the two.
    pub partial: u64,
    pub awaiting: u64,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct LibraryStats {
    pub tracks: u64,
    pub albums: u64,
    pub artists: u64,
    pub duration_seconds: f64,
    pub size_bytes: i64,
    /// DSD, Hi-Res lossless, CD-quality lossless, lossy, unknown.
    pub tiers: Vec<Breakdown>,
    /// FLAC, ALAC, WAV, DSD, MP3, AAC, ...
    pub formats: Vec<Breakdown>,
    /// Sample rates of lossless and DSD tracks.
    pub sample_rates: Vec<Breakdown>,
    /// Bit depths of lossless PCM tracks.
    pub bit_depths: Vec<Breakdown>,
    pub identification: Identification,
    pub analysis: Analysis,
}

/// Per-album identification state from the enrichment tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlbumIdentity {
    Identified,
    Awaiting,
    Unidentified,
}

pub fn album_identity(
    task: Option<&str>,
    release_match: Option<&str>,
    tagged_mbid: bool,
) -> AlbumIdentity {
    if release_match == Some("verified") || tagged_mbid && release_match.is_none() {
        return AlbumIdentity::Identified;
    }
    match task {
        Some("pending") | Some("running") => AlbumIdentity::Awaiting,
        None if release_match.is_none() => AlbumIdentity::Awaiting,
        _ => AlbumIdentity::Unidentified,
    }
}

fn tier_order(tier: QualityTier) -> usize {
    match tier {
        QualityTier::Dsd => 0,
        QualityTier::HiRes => 1,
        QualityTier::Lossless => 2,
        QualityTier::AiEnhanced => 3,
        QualityTier::Lossy => 4,
        QualityTier::Unknown => 5,
    }
}

fn tier_key(tier: QualityTier) -> &'static str {
    match tier {
        QualityTier::Dsd => "dsd",
        QualityTier::HiRes => "hi_res",
        QualityTier::Lossless => "lossless",
        QualityTier::AiEnhanced => "ai_enhanced",
        QualityTier::Lossy => "lossy",
        QualityTier::Unknown => "unknown",
    }
}

pub struct StatsInputs<'a> {
    pub library: &'a [Artist],
    pub size_bytes: i64,
    /// album id -> identity.
    pub identities: &'a HashMap<String, AlbumIdentity>,
    pub identification_enabled: bool,
    pub measured_songs: &'a HashSet<String>,
    pub analyzed_paths: &'a HashSet<String>,
}

pub fn compute(inputs: StatsInputs<'_>) -> LibraryStats {
    let mut stats = LibraryStats {
        size_bytes: inputs.size_bytes,
        identification: Identification {
            enabled: inputs.identification_enabled,
            ..Identification::default()
        },
        ..LibraryStats::default()
    };
    let mut tiers: BTreeMap<usize, (QualityTier, u64)> = BTreeMap::new();
    let mut formats: HashMap<String, u64> = HashMap::new();
    let mut rates: BTreeMap<u32, (bool, u64)> = BTreeMap::new();
    let mut depths: BTreeMap<u8, u64> = BTreeMap::new();
    for artist in inputs.library {
        if !artist.albums.is_empty() {
            stats.artists += 1;
        }
        for album in &artist.albums {
            stats.albums += 1;
            let identity = inputs.identities.get(&album.id).copied().unwrap_or(
                if album.musicbrainz_id.is_empty() {
                    AlbumIdentity::Unidentified
                } else {
                    AlbumIdentity::Identified
                },
            );
            for song in &album.songs {
                stats.tracks += 1;
                stats.duration_seconds += song.duration.max(0.0);
                // Lookups wait forever while online metadata is off.
                let identity = match identity {
                    AlbumIdentity::Awaiting if !inputs.identification_enabled => {
                        AlbumIdentity::Unidentified
                    }
                    identity => identity,
                };
                match identity {
                    AlbumIdentity::Identified => stats.identification.identified += 1,
                    AlbumIdentity::Awaiting => stats.identification.awaiting += 1,
                    AlbumIdentity::Unidentified => stats.identification.unidentified += 1,
                }
                let measured = inputs.measured_songs.contains(&song.id);
                let analyzed = inputs.analyzed_paths.contains(&song.path);
                match (measured, analyzed) {
                    (true, true) => stats.analysis.analyzed += 1,
                    (false, false) => stats.analysis.awaiting += 1,
                    _ => stats.analysis.partial += 1,
                }
                let quality = song_quality(song);
                tiers
                    .entry(tier_order(quality.tier))
                    .or_insert((quality.tier, 0))
                    .1 += 1;
                *formats.entry(quality.format.clone()).or_default() += 1;
                let audio = song.audio.as_ref();
                match quality.tier {
                    QualityTier::Dsd => {
                        if let Some(rate) = audio.and_then(|audio| audio.sample_rate) {
                            rates.entry(rate).or_insert((true, 0)).1 += 1;
                        }
                    }
                    QualityTier::HiRes | QualityTier::Lossless => {
                        if let Some(rate) = audio.and_then(|audio| audio.sample_rate) {
                            rates.entry(rate).or_insert((false, 0)).1 += 1;
                        }
                        if let Some(depth) = audio.and_then(|audio| audio.bit_depth) {
                            *depths.entry(depth).or_default() += 1;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    stats.duration_seconds = stats.duration_seconds.round();
    stats.tiers = tiers
        .into_values()
        .map(|(tier, tracks)| Breakdown {
            key: tier_key(tier).into(),
            label: tier.label().into(),
            tracks,
        })
        .collect();
    let mut formats: Vec<Breakdown> = formats
        .into_iter()
        .map(|(format, tracks)| Breakdown {
            key: format.to_ascii_lowercase(),
            label: format,
            tracks,
        })
        .collect();
    formats.sort_by(|a, b| b.tracks.cmp(&a.tracks).then(a.label.cmp(&b.label)));
    stats.formats = formats;
    stats.sample_rates = rates
        .into_iter()
        .map(|(rate, (dsd, tracks))| Breakdown {
            key: rate.to_string(),
            label: if dsd {
                dsd_rate_label(rate)
            } else {
                format!("{} kHz", khz(rate))
            },
            tracks,
        })
        .collect();
    stats.bit_depths = depths
        .into_iter()
        .map(|(depth, tracks)| Breakdown {
            key: depth.to_string(),
            label: format!("{depth}-bit"),
            tracks,
        })
        .collect();
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Album, AudioProperties, Song};

    fn song(id: &str, codec: &str, rate: u32, depth: Option<u8>, lossless: bool) -> Song {
        Song {
            id: id.into(),
            path: format!("/m/{id}.{codec}"),
            duration: 100.0,
            audio: Some(AudioProperties {
                codec: Some(codec.into()),
                sample_rate: Some(rate),
                bit_depth: depth,
                bitrate_kbps: Some(320),
                lossless: Some(lossless),
                ..AudioProperties::default()
            }),
            ..Song::default()
        }
    }

    #[test]
    fn stats_break_down_tiers_formats_rates_identification_and_analysis() {
        let library = vec![Artist {
            id: "artist".into(),
            albums: vec![
                Album {
                    id: "hires".into(),
                    songs: vec![
                        song("a", "flac", 96_000, Some(24), true),
                        song("b", "flac", 44_100, Some(16), true),
                    ],
                    ..Album::default()
                },
                Album {
                    id: "lossy".into(),
                    songs: vec![
                        song("c", "mp3", 44_100, None, false),
                        song("d", "dsd", 2_822_400, Some(1), true),
                    ],
                    ..Album::default()
                },
            ],
            ..Artist::default()
        }];
        let identities = HashMap::from([
            ("hires".to_string(), AlbumIdentity::Identified),
            ("lossy".to_string(), AlbumIdentity::Awaiting),
        ]);
        let measured = HashSet::from(["a".to_string(), "c".to_string()]);
        let analyzed = HashSet::from(["/m/a.flac".to_string()]);
        let stats = compute(StatsInputs {
            library: &library,
            size_bytes: 1000,
            identities: &identities,
            identification_enabled: true,
            measured_songs: &measured,
            analyzed_paths: &analyzed,
        });
        assert_eq!((stats.tracks, stats.albums, stats.artists), (4, 2, 1));
        assert_eq!(stats.duration_seconds, 400.0);
        let tiers: Vec<(&str, u64)> = stats
            .tiers
            .iter()
            .map(|t| (t.key.as_str(), t.tracks))
            .collect();
        assert_eq!(
            tiers,
            vec![("dsd", 1), ("hi_res", 1), ("lossless", 1), ("lossy", 1)]
        );
        assert_eq!(stats.formats[0].label, "FLAC");
        assert_eq!(stats.formats[0].tracks, 2);
        let rates: Vec<&str> = stats
            .sample_rates
            .iter()
            .map(|r| r.label.as_str())
            .collect();
        assert_eq!(rates, vec!["44.1 kHz", "96 kHz", "DSD64"]);
        assert_eq!(stats.bit_depths.len(), 2);
        assert_eq!(stats.identification.identified, 2);
        assert_eq!(stats.identification.awaiting, 2);
        assert_eq!(
            (
                stats.analysis.analyzed,
                stats.analysis.partial,
                stats.analysis.awaiting
            ),
            (1, 1, 2)
        );
    }

    #[test]
    fn identity_prefers_a_verified_match_and_waits_for_pending_lookups() {
        assert_eq!(
            album_identity(Some("done"), Some("verified"), false),
            AlbumIdentity::Identified
        );
        assert_eq!(
            album_identity(Some("pending"), None, false),
            AlbumIdentity::Awaiting
        );
        assert_eq!(
            album_identity(Some("done"), Some("unverified"), false),
            AlbumIdentity::Unidentified
        );
        assert_eq!(
            album_identity(Some("failed"), None, false),
            AlbumIdentity::Unidentified
        );
        assert_eq!(
            album_identity(Some("done"), None, true),
            AlbumIdentity::Identified
        );
    }
}
