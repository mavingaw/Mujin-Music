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
        Box::pin(async move { serve_file(file.ok_or(MediaError::Status(404))?, range) })
    }
}

/// Answers a request for an in-memory file, honouring a byte range, in
/// small chunks as a network would deliver them.
pub fn serve_file(file: Bytes, range: Option<String>) -> Result<MediaResponse, MediaError> {
    let length = file.len() as u64;
    let (status, body, content_range) = match range.as_deref() {
        Some(header) => {
            let (start, end) = byte_range(Some(header), length).ok_or(MediaError::Status(416))?;
            (
                206,
                file.slice(start as usize..=end as usize),
                Some(format!("bytes {start}-{end}/{length}")),
            )
        }
        None => (200, file, None),
    };
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
}

/// Plays DASH segments back-to-back as one body.
pub fn segmented(media: Arc<dyn MediaTransport>, urls: Vec<String>) -> MediaBody {
    pieces(media, urls.into_iter().map(|url| (url, None)).collect())
}

/// How many segment sizes are read at once when indexing a DASH stream.
const INDEX_CONCURRENCY: usize = 16;

/// Byte sizes of a DASH representation's segments, in order. With it the
/// proxy can serve the back-to-back segments as one file of known length
/// and answer byte ranges, so players can seek inside TIDAL's DASH streams
/// the way they seek in a progressive file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SegmentIndex {
    lengths: Vec<u64>,
}

impl SegmentIndex {
    pub fn new(lengths: Vec<u64>) -> Self {
        Self { lengths }
    }

    pub fn segments(&self) -> usize {
        self.lengths.len()
    }

    pub fn total(&self) -> u64 {
        self.lengths.iter().sum()
    }

    /// The per-segment requests that cover bytes `start..=end` of the joined
    /// stream. Whole segments are fetched without a `Range` header.
    pub fn plan(&self, urls: &[String], start: u64, end: u64) -> Vec<(String, Option<String>)> {
        let mut plan = Vec::new();
        let mut offset = 0u64;
        for (url, &length) in urls.iter().zip(&self.lengths) {
            let segment_end = offset + length;
            if length > 0 && segment_end > start && offset <= end {
                let from = start.saturating_sub(offset);
                let to = (end - offset).min(length - 1);
                let range = (from > 0 || to < length - 1).then(|| format!("bytes={from}-{to}"));
                plan.push((url.clone(), range));
            }
            if offset > end {
                break;
            }
            offset = segment_end;
        }
        plan
    }
}

/// Reads the size of a resource from a one-byte range request.
async fn resource_length(media: Arc<dyn MediaTransport>, url: String) -> Result<u64, MediaError> {
    match media.open(url, Some("bytes=0-0".into())).await {
        Ok(response) => {
            let from_range = response
                .content_range
                .as_deref()
                .and_then(|range| range.rsplit_once('/'))
                .and_then(|(_, total)| total.trim().parse::<u64>().ok());
            match (response.status, from_range, response.content_length) {
                (206, Some(total), _) => Ok(total),
                // The host ignored the range and sent the whole resource.
                (200, _, Some(length)) => Ok(length),
                _ => Err(MediaError::Read(
                    "the media host did not report a segment size".into(),
                )),
            }
        }
        // An empty segment cannot satisfy any range.
        Err(MediaError::Status(416)) => Ok(0),
        Err(error) => Err(error),
    }
}

/// Measures every segment, a few at a time, keeping their order.
pub async fn index_segments(
    media: Arc<dyn MediaTransport>,
    urls: &[String],
) -> Result<SegmentIndex, MediaError> {
    let lengths = futures::stream::iter(urls.iter().cloned())
        .map(|url| resource_length(media.clone(), url))
        .buffered(INDEX_CONCURRENCY)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    Ok(SegmentIndex::new(lengths))
}

/// Plays the given requests back-to-back as one body.
pub fn pieces(
    media: Arc<dyn MediaTransport>,
    requests: Vec<(String, Option<String>)>,
) -> MediaBody {
    futures::stream::unfold(
        (requests.into_iter(), None::<MediaBody>),
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
                    let (url, range) = urls.next()?;
                    match media.open(url, range).await {
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

    use super::{
        FixtureMedia, MediaTransport, SegmentIndex, index_segments, pieces, probe_body, segmented,
        starts_at_beginning, tap,
    };

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

    #[test]
    fn range_plans_map_onto_segments() {
        let urls: Vec<String> = (0..4).map(|n| format!("https://media.test/{n}")).collect();
        let index = SegmentIndex::new(vec![10, 0, 20, 5]);
        assert_eq!(index.total(), 35);
        let plan = |start, end| index.plan(&urls, start, end);
        assert_eq!(
            plan(0, 34),
            vec![
                (urls[0].clone(), None),
                (urls[2].clone(), None),
                (urls[3].clone(), None),
            ],
            "whole segments go without a Range; empty ones are skipped"
        );
        assert_eq!(
            plan(5, 12),
            vec![
                (urls[0].clone(), Some("bytes=5-9".into())),
                (urls[2].clone(), Some("bytes=0-2".into())),
            ]
        );
        assert_eq!(
            plan(30, 34),
            vec![(urls[3].clone(), None)],
            "a range that is exactly one segment"
        );
        assert_eq!(
            plan(31, 31),
            vec![(urls[3].clone(), Some("bytes=1-1".into()))]
        );
    }

    #[actix_web::test]
    async fn segment_sizes_are_measured_and_ranges_join_up() {
        let split = HIRES_DASH.len() / 3;
        let urls = vec![
            "https://media.test/a".to_string(),
            "https://media.test/b".to_string(),
            "https://media.test/c".to_string(),
        ];
        let media: Arc<dyn MediaTransport> = Arc::new(
            FixtureMedia::new()
                .with(&urls[0], HIRES_DASH[..split].to_vec())
                .with(&urls[1], HIRES_DASH[split..2 * split].to_vec())
                .with(&urls[2], HIRES_DASH[2 * split..].to_vec()),
        );
        let index = index_segments(media.clone(), &urls).await.expect("index");
        assert_eq!(index.total(), HIRES_DASH.len() as u64);
        let (start, end) = (split - 10, 2 * split + 10);
        let bytes = pieces(media, index.plan(&urls, start as u64, end as u64))
            .map(|chunk| chunk.expect("chunk").to_vec())
            .concat()
            .await;
        assert_eq!(bytes, &HIRES_DASH[start..=end]);
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
