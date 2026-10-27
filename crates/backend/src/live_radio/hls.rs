//! Live HLS: turns an HLS playlist into one continuous byte stream.
//!
//! Parson picks the best audio rendition (lossless first, then the highest
//! bandwidth), starts near the live edge, and keeps fetching new segments
//! as the media playlist grows. Every playlist and segment fetch goes
//! through the SSRF guard. Packed-audio segments (`.aac`, `.mp3`) are
//! joined into a plain ADTS/MP3 stream (their per-segment ID3 timestamp
//! tags removed); transport-stream and fragmented-MP4 segments are
//! repackaged by the relay. Track titles in `#EXTINF` lines feed the
//! now-playing board.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures::stream::StreamExt;
use reqwest::Url;

use super::format::skip_id3;
use super::guard::{Body, GuardError, GuardedClient, NetPolicy};
use super::playlist::{self, MAX_PLAYLIST_BYTES, MediaPlaylist};

/// Largest segment Parson reads (a 10 s 24/192 FLAC segment is ~10 MB).
const MAX_SEGMENT_BYTES: usize = 32 * 1024 * 1024;
/// Segments behind the live edge to start with.
const LIVE_EDGE_SEGMENTS: usize = 3;
/// Failed fetches in a row before the stream ends.
const MAX_FAILURES: u32 = 6;

pub type TitleSink = Arc<dyn Fn(String) + Send + Sync>;

#[derive(Debug)]
pub enum HlsError {
    Guard(GuardError),
    /// Encrypted (DRM or AES) streams cannot be relayed.
    Encrypted,
    Empty,
    /// The playlist has no HLS tags: a plain M3U list of stream URLs
    /// (often named `.m3u8`), read from `url`.
    NotHls {
        url: Url,
        text: String,
    },
}

/// An opened HLS stream.
pub struct HlsStream {
    pub body: Body,
    /// Codec named by the master playlist (or guessed from segment names).
    pub codec: Option<&'static str>,
    pub bandwidth: Option<u64>,
    pub playlist_url: Url,
}

async fn read_text(
    client: &GuardedClient,
    policy: NetPolicy,
    url: Url,
) -> Result<(Url, String), GuardError> {
    let (url, _, bytes) = client.read(url, policy, &[], MAX_PLAYLIST_BYTES).await?;
    Ok((url, String::from_utf8_lossy(&bytes).into_owned()))
}

fn codec_from_segments(playlist: &MediaPlaylist) -> Option<&'static str> {
    let segment = playlist.segments.first()?;
    let name = segment.url.path().to_ascii_lowercase();
    if name.ends_with(".aac") {
        Some("aac")
    } else if name.ends_with(".mp3") {
        Some("mp3")
    } else if name.ends_with(".flac") {
        Some("flac")
    } else {
        None
    }
}

/// Opens an HLS stream; `text` is the playlist when the caller already
/// read it.
pub async fn open(
    client: &GuardedClient,
    policy: NetPolicy,
    url: Url,
    text: Option<String>,
    titles: TitleSink,
) -> Result<HlsStream, HlsError> {
    let (mut url, mut text) = match text {
        Some(text) => (url, text),
        None => read_text(client, policy, url)
            .await
            .map_err(HlsError::Guard)?,
    };
    if !playlist::is_hls_text(text.trim_start_matches('\u{feff}')) {
        return Err(HlsError::NotHls { url, text });
    }
    let mut codec = None;
    let mut bandwidth = None;
    if playlist::is_master(&text) {
        let variants = playlist::parse_master(&text, &url);
        let best = playlist::best_variant(&variants)
            .ok_or(HlsError::Empty)?
            .clone();
        codec = best.audio_codec();
        bandwidth = best.bandwidth;
        (url, text) = read_text(client, policy, best.url)
            .await
            .map_err(HlsError::Guard)?;
    }
    let media = playlist::parse_media(&text, &url);
    if media.encrypted {
        return Err(HlsError::Encrypted);
    }
    if media.segments.is_empty() {
        return Err(HlsError::Empty);
    }
    let codec = codec.or_else(|| codec_from_segments(&media));
    let mut live = Live {
        client: client.clone(),
        policy,
        playlist_url: url.clone(),
        last_sequence: None,
        queue: VecDeque::new(),
        init: media.init.clone(),
        init_sent: false,
        target: media.target_duration,
        ended: media.ended,
        failures: 0,
        titles,
        last_title: None,
        first_poll: true,
    };
    live.enqueue(&media);
    let body = futures::stream::unfold(live, |mut live| async move {
        let item = live.next().await?;
        Some((item, live))
    })
    .fuse()
    .boxed();
    Ok(HlsStream {
        body,
        codec,
        bandwidth,
        playlist_url: url,
    })
}

struct Live {
    client: GuardedClient,
    policy: NetPolicy,
    playlist_url: Url,
    last_sequence: Option<u64>,
    queue: VecDeque<playlist::Segment>,
    init: Option<Url>,
    init_sent: bool,
    target: f64,
    ended: bool,
    failures: u32,
    titles: TitleSink,
    last_title: Option<String>,
    first_poll: bool,
}

impl Live {
    /// Queues segments newer than the last one queued; the first time,
    /// only the few nearest the live edge.
    fn enqueue(&mut self, media: &MediaPlaylist) {
        let segments = &media.segments;
        let fresh: Vec<&playlist::Segment> = match self.last_sequence {
            None if media.ended => segments.iter().collect(),
            None => segments
                .iter()
                .skip(segments.len().saturating_sub(LIVE_EDGE_SEGMENTS))
                .collect(),
            Some(last) => {
                let newest = segments.last().map(|segment| segment.sequence).unwrap_or(0);
                if newest.saturating_add(100) < last {
                    // The packager restarted its numbering: rejoin at the edge.
                    segments
                        .iter()
                        .skip(segments.len().saturating_sub(2))
                        .collect()
                } else {
                    segments
                        .iter()
                        .filter(|segment| segment.sequence > last)
                        .collect()
                }
            }
        };
        for segment in fresh {
            self.last_sequence = Some(segment.sequence);
            self.queue.push_back(segment.clone());
        }
        self.target = media.target_duration;
        self.ended = media.ended;
        if self.init.is_none() {
            self.init = media.init.clone();
        }
    }

    async fn fetch(&self, url: Url) -> Result<Vec<u8>, GuardError> {
        let (_, _, bytes) = self
            .client
            .read(url, self.policy, &[], MAX_SEGMENT_BYTES)
            .await?;
        Ok(bytes)
    }

    async fn next(&mut self) -> Option<Result<Bytes, String>> {
        loop {
            if !self.init_sent {
                self.init_sent = true;
                if let Some(init) = self.init.clone() {
                    match self.fetch(init).await {
                        Ok(bytes) => return Some(Ok(Bytes::from(bytes))),
                        Err(error) => return Some(Err(error.to_string())),
                    }
                }
            }
            if let Some(segment) = self.queue.pop_front() {
                if let Some(title) = segment.title.clone()
                    && self.last_title.as_ref() != Some(&title)
                {
                    self.last_title = Some(title.clone());
                    (self.titles)(title);
                }
                match self.fetch(segment.url).await {
                    Ok(bytes) => {
                        self.failures = 0;
                        let start = skip_id3(&bytes).unwrap_or(0);
                        if start < bytes.len() {
                            return Some(Ok(Bytes::copy_from_slice(&bytes[start..])));
                        }
                        continue;
                    }
                    Err(error) => {
                        self.failures += 1;
                        if self.failures >= MAX_FAILURES {
                            return Some(Err(error.to_string()));
                        }
                        continue;
                    }
                }
            }
            if self.ended {
                return None;
            }
            // Poll at half the target duration (the HLS spec's advice
            // after an unchanged playlist), but go straight away the first
            // time so a short start window does not stall.
            if !self.first_poll {
                let wait = if self.target.is_finite() {
                    (self.target / 2.0).clamp(1.0, 10.0)
                } else {
                    1.0
                };
                tokio::time::sleep(Duration::from_secs_f64(wait)).await;
            }
            self.first_poll = false;
            match read_text(&self.client, self.policy, self.playlist_url.clone()).await {
                Ok((_, text)) => {
                    let media = playlist::parse_media(&text, &self.playlist_url);
                    if media.encrypted {
                        return Some(Err("the stream became encrypted".into()));
                    }
                    self.enqueue(&media);
                }
                Err(error) => {
                    self.failures += 1;
                    if self.failures >= MAX_FAILURES {
                        return Some(Err(error.to_string()));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use futures::StreamExt;

    use super::{HlsError, open};
    use crate::live_radio::format::tests::adts_frames;
    use crate::live_radio::guard::fakes::{FakeFetch, FakeReply, FakeResolver};
    use crate::live_radio::guard::{GuardedClient, NetPolicy, parse_url};

    fn id3_tagged(payload: Vec<u8>) -> Vec<u8> {
        let mut out = b"ID3\x04\0\0\0\0\0\x05".to_vec();
        out.extend([0_u8; 5]);
        out.extend(payload);
        out
    }

    #[actix_web::test]
    async fn master_playlists_play_the_best_rendition_from_the_live_edge() {
        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=48000,CODECS=\"mp4a.40.5\"\nlow.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=256000,CODECS=\"mp4a.40.2\"\nhigh.m3u8\n";
        let media_1 = "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:10\n\
            #EXTINF:1,\ns10.aac\n#EXTINF:1,\ns11.aac\n#EXTINF:1,\ns12.aac\n#EXTINF:1,title=\"Blue in Green\",artist=\"Bill Evans\"\ns13.aac\n";
        let media_2 = "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:12\n\
            #EXTINF:1,\ns12.aac\n#EXTINF:1,title=\"Blue in Green\",artist=\"Bill Evans\"\ns13.aac\n#EXTINF:1,\ns14.aac\n#EXT-X-ENDLIST\n";
        let base = "https://hls.example/live/";
        let segment = |n: u8| {
            let mut frames = adts_frames(1);
            frames[10] = n;
            id3_tagged(frames)
        };
        let mut fetch = FakeFetch::default()
            .with(
                &format!("{base}master.m3u8"),
                FakeReply::ok("application/vnd.apple.mpegurl", master),
            )
            .with(
                &format!("{base}high.m3u8"),
                FakeReply::ok("application/vnd.apple.mpegurl", media_1),
            )
            .with(
                &format!("{base}high.m3u8"),
                FakeReply::ok("application/vnd.apple.mpegurl", media_2),
            );
        for n in 10..=14 {
            fetch = fetch.with(
                &format!("{base}s{n}.aac"),
                FakeReply::ok("audio/aac", segment(n)),
            );
        }
        let fetch = Arc::new(fetch);
        let client = GuardedClient::new(
            Arc::new(FakeResolver::default().with("hls.example", &["93.184.216.34"])),
            fetch.clone(),
        );
        let titles = Arc::new(Mutex::new(Vec::new()));
        let sink = {
            let titles = titles.clone();
            Arc::new(move |title: String| titles.lock().unwrap().push(title))
        };
        let stream = open(
            &client,
            NetPolicy::PUBLIC,
            parse_url(&format!("{base}master.m3u8")).unwrap(),
            None,
            sink,
        )
        .await
        .unwrap_or_else(|error| panic!("{error:?}"));
        assert_eq!(stream.codec, Some("aac"));
        assert_eq!(stream.bandwidth, Some(256_000));
        let chunks: Vec<_> = stream.body.collect().await;
        let markers: Vec<u8> = chunks
            .iter()
            .map(|chunk| {
                let chunk = chunk.as_ref().expect("chunk");
                assert_eq!(&chunk[..2], &[0xff, 0xf1], "ID3 tags are removed");
                chunk[10]
            })
            .collect();
        // Live edge (last three of the first playlist), then the new one.
        assert_eq!(markers, [11, 12, 13, 14]);
        assert_eq!(
            titles.lock().unwrap().as_slice(),
            ["Bill Evans - Blue in Green"]
        );
        assert!(!fetch.urls().iter().any(|url| url.contains("low.m3u8")));
        assert!(!fetch.urls().iter().any(|url| url.ends_with("s10.aac")));
    }

    #[actix_web::test]
    async fn encrypted_and_private_streams_are_refused() {
        let fetch = FakeFetch::default().with(
            "https://hls.example/drm.m3u8",
            FakeReply::ok(
                "application/vnd.apple.mpegurl",
                "#EXTM3U\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://x\"\n#EXTINF:6,\na.ts\n",
            ),
        );
        let client = GuardedClient::new(
            Arc::new(
                FakeResolver::default()
                    .with("hls.example", &["93.184.216.34"])
                    .with("intranet.example", &["10.0.0.4"]),
            ),
            Arc::new(fetch),
        );
        let sink = Arc::new(|_: String| {});
        assert!(matches!(
            open(
                &client,
                NetPolicy::PUBLIC,
                parse_url("https://hls.example/drm.m3u8").unwrap(),
                None,
                sink.clone()
            )
            .await,
            Err(HlsError::Encrypted)
        ));
        assert!(matches!(
            open(
                &client,
                NetPolicy::PUBLIC,
                parse_url("https://intranet.example/live.m3u8").unwrap(),
                None,
                sink
            )
            .await,
            Err(HlsError::Guard(_))
        ));
    }
}
