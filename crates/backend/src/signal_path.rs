//! Server leg of the playback signal path.
//!
//! Describes what happens to a track between the file on disk and the bytes
//! the server sends: the source format, any requested DSP, conversions the
//! encoder forces, the lossy encode and the HTTP transport. Clients append
//! their own leg (decoder, Web Audio graph, volume, output device) and show
//! the combined path behind a quality light, following Roon's colours:
//! lossless (purple), enhanced (blue), high quality (green), low quality
//! (yellow).
//!
//! Everything here is pure so the classification can be unit tested without
//! a library, ffmpeg or HTTP.

use serde::{Deserialize, Serialize};

use crate::domain::{AudioProperties, codec_is_lossless};

/// Quality of one stage, or of a whole path (the worst stage wins).
///
/// Ordered worst to best so `min` picks the overall quality.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SignalQuality {
    /// A lossy format is involved (yellow).
    LowQuality,
    /// Unrequested but high-fidelity changes: compatibility conversions,
    /// software volume, an OS mixer (green).
    HighQuality,
    /// Processing the listener asked for, such as DSP or leveling (blue).
    Enhanced,
    /// Bits arrive unmodified (purple).
    Lossless,
}

/// Broad stage category, used by clients to choose an icon.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StageKind {
    Source,
    Dsp,
    Conversion,
    Encode,
    Transport,
    Decode,
    Output,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct SignalStage {
    /// Stable key, e.g. `source`, `dsp`, `resample`, `downmix`, `encode`.
    pub id: &'static str,
    pub kind: StageKind,
    pub title: String,
    pub detail: String,
    /// `None` when the stage cannot be classified (unknown codec).
    pub quality: Option<SignalQuality>,
    /// Plain-language explanation shown when the stage is selected.
    pub explanation: String,
}

/// Format at one end of the server leg.
#[derive(Serialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct SignalFormat {
    pub codec: Option<String>,
    pub container: Option<String>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u8>,
    pub channels: Option<u8>,
    pub bitrate_kbps: Option<u32>,
    pub lossless: Option<bool>,
    /// True for lossless audio above CD resolution (more than 16 bit or
    /// 48 kHz).
    pub high_res: bool,
    /// Short label, e.g. `FLAC 96kHz 24bit 2ch`.
    pub summary: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StreamTarget {
    /// A Parson client playing the stream URL itself.
    Direct,
    /// A Google Cast receiver fed by the server.
    Cast,
    /// A device attached to the server, played by the server itself
    /// through its native output (ALSA hw, WASAPI, CoreAudio).
    Native,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    /// The file's own bytes.
    Original,
    /// An ffmpeg MP3 encode.
    Transcode,
    /// Decoded on the server and written to a local device as PCM.
    Native,
}

/// A server-side DSP effect as the signal path presents it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DspEffect {
    pub id: &'static str,
    pub label: &'static str,
    /// What the effect does, e.g. `Echo-based reverb, 20% wet`.
    pub detail: &'static str,
    /// Sample rate the effect chain outputs, when it fixes one.
    pub output_sample_rate: Option<u32>,
}

/// What the client asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerLegRequest {
    /// Requested MP3 bitrate in kbps; 0 asks for the original file.
    pub bitrate_kbps: u32,
    pub effect: Option<DspEffect>,
    pub target: StreamTarget,
}

/// The server's own DSP chain on a native output, and how it is running.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SignalProcessing {
    /// e.g. `Parson DSP, 64-bit float`.
    pub engine: String,
    /// Stage ids the chain runs, in order.
    pub stages: Vec<&'static str>,
    /// Things the chain could not do as asked, in plain language.
    pub notes: Vec<String>,
    /// Live measurements while it plays; `None` when only described.
    pub status: Option<parson_output::dsp::DspStatus>,
}

pub const SERVER_DSP_ENGINE: &str = "Parson DSP, 64-bit float";

/// Browsers and phones cannot decode DSD, so the "original" stream of a
/// DSF or DSDIFF file is converted to PCM: 24-bit FLAC at 88.2 kHz, as
/// Roon's "Conversions for Compatibility" does for a DAC without DSD.
pub const DSD_COMPAT_RATE: u32 = 88_200;
pub const DSD_COMPAT_BITS: u8 = 24;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ServerSignalPath {
    pub song_id: String,
    pub target: StreamTarget,
    pub mode: DeliveryMode,
    /// Worst classified stage of the server leg.
    pub quality: SignalQuality,
    pub source: SignalFormat,
    pub delivered: SignalFormat,
    /// MIME type of the response body.
    pub content_type: String,
    pub requested_bitrate_kbps: u32,
    pub effect: Option<&'static str>,
    pub stages: Vec<SignalStage>,
    /// The server-side DSP chain, on a native output that processes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processing: Option<SignalProcessing>,
}

/// Bitrate the stream is transcoded at, or `None` for the original bytes.
///
/// Server DSP always renders through a 192 kbps MP3 encode.
pub fn transcode_bitrate(bitrate_kbps: u32, effect_active: bool) -> Option<u32> {
    if effect_active {
        Some(192)
    } else if bitrate_kbps > 0 {
        Some(bitrate_kbps)
    } else {
        None
    }
}

/// Whether a Cast receiver gets the file as-is (by extension).
pub fn cast_plays_directly(extension: Option<&str>) -> bool {
    extension
        .map(str::to_ascii_lowercase)
        .is_some_and(|extension| matches!(extension.as_str(), "mp3" | "aac" | "m4a" | "mp4"))
}

/// Bitrate a Cast receiver is transcoded at when it cannot take the file.
pub const CAST_TRANSCODE_BITRATE: u32 = 192;

/// Sample rates the MP3 encoder accepts.
const MP3_SAMPLE_RATES: [u32; 9] = [
    8_000, 11_025, 12_000, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000,
];

/// Sample rate ffmpeg picks for an MP3 encode of `rate`: the rate itself
/// when supported, 48 kHz above that, otherwise the closest supported rate.
pub fn mp3_output_rate(rate: u32) -> u32 {
    if MP3_SAMPLE_RATES.contains(&rate) {
        return rate;
    }
    if rate > 48_000 {
        return 48_000;
    }
    MP3_SAMPLE_RATES
        .iter()
        .copied()
        .min_by_key(|candidate| candidate.abs_diff(rate))
        .unwrap_or(44_100)
}

/// `DSD64 2.8MHz`, for a one-bit DSD rate.
pub fn format_dsd_rate(rate: u32) -> String {
    format!(
        "{} {:.1}MHz",
        parson_output::dsd_rate_label(rate),
        f64::from(rate) / 1_000_000.0
    )
}

fn is_dsd(format: &SignalFormat) -> bool {
    format.codec.as_deref() == Some("dsd")
}

/// `44.1kHz`, `96kHz`, `352.8kHz`.
pub fn format_rate(rate: u32) -> String {
    let khz = f64::from(rate) / 1000.0;
    if (khz - khz.round()).abs() < 1e-9 {
        format!("{}kHz", khz.round() as u64)
    } else {
        let text = format!("{khz:.1}");
        format!("{text}kHz")
    }
}

fn codec_label(codec: &str) -> String {
    match codec {
        "flac" => "FLAC".into(),
        "alac" => "ALAC".into(),
        "aac" => "AAC".into(),
        "mp3" => "MP3".into(),
        "vorbis" => "Ogg Vorbis".into(),
        "opus" => "Opus".into(),
        "pcm" => "PCM".into(),
        "pcm_float" => "PCM float".into(),
        "wavpack" => "WavPack".into(),
        "ape" => "Monkey's Audio".into(),
        "dsd" => "DSD".into(),
        other => other.to_ascii_uppercase(),
    }
}

fn container_label(container: &str) -> String {
    match container {
        "mp4" => "MP4".into(),
        "wav" => "WAV".into(),
        "aiff" => "AIFF".into(),
        "ogg" => "Ogg".into(),
        "dff" => "DSDIFF".into(),
        "native" => "Native".into(),
        "DoP" => "DoP".into(),
        other => other.to_ascii_uppercase(),
    }
}

/// Codec guessed from a file extension when the indexer has no properties.
fn codec_from_extension(extension: &str) -> Option<&'static str> {
    match extension.to_ascii_lowercase().as_str() {
        "flac" => Some("flac"),
        "mp3" => Some("mp3"),
        "wav" | "aif" | "aiff" | "aifc" => Some("pcm"),
        "ogg" | "oga" => Some("vorbis"),
        "opus" => Some("opus"),
        "aac" => Some("aac"),
        "wv" => Some("wavpack"),
        "ape" => Some("ape"),
        "dsf" | "dff" => Some("dsd"),
        _ => None,
    }
}

fn is_high_res(lossless: Option<bool>, sample_rate: Option<u32>, bit_depth: Option<u8>) -> bool {
    lossless == Some(true)
        && (sample_rate.is_some_and(|rate| rate > 48_000)
            || bit_depth.is_some_and(|depth| depth > 16))
}

fn summarize(format: &SignalFormat) -> String {
    let mut parts = Vec::new();
    if is_dsd(format) {
        // e.g. `DSF DSD64 2.8MHz 2ch`.
        parts.push(
            format
                .container
                .as_deref()
                .map(container_label)
                .unwrap_or_else(|| "DSD".into()),
        );
        if let Some(rate) = format.sample_rate {
            parts.push(format_dsd_rate(rate));
        }
        if let Some(channels) = format.channels {
            parts.push(format!("{channels}ch"));
        }
        return parts.join(" ");
    }
    let codec = format.codec.as_deref().map(codec_label);
    match (&codec, format.container.as_deref()) {
        (Some(codec), Some(container))
            if matches!((codec.as_str(), container), ("ALAC" | "AAC", "mp4")) =>
        {
            parts.push(format!("{codec} ({})", container_label(container)));
        }
        (Some(codec), Some("wav" | "aiff")) if codec == "PCM" => {
            parts.push(container_label(format.container.as_deref().unwrap_or("")));
        }
        (Some(codec), _) => parts.push(codec.clone()),
        (None, Some(container)) => parts.push(container_label(container)),
        (None, None) => parts.push("Unknown format".into()),
    }
    if let Some(rate) = format.sample_rate {
        parts.push(format_rate(rate));
    }
    if let Some(depth) = format.bit_depth {
        parts.push(format!("{depth}bit"));
    }
    if let Some(channels) = format.channels {
        parts.push(format!("{channels}ch"));
    }
    if format.lossless != Some(true)
        && let Some(bitrate) = format.bitrate_kbps
    {
        parts.push(format!("{bitrate}kbps"));
    }
    parts.join(" ")
}

fn source_format(audio: Option<&AudioProperties>, extension: Option<&str>) -> SignalFormat {
    let audio = audio.cloned().unwrap_or_default();
    let codec = audio.codec.clone().or_else(|| {
        extension
            .and_then(codec_from_extension)
            .map(ToString::to_string)
    });
    let lossless = audio
        .lossless
        .or_else(|| codec.as_deref().and_then(codec_is_lossless));
    let mut format = SignalFormat {
        codec,
        container: audio.container.clone(),
        sample_rate: audio.sample_rate,
        bit_depth: audio.bit_depth,
        channels: audio.channels,
        bitrate_kbps: audio.bitrate_kbps,
        lossless,
        high_res: is_high_res(lossless, audio.sample_rate, audio.bit_depth),
        summary: String::new(),
    };
    format.summary = summarize(&format);
    format
}

fn source_stage(format: &SignalFormat, indexed: bool) -> SignalStage {
    let (quality, explanation) = match format.lossless {
        Some(true) if format.high_res => (
            Some(SignalQuality::Lossless),
            "A lossless file above CD resolution. Every sample in the file is available to the rest of the path.".to_string(),
        ),
        Some(true) => (
            Some(SignalQuality::Lossless),
            "A lossless file: decoding it reproduces the original samples exactly.".to_string(),
        ),
        Some(false) => (
            Some(SignalQuality::LowQuality),
            "A lossy file. Detail was discarded when it was encoded, so the path can be no better than low quality, whatever happens later.".to_string(),
        ),
        None => (
            None,
            "Parson could not identify this file's codec, so it cannot say whether the source is lossless.".to_string(),
        ),
    };
    let detail = if indexed {
        format.summary.clone()
    } else {
        format!(
            "{} (from the file name; rescan the library for full details)",
            format.summary
        )
    };
    SignalStage {
        id: "source",
        kind: StageKind::Source,
        title: "Source".into(),
        detail,
        quality,
        explanation,
    }
}

/// Builds the server leg for one song and request.
pub fn describe_server_leg(
    song_id: &str,
    audio: Option<&AudioProperties>,
    extension: Option<&str>,
    content_type: &str,
    request: &ServerLegRequest,
) -> ServerSignalPath {
    let indexed = audio.is_some_and(|audio| !audio.is_empty());
    let source = source_format(audio, extension);
    let mut stages = vec![source_stage(&source, indexed)];

    let effect = request.effect.as_ref();
    let mut transcode = transcode_bitrate(request.bitrate_kbps, effect.is_some());
    let mut cast_conversion = false;
    if request.target == StreamTarget::Cast
        && transcode.is_none()
        && !cast_plays_directly(extension)
    {
        transcode = Some(CAST_TRANSCODE_BITRATE);
        cast_conversion = true;
    }

    let dsd_source = is_dsd(&source);
    if dsd_source && transcode.is_none() {
        return describe_dsd_compat(song_id, source, stages, request);
    }

    let Some(bitrate) = transcode else {
        stages.push(SignalStage {
            id: "server",
            kind: StageKind::Transport,
            title: "Parson server".into(),
            detail: "Original file, streamed byte for byte".into(),
            quality: Some(SignalQuality::Lossless),
            explanation: "The server sends the file exactly as it is on disk, with byte-range seeking. Nothing is decoded or re-encoded on the server.".into(),
        });
        carry_lossy_generation(&mut stages);
        let quality = overall(&stages);
        return ServerSignalPath {
            song_id: song_id.to_string(),
            target: request.target,
            mode: DeliveryMode::Original,
            quality,
            delivered: source.clone(),
            source,
            content_type: content_type.to_string(),
            requested_bitrate_kbps: request.bitrate_kbps,
            effect: None,
            stages,
            processing: None,
        };
    };

    let mut rate = source.sample_rate;
    if dsd_source && let Some(dsd_rate) = rate {
        let pcm = parson_output::dsd::pcm_rate(dsd_rate);
        stages.push(dsd_to_pcm_stage(
            dsd_rate,
            pcm,
            "ffmpeg's DSD decoder",
            None,
        ));
        rate = Some(pcm);
    }
    if let Some(effect) = effect {
        stages.push(SignalStage {
            id: "dsp",
            kind: StageKind::Dsp,
            title: effect.label.to_string(),
            detail: effect.detail.to_string(),
            quality: Some(SignalQuality::Enhanced),
            explanation: "Server-side DSP you chose. It changes the audio on purpose, so the path is enhanced rather than lossless.".into(),
        });
        if let Some(effect_rate) = effect.output_sample_rate {
            if rate.is_some_and(|rate| rate != effect_rate) || rate.is_none() {
                stages.push(SignalStage {
                    id: "dsp-resample",
                    kind: StageKind::Conversion,
                    title: "Sample rate conversion".into(),
                    detail: match rate {
                        Some(rate) => format!("{} to {}", format_rate(rate), format_rate(effect_rate)),
                        None => format!("to {}", format_rate(effect_rate)),
                    },
                    quality: Some(SignalQuality::Enhanced),
                    explanation: "The effect works at a fixed rate, so the audio is resampled before it is processed.".into(),
                });
            }
            rate = Some(effect_rate);
        }
    }

    if let Some(current) = rate {
        let target_rate = mp3_output_rate(current);
        if target_rate != current {
            stages.push(SignalStage {
                id: "resample",
                kind: StageKind::Conversion,
                title: "Sample rate conversion".into(),
                detail: format!("{} to {}", format_rate(current), format_rate(target_rate)),
                quality: Some(SignalQuality::HighQuality),
                explanation: "MP3 cannot carry this sample rate, so the server converts it for compatibility. The conversion is high quality but not lossless.".into(),
            });
            rate = Some(target_rate);
        }
    }

    let channels = source.channels.map(|channels| channels.min(2));
    if let Some(source_channels) = source.channels.filter(|channels| *channels > 2) {
        stages.push(SignalStage {
            id: "downmix",
            kind: StageKind::Conversion,
            title: "Downmix".into(),
            detail: format!("{source_channels}ch to stereo"),
            quality: Some(SignalQuality::HighQuality),
            explanation:
                "MP3 carries at most two channels, so the surround channels are folded into stereo."
                    .into(),
        });
    }

    let lossy_source = source.lossless == Some(false);
    let reason = if cast_conversion {
        "Converted because Cast receivers cannot be relied on to play this format."
    } else if effect.is_some() {
        "Server DSP is rendered into an MP3 stream."
    } else {
        "You chose a streaming bitrate in Settings; set it to Original for lossless playback."
    };
    let mut explanation = format!(
        "The server decodes the file and re-encodes it as a {bitrate} kbps MP3 with LAME. {reason}"
    );
    if lossy_source {
        explanation.push_str(
            " The source is already lossy, so this is a second lossy generation and loses further detail.",
        );
        if source
            .bitrate_kbps
            .is_some_and(|source_bitrate| bitrate > source_bitrate)
        {
            explanation.push_str(
                " The target bitrate is higher than the source's, which adds data but no detail.",
            );
        }
    }
    stages.push(SignalStage {
        id: "encode",
        kind: StageKind::Encode,
        title: if cast_conversion {
            "Transcode for Cast".into()
        } else {
            "Transcode".into()
        },
        detail: format!("MP3 {bitrate}kbps (LAME)"),
        quality: Some(SignalQuality::LowQuality),
        explanation,
    });
    stages.push(SignalStage {
        id: "server",
        kind: StageKind::Transport,
        title: "Parson server".into(),
        detail: "Transcoded stream, cached for seeking".into(),
        quality: Some(SignalQuality::Lossless),
        explanation: "The encoded stream is written to the transcode cache as it is produced, so later plays and seeks reuse it.".into(),
    });

    let mut delivered = SignalFormat {
        codec: Some("mp3".into()),
        container: Some("mp3".into()),
        sample_rate: rate,
        bit_depth: None,
        channels,
        bitrate_kbps: Some(bitrate),
        lossless: Some(false),
        high_res: false,
        summary: String::new(),
    };
    delivered.summary = summarize(&delivered);
    carry_lossy_generation(&mut stages);
    let quality = overall(&stages);
    ServerSignalPath {
        song_id: song_id.to_string(),
        target: request.target,
        mode: DeliveryMode::Transcode,
        quality,
        source,
        delivered,
        content_type: "audio/mpeg".into(),
        requested_bitrate_kbps: request.bitrate_kbps,
        effect: effect.map(|effect| effect.id),
        stages,
        processing: None,
    }
}

fn dsd_to_pcm_stage(
    dsd_rate: u32,
    pcm_rate: u32,
    converter: &str,
    why: Option<&str>,
) -> SignalStage {
    let mut explanation = format!(
        "DSD is a one-bit stream at {}. This output cannot take it as DSD, so {converter} converts it to PCM at {} with a low-pass filter for the ultrasonic noise DSD carries. High quality, but the original bits do not reach the device.",
        format_dsd_rate(dsd_rate),
        format_rate(pcm_rate)
    );
    if let Some(why) = why {
        explanation.push_str(&format!(" Converted because {why}."));
    }
    SignalStage {
        id: "dsd-to-pcm",
        kind: StageKind::Conversion,
        title: "DSD to PCM conversion".into(),
        detail: format!(
            "{} to {}",
            parson_output::dsd_rate_label(dsd_rate),
            format_rate(pcm_rate)
        ),
        quality: Some(SignalQuality::HighQuality),
        explanation,
    }
}

/// A DSD file streamed to a browser or phone: no web or mobile decoder
/// plays DSD, so the server converts it to 24-bit FLAC (Roon's
/// "Conversions for Compatibility").
fn describe_dsd_compat(
    song_id: &str,
    source: SignalFormat,
    mut stages: Vec<SignalStage>,
    request: &ServerLegRequest,
) -> ServerSignalPath {
    let rate = DSD_COMPAT_RATE;
    let bits = DSD_COMPAT_BITS;
    if let Some(dsd_rate) = source.sample_rate {
        let mut stage = dsd_to_pcm_stage(
            dsd_rate,
            rate,
            "the server",
            Some(
                "browsers and phones cannot decode DSD; play it on a server output with native DSD or DoP to keep it bit-perfect",
            ),
        );
        stage.detail = format!(
            "{} to {} {bits}bit",
            parson_output::dsd_rate_label(dsd_rate),
            format_rate(rate)
        );
        stages.push(stage);
    }
    stages.push(SignalStage {
        id: "server",
        kind: StageKind::Transport,
        title: "Parson server".into(),
        detail: format!("{bits}-bit FLAC, cached for seeking"),
        quality: Some(SignalQuality::Lossless),
        explanation: "The converted PCM is stored as lossless FLAC in the transcode cache, so nothing further is lost on the way to the client and later plays reuse it.".into(),
    });
    let mut delivered = SignalFormat {
        codec: Some("flac".into()),
        container: Some("flac".into()),
        sample_rate: Some(rate),
        bit_depth: Some(bits),
        channels: source.channels.map(|channels| channels.min(2)),
        bitrate_kbps: None,
        lossless: Some(true),
        high_res: true,
        summary: String::new(),
    };
    delivered.summary = summarize(&delivered);
    let quality = overall(&stages);
    ServerSignalPath {
        song_id: song_id.to_string(),
        target: request.target,
        mode: DeliveryMode::Transcode,
        quality,
        source,
        delivered,
        content_type: "audio/flac".into(),
        requested_bitrate_kbps: request.bitrate_kbps,
        effect: None,
        stages,
        processing: None,
    }
}

/// Volume leveling the server applies on a native output.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeLeveling<'a> {
    pub gain_db: f64,
    /// `track` or `album`.
    pub source: &'a str,
}

/// `+1.5 dB`, `−6.4 dB` (with a real minus sign, as clients show it).
pub fn format_decibels(db: f64) -> String {
    let rounded = (db * 10.0).round() / 10.0;
    if rounded == 0.0 {
        return "0.0 dB".into();
    }
    format!(
        "{}{:.1} dB",
        if rounded > 0.0 { "+" } else { "\u{2212}" },
        rounded.abs()
    )
}

/// The device end of a native-output path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeOutputTarget<'a> {
    pub device_name: &'a str,
    /// e.g. `WASAPI Exclusive Mode`.
    pub mode_label: &'a str,
    /// Whether the driver is a simulated device without hardware.
    pub simulated: bool,
    /// e.g. `soxr, very high quality`.
    pub resampler: &'a str,
    /// Volume leveling the listener turned on, applied by the server.
    pub leveling: Option<NativeLeveling<'a>>,
    /// The server's DSP chain for this track, when anything processes.
    /// Leveling alone (without a plan) is treated as a chain of one gain.
    pub processing: Option<&'a parson_output::dsp::ProcessingPlan>,
}

fn plan_format_summary(plan: &parson_output::OutputPlan) -> String {
    use parson_output::DsdMode;
    match plan.dsd.as_ref() {
        Some(dsd) if dsd.mode == DsdMode::Native => {
            return format!(
                "native {} ({}, {} frames) {}ch",
                dsd.dsd_label,
                dsd.container.unwrap_or("DSD"),
                format_rate(plan.sample_rate),
                plan.channels
            );
        }
        Some(dsd) if dsd.mode == DsdMode::Dop => {
            return format!(
                "DoP: {} in {} {}bit {}ch",
                dsd.dsd_label,
                format_rate(plan.sample_rate),
                plan.bit_depth,
                plan.channels
            );
        }
        _ => {}
    }
    format!(
        "{} {}bit {}ch",
        format_rate(plan.sample_rate),
        plan.bit_depth,
        plan.channels
    )
}

/// Describes a track played by the server on one of its own devices.
///
/// The server decodes the file and writes PCM straight to the device; when
/// the device takes the source's rate and depth (and is opened
/// exclusively), nothing touches the samples and the path is lossless end
/// to end. Only conversions the device forces appear as stages, rated
/// high quality, the way Roon rates "conversions for compatibility".
pub fn describe_native_leg(
    song_id: &str,
    audio: Option<&AudioProperties>,
    extension: Option<&str>,
    plan: &parson_output::OutputPlan,
    target: &NativeOutputTarget<'_>,
) -> ServerSignalPath {
    use parson_output::DepthChange;
    let indexed = audio.is_some_and(|audio| !audio.is_empty());
    let source = source_format(audio, extension);
    let codec = source
        .codec
        .as_deref()
        .map(codec_label)
        .unwrap_or_else(|| "The file".into());
    let mut stages = vec![source_stage(&source, indexed)];
    let dsd = plan.dsd.as_ref();
    match dsd {
        Some(dsd) if dsd.mode != parson_output::DsdMode::Pcm => stages.push(SignalStage {
            id: "native-dsd-read",
            kind: StageKind::Decode,
            title: "Parson server".into(),
            detail: format!("{} read from the file, not decoded", dsd.dsd_label),
            quality: Some(SignalQuality::Lossless),
            explanation: "The server reads the one-bit DSD stream straight from the file and puts it in time order. It is never turned into PCM, filtered or scaled.".into(),
        }),
        Some(dsd) => {
            stages.push(SignalStage {
                id: "native-decode",
                kind: StageKind::Decode,
                title: "Parson server".into(),
                detail: "DSD decoded to PCM".into(),
                quality: Some(SignalQuality::Lossless),
                explanation: "The server decodes the file itself and writes the samples to the device, so no browser, phone or system mixer handles them.".into(),
            });
            if let Some(pcm_rate) = dsd.pcm_rate {
                let mut stage = dsd_to_pcm_stage(
                    dsd.dsd_rate,
                    pcm_rate,
                    "ffmpeg's DSD decoder",
                    dsd.fallback_reason.as_deref(),
                );
                if dsd.fallback_reason.is_none() {
                    stage.explanation.push_str(" The device's DSD Playback Strategy is set to Convert to PCM.");
                }
                stages.push(stage);
            }
        }
        None => stages.push(SignalStage {
            id: "native-decode",
            kind: StageKind::Decode,
            title: "Parson server".into(),
            detail: format!("{codec} decoded to PCM"),
            quality: Some(SignalQuality::Lossless),
            explanation: format!(
                "The server decodes {codec} itself and writes the samples to the device, so no browser, phone or system mixer handles them. Decoding is exact."
            ),
        }),
    }
    if let Some(dsd) = dsd.filter(|dsd| dsd.mode == parson_output::DsdMode::Dop) {
        stages.push(SignalStage {
            id: "native-dop",
            kind: StageKind::Transport,
            title: "DSD over PCM (DoP v1.0)".into(),
            detail: format!(
                "{} packed into {} {}bit frames",
                dsd.dsd_label,
                format_rate(plan.sample_rate),
                plan.bit_depth
            ),
            quality: Some(SignalQuality::Lossless),
            explanation: format!(
                "Each 24-bit PCM frame carries 16 DSD bits per channel under a marker byte that alternates 0x05 and 0xFA. A DoP DAC recognises the markers and plays the DSD unchanged; the bits are not altered. If {} plays noise, it does not decode DoP: set its DSD Playback Strategy to Native or Convert to PCM.",
                target.device_name
            ),
        });
    }
    let derived;
    let processing = match (target.processing, target.leveling) {
        (Some(processing), _) => Some(processing),
        (None, Some(leveling)) => {
            derived = parson_output::dsp::ProcessingPlan {
                leveling_db: Some(leveling.gain_db),
                headroom_db: None,
                bands: Vec::new(),
                crossfeed: None,
                crossfeed_skipped: false,
                channels: plan.channels,
                input_rate: plan.resample.map_or(plan.sample_rate, |(from, _)| from),
                output_rate: plan.sample_rate,
                rate_requested: false,
                requested_rate_unavailable: None,
                resampler: plan
                    .resample
                    .map(|_| parson_output::dsp::ResamplerKind::Decoder),
                output_bits: plan.bit_depth,
            };
            Some(&derived)
        }
        (None, None) => None,
    };
    let mut chain: Vec<&'static str> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    if let Some(processing) = processing {
        stages.push(SignalStage {
            id: "native-float-in",
            kind: StageKind::Conversion,
            title: "Bit depth conversion".into(),
            detail: match source.bit_depth.filter(|_| source.lossless == Some(true)) {
                Some(bits) => format!("{bits}bit to 64bit float"),
                None => "Decoded to 64bit float".into(),
            },
            quality: Some(SignalQuality::Lossless),
            explanation: "64-bit float holds every integer sample exactly, so moving into it for processing loses nothing. Every stage below runs on the server, before the device, in 64-bit float.".into(),
        });
        if let Some(gain_db) = processing.leveling_db {
            chain.push("native-leveling");
            stages.push(SignalStage {
                id: "native-leveling",
                kind: StageKind::Dsp,
                title: "Volume leveling".into(),
                detail: format!(
                    "{}, {} ReplayGain",
                    format_decibels(gain_db),
                    target.leveling.map_or("track", |leveling| leveling.source)
                ),
                quality: Some(SignalQuality::Enhanced),
                explanation: "Volume leveling, which you turned on, is applied by the server in 64-bit float using the track's ReplayGain tags, so tracks play at a similar loudness. The device's volume stays fixed.".into(),
            });
        }
        if let Some(headroom) = processing.headroom_db {
            chain.push("native-headroom");
            stages.push(SignalStage {
                id: "native-headroom",
                kind: StageKind::Dsp,
                title: "Headroom adjustment".into(),
                detail: format_decibels(headroom),
                quality: Some(SignalQuality::Enhanced),
                explanation: "Headroom management lowers the level before the EQ, in 64-bit float, so its boosts cannot push loud passages past full scale.".into(),
            });
        }
        if !processing.bands.is_empty() {
            chain.push("native-eq");
            let count = processing.bands.len();
            stages.push(SignalStage {
                id: "native-eq",
                kind: StageKind::Dsp,
                title: "Parametric EQ".into(),
                detail: format!(
                    "{count} {}: {}",
                    if count == 1 { "band" } else { "bands" },
                    processing
                        .bands
                        .iter()
                        .map(parson_output::dsp::EqBand::describe)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                quality: Some(SignalQuality::Enhanced),
                explanation: format!(
                    "The parametric EQ you set up for {}, run by the server as {count} biquad filter{} with 64-bit coefficients and state, at {}.",
                    target.device_name,
                    if count == 1 { "" } else { "s" },
                    format_rate(match processing.resampler {
                        Some(parson_output::dsp::ResamplerKind::Decoder) => processing.output_rate,
                        _ => processing.input_rate,
                    })
                ),
            });
        }
        if let Some(strength) = processing.crossfeed {
            chain.push("native-crossfeed");
            let (cutoff, feed, delay) = strength.parameters();
            stages.push(SignalStage {
                id: "native-crossfeed",
                kind: StageKind::Dsp,
                title: "Crossfeed".into(),
                detail: format!(
                    "{}: {cutoff:.0} Hz, {}, {delay} ms",
                    strength.label(),
                    format_decibels(feed)
                ),
                quality: Some(SignalQuality::Enhanced),
                explanation: "Crossfeed mixes a delayed, low-passed share of each channel into the other ear, so headphone listening sounds less like the music is inside your head. The server runs it in 64-bit float.".into(),
            });
        } else if processing.crossfeed_skipped {
            notes.push(format!(
                "Crossfeed is on but this track plays in {} channels, so it is skipped.",
                processing.channels
            ));
        }
        if let Some(rate) = processing.requested_rate_unavailable {
            notes.push(format!(
                "{} does not accept {} as configured, so the DSP sample rate choice is ignored for this track.",
                target.device_name,
                format_rate(rate)
            ));
        }
    }
    if let Some((from, to)) = plan.resample {
        let (converter, requested) = match processing {
            Some(processing) => (
                match processing.resampler {
                    Some(parson_output::dsp::ResamplerKind::Parson) => {
                        parson_output::dsp::resampler_label()
                    }
                    _ => target.resampler.to_string(),
                },
                processing.rate_requested,
            ),
            None => (target.resampler.to_string(), false),
        };
        if processing.is_some() {
            chain.push("native-resample");
        }
        stages.push(SignalStage {
            id: "native-resample",
            kind: StageKind::Conversion,
            title: "Sample rate conversion".into(),
            detail: format!("{} to {} ({converter})", format_rate(from), format_rate(to)),
            quality: Some(if requested {
                SignalQuality::Enhanced
            } else {
                SignalQuality::HighQuality
            }),
            explanation: if requested {
                format!(
                    "You chose {} as the DSP sample rate, so the server converts the track to it with a linear-phase filter in 64-bit float, after the EQ and crossfeed. The device receives exactly that rate; nothing converts it again.",
                    format_rate(to)
                )
            } else {
                format!(
                    "{} cannot play {} as configured, so the server converts it to {}, the closest rate it accepts in the same family. High quality, not bit-perfect. Raise the device's max sample rate in Settings if it really supports {}.",
                    target.device_name,
                    format_rate(from),
                    format_rate(to),
                    format_rate(from)
                )
            },
        });
    }
    if processing.is_some() {
        chain.push("native-depth");
        stages.push(SignalStage {
            id: "native-depth",
            kind: StageKind::Conversion,
            title: "Bit depth conversion".into(),
            detail: format!("64bit float to {}bit, TPDF dither", plan.bit_depth),
            quality: Some(SignalQuality::Enhanced),
            explanation: format!(
                "After processing, the samples are returned to the device's integer format with triangular dither, at the highest depth {} accepts ({} bits). Part of the processing you asked for.",
                target.device_name, plan.bit_depth
            ),
        });
    }
    match plan.depth {
        _ if processing.is_some() => {}
        DepthChange::None => {}
        DepthChange::Pad { from, to } => stages.push(SignalStage {
            id: "native-depth",
            kind: StageKind::Conversion,
            title: "Bit depth".into(),
            detail: format!("{from}bit padded to {to}bit"),
            quality: Some(SignalQuality::Lossless),
            explanation: format!(
                "The device takes {to}-bit words, so each {from}-bit sample is sent with zeros below it. The value of every sample is unchanged."
            ),
        }),
        DepthChange::Reduce { from, to } => stages.push(SignalStage {
            id: "native-depth",
            kind: StageKind::Conversion,
            title: "Bit depth reduction".into(),
            detail: format!("{from}bit to {to}bit, TPDF dither"),
            quality: Some(SignalQuality::HighQuality),
            explanation: format!(
                "The device accepts at most {to} bits as configured, so the server reduces the {from}-bit samples with triangular dither."
            ),
        }),
        DepthChange::Quantize { to } => stages.push(SignalStage {
            id: "native-depth",
            kind: StageKind::Conversion,
            title: "Bit depth".into(),
            detail: format!("Decoded to {to}bit"),
            quality: Some(SignalQuality::HighQuality),
            explanation: if dsd.is_some() {
                format!("DSD has no PCM word length; the converted PCM is quantised to the device's {to}-bit integer format.")
            } else {
                "A lossy file has no fixed bit depth; the decoder's output is quantised to the device's integer format.".into()
            },
        }),
    }
    if let Some((from, to)) = plan.channel_change {
        let upmix = to > from;
        stages.push(SignalStage {
            id: "native-channels",
            kind: StageKind::Conversion,
            title: if upmix { "Channel mapping".into() } else { "Downmix".into() },
            detail: format!("{from}ch to {to}ch"),
            quality: Some(if upmix {
                SignalQuality::Lossless
            } else {
                SignalQuality::HighQuality
            }),
            explanation: if upmix {
                "The device only opens in stereo, so the mono channel is sent to both sides unchanged.".into()
            } else {
                "The device has fewer channels than the file, so the extra channels are folded in.".into()
            },
        });
    }
    let format = plan_format_summary(plan);
    let (quality, mut explanation) = if plan.exclusive {
        (
            SignalQuality::Lossless,
            format!(
                "{} is open in exclusive mode ({}): no other program can play through it, there is no system mixer, and the device runs at {} because the server switched it to this track's format. Volume is fixed; use the device's or amplifier's volume control.",
                target.device_name, target.mode_label, format
            ),
        )
    } else {
        (
            SignalQuality::HighQuality,
            format!(
                "{} is open in shared mode ({}), so the system can still mix in other sounds or convert the format after this point. Turn on Exclusive Mode in the device's settings for a bit-perfect path.",
                target.device_name, target.mode_label
            ),
        )
    };
    if dsd.is_some_and(|dsd| dsd.mode == parson_output::DsdMode::Native) {
        explanation.push_str(" The driver takes DSD natively, so the DAC receives the file's one-bit stream as it is.");
    }
    stages.push(SignalStage {
        id: "native-output",
        kind: StageKind::Output,
        title: target.device_name.to_string(),
        detail: format!("{}, {}", target.mode_label, format),
        quality: Some(quality),
        explanation: if target.simulated {
            format!("{explanation} This is a simulated device: it consumes audio in real time without a sound card.")
        } else {
            explanation
        },
    });
    let as_dsd = dsd.filter(|dsd| dsd.mode != parson_output::DsdMode::Pcm);
    let mut delivered = SignalFormat {
        codec: Some(if as_dsd.is_some() { "dsd" } else { "pcm" }.into()),
        container: as_dsd.map(|dsd| match dsd.mode {
            parson_output::DsdMode::Dop => "DoP".to_string(),
            _ => "native".to_string(),
        }),
        sample_rate: Some(as_dsd.map_or(plan.sample_rate, |dsd| dsd.dsd_rate)),
        bit_depth: Some(if as_dsd.is_some() { 1 } else { plan.bit_depth }),
        channels: Some(plan.channels),
        bitrate_kbps: None,
        lossless: Some(source.lossless == Some(true)),
        high_res: false,
        summary: String::new(),
    };
    delivered.high_res = is_high_res(
        delivered.lossless,
        delivered.sample_rate,
        delivered.bit_depth,
    );
    delivered.summary = summarize(&delivered);
    carry_lossy_generation(&mut stages);
    let quality = overall(&stages);
    ServerSignalPath {
        song_id: song_id.to_string(),
        target: StreamTarget::Native,
        mode: DeliveryMode::Native,
        quality,
        source,
        delivered,
        content_type: "audio/pcm".into(),
        requested_bitrate_kbps: 0,
        effect: None,
        stages,
        processing: processing.map(|_| SignalProcessing {
            engine: SERVER_DSP_ENGINE.into(),
            stages: chain,
            notes,
            status: None,
        }),
    }
}

/// Adds what the operating system reports the hardware is running at, read
/// while the track plays, as the last stage of a native path. When it
/// differs from what the server opened, something below Parson converted
/// the stream, and the path can no longer be called lossless.
pub fn add_hardware_report(
    path: &mut ServerSignalPath,
    device_name: &str,
    hardware: &parson_output::procfs::HardwareStatus,
    stream: &parson_output::StreamFormat,
) {
    let params = &hardware.params;
    let mut detail = format!(
        "{} at {} {}ch",
        params.format,
        format_rate(params.rate),
        params.channels
    );
    if let Some(hz) = hardware.momentary_hz {
        detail.push_str(&format!(", USB clock {}", format_rate(hz)));
    }
    let opened = format!(
        "{} at {} {}ch",
        parson_output::procfs::alsa_format_name(stream.sample_format),
        format_rate(stream.sample_rate),
        stream.channels
    );
    let (quality, explanation) = if hardware.matches_stream {
        (
            SignalQuality::Lossless,
            format!(
                "Read from the kernel ({}) while this track plays: the driver for {device_name} is running exactly the format the server opened ({opened}). Nothing between Parson and the hardware converted it.",
                hardware.source
            ),
        )
    } else {
        (
            SignalQuality::HighQuality,
            format!(
                "Read from the kernel ({}) while this track plays: the server opened {opened}, but the hardware runs {detail}. Something between Parson and {device_name} converted the stream, so it is not bit-perfect.",
                hardware.source
            ),
        )
    };
    path.stages.retain(|stage| stage.id != "native-hardware");
    path.stages.push(SignalStage {
        id: "native-hardware",
        kind: StageKind::Output,
        title: "Driver report".into(),
        detail,
        quality: Some(quality),
        explanation,
    });
    carry_lossy_generation(&mut path.stages);
    path.quality = overall(&path.stages);
}

/// Once a lossy stage has run, every later stage carries lossy audio, so
/// it can be no better than low quality: a byte-exact copy of an MP3 is
/// still an MP3. Roon rates the rest of a path after a lossy file the same
/// way.
pub fn carry_lossy_generation(stages: &mut [SignalStage]) {
    let Some(first_lossy) = stages
        .iter()
        .position(|stage| stage.quality == Some(SignalQuality::LowQuality))
    else {
        return;
    };
    for stage in &mut stages[first_lossy + 1..] {
        if stage.quality.is_some() {
            stage.quality = Some(SignalQuality::LowQuality);
        }
    }
}

/// Worst classified stage. A path whose source is unclassified cannot be
/// called lossless, so it is capped at high quality.
pub fn overall(stages: &[SignalStage]) -> SignalQuality {
    let worst = stages
        .iter()
        .filter_map(|stage| stage.quality)
        .min()
        .unwrap_or(SignalQuality::HighQuality);
    let source_unknown = stages
        .iter()
        .any(|stage| stage.kind == StageKind::Source && stage.quality.is_none());
    if source_unknown {
        worst.min(SignalQuality::HighQuality)
    } else {
        worst
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flac(rate: u32, depth: u8, channels: u8) -> AudioProperties {
        AudioProperties {
            codec: Some("flac".into()),
            container: Some("flac".into()),
            sample_rate: Some(rate),
            bit_depth: Some(depth),
            channels: Some(channels),
            bitrate_kbps: Some(2_100),
            lossless: Some(true),
        }
    }

    fn mp3(bitrate: u32) -> AudioProperties {
        AudioProperties {
            codec: Some("mp3".into()),
            container: Some("mp3".into()),
            sample_rate: Some(44_100),
            bit_depth: None,
            channels: Some(2),
            bitrate_kbps: Some(bitrate),
            lossless: Some(false),
        }
    }

    fn direct(bitrate_kbps: u32) -> ServerLegRequest {
        ServerLegRequest {
            bitrate_kbps,
            effect: None,
            target: StreamTarget::Direct,
        }
    }

    fn ids(path: &ServerSignalPath) -> Vec<&'static str> {
        path.stages.iter().map(|stage| stage.id).collect()
    }

    fn dsd64() -> AudioProperties {
        AudioProperties {
            codec: Some("dsd".into()),
            container: Some("dsf".into()),
            sample_rate: Some(2_822_400),
            bit_depth: Some(1),
            channels: Some(2),
            bitrate_kbps: Some(5_644),
            lossless: Some(true),
        }
    }

    #[test]
    fn browsers_get_dsd_converted_to_flac_for_compatibility() {
        let audio = dsd64();
        let path = describe_server_leg("s", Some(&audio), Some("dsf"), "audio/flac", &direct(0));
        assert_eq!(path.source.summary, "DSF DSD64 2.8MHz 2ch");
        assert!(path.source.high_res);
        assert_eq!(ids(&path), ["source", "dsd-to-pcm", "server"]);
        assert_eq!(path.stages[1].detail, "DSD64 to 88.2kHz 24bit");
        assert_eq!(path.quality, SignalQuality::HighQuality);
        assert_eq!(path.delivered.summary, "FLAC 88.2kHz 24bit 2ch");
        assert_eq!(path.content_type, "audio/flac");
        // A bitrate choice still converts DSD to PCM before the MP3 encode.
        let mp3 = describe_server_leg("s", Some(&audio), Some("dsf"), "audio/mpeg", &direct(320));
        assert_eq!(ids(&mp3)[..3], ["source", "dsd-to-pcm", "resample"]);
        assert_eq!(mp3.stages[2].detail, "352.8kHz to 48kHz");
        assert_eq!(mp3.quality, SignalQuality::LowQuality);
        assert_eq!(codec_from_extension("DFF"), Some("dsd"));
    }

    #[test]
    fn hardware_reports_are_added_once_and_rate_the_path() {
        let mut path = describe_server_leg(
            "s",
            Some(&flac(96_000, 24, 2)),
            Some("flac"),
            "audio/flac",
            &direct(0),
        );
        let stream = parson_output::StreamFormat {
            sample_rate: 96_000,
            sample_format: parson_output::SampleFormat::S24Packed,
            channels: 2,
        };
        let params = parson_output::procfs::parse_hw_params(
            "format: S24_3LE\nchannels: 2\nrate: 96000 (96000/1)\n",
        )
        .unwrap();
        let report = parson_output::procfs::HardwareStatus {
            source: "/proc/asound/card1/pcm0p/sub0/hw_params".into(),
            matches_stream: parson_output::procfs::params_match(&params, &stream),
            params,
            momentary_hz: None,
        };
        add_hardware_report(&mut path, "DragonFly Red", &report, &stream);
        add_hardware_report(&mut path, "DragonFly Red", &report, &stream);
        assert_eq!(
            path.stages
                .iter()
                .filter(|s| s.id == "native-hardware")
                .count(),
            1
        );
        let stage = path.stages.last().unwrap();
        assert_eq!(stage.detail, "S24_3LE at 96kHz 2ch");
        assert!(
            stage
                .explanation
                .contains("exactly the format the server opened")
        );
        assert_eq!(path.quality, SignalQuality::Lossless);
        let wrong = parson_output::procfs::HardwareStatus {
            matches_stream: false,
            ..report
        };
        add_hardware_report(&mut path, "DragonFly Red", &wrong, &stream);
        assert_eq!(path.quality, SignalQuality::HighQuality);
    }

    #[test]
    fn original_lossless_files_stay_lossless_and_flag_high_res() {
        let audio = flac(96_000, 24, 2);
        let path = describe_server_leg("s", Some(&audio), Some("flac"), "audio/flac", &direct(0));
        assert_eq!(path.mode, DeliveryMode::Original);
        assert_eq!(path.quality, SignalQuality::Lossless);
        assert!(path.source.high_res);
        assert_eq!(path.source.summary, "FLAC 96kHz 24bit 2ch");
        assert_eq!(path.delivered, path.source);
        assert_eq!(ids(&path), ["source", "server"]);
        assert_eq!(path.content_type, "audio/flac");

        let cd = flac(44_100, 16, 2);
        let path = describe_server_leg("s", Some(&cd), Some("flac"), "audio/flac", &direct(0));
        assert!(!path.source.high_res);
        assert_eq!(path.source.summary, "FLAC 44.1kHz 16bit 2ch");
    }

    #[test]
    fn lossy_sources_are_low_quality_even_when_untouched() {
        let audio = mp3(320);
        let path = describe_server_leg("s", Some(&audio), Some("mp3"), "audio/mpeg", &direct(0));
        assert_eq!(path.quality, SignalQuality::LowQuality);
        assert_eq!(path.source.summary, "MP3 44.1kHz 2ch 320kbps");
        assert!(
            path.stages
                .iter()
                .all(|stage| stage.quality == Some(SignalQuality::LowQuality))
        );
    }

    #[test]
    fn stages_after_a_lossy_transcode_are_low_quality() {
        let audio = flac(44_100, 16, 2);
        let path = describe_server_leg("s", Some(&audio), Some("flac"), "audio/flac", &direct(320));
        let qualities: Vec<_> = path.stages.iter().map(|stage| stage.quality).collect();
        assert_eq!(
            qualities,
            [
                Some(SignalQuality::Lossless),
                Some(SignalQuality::LowQuality),
                Some(SignalQuality::LowQuality)
            ]
        );
    }

    #[test]
    fn bitrate_transcodes_resample_high_rates_and_downmix_surround() {
        let audio = flac(192_000, 24, 6);
        let path = describe_server_leg("s", Some(&audio), Some("flac"), "audio/flac", &direct(256));
        assert_eq!(path.mode, DeliveryMode::Transcode);
        assert_eq!(path.quality, SignalQuality::LowQuality);
        assert_eq!(
            ids(&path),
            ["source", "resample", "downmix", "encode", "server"]
        );
        assert_eq!(path.stages[1].detail, "192kHz to 48kHz");
        assert_eq!(path.stages[1].quality, Some(SignalQuality::HighQuality));
        assert_eq!(path.stages[2].detail, "6ch to stereo");
        assert_eq!(path.stages[3].detail, "MP3 256kbps (LAME)");
        assert_eq!(path.delivered.summary, "MP3 48kHz 2ch 256kbps");
        assert_eq!(path.content_type, "audio/mpeg");
    }

    #[test]
    fn re_encoding_lossy_sources_explains_the_generation_loss() {
        let audio = mp3(128);
        let path = describe_server_leg("s", Some(&audio), Some("mp3"), "audio/mpeg", &direct(320));
        let encode = path
            .stages
            .iter()
            .find(|stage| stage.id == "encode")
            .unwrap();
        assert!(encode.explanation.contains("second lossy generation"));
        assert!(encode.explanation.contains("adds data but no detail"));
    }

    #[test]
    fn server_dsp_is_enhanced_but_its_mp3_render_is_low_quality() {
        let audio = flac(48_000, 24, 2);
        let request = ServerLegRequest {
            bitrate_kbps: 0,
            effect: Some(DspEffect {
                id: "pitch_down",
                label: "Pitch down",
                detail: "0.82x pitch",
                output_sample_rate: Some(44_100),
            }),
            target: StreamTarget::Direct,
        };
        let path = describe_server_leg("s", Some(&audio), Some("flac"), "audio/flac", &request);
        assert_eq!(
            ids(&path),
            ["source", "dsp", "dsp-resample", "encode", "server"]
        );
        assert_eq!(path.stages[1].quality, Some(SignalQuality::Enhanced));
        assert_eq!(path.stages[2].detail, "48kHz to 44.1kHz");
        assert_eq!(path.delivered.sample_rate, Some(44_100));
        assert_eq!(path.delivered.bitrate_kbps, Some(192));
        assert_eq!(path.effect, Some("pitch_down"));
        assert_eq!(path.quality, SignalQuality::LowQuality);
    }

    #[test]
    fn cast_targets_convert_formats_receivers_may_not_play() {
        let audio = flac(44_100, 16, 2);
        let cast = ServerLegRequest {
            bitrate_kbps: 0,
            effect: None,
            target: StreamTarget::Cast,
        };
        let path = describe_server_leg("s", Some(&audio), Some("flac"), "audio/flac", &cast);
        assert_eq!(path.mode, DeliveryMode::Transcode);
        assert_eq!(path.stages[1].title, "Transcode for Cast");
        assert_eq!(path.delivered.bitrate_kbps, Some(CAST_TRANSCODE_BITRATE));

        let audio = mp3(256);
        let path = describe_server_leg("s", Some(&audio), Some("MP3"), "audio/mpeg", &cast);
        assert_eq!(path.mode, DeliveryMode::Original);
    }

    #[test]
    fn unindexed_files_fall_back_to_the_extension_and_cap_the_quality() {
        let path = describe_server_leg("s", None, Some("flac"), "audio/flac", &direct(0));
        assert_eq!(path.source.codec.as_deref(), Some("flac"));
        assert_eq!(path.source.lossless, Some(true));
        assert!(path.stages[0].detail.contains("rescan"));
        assert_eq!(path.quality, SignalQuality::Lossless);

        let path = describe_server_leg("s", None, Some("m4a"), "audio/mp4", &direct(0));
        assert_eq!(path.stages[0].quality, None);
        assert_eq!(path.quality, SignalQuality::HighQuality);
        assert_eq!(path.source.summary, "Unknown format");
    }

    #[test]
    fn container_and_rate_labels_read_like_roon() {
        let alac = AudioProperties {
            codec: Some("alac".into()),
            container: Some("mp4".into()),
            sample_rate: Some(88_200),
            bit_depth: Some(24),
            channels: Some(2),
            bitrate_kbps: Some(3_000),
            lossless: Some(true),
        };
        let path = describe_server_leg("s", Some(&alac), Some("m4a"), "audio/mp4", &direct(0));
        assert_eq!(path.source.summary, "ALAC (MP4) 88.2kHz 24bit 2ch");
        assert_eq!(format_rate(352_800), "352.8kHz");
        assert_eq!(format_rate(8_000), "8kHz");
        let wav = AudioProperties {
            codec: Some("pcm".into()),
            container: Some("wav".into()),
            sample_rate: Some(44_100),
            bit_depth: Some(16),
            channels: Some(2),
            bitrate_kbps: Some(1_411),
            lossless: Some(true),
        };
        let path = describe_server_leg("s", Some(&wav), Some("wav"), "audio/wav", &direct(0));
        assert_eq!(path.source.summary, "WAV 44.1kHz 16bit 2ch");
    }

    #[test]
    fn mp3_rates_follow_ffmpeg_negotiation() {
        assert_eq!(mp3_output_rate(44_100), 44_100);
        assert_eq!(mp3_output_rate(88_200), 48_000);
        assert_eq!(mp3_output_rate(96_000), 48_000);
        assert_eq!(mp3_output_rate(22_050), 22_050);
        assert_eq!(mp3_output_rate(37_800), 32_000);
    }

    #[test]
    fn quality_serializes_with_roon_names() {
        let json = serde_json::to_string(&[
            SignalQuality::Lossless,
            SignalQuality::Enhanced,
            SignalQuality::HighQuality,
            SignalQuality::LowQuality,
        ])
        .unwrap();
        assert_eq!(
            json,
            r#"["lossless","enhanced","high_quality","low_quality"]"#
        );
        assert!(SignalQuality::LowQuality < SignalQuality::HighQuality);
        assert!(SignalQuality::Enhanced < SignalQuality::Lossless);
    }

    fn native_plan(
        audio: &AudioProperties,
        caps: &parson_output::DeviceCaps,
        exclusive: bool,
    ) -> parson_output::OutputPlan {
        let lossless = audio.lossless == Some(true);
        parson_output::plan_output(
            parson_output::SourceFormat {
                sample_rate: audio.sample_rate.unwrap(),
                bit_depth: if lossless { audio.bit_depth } else { None },
                channels: audio.channels.unwrap_or(2),
                lossless,
            },
            Some(caps),
            &parson_output::OutputSettings {
                exclusive,
                ..parson_output::OutputSettings::default()
            },
        )
        .unwrap()
    }

    fn usb_dac() -> parson_output::DeviceCaps {
        parson_output::DeviceCaps {
            sample_rates: vec![44_100, 48_000, 88_200, 96_000],
            sample_formats: vec![
                parson_output::SampleFormat::S24Packed,
                parson_output::SampleFormat::S32,
            ],
            min_channels: 2,
            max_channels: 2,
            ..parson_output::DeviceCaps::default()
        }
    }

    const DAC: NativeOutputTarget<'static> = NativeOutputTarget {
        device_name: "USB DAC",
        mode_label: "WASAPI Exclusive Mode",
        simulated: false,
        resampler: "soxr, very high quality",
        leveling: None,
        processing: None,
    };

    #[test]
    fn native_exclusive_output_of_a_supported_format_is_lossless() {
        let audio = flac(96_000, 24, 2);
        let plan = native_plan(&audio, &usb_dac(), true);
        let path = describe_native_leg("s", Some(&audio), Some("flac"), &plan, &DAC);
        assert_eq!(path.quality, SignalQuality::Lossless);
        assert_eq!(path.target, StreamTarget::Native);
        let output = path.stages.last().unwrap();
        assert_eq!(output.kind, StageKind::Output);
        assert_eq!(output.title, "USB DAC");
        assert_eq!(output.detail, "WASAPI Exclusive Mode, 96kHz 24bit 2ch");
        assert!(
            !path
                .stages
                .iter()
                .any(|stage| stage.id == "native-resample")
        );
        assert!(path.delivered.high_res);
    }

    #[test]
    fn native_padding_stays_lossless_but_forced_conversions_are_high_quality() {
        let cd = flac(44_100, 16, 2);
        let plan = native_plan(&cd, &usb_dac(), true);
        let path = describe_native_leg("s", Some(&cd), Some("flac"), &plan, &DAC);
        assert_eq!(path.quality, SignalQuality::Lossless);
        let depth = path
            .stages
            .iter()
            .find(|stage| stage.id == "native-depth")
            .unwrap();
        assert_eq!(depth.detail, "16bit padded to 24bit");
        assert_eq!(depth.quality, Some(SignalQuality::Lossless));

        let high = flac(192_000, 24, 2);
        let plan = native_plan(&high, &usb_dac(), true);
        let path = describe_native_leg("s", Some(&high), Some("flac"), &plan, &DAC);
        assert_eq!(path.quality, SignalQuality::HighQuality);
        let resample = path
            .stages
            .iter()
            .find(|stage| stage.id == "native-resample")
            .unwrap();
        assert_eq!(resample.detail, "192kHz to 96kHz (soxr, very high quality)");
        assert_eq!(resample.quality, Some(SignalQuality::HighQuality));
    }

    #[test]
    fn native_shared_mode_and_lossy_sources_are_never_lossless() {
        let audio = flac(44_100, 24, 2);
        let plan = native_plan(&audio, &usb_dac(), false);
        let shared = NativeOutputTarget {
            mode_label: "WASAPI Shared Mode",
            ..DAC
        };
        let path = describe_native_leg("s", Some(&audio), Some("flac"), &plan, &shared);
        assert_eq!(path.quality, SignalQuality::HighQuality);
        assert_eq!(
            path.stages.last().unwrap().quality,
            Some(SignalQuality::HighQuality)
        );

        let lossy = mp3(320);
        let plan = native_plan(&lossy, &usb_dac(), true);
        let path = describe_native_leg("s", Some(&lossy), Some("mp3"), &plan, &DAC);
        assert_eq!(path.quality, SignalQuality::LowQuality);
        assert!(!path.delivered.high_res);
        assert!(
            path.stages
                .iter()
                .all(|stage| stage.quality == Some(SignalQuality::LowQuality))
        );
    }

    #[test]
    fn native_leveling_is_enhanced_processing_in_float() {
        let audio = flac(96_000, 24, 2);
        let plan = native_plan(&audio, &usb_dac(), true);
        let target = NativeOutputTarget {
            leveling: Some(NativeLeveling {
                gain_db: -6.43,
                source: "track",
            }),
            ..DAC
        };
        let path = describe_native_leg("s", Some(&audio), Some("flac"), &plan, &target);
        assert_eq!(path.quality, SignalQuality::Enhanced);
        let details: Vec<_> = path
            .stages
            .iter()
            .map(|stage| (stage.id, stage.detail.as_str()))
            .collect();
        assert_eq!(
            details[2..5],
            [
                ("native-float-in", "24bit to 64bit float"),
                ("native-leveling", "\u{2212}6.4 dB, track ReplayGain"),
                ("native-depth", "64bit float to 24bit, TPDF dither"),
            ]
        );
        assert_eq!(format_decibels(1.25), "+1.3 dB");
        assert_eq!(format_decibels(0.01), "0.0 dB");
    }
}
