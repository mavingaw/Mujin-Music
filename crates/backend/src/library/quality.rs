//! An album's format for its quality badge ("24/96 FLAC", "16/44.1 ALAC",
//! "MP3 320", "Mixed") on album cards in Home rows, the Library grid and
//! everywhere else. Every screen reads this one answer, so an album never
//! shows one badge on Home and another in the Library.

use serde::Serialize;

use crate::domain::{Album, AudioProperties};

/// An album's format, for its quality badge.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct AlbumQuality {
    pub codec: Option<String>,
    /// The lowest sample rate among the album's tracks: the album is only
    /// as good as its weakest file.
    pub sample_rate: Option<u32>,
    /// The lowest bit depth among the album's tracks.
    pub bit_depth: Option<u8>,
    /// The lowest bitrate among lossy tracks.
    pub bitrate_kbps: Option<u32>,
    /// Every track with a known codec is lossless.
    pub lossless: bool,
    /// Every track is lossless above CD resolution (more than 16 bit or
    /// 48 kHz).
    pub hi_res: bool,
    /// The tracks come in more than one codec, or mix lossless and lossy.
    pub mixed: bool,
    /// The badge text every screen shows.
    pub label: String,
}

fn khz(rate: u32) -> String {
    if rate.is_multiple_of(1000) {
        (rate / 1000).to_string()
    } else {
        format!("{:.1}", f64::from(rate) / 1000.0)
    }
}

fn is_hi_res(audio: &AudioProperties) -> bool {
    audio.lossless == Some(true)
        && (audio.bit_depth.unwrap_or(0) > 16 || audio.sample_rate.unwrap_or(0) > 48_000)
}

pub fn album_quality(album: &Album) -> Option<AlbumQuality> {
    let tracks = album
        .songs
        .iter()
        .filter_map(|song| song.audio.as_ref())
        .filter(|audio| !audio.is_empty() && audio.codec.is_some())
        .collect::<Vec<_>>();
    let first = tracks.first()?;
    let codec_of = |audio: &AudioProperties| audio.codec.as_deref().unwrap_or("").to_lowercase();
    let codec = codec_of(first);
    let lossless = tracks.iter().all(|audio| audio.lossless == Some(true));
    let mixed = tracks
        .iter()
        .any(|audio| codec_of(audio) != codec || audio.lossless != first.lossless);
    let hi_res = tracks.iter().all(|audio| is_hi_res(audio));
    let bit_depth = tracks.iter().filter_map(|audio| audio.bit_depth).min();
    let sample_rate = tracks.iter().filter_map(|audio| audio.sample_rate).min();
    let bitrate_kbps = tracks
        .iter()
        .filter(|audio| audio.lossless == Some(false))
        .filter_map(|audio| audio.bitrate_kbps)
        .min();
    let name = codec.to_uppercase();
    let label = if mixed {
        "Mixed".to_string()
    } else if lossless {
        match (bit_depth, sample_rate) {
            (Some(bits), Some(rate)) if bits > 0 && rate > 0 => {
                format!("{bits}/{} {name}", khz(rate))
            }
            _ => name,
        }
    } else {
        match bitrate_kbps {
            Some(rate) if rate > 0 => format!("{name} {rate}"),
            _ => name,
        }
    };
    Some(AlbumQuality {
        codec: Some(codec),
        sample_rate,
        bit_depth,
        bitrate_kbps,
        lossless,
        hi_res: hi_res && !mixed,
        mixed,
        label,
    })
}

/// The badge text; `None` when nothing is known.
pub fn quality_label(quality: &AlbumQuality) -> Option<String> {
    Some(quality.label.clone()).filter(|label| !label.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Song;

    fn track(codec: &str, lossless: bool, bits: Option<u8>, rate: u32, kbps: Option<u32>) -> Song {
        Song {
            audio: Some(AudioProperties {
                codec: Some(codec.into()),
                sample_rate: Some(rate),
                bit_depth: bits,
                bitrate_kbps: kbps,
                lossless: Some(lossless),
                ..AudioProperties::default()
            }),
            ..Song::default()
        }
    }

    fn album(songs: Vec<Song>) -> Album {
        Album {
            songs,
            ..Album::default()
        }
    }

    fn label(songs: Vec<Song>) -> Option<String> {
        album_quality(&album(songs))
            .as_ref()
            .and_then(quality_label)
    }

    #[test]
    fn labels_show_depth_rate_and_codec_for_lossless_and_bitrate_for_lossy() {
        assert_eq!(
            label(vec![track("flac", true, Some(24), 96_000, None)]).as_deref(),
            Some("24/96 FLAC")
        );
        assert_eq!(
            label(vec![track("alac", true, Some(16), 44_100, None)]).as_deref(),
            Some("16/44.1 ALAC")
        );
        assert_eq!(
            label(vec![track("mp3", false, None, 44_100, Some(320))]).as_deref(),
            Some("MP3 320")
        );
        assert_eq!(album_quality(&Album::default()), None);
    }

    #[test]
    fn a_mixed_album_says_mixed_and_never_claims_hi_res() {
        // "Discovery": two MP3s and a Vorbis track.
        let discovery = album_quality(&album(vec![
            track("mp3", false, None, 44_100, Some(320)),
            track("mp3", false, None, 44_100, Some(320)),
            track("vorbis", false, None, 48_000, Some(256)),
        ]))
        .unwrap();
        assert_eq!(discovery.label, "Mixed");
        assert!(discovery.mixed && !discovery.lossless && !discovery.hi_res);
        // "Tidewater EP": a 24/48 WAV and an AAC.
        let tidewater = album_quality(&album(vec![
            track("pcm", true, Some(24), 48_000, None),
            track("aac", false, None, 44_100, Some(256)),
        ]))
        .unwrap();
        assert_eq!(tidewater.label, "Mixed");
        assert!(!tidewater.lossless && !tidewater.hi_res);
    }

    #[test]
    fn an_album_is_as_good_as_its_weakest_track() {
        let quality = album_quality(&album(vec![
            track("flac", true, Some(24), 96_000, None),
            track("flac", true, Some(16), 44_100, None),
        ]))
        .unwrap();
        assert_eq!(quality.label, "16/44.1 FLAC");
        assert!(quality.lossless && !quality.hi_res && !quality.mixed);
    }
}
