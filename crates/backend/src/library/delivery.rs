//! Chooses how a track reaches a player: its original bytes, a lossless
//! conversion, DSD over PCM, or a lossy transcode, and explains every step.
//!
//! The rule is "never lose what the output can carry". Original bytes are
//! sent whenever the player decodes the format within its limits. Otherwise
//! the server converts on demand to the best lossless form the player
//! accepts (FLAC, then WAV), keeping the source rate, depth and channel count
//! unless the player cannot take them. DSD becomes 24-bit PCM at the DSD
//! rate / 16 (176.4 kHz for DSD64), or DoP for outputs that pass PCM through
//! untouched to a DoP DAC. A lossy transcode only happens when the listener
//! asks for a bitrate or an effect, or when the player decodes nothing better.

use serde::{Deserialize, Serialize};

use crate::domain::AudioProperties;
use crate::library::formats::{dsd_rate_name, format_for_extension};

/// Highest PCM rate assumed for players that do not state a limit.
pub const DEFAULT_MAX_SAMPLE_RATE: u32 = 192_000;
/// Highest PCM depth the server writes (FLAC/WAV players accept 24-bit universally).
pub const MAX_OUTPUT_BIT_DEPTH: u8 = 24;
/// Bumped whenever conversion settings change, so cached conversions are rebuilt.
pub const CONVERSION_REVISION: &str = "v1";

/// How a player wants DSD.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DsdOutput {
    /// Convert to PCM (browsers, phones, Chromecast).
    #[default]
    Pcm,
    /// DSD over PCM v1.1 for a bit-transparent path to a DoP DAC.
    Dop,
    /// Original DSF/DFF bytes for players that decode DSD themselves.
    Native,
}

impl DsdOutput {
    pub fn parse(value: Option<&str>) -> Self {
        match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("dop") => Self::Dop,
            Some("native") => Self::Native,
            _ => Self::Pcm,
        }
    }
}

/// What the requesting player can decode.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct OutputCapabilities {
    /// Format tokens decoded natively: `flac`, `alac`, `aac`, `mp3`, `vorbis`,
    /// `opus`, `wav`, `wav_float`, `aiff`, `wavpack`, `ape`, `dsf`, `dff`, ...
    pub accept: Vec<String>,
    pub max_sample_rate: Option<u32>,
    pub max_bit_depth: Option<u8>,
    pub max_channels: Option<u8>,
    #[serde(default)]
    pub dsd: DsdOutput,
}

impl OutputCapabilities {
    /// Parses a comma-separated `accept` list.
    pub fn from_query(
        accept: Option<&str>,
        max_sample_rate: Option<u32>,
        max_bit_depth: Option<u8>,
        max_channels: Option<u8>,
        dsd: Option<&str>,
    ) -> Self {
        Self {
            accept: accept
                .unwrap_or_default()
                .split(',')
                .map(|token| token.trim().to_ascii_lowercase())
                .filter(|token| !token.is_empty() && token.len() <= 16)
                .take(32)
                .collect(),
            max_sample_rate: max_sample_rate.filter(|rate| *rate >= 8_000),
            max_bit_depth: max_bit_depth.filter(|depth| *depth >= 8),
            max_channels: max_channels.filter(|channels| *channels >= 1),
            dsd: DsdOutput::parse(dsd),
        }
    }

    /// Google Cast Default Media Receiver: FLAC/WAV up to 24-bit/96 kHz, stereo.
    pub fn google_cast() -> Self {
        Self {
            accept: ["flac", "mp3", "aac", "wav", "vorbis", "opus"]
                .into_iter()
                .map(String::from)
                .collect(),
            max_sample_rate: Some(96_000),
            max_bit_depth: Some(24),
            max_channels: Some(2),
            dsd: DsdOutput::Pcm,
        }
    }

    fn accepts(&self, token: &str) -> bool {
        self.accept.iter().any(|accepted| accepted == token)
    }

    fn max_rate(&self) -> u32 {
        self.max_sample_rate.unwrap_or(DEFAULT_MAX_SAMPLE_RATE)
    }
}

/// Format summary used for both ends of the path.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct StreamFormat {
    /// Display name, e.g. `FLAC`, `WavPack`, `DSD64`.
    pub name: String,
    pub codec: Option<String>,
    pub container: Option<String>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u8>,
    pub channels: Option<u8>,
    pub bitrate_kbps: Option<u32>,
    pub lossless: Option<bool>,
}

/// One processing stage of the server leg.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SignalStep {
    /// `source`, `passthrough`, `decode`, `dsd_to_pcm`, `dop`, `resample`,
    /// `requantize`, `downmix`, `effect`, `encode`.
    pub kind: &'static str,
    pub description: String,
    /// `bit_perfect`, `lossless`, `high`, `enhanced` or `lossy`.
    pub quality: &'static str,
}

/// How the server delivers a track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delivery {
    Original,
    Pcm {
        codec: PcmCodec,
        sample_rate: u32,
        bit_depth: u8,
        downmix_to: Option<u8>,
    },
    Dop {
        sample_rate: u32,
    },
    Mp3 {
        bitrate: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcmCodec {
    Flac,
    Wav,
}

/// The decision plus the explanation the signal path shows.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct DeliveryPlan {
    /// `original`, `lossless_conversion`, `dop` or `lossy_transcode`.
    pub mode: &'static str,
    pub content_type: String,
    pub source: StreamFormat,
    pub output: StreamFormat,
    pub steps: Vec<SignalStep>,
    /// `bit_perfect`, `lossless`, `high_quality` or `lossy`.
    pub quality: &'static str,
    /// The audio samples (or DSD bits) leave the server unchanged.
    pub bit_perfect: bool,
    /// Track boundaries are sample accurate (no encoder padding added).
    pub gapless: bool,
    /// Why this delivery was chosen, in plain words.
    pub reason: String,
    #[serde(skip)]
    pub delivery: Delivery,
}

/// What the server knows about the source file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceFile {
    pub extension: String,
    pub audio: AudioProperties,
}

impl SourceFile {
    pub fn new(path: &str, audio: Option<&AudioProperties>) -> Self {
        let extension = std::path::Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        Self {
            extension,
            audio: audio.cloned().unwrap_or_default(),
        }
    }

    fn codec(&self) -> String {
        self.audio.codec.clone().unwrap_or_else(|| {
            match self.extension.as_str() {
                "flac" => "flac",
                "alac" => "alac",
                "mp3" => "mp3",
                "aac" => "aac",
                "opus" => "opus",
                "ogg" | "oga" => "vorbis",
                "wav" | "aiff" | "aif" | "w64" => "pcm",
                "wv" => "wavpack",
                "ape" => "ape",
                "tak" => "tak",
                "tta" => "tta",
                "dsf" | "dff" => "dsd",
                "mpc" => "musepack",
                "wma" => "wma",
                _ => "unknown",
            }
            .to_string()
        })
    }

    fn is_dsd(&self) -> bool {
        matches!(self.codec().as_str(), "dsd" | "dst")
    }

    fn lossless(&self) -> Option<bool> {
        self.audio
            .lossless
            .or_else(|| crate::domain::codec_is_lossless(&self.codec()))
            .or_else(|| format_for_extension(&self.extension).and_then(|format| format.lossless))
    }

    /// The token a player must accept to receive the original bytes.
    fn token(&self) -> Option<&'static str> {
        let container = self
            .audio
            .container
            .as_deref()
            .unwrap_or(match self.extension.as_str() {
                "m4a" | "alac" => "mp4",
                "ogg" | "oga" | "opus" => "ogg",
                "aif" | "aifc" => "aiff",
                other => other,
            });
        Some(match (self.codec().as_str(), container) {
            ("flac", "flac") => "flac",
            ("alac", "mp4") => "alac",
            ("aac", "mp4") | ("aac", "aac") => "aac",
            ("mp3", "mp3") => "mp3",
            ("vorbis", "ogg") => "vorbis",
            ("opus", "ogg") => "opus",
            ("pcm", "wav") => "wav",
            ("pcm_float", "wav") => "wav_float",
            ("pcm", "aiff") => "aiff",
            ("wavpack", _) => "wavpack",
            ("ape", _) => "ape",
            ("dsd", "dsf") => "dsf",
            ("dsd", "dff") | ("dst", "dff") => "dff",
            _ => return None,
        })
    }

    fn format(&self) -> StreamFormat {
        let codec = self.codec();
        let name = if self.is_dsd() {
            self.audio
                .sample_rate
                .and_then(dsd_rate_name)
                .unwrap_or_else(|| "DSD".into())
        } else {
            codec_display_name(&codec, &self.extension)
        };
        StreamFormat {
            name,
            codec: Some(codec),
            container: self.audio.container.clone(),
            sample_rate: self.audio.sample_rate,
            bit_depth: self.audio.bit_depth,
            channels: self.audio.channels,
            bitrate_kbps: self.audio.bitrate_kbps,
            lossless: self.lossless(),
        }
    }
}

/// Display name for a codec.
pub fn codec_display_name(codec: &str, extension: &str) -> String {
    match codec {
        "flac" => "FLAC",
        "alac" => "ALAC",
        "pcm" | "pcm_float" => match extension {
            "aiff" | "aif" | "aifc" => "AIFF",
            "w64" => "Wave64",
            "caf" => "CAF",
            _ => "WAV",
        },
        "wavpack" => "WavPack",
        "ape" => "Monkey's Audio",
        "tak" => "TAK",
        "tta" => "True Audio",
        "mp3" => "MP3",
        "aac" => "AAC",
        "vorbis" => "Ogg Vorbis",
        "opus" => "Opus",
        "musepack" => "Musepack",
        "wma" => "WMA",
        "wma_pro" => "WMA Pro",
        "wma_lossless" => "WMA Lossless",
        "dsd" | "dst" => "DSD",
        "speex" => "Speex",
        "ac3" => "Dolby Digital",
        "eac3" => "Dolby Digital Plus",
        "truehd" => "Dolby TrueHD",
        "dts" => "DTS",
        other => return other.to_ascii_uppercase(),
    }
    .to_string()
}

fn khz(rate: u32) -> String {
    if rate.is_multiple_of(1000) {
        format!("{} kHz", rate / 1000)
    } else {
        format!("{:.1} kHz", f64::from(rate) / 1000.0)
    }
}

fn channel_name(channels: u8) -> String {
    match channels {
        1 => "mono".into(),
        2 => "stereo".into(),
        3 => "3.0".into(),
        4 => "4.0".into(),
        5 => "5.0".into(),
        6 => "5.1".into(),
        7 => "6.1".into(),
        8 => "7.1".into(),
        other => format!("{other} channels"),
    }
}

/// Fits a PCM rate under a player's limit, staying in the same rate family
/// (352.8 kHz becomes 176.4 kHz, never 192 kHz).
pub fn fit_sample_rate(rate: u32, max: u32) -> u32 {
    let mut fitted = rate;
    while fitted > max && fitted.is_multiple_of(2) && fitted / 2 >= 8_000 {
        fitted /= 2;
    }
    if fitted > max { max } else { fitted }
}

fn describe_pcm(sample_rate: u32, bit_depth: u8, channels: Option<u8>) -> String {
    let mut text = format!("{bit_depth}-bit / {}", khz(sample_rate));
    if let Some(channels) = channels {
        text.push_str(", ");
        text.push_str(&channel_name(channels));
    }
    text
}

fn source_description(source: &StreamFormat) -> String {
    let mut parts = vec![source.name.clone()];
    if source
        .codec
        .as_deref()
        .is_some_and(|codec| codec == "dsd" || codec == "dst")
    {
        if let Some(rate) = source.sample_rate {
            parts.push(format!("{:.4} MHz", f64::from(rate) / 1_000_000.0));
        }
    } else {
        match (source.bit_depth, source.sample_rate) {
            (Some(depth), Some(rate)) => parts.push(format!("{depth}-bit / {}", khz(rate))),
            (None, Some(rate)) => parts.push(khz(rate)),
            _ => {}
        }
        if source.lossless == Some(false)
            && let Some(kbps) = source.bitrate_kbps
        {
            parts.push(format!("{kbps} kbps"));
        }
    }
    if let Some(channels) = source.channels {
        parts.push(channel_name(channels));
    }
    parts.join(", ")
}

/// Plans delivery of `source` to a player.
///
/// `bitrate` is the listener's lossy streaming preference (0 = best quality);
/// `effect` names a server-side effect that requires re-encoding.
pub fn plan_delivery(
    source: &SourceFile,
    capabilities: &OutputCapabilities,
    bitrate: u32,
    effect: Option<&str>,
) -> DeliveryPlan {
    let source_format = source.format();
    let mut steps = vec![SignalStep {
        kind: "source",
        description: source_description(&source_format),
        quality: match source_format.lossless {
            Some(true) => "lossless",
            Some(false) => "lossy",
            None => "high",
        },
    }];

    if bitrate > 0 || effect.is_some() {
        let bitrate = if effect.is_some() { 192 } else { bitrate };
        steps.push(SignalStep {
            kind: "decode",
            description: format!("Decode {}", source_format.name),
            quality: "lossless",
        });
        if let Some(effect) = effect {
            steps.push(SignalStep {
                kind: "effect",
                description: format!("Apply the {} effect", effect.replace('_', " ")),
                quality: "enhanced",
            });
        }
        steps.push(SignalStep {
            kind: "encode",
            description: format!("Encode MP3 {bitrate} kbps"),
            quality: "lossy",
        });
        return DeliveryPlan {
            mode: "lossy_transcode",
            content_type: "audio/mpeg".into(),
            output: StreamFormat {
                name: "MP3".into(),
                codec: Some("mp3".into()),
                container: Some("mp3".into()),
                sample_rate: None,
                bit_depth: None,
                channels: None,
                bitrate_kbps: Some(bitrate),
                lossless: Some(false),
            },
            source: source_format,
            steps,
            quality: "lossy",
            bit_perfect: false,
            gapless: false,
            reason: if effect.is_some() {
                "An effect is on, so the server renders it into the stream.".into()
            } else {
                format!("Streaming quality is set to {bitrate} kbps.")
            },
            delivery: Delivery::Mp3 { bitrate },
        };
    }

    let token = source.token();
    let within_limits = source_format
        .sample_rate
        .is_none_or(|rate| source.is_dsd() || rate <= capabilities.max_rate())
        && source_format.bit_depth.is_none_or(|depth| {
            source.is_dsd() || capabilities.max_bit_depth.is_none_or(|max| depth <= max)
        })
        && source_format
            .channels
            .is_none_or(|channels| capabilities.max_channels.is_none_or(|max| channels <= max));
    let legacy = capabilities.accept.is_empty();
    let dsd_native = !source.is_dsd() || capabilities.dsd == DsdOutput::Native;
    if legacy
        || (token.is_some_and(|token| capabilities.accepts(token)) && within_limits && dsd_native)
    {
        steps.push(SignalStep {
            kind: "passthrough",
            description: "Original file bytes, untouched".into(),
            quality: if source_format.lossless == Some(false) {
                "lossy"
            } else {
                "bit_perfect"
            },
        });
        let lossless = source_format.lossless != Some(false);
        return DeliveryPlan {
            mode: "original",
            content_type: format_for_extension(&source.extension)
                .map(|format| format.content_type)
                .unwrap_or("application/octet-stream")
                .into(),
            output: source_format.clone(),
            reason: if legacy {
                "Original file requested.".into()
            } else {
                format!("This player decodes {} natively.", source_format.name)
            },
            source: source_format,
            steps,
            quality: if lossless { "bit_perfect" } else { "lossy" },
            bit_perfect: true,
            gapless: true,
            delivery: Delivery::Original,
        };
    }

    let channels = source_format.channels;
    let downmix_to = match (channels, capabilities.max_channels) {
        (Some(channels), Some(max)) if channels > max => Some(max),
        _ => None,
    };

    if source.is_dsd()
        && capabilities.dsd == DsdOutput::Dop
        && source_format.codec.as_deref() == Some("dsd")
        && downmix_to.is_none()
        && let Some(rate) = source_format.sample_rate
        && rate / 16 <= capabilities.max_rate()
    {
        let sample_rate = rate / 16;
        steps.push(SignalStep {
            kind: "dop",
            description: format!(
                "Pack {} into DoP v1.1 (24-bit / {}), every DSD bit unchanged",
                source_format.name,
                khz(sample_rate)
            ),
            quality: "bit_perfect",
        });
        return DeliveryPlan {
            mode: "dop",
            content_type: "audio/wav".into(),
            output: StreamFormat {
                name: format!("DoP ({})", source_format.name),
                codec: Some("pcm".into()),
                container: Some("wav".into()),
                sample_rate: Some(sample_rate),
                bit_depth: Some(24),
                channels,
                bitrate_kbps: channels.map(|c| sample_rate * 24 * u32::from(c) / 1000),
                lossless: Some(true),
            },
            source: source_format,
            steps,
            quality: "bit_perfect",
            bit_perfect: true,
            gapless: true,
            reason: "DoP output requested: the DAC receives the original DSD stream.".into(),
            delivery: Delivery::Dop { sample_rate },
        };
    }

    let codec = if capabilities.accepts("flac") || legacy {
        Some(PcmCodec::Flac)
    } else if capabilities.accepts("wav") {
        Some(PcmCodec::Wav)
    } else {
        None
    };
    let Some(codec) = codec else {
        steps.push(SignalStep {
            kind: "decode",
            description: format!("Decode {}", source_format.name),
            quality: "lossless",
        });
        steps.push(SignalStep {
            kind: "encode",
            description: "Encode MP3 320 kbps".into(),
            quality: "lossy",
        });
        return DeliveryPlan {
            mode: "lossy_transcode",
            content_type: "audio/mpeg".into(),
            output: StreamFormat {
                name: "MP3".into(),
                codec: Some("mp3".into()),
                container: Some("mp3".into()),
                bitrate_kbps: Some(320),
                lossless: Some(false),
                ..StreamFormat::default()
            },
            source: source_format,
            steps,
            quality: "lossy",
            bit_perfect: false,
            gapless: false,
            reason: "This player decodes neither the original format nor FLAC or WAV.".into(),
            delivery: Delivery::Mp3 { bitrate: 320 },
        };
    };

    let max_rate = capabilities.max_rate();
    let max_depth = capabilities
        .max_bit_depth
        .unwrap_or(MAX_OUTPUT_BIT_DEPTH)
        .clamp(16, MAX_OUTPUT_BIT_DEPTH);
    let source_lossless = source_format.lossless != Some(false);
    let (sample_rate, bit_depth) = if source.is_dsd() {
        let rate = source_format.sample_rate.unwrap_or(2_822_400) / 16;
        (fit_sample_rate(rate, max_rate), max_depth)
    } else {
        let rate = source_format.sample_rate.unwrap_or(44_100);
        let depth = if source_lossless {
            source_format.bit_depth.unwrap_or(16).clamp(16, max_depth)
        } else {
            // Keep the decoder's full precision; never add a second generation of loss.
            max_depth
        };
        (fit_sample_rate(rate, max_rate), depth)
    };

    steps.push(SignalStep {
        kind: "decode",
        description: format!("Decode {}", source_format.name),
        quality: "lossless",
    });
    let mut altered = false;
    if source.is_dsd() {
        altered = true;
        steps.push(SignalStep {
            kind: "dsd_to_pcm",
            description: format!(
                "Convert {} to {bit_depth}-bit / {} PCM (SoX VHQ)",
                source_format.name,
                khz(sample_rate)
            ),
            quality: "high",
        });
    } else if source_format
        .sample_rate
        .is_some_and(|rate| rate != sample_rate)
    {
        altered = true;
        steps.push(SignalStep {
            kind: "resample",
            description: format!(
                "Resample {} → {} (SoX VHQ), the player's limit is {}",
                khz(source_format.sample_rate.unwrap_or_default()),
                khz(sample_rate),
                khz(max_rate)
            ),
            quality: "high",
        });
    }
    if !source.is_dsd()
        && source_lossless
        && let Some(depth) = source_format.bit_depth
        && depth > bit_depth
    {
        altered = true;
        steps.push(SignalStep {
            kind: "requantize",
            description: format!("Reduce {depth}-bit → {bit_depth}-bit with triangular dither"),
            quality: "high",
        });
    }
    if let (Some(from), Some(to)) = (channels, downmix_to) {
        altered = true;
        steps.push(SignalStep {
            kind: "downmix",
            description: format!(
                "Downmix {} → {} (this player is limited to {to} channels)",
                channel_name(from),
                channel_name(to)
            ),
            quality: "high",
        });
    }
    let output_channels = downmix_to.or(channels);
    let container = match codec {
        PcmCodec::Flac => "FLAC",
        PcmCodec::Wav => "WAV",
    };
    steps.push(SignalStep {
        kind: "encode",
        description: format!(
            "Encode lossless {container} {}",
            describe_pcm(sample_rate, bit_depth, output_channels)
        ),
        quality: "lossless",
    });
    let quality = if !source_lossless {
        "lossy"
    } else if altered {
        "high_quality"
    } else {
        "lossless"
    };
    let reason = match token {
        Some(_) if !within_limits => format!(
            "{} exceeds what this player accepts, so it is converted.",
            source_format.name
        ),
        _ if source.is_dsd() => {
            "This player cannot decode DSD, so it is converted to high-resolution PCM.".into()
        }
        _ => format!(
            "This player cannot decode {}, so it is converted {}.",
            source_format.name,
            if source_lossless {
                "losslessly"
            } else {
                "without adding loss"
            }
        ),
    };
    DeliveryPlan {
        mode: "lossless_conversion",
        content_type: match codec {
            PcmCodec::Flac => "audio/flac",
            PcmCodec::Wav => "audio/wav",
        }
        .into(),
        output: StreamFormat {
            name: container.into(),
            codec: Some(
                match codec {
                    PcmCodec::Flac => "flac",
                    PcmCodec::Wav => "pcm",
                }
                .into(),
            ),
            container: Some(
                match codec {
                    PcmCodec::Flac => "flac",
                    PcmCodec::Wav => "wav",
                }
                .into(),
            ),
            sample_rate: Some(sample_rate),
            bit_depth: Some(bit_depth),
            channels: output_channels,
            bitrate_kbps: None,
            lossless: Some(true),
        },
        source: source_format,
        steps,
        quality,
        bit_perfect: quality == "lossless",
        gapless: true,
        reason,
        delivery: Delivery::Pcm {
            codec,
            sample_rate,
            bit_depth,
            downmix_to,
        },
    }
}

/// Stable identity of a conversion's output, for the transcode cache.
pub fn conversion_cache_name(delivery: &Delivery) -> Option<String> {
    match delivery {
        Delivery::Pcm {
            codec,
            sample_rate,
            bit_depth,
            downmix_to,
        } => Some(format!(
            "{}-{sample_rate}-{bit_depth}-{}-{CONVERSION_REVISION}",
            match codec {
                PcmCodec::Flac => "flac",
                PcmCodec::Wav => "wav",
            },
            downmix_to.map_or_else(|| "native".to_string(), |channels| format!("{channels}ch")),
        )),
        _ => None,
    }
}

/// ffmpeg output arguments for a PCM conversion (after `-i <input>`).
pub fn conversion_ffmpeg_args(source: &SourceFile, delivery: &Delivery) -> Option<Vec<String>> {
    let Delivery::Pcm {
        codec,
        sample_rate,
        bit_depth,
        downmix_to,
    } = delivery
    else {
        return None;
    };
    let mut args: Vec<String> = ["-map", "0:a:0", "-map_metadata", "-1", "-vn", "-sn"]
        .into_iter()
        .map(String::from)
        .collect();
    let source_rate = source.audio.sample_rate;
    let reduces_depth = source
        .audio
        .bit_depth
        .is_some_and(|depth| depth > *bit_depth && !source.is_dsd());
    if source.is_dsd() || source_rate != Some(*sample_rate) || reduces_depth {
        let mut filter = format!("aresample=resampler=soxr:precision=28:osr={sample_rate}");
        if reduces_depth || source.is_dsd() || source.lossless() == Some(false) {
            filter.push_str(":dither_method=triangular");
        }
        args.push("-af".into());
        args.push(filter);
    }
    if let Some(channels) = downmix_to {
        args.push("-ac".into());
        args.push(channels.to_string());
    }
    let sample_format = if *bit_depth <= 16 { "s16" } else { "s32" };
    match codec {
        PcmCodec::Flac => {
            args.extend(
                [
                    "-c:a",
                    "flac",
                    "-sample_fmt",
                    sample_format,
                    "-compression_level",
                    "5",
                    "-f",
                    "flac",
                ]
                .into_iter()
                .map(String::from),
            );
            if *bit_depth > 16 {
                args.push("-bits_per_raw_sample".into());
                args.push(bit_depth.to_string());
            }
        }
        PcmCodec::Wav => {
            args.extend(
                [
                    "-c:a",
                    if *bit_depth <= 16 {
                        "pcm_s16le"
                    } else {
                        "pcm_s24le"
                    },
                    "-f",
                    "wav",
                ]
                .into_iter()
                .map(String::from),
            );
        }
    }
    Some(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio(
        codec: &str,
        container: &str,
        rate: u32,
        depth: Option<u8>,
        channels: u8,
    ) -> AudioProperties {
        AudioProperties {
            codec: Some(codec.into()),
            container: Some(container.into()),
            sample_rate: Some(rate),
            bit_depth: depth,
            channels: Some(channels),
            bitrate_kbps: Some(900),
            lossless: crate::domain::codec_is_lossless(codec),
        }
    }

    fn browser() -> OutputCapabilities {
        OutputCapabilities::from_query(
            Some("flac,mp3,aac,vorbis,opus,wav"),
            Some(192_000),
            None,
            None,
            None,
        )
    }

    #[test]
    fn natively_decoded_formats_stream_original_bytes() {
        let source = SourceFile::new(
            "/m/a.flac",
            Some(&audio("flac", "flac", 96_000, Some(24), 2)),
        );
        let plan = plan_delivery(&source, &browser(), 0, None);
        assert_eq!(plan.mode, "original");
        assert_eq!(plan.delivery, Delivery::Original);
        assert_eq!(plan.quality, "bit_perfect");
        assert!(plan.bit_perfect && plan.gapless);
        assert_eq!(plan.content_type, "audio/flac");
        assert_eq!(plan.steps.last().unwrap().kind, "passthrough");
        assert!(plan.steps[0].description.contains("24-bit / 96 kHz"));
    }

    #[test]
    fn unsupported_lossless_formats_convert_to_flac_at_full_resolution() {
        for (path, codec, container) in [
            ("/m/a.wv", "wavpack", "wavpack"),
            ("/m/a.ape", "ape", "ape"),
            ("/m/a.tak", "tak", "tak"),
            ("/m/a.m4a", "alac", "mp4"),
            ("/m/a.aiff", "pcm", "aiff"),
            ("/m/a.wma", "wma_lossless", "asf"),
        ] {
            let source = SourceFile::new(path, Some(&audio(codec, container, 96_000, Some(24), 2)));
            let plan = plan_delivery(&source, &browser(), 0, None);
            assert_eq!(plan.mode, "lossless_conversion", "{path}");
            assert_eq!(plan.quality, "lossless", "{path}");
            assert!(plan.bit_perfect, "{path}");
            assert_eq!(
                plan.delivery,
                Delivery::Pcm {
                    codec: PcmCodec::Flac,
                    sample_rate: 96_000,
                    bit_depth: 24,
                    downmix_to: None
                },
                "{path}"
            );
            let args = conversion_ffmpeg_args(&source, &plan.delivery).unwrap();
            assert!(!args.iter().any(|arg| arg == "-af"), "{path}: {args:?}");
            assert!(args.windows(2).any(|pair| pair == ["-c:a", "flac"]));
        }
    }

    #[test]
    fn dsd_becomes_24_bit_pcm_at_the_dsd_rate_over_16() {
        let dsd64 = SourceFile::new(
            "/m/a.dsf",
            Some(&audio("dsd", "dsf", 2_822_400, Some(1), 2)),
        );
        let plan = plan_delivery(&dsd64, &browser(), 0, None);
        assert_eq!(plan.source.name, "DSD64");
        assert_eq!(
            plan.delivery,
            Delivery::Pcm {
                codec: PcmCodec::Flac,
                sample_rate: 176_400,
                bit_depth: 24,
                downmix_to: None
            }
        );
        assert_eq!(plan.quality, "high_quality");
        assert!(plan.steps.iter().any(|step| step.kind == "dsd_to_pcm"));
        let args = conversion_ffmpeg_args(&dsd64, &plan.delivery).unwrap();
        assert!(args.iter().any(|arg| arg.contains("osr=176400")));

        let dsd256 = SourceFile::new(
            "/m/a.dff",
            Some(&audio("dsd", "dff", 11_289_600, Some(1), 2)),
        );
        let limited = OutputCapabilities::from_query(Some("flac"), Some(96_000), None, None, None);
        let plan = plan_delivery(&dsd256, &limited, 0, None);
        assert_eq!(plan.source.name, "DSD256");
        assert!(matches!(
            plan.delivery,
            Delivery::Pcm {
                sample_rate: 88_200,
                ..
            }
        ));
    }

    #[test]
    fn dop_and_native_dsd_are_offered_when_requested() {
        let dsd = SourceFile::new(
            "/m/a.dsf",
            Some(&audio("dsd", "dsf", 2_822_400, Some(1), 2)),
        );
        let mut caps = browser();
        caps.dsd = DsdOutput::Dop;
        let plan = plan_delivery(&dsd, &caps, 0, None);
        assert_eq!(plan.mode, "dop");
        assert_eq!(
            plan.delivery,
            Delivery::Dop {
                sample_rate: 176_400
            }
        );
        assert!(plan.bit_perfect);

        let dst = SourceFile::new(
            "/m/a.dff",
            Some(&audio("dst", "dff", 2_822_400, Some(1), 2)),
        );
        assert_eq!(
            plan_delivery(&dst, &caps, 0, None).mode,
            "lossless_conversion"
        );

        let native =
            OutputCapabilities::from_query(Some("dsf,flac"), None, None, None, Some("native"));
        assert_eq!(plan_delivery(&dsd, &native, 0, None).mode, "original");
        // Accepting the container is not enough without asking for native DSD.
        let pcm = OutputCapabilities::from_query(Some("dsf,flac"), None, None, None, None);
        assert_eq!(
            plan_delivery(&dsd, &pcm, 0, None).mode,
            "lossless_conversion"
        );
    }

    #[test]
    fn limits_resample_within_the_rate_family_and_dither_down() {
        let source = SourceFile::new(
            "/m/a.flac",
            Some(&audio("flac", "flac", 352_800, Some(32), 2)),
        );
        let plan = plan_delivery(&source, &browser(), 0, None);
        assert_eq!(
            plan.delivery,
            Delivery::Pcm {
                codec: PcmCodec::Flac,
                sample_rate: 176_400,
                bit_depth: 24,
                downmix_to: None
            }
        );
        assert!(plan.steps.iter().any(|step| step.kind == "resample"));
        assert!(plan.steps.iter().any(|step| step.kind == "requantize"));
        assert!(!plan.bit_perfect);
        let args = conversion_ffmpeg_args(&source, &plan.delivery).unwrap();
        assert!(
            args.iter()
                .any(|arg| arg.contains("dither_method=triangular"))
        );
        assert_eq!(fit_sample_rate(384_000, 192_000), 192_000);
        assert_eq!(fit_sample_rate(705_600, 96_000), 88_200);
        assert_eq!(fit_sample_rate(44_100, 192_000), 44_100);
    }

    #[test]
    fn multichannel_is_kept_unless_the_player_is_limited() {
        let source = SourceFile::new(
            "/m/a.wv",
            Some(&audio("wavpack", "wavpack", 48_000, Some(24), 6)),
        );
        let plan = plan_delivery(&source, &browser(), 0, None);
        assert_eq!(plan.output.channels, Some(6));
        assert!(plan.steps.last().unwrap().description.contains("5.1"));

        let cast = plan_delivery(&source, &OutputCapabilities::google_cast(), 0, None);
        assert!(matches!(
            cast.delivery,
            Delivery::Pcm {
                downmix_to: Some(2),
                ..
            }
        ));
        let args = conversion_ffmpeg_args(&source, &cast.delivery).unwrap();
        assert!(args.windows(2).any(|pair| pair == ["-ac", "2"]));
        // A multichannel FLAC exceeds the Cast channel limit and is converted too.
        let flac = SourceFile::new(
            "/m/a.flac",
            Some(&audio("flac", "flac", 48_000, Some(24), 6)),
        );
        assert_eq!(
            plan_delivery(&flac, &OutputCapabilities::google_cast(), 0, None).mode,
            "lossless_conversion"
        );
    }

    #[test]
    fn lossy_sources_are_decoded_without_a_second_generation_of_loss() {
        let wma = SourceFile::new("/m/a.wma", Some(&audio("wma", "asf", 44_100, None, 2)));
        let plan = plan_delivery(&wma, &browser(), 0, None);
        assert_eq!(plan.mode, "lossless_conversion");
        assert_eq!(plan.quality, "lossy");
        assert!(matches!(plan.delivery, Delivery::Pcm { bit_depth: 24, .. }));
        let mp3 = SourceFile::new("/m/a.mp3", Some(&audio("mp3", "mp3", 44_100, None, 2)));
        let plan = plan_delivery(&mp3, &browser(), 0, None);
        assert_eq!(plan.mode, "original");
        assert_eq!(plan.quality, "lossy");
    }

    #[test]
    fn listener_bitrates_effects_and_legacy_clients_keep_their_behaviour() {
        let source = SourceFile::new(
            "/m/a.flac",
            Some(&audio("flac", "flac", 44_100, Some(16), 2)),
        );
        let plan = plan_delivery(&source, &browser(), 128, None);
        assert_eq!(plan.delivery, Delivery::Mp3 { bitrate: 128 });
        let plan = plan_delivery(&source, &browser(), 0, Some("slowed_reverb"));
        assert_eq!(plan.delivery, Delivery::Mp3 { bitrate: 192 });
        assert!(plan.steps.iter().any(|step| step.kind == "effect"));
        let dsd = SourceFile::new(
            "/m/a.dsf",
            Some(&audio("dsd", "dsf", 2_822_400, Some(1), 2)),
        );
        let legacy = plan_delivery(&dsd, &OutputCapabilities::default(), 0, None);
        assert_eq!(legacy.mode, "original");
        let mp3_only = OutputCapabilities::from_query(Some("mp3"), None, None, None, None);
        assert_eq!(
            plan_delivery(&dsd, &mp3_only, 0, None).delivery,
            Delivery::Mp3 { bitrate: 320 }
        );
    }

    #[test]
    fn conversions_have_distinct_cache_names() {
        let a = conversion_cache_name(&Delivery::Pcm {
            codec: PcmCodec::Flac,
            sample_rate: 176_400,
            bit_depth: 24,
            downmix_to: None,
        });
        let b = conversion_cache_name(&Delivery::Pcm {
            codec: PcmCodec::Flac,
            sample_rate: 88_200,
            bit_depth: 24,
            downmix_to: None,
        });
        assert_ne!(a, b);
        assert_eq!(conversion_cache_name(&Delivery::Original), None);
    }
}
