//! Fetches stream bytes for the media proxy.
//!
//! Like [`super::transport::Transport`] for API calls, the media proxy reads
//! audio through [`MediaTransport`], so tests and the stream check can play
//! recorded media files through the exact proxy path used for real streams.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use futures::future::BoxFuture;
use futures::stream::{BoxStream, StreamExt};

use super::probe::{PROBE_BYTES, ProbedFormat, probe};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaError {
    Connect(String),
    Status(u16),
    Read(String),
}

impl std::fmt::Display for MediaError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(message) => {
                write!(formatter, "could not reach the media host: {message}")
            }
            Self::Status(status) => write!(formatter, "the media host answered HTTP {status}"),
            Self::Read(message) => write!(formatter, "the media stream broke off: {message}"),
        }
    }
}

impl std::error::Error for MediaError {}

pub type MediaBody = BoxStream<'static, Result<Bytes, MediaError>>;

pub struct MediaResponse {
    pub status: u16,
    pub content_length: Option<u64>,
    pub content_range: Option<String>,
    pub body: MediaBody,
}

pub trait MediaTransport: Send + Sync {
    /// Opens `url`, forwarding a `Range` header when given. Non-success
    /// statuses are errors.
    fn open(
        &self,
        url: String,
        range: Option<String>,
    ) -> BoxFuture<'static, Result<MediaResponse, MediaError>>;
}

pub struct ReqwestMedia {
    client: reqwest::Client,
}

impl ReqwestMedia {
    pub fn new() -> Result<Self, reqwest::Error> {
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(6))
                .read_timeout(Duration::from_secs(30))
                .user_agent(concat!("Parson/", env!("CARGO_PKG_VERSION")))
                .build()?,
        })
    }
}

impl MediaTransport for ReqwestMedia {
    fn open(
        &self,
        url: String,
        range: Option<String>,
    ) -> BoxFuture<'static, Result<MediaResponse, MediaError>> {
        let client = self.client.clone();
        Box::pin(async move {
            let mut request = client.get(&url);
            if let Some(range) = range {
                request = request.header(reqwest::header::RANGE, range);
            }
            let response = request
                .send()
                .await
                .map_err(|error| MediaError::Connect(error.without_url().to_string()))?;
            let status = response.status().as_u16();
            if !(200..300).contains(&status) {
                return Err(MediaError::Status(status));
            }
            let header = |name: reqwest::header::HeaderName| {
                response
                    .headers()
                    .get(name)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string)
            };
            let content_length =
                header(reqwest::header::CONTENT_LENGTH).and_then(|value| value.parse::<u64>().ok());
            let content_range = header(reqwest::header::CONTENT_RANGE);
            let body = futures::stream::unfold(Some(response), |state| async move {
                let mut response = state?;
                match response.chunk().await {
                    Ok(Some(chunk)) => Some((Ok(chunk), Some(response))),
                    Ok(None) => None,
                    Err(error) => {
                        Some((Err(MediaError::Read(error.without_url().to_string())), None))
                    }
                }
            })
            .boxed();
            Ok(MediaResponse {
                status,
                content_length,
                content_range,
                body,
            })
        })
    }
}

/// Parses a single `bytes=` range against a known length.
pub fn byte_range(header: Option<&str>, length: u64) -> Option<(u64, u64)> {
    let spec = header?.trim().strip_prefix("bytes=")?;
    if spec.contains(',') || length == 0 {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    let (start, end) = if start.is_empty() {
        let suffix = end.parse::<u64>().ok()?.min(length);
        (length - suffix, length - 1)
    } else {
        let start = start.parse::<u64>().ok()?;
        let end = if end.is_empty() {
            length - 1
        } else {
            end.parse::<u64>().ok()?.min(length - 1)
        };
        (start, end)
    };
    (start <= end && start < length).then_some((start, end))
}

/// Whether a request's `Range` starts at the first byte, so the response
/// carries the stream's headers and can be probed.
pub fn starts_at_beginning(range: Option<&str>) -> bool {
    range.is_none_or(|range| {
        range
            .trim()
            .strip_prefix("bytes=")
            .is_some_and(|spec| spec.starts_with("0-"))
    })
}

/// Serves recorded media files by URL, honouring byte ranges, and records
/// every request so tests can check what the proxy asked for.
#[derive(Default)]
pub struct FixtureMedia {
    files: HashMap<String, Bytes>,
    requests: Mutex<Vec<(String, Option<String>)>>,
}

impl FixtureMedia {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, url: &str, body: impl Into<Bytes>) -> Self {
        self.files.insert(url.to_string(), body.into());
        self
    }

    pub fn requests(&self) -> Vec<(String, Option<String>)> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .unwrap_or_default()
    }
}

impl MediaTransport for FixtureMedia {
    fn open(
        &self,
        url: String,
        range: Option<String>,
    ) -> BoxFuture<'static, Result<MediaResponse, MediaError>> {
        if let Ok(mut requests) = self.requests.lock() {
            requests.push((url.clone(), range.clone()));
        }
        let file = self.files.get(&url).cloned();
        Box::pin(async move {
            let file = file.ok_or(MediaError::Status(404))?;
            let length = file.len() as u64;
            let (status, body, content_range) = match range.as_deref() {
                Some(header) => {
                    let (start, end) =
                        byte_range(Some(header), length).ok_or(MediaError::Status(416))?;
                    (
                        206,
                        file.slice(start as usize..=end as usize),
                        Some(format!("bytes {start}-{end}/{length}")),
                    )
                }
                None => (200, file, None),
            };
            // Deliver in small chunks, as a network would.
            let chunks = body
                .chunks(4096)
                .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
                .collect::<Vec<_>>();
            Ok(MediaResponse {
                status,
                content_length: Some(body.len() as u64),
                content_range,
                body: futures::stream::iter(chunks).boxed(),
            })
        })
    }
}

/// Plays DASH segments back-to-back as one body.
pub fn segmented(media: Arc<dyn MediaTransport>, urls: Vec<String>) -> MediaBody {
    futures::stream::unfold(
        (urls.into_iter(), None::<MediaBody>),
        move |(mut urls, mut current)| {
            let media = media.clone();
            async move {
                loop {
                    if let Some(mut body) = current.take() {
                        match body.next().await {
                            Some(Ok(chunk)) => return Some((Ok(chunk), (urls, Some(body)))),
                            Some(Err(error)) => {
                                return Some((Err(error), (Vec::new().into_iter(), None)));
                            }
                            None => {}
                        }
                    }
                    let url = urls.next()?;
                    match media.open(url, None).await {
                        Ok(response) => current = Some(response.body),
                        Err(error) => {
                            return Some((Err(error), (Vec::new().into_iter(), None)));
                        }
                    }
                }
            }
        },
    )
    .boxed()
}

/// Passes a body through unchanged while reading its first bytes; calls
/// `on_probe` once with the delivered format (or `None` if it could not be
/// read) as soon as it is known for certain.
pub fn tap(
    body: MediaBody,
    on_probe: impl FnOnce(Option<ProbedFormat>) + Send + 'static,
) -> MediaBody {
    struct Tap<F: FnOnce(Option<ProbedFormat>)> {
        buffer: Vec<u8>,
        on_probe: Option<F>,
    }
    impl<F: FnOnce(Option<ProbedFormat>)> Tap<F> {
        fn feed(&mut self, chunk: &[u8]) {
            if self.on_probe.is_none() {
                return;
            }
            let room = PROBE_BYTES.saturating_sub(self.buffer.len());
            self.buffer
                .extend_from_slice(&chunk[..chunk.len().min(room)]);
            let probed = probe(&self.buffer);
            if probed.as_ref().is_some_and(|probed| probed.frame_verified)
                || self.buffer.len() >= PROBE_BYTES
            {
                self.finish(probed);
            }
        }

        fn finish(&mut self, probed: Option<ProbedFormat>) {
            if let Some(on_probe) = self.on_probe.take() {
                on_probe(probed);
                self.buffer = Vec::new();
            }
        }
    }
    impl<F: FnOnce(Option<ProbedFormat>)> Drop for Tap<F> {
        fn drop(&mut self) {
            // The listener stopped or skipped before the probe window
            // filled: report what arrived.
            let probed = probe(&self.buffer);
            self.finish(probed);
        }
    }
    let mut state = Tap {
        buffer: Vec::new(),
        on_probe: Some(on_probe),
    };
    let mut body = body;
    futures::stream::poll_fn(move |context| {
        let polled = body.poll_next_unpin(context);
        match &polled {
            std::task::Poll::Ready(Some(Ok(chunk))) => state.feed(chunk),
            std::task::Poll::Ready(None) | std::task::Poll::Ready(Some(Err(_))) => {
                let probed = probe(&state.buffer);
                state.finish(probed);
            }
            std::task::Poll::Pending => {}
        }
        polled
    })
    .boxed()
}

/// Reads up to [`PROBE_BYTES`] from a body and probes them.
pub async fn probe_body(mut body: MediaBody) -> Result<Option<ProbedFormat>, MediaError> {
    let mut buffer = Vec::new();
    while buffer.len() < PROBE_BYTES {
        match body.next().await {
            Some(Ok(chunk)) => {
                let room = PROBE_BYTES - buffer.len();
                buffer.extend_from_slice(&chunk[..chunk.len().min(room)]);
                if probe(&buffer).is_some_and(|probed| probed.frame_verified) {
                    break;
                }
            }
            Some(Err(error)) => return Err(error),
            None => break,
        }
    }
    Ok(probe(&buffer))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use futures::StreamExt;

    use super::{FixtureMedia, MediaTransport, probe_body, segmented, starts_at_beginning, tap};

    const HIRES_DASH: &[u8] = include_bytes!("fixtures/media/hires-192-dash.mp4");

    #[test]
    fn only_requests_from_the_first_byte_are_probed() {
        assert!(starts_at_beginning(None));
        assert!(starts_at_beginning(Some("bytes=0-")));
        assert!(starts_at_beginning(Some("bytes=0-1023")));
        assert!(!starts_at_beginning(Some("bytes=1024-")));
        assert!(!starts_at_beginning(Some("bytes=-500")));
    }

    #[actix_web::test]
    async fn segments_play_back_to_back_and_the_tap_reads_the_format() {
        let split = HIRES_DASH.len() / 3;
        let media: Arc<dyn MediaTransport> = Arc::new(
            FixtureMedia::new()
                .with("https://media.test/init.mp4", HIRES_DASH[..split].to_vec())
                .with("https://media.test/1.m4s", HIRES_DASH[split..].to_vec()),
        );
        let body = segmented(
            media,
            vec![
                "https://media.test/init.mp4".into(),
                "https://media.test/1.m4s".into(),
            ],
        );
        let seen = Arc::new(Mutex::new(None));
        let sink = seen.clone();
        let body = tap(body, move |probed| {
            *sink.lock().expect("lock") = Some(probed);
        });
        let bytes = body
            .map(|chunk| chunk.expect("chunk").to_vec())
            .concat()
            .await;
        assert_eq!(bytes, HIRES_DASH, "the proxy forwards bytes unchanged");
        let probed = seen
            .lock()
            .expect("lock")
            .clone()
            .flatten()
            .expect("probed");
        assert_eq!(probed.format.label, "24/192 FLAC");
        assert!(probed.frame_verified);
    }

    #[actix_web::test]
    async fn missing_media_is_an_error_not_a_guess() {
        let media = FixtureMedia::new();
        let error = media
            .open("https://media.test/missing.flac".into(), None)
            .await
            .err()
            .expect("missing");
        assert_eq!(error, super::MediaError::Status(404));
        let body = futures::stream::iter(vec![Ok(bytes::Bytes::from_static(b"<html>"))]).boxed();
        assert_eq!(probe_body(body).await, Ok(None));
    }
}
