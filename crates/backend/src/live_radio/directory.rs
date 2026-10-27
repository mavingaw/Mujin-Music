//! The community station directory at radio-browser.info.
//!
//! radio-browser is free, needs no key and asks clients to:
//! - pick one of its API servers at random from the published list (DNS
//!   SRV `_api._tcp.radio-browser.info`, or the same list as JSON at
//!   `all.api.radio-browser.info/json/servers`) and move on to another one
//!   when a server fails;
//! - send a descriptive User-Agent;
//! - report a click (`/json/url/<uuid>`) when a listener starts a station,
//!   which keeps the popularity ranking meaningful.
//!
//! Parson does all three and caches answers so browsing stays fast and the
//! community servers see little load. Requests go through
//! [`Transport`] so tests replay recorded answers and never reach the live
//! service.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::guard::user_agent;
use super::station::{Station, StationSource, clean, normalize_codec, valid_station_id};
use crate::providers::transport::{ApiRequest, Transport, TransportError};

/// The published server list.
pub const SERVER_LIST_URL: &str = "https://all.api.radio-browser.info/json/servers";
/// Used when the server list itself cannot be fetched.
pub const FALLBACK_SERVERS: [&str; 3] = [
    "de1.api.radio-browser.info",
    "de2.api.radio-browser.info",
    "fi1.api.radio-browser.info",
];
const SERVER_LIST_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const SEARCH_TTL: Duration = Duration::from_secs(10 * 60);
const FACET_TTL: Duration = Duration::from_secs(12 * 60 * 60);
const STATION_TTL: Duration = Duration::from_secs(60 * 60);
const MAX_CACHE_ENTRIES: usize = 512;
/// Most stations one search returns.
pub const MAX_PAGE: u32 = 100;
const SERVERS_TRIED: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectoryError {
    /// Every server failed.
    Unavailable(String),
    BadResponse,
}

impl std::fmt::Display for DirectoryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(reason) => write!(
                formatter,
                "The radio directory is unreachable right now ({reason})."
            ),
            Self::BadResponse => {
                formatter.write_str("The radio directory sent an unexpected answer.")
            }
        }
    }
}

impl std::error::Error for DirectoryError {}

/// How search results are ordered.
#[derive(Deserialize, Serialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    /// Most listened (radio-browser click count).
    #[default]
    Popular,
    /// Most clicked lately.
    Trending,
    /// Community votes.
    Votes,
    /// Highest bitrate first.
    Bitrate,
    Name,
    Random,
}

impl Order {
    fn api(self) -> (&'static str, bool) {
        match self {
            Self::Popular => ("clickcount", true),
            Self::Trending => ("clicktrend", true),
            Self::Votes => ("votes", true),
            Self::Bitrate => ("bitrate", true),
            Self::Name => ("name", false),
            Self::Random => ("random", false),
        }
    }
}

/// A directory search.
#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct SearchQuery {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    /// ISO 3166-1 alpha-2.
    #[serde(default)]
    pub country_code: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    /// `mp3`, `aac`, `flac`, `ogg`, `opus`.
    #[serde(default)]
    pub codec: Option<String>,
    #[serde(default)]
    pub min_bitrate: Option<u32>,
    /// Only lossless (FLAC) stations.
    #[serde(default)]
    pub lossless: bool,
    #[serde(default)]
    pub order: Order,
    #[serde(default)]
    pub offset: u32,
    #[serde(default)]
    pub limit: Option<u32>,
}

impl SearchQuery {
    /// The query string radio-browser expects.
    pub fn params(&self) -> Vec<(String, String)> {
        let mut params = Vec::new();
        let mut push = |key: &str, value: String| params.push((key.to_string(), value));
        if let Some(name) = clean(self.name.as_deref(), 100) {
            push("name", name);
        }
        if let Some(tag) = clean(self.tag.as_deref(), 60) {
            push("tag", tag.to_lowercase());
        }
        if let Some(code) = clean(self.country_code.as_deref(), 3)
            .filter(|code| code.len() == 2 && code.bytes().all(|byte| byte.is_ascii_alphabetic()))
        {
            push("countrycode", code.to_ascii_uppercase());
        }
        if let Some(language) = clean(self.language.as_deref(), 40) {
            push("language", language.to_lowercase());
        }
        let codec = if self.lossless {
            Some("FLAC".to_string())
        } else {
            clean(self.codec.as_deref(), 12).map(|codec| {
                match codec.to_ascii_lowercase().as_str() {
                    "vorbis" => "OGG".to_string(),
                    other => other.to_ascii_uppercase(),
                }
            })
        };
        if let Some(codec) = codec {
            push("codec", codec);
        }
        if let Some(rate) = self.min_bitrate.filter(|&rate| rate > 0) {
            push("bitrateMin", rate.min(10_000).to_string());
        }
        let (order, reverse) = self.order.api();
        push("order", order.to_string());
        push("reverse", reverse.to_string());
        push("hidebroken", "true".to_string());
        push("offset", self.offset.min(100_000).to_string());
        push(
            "limit",
            self.limit.unwrap_or(40).clamp(1, MAX_PAGE).to_string(),
        );
        params
    }
}

/// A browsing facet (tag, country or language) with its station count.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Facet {
    pub name: String,
    /// Country (ISO 3166-1) or language (ISO 639) code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub station_count: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FacetKind {
    Tags,
    Countries,
    Languages,
    Codecs,
}

impl FacetKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "tags" => Some(Self::Tags),
            "countries" => Some(Self::Countries),
            "languages" => Some(Self::Languages),
            "codecs" => Some(Self::Codecs),
            _ => None,
        }
    }

    fn path(self) -> &'static str {
        match self {
            Self::Tags => "/json/tags",
            Self::Countries => "/json/countries",
            Self::Languages => "/json/languages",
            Self::Codecs => "/json/codecs",
        }
    }
}

/// radio-browser's station record (only the fields Parson uses).
#[derive(Deserialize, Default)]
#[serde(default)]
struct RawStation {
    stationuuid: String,
    name: String,
    url: String,
    url_resolved: String,
    homepage: String,
    favicon: String,
    tags: String,
    country: String,
    countrycode: String,
    state: String,
    language: String,
    codec: String,
    bitrate: u32,
    hls: u8,
    votes: u64,
    clickcount: u64,
    clicktrend: i64,
    lastcheckok: u8,
}

fn split_list(value: &str, limit: usize) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    value
        .split(',')
        .map(|item| item.trim().to_lowercase())
        .filter(|item| !item.is_empty() && item.chars().count() <= 40)
        .filter(|item| seen.insert(item.clone()))
        .take(limit)
        .collect()
}

fn http_url(value: &str) -> Option<String> {
    let value = value.trim();
    (value.starts_with("http://") || value.starts_with("https://"))
        .then(|| value.chars().take(2048).collect())
}

fn into_station(raw: RawStation) -> Option<Station> {
    let id = raw.stationuuid.trim().to_ascii_lowercase();
    if !valid_station_id(&id) {
        return None;
    }
    let stream_url = http_url(&raw.url_resolved).or_else(|| http_url(&raw.url))?;
    let name = raw.name.split_whitespace().collect::<Vec<_>>().join(" ");
    let name = clean(Some(&name), 200)?;
    let tags = split_list(&raw.tags, 12);
    let mut codec = normalize_codec(&raw.codec);
    // The directory lists Ogg FLAC streams as "OGG" (or not at all); a
    // station that names FLAC is listed as FLAC until Parson hears it.
    let says_flac = name.to_lowercase().contains("flac")
        || tags.iter().any(|tag| tag == "flac" || tag == "lossless");
    if says_flac && matches!(codec.as_deref(), None | Some("vorbis")) {
        codec = Some("flac".into());
    }
    Some(Station {
        id,
        source: StationSource::Directory,
        name,
        stream_url,
        homepage: http_url(&raw.homepage),
        favicon: http_url(&raw.favicon),
        country: clean(Some(&raw.country), 80),
        country_code: clean(Some(&raw.countrycode), 2).map(|code| code.to_ascii_uppercase()),
        region: clean(Some(&raw.state), 80),
        languages: split_list(&raw.language, 4),
        tags,
        codec,
        bitrate_kbps: (raw.bitrate > 0).then_some(raw.bitrate),
        hls: raw.hls == 1,
        votes: Some(raw.votes),
        clicks: Some(raw.clickcount),
        click_trend: Some(raw.clicktrend),
        online: Some(raw.lastcheckok == 1),
        description: None,
        allow_lan: false,
    })
}

/// Parses a station list answer, dropping records Parson cannot play.
pub fn parse_stations(body: &[u8]) -> Result<Vec<Station>, DirectoryError> {
    let raw: Vec<RawStation> =
        serde_json::from_slice(body).map_err(|_| DirectoryError::BadResponse)?;
    Ok(raw.into_iter().filter_map(into_station).collect())
}

#[derive(Deserialize)]
struct RawFacet {
    name: String,
    #[serde(default)]
    iso_3166_1: Option<String>,
    #[serde(default)]
    iso_639: Option<String>,
    #[serde(default)]
    stationcount: u64,
}

fn parse_facets(body: &[u8]) -> Result<Vec<Facet>, DirectoryError> {
    let raw: Vec<RawFacet> =
        serde_json::from_slice(body).map_err(|_| DirectoryError::BadResponse)?;
    Ok(raw
        .into_iter()
        .filter(|facet| facet.stationcount > 0 && !facet.name.trim().is_empty())
        .map(|facet| Facet {
            name: facet.name.trim().chars().take(80).collect(),
            code: facet
                .iso_3166_1
                .or(facet.iso_639)
                .filter(|code| !code.trim().is_empty()),
            station_count: facet.stationcount,
        })
        .collect())
}

#[derive(Default)]
struct Servers {
    names: Vec<String>,
    /// Index of the server in use.
    current: usize,
    fetched: Option<Instant>,
}

struct Cached {
    at: Instant,
    ttl: Duration,
    body: Arc<Vec<u8>>,
}

/// The radio-browser client.
pub struct Directory {
    transport: Arc<dyn Transport>,
    servers: Mutex<Servers>,
    cache: Mutex<HashMap<String, Cached>>,
    shuffle_servers: bool,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Directory {
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self {
            transport,
            servers: Mutex::new(Servers::default()),
            cache: Mutex::new(HashMap::new()),
            shuffle_servers: true,
        }
    }

    /// Keeps the published server order (tests).
    #[cfg(test)]
    pub fn unshuffled(mut self) -> Self {
        self.shuffle_servers = false;
        self
    }

    fn request(&self, url: &str) -> ApiRequest {
        ApiRequest::get(url).header("User-Agent", user_agent())
    }

    /// The server list, refreshed every few hours; shuffled once so load
    /// spreads over the community's servers.
    async fn server_names(&self) -> Vec<String> {
        {
            let servers = lock(&self.servers);
            if servers
                .fetched
                .is_some_and(|at| at.elapsed() < SERVER_LIST_TTL)
                && !servers.names.is_empty()
            {
                let mut names = servers.names.clone();
                let start = servers.current % names.len();
                names.rotate_left(start);
                return names;
            }
        }
        let mut names: Vec<String> = match self.transport.send(self.request(SERVER_LIST_URL)).await
        {
            Ok(reply) if reply.status == 200 => {
                #[derive(Deserialize)]
                struct Server {
                    name: String,
                }
                serde_json::from_slice::<Vec<Server>>(&reply.body)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|server| server.name.trim().to_ascii_lowercase())
                    .filter(|name| {
                        name.ends_with(".api.radio-browser.info")
                            && name.bytes().all(|byte| {
                                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-')
                            })
                    })
                    .collect()
            }
            _ => Vec::new(),
        };
        names.sort();
        names.dedup();
        if names.is_empty() {
            names = FALLBACK_SERVERS
                .iter()
                .map(|name| name.to_string())
                .collect();
        }
        if self.shuffle_servers {
            use rand::seq::SliceRandom;
            names.shuffle(&mut rand::rng());
        }
        let mut servers = lock(&self.servers);
        servers.names = names.clone();
        servers.current = 0;
        servers.fetched = Some(Instant::now());
        names
    }

    fn cached(&self, key: &str) -> Option<Arc<Vec<u8>>> {
        let cache = lock(&self.cache);
        cache
            .get(key)
            .filter(|entry| entry.at.elapsed() < entry.ttl)
            .map(|entry| entry.body.clone())
    }

    fn store(&self, key: String, ttl: Duration, body: Arc<Vec<u8>>) {
        let mut cache = lock(&self.cache);
        if cache.len() >= MAX_CACHE_ENTRIES {
            cache.retain(|_, entry| entry.at.elapsed() < entry.ttl);
            if cache.len() >= MAX_CACHE_ENTRIES
                && let Some(oldest) = cache
                    .iter()
                    .min_by_key(|(_, entry)| entry.at)
                    .map(|(key, _)| key.clone())
            {
                cache.remove(&oldest);
            }
        }
        cache.insert(
            key,
            Cached {
                at: Instant::now(),
                ttl,
                body,
            },
        );
    }

    /// GETs `path` from a radio-browser server, trying up to three servers.
    async fn get(
        &self,
        path: &str,
        params: &[(String, String)],
        ttl: Option<Duration>,
    ) -> Result<Arc<Vec<u8>>, DirectoryError> {
        let key = format!(
            "{path}?{}",
            params
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join("&")
        );
        if ttl.is_some()
            && let Some(body) = self.cached(&key)
        {
            return Ok(body);
        }
        let names = self.server_names().await;
        let mut last_error = String::from("no servers");
        for (attempt, name) in names.iter().take(SERVERS_TRIED).enumerate() {
            let mut request = self.request(&format!("https://{name}{path}"));
            for (key, value) in params {
                request = request.query(key, value.clone());
            }
            match self.transport.send(request).await {
                Ok(reply) if reply.status == 200 => {
                    if attempt > 0 {
                        let mut servers = lock(&self.servers);
                        if let Some(index) = servers.names.iter().position(|server| server == name)
                        {
                            servers.current = index;
                        }
                    }
                    let body = Arc::new(reply.body);
                    if let Some(ttl) = ttl {
                        self.store(key, ttl, body.clone());
                    }
                    return Ok(body);
                }
                Ok(reply) => last_error = format!("HTTP {}", reply.status),
                Err(TransportError::Timeout) => last_error = "timed out".into(),
                Err(error) => last_error = error.to_string(),
            }
            tracing::debug!(server = %name, error = %last_error, "radio directory server failed");
        }
        Err(DirectoryError::Unavailable(last_error))
    }

    pub async fn search(&self, query: &SearchQuery) -> Result<Vec<Station>, DirectoryError> {
        let ttl = (query.order != Order::Random).then_some(SEARCH_TTL);
        let body = self
            .get("/json/stations/search", &query.params(), ttl)
            .await?;
        parse_stations(&body)
    }

    /// Stations by id, in the order asked.
    pub async fn stations(&self, ids: &[String]) -> Result<Vec<Station>, DirectoryError> {
        let ids: Vec<&String> = ids
            .iter()
            .filter(|id| valid_station_id(id))
            .take(100)
            .collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let joined = ids
            .iter()
            .map(|id| id.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let body = self
            .get(
                "/json/stations/byuuid",
                &[("uuids".to_string(), joined)],
                Some(STATION_TTL),
            )
            .await?;
        let mut stations = parse_stations(&body)?;
        stations.retain(|station| ids.iter().any(|id| **id == station.id));
        stations.sort_by_key(|station| {
            ids.iter()
                .position(|id| **id == station.id)
                .unwrap_or(usize::MAX)
        });
        Ok(stations)
    }

    pub async fn station(&self, id: &str) -> Result<Option<Station>, DirectoryError> {
        Ok(self.stations(&[id.to_string()]).await?.into_iter().next())
    }

    pub async fn facets(&self, kind: FacetKind, limit: u32) -> Result<Vec<Facet>, DirectoryError> {
        let params = vec![
            ("order".to_string(), "stationcount".to_string()),
            ("reverse".to_string(), "true".to_string()),
            ("hidebroken".to_string(), "true".to_string()),
            ("limit".to_string(), limit.clamp(1, 500).to_string()),
        ];
        let body = self.get(kind.path(), &params, Some(FACET_TTL)).await?;
        parse_facets(&body)
    }

    /// Tells radio-browser a listener started a station (its popularity
    /// ranking counts these). Failures are only logged.
    pub async fn report_click(&self, id: &str) {
        if !valid_station_id(id) {
            return;
        }
        if let Err(error) = self.get(&format!("/json/url/{id}"), &[], None).await {
            tracing::debug!(%error, "could not report a station click");
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use super::{Directory, FacetKind, Order, SearchQuery, parse_stations};
    use crate::live_radio::station::StationSource;
    use crate::providers::transport::{FixtureReply, FixtureRoute, FixtureTransport, Method};

    pub(crate) const FIXTURE: &str = include_str!("fixtures/radio_browser.json");

    pub(crate) fn directory() -> (Directory, Arc<FixtureTransport>) {
        let transport = Arc::new(FixtureTransport::from_json(&[FIXTURE]).expect("fixture"));
        (Directory::new(transport.clone()), transport)
    }

    #[test]
    fn records_become_stations_and_bad_ones_are_dropped() {
        let body = br#"[
            {"stationuuid":"D1A54D2E-623E-4970-AB11-35F7B56C5EC3","name":"  Classic   Vinyl HD ","url":"https://x.example/pls","url_resolved":"https://icecast.example:8443/classic","favicon":"https://icecast.example/logo.jpg","tags":"Jazz,jazz, Swing ,","countrycode":"us","language":"english,","codec":"MP3","bitrate":320,"hls":0,"votes":3,"clickcount":811,"clicktrend":12,"lastcheckok":1},
            {"stationuuid":"not-a-uuid","name":"Bad","url":"https://x.example/a"},
            {"stationuuid":"98adecf7-2683-4408-9be7-02d3f9098eb8","name":"No URL","url":"rtsp://x"},
            {"stationuuid":"e8f9ffdc-22a8-4c8b-82dd-ec63685feee6","name":"","url":"https://x.example/a"}
        ]"#;
        let stations = parse_stations(body).unwrap();
        assert_eq!(stations.len(), 1);
        let station = &stations[0];
        assert_eq!(station.id, "d1a54d2e-623e-4970-ab11-35f7b56c5ec3");
        assert_eq!(station.name, "Classic Vinyl HD");
        assert_eq!(station.stream_url, "https://icecast.example:8443/classic");
        assert_eq!(station.tags, ["jazz", "swing"]);
        assert_eq!(station.languages, ["english"]);
        assert_eq!(station.country_code.as_deref(), Some("US"));
        assert_eq!(station.codec.as_deref(), Some("mp3"));
        assert_eq!(station.source, StationSource::Directory);
        assert_eq!(station.online, Some(true));
        assert!(parse_stations(b"{}").is_err());
    }

    #[test]
    fn queries_map_to_radio_browser_parameters() {
        let query = SearchQuery {
            name: Some(" jazz24 ".into()),
            tag: Some("Jazz".into()),
            country_code: Some("us".into()),
            language: Some("English".into()),
            codec: Some("aac".into()),
            min_bitrate: Some(256),
            order: Order::Votes,
            limit: Some(500),
            ..SearchQuery::default()
        };
        let params = query.params();
        let get = |key: &str| {
            params
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(get("name"), Some("jazz24"));
        assert_eq!(get("tag"), Some("jazz"));
        assert_eq!(get("countrycode"), Some("US"));
        assert_eq!(get("language"), Some("english"));
        assert_eq!(get("codec"), Some("AAC"));
        assert_eq!(get("bitrateMin"), Some("256"));
        assert_eq!(get("order"), Some("votes"));
        assert_eq!(get("reverse"), Some("true"));
        assert_eq!(get("hidebroken"), Some("true"));
        assert_eq!(get("limit"), Some("100"));
        let lossless = SearchQuery {
            lossless: true,
            codec: Some("mp3".into()),
            country_code: Some("usa".into()),
            ..SearchQuery::default()
        };
        let params = lossless.params();
        assert!(params.contains(&("codec".into(), "FLAC".into())));
        assert!(!params.iter().any(|(key, _)| key == "countrycode"));
    }

    #[actix_web::test]
    async fn searches_pick_a_listed_server_send_a_user_agent_and_cache() {
        let (directory, transport) = directory();
        let stations = directory
            .search(&SearchQuery::default())
            .await
            .expect("search");
        assert_eq!(stations.len(), 6);
        assert_eq!(stations[0].name, "Classic Vinyl HD");
        let again = directory.search(&SearchQuery::default()).await.unwrap();
        assert_eq!(again, stations);
        let requests = transport.requests();
        assert_eq!(requests[0].url, super::SERVER_LIST_URL);
        let searches: Vec<_> = requests
            .iter()
            .filter(|request| request.url.ends_with("/json/stations/search"))
            .collect();
        assert_eq!(searches.len(), 1, "the second search came from the cache");
        assert!(
            searches[0]
                .url
                .starts_with("https://de1.api.radio-browser.info")
        );
        let agent = searches[0].header_value("User-Agent").expect("user agent");
        assert!(agent.starts_with("Parson/"), "{agent}");
        assert!(agent.contains("live radio"));
    }

    #[actix_web::test]
    async fn a_failing_server_falls_over_to_the_next() {
        let transport = Arc::new(FixtureTransport::from_json(&[FIXTURE]).expect("fixture"));
        transport.prepend(FixtureRoute {
            method: Method::Get,
            url: super::SERVER_LIST_URL.into(),
            query: Vec::new(),
            form: Vec::new(),
            replies: [FixtureReply {
                status: 200,
                body: serde_json::json!([
                    {"name": "de1.api.radio-browser.info"},
                    {"name": "fi1.api.radio-browser.info"},
                    {"name": "evil.example.com"}
                ]),
            }]
            .into(),
        });
        transport.prepend(FixtureRoute {
            method: Method::Get,
            url: "https://de1.api.radio-browser.info/json/tags".into(),
            query: Vec::new(),
            form: Vec::new(),
            replies: [FixtureReply {
                status: 503,
                body: serde_json::json!({}),
            }]
            .into(),
        });
        let directory = Directory::new(transport.clone()).unshuffled();
        let tags = directory.facets(FacetKind::Tags, 20).await.expect("tags");
        assert_eq!(tags[0].name, "pop");
        let hosts: Vec<_> = transport
            .requests()
            .iter()
            .filter(|request| request.url.ends_with("/json/tags"))
            .map(|request| request.url.clone())
            .collect();
        assert_eq!(hosts.len(), 2, "{hosts:?}");
        assert!(!hosts.iter().any(|url| url.contains("evil")));
    }

    #[actix_web::test]
    async fn stations_by_id_keep_the_requested_order_and_clicks_are_reported() {
        let (directory, transport) = directory();
        let ids = vec![
            "e8f9ffdc-22a8-4c8b-82dd-ec63685feee6".to_string(),
            "d1a54d2e-623e-4970-ab11-35f7b56c5ec3".to_string(),
            "../../etc".to_string(),
        ];
        let stations = directory.stations(&ids).await.expect("stations");
        assert_eq!(stations.len(), 2);
        assert_eq!(stations[0].id, ids[0]);
        assert_eq!(stations[0].codec.as_deref(), Some("flac"));
        directory.report_click(&ids[1]).await;
        directory.report_click("not-an-id").await;
        let clicks: Vec<_> = transport
            .requests()
            .into_iter()
            .filter(|request| request.url.contains("/json/url/"))
            .collect();
        assert_eq!(clicks.len(), 1);
        assert!(clicks[0].url.ends_with(&ids[1]));

        let countries = directory.facets(FacetKind::Countries, 10).await.unwrap();
        assert_eq!(countries[0].code.as_deref(), Some("US"));
        let languages = directory.facets(FacetKind::Languages, 10).await.unwrap();
        assert_eq!(languages[1].name, "spanish");
    }
}
