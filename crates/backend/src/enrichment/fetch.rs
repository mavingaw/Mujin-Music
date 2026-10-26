//! Outbound HTTP for enrichment: one polite client with a per-host rate
//! limit, a descriptive User-Agent and bounded bodies, behind a small trait
//! so tests can answer requests from fixtures instead of the network.

use std::collections::HashMap;
use std::future::Future;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// MusicBrainz asks clients to identify themselves with an application name,
/// version and contact URL.
pub const USER_AGENT: &str = concat!(
    "Parson/",
    env!("CARGO_PKG_VERSION"),
    " ( https://parson.dev/docs/accounts-privacy-security )"
);
/// MusicBrainz allows one request per second per client; stay a little under.
pub const MUSICBRAINZ_INTERVAL: Duration = Duration::from_millis(1_100);
/// Wikimedia answers bursts from anonymous clients with 429, even for
/// cached pages, so background enrichment stays well under one request a
/// second per host.
pub const WIKIMEDIA_INTERVAL: Duration = Duration::from_millis(900);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
pub const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_IMAGE_BYTES: usize = 6 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

impl FetchResponse {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Fetches a URL. Implementations must bound the body to `max_bytes`.
pub trait Fetch: Send + Sync {
    fn get(
        &self,
        url: &str,
        max_bytes: usize,
    ) -> impl Future<Output = Result<FetchResponse, BoxError>> + Send;
}

/// Minimum spacing between requests to one host.
pub fn host_interval(host: &str) -> Duration {
    if host == "musicbrainz.org" || host.ends_with(".musicbrainz.org") {
        MUSICBRAINZ_INTERVAL
    } else {
        WIKIMEDIA_INTERVAL
    }
}

pub fn url_host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['/', '?', '#']).next().unwrap_or_default()
}

/// How long to wait before the next request to a host that was last
/// contacted at `last`.
pub fn wait_needed(last: Option<Instant>, now: Instant, interval: Duration) -> Duration {
    match last {
        Some(last) => (last + interval).saturating_duration_since(now),
        None => Duration::ZERO,
    }
}

/// Per-host request spacing shared by every caller of one client.
#[derive(Default)]
pub struct RateLimiter {
    last: Mutex<HashMap<String, Instant>>,
}

impl RateLimiter {
    pub async fn acquire(&self, host: &str) {
        let mut last = self.last.lock().await;
        let wait = wait_needed(last.get(host).copied(), Instant::now(), host_interval(host));
        if !wait.is_zero() {
            // Holding the lock while sleeping serializes requests, which is
            // what a background worker wants.
            tokio::time::sleep(wait).await;
        }
        last.insert(host.to_string(), Instant::now());
    }
}

/// One limiter for the whole process, so the background worker, an
/// administrator's "Identify album" search and the library's work lookups
/// (`library::works_online`) together stay within MusicBrainz' one request
/// per second.
pub(crate) fn shared_limiter() -> &'static RateLimiter {
    static LIMITER: std::sync::OnceLock<RateLimiter> = std::sync::OnceLock::new();
    LIMITER.get_or_init(RateLimiter::default)
}

pub struct HttpFetch {
    client: reqwest::Client,
}

impl HttpFetch {
    pub fn new() -> Result<Self, BoxError> {
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .user_agent(USER_AGENT)
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()?;
        Ok(Self { client })
    }
}

impl Fetch for HttpFetch {
    async fn get(&self, url: &str, max_bytes: usize) -> Result<FetchResponse, BoxError> {
        if !url.starts_with("https://") {
            return Err(std::io::Error::other("enrichment only uses HTTPS").into());
        }
        shared_limiter().acquire(url_host(url)).await;
        let mut response = self
            .client
            .get(url)
            .header(reqwest::header::ACCEPT, "application/json, image/*;q=0.9")
            .send()
            .await?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        if response
            .content_length()
            .is_some_and(|length| length > max_bytes as u64)
        {
            return Err(std::io::Error::other("response exceeds size limit").into());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len().saturating_add(chunk.len()) > max_bytes {
                return Err(std::io::Error::other("response exceeds size limit").into());
            }
            body.extend_from_slice(&chunk);
        }
        Ok(FetchResponse {
            status,
            content_type,
            body,
        })
    }
}

#[cfg(test)]
pub mod mock {
    //! An in-memory [`Fetch`] that answers from fixtures and records requests.

    use super::{BoxError, Fetch, FetchResponse};
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MockFetch {
        responses: HashMap<String, FetchResponse>,
        pub requests: Mutex<Vec<String>>,
    }

    impl MockFetch {
        pub fn json(mut self, url: &str, body: serde_json::Value) -> Self {
            self.responses.insert(
                url.to_string(),
                FetchResponse {
                    status: 200,
                    content_type: Some("application/json".into()),
                    body: serde_json::to_vec(&body).unwrap(),
                },
            );
            self
        }

        pub fn bytes(mut self, url: &str, content_type: &str, body: &[u8]) -> Self {
            self.responses.insert(
                url.to_string(),
                FetchResponse {
                    status: 200,
                    content_type: Some(content_type.into()),
                    body: body.to_vec(),
                },
            );
            self
        }

        pub fn status(mut self, url: &str, status: u16) -> Self {
            self.responses.insert(
                url.to_string(),
                FetchResponse {
                    status,
                    content_type: None,
                    body: Vec::new(),
                },
            );
            self
        }

        pub fn requested(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl Fetch for MockFetch {
        async fn get(&self, url: &str, max_bytes: usize) -> Result<FetchResponse, BoxError> {
            self.requests.lock().unwrap().push(url.to_string());
            let response = self.responses.get(url).cloned().unwrap_or(FetchResponse {
                status: 404,
                content_type: None,
                body: Vec::new(),
            });
            if response.body.len() > max_bytes {
                return Err(std::io::Error::other("response exceeds size limit").into());
            }
            Ok(response)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn musicbrainz_is_spaced_at_least_one_second_and_wikimedia_less() {
        assert!(host_interval("musicbrainz.org") >= Duration::from_secs(1));
        assert!(host_interval("beta.musicbrainz.org") >= Duration::from_secs(1));
        assert!(host_interval("en.wikipedia.org") < Duration::from_secs(1));
        assert_eq!(
            url_host("https://musicbrainz.org/ws/2/artist?query=x"),
            "musicbrainz.org"
        );
        assert_eq!(
            url_host("https://www.wikidata.org/wiki/Special:EntityData/Q1.json"),
            "www.wikidata.org"
        );
    }

    #[test]
    fn wait_needed_spaces_requests_from_the_previous_one() {
        let start = Instant::now();
        let interval = Duration::from_millis(1_100);
        assert_eq!(wait_needed(None, start, interval), Duration::ZERO);
        assert_eq!(
            wait_needed(Some(start), start + Duration::from_millis(100), interval),
            Duration::from_millis(1_000)
        );
        assert_eq!(
            wait_needed(Some(start), start + Duration::from_secs(5), interval),
            Duration::ZERO
        );
    }

    #[test]
    fn user_agent_names_the_application_version_and_a_contact_url() {
        assert!(USER_AGENT.starts_with("Parson/"));
        assert!(USER_AGENT.contains(env!("CARGO_PKG_VERSION")));
        assert!(USER_AGENT.contains("https://"));
    }

    #[tokio::test]
    async fn the_limiter_delays_a_second_request_to_the_same_host() {
        let limiter = RateLimiter::default();
        let started = Instant::now();
        limiter.acquire("en.wikipedia.org").await;
        limiter.acquire("en.wikipedia.org").await;
        assert!(started.elapsed() >= WIKIMEDIA_INTERVAL);
        let other = Instant::now();
        limiter.acquire("www.wikidata.org").await;
        assert!(other.elapsed() < WIKIMEDIA_INTERVAL);
    }
}
