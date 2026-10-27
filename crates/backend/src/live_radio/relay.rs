//! The live radio relay: from a station URL to bytes a player can play.
//!
//! 1. Follow the URL (through the SSRF guard) and any playlist files it
//!    names (PLS, M3U, ASX, XSPF), trying alternatives when one fails.
//! 2. For Icecast/SHOUTcast streams, ask for ICY metadata and strip it
//!    from the audio, reporting titles as they change; for HLS, join the
//!    segments live.
//! 3. Read the real format from the first audio bytes.
//! 4. Deliver: unchanged when the destination plays the format; otherwise
//!    repackaged (HLS segments into a plain stream, no re-encode) or
//!    converted with ffmpeg, losslessly (FLAC/WAV) when the destination
//!    takes it, so a lossy station is never encoded a second time unless
//!    the destination only plays MP3.

use std::collections::VecDeque;
use std::time::Duration;

use bytes::Bytes;
use futures::stream::StreamExt;
use reqwest::Url;
use serde::Serialize;

use super::format::{Container, DETECT_BYTES, Detected, detect};
use super::guard::{Body, GuardError, GuardedClient, NetPolicy, fused, parse_url};
use super::hls::{self, HlsError, TitleSink};
use super::icy::{IcyDemuxer, IcyHeaders};
use super::playlist::{self, MAX_PLAYLIST_BYTES, Payload};
use super::station::{Station, normalize_codec};
use crate::outputs::FormatLimits;

/// Station URLs and playlist entries tried before giving up.
const MAX_ATTEMPTS: usize = 8;
/// Playlists inside playlists followed at most this deep.
const MAX_DEPTH: usize = 3;

#[derive(Debug)]
pub enum RelayError {
    Guard(GuardError),
    /// Nothing playable behind the URL.
    NoStream,
    /// The URL is a web page.
    WebPage,
    Unsupported(&'static str),
    /// ffmpeg is needed but missing or failed to start.
    Converter(String),
}

impl RelayError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Guard(error) => error.code(),
            Self::NoStream => "no_audio_stream",
            Self::WebPage => "web_page_not_stream",
            Self::Unsupported(_) => "unsupported_stream",
            Self::Converter(_) => "converter_unavailable",
        }
    }

    /// Whether the listener (not the station) is at fault.
    pub fn is_client_error(&self) -> bool {
        matches!(
            self,
            Self::Guard(GuardError::InvalidUrl(_) | GuardError::Blocked(_))
        )
    }
}

impl std::fmt::Display for RelayError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Guard(error) => write!(formatter, "{error}"),
            Self::NoStream => formatter.write_str(
                "No audio stream was found at that address. Use the stream URL (often ending in .mp3, .aac, .pls, .m3u or .m3u8), not the station's web page.",
            ),
            Self::WebPage => formatter.write_str(
                "That address is a web page, not an audio stream. Look on the station's site for a \"listen\" link or a stream URL (often ending in .mp3, .aac, .pls, .m3u or .m3u8).",
            ),
            Self::Unsupported(reason) => write!(formatter, "{reason}"),
            Self::Converter(message) => write!(formatter, "Mujin Music could not convert the stream: {message}"),
        }
    }
}

impl From<GuardError> for RelayError {
    fn from(error: GuardError) -> Self {
        Self::Guard(error)
    }
}

impl From<HlsError> for RelayError {
    fn from(error: HlsError) -> Self {
        match error {
            HlsError::Guard(error) => Self::Guard(error),
            HlsError::Encrypted => Self::Unsupported(
                "This HLS stream is encrypted (DRM), so it cannot be played outside the station's own player.",
            ),
            HlsError::Empty | HlsError::NotHls { .. } => Self::NoStream,
        }
    }
}

/// How the audio left the station.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    /// Icecast/SHOUTcast or plain HTTP progressive stream.
    Http,
    Hls,
}

/// An opened station, before delivery.
pub struct Source {
    pub url: Url,
    pub transport: Transport,
    pub content_type: Option<String>,
    pub icy: IcyHeaders,
    /// HLS master playlist codec, when stated.
    pub hls_codec: Option<&'static str>,
    pub hls_bandwidth: Option<u64>,
    /// ICY metadata was requested and offered.
    pub icy_metadata: bool,
    pub body: Body,
}

/// Reads up to `wanted` bytes (or the first chunk after `timeout`) and
/// returns them with a body that replays them first. A body that ended (or
/// failed) is not polled again: the replay is all that is left of it.
pub async fn read_head(body: Body, wanted: usize, timeout: Duration) -> (Vec<u8>, Body) {
    let mut body = fused(body);
    let mut head = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;
    let mut failure = None;
    let mut finished = false;
    while head.len() < wanted {
        match tokio::time::timeout_at(deadline, body.next()).await {
            Ok(Some(Ok(chunk))) => head.extend_from_slice(&chunk),
            Ok(Some(Err(error))) => {
                failure = Some(error);
                finished = true;
                break;
            }
            Ok(None) => {
                finished = true;
                break;
            }
            Err(_) => break,
        }
    }
    (
        head.clone(),
        replay_then(head, failure, (!finished).then_some(body)),
    )
}

/// `bytes` (and `failure`), then the rest of `body` when it has more.
fn replay_then(bytes: Vec<u8>, failure: Option<String>, body: Option<Body>) -> Body {
    let prefix = futures::stream::iter(
        (!bytes.is_empty())
            .then(|| Ok(Bytes::from(bytes)))
            .into_iter()
            .chain(failure.map(Err)),
    );
    match body {
        Some(body) => fused(prefix.chain(body).boxed()),
        None => prefix.boxed(),
    }
}

async fn read_text(body: Body) -> String {
    let (bytes, _) = read_head(body, MAX_PLAYLIST_BYTES, Duration::from_secs(10)).await;
    String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_PLAYLIST_BYTES)]).into_owned()
}

fn demux(body: Body, metaint: usize, titles: TitleSink) -> Body {
    let Some(demuxer) = IcyDemuxer::new(metaint) else {
        return body;
    };
    body.scan(demuxer, move |demuxer, chunk| {
        let item = chunk.map(|chunk| {
            let mut audio = Vec::with_capacity(chunk.len());
            for metadata in demuxer.feed(&chunk, &mut audio) {
                titles(metadata.stream_title);
            }
            Bytes::from(audio)
        });
        futures::future::ready(Some(item))
    })
    .filter(|item| futures::future::ready(item.as_ref().map_or(true, |chunk| !chunk.is_empty())))
    .boxed()
}

/// Opens a station: follows playlists to audio (see the module docs).
pub async fn open_source(
    client: &GuardedClient,
    station: &Station,
    titles: TitleSink,
) -> Result<Source, RelayError> {
    let policy = if station.allow_lan {
        NetPolicy::LAN
    } else {
        NetPolicy::PUBLIC
    };
    let first = parse_url(&station.stream_url)?;
    let mut candidates: VecDeque<(Url, usize)> = VecDeque::from([(first, 0)]);
    let mut last_error = RelayError::NoStream;
    let mut attempts = 0;
    while let Some((url, depth)) = candidates.pop_front() {
        attempts += 1;
        if attempts > MAX_ATTEMPTS {
            break;
        }
        if playlist::looks_like_hls(&url) || (station.hls && depth == 0) {
            match hls::open(client, policy, url.clone(), None, titles.clone()).await {
                Ok(stream) => return Ok(hls_source(stream)),
                Err(HlsError::NotHls { url, text }) => {
                    // An `.m3u8` that is a plain list of stream URLs.
                    last_error = follow_list(&mut candidates, Payload::M3u, &text, &url, depth);
                    continue;
                }
                Err(error) => {
                    last_error = error.into();
                    continue;
                }
            }
        }
        let opened = match client
            .open(url, policy, &[("Icy-MetaData", "1"), ("Accept", "*/*")])
            .await
        {
            Ok(opened) => opened,
            Err(error) => {
                last_error = error.into();
                continue;
            }
        };
        let headers = opened.response.headers.clone();
        let header = |name: &str| {
            headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.clone())
        };
        let content_type = header("content-type");
        let (head, body) = read_head(opened.response.body, 2048, Duration::from_secs(8)).await;
        if head.is_empty() {
            last_error = RelayError::NoStream;
            continue;
        }
        match playlist::sniff(content_type.as_deref(), &opened.url, &head) {
            Payload::Audio => {
                let metaint =
                    header("icy-metaint").and_then(|value| value.trim().parse::<usize>().ok());
                let icy = IcyHeaders::read(|name| header(name));
                let body = match metaint {
                    Some(metaint) => demux(body, metaint, titles.clone()),
                    None => body,
                };
                return Ok(Source {
                    url: opened.url,
                    transport: Transport::Http,
                    content_type,
                    icy,
                    hls_codec: None,
                    hls_bandwidth: None,
                    icy_metadata: metaint.is_some(),
                    body,
                });
            }
            Payload::Hls => {
                let text = read_text(body).await;
                match hls::open(client, policy, opened.url, Some(text), titles.clone()).await {
                    Ok(stream) => return Ok(hls_source(stream)),
                    Err(HlsError::NotHls { url, text }) => {
                        last_error = follow_list(&mut candidates, Payload::M3u, &text, &url, depth);
                    }
                    Err(error) => last_error = error.into(),
                }
            }
            Payload::WebPage => last_error = RelayError::WebPage,
            kind => {
                let text = read_text(body).await;
                last_error = follow_list(&mut candidates, kind, &text, &opened.url, depth);
            }
        }
    }
    Err(last_error)
}

/// Queues the stream URLs of a playlist file (PLS, M3U, ASX, XSPF) to be
/// tried next, in the file's order, unless playlists are already nested
/// [`MAX_DEPTH`] deep. Returns the error to report if none of them plays.
fn follow_list(
    candidates: &mut VecDeque<(Url, usize)>,
    kind: Payload,
    text: &str,
    base: &Url,
    depth: usize,
) -> RelayError {
    if depth >= MAX_DEPTH {
        return RelayError::NoStream;
    }
    let entries = playlist::entries(kind, text, base);
    for entry in entries.into_iter().rev() {
        candidates.push_front((entry.url, depth + 1));
    }
    RelayError::NoStream
}

fn hls_source(stream: hls::HlsStream) -> Source {
    Source {
        url: stream.playlist_url,
        transport: Transport::Hls,
        content_type: None,
        icy: IcyHeaders::default(),
        hls_codec: stream.codec,
        hls_bandwidth: stream.bandwidth,
        icy_metadata: false,
        body: stream.body,
    }
}

/// The source's format: read from its bytes, or from its headers.
pub async fn identify(source: Source) -> (Source, Option<Detected>) {
    let Source { body, .. } = source;
    let mut collected = Vec::new();
    let mut body = fused(body);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let mut detected = None;
    let mut failure = None;
    let mut finished = false;
    while collected.len() < DETECT_BYTES {
        match tokio::time::timeout_at(deadline, body.next()).await {
            Ok(Some(Ok(chunk))) => {
                collected.extend_from_slice(&chunk);
                detected = detect(&collected);
                if detected.as_ref().is_some_and(|found| found.codec.is_some()) {
                    break;
                }
            }
            Ok(Some(Err(error))) => {
                failure = Some(error);
                finished = true;
                break;
            }
            Ok(None) => {
                finished = true;
                break;
            }
            Err(_) => break,
        }
    }
    let body = replay_then(collected, failure, (!finished).then_some(body));
    (Source { body, ..source }, detected)
}

/// What the stream is, combining the bytes, headers and HLS playlist.
#[derive(Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SourceFormat {
    pub container: Container,
    pub codec: Option<String>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u8>,
    pub channels: Option<u8>,
    pub bitrate_kbps: Option<u32>,
    /// The codec was read from the audio itself, not taken on trust.
    pub verified: bool,
}

pub fn source_format(
    source: &Source,
    detected: Option<&Detected>,
    station: &Station,
) -> SourceFormat {
    let header_codec = source
        .content_type
        .as_deref()
        .and_then(|value| normalize_codec(value.split(';').next().unwrap_or_default()))
        .filter(|codec| codec != "application/octet-stream");
    let claimed_codec = source
        .hls_codec
        .map(str::to_string)
        .or(header_codec)
        .or_else(|| station.codec.as_deref().and_then(normalize_codec));
    let container =
        detected
            .map(|found| found.container)
            .unwrap_or_else(|| match claimed_codec.as_deref() {
                Some("mp3") => Container::Mp3,
                Some("aac") => Container::Adts,
                Some("flac") => Container::Flac,
                Some("vorbis" | "opus") => Container::Ogg,
                _ => Container::Unknown,
            });
    let verified_codec = detected.and_then(|found| found.codec.clone());
    let bitrate = detected
        .and_then(|found| found.bitrate_kbps)
        .or(source.icy.bitrate_kbps)
        .or(source
            .hls_bandwidth
            .map(|bandwidth| (bandwidth / 1000) as u32))
        .or(station.bitrate_kbps);
    SourceFormat {
        container,
        verified: verified_codec.is_some(),
        codec: verified_codec.or(claimed_codec),
        sample_rate: detected
            .and_then(|found| found.sample_rate)
            .or(source.icy.sample_rate),
        bit_depth: detected.and_then(|found| found.bit_depth),
        channels: detected
            .and_then(|found| found.channels)
            .or(source.icy.channels),
        bitrate_kbps: bitrate.filter(|&rate| rate > 0),
    }
}

/// A container Parson sends.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Output {
    Mp3,
    Adts,
    Flac,
    Ogg,
    Wav,
}

impl Output {
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Mp3 => "audio/mpeg",
            Self::Adts => "audio/aac",
            Self::Flac => "audio/flac",
            Self::Ogg => "audio/ogg",
            Self::Wav => "audio/wav",
        }
    }

    fn muxer(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Adts => "adts",
            Self::Flac => "flac",
            Self::Ogg => "ogg",
            Self::Wav => "wav",
        }
    }

    pub fn codec(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Adts => "aac",
            Self::Flac => "flac",
            Self::Ogg => "ogg",
            Self::Wav => "pcm",
        }
    }
}

/// Who plays the stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destination {
    /// A Parson app (browser, desktop, phone). `accepts` lists the codecs
    /// its player decodes, when it said.
    Player { accepts: Option<Vec<String>> },
    /// A network renderer, Sonos or a server-side output.
    Renderer(FormatLimits),
}

/// How the relay delivers.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Delivery {
    /// The station's own bytes.
    Original,
    /// Same audio, new container (no re-encode).
    Repackage { output: Output },
    /// Decoded and re-encoded.
    Convert {
        output: Output,
        sample_rate: Option<u32>,
        bit_depth: Option<u8>,
        bitrate_kbps: Option<u32>,
    },
}

impl Delivery {
    pub fn output(self) -> Option<Output> {
        match self {
            Self::Original => None,
            Self::Repackage { output } | Self::Convert { output, .. } => Some(output),
        }
    }
}

fn natural_output(codec: &str) -> Option<Output> {
    match codec {
        "mp3" => Some(Output::Mp3),
        "aac" => Some(Output::Adts),
        "flac" => Some(Output::Flac),
        "opus" | "vorbis" => Some(Output::Ogg),
        _ => None,
    }
}

const PLAYER_CODECS: [&str; 5] = ["mp3", "aac", "flac", "opus", "vorbis"];

/// Chooses the delivery for a format and destination.
pub fn plan(format: &SourceFormat, destination: &Destination) -> Delivery {
    let codec = format.codec.as_deref().unwrap_or("unknown");
    let needs_repackage = matches!(format.container, Container::Ts | Container::Fmp4);
    let plays = match destination {
        Destination::Player { accepts } => match accepts {
            Some(list) => list.iter().any(|accepted| {
                accepted == codec || (accepted == "ogg" && matches!(codec, "opus" | "vorbis"))
            }),
            None => PLAYER_CODECS.contains(&codec) || codec == "unknown",
        },
        Destination::Renderer(limits) => match codec {
            "mp3" => limits.mp3,
            "aac" => limits.aac,
            "flac" => {
                limits.flac
                    && format
                        .sample_rate
                        .is_none_or(|rate| rate <= limits.max_sample_rate)
                    && format
                        .bit_depth
                        .is_none_or(|depth| depth <= limits.max_bit_depth)
            }
            _ => false,
        },
    };
    if plays {
        if !needs_repackage {
            return Delivery::Original;
        }
        if let Some(output) = natural_output(codec) {
            return Delivery::Repackage { output };
        }
    }
    let (output, max_rate, max_depth) = match destination {
        Destination::Player { accepts } => {
            let accepts = |codec: &str| {
                accepts
                    .as_ref()
                    .is_none_or(|list| list.iter().any(|accepted| accepted == codec))
            };
            if accepts("flac") {
                (Output::Flac, 192_000, 24)
            } else if accepts("wav") {
                (Output::Wav, 192_000, 24)
            } else {
                (Output::Mp3, 48_000, 16)
            }
        }
        Destination::Renderer(limits) => {
            if limits.flac {
                (Output::Flac, limits.max_sample_rate, limits.max_bit_depth)
            } else if limits.wav {
                (Output::Wav, limits.max_sample_rate, limits.max_bit_depth)
            } else {
                (Output::Mp3, limits.max_sample_rate.min(48_000), 16)
            }
        }
    };
    if output == Output::Mp3 {
        return Delivery::Convert {
            output,
            sample_rate: format
                .sample_rate
                .filter(|&rate| rate > max_rate)
                .map(|_| max_rate.min(48_000)),
            bit_depth: None,
            bitrate_kbps: Some(320),
        };
    }
    let source_depth = format.bit_depth.unwrap_or(24);
    Delivery::Convert {
        output,
        sample_rate: format
            .sample_rate
            .filter(|&rate| rate > max_rate)
            .map(|_| max_rate),
        bit_depth: Some(source_depth.min(max_depth).max(16)),
        bitrate_kbps: None,
    }
}

/// ffmpeg arguments for a delivery; `None` for [`Delivery::Original`].
pub fn ffmpeg_args(delivery: Delivery, input: Container) -> Option<Vec<String>> {
    let mut args: Vec<String> = ["-hide_banner", "-loglevel", "error", "-probesize", "262144"]
        .into_iter()
        .map(str::to_string)
        .collect();
    let demuxer = match input {
        Container::Ts => Some("mpegts"),
        Container::Fmp4 => Some("mp4"),
        Container::Adts => Some("aac"),
        Container::Mp3 => Some("mp3"),
        Container::Ogg => Some("ogg"),
        Container::Flac => Some("flac"),
        Container::Wav => Some("wav"),
        Container::Unknown => None,
    };
    if let Some(demuxer) = demuxer {
        args.extend(["-f".into(), demuxer.into()]);
    }
    args.extend(["-i", "pipe:0", "-map", "0:a:0", "-vn"].map(str::to_string));
    match delivery {
        Delivery::Original => return None,
        Delivery::Repackage { output } => {
            args.extend(["-c:a".into(), "copy".into()]);
            args.extend(["-f".into(), output.muxer().into()]);
        }
        Delivery::Convert {
            output,
            sample_rate,
            bit_depth,
            bitrate_kbps,
        } => {
            if let Some(rate) = sample_rate {
                args.extend(["-af".into(), format!("aresample={rate}:resampler=soxr")]);
            }
            match output {
                Output::Flac => {
                    args.extend(["-c:a", "flac", "-compression_level", "5"].map(str::to_string));
                    if bit_depth.unwrap_or(24) > 16 {
                        args.extend(
                            ["-sample_fmt", "s32", "-bits_per_raw_sample", "24"]
                                .map(str::to_string),
                        );
                    } else {
                        args.extend(["-sample_fmt", "s16"].map(str::to_string));
                    }
                }
                Output::Wav => {
                    let codec = if bit_depth.unwrap_or(24) > 16 {
                        "pcm_s24le"
                    } else {
                        "pcm_s16le"
                    };
                    args.extend(["-c:a".into(), codec.into()]);
                }
                Output::Mp3 => {
                    args.extend(["-c:a".into(), "libmp3lame".into()]);
                    args.extend(["-b:a".into(), format!("{}k", bitrate_kbps.unwrap_or(320))]);
                }
                Output::Adts => args.extend(["-c:a", "aac", "-b:a", "256k"].map(str::to_string)),
                Output::Ogg => args.extend(["-c:a", "libopus", "-b:a", "192k"].map(str::to_string)),
            }
            args.extend(["-f".into(), output.muxer().into()]);
        }
    }
    args.push("pipe:1".into());
    Some(args)
}

/// Runs `body` through ffmpeg with `args`. ffmpeg is killed when the
/// returned stream is dropped (the listener left).
pub fn pipe_through_ffmpeg(mut body: Body, args: Vec<String>) -> Result<Body, RelayError> {
    use tokio::io::AsyncWriteExt;

    let mut child = tokio::process::Command::new("ffmpeg")
        .args(&args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| RelayError::Converter(error.to_string()))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| RelayError::Converter("no input pipe".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| RelayError::Converter("no output pipe".into()))?;
    let writer = tokio::spawn(async move {
        while let Some(Ok(chunk)) = body.next().await {
            if stdin.write_all(&chunk).await.is_err() {
                break;
            }
        }
        let _ = stdin.shutdown().await;
    });
    struct Running {
        _child: tokio::process::Child,
        writer: tokio::task::JoinHandle<()>,
    }
    impl Drop for Running {
        fn drop(&mut self) {
            self.writer.abort();
        }
    }
    let running = Running {
        _child: child,
        writer,
    };
    let output = tokio_util::io::ReaderStream::with_capacity(stdout, 32 * 1024);
    Ok(
        futures::stream::unfold((output, running), |(mut output, running)| async move {
            let chunk = output.next().await?;
            Some((chunk.map_err(|error| error.to_string()), (output, running)))
        })
        .fuse()
        .boxed(),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use futures::StreamExt;

    use super::*;
    use crate::live_radio::format::tests::{adts_frames, mp3_frames};
    use crate::live_radio::guard::fakes::{FakeFetch, FakeReply, FakeResolver};
    use crate::live_radio::store::tests::station;

    fn client(fetch: FakeFetch) -> (GuardedClient, Arc<FakeFetch>) {
        let fetch = Arc::new(fetch);
        (
            GuardedClient::new(
                Arc::new(
                    FakeResolver::default()
                        .with("radio.example", &["93.184.216.34"])
                        .with("ice.example", &["93.184.216.35"])
                        .with("lan.example", &["192.168.1.40"]),
                ),
                fetch.clone(),
            ),
            fetch,
        )
    }

    fn sink() -> (TitleSink, Arc<Mutex<Vec<String>>>) {
        let titles = Arc::new(Mutex::new(Vec::new()));
        let sink: TitleSink = {
            let titles = titles.clone();
            Arc::new(move |title| titles.lock().unwrap().push(title))
        };
        (sink, titles)
    }

    fn icy_wire(audio: &[u8], metaint: usize, title: &str) -> Vec<u8> {
        let mut wire = Vec::new();
        for (index, run) in audio.chunks(metaint).enumerate() {
            wire.extend_from_slice(run);
            if run.len() == metaint {
                if index == 0 {
                    let text = format!("StreamTitle='{title}';");
                    let padded = text.len().div_ceil(16) * 16;
                    wire.push((padded / 16) as u8);
                    let mut bytes = text.into_bytes();
                    bytes.resize(padded, 0);
                    wire.extend(bytes);
                } else {
                    wire.push(0);
                }
            }
        }
        wire
    }

    #[actix_web::test]
    async fn pls_playlists_lead_to_an_icy_stream_with_clean_audio_and_titles() {
        let audio = mp3_frames(128, 12);
        let wire = icy_wire(&audio, 1000, "Chet Baker - Almost Blue");
        let fetch = FakeFetch::default()
            .with(
                "https://radio.example/listen.pls",
                FakeReply::ok(
                    "audio/x-scpls",
                    "[playlist]\nFile1=http://down.example/x\nFile2=http://ice.example:8000/jazz\n",
                ),
            )
            .with(
                "http://ice.example:8000/jazz",
                FakeReply::ok("audio/mpeg", Vec::new())
                    .header("icy-metaint", "1000")
                    .header("icy-br", "128")
                    .header("icy-name", "Jazz Example")
                    .chunked(wire.chunks(777).map(<[u8]>::to_vec).collect()),
            );
        let (client, fetch) = client(fetch);
        let (sink, titles) = sink();
        let mut station = station("d1a54d2e-623e-4970-ab11-35f7b56c5ec3", "jazz");
        station.stream_url = "https://radio.example/listen.pls".into();
        let source = open_source(&client, &station, sink)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(source.url.as_str(), "http://ice.example:8000/jazz");
        assert!(source.icy_metadata);
        assert_eq!(source.icy.name.as_deref(), Some("Jazz Example"));
        let (source, detected) = identify(source).await;
        let format = source_format(&source, detected.as_ref(), &station);
        assert!(format.verified);
        assert_eq!(format.codec.as_deref(), Some("mp3"));
        assert_eq!(format.bitrate_kbps, Some(128));
        let bytes: Vec<u8> = source
            .body
            .map(|chunk| chunk.expect("chunk").to_vec())
            .concat()
            .await;
        assert_eq!(bytes, audio, "metadata blocks are stripped exactly");
        assert_eq!(
            titles.lock().unwrap().as_slice(),
            ["Chet Baker - Almost Blue"]
        );
        // The dead first entry was tried, then the second.
        let urls = fetch.urls();
        assert!(urls.iter().any(|url| url.contains("ice.example")));
        assert!(
            urls.iter().all(|url| !url.contains("down.example")),
            "down.example does not resolve, so it is never fetched"
        );
        let requests = fetch.requests.lock().unwrap();
        assert!(requests.iter().all(|request| {
            request
                .headers
                .iter()
                .any(|(name, value)| name == "Icy-MetaData" && value == "1")
        }));
    }

    #[actix_web::test]
    async fn web_pages_and_private_addresses_are_not_streams() {
        let fetch = FakeFetch::default()
            .with(
                "https://radio.example/",
                FakeReply::ok("text/html", "<!doctype html><title>Radio</title>"),
            )
            .with(
                "https://radio.example/lan.m3u",
                FakeReply::ok("audio/x-mpegurl", "#EXTM3U\nhttp://lan.example/stream\n"),
            )
            .with(
                "http://lan.example/stream",
                FakeReply::ok("audio/aac", adts_frames(4)),
            );
        let (client, _) = client(fetch);
        let (sink, _) = sink();
        let mut page = station("d1a54d2e-623e-4970-ab11-35f7b56c5ec3", "page");
        page.stream_url = "https://radio.example/".into();
        let error = open_source(&client, &page, sink.clone())
            .await
            .err()
            .unwrap();
        assert_eq!(error.code(), "web_page_not_stream");

        let mut lan = station("d1a54d2e-623e-4970-ab11-35f7b56c5ec3", "lan");
        lan.stream_url = "https://radio.example/lan.m3u".into();
        let error = open_source(&client, &lan, sink.clone())
            .await
            .err()
            .unwrap();
        assert_eq!(error.code(), "stream_address_blocked");
        lan.allow_lan = true;
        let source = open_source(&client, &lan, sink).await.unwrap();
        assert_eq!(source.url.as_str(), "http://lan.example/stream");
    }

    fn format(container: Container, codec: &str, rate: Option<u32>) -> SourceFormat {
        SourceFormat {
            container,
            codec: Some(codec.into()),
            sample_rate: rate,
            bit_depth: None,
            channels: Some(2),
            bitrate_kbps: Some(320),
            verified: true,
        }
    }

    #[test]
    fn delivery_keeps_original_bytes_whenever_the_destination_plays_them() {
        let player = Destination::Player { accepts: None };
        assert_eq!(
            plan(&format(Container::Mp3, "mp3", None), &player),
            Delivery::Original
        );
        assert_eq!(
            plan(&format(Container::Ogg, "opus", None), &player),
            Delivery::Original
        );
        assert_eq!(
            plan(&format(Container::Ts, "aac", None), &player),
            Delivery::Repackage {
                output: Output::Adts
            }
        );
        assert_eq!(
            plan(&format(Container::Fmp4, "flac", Some(96_000)), &player),
            Delivery::Repackage {
                output: Output::Flac
            }
        );
        // Safari without Opus: decoded to FLAC, not re-encoded lossy.
        let safari = Destination::Player {
            accepts: Some(vec!["mp3".into(), "aac".into(), "flac".into()]),
        };
        assert_eq!(
            plan(&format(Container::Ogg, "opus", Some(48_000)), &safari),
            Delivery::Convert {
                output: Output::Flac,
                sample_rate: None,
                bit_depth: Some(24),
                bitrate_kbps: None
            }
        );

        let sonos = Destination::Renderer(FormatLimits::SONOS);
        assert_eq!(
            plan(&format(Container::Adts, "aac", None), &sonos),
            Delivery::Original
        );
        assert_eq!(
            plan(&format(Container::Flac, "flac", Some(96_000)), &sonos),
            Delivery::Convert {
                output: Output::Flac,
                sample_rate: Some(48_000),
                bit_depth: Some(24),
                bitrate_kbps: None
            }
        );
        assert_eq!(
            plan(&format(Container::Ogg, "vorbis", Some(44_100)), &sonos).output(),
            Some(Output::Flac)
        );
        let mp3_only = Destination::Renderer(FormatLimits {
            max_sample_rate: 48_000,
            max_bit_depth: 16,
            flac: false,
            wav: false,
            alac: false,
            mp3: true,
            aac: false,
            lpcm: false,
        });
        assert_eq!(
            plan(&format(Container::Adts, "aac", Some(44_100)), &mp3_only),
            Delivery::Convert {
                output: Output::Mp3,
                sample_rate: None,
                bit_depth: None,
                bitrate_kbps: Some(320)
            }
        );
    }

    #[test]
    fn ffmpeg_repackages_without_reencoding_and_converts_losslessly() {
        let args = ffmpeg_args(
            Delivery::Repackage {
                output: Output::Adts,
            },
            Container::Ts,
        )
        .unwrap();
        let joined = args.join(" ");
        assert!(joined.contains("-f mpegts -i pipe:0"), "{joined}");
        assert!(joined.contains("-c:a copy -f adts pipe:1"), "{joined}");
        let args = ffmpeg_args(
            Delivery::Convert {
                output: Output::Flac,
                sample_rate: Some(48_000),
                bit_depth: Some(24),
                bitrate_kbps: None,
            },
            Container::Ogg,
        )
        .unwrap()
        .join(" ");
        assert!(args.contains("aresample=48000"), "{args}");
        assert!(args.contains("-c:a flac"), "{args}");
        assert!(args.contains("-sample_fmt s32"), "{args}");
        assert!(ffmpeg_args(Delivery::Original, Container::Mp3).is_none());
    }

    #[actix_web::test]
    async fn ffmpeg_repackages_adts_when_available() {
        if std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_err()
        {
            return;
        }
        let input = futures::stream::iter(
            adts_frames(50)
                .chunks(500)
                .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
                .collect::<Vec<_>>(),
        )
        .boxed();
        let args = ffmpeg_args(
            Delivery::Convert {
                output: Output::Wav,
                sample_rate: None,
                bit_depth: Some(16),
                bitrate_kbps: None,
            },
            Container::Unknown,
        )
        .unwrap();
        let body = pipe_through_ffmpeg(input, args).expect("ffmpeg");
        // Synthetic frames are not real AAC; the pipeline must still end
        // cleanly instead of hanging.
        let finished =
            tokio::time::timeout(Duration::from_secs(20), body.collect::<Vec<_>>()).await;
        assert!(finished.is_ok());
    }

    /// A small HTTP server on loopback with station files, so the real
    /// fetcher (reqwest, pinned addresses, manual redirects) is exercised
    /// end to end, as when a listener adds a station by a playlist URL.
    async fn station_server(audio: Vec<u8>) -> (actix_web::dev::ServerHandle, String) {
        use actix_web::{App, HttpResponse, HttpServer, web};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let origin = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let files = Arc::new(vec![
            (
                "/listen.pls",
                "audio/x-scpls",
                format!(
                    "[playlist]\r\nNumberOfEntries=2\r\nFile1={origin}/gone\r\nTitle1=Jazz\r\nFile2={origin}/stream\r\nLength2=-1\r\nVersion=2\r\n"
                ),
            ),
            (
                "/listen.m3u",
                "audio/x-mpegurl",
                format!("#EXTM3U\n#EXTINF:-1,Jazz\n{origin}/stream\n"),
            ),
            (
                "/listen.m3u8",
                "application/vnd.apple.mpegurl",
                format!("#EXTM3U\n#EXTINF:-1,Jazz\n{origin}/stream\n"),
            ),
            (
                "/listen.asx",
                "video/x-ms-asf",
                format!(
                    "<ASX version=\"3.0\"><Entry><Title>Jazz</Title><Ref HREF = \"{origin}/stream\"/></Entry></ASX>"
                ),
            ),
            ("/nested.m3u", "audio/x-mpegurl", "listen.pls\n".to_string()),
            (
                "/loop.pls",
                "audio/x-scpls",
                "[playlist]\nFile1=loop.m3u\n".to_string(),
            ),
            ("/loop.m3u", "text/plain", "loop.pls\n".to_string()),
        ]);
        let audio = Arc::new(audio);
        let server = HttpServer::new(move || {
            let files = files.clone();
            let audio = audio.clone();
            App::new().default_service(web::to(move |request: actix_web::HttpRequest| {
                let files = files.clone();
                let audio = audio.clone();
                async move {
                    let path = request.path();
                    if path == "/stream" {
                        return HttpResponse::Ok()
                            .content_type("audio/mpeg")
                            .body(audio.as_ref().clone());
                    }
                    if path == "/moved" {
                        return HttpResponse::Found()
                            .insert_header(("Location", "/listen.pls"))
                            .finish();
                    }
                    match files.iter().find(|(name, _, _)| *name == path) {
                        Some((_, content_type, body)) => HttpResponse::Ok()
                            .content_type(*content_type)
                            .body(body.clone()),
                        None => HttpResponse::NotFound().finish(),
                    }
                }
            }))
        })
        .workers(1)
        .listen(listener)
        .expect("listen")
        .run();
        let handle = server.handle();
        actix_web::rt::spawn(server);
        (handle, origin)
    }

    #[actix_web::test]
    async fn playlist_urls_resolve_through_the_real_fetcher_without_panicking() {
        let audio = mp3_frames(128, 40);
        let (server, origin) = station_server(audio.clone()).await;
        let client = GuardedClient::system();
        for path in [
            "/listen.pls",
            "/listen.m3u",
            "/listen.m3u8",
            "/listen.asx",
            "/moved",
            "/nested.m3u",
        ] {
            let mut station = station("d1a54d2e-623e-4970-ab11-35f7b56c5ec3", "jazz");
            station.stream_url = format!("{origin}{path}");
            // Loopback is a LAN address: only a LAN station may reach it.
            station.allow_lan = true;
            let (sink, _) = sink();
            let source = open_source(&client, &station, sink)
                .await
                .unwrap_or_else(|error| panic!("{path}: {error}"));
            assert_eq!(source.url.as_str(), format!("{origin}/stream"), "{path}");
            let (source, detected) = identify(source).await;
            assert_eq!(
                detected.and_then(|found| found.codec).as_deref(),
                Some("mp3"),
                "{path}"
            );
            let mut body = source.body;
            let mut bytes = Vec::new();
            while let Some(chunk) = body.next().await {
                bytes.extend_from_slice(&chunk.expect("chunk"));
            }
            assert_eq!(bytes, audio, "{path}: the stream arrives whole");
            // Polling an ended body again is harmless.
            assert!(body.next().await.is_none());
            assert!(body.next().await.is_none());
        }

        // Playlists that only point at each other end, with an error.
        let mut looping = station("d1a54d2e-623e-4970-ab11-35f7b56c5ec3", "loop");
        looping.stream_url = format!("{origin}/loop.pls");
        looping.allow_lan = true;
        let (titles, _) = sink();
        assert!(matches!(
            open_source(&client, &looping, titles).await,
            Err(RelayError::NoStream)
        ));

        // The same playlist on a public station is refused: the guard
        // still checks every address, loopback included.
        let mut public = station("d1a54d2e-623e-4970-ab11-35f7b56c5ec3", "jazz");
        public.stream_url = format!("{origin}/listen.pls");
        let (titles, _) = sink();
        let error = open_source(&client, &public, titles)
            .await
            .err()
            .expect("blocked");
        assert_eq!(error.code(), "stream_address_blocked");
        server.stop(true).await;
    }

    #[actix_web::test]
    async fn playlists_cannot_redirect_or_point_a_public_station_into_the_lan() {
        let fetch = FakeFetch::default()
            .with(
                "https://radio.example/r.pls",
                FakeReply::redirect("http://lan.example/listen.pls"),
            )
            .with(
                "https://radio.example/p.pls",
                FakeReply::ok(
                    "audio/x-scpls",
                    "[playlist]\nFile1=http://127.0.0.1:8000/admin\nFile2=http://[::1]/x\nFile3=http://lan.example/x\n",
                ),
            );
        let (client, fetch) = client(fetch);
        for url in ["https://radio.example/r.pls", "https://radio.example/p.pls"] {
            let mut station = station("d1a54d2e-623e-4970-ab11-35f7b56c5ec3", "jazz");
            station.stream_url = url.into();
            let (sink, _) = sink();
            let error = open_source(&client, &station, sink)
                .await
                .err()
                .expect("blocked");
            assert_eq!(error.code(), "stream_address_blocked", "{url}");
        }
        assert!(
            fetch
                .urls()
                .iter()
                .all(|url| url.starts_with("https://radio.example/")),
            "nothing on the LAN was fetched: {:?}",
            fetch.urls()
        );
    }

    #[actix_web::test]
    async fn malformed_playlists_from_stations_never_panic_the_relay() {
        let names = ["f.pls", "f.m3u", "f.m3u8", "f.asx", "f.xspf", "f"];
        let types = [
            "audio/x-scpls",
            "audio/x-mpegurl",
            "application/vnd.apple.mpegurl",
            "video/x-ms-asf",
            "text/plain",
            "application/octet-stream",
        ];
        let cases = crate::live_radio::playlist::tests::malformed_playlists(240);
        let mut fetch = FakeFetch::default();
        for (index, bytes) in cases.iter().enumerate() {
            let url = format!(
                "https://radio.example/{index}/{}",
                names[index % names.len()]
            );
            let split = 1 + index % 37;
            fetch = fetch.with(
                &url,
                FakeReply::ok(types[index % types.len()], Vec::new())
                    .chunked(bytes.chunks(split).map(<[u8]>::to_vec).collect()),
            );
        }
        let (client, _) = client(fetch);
        for (index, _) in cases.iter().enumerate() {
            let mut station = station("d1a54d2e-623e-4970-ab11-35f7b56c5ec3", "fuzz");
            station.stream_url = format!(
                "https://radio.example/{index}/{}",
                names[index % names.len()]
            );
            station.hls = index % 11 == 0;
            let (sink, _) = sink();
            // Live HLS keeps polling, so each case gets a short look.
            let _ = tokio::time::timeout(std::time::Duration::from_millis(250), async {
                if let Ok(source) = open_source(&client, &station, sink).await {
                    let (source, _) = identify(source).await;
                    let mut body = source.body;
                    while body.next().await.is_some() {}
                    assert!(body.next().await.is_none());
                }
            })
            .await;
        }
    }
}
