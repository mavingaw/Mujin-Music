//! The best format an album is available in, for quality badges on album
//! cards ("24/96", "16/44.1", "MP3") in Home rows and the library grid.

use serde::Serialize;

use crate::domain::Album;

/// The best format an album is available in, for its quality badge.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct AlbumQuality {
    pub codec: Option<String>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u8>,
    /// Every track with a known codec is lossless.
    pub lossless: bool,
    /// Lossless and above CD resolution (more than 16 bit or 48 kHz).
    pub hi_res: bool,
}

pub fn album_quality(album: &Album) -> Option<AlbumQuality> {
    let mut best: Option<AlbumQuality> = None;
    let mut all_lossless = true;
    for audio in album.songs.iter().filter_map(|song| song.audio.as_ref()) {
        if audio.is_empty() {
            continue;
        }
        if audio.lossless == Some(false) {
            all_lossless = false;
        }
        let candidate = (audio.bit_depth.unwrap_or(0), audio.sample_rate.unwrap_or(0));
        let current = best
            .as_ref()
            .map(|quality| {
                (
                    quality.bit_depth.unwrap_or(0),
                    quality.sample_rate.unwrap_or(0),
                )
            })
            .unwrap_or((0, 0));
        if best.is_none() || candidate > current {
            best = Some(AlbumQuality {
                codec: audio.codec.clone(),
                sample_rate: audio.sample_rate,
                bit_depth: audio.bit_depth,
                lossless: false,
                hi_res: false,
            });
        }
    }
    best.map(|mut quality| {
        quality.lossless = all_lossless;
        quality.hi_res = all_lossless
            && (quality.bit_depth.unwrap_or(0) > 16 || quality.sample_rate.unwrap_or(0) > 48_000);
        quality
    })
}

/// The short badge text: "24/96" for lossless with a known depth and rate,
/// else the codec in capitals ("MP3"); `None` when nothing is known.
pub fn quality_label(quality: &AlbumQuality) -> Option<String> {
    match (quality.lossless, quality.bit_depth, quality.sample_rate) {
        (true, Some(bits), Some(rate)) if bits > 0 && rate > 0 => {
            let khz = f64::from(rate) / 1000.0;
            let rate = if khz.fract() == 0.0 {
                format!("{khz:.0}")
            } else {
                format!("{khz:.1}")
            };
            Some(format!("{bits}/{rate}"))
        }
        _ => quality.codec.as_ref().map(|codec| codec.to_uppercase()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quality(lossless: bool, bits: Option<u8>, rate: Option<u32>, codec: &str) -> AlbumQuality {
        AlbumQuality {
            codec: Some(codec.into()),
            sample_rate: rate,
            bit_depth: bits,
            lossless,
            hi_res: false,
        }
    }

    #[test]
    fn labels_show_depth_and_rate_for_lossless_and_the_codec_otherwise() {
        assert_eq!(
            quality_label(&quality(true, Some(24), Some(96_000), "flac")).as_deref(),
            Some("24/96")
        );
        assert_eq!(
            quality_label(&quality(true, Some(16), Some(44_100), "flac")).as_deref(),
            Some("16/44.1")
        );
        assert_eq!(
            quality_label(&quality(false, None, Some(44_100), "mp3")).as_deref(),
            Some("MP3")
        );
        assert_eq!(
            quality_label(&quality(true, None, Some(44_100), "alac")).as_deref(),
            Some("ALAC")
        );
    }
}
