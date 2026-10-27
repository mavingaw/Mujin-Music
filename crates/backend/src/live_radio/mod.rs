//! Live internet radio.
//!
//! - [`directory`]: the radio-browser.info community directory (search by
//!   genre, country, language, codec and bitrate);
//! - [`store`]: My Live Radio and recently played stations;
//! - [`guard`]: SSRF-safe fetching for station URLs;
//! - [`playlist`], [`hls`], [`icy`], [`format`]: following playlists,
//!   joining HLS live, stripping ICY titles and reading the real format;
//! - [`relay`]: delivering a station to any player or zone, unchanged when
//!   possible;
//! - [`signal`]: the signal path, with an honest quality light.
//!
//! Stations play through the normal stream route as `radio:<id>` songs, so
//! web, desktop, mobile and every zone type (Sonos, UPnP, Chromecast,
//! AirPlay, bridges, server outputs) play them through Parson.

pub mod directory;
pub mod format;
pub mod guard;
pub mod hls;
pub mod icy;
pub mod playlist;
pub mod relay;
pub mod signal;
pub mod station;
pub mod store;

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures::stream::StreamExt;
use serde::Serialize;

use self::directory::Directory;
use self::guard::{Body, GuardedClient, NetPolicy};
use self::icy::{TrackTitle, split_title};
use self::relay::{Delivery, Destination, RelayError, SourceFormat, Transport};
use self::station::{Quality, Station, classify};
use crate::persistence::connection::DbPool;
use crate::providers::transport::{FixtureTransport, ReqwestTransport, Transport as ApiTransport};

/// Titles kept per station for "Played on this station".
const HISTORY: usize = 12;
/// Stations remembered in memory (for zones, logos and lookups).
const MAX_KNOWN: usize = 4_096;
/// A click is reported once per station per this long.
const CLICK_INTERVAL: Duration = Duration::from_secs(10 * 60);
/// Largest logo Parson relays.
const MAX_LOGO_BYTES: usize = 512 * 1024;
const MAX_LOGO_CACHE_BYTES: usize = 32 * 1024 * 1024;
const LOGO_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// One title heard on a station.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PlayedTitle {
    #[serde(flatten)]
    pub title: TrackTitle,
    /// Unix milliseconds.
    pub at: i64,
}

#[derive(Default)]
struct Live {
    title: Option<PlayedTitle>,
    history: VecDeque<PlayedTitle>,
    format: Option<SourceFormat>,
    transport: Option<Transport>,
    delivery: Option<Delivery>,
    stream_name: Option<String>,
    genre: Option<String>,
    listeners: usize,
}

/// What a station is playing, as far as Parson heard.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct NowPlaying {
    pub station_id: String,
    /// Someone is listening through Parson right now.
    pub live: bool,
    pub title: Option<PlayedTitle>,
    /// Earlier titles, newest first.
    pub history: Vec<PlayedTitle>,
    /// What the stream carries, read from its audio.
    pub format: Option<SourceFormat>,
    pub quality: Option<Quality>,
    pub transport: Option<Transport>,
    pub delivery: Option<Delivery>,
    /// The stream's own name and genre (ICY headers).
    pub stream_name: Option<String>,
    pub genre: Option<String>,
}

struct Logo {
    at: Instant,
    image: Option<(&'static str, Bytes)>,
}

#[derive(Default)]
struct State {
    live: HashMap<String, Live>,
    known: HashMap<String, Station>,
    clicks: HashMap<String, Instant>,
    logos: HashMap<String, Logo>,
    logo_bytes: usize,
    /// Formats read from stations' audio, by station.
    verified: HashMap<String, SourceFormat>,
}

/// The live radio service.
pub struct LiveRadio {
    directory: Directory,
    client: GuardedClient,
    state: Mutex<State>,
    /// Where verified formats are kept across restarts.
    pool: OnceLock<DbPool>,
}

/// A station ready to stream.
pub struct Relayed {
    pub content_type: String,
    pub body: Body,
    pub format: SourceFormat,
    pub delivery: Delivery,
}

/// The result of checking a stream URL before saving it.
#[derive(Serialize, Clone, Debug)]
pub struct StreamCheck {
    /// Where the audio finally came from (after playlists and redirects).
    pub resolved_url: String,
    pub transport: Transport,
    pub format: SourceFormat,
    pub quality: Quality,
    pub stream_name: Option<String>,
    pub genre: Option<String>,
    /// The first title heard, when the stream sends titles.
    pub title: Option<String>,
    pub titles_supported: bool,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(&[0, 0, 1, 0]) {
        Some("image/x-icon")
    } else {
        // Many stations publish SVG logos. They are served with a CSP that
        // forbids scripts and sandboxes the document (see the logo route),
        // and `<img>` never runs SVG scripts anyway.
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]).to_lowercase();
        let head = head.trim_start_matches('\u{feff}').trim_start();
        ((head.starts_with("<svg") || head.starts_with("<?xml") || head.starts_with("<!--"))
            && head.contains("<svg"))
        .then_some("image/svg+xml")
    }
}

impl LiveRadio {
    pub fn new(transport: Arc<dyn ApiTransport>, client: GuardedClient) -> Self {
        Self {
            directory: Directory::new(transport),
            client,
            state: Mutex::new(State::default()),
            pool: OnceLock::new(),
        }
    }

    /// Keeps verified formats in the database and loads earlier ones.
    pub fn set_pool(self: &Arc<Self>, pool: DbPool) {
        if self.pool.set(pool.clone()).is_err() {
            return;
        }
        let service = Arc::downgrade(self);
        std::thread::spawn(move || {
            let Ok(mut connection) = pool.get() else {
                return;
            };
            match store::load_verified(&mut connection) {
                Ok(loaded) => {
                    if let Some(service) = service.upgrade() {
                        let mut state = service.lock();
                        for (id, format) in loaded {
                            state.verified.entry(id).or_insert(format);
                        }
                    }
                }
                Err(error) => tracing::warn!(%error, "could not load verified station formats"),
            }
        });
    }

    /// What Parson heard when it last opened a station.
    pub fn verified(&self, station_id: &str) -> Option<SourceFormat> {
        self.lock().verified.get(station_id).cloned()
    }

    fn record_verified(&self, station_id: &str, format: &SourceFormat) {
        if !format.verified {
            return;
        }
        {
            let mut state = self.lock();
            if state.verified.len() >= 50_000 {
                state.verified.clear();
            }
            state
                .verified
                .insert(station_id.to_string(), format.clone());
        }
        if let Some(pool) = self.pool.get().cloned() {
            let id = station_id.to_string();
            let format = format.clone();
            tokio::task::spawn_blocking(move || {
                if let Ok(mut connection) = pool.get()
                    && let Err(error) = store::save_verified(&mut connection, &id, &format)
                {
                    tracing::debug!(%error, "could not save a verified station format");
                }
            });
        }
    }

    /// A station as clients see it, with the format Parson heard.
    pub fn view(&self, station: &Station, saved: bool) -> station::StationView {
        station::StationView::new(station, saved).with_verified(self.verified(&station.id))
    }

    /// Opens a station just long enough to read its format.
    pub async fn verify(&self, station: &Station) -> Result<SourceFormat, RelayError> {
        let work = async {
            let source =
                relay::open_source(&self.client, station, Arc::new(|_: String| {})).await?;
            let (source, detected) = relay::identify(source).await;
            Ok::<_, RelayError>(relay::source_format(&source, detected.as_ref(), station))
        };
        let format = tokio::time::timeout(Duration::from_secs(12), work)
            .await
            .map_err(|_| RelayError::Guard(guard::GuardError::Connect("timed out".into())))??;
        self.record_verified(&station.id, &format);
        Ok(format)
    }

    /// Searches the directory. Lossless searches also look for stations
    /// that name FLAC (the directory lists most Ogg FLAC streams as "OGG"),
    /// drop those Parson heard sending lossy audio, and put the ones it
    /// verified first.
    pub async fn search(
        &self,
        query: &directory::SearchQuery,
    ) -> Result<Vec<Station>, directory::DirectoryError> {
        if !query.lossless {
            return self.directory.search(query).await;
        }
        if query.offset > 0 {
            return Ok(Vec::new());
        }
        let base = directory::SearchQuery {
            lossless: false,
            codec: None,
            offset: 0,
            limit: Some(directory::MAX_PAGE),
            ..query.clone()
        };
        let variants = [
            directory::SearchQuery {
                codec: Some("flac".into()),
                ..base.clone()
            },
            directory::SearchQuery {
                tag: Some("flac".into()),
                ..base.clone()
            },
            directory::SearchQuery {
                tag: Some("lossless".into()),
                ..base.clone()
            },
            directory::SearchQuery {
                name: Some(match &base.name {
                    Some(name) => format!("{name} flac"),
                    None => "flac".into(),
                }),
                ..base.clone()
            },
        ];
        let results = futures::future::join_all(
            variants
                .iter()
                .map(|variant| self.directory.search(variant)),
        )
        .await;
        let mut first_error = None;
        let mut seen = std::collections::HashSet::new();
        let mut stations = Vec::new();
        for result in results {
            match result {
                Ok(found) => {
                    for station in found {
                        if seen.insert(station.id.clone()) {
                            stations.push(station);
                        }
                    }
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        if stations.is_empty()
            && let Some(error) = first_error
        {
            return Err(error);
        }
        let lossless = |codec: Option<&str>| codec.is_some_and(station::codec_is_lossless);
        let mut ranked: Vec<(u8, Station)> = stations
            .into_iter()
            .filter_map(|station| {
                let heard = self.verified(&station.id);
                let rank = match heard.as_ref().and_then(|format| format.codec.as_deref()) {
                    Some(codec) if station::codec_is_lossless(codec) => 0,
                    Some(_) => return None,
                    None if lossless(station.codec.as_deref()) => 1,
                    None => return None,
                };
                Some((rank, station))
            })
            .collect();
        ranked.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then(right.1.votes.unwrap_or(0).cmp(&left.1.votes.unwrap_or(0)))
        });
        let limit = query.limit.unwrap_or(40).clamp(1, directory::MAX_PAGE) as usize;
        Ok(ranked
            .into_iter()
            .map(|(_, station)| station)
            .take(limit)
            .collect())
    }

    /// The live service, or recorded directory answers when
    /// `PARSON_LIVE_RADIO_SANDBOX=1` (demos and screenshots offline).
    pub fn from_env() -> Self {
        let sandbox = std::env::var("PARSON_LIVE_RADIO_SANDBOX")
            .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "yes"));
        let transport: Arc<dyn ApiTransport> = if sandbox {
            tracing::warn!("live radio directory runs in sandbox mode with recorded answers");
            Arc::new(
                FixtureTransport::from_json(&[include_str!("fixtures/radio_browser.json")])
                    .expect("bundled radio directory fixture"),
            )
        } else {
            match ReqwestTransport::new() {
                Ok(transport) => Arc::new(transport),
                Err(error) => {
                    tracing::warn!(%error, "live radio directory client unavailable");
                    Arc::new(FixtureTransport::default())
                }
            }
        };
        Self::new(transport, GuardedClient::system())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn directory(&self) -> &Directory {
        &self.directory
    }

    pub fn client(&self) -> &GuardedClient {
        &self.client
    }

    /// Remembers stations seen in listings, so zones, logos and the stream
    /// route find them without asking the directory again.
    pub fn remember(&self, stations: &[Station]) {
        let mut state = self.lock();
        if state.known.len() + stations.len() > MAX_KNOWN {
            let listening: Vec<String> = state
                .live
                .iter()
                .filter(|(_, live)| live.listeners > 0)
                .map(|(id, _)| id.clone())
                .collect();
            state.known.retain(|id, _| listening.contains(id));
        }
        for station in stations {
            state.known.insert(station.id.clone(), station.clone());
        }
    }

    pub fn known(&self, station_id: &str) -> Option<Station> {
        self.lock().known.get(station_id).cloned()
    }

    pub fn forget(&self, station_id: &str) {
        self.lock().known.remove(station_id);
    }

    /// Finds a station: the listener's saved copy, one seen recently, a
    /// saved copy of anyone's (signed zone requests), or the directory.
    pub async fn find(
        &self,
        pool: Option<&DbPool>,
        station_id: &str,
        user_id: Option<i32>,
        ask_directory: bool,
    ) -> Option<Station> {
        if !station::valid_station_id(station_id) {
            return None;
        }
        if let Some(pool) = pool {
            let pool = pool.clone();
            let id = station_id.to_string();
            let saved = tokio::task::spawn_blocking(move || {
                let mut connection = pool.get().ok()?;
                store::find_saved(&mut connection, &id, user_id)
                    .ok()
                    .flatten()
            })
            .await
            .ok()
            .flatten();
            if let Some(saved) = saved {
                // A custom station is only visible to its owner, except to
                // signed zone requests (no listener).
                self.remember(std::slice::from_ref(&saved));
                return Some(saved);
            }
        }
        if let Some(known) = self.known(station_id)
            && (known.source == station::StationSource::Directory || user_id.is_none())
        {
            return Some(known);
        }
        if !ask_directory {
            return None;
        }
        let found = self.directory.station(station_id).await.ok().flatten()?;
        self.remember(std::slice::from_ref(&found));
        Some(found)
    }

    fn record_title(&self, station_id: &str, raw: String, station_name: &str) {
        let Some(title) = split_title(&raw, Some(station_name)) else {
            return;
        };
        let mut state = self.lock();
        let live = state.live.entry(station_id.to_string()).or_default();
        if live
            .title
            .as_ref()
            .is_some_and(|current| current.title.raw == title.raw)
        {
            return;
        }
        if let Some(previous) = live.title.take() {
            live.history.push_front(previous);
            live.history.truncate(HISTORY);
        }
        live.title = Some(PlayedTitle {
            title,
            at: now_ms(),
        });
    }

    pub fn now_playing(&self, station_id: &str) -> NowPlaying {
        let state = self.lock();
        let live = state.live.get(station_id);
        let format = live.and_then(|live| live.format.clone());
        NowPlaying {
            station_id: station_id.to_string(),
            live: live.is_some_and(|live| live.listeners > 0),
            title: live.and_then(|live| live.title.clone()),
            history: live
                .map(|live| live.history.iter().cloned().collect())
                .unwrap_or_default(),
            quality: format.as_ref().map(|format| {
                let mut quality = classify(format.codec.as_deref(), format.bitrate_kbps);
                quality.verified = format.verified;
                quality
            }),
            format,
            transport: live.and_then(|live| live.transport),
            delivery: live.and_then(|live| live.delivery),
            stream_name: live.and_then(|live| live.stream_name.clone()),
            genre: live.and_then(|live| live.genre.clone()),
        }
    }

    fn title_sink(self: &Arc<Self>, station: &Station) -> hls::TitleSink {
        let service = Arc::downgrade(self);
        let id = station.id.clone();
        let name = station.name.clone();
        Arc::new(move |raw: String| {
            if let Some(service) = service.upgrade() {
                service.record_title(&id, raw, &name);
            }
        })
    }

    /// Opens a station for `destination`. The returned body counts as a
    /// listener until it is dropped.
    pub async fn open(
        self: &Arc<Self>,
        station: &Station,
        destination: &Destination,
    ) -> Result<Relayed, RelayError> {
        let source = relay::open_source(&self.client, station, self.title_sink(station)).await?;
        let (source, detected) = relay::identify(source).await;
        let format = relay::source_format(&source, detected.as_ref(), station);
        self.record_verified(&station.id, &format);
        let delivery = relay::plan(&format, destination);
        {
            let mut state = self.lock();
            let live = state.live.entry(station.id.clone()).or_default();
            live.format = Some(format.clone());
            live.transport = Some(source.transport);
            live.delivery = Some(delivery);
            live.stream_name = source.icy.name.clone();
            live.genre = source.icy.genre.clone();
        }
        let content_type = match delivery.output() {
            Some(output) => output.content_type().to_string(),
            None => source
                .content_type
                .clone()
                .filter(|value| value.starts_with("audio/") || value.starts_with("application/ogg"))
                .unwrap_or_else(|| match format.container {
                    format::Container::Mp3 => "audio/mpeg".into(),
                    format::Container::Adts => "audio/aac".into(),
                    format::Container::Flac => "audio/flac".into(),
                    format::Container::Ogg => "audio/ogg".into(),
                    _ => "application/octet-stream".into(),
                }),
        };
        let body = match relay::ffmpeg_args(delivery, format.container) {
            Some(args) => relay::pipe_through_ffmpeg(source.body, args)?,
            None => source.body,
        };
        Ok(Relayed {
            content_type,
            body: self.counted(&station.id, body),
            format,
            delivery,
        })
    }

    /// Counts a listener while `body` is alive.
    fn counted(self: &Arc<Self>, station_id: &str, body: Body) -> Body {
        struct Listener {
            service: std::sync::Weak<LiveRadio>,
            station_id: String,
        }
        impl Drop for Listener {
            fn drop(&mut self) {
                if let Some(service) = self.service.upgrade() {
                    let mut state = service.lock();
                    if let Some(live) = state.live.get_mut(&self.station_id) {
                        live.listeners = live.listeners.saturating_sub(1);
                    }
                }
            }
        }
        self.lock()
            .live
            .entry(station_id.to_string())
            .or_default()
            .listeners += 1;
        let listener = Listener {
            service: Arc::downgrade(self),
            station_id: station_id.to_string(),
        };
        futures::stream::unfold((body, listener), |(mut body, listener)| async move {
            let chunk = body.next().await?;
            Some((chunk, (body, listener)))
        })
        .boxed()
    }

    /// Opens a URL the way the relay would and reports what it found,
    /// without saving anything.
    pub async fn check(&self, url: &str, allow_lan: bool) -> Result<StreamCheck, RelayError> {
        let titles = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink: hls::TitleSink = {
            let titles = titles.clone();
            Arc::new(move |title| {
                titles
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(title)
            })
        };
        let probe = Station {
            id: "00000000-0000-4000-8000-000000000000".into(),
            source: station::StationSource::Custom,
            name: String::new(),
            stream_url: url.to_string(),
            homepage: None,
            favicon: None,
            country: None,
            country_code: None,
            region: None,
            languages: Vec::new(),
            tags: Vec::new(),
            codec: None,
            bitrate_kbps: None,
            hls: false,
            votes: None,
            clicks: None,
            click_trend: None,
            online: None,
            description: None,
            allow_lan,
        };
        let source = relay::open_source(&self.client, &probe, sink).await?;
        let (source, detected) = relay::identify(source).await;
        let format = relay::source_format(&source, detected.as_ref(), &probe);
        if format.codec.is_none() && detected.is_none() {
            return Err(RelayError::NoStream);
        }
        // Listen a little longer for the first title.
        let mut body = source.body;
        let _ = tokio::time::timeout(Duration::from_secs(3), async {
            let mut read = 0_usize;
            while let Some(Ok(chunk)) = body.next().await {
                read += chunk.len();
                if read > 256 * 1024
                    || !titles
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .is_empty()
                {
                    break;
                }
            }
        })
        .await;
        let title = titles
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .first()
            .cloned();
        Ok(StreamCheck {
            resolved_url: source.url.to_string(),
            transport: source.transport,
            quality: classify(format.codec.as_deref(), format.bitrate_kbps),
            format,
            stream_name: source.icy.name,
            genre: source.icy.genre,
            titles_supported: source.icy_metadata || source.transport == Transport::Hls,
            title,
        })
    }

    /// Reports a click to the directory, at most once per station per ten
    /// minutes from this server.
    pub fn report_click(self: &Arc<Self>, station: &Station) {
        if station.source != station::StationSource::Directory {
            return;
        }
        {
            let mut state = self.lock();
            if state
                .clicks
                .get(&station.id)
                .is_some_and(|at| at.elapsed() < CLICK_INTERVAL)
            {
                return;
            }
            state.clicks.retain(|_, at| at.elapsed() < CLICK_INTERVAL);
            state.clicks.insert(station.id.clone(), Instant::now());
        }
        let service = self.clone();
        let id = station.id.clone();
        tokio::spawn(async move { service.directory.report_click(&id).await });
    }

    /// The station's logo, fetched through the guard and cached. Only
    /// PNG, JPEG, GIF, WebP, ICO and SVG (read from the bytes, never the
    /// remote content type) are relayed.
    pub async fn logo(&self, station: &Station) -> Option<(&'static str, Bytes)> {
        if let Some(cached) = self.lock().logos.get(&station.id)
            && cached.at.elapsed() < LOGO_TTL
        {
            return cached.image.clone();
        }
        let url = guard::parse_url(station.favicon.as_deref()?).ok()?;
        let policy = if station.allow_lan {
            NetPolicy::LAN
        } else {
            NetPolicy::PUBLIC
        };
        let image = match self
            .client
            .read(url, policy, &[("Accept", "image/*")], MAX_LOGO_BYTES + 1)
            .await
        {
            Ok((_, _, bytes)) if bytes.len() <= MAX_LOGO_BYTES => {
                sniff_image(&bytes).map(|kind| (kind, Bytes::from(bytes)))
            }
            _ => None,
        };
        let mut state = self.lock();
        let size = image.as_ref().map_or(0, |(_, bytes)| bytes.len());
        if state.logo_bytes + size > MAX_LOGO_CACHE_BYTES {
            state.logos.clear();
            state.logo_bytes = 0;
        }
        state.logo_bytes += size;
        state.logos.insert(
            station.id.clone(),
            Logo {
                at: Instant::now(),
                image: image.clone(),
            },
        );
        image
    }

    /// A synthetic library song for a station, so zones can queue and show
    /// it like a track.
    pub fn zone_song(&self, song_id: &str) -> Option<crate::domain::Song> {
        let station_id = station::station_id_from_song(song_id)?;
        let station = self.known(station_id)?;
        let playing = self.now_playing(station_id);
        let format = playing.format.as_ref();
        let codec = format
            .and_then(|format| format.codec.clone())
            .or_else(|| station.codec.as_deref().and_then(station::normalize_codec));
        Some(crate::domain::Song {
            id: song_id.to_string(),
            name: station.name.clone(),
            artist: playing
                .title
                .map(|played| played.title.raw)
                .unwrap_or_else(|| "Live radio".into()),
            contributing_artists: Vec::new(),
            contributing_artist_ids: Vec::new(),
            track_number: 0,
            path: String::new(),
            duration: 0.0,
            audio: Some(crate::domain::AudioProperties {
                lossless: codec.as_deref().map(station::codec_is_lossless),
                codec,
                container: format.map(|format| format.container.as_str().to_string()),
                sample_rate: format.and_then(|format| format.sample_rate),
                bit_depth: format.and_then(|format| format.bit_depth),
                channels: format.and_then(|format| format.channels),
                bitrate_kbps: format
                    .and_then(|format| format.bitrate_kbps)
                    .or(station.bitrate_kbps),
            }),
            credits: Vec::new(),
            isrc: None,
            label: None,
            loudness: None,
        })
    }
}

static GLOBAL: OnceLock<Arc<LiveRadio>> = OnceLock::new();

/// The process-wide service (zones reach it without app data).
pub fn global() -> Arc<LiveRadio> {
    GLOBAL
        .get_or_init(|| Arc::new(LiveRadio::from_env()))
        .clone()
}

pub use station::{is_live_song_id, station_id_from_song};

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use futures::StreamExt;

    use super::LiveRadio;
    use super::guard::GuardedClient;
    use super::guard::fakes::{FakeFetch, FakeReply, FakeResolver};
    use super::relay::Destination;
    use super::store::tests::station;
    use crate::live_radio::format::tests::mp3_frames;
    use crate::providers::transport::FixtureTransport;

    pub(crate) fn service(fetch: FakeFetch) -> Arc<LiveRadio> {
        let transport = Arc::new(
            FixtureTransport::from_json(&[super::directory::tests::FIXTURE]).expect("fixture"),
        );
        Arc::new(LiveRadio::new(
            transport,
            GuardedClient::new(
                Arc::new(
                    FakeResolver::default()
                        .with("radio.example", &["93.184.216.34"])
                        .with("img.example", &["93.184.216.36"]),
                ),
                Arc::new(fetch),
            ),
        ))
    }

    /// A stream with an ICY title block after every 1000 audio bytes.
    pub(crate) fn icy_wire(audio: &[u8], titles: &[&str]) -> Vec<u8> {
        let mut wire = Vec::new();
        for (index, run) in audio.chunks(1000).enumerate() {
            wire.extend_from_slice(run);
            if run.len() == 1000 {
                match titles.get(index) {
                    Some(title) => {
                        let text = format!("StreamTitle='{title}';");
                        let padded = text.len().div_ceil(16) * 16;
                        wire.push((padded / 16) as u8);
                        let mut bytes = text.into_bytes();
                        bytes.resize(padded, 0);
                        wire.extend(bytes);
                    }
                    None => wire.push(0),
                }
            }
        }
        wire
    }

    #[actix_web::test]
    async fn playing_a_station_records_titles_format_and_listeners() {
        let audio = mp3_frames(192, 20);
        let fetch = FakeFetch::default().with(
            "https://radio.example/live",
            FakeReply::ok("audio/mpeg", Vec::new())
                .header("icy-metaint", "1000")
                .header("icy-name", "Example FM")
                .chunked(vec![icy_wire(&audio, &["A - One", "A - One", "B - Two"])]),
        );
        let service = service(fetch);
        let mut station = station("d1a54d2e-623e-4970-ab11-35f7b56c5ec3", "Example FM");
        station.stream_url = "https://radio.example/live".into();
        let relayed = service
            .open(&station, &Destination::Player { accepts: None })
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(relayed.content_type, "audio/mpeg");
        assert_eq!(relayed.format.bitrate_kbps, Some(192));
        assert!(service.now_playing(&station.id).live);
        let bytes: Vec<u8> = relayed
            .body
            .map(|chunk| chunk.unwrap().to_vec())
            .concat()
            .await;
        assert_eq!(bytes, audio);
        let playing = service.now_playing(&station.id);
        assert!(!playing.live, "the listener left");
        assert_eq!(playing.title.as_ref().unwrap().title.title, "Two");
        assert_eq!(playing.history.len(), 1);
        assert_eq!(playing.history[0].title.artist.as_deref(), Some("A"));
        assert_eq!(playing.stream_name.as_deref(), Some("Example FM"));
        assert_eq!(playing.quality.unwrap().label, "MP3 192");

        service.remember(&[station.clone()]);
        let song = service.zone_song(&station.song_id()).expect("zone song");
        assert_eq!(song.name, "Example FM");
        assert_eq!(song.artist, "B - Two");
        assert_eq!(song.audio.unwrap().bitrate_kbps, Some(192));
    }

    #[actix_web::test]
    async fn checks_report_format_and_first_title() {
        let audio = mp3_frames(320, 20);
        let fetch = FakeFetch::default()
            .with(
                "https://radio.example/check.m3u",
                FakeReply::ok("audio/x-mpegurl", "#EXTM3U\nhttps://radio.example/live\n"),
            )
            .with(
                "https://radio.example/live",
                FakeReply::ok("audio/mpeg", Vec::new())
                    .header("icy-metaint", "1000")
                    .header("icy-genre", "Jazz")
                    .chunked(vec![icy_wire(&audio, &["Ella Fitzgerald - Misty"])]),
            );
        let service = service(fetch);
        let check = service
            .check("https://radio.example/check.m3u", false)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(check.resolved_url, "https://radio.example/live");
        assert_eq!(check.quality.label, "MP3 320");
        assert_eq!(check.title.as_deref(), Some("Ella Fitzgerald - Misty"));
        assert_eq!(check.genre.as_deref(), Some("Jazz"));
        assert!(check.titles_supported);
        assert_eq!(
            service
                .check("http://127.0.0.1/", false)
                .await
                .err()
                .unwrap()
                .code(),
            "stream_address_blocked"
        );
    }

    #[actix_web::test]
    async fn logos_are_relayed_only_as_raster_images() {
        let png = b"\x89PNG\r\n\x1a\nrest".to_vec();
        let fetch = FakeFetch::default()
            .with(
                "https://img.example/logo.png",
                FakeReply::ok("image/png", png.clone()),
            )
            .with(
                "https://img.example/logo.svg",
                FakeReply::ok("image/svg+xml", b"<svg onload=alert(1)>".to_vec()),
            )
            .with(
                "https://img.example/page",
                FakeReply::ok("image/png", b"<!doctype html><title>x</title>".to_vec()),
            );
        let service = service(fetch);
        let mut with_logo = station("d1a54d2e-623e-4970-ab11-35f7b56c5ec3", "a");
        with_logo.favicon = Some("https://img.example/logo.png".into());
        let (kind, bytes) = service.logo(&with_logo).await.expect("png");
        assert_eq!(kind, "image/png");
        assert_eq!(bytes.as_ref(), png.as_slice());
        let mut svg = station("e8f9ffdc-22a8-4c8b-82dd-ec63685feee6", "b");
        svg.favicon = Some("https://img.example/logo.svg".into());
        // Served as SVG under a sandboxing CSP (the route sets it).
        assert_eq!(service.logo(&svg).await.expect("svg").0, "image/svg+xml");
        let mut html = station("002859e6-fa1c-4ad8-8f4c-f204c42e36ae", "d");
        html.favicon = Some("https://img.example/page".into());
        assert!(service.logo(&html).await.is_none());
        let mut private = station("98adecf7-2683-4408-9be7-02d3f9098eb8", "c");
        private.favicon = Some("http://169.254.169.254/latest".into());
        assert!(service.logo(&private).await.is_none());
    }

    #[actix_web::test]
    async fn lossless_searches_trust_what_parson_heard() {
        use super::directory::SearchQuery;
        use super::format::Container;
        use super::relay::SourceFormat;
        let service = service(FakeFetch::default());
        let heard = |codec: &str| SourceFormat {
            container: Container::Ogg,
            codec: Some(codec.into()),
            sample_rate: Some(44_100),
            bit_depth: None,
            channels: Some(2),
            bitrate_kbps: None,
            verified: true,
        };
        // Listed as FLAC, but Parson heard AAC: not lossless.
        service.record_verified("e8f9ffdc-22a8-4c8b-82dd-ec63685feee6", &heard("aac"));
        // Verified FLAC goes first.
        service.record_verified("45be7e54-944f-4a57-9d51-8f74c88097cb", &heard("flac"));
        let found = service
            .search(&SearchQuery {
                lossless: true,
                ..SearchQuery::default()
            })
            .await
            .expect("search");
        let ids: Vec<_> = found.iter().map(|station| station.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "45be7e54-944f-4a57-9d51-8f74c88097cb",
                "002859e6-fa1c-4ad8-8f4c-f204c42e36ae"
            ]
        );
        let view = service.view(&found[0], false);
        assert!(view.quality.verified);
        assert_eq!(view.quality.label, "FLAC");
        let unheard = service.view(&found[1], false);
        assert!(!unheard.quality.verified);
        assert!(
            service
                .search(&SearchQuery {
                    lossless: true,
                    offset: 40,
                    ..SearchQuery::default()
                })
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[actix_web::test]
    async fn custom_stations_stay_private_to_their_owner() {
        let service = service(FakeFetch::default());
        let pool = super::store::tests::pool();
        let mut custom = station("98adecf7-2683-4408-9be7-02d3f9098eb8", "mine");
        custom.source = super::station::StationSource::Custom;
        {
            let mut connection = pool.get().unwrap();
            super::store::save(&mut connection, 1, &custom).unwrap();
        }
        assert!(
            service
                .find(Some(&pool), &custom.id, Some(1), false)
                .await
                .is_some()
        );
        // Another listener cannot reach it, even after the owner played it.
        service.forget(&custom.id);
        assert!(
            service
                .find(Some(&pool), &custom.id, Some(2), false)
                .await
                .is_none()
        );
        // Signed zone requests (no listener) can.
        assert!(
            service
                .find(Some(&pool), &custom.id, None, false)
                .await
                .is_some()
        );
        // Directory stations come from the directory.
        let found = service
            .find(None, "d1a54d2e-623e-4970-ab11-35f7b56c5ec3", Some(2), true)
            .await
            .expect("directory station");
        assert_eq!(found.name, "Classic Vinyl HD");
        assert!(service.find(None, "bad id", Some(2), true).await.is_none());
    }
}
