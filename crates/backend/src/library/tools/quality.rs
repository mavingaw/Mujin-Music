//! One quality scale for every copy of a recording, so "prefer the highest
//! quality" means the same thing on the duplicates page, the album versions
//! page and the format breakdown.

use serde::Serialize;

use crate::domain::AudioProperties;

/// Broad quality tier, best first in [`QualityTier::rank`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityTier {
    /// Direct Stream Digital (DSF/DFF).
    Dsd,
    /// Lossless above CD resolution (more than 16 bits or 48 kHz).
    HiRes,
    /// Lossless at CD resolution.
    Lossless,
    /// Made by the AI enhancer from a lossy copy: plausible restored
    /// treble, never the original master, so never graded as lossless.
    AiEnhanced,
    /// Lossy codecs (MP3, AAC, Vorbis, Opus...).
    Lossy,
    /// The file's stream could not be read.
    Unknown,
}

impl QualityTier {
    pub fn rank(self) -> u64 {
        match self {
            Self::Dsd => 5,
            Self::HiRes => 4,
            Self::Lossless => 3,
            Self::AiEnhanced => 2,
            Self::Lossy => 1,
            Self::Unknown => 0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Dsd => "DSD",
            Self::HiRes => "Hi-Res lossless",
            Self::Lossless => "CD-quality lossless",
            Self::AiEnhanced => "AI-enhanced",
            Self::Lossy => "Lossy",
            Self::Unknown => "Unknown",
        }
    }
}

/// Quality of one file: a tier, a comparable score and a short label such
/// as `FLAC 24/96`, `DSD128` or `MP3 320`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Quality {
    pub tier: QualityTier,
    pub score: u64,
    pub label: String,
    /// Codec family for the format breakdown, e.g. `FLAC`, `ALAC`, `MP3`.
    pub format: String,
}

/// Display name of a codec, using the container to name PCM.
pub fn format_name(codec: Option<&str>, container: Option<&str>, extension: &str) -> String {
    let extension = extension.to_ascii_lowercase();
    match codec.unwrap_or_default() {
        "flac" => "FLAC".into(),
        "alac" => "ALAC".into(),
        "pcm" | "pcm_float" => match container.unwrap_or(extension.as_str()) {
            "aiff" | "aif" | "aifc" => "AIFF".into(),
            "w64" | "wave64" => "Wave64".into(),
            "caf" => "CAF".into(),
            _ => "WAV".into(),
        },
        "dsd" | "dst" => "DSD".into(),
        "wavpack" => "WavPack".into(),
        "ape" => "APE".into(),
        "tta" => "TTA".into(),
        "tak" => "TAK".into(),
        "mp3" => "MP3".into(),
        "aac" => "AAC".into(),
        "vorbis" => "Vorbis".into(),
        "opus" => "Opus".into(),
        "musepack" => "Musepack".into(),
        "wma" | "wma_pro" | "wma_lossless" => "WMA".into(),
        "" => match extension.as_str() {
            "flac" => "FLAC".into(),
            "mp3" => "MP3".into(),
            "dsf" | "dff" => "DSD".into(),
            "wav" => "WAV".into(),
            "aiff" | "aif" => "AIFF".into(),
            "m4a" | "aac" => "AAC".into(),
            "ogg" | "oga" => "Vorbis".into(),
            "opus" => "Opus".into(),
            "" => "Unknown".into(),
            other => other.to_ascii_uppercase(),
        },
        other => other.to_ascii_uppercase(),
    }
}

/// `44.1`, `96`, `176.4` for a sample rate in Hz.
pub fn khz(sample_rate: u32) -> String {
    if sample_rate.is_multiple_of(1000) {
        format!("{}", sample_rate / 1000)
    } else {
        let value = f64::from(sample_rate) / 1000.0;
        let text = format!("{value:.1}");
        text.trim_end_matches(".0").to_string()
    }
}

/// `DSD64` .. `DSD512` from a 1-bit sample rate.
pub fn dsd_rate_label(sample_rate: u32) -> String {
    let multiple = (f64::from(sample_rate) / 44_100.0).round() as u32;
    if multiple >= 32 {
        format!("DSD{multiple}")
    } else {
        "DSD".into()
    }
}

/// Grades one file from its technical properties and extension.
pub fn grade(audio: Option<&AudioProperties>, extension: &str) -> Quality {
    let codec = audio.and_then(|audio| audio.codec.as_deref());
    let container = audio.and_then(|audio| audio.container.as_deref());
    let format = format_name(codec, container, extension);
    let sample_rate = audio.and_then(|audio| audio.sample_rate).unwrap_or(0);
    let bit_depth = audio.and_then(|audio| audio.bit_depth).unwrap_or(0);
    let bitrate = audio.and_then(|audio| audio.bitrate_kbps).unwrap_or(0);
    let extension = extension.to_ascii_lowercase();
    // No readable stream properties: the file's quality is unknown, whatever
    // its extension promises.
    let no_stream = audio.is_none_or(|audio| {
        audio.is_empty() || audio.sample_rate.is_none() && audio.bitrate_kbps.is_none()
    });
    if no_stream {
        return Quality {
            tier: QualityTier::Unknown,
            score: 0,
            label: format.clone(),
            format,
        };
    }
    let is_dsd =
        matches!(codec, Some("dsd" | "dst")) || matches!(extension.as_str(), "dsf" | "dff");
    let lossless = audio.and_then(|audio| audio.lossless).or_else(|| {
        crate::library::formats::format_for_extension(&extension).and_then(|info| info.lossless)
    });

    if is_dsd {
        let label = if sample_rate > 0 {
            dsd_rate_label(sample_rate)
        } else {
            "DSD".into()
        };
        return Quality {
            tier: QualityTier::Dsd,
            score: QualityTier::Dsd.rank() * 10_000_000 + u64::from(sample_rate / 1000),
            label,
            format,
        };
    }
    match lossless {
        Some(true) => {
            let hi_res = bit_depth > 16 || sample_rate > 48_000;
            let tier = if hi_res {
                QualityTier::HiRes
            } else {
                QualityTier::Lossless
            };
            let label = match (bit_depth, sample_rate) {
                (0, 0) => format.clone(),
                (0, rate) => format!("{format} {}kHz", khz(rate)),
                (bits, 0) => format!("{format} {bits}-bit"),
                (bits, rate) => format!("{format} {bits}/{}", khz(rate)),
            };
            Quality {
                tier,
                score: tier.rank() * 10_000_000
                    + u64::from(bit_depth) * 100_000
                    + u64::from(sample_rate / 100),
                label,
                format,
            }
        }
        Some(false) => {
            // Modern codecs sound better per bit than MP3.
            let efficiency = match codec.unwrap_or(extension.as_str()) {
                "opus" => 160,
                "aac" | "m4a" | "vorbis" | "ogg" => 125,
                _ => 100,
            };
            let label = if bitrate > 0 {
                format!("{format} {bitrate}")
            } else {
                format.clone()
            };
            Quality {
                tier: QualityTier::Lossy,
                score: QualityTier::Lossy.rank() * 10_000_000
                    + u64::from(bitrate) * efficiency
                    + u64::from(sample_rate / 1000),
                label,
                format,
            }
        }
        None => Quality {
            tier: QualityTier::Unknown,
            score: u64::from(bitrate),
            label: format.clone(),
            format,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(
        codec: &str,
        sample_rate: u32,
        bit_depth: u8,
        bitrate: u32,
        lossless: bool,
    ) -> AudioProperties {
        AudioProperties {
            codec: Some(codec.into()),
            container: None,
            sample_rate: Some(sample_rate),
            bit_depth: (bit_depth > 0).then_some(bit_depth),
            channels: Some(2),
            bitrate_kbps: Some(bitrate),
            lossless: Some(lossless),
            enhanced_by: None,
        }
    }

    #[test]
    fn hi_res_beats_cd_beats_lossy_and_dsd_tops_the_scale() {
        let dsd = grade(Some(&props("dsd", 5_644_800, 1, 0, true)), "dsf");
        let hires = grade(Some(&props("flac", 96_000, 24, 3000, true)), "flac");
        let cd = grade(Some(&props("flac", 44_100, 16, 900, true)), "flac");
        let mp3 = grade(Some(&props("mp3", 44_100, 0, 320, false)), "mp3");
        let aac = grade(Some(&props("aac", 44_100, 0, 256, false)), "m4a");
        assert_eq!(dsd.label, "DSD128");
        assert_eq!(hires.label, "FLAC 24/96");
        assert_eq!(cd.label, "FLAC 16/44.1");
        assert_eq!(mp3.label, "MP3 320");
        assert_eq!(hires.tier, QualityTier::HiRes);
        assert_eq!(cd.tier, QualityTier::Lossless);
        assert!(dsd.score > hires.score);
        assert!(hires.score > cd.score);
        assert!(cd.score > mp3.score);
        // AAC 256 is graded at least as well as MP3 320.
        assert!(aac.score >= mp3.score);
    }

    #[test]
    fn pcm_is_named_by_its_container_and_rates_read_naturally() {
        assert_eq!(format_name(Some("pcm"), Some("aiff"), "aif"), "AIFF");
        assert_eq!(format_name(Some("pcm"), None, "wav"), "WAV");
        assert_eq!(format_name(None, None, "flac"), "FLAC");
        assert_eq!(khz(176_400), "176.4");
        assert_eq!(khz(192_000), "192");
        assert_eq!(dsd_rate_label(2_822_400), "DSD64");
        let unknown = grade(None, "xyz");
        assert_eq!(unknown.tier, QualityTier::Unknown);
        // A FLAC whose stream could not be read is not graded as lossless.
        assert_eq!(grade(None, "flac").tier, QualityTier::Unknown);
        let codec_only = AudioProperties {
            codec: Some("flac".into()),
            lossless: Some(true),
            ..AudioProperties::default()
        };
        assert_eq!(grade(Some(&codec_only), "flac").tier, QualityTier::Unknown);
    }
}
