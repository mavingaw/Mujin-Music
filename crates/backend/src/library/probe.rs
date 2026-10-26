//! `ffprobe` reader for formats without an in-process parser (TAK, True
//! Audio, WMA, Matroska audio, CAF, Wave64).

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Deserialize;

const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_PICTURE_BYTES: usize = 32 * 1024 * 1024;

/// Tags and stream properties `ffprobe` reported for the first audio stream.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProbedAudio {
    /// Tag key/value pairs from the container and the audio stream, keys upper case.
    pub tags: Vec<(String, String)>,
    pub codec: Option<String>,
    pub container: Option<String>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u8>,
    pub channels: Option<u8>,
    pub bitrate_kbps: Option<u32>,
    pub duration_seconds: f64,
    pub has_attached_picture: bool,
}

impl ProbedAudio {
    /// First value for a tag key (case-insensitive).
    pub fn tag(&self, key: &str) -> Option<&str> {
        self.tags
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.as_str())
            .filter(|value| !value.trim().is_empty())
    }
}

#[derive(Deserialize)]
struct ProbeOutput {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    format: Option<ProbeFormat>,
}

#[derive(Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    sample_fmt: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
    bits_per_raw_sample: Option<String>,
    bits_per_sample: Option<u32>,
    bit_rate: Option<String>,
    duration: Option<String>,
    #[serde(default)]
    disposition: std::collections::HashMap<String, i64>,
    #[serde(default)]
    tags: std::collections::BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct ProbeFormat {
    format_name: Option<String>,
    duration: Option<String>,
    bit_rate: Option<String>,
    #[serde(default)]
    tags: std::collections::BTreeMap<String, String>,
}

/// Parser-neutral codec name, matching the names the in-process parsers use.
pub fn canonical_codec(name: &str) -> String {
    let name = name.to_ascii_lowercase();
    match name.as_str() {
        n if n.starts_with("pcm_f") => "pcm_float".into(),
        n if n.starts_with("pcm_") => "pcm".into(),
        n if n.starts_with("dsd_") => "dsd".into(),
        "wmav1" | "wmav2" => "wma".into(),
        "wmapro" => "wma_pro".into(),
        "wmalossless" => "wma_lossless".into(),
        "musepack7" | "musepack8" => "musepack".into(),
        "dca" => "dts".into(),
        _ => name,
    }
}

fn canonical_container(format_name: &str) -> String {
    let first = format_name
        .split(',')
        .next()
        .unwrap_or(format_name)
        .to_ascii_lowercase();
    match first.as_str() {
        "mov" => "mp4".into(),
        _ => first,
    }
}

fn parse_number<T: std::str::FromStr>(value: Option<&String>) -> Option<T> {
    value.and_then(|value| value.trim().parse::<T>().ok())
}

/// Parses `ffprobe -print_format json -show_format -show_streams` output.
pub fn parse_ffprobe_json(json: &str) -> Result<ProbedAudio, String> {
    let output: ProbeOutput = serde_json::from_str(json).map_err(|error| error.to_string())?;
    let has_attached_picture = output.streams.iter().any(|stream| {
        stream.codec_type.as_deref() == Some("video")
            && stream.disposition.get("attached_pic").copied() == Some(1)
    });
    let stream = output
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("audio"))
        .ok_or_else(|| "no audio stream".to_string())?;
    let format = output.format.as_ref();
    let mut tags = Vec::new();
    for (key, value) in format
        .map(|format| format.tags.iter())
        .into_iter()
        .flatten()
        .chain(stream.tags.iter())
    {
        let key = key.to_ascii_uppercase();
        if !tags.iter().any(|(existing, _)| existing == &key) {
            tags.push((key, value.clone()));
        }
    }
    let codec = stream.codec_name.as_deref().map(canonical_codec);
    let lossy_float = stream
        .sample_fmt
        .as_deref()
        .is_some_and(|format| format.starts_with("flt") || format.starts_with("dbl"));
    let bit_depth = parse_number::<u8>(stream.bits_per_raw_sample.as_ref())
        .or_else(|| {
            stream
                .bits_per_sample
                .and_then(|bits| u8::try_from(bits).ok())
        })
        .filter(|bits| *bits > 0)
        .filter(|_| {
            !lossy_float
                || codec
                    .as_deref()
                    .is_some_and(|codec| codec == "pcm_float" || codec == "dsd")
        });
    let duration_seconds = parse_number::<f64>(stream.duration.as_ref())
        .or_else(|| format.and_then(|format| parse_number::<f64>(format.duration.as_ref())))
        .filter(|duration| duration.is_finite() && *duration > 0.0)
        .unwrap_or_default();
    let bitrate_kbps = parse_number::<u64>(stream.bit_rate.as_ref())
        .or_else(|| format.and_then(|format| parse_number::<u64>(format.bit_rate.as_ref())))
        .map(|bits| (bits / 1000) as u32)
        .filter(|kbps| *kbps > 0);
    Ok(ProbedAudio {
        tags,
        bit_depth: if codec.as_deref() == Some("dsd") {
            Some(1)
        } else {
            bit_depth
        },
        codec,
        container: format
            .and_then(|format| format.format_name.as_deref())
            .map(canonical_container),
        sample_rate: parse_number::<u32>(stream.sample_rate.as_ref()).filter(|rate| *rate > 0),
        channels: stream
            .channels
            .and_then(|channels| u8::try_from(channels).ok())
            .filter(|channels| *channels > 0),
        bitrate_kbps,
        duration_seconds,
        has_attached_picture,
    })
}

/// Runs a command, killing it if it outlives the timeout.
fn run_bounded(mut command: Command, limit: usize) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not start {:?}: {error}", command.get_program()))?;
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = (&mut stdout).take(limit as u64 + 1).read_to_end(&mut bytes);
        result.map(|_| bytes)
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > PROBE_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("timed out".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(error) => return Err(error.to_string()),
        }
    }
    let status = child.wait().map_err(|error| error.to_string())?;
    let bytes = reader
        .join()
        .map_err(|_| "reader panicked".to_string())?
        .map_err(|error| error.to_string())?;
    if !status.success() {
        return Err(format!("exited with {status}"));
    }
    if bytes.len() > limit {
        return Err("output too large".into());
    }
    Ok(bytes)
}

/// Probes a file with `ffprobe`.
pub fn ffprobe(path: &Path) -> Result<ProbedAudio, String> {
    let mut command = Command::new("ffprobe");
    command
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path);
    let bytes = run_bounded(command, 4 * 1024 * 1024)?;
    parse_ffprobe_json(&String::from_utf8_lossy(&bytes))
}

/// Extracts an attached cover picture with ffmpeg (WMA `WM/Picture`, TAK, Matroska).
pub fn ffmpeg_attached_picture(path: &Path) -> Option<Vec<u8>> {
    let mut command = Command::new("ffmpeg");
    command.args(["-v", "error", "-i"]).arg(path).args([
        "-map",
        "0:v:0",
        "-frames:v",
        "1",
        "-c",
        "copy",
        "-f",
        "image2pipe",
        "pipe:1",
    ]);
    run_bounded(command, MAX_PICTURE_BYTES)
        .ok()
        .filter(|bytes| !bytes.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WMA: &str = r#"{
        "streams": [{"index":0,"codec_name":"wmav2","codec_type":"audio","sample_fmt":"fltp",
            "sample_rate":"44100","channels":2,"bits_per_sample":0,"bit_rate":"128000",
            "duration":"61.500000","disposition":{"attached_pic":0}},
            {"index":1,"codec_name":"mjpeg","codec_type":"video","disposition":{"attached_pic":1}}],
        "format": {"format_name":"asf","duration":"61.500000","bit_rate":"160576",
            "tags":{"title":"So What","artist":"Miles Davis","album":"Kind of Blue",
                "track":"1/5","composer":"Miles Davis","album_artist":"Miles Davis","date":"1959"}}
    }"#;

    #[test]
    fn wma_probe_reads_tags_properties_and_cover_presence() {
        let probed = parse_ffprobe_json(WMA).unwrap();
        assert_eq!(probed.codec.as_deref(), Some("wma"));
        assert_eq!(probed.container.as_deref(), Some("asf"));
        assert_eq!(probed.sample_rate, Some(44_100));
        assert_eq!(probed.channels, Some(2));
        assert_eq!(probed.bit_depth, None);
        assert_eq!(probed.bitrate_kbps, Some(128));
        assert!((probed.duration_seconds - 61.5).abs() < 1e-9);
        assert!(probed.has_attached_picture);
        assert_eq!(probed.tag("TITLE"), Some("So What"));
        assert_eq!(probed.tag("track"), Some("1/5"));
    }

    #[test]
    fn lossless_probe_keeps_bit_depth_and_stream_tags() {
        let json = r#"{"streams":[{"codec_name":"tak","codec_type":"audio","sample_fmt":"s32p",
            "sample_rate":"96000","channels":6,"bits_per_raw_sample":"24",
            "tags":{"TITLE":"Surround"}}],
            "format":{"format_name":"tak","duration":"3.0"}}"#;
        let probed = parse_ffprobe_json(json).unwrap();
        assert_eq!(probed.codec.as_deref(), Some("tak"));
        assert_eq!(probed.bit_depth, Some(24));
        assert_eq!(probed.channels, Some(6));
        assert_eq!(probed.tag("title"), Some("Surround"));
        assert_eq!(canonical_codec("wmalossless"), "wma_lossless");
        assert_eq!(canonical_codec("pcm_s24le"), "pcm");
        assert_eq!(canonical_codec("dsd_lsbf_planar"), "dsd");
        assert!(parse_ffprobe_json(r#"{"streams":[]}"#).is_err());
    }
}
