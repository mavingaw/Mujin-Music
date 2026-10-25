//! Sandbox streams in the formats the services really deliver.
//!
//! In sandbox mode Parson has no TIDAL or Qobuz account to stream from, so
//! this module stands in for the two parts of the services that hand out
//! audio:
//!
//! - [`SandboxTransport`] answers TIDAL `playbackinfopostpaywall` and Qobuz
//!   `track/getFileUrl` the way the services do: TIDAL Max as an MPEG-DASH
//!   manifest of fragmented-MP4 FLAC segments, TIDAL High as a FLAC file,
//!   TIDAL's AAC tiers as MP4 files, Qobuz Hi-Res and CD as FLAC files and
//!   Qobuz MP3 as an MP3 file, each at the best format the recorded track is
//!   offered in. Every other API call goes to the recorded fixtures.
//! - [`SandboxMedia`] serves those files. They are the sandbox tone,
//!   rendered by ffmpeg at the announced bit depth and sample rate, so a
//!   24/192 stream really carries 24-bit samples at 192 kHz.
//!
//! The real TIDAL and Qobuz clients parse these replies and the real media
//! proxy streams, joins, seeks and probes the files, so the delivered-format
//! report checks the same thing it checks for a live account: whether the
//! bytes that arrived match what the service announced. The audio itself is
//! synthesized, and the UI says so.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use futures::future::BoxFuture;
use serde_json::{Value, json};

use super::media::{MediaError, MediaResponse, MediaTransport, serve_file};
use super::sandbox::{PREVIEW_SECONDS, Tone, seed};
use super::transport::{ApiReply, ApiRequest, FixtureTransport, Transport, TransportError};
use super::{ProviderId, valid_raw_id};

/// Where sandbox media "lives". `.invalid` never resolves, so nothing can
/// leave the server even if a URL escaped.
pub const MEDIA_HOST: &str = "https://media.sandbox.parson.invalid";
const TIDAL_PLAYBACK_PREFIX: &str = "https://api.tidal.com/v1/tracks/";
const TIDAL_PLAYBACK_SUFFIX: &str = "/playbackinfopostpaywall";
const QOBUZ_FILE_URL: &str = "https://www.qobuz.com/api.json/0.2/track/getFileUrl";
/// Rendered streams kept on disk; older ones are removed.
const MAX_CACHED_STREAMS: usize = 16;
const DASH_SEGMENT_SECONDS: u32 = 4;

/// One way of encoding a sandbox stream. Only these exist, so a request
/// cannot make the server render arbitrary formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rendition {
    Flac { bits: u8, rate: u32 },
    DashFlac { bits: u8, rate: u32 },
    Aac { kbps: u32 },
    Mp3 { kbps: u32 },
}

const RENDITIONS: [Rendition; 8] = [
    Rendition::Flac {
        bits: 24,
        rate: 192_000,
    },
    Rendition::Flac {
        bits: 24,
        rate: 96_000,
    },
    Rendition::Flac {
        bits: 16,
        rate: 44_100,
    },
    Rendition::DashFlac {
        bits: 24,
        rate: 192_000,
    },
    Rendition::DashFlac {
        bits: 24,
        rate: 96_000,
    },
    Rendition::Aac { kbps: 320 },
    Rendition::Aac { kbps: 96 },
    Rendition::Mp3 { kbps: 320 },
];

impl Rendition {
    pub fn slug(self) -> String {
        match self {
            Self::Flac { bits, rate } => format!("flac-{bits}-{rate}"),
            Self::DashFlac { bits, rate } => format!("dash-{bits}-{rate}"),
            Self::Aac { kbps } => format!("aac-{kbps}"),
            Self::Mp3 { kbps } => format!("mp3-{kbps}"),
        }
    }

    pub fn parse(slug: &str) -> Option<Self> {
        RENDITIONS
            .into_iter()
            .find(|rendition| rendition.slug() == slug)
    }

    /// The file a stream starts from: the whole file, or the DASH manifest.
    pub fn main_file(self) -> &'static str {
        match self {
            Self::Flac { .. } => "audio.flac",
            Self::DashFlac { .. } => "manifest.mpd",
            Self::Aac { .. } => "audio.m4a",
            Self::Mp3 { .. } => "audio.mp3",
        }
    }

    fn rate(self) -> u32 {
        match self {
            Self::Flac { rate, .. } | Self::DashFlac { rate, .. } => rate,
            Self::Aac { .. } | Self::Mp3 { .. } => 44_100,
        }
    }

    fn ffmpeg_output(self, dir: &Path) -> Vec<String> {
        let flac = |bits: u8| {
            let mut args = vec!["-c:a".to_string(), "flac".to_string()];
            if bits > 16 {
                args.extend(["-sample_fmt", "s32", "-bits_per_raw_sample", "24"].map(String::from));
            } else {
                args.extend(["-sample_fmt", "s16"].map(String::from));
            }
            args
        };
        let path = dir.join(self.main_file()).to_string_lossy().into_owned();
        let mut args = match self {
            Self::Flac { bits, .. } => {
                let mut args = flac(bits);
                args.extend(["-f".to_string(), "flac".to_string()]);
                args
            }
            Self::DashFlac { bits, .. } => {
                let mut args = flac(bits);
                args.extend(
                    [
                        "-f",
                        "dash",
                        "-seg_duration",
                        &DASH_SEGMENT_SECONDS.to_string(),
                        "-use_template",
                        "1",
                        "-use_timeline",
                        "1",
                        "-init_seg_name",
                        "init.mp4",
                        "-media_seg_name",
                        "seg-$Number$.mp4",
                    ]
                    .map(String::from),
                );
                args
            }
            Self::Aac { kbps } => [
                "-c:a",
                "aac",
                "-b:a",
                &format!("{kbps}k"),
                "-movflags",
                "+faststart",
                "-f",
                "mp4",
            ]
            .map(String::from)
            .to_vec(),
            Self::Mp3 { kbps } => [
                "-c:a",
                "libmp3lame",
                "-b:a",
                &format!("{kbps}k"),
                "-f",
                "mp3",
            ]
            .map(String::from)
            .to_vec(),
        };
        args.push(path);
        args
    }
}

/// The best lossless format a recorded track is offered in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Offered {
    pub bits: u8,
    pub rate: u32,
}

const CD: Offered = Offered {
    bits: 16,
    rate: 44_100,
};

/// Reads each recorded track's best format: Qobuz `maximum_bit_depth` and
/// `maximum_sampling_rate`, TIDAL `HIRES_LOSSLESS` media tags (TIDAL Max
/// streams as 24/192 here).
fn offered_formats(fixtures: &[&str]) -> HashMap<String, Offered> {
    fn walk(value: &Value, offered: &mut HashMap<String, Offered>) {
        match value {
            Value::Object(object) => {
                let id = match object.get("id") {
                    Some(Value::String(id)) => Some(id.clone()),
                    Some(Value::Number(id)) => Some(id.to_string()),
                    _ => None,
                };
                if let Some(id) = id {
                    if (object.contains_key("track_number") || object.contains_key("isrc"))
                        && let (Some(bits), Some(khz)) = (
                            object.get("maximum_bit_depth").and_then(Value::as_u64),
                            object.get("maximum_sampling_rate").and_then(Value::as_f64),
                        )
                    {
                        offered.insert(
                            format!("qobuz:{id}"),
                            Offered {
                                bits: bits.min(24) as u8,
                                rate: (khz * 1000.0).round() as u32,
                            },
                        );
                    }
                    if object.contains_key("trackNumber") || object.contains_key("isrc") {
                        let tags = object
                            .get("mediaMetadata")
                            .and_then(|metadata| metadata.get("tags"))
                            .and_then(Value::as_array);
                        if let Some(tags) = tags {
                            let hires = tags
                                .iter()
                                .any(|tag| tag.as_str() == Some("HIRES_LOSSLESS"));
                            offered.insert(
                                format!("tidal:{id}"),
                                if hires {
                                    Offered {
                                        bits: 24,
                                        rate: 192_000,
                                    }
                                } else {
                                    CD
                                },
                            );
                        }
                    }
                }
                for child in object.values() {
                    walk(child, offered);
                }
            }
            Value::Array(items) => items.iter().for_each(|item| walk(item, offered)),
            _ => {}
        }
    }
    let mut offered = HashMap::new();
    for fixture in fixtures {
        if let Ok(value) = serde_json::from_str::<Value>(fixture) {
            walk(&value, &mut offered);
        }
    }
    offered
}

/// Whether `ffmpeg` can be run. Checked once.
pub fn ffmpeg_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

/// Renders and caches sandbox streams.
pub struct SandboxStreams {
    root: PathBuf,
    offered: HashMap<String, Offered>,
    render: tokio::sync::Mutex<()>,
}

impl SandboxStreams {
    pub fn new(root: PathBuf, fixtures: &[&str]) -> Self {
        Self {
            root,
            offered: offered_formats(fixtures),
            render: tokio::sync::Mutex::new(()),
        }
    }

    /// Streams for the embedded sandbox fixtures, cached under the system
    /// temporary directory.
    pub fn for_sandbox() -> Self {
        Self::for_sandbox_in(std::env::temp_dir().join("parson-sandbox-streams"))
    }

    pub fn for_sandbox_in(root: PathBuf) -> Self {
        Self::new(
            root,
            &[
                super::sandbox::TIDAL_FIXTURES,
                super::sandbox::QOBUZ_FIXTURES,
                super::sandbox::TIDAL_BROWSE_FIXTURES,
                super::sandbox::QOBUZ_BROWSE_FIXTURES,
                super::sandbox::TIDAL_NOTES_FIXTURES,
                super::sandbox::QOBUZ_NOTES_FIXTURES,
            ],
        )
    }

    pub fn offered(&self, provider: ProviderId, raw_id: &str) -> Offered {
        self.offered
            .get(&super::entity_id(provider, raw_id))
            .copied()
            .unwrap_or(CD)
    }

    fn directory(&self, provider: ProviderId, raw_id: &str, rendition: Rendition) -> PathBuf {
        self.root.join(format!(
            "{}-{raw_id}-{}",
            provider.as_str(),
            rendition.slug()
        ))
    }

    pub fn url(provider: ProviderId, raw_id: &str, rendition: Rendition, file: &str) -> String {
        format!(
            "{MEDIA_HOST}/{}/{raw_id}/{}/{file}",
            provider.as_str(),
            rendition.slug()
        )
    }

    /// Renders a stream once and returns its directory.
    pub async fn ensure(
        &self,
        provider: ProviderId,
        raw_id: &str,
        rendition: Rendition,
    ) -> std::io::Result<PathBuf> {
        if !valid_raw_id(raw_id) {
            return Err(std::io::Error::other("invalid track id"));
        }
        let directory = self.directory(provider, raw_id, rendition);
        if directory.join(rendition.main_file()).is_file() {
            return Ok(directory);
        }
        let _guard = self.render.lock().await;
        if directory.join(rendition.main_file()).is_file() {
            return Ok(directory);
        }
        tokio::fs::create_dir_all(&self.root).await?;
        let staging = directory.with_extension(format!("partial-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&staging).await;
        tokio::fs::create_dir_all(&staging).await?;
        let tone_seed = seed(raw_id);
        let target = staging.clone();
        let rendered =
            tokio::task::spawn_blocking(move || render(&target, rendition, tone_seed)).await;
        match rendered {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                let _ = tokio::fs::remove_dir_all(&staging).await;
                return Err(error);
            }
            Err(error) => {
                let _ = tokio::fs::remove_dir_all(&staging).await;
                return Err(std::io::Error::other(error.to_string()));
            }
        }
        let _ = tokio::fs::remove_dir_all(&directory).await;
        tokio::fs::rename(&staging, &directory).await?;
        self.prune(&directory);
        Ok(directory)
    }

    fn prune(&self, keep: &Path) {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return;
        };
        let mut streams = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path() != keep && entry.path().is_dir())
            .filter_map(|entry| {
                let modified = entry.metadata().ok()?.modified().ok()?;
                Some((modified, entry.path()))
            })
            .collect::<Vec<_>>();
        if streams.len() < MAX_CACHED_STREAMS {
            return;
        }
        streams.sort();
        for (_, path) in streams.iter().take(streams.len() + 1 - MAX_CACHED_STREAMS) {
            let _ = std::fs::remove_dir_all(path);
        }
    }

    /// Answers the two API calls that hand out audio; `None` for all others.
    pub async fn answer(&self, request: &ApiRequest) -> Option<Result<ApiReply, TransportError>> {
        if let Some(raw_id) = request
            .url
            .strip_prefix(TIDAL_PLAYBACK_PREFIX)
            .and_then(|rest| rest.strip_suffix(TIDAL_PLAYBACK_SUFFIX))
        {
            return Some(self.tidal_playback(raw_id, request).await);
        }
        if request.url == QOBUZ_FILE_URL {
            return Some(self.qobuz_file_url(request));
        }
        None
    }

    async fn tidal_playback(
        &self,
        raw_id: &str,
        request: &ApiRequest,
    ) -> Result<ApiReply, TransportError> {
        if !raw_id.bytes().all(|byte| byte.is_ascii_digit()) || raw_id.is_empty() {
            return Ok(reply(
                404,
                json!({ "status": 404, "userMessage": "Not found" }),
            ));
        }
        if request.header_value("authorization").is_none() {
            return Ok(reply(401, json!({ "status": 401, "subStatus": 11002 })));
        }
        let offered = self.offered(ProviderId::Tidal, raw_id);
        let asked = request.query_value("audioquality").unwrap_or("HIGH");
        let track_id: u64 = raw_id.parse().unwrap_or_default();
        let bts = |audio_quality: &str, mime: &str, codecs: &str, rendition: Rendition| {
            let manifest = json!({
                "mimeType": mime,
                "codecs": codecs,
                "encryptionType": "NONE",
                "urls": [SandboxStreams::url(
                    ProviderId::Tidal,
                    raw_id,
                    rendition,
                    rendition.main_file(),
                )],
            });
            let mut body = json!({
                "trackId": track_id,
                "assetPresentation": "FULL",
                "audioMode": "STEREO",
                "audioQuality": audio_quality,
                "manifestMimeType": "application/vnd.tidal.bts",
                "manifest": STANDARD.encode(manifest.to_string()),
            });
            if let Rendition::Flac { bits, rate } = rendition {
                body["bitDepth"] = json!(bits);
                body["sampleRate"] = json!(rate);
            }
            reply(200, body)
        };
        match asked {
            "HI_RES_LOSSLESS" if offered.bits > 16 => {
                let rendition = Rendition::DashFlac {
                    bits: offered.bits,
                    rate: offered.rate,
                };
                let directory = self
                    .ensure(ProviderId::Tidal, raw_id, rendition)
                    .await
                    .map_err(render_error)?;
                let mpd = std::fs::read_to_string(directory.join(rendition.main_file()))
                    .map_err(render_error)?;
                let base = SandboxStreams::url(ProviderId::Tidal, raw_id, rendition, "");
                let mpd = mpd
                    .replace(
                        "initialization=\"init.mp4\"",
                        &format!("initialization=\"{base}init.mp4\""),
                    )
                    .replace(
                        "media=\"seg-$Number$.mp4\"",
                        &format!("media=\"{base}seg-$Number$.mp4\""),
                    );
                Ok(reply(
                    200,
                    json!({
                        "trackId": track_id,
                        "assetPresentation": "FULL",
                        "audioMode": "STEREO",
                        "audioQuality": "HI_RES_LOSSLESS",
                        "manifestMimeType": "application/dash+xml",
                        "manifest": STANDARD.encode(mpd),
                        "bitDepth": offered.bits,
                        "sampleRate": offered.rate,
                    }),
                ))
            }
            "HI_RES_LOSSLESS" | "LOSSLESS" => Ok(bts(
                "LOSSLESS",
                "audio/flac",
                "flac",
                Rendition::Flac {
                    bits: 16,
                    rate: 44_100,
                },
            )),
            "LOW" => Ok(bts(
                "LOW",
                "audio/mp4",
                "mp4a.40.5",
                Rendition::Aac { kbps: 96 },
            )),
            _ => Ok(bts(
                "HIGH",
                "audio/mp4",
                "mp4a.40.2",
                Rendition::Aac { kbps: 320 },
            )),
        }
    }

    fn qobuz_file_url(&self, request: &ApiRequest) -> Result<ApiReply, TransportError> {
        let Some(raw_id) = request
            .query_value("track_id")
            .filter(|raw_id| valid_raw_id(raw_id))
        else {
            return Ok(reply(
                400,
                json!({ "status": "error", "code": 400, "message": "track_id" }),
            ));
        };
        // Qobuz refuses unsigned or unauthenticated file requests.
        if request.query_value("request_sig").is_none_or(str::is_empty)
            || request.query_value("request_ts").is_none()
        {
            return Ok(reply(
                400,
                json!({ "status": "error", "code": 400, "message": "Invalid Request Signature parameter (request_sig)" }),
            ));
        }
        if request.header_value("x-user-auth-token").is_none() {
            return Ok(reply(
                401,
                json!({ "status": "error", "code": 401, "message": "User authentication is required." }),
            ));
        }
        let offered = self.offered(ProviderId::Qobuz, raw_id);
        let asked = request
            .query_value("format_id")
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(5);
        let (rendition, format_id) = match asked {
            27 | 7 if offered.bits > 16 => {
                let rate = if asked == 7 {
                    offered.rate.min(96_000)
                } else {
                    offered.rate
                };
                let rate = if rate > 96_000 { 192_000 } else { 96_000 };
                (
                    Rendition::Flac {
                        bits: offered.bits,
                        rate,
                    },
                    if rate > 96_000 { 27 } else { 7 },
                )
            }
            27 | 7 | 6 => (
                Rendition::Flac {
                    bits: 16,
                    rate: 44_100,
                },
                6,
            ),
            _ => (Rendition::Mp3 { kbps: 320 }, 5),
        };
        let mut body = json!({
            "track_id": raw_id.parse::<u64>().map_or_else(|_| json!(raw_id), |id| json!(id)),
            "duration": PREVIEW_SECONDS,
            "url": SandboxStreams::url(ProviderId::Qobuz, raw_id, rendition, rendition.main_file()),
            "format_id": format_id,
            "restrictions": if format_id < asked {
                json!([{ "code": "FormatRestrictedByFormatAvailability" }])
            } else {
                json!([])
            },
        });
        match rendition {
            Rendition::Flac { bits, rate } => {
                body["mime_type"] = json!("audio/flac");
                body["bit_depth"] = json!(bits);
                body["sampling_rate"] = json!(f64::from(rate) / 1000.0);
            }
            _ => body["mime_type"] = json!("audio/mpeg"),
        }
        Ok(reply(200, body))
    }

    /// Reads a rendered file named by a sandbox media URL.
    async fn file(&self, url: &str) -> Result<Bytes, MediaError> {
        let path = url
            .strip_prefix(MEDIA_HOST)
            .and_then(|rest| rest.strip_prefix('/'))
            .ok_or(MediaError::Status(404))?;
        let parts = path.split('/').collect::<Vec<_>>();
        let [provider, raw_id, slug, file] = parts[..] else {
            return Err(MediaError::Status(404));
        };
        let provider = ProviderId::parse(provider).ok_or(MediaError::Status(404))?;
        let rendition = Rendition::parse(slug).ok_or(MediaError::Status(404))?;
        let safe_file = !file.is_empty()
            && file.len() <= 32
            && !file.starts_with('.')
            && file
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.');
        if !valid_raw_id(raw_id) || !safe_file || file.ends_with(".mpd") {
            return Err(MediaError::Status(404));
        }
        let directory = self
            .ensure(provider, raw_id, rendition)
            .await
            .map_err(|error| MediaError::Read(error.to_string()))?;
        match tokio::fs::read(directory.join(file)).await {
            Ok(bytes) => Ok(Bytes::from(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(MediaError::Status(404))
            }
            Err(error) => Err(MediaError::Read(error.to_string())),
        }
    }
}

fn reply(status: u16, body: Value) -> ApiReply {
    ApiReply {
        status,
        body: serde_json::to_vec(&body).unwrap_or_default(),
    }
}

fn render_error(error: std::io::Error) -> TransportError {
    TransportError::Connect(format!("sandbox audio could not be rendered: {error}"))
}

/// Pipes the tone into ffmpeg at the rendition's rate, as 32-bit float
/// stereo, and lets ffmpeg encode and package it.
fn render(directory: &Path, rendition: Rendition, tone_seed: u64) -> std::io::Result<()> {
    let rate = rendition.rate();
    let mut child = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "f32le", "-ar", &rate.to_string(), "-ac", "2"])
        .args(["-i", "pipe:0"])
        .args(rendition.ffmpeg_output(directory))
        .current_dir(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let tone = Tone::new(tone_seed, PREVIEW_SECONDS, rate);
    if let Some(mut stdin) = child.stdin.take() {
        const CHUNK: usize = 16_384;
        let mut buffer = Vec::with_capacity(CHUNK * 8);
        let mut index = 0;
        while index < tone.frames() {
            buffer.clear();
            for frame in index..(index + CHUNK).min(tone.frames()) {
                let (left, right) = tone.frame(frame);
                buffer.extend_from_slice(&left.to_le_bytes());
                buffer.extend_from_slice(&right.to_le_bytes());
            }
            if let Err(error) = stdin.write_all(&buffer) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
            index += CHUNK;
        }
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        return Err(std::io::Error::other(format!(
            "ffmpeg failed: {}",
            message.trim().chars().take(300).collect::<String>()
        )));
    }
    if !directory.join(rendition.main_file()).is_file() {
        return Err(std::io::Error::other("ffmpeg wrote no output"));
    }
    Ok(())
}

/// The recorded fixtures, with the services' audio hand-out calls answered
/// by [`SandboxStreams`].
pub struct SandboxTransport {
    fixtures: FixtureTransport,
    streams: Arc<SandboxStreams>,
}

impl SandboxTransport {
    pub fn new(fixtures: FixtureTransport, streams: Arc<SandboxStreams>) -> Self {
        Self { fixtures, streams }
    }
}

impl Transport for SandboxTransport {
    fn send(&self, request: ApiRequest) -> BoxFuture<'_, Result<ApiReply, TransportError>> {
        Box::pin(async move {
            if let Some(reply) = self.streams.answer(&request).await {
                return reply;
            }
            self.fixtures.send(request).await
        })
    }
}

/// Serves rendered sandbox media files, honouring byte ranges.
pub struct SandboxMedia {
    streams: Arc<SandboxStreams>,
}

impl SandboxMedia {
    pub fn new(streams: Arc<SandboxStreams>) -> Self {
        Self { streams }
    }
}

impl MediaTransport for SandboxMedia {
    fn open(
        &self,
        url: String,
        range: Option<String>,
    ) -> BoxFuture<'static, Result<MediaResponse, MediaError>> {
        let streams = self.streams.clone();
        Box::pin(async move { serve_file(streams.file(&url).await?, range) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renditions_round_trip_and_reject_others() {
        for rendition in RENDITIONS {
            assert_eq!(Rendition::parse(&rendition.slug()), Some(rendition));
        }
        assert_eq!(Rendition::parse("flac-32-384000"), None);
        assert_eq!(Rendition::parse("../etc"), None);
    }

    #[test]
    fn offered_formats_come_from_the_recorded_tracks() {
        let streams = SandboxStreams::for_sandbox();
        assert_eq!(
            streams.offered(ProviderId::Tidal, "300101"),
            Offered {
                bits: 24,
                rate: 192_000
            }
        );
        assert_eq!(
            streams.offered(ProviderId::Qobuz, "7700001"),
            Offered {
                bits: 24,
                rate: 192_000
            }
        );
        assert_eq!(streams.offered(ProviderId::Qobuz, "no-such-track"), CD);
    }

    #[tokio::test]
    async fn media_urls_outside_the_sandbox_layout_are_not_found() {
        let streams = SandboxStreams::new(std::env::temp_dir().join("parson-sandbox-none"), &[]);
        for url in [
            "https://example.test/a",
            "https://media.sandbox.parson.invalid/tidal/1/flac-16-44100/../../x",
            "https://media.sandbox.parson.invalid/tidal/1/flac-16-44100/.hidden",
            "https://media.sandbox.parson.invalid/tidal/1/flac-32-384000/audio.flac",
            "https://media.sandbox.parson.invalid/spotify/1/flac-16-44100/audio.flac",
            "https://media.sandbox.parson.invalid/tidal/1/dash-24-96000/manifest.mpd",
        ] {
            assert_eq!(
                streams.file(url).await.err(),
                Some(MediaError::Status(404)),
                "{url}"
            );
        }
    }

    #[tokio::test]
    async fn unsigned_qobuz_requests_are_refused() {
        let streams = SandboxStreams::for_sandbox();
        let request = ApiRequest::get(QOBUZ_FILE_URL)
            .query("track_id", "7700001")
            .query("format_id", "27");
        let reply = streams
            .answer(&request)
            .await
            .expect("handled")
            .expect("reply");
        assert_eq!(reply.status, 400);
    }
}
