//! ConnectionManager `GetProtocolInfo` sink lists: which formats a
//! renderer plays, and how far up.

use crate::outputs::FormatLimits;

/// What a renderer's sink protocol list says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SinkCapabilities {
    pub limits: FormatLimits,
    /// The renderer reported a sample rate ceiling (`rate=` on PCM
    /// entries); otherwise the ceiling is Parson's assumption.
    pub rate_reported: bool,
}

/// Ceiling assumed for renderers that list FLAC without a rate.
pub const ASSUMED_MAX_RATE: u32 = 192_000;

/// Parses a `Sink` value such as
/// `http-get:*:audio/flac:*,http-get:*:audio/L16;rate=96000;channels=2:*`.
pub fn parse_sink(sink: &str) -> SinkCapabilities {
    let mut limits = FormatLimits {
        max_sample_rate: 0,
        max_bit_depth: 16,
        flac: false,
        wav: false,
        alac: false,
        mp3: false,
        aac: false,
        lpcm: false,
    };
    let mut reported_rate = 0;
    let mut pcm24 = false;
    for entry in sink.split(',') {
        let mut fields = entry.trim().splitn(4, ':');
        let (Some(protocol), Some(_network), Some(content)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if !protocol.eq_ignore_ascii_case("http-get") {
            continue;
        }
        let mut parts = content.split(';');
        let mime = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
        for parameter in parts {
            if let Some((name, value)) = parameter.split_once('=')
                && name.trim().eq_ignore_ascii_case("rate")
                && let Ok(rate) = value.trim().parse::<u32>()
                && (8_000..=768_000).contains(&rate)
            {
                reported_rate = reported_rate.max(rate);
            }
        }
        match mime.as_str() {
            "audio/flac" | "audio/x-flac" => limits.flac = true,
            "audio/wav" | "audio/x-wav" | "audio/wave" | "audio/vnd.wave" => limits.wav = true,
            "audio/l16" => limits.lpcm = true,
            "audio/l24" => {
                limits.lpcm = true;
                pcm24 = true;
            }
            "audio/mpeg" | "audio/mp3" | "audio/x-mpeg" => limits.mp3 = true,
            "audio/mp4" | "audio/aac" | "audio/x-aac" | "audio/m4a" | "audio/x-m4a" => {
                limits.aac = true
            }
            "audio/x-alac" | "audio/alac" | "audio/apple-lossless" => limits.alac = true,
            _ => {}
        }
    }
    let lossless = limits.flac || limits.wav || limits.alac || limits.lpcm;
    limits.max_sample_rate = if reported_rate > 0 {
        reported_rate
    } else if lossless {
        ASSUMED_MAX_RATE
    } else {
        48_000
    };
    // L16 alone carries 16 bits; L24, FLAC, WAV and ALAC carry 24.
    limits.max_bit_depth = if pcm24 || limits.flac || limits.wav || limits.alac {
        24
    } else {
        16
    };
    SinkCapabilities {
        limits,
        rate_reported: reported_rate > 0,
    }
}

/// The protocol list a renderer without a ConnectionManager gets: the
/// formats every DLNA audio renderer must play.
pub fn baseline() -> SinkCapabilities {
    parse_sink("http-get:*:audio/mpeg:*")
}

#[cfg(test)]
mod tests {
    use super::{ASSUMED_MAX_RATE, parse_sink};

    #[test]
    fn reads_formats_and_reported_rates() {
        let caps = parse_sink(
            "http-get:*:audio/flac:*,http-get:*:audio/L16;rate=96000;channels=2:DLNA.ORG_PN=LPCM,http-get:*:audio/L24;rate=96000;channels=2:*,http-get:*:audio/mpeg:DLNA.ORG_PN=MP3,rtsp-rtp-udp:*:audio/L16;rate=384000:*,http-get:*:audio/mp4:*",
        );
        assert!(caps.limits.flac && caps.limits.mp3 && caps.limits.aac);
        assert!(!caps.limits.alac && !caps.limits.wav);
        assert_eq!(caps.limits.max_sample_rate, 96_000, "rtsp entries ignored");
        assert_eq!(caps.limits.max_bit_depth, 24);
        assert!(caps.rate_reported);

        let assumed = parse_sink("http-get:*:audio/x-flac:*, http-get:*:audio/wav:*");
        assert_eq!(assumed.limits.max_sample_rate, ASSUMED_MAX_RATE);
        assert!(!assumed.rate_reported);
        assert!(assumed.limits.wav);

        let mp3 = parse_sink("http-get:*:audio/mpeg:*");
        assert_eq!(
            (mp3.limits.max_sample_rate, mp3.limits.max_bit_depth),
            (48_000, 16)
        );
        assert!(!parse_sink("garbage").limits.mp3);
    }
}
