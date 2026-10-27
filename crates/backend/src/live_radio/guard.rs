//! Outbound fetches for live radio with SSRF protection.
//!
//! A station URL comes from a listener or from a community directory, so
//! the server must never be talked into fetching something on its own
//! network: the router's admin page, a cloud metadata endpoint, or Parson's
//! own API on loopback. Every hop (including each redirect) is checked:
//!
//! - only `http` and `https`, no credentials in the URL;
//! - the host is resolved by Parson and *every* address must be public
//!   (a name that answers with one public and one private address is
//!   refused, so DNS tricks cannot slip a private address in);
//! - the connection is pinned to the addresses that were checked, so the
//!   name cannot be re-resolved to something else between check and use
//!   (DNS rebinding);
//! - redirects are followed by Parson, at most [`MAX_REDIRECTS`] times, and
//!   each target is checked again.
//!
//! An administrator can mark a station as a LAN stream, which allows
//! private, link-local and loopback addresses for that station only.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures::future::BoxFuture;
use futures::stream::{BoxStream, StreamExt};
use reqwest::Url;

/// Redirect hops a station may take before Parson gives up.
pub const MAX_REDIRECTS: usize = 5;

/// What a station may reach.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetPolicy {
    /// Private, link-local and loopback addresses are allowed (LAN
    /// streams, set by an administrator).
    pub allow_lan: bool,
}

impl NetPolicy {
    pub const PUBLIC: Self = Self { allow_lan: false };
    pub const LAN: Self = Self { allow_lan: true };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuardError {
    /// Not an `http(s)` URL, or it carries credentials.
    InvalidUrl(String),
    /// The host resolves to an address the policy does not allow.
    Blocked(String),
    Resolve(String),
    TooManyRedirects,
    Connect(String),
    Status(u16),
}

impl GuardError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidUrl(_) => "invalid_stream_url",
            Self::Blocked(_) => "stream_address_blocked",
            Self::Resolve(_) => "stream_host_not_found",
            Self::TooManyRedirects => "stream_redirect_loop",
            Self::Connect(_) => "stream_unreachable",
            Self::Status(_) => "stream_http_error",
        }
    }
}

impl std::fmt::Display for GuardError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidUrl(reason) => write!(formatter, "{reason}"),
            Self::Blocked(host) => write!(
                formatter,
                "{host} is on a private network. Only an administrator can add LAN streams."
            ),
            Self::Resolve(host) => write!(formatter, "Could not find the host {host}."),
            Self::TooManyRedirects => formatter.write_str("The stream redirected too many times."),
            Self::Connect(message) => write!(formatter, "Could not reach the stream: {message}"),
            Self::Status(status) => write!(formatter, "The stream answered HTTP {status}."),
        }
    }
}

impl std::error::Error for GuardError {}

/// Whether an address is on the public internet.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => is_public_v6(ip),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        // 0.0.0.0/8 "this network"
        || a == 0
        // 100.64.0.0/10 carrier-grade NAT
        || (a == 100 && (64..=127).contains(&b))
        // 192.0.0.0/24 IETF protocol assignments
        || (a == 192 && b == 0 && c == 0)
        // 198.18.0.0/15 benchmarking
        || (a == 198 && (b == 18 || b == 19))
        // 240.0.0.0/4 reserved
        || a >= 240)
}

fn embedded_v4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let segments = ip.segments();
    if let Some(v4) = ip.to_ipv4_mapped() {
        return Some(v4);
    }
    // 64:ff9b::/96 NAT64
    if segments[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        let octets = ip.octets();
        return Some(Ipv4Addr::new(
            octets[12], octets[13], octets[14], octets[15],
        ));
    }
    // 2002::/16 6to4
    if segments[0] == 0x2002 {
        let octets = ip.octets();
        return Some(Ipv4Addr::new(octets[2], octets[3], octets[4], octets[5]));
    }
    None
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = embedded_v4(ip) {
        return is_public_v4(v4);
    }
    let first = ip.segments()[0];
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        // fc00::/7 unique local
        || (first & 0xfe00) == 0xfc00
        // fe80::/10 link local, fec0::/10 site local
        || (first & 0xffc0) == 0xfe80
        || (first & 0xffc0) == 0xfec0
        // 2001:db8::/32 documentation
        || (first == 0x2001 && ip.segments()[1] == 0x0db8)
        // ::/96 IPv4-compatible (deprecated)
        || ip.segments()[..6] == [0, 0, 0, 0, 0, 0])
}

/// Whether `policy` lets a station reach `ip`.
pub fn allowed(ip: IpAddr, policy: NetPolicy) -> bool {
    if is_public(ip) {
        return true;
    }
    if !policy.allow_lan {
        return false;
    }
    // Even LAN streams never reach unspecified, multicast or broadcast.
    match ip {
        IpAddr::V4(ip) => !(ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast()),
        IpAddr::V6(ip) => !(ip.is_unspecified() || ip.is_multicast()),
    }
}

/// Parses and checks a station URL's shape (no network).
pub fn parse_url(raw: &str) -> Result<Url, GuardError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 2048 {
        return Err(GuardError::InvalidUrl("Enter a stream URL.".into()));
    }
    let url = Url::parse(trimmed)
        .map_err(|_| GuardError::InvalidUrl("That is not a valid URL.".into()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(GuardError::InvalidUrl(
            "Stream URLs start with http:// or https://.".into(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(GuardError::InvalidUrl(
            "Stream URLs cannot carry a user name or password.".into(),
        ));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(GuardError::InvalidUrl("The URL has no host.".into()));
    }
    Ok(url)
}

/// Finds the addresses of a host.
pub trait Resolve: Send + Sync {
    fn resolve(
        &self,
        host: String,
        port: u16,
    ) -> BoxFuture<'static, Result<Vec<SocketAddr>, String>>;
}

/// The system resolver.
pub struct SystemResolver;

impl Resolve for SystemResolver {
    fn resolve(
        &self,
        host: String,
        port: u16,
    ) -> BoxFuture<'static, Result<Vec<SocketAddr>, String>> {
        Box::pin(async move {
            tokio::time::timeout(
                Duration::from_secs(5),
                tokio::net::lookup_host((host.as_str(), port)),
            )
            .await
            .map_err(|_| "timed out".to_string())?
            .map(|addresses| addresses.collect())
            .map_err(|error| error.to_string())
        })
    }
}

pub type Body = BoxStream<'static, Result<Bytes, String>>;

/// A body that answers `None` for good once it has ended, however often it
/// is polled after that. Streams built with `unfold` panic when polled
/// again after their end, and a body is often read in parts (a head to
/// sniff, then the rest), so every body handed out here is fused.
pub fn fused(body: Body) -> Body {
    body.fuse().boxed()
}

/// One HTTP response, headers lower-cased.
pub struct Fetched {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Body,
}

impl Fetched {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// One request to an address that was already checked.
#[derive(Clone, Debug)]
pub struct PinnedRequest {
    pub url: Url,
    pub addresses: Vec<SocketAddr>,
    pub headers: Vec<(String, String)>,
}

/// Sends one request without following redirects.
pub trait Fetch: Send + Sync {
    fn fetch(&self, request: PinnedRequest) -> BoxFuture<'static, Result<Fetched, String>>;
}

/// Parson's User-Agent for station and directory requests.
pub fn user_agent() -> String {
    format!(
        "Parson/{} (self-hosted music server; live radio)",
        env!("CARGO_PKG_VERSION")
    )
}

/// `reqwest`, pinned to the checked addresses, never following redirects.
pub struct ReqwestFetch;

impl Fetch for ReqwestFetch {
    fn fetch(&self, request: PinnedRequest) -> BoxFuture<'static, Result<Fetched, String>> {
        Box::pin(async move {
            let host = request.url.host_str().unwrap_or_default().to_string();
            let mut builder = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(8))
                .read_timeout(Duration::from_secs(30))
                .user_agent(user_agent());
            if !host.is_empty() && host.parse::<IpAddr>().is_err() {
                builder = builder.resolve_to_addrs(&host, &request.addresses);
            }
            let client = builder.build().map_err(|error| error.to_string())?;
            let mut outgoing = client.get(request.url.clone());
            for (name, value) in &request.headers {
                outgoing = outgoing.header(name.as_str(), value.as_str());
            }
            let response = outgoing
                .send()
                .await
                .map_err(|error| error.without_url().to_string())?;
            let status = response.status().as_u16();
            let headers = response
                .headers()
                .iter()
                .filter_map(|(name, value)| {
                    Some((
                        name.as_str().to_ascii_lowercase(),
                        value.to_str().ok()?.to_string(),
                    ))
                })
                .collect();
            let body = futures::stream::unfold(Some(response), |state| async move {
                let mut response = state?;
                match response.chunk().await {
                    Ok(Some(chunk)) => Some((Ok(chunk), Some(response))),
                    Ok(None) => None,
                    Err(error) => Some((Err(error.without_url().to_string()), None)),
                }
            })
            .fuse()
            .boxed();
            Ok(Fetched {
                status,
                headers,
                body,
            })
        })
    }
}

/// A response and the URL it finally came from.
pub struct Opened {
    pub url: Url,
    pub response: Fetched,
}

/// Fetches station URLs safely (see the module docs).
#[derive(Clone)]
pub struct GuardedClient {
    resolver: Arc<dyn Resolve>,
    fetcher: Arc<dyn Fetch>,
}

impl GuardedClient {
    pub fn new(resolver: Arc<dyn Resolve>, fetcher: Arc<dyn Fetch>) -> Self {
        Self { resolver, fetcher }
    }

    pub fn system() -> Self {
        Self::new(Arc::new(SystemResolver), Arc::new(ReqwestFetch))
    }

    /// Checks a URL's host against the policy and returns its addresses.
    pub async fn check(&self, url: &Url, policy: NetPolicy) -> Result<Vec<SocketAddr>, GuardError> {
        let host = url
            .host_str()
            .ok_or_else(|| GuardError::InvalidUrl("The URL has no host.".into()))?;
        let port = url.port_or_known_default().unwrap_or(80);
        let bare = host.trim_start_matches('[').trim_end_matches(']');
        let addresses = match bare.parse::<IpAddr>() {
            Ok(ip) => vec![SocketAddr::new(ip, port)],
            Err(_) => {
                let lowered = bare.to_ascii_lowercase();
                if !policy.allow_lan
                    && (lowered == "localhost"
                        || lowered.ends_with(".localhost")
                        || lowered.ends_with(".local")
                        || lowered.ends_with(".internal")
                        || lowered.ends_with(".lan")
                        || lowered.ends_with(".home.arpa"))
                {
                    return Err(GuardError::Blocked(bare.to_string()));
                }
                self.resolver
                    .resolve(bare.to_string(), port)
                    .await
                    .map_err(|_| GuardError::Resolve(bare.to_string()))?
            }
        };
        if addresses.is_empty() {
            return Err(GuardError::Resolve(bare.to_string()));
        }
        if addresses
            .iter()
            .any(|address| !allowed(address.ip(), policy))
        {
            return Err(GuardError::Blocked(bare.to_string()));
        }
        Ok(addresses)
    }

    /// GETs `url`, following redirects (each one checked), and returns the
    /// first non-redirect response. Non-success statuses are errors.
    pub async fn open(
        &self,
        url: Url,
        policy: NetPolicy,
        headers: &[(&str, &str)],
    ) -> Result<Opened, GuardError> {
        let mut current = url;
        for _ in 0..=MAX_REDIRECTS {
            let addresses = self.check(&current, policy).await?;
            let response = self
                .fetcher
                .fetch(PinnedRequest {
                    url: current.clone(),
                    addresses,
                    headers: headers
                        .iter()
                        .map(|(name, value)| (name.to_string(), value.to_string()))
                        .collect(),
                })
                .await
                .map_err(GuardError::Connect)?;
            if (300..400).contains(&response.status)
                && let Some(location) = response.header("location")
            {
                let next = current.join(location.trim()).map_err(|_| {
                    GuardError::InvalidUrl("The stream redirected to an invalid URL.".into())
                })?;
                current = parse_url(next.as_str())?;
                continue;
            }
            // Shoutcast v1 answers `ICY 200 OK`, which reqwest reads as 200.
            if !(200..300).contains(&response.status) {
                return Err(GuardError::Status(response.status));
            }
            let response = Fetched {
                body: fused(response.body),
                ..response
            };
            return Ok(Opened {
                url: current,
                response,
            });
        }
        Err(GuardError::TooManyRedirects)
    }

    /// GETs a small text or image resource whole, up to `limit` bytes.
    pub async fn read(
        &self,
        url: Url,
        policy: NetPolicy,
        headers: &[(&str, &str)],
        limit: usize,
    ) -> Result<(Url, Vec<(String, String)>, Vec<u8>), GuardError> {
        let Opened { url, response } = self.open(url, policy, headers).await?;
        let headers = response.headers;
        let mut body = response.body;
        let mut bytes = Vec::new();
        let read = async {
            while let Some(chunk) = body.next().await {
                let chunk = chunk.map_err(GuardError::Connect)?;
                let room = limit.saturating_sub(bytes.len());
                bytes.extend_from_slice(&chunk[..chunk.len().min(room)]);
                if bytes.len() >= limit {
                    break;
                }
            }
            Ok::<_, GuardError>(())
        };
        tokio::time::timeout(Duration::from_secs(15), read)
            .await
            .map_err(|_| GuardError::Connect("timed out".into()))??;
        Ok((url, headers, bytes))
    }
}

#[cfg(test)]
pub(crate) mod fakes {
    use std::collections::HashMap;
    use std::net::SocketAddr;
    use std::sync::Mutex;

    use bytes::Bytes;
    use futures::future::BoxFuture;
    use futures::stream::StreamExt;

    use super::{Fetch, Fetched, PinnedRequest, Resolve};

    /// Resolves names from a table.
    #[derive(Default)]
    pub struct FakeResolver(pub HashMap<String, Vec<std::net::IpAddr>>);

    impl FakeResolver {
        pub fn with(mut self, host: &str, ips: &[&str]) -> Self {
            self.0.insert(
                host.to_string(),
                ips.iter().map(|ip| ip.parse().expect("ip")).collect(),
            );
            self
        }
    }

    impl Resolve for FakeResolver {
        fn resolve(
            &self,
            host: String,
            port: u16,
        ) -> BoxFuture<'static, Result<Vec<SocketAddr>, String>> {
            let found = self.0.get(&host).cloned();
            Box::pin(async move {
                found
                    .map(|ips| {
                        ips.into_iter()
                            .map(|ip| SocketAddr::new(ip, port))
                            .collect()
                    })
                    .ok_or_else(|| "not found".to_string())
            })
        }
    }

    #[derive(Clone)]
    pub struct FakeReply {
        pub status: u16,
        pub headers: Vec<(String, String)>,
        pub chunks: Vec<Vec<u8>>,
    }

    impl FakeReply {
        pub fn ok(content_type: &str, body: impl Into<Vec<u8>>) -> Self {
            Self {
                status: 200,
                headers: vec![("content-type".into(), content_type.into())],
                chunks: vec![body.into()],
            }
        }

        pub fn redirect(location: &str) -> Self {
            Self {
                status: 302,
                headers: vec![("location".into(), location.into())],
                chunks: Vec::new(),
            }
        }

        pub fn header(mut self, name: &str, value: &str) -> Self {
            self.headers.push((name.into(), value.into()));
            self
        }

        pub fn chunked(mut self, chunks: Vec<Vec<u8>>) -> Self {
            self.chunks = chunks;
            self
        }
    }

    /// Serves canned replies by URL and records each request.
    #[derive(Default)]
    pub struct FakeFetch {
        pub replies: Mutex<HashMap<String, Vec<FakeReply>>>,
        pub requests: Mutex<Vec<PinnedRequest>>,
    }

    impl FakeFetch {
        pub fn with(self, url: &str, reply: FakeReply) -> Self {
            self.replies
                .lock()
                .unwrap()
                .entry(url.to_string())
                .or_default()
                .push(reply);
            self
        }

        pub fn urls(&self) -> Vec<String> {
            self.requests
                .lock()
                .unwrap()
                .iter()
                .map(|request| request.url.to_string())
                .collect()
        }
    }

    impl Fetch for FakeFetch {
        fn fetch(&self, request: PinnedRequest) -> BoxFuture<'static, Result<Fetched, String>> {
            let url = request.url.to_string();
            self.requests.lock().unwrap().push(request);
            let reply = {
                let mut replies = self.replies.lock().unwrap();
                replies.get_mut(&url).and_then(|list| {
                    if list.len() > 1 {
                        Some(list.remove(0))
                    } else {
                        list.first().cloned()
                    }
                })
            };
            Box::pin(async move {
                let reply = reply.ok_or_else(|| format!("no fake reply for {url}"))?;
                Ok(Fetched {
                    status: reply.status,
                    headers: reply.headers,
                    body: futures::stream::iter(
                        reply.chunks.into_iter().map(|chunk| Ok(Bytes::from(chunk))),
                    )
                    .boxed(),
                })
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;
    use std::sync::Arc;

    use super::fakes::{FakeFetch, FakeReply, FakeResolver};
    use super::{GuardError, GuardedClient, NetPolicy, allowed, is_public, parse_url};

    fn ip(value: &str) -> IpAddr {
        value.parse().expect("ip")
    }

    #[test]
    fn private_loopback_and_special_addresses_are_not_public() {
        for blocked in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.20",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "0.1.2.3",
            "198.18.0.5",
            "224.0.0.251",
            "255.255.255.255",
            "240.0.0.1",
            "192.0.0.8",
            "::1",
            "::",
            "fd00::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "64:ff9b::a00:1",
            "2002:c0a8:0101::1",
            "::127.0.0.1",
        ] {
            assert!(!is_public(ip(blocked)), "{blocked} must not be public");
            assert!(!allowed(ip(blocked), NetPolicy::PUBLIC), "{blocked}");
        }
        for public in [
            "93.184.216.34",
            "8.8.8.8",
            "2606:4700::1111",
            "::ffff:93.184.216.34",
        ] {
            assert!(is_public(ip(public)), "{public} is public");
        }
    }

    #[test]
    fn lan_streams_reach_private_addresses_but_never_multicast() {
        assert!(allowed(ip("192.168.1.20"), NetPolicy::LAN));
        assert!(allowed(ip("127.0.0.1"), NetPolicy::LAN));
        assert!(allowed(ip("fd00::5"), NetPolicy::LAN));
        assert!(!allowed(ip("224.0.0.251"), NetPolicy::LAN));
        assert!(!allowed(ip("0.0.0.0"), NetPolicy::LAN));
        assert!(!allowed(ip("ff02::1"), NetPolicy::LAN));
    }

    #[test]
    fn urls_must_be_http_without_credentials() {
        assert!(parse_url("https://radio.example/stream").is_ok());
        assert!(parse_url("  http://radio.example:8000/live.mp3 ").is_ok());
        for bad in [
            "",
            "ftp://radio.example/stream",
            "file:///etc/passwd",
            "gopher://x",
            "https://user:secret@radio.example/stream",
            "not a url",
            "javascript:alert(1)",
        ] {
            assert!(
                matches!(parse_url(bad), Err(GuardError::InvalidUrl(_))),
                "{bad} must be rejected"
            );
        }
    }

    fn client(resolver: FakeResolver, fetch: FakeFetch) -> (GuardedClient, Arc<FakeFetch>) {
        let fetch = Arc::new(fetch);
        (GuardedClient::new(Arc::new(resolver), fetch.clone()), fetch)
    }

    #[actix_web::test]
    async fn names_that_resolve_to_any_private_address_are_refused() {
        let resolver = FakeResolver::default()
            .with("metadata.example", &["169.254.169.254"])
            .with("mixed.example", &["93.184.216.34", "10.0.0.7"])
            .with("router.example", &["192.168.0.1"]);
        let (client, fetch) = client(resolver, FakeFetch::default());
        for url in [
            "http://metadata.example/latest/meta-data/",
            "http://mixed.example/stream",
            "http://127.0.0.1:1993/api/v1/users",
            "http://[::1]/",
            "http://localhost:8000/stream",
            "http://nas.local/stream",
        ] {
            let error = client
                .open(parse_url(url).unwrap(), NetPolicy::PUBLIC, &[])
                .await
                .err()
                .expect("blocked");
            assert!(matches!(error, GuardError::Blocked(_)), "{url}: {error:?}");
        }
        assert!(fetch.urls().is_empty(), "nothing was fetched");

        // An administrator's LAN stream may reach the router.
        let (client, _) = client_with_lan();
        assert!(
            client
                .check(
                    &parse_url("http://router.example/").unwrap(),
                    NetPolicy::LAN
                )
                .await
                .is_ok()
        );
    }

    fn client_with_lan() -> (GuardedClient, Arc<FakeFetch>) {
        client(
            FakeResolver::default().with("router.example", &["192.168.0.1"]),
            FakeFetch::default(),
        )
    }

    #[actix_web::test]
    async fn every_redirect_hop_is_checked_and_pinned() {
        let resolver = FakeResolver::default()
            .with("radio.example", &["93.184.216.34"])
            .with("cdn.example", &["93.184.216.35"])
            .with("evil.example", &["10.0.0.1"]);
        let fetch = FakeFetch::default()
            .with(
                "http://radio.example/listen",
                FakeReply::redirect("https://cdn.example/live.mp3"),
            )
            .with(
                "https://cdn.example/live.mp3",
                FakeReply::ok("audio/mpeg", b"ID3".to_vec()),
            )
            .with(
                "http://radio.example/sneaky",
                FakeReply::redirect("http://evil.example/admin"),
            )
            .with("http://radio.example/loop", FakeReply::redirect("/loop"));
        let (client, fetch) = client(resolver, fetch);
        let opened = client
            .open(
                parse_url("http://radio.example/listen").unwrap(),
                NetPolicy::PUBLIC,
                &[("Icy-MetaData", "1")],
            )
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(opened.url.as_str(), "https://cdn.example/live.mp3");
        {
            let requests = fetch.requests.lock().unwrap();
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[1].addresses[0].ip(), ip("93.184.216.35"));
            assert_eq!(requests[1].addresses[0].port(), 443);
            assert_eq!(requests[1].headers[0].0, "Icy-MetaData");
        }

        let sneaky = client
            .open(
                parse_url("http://radio.example/sneaky").unwrap(),
                NetPolicy::PUBLIC,
                &[],
            )
            .await;
        assert!(matches!(sneaky, Err(GuardError::Blocked(_))));
        assert!(!fetch.urls().iter().any(|url| url.contains("evil")));

        let looped = client
            .open(
                parse_url("http://radio.example/loop").unwrap(),
                NetPolicy::PUBLIC,
                &[],
            )
            .await;
        assert!(matches!(looped, Err(GuardError::TooManyRedirects)));
    }

    #[actix_web::test]
    async fn error_statuses_and_unknown_hosts_are_reported() {
        let resolver = FakeResolver::default().with("radio.example", &["93.184.216.34"]);
        let fetch = FakeFetch::default().with(
            "http://radio.example/gone",
            FakeReply {
                status: 404,
                headers: Vec::new(),
                chunks: Vec::new(),
            },
        );
        let (client, _) = client(resolver, fetch);
        assert!(matches!(
            client
                .open(
                    parse_url("http://radio.example/gone").unwrap(),
                    NetPolicy::PUBLIC,
                    &[]
                )
                .await,
            Err(GuardError::Status(404))
        ));
        assert!(matches!(
            client
                .open(
                    parse_url("http://nowhere.example/").unwrap(),
                    NetPolicy::PUBLIC,
                    &[]
                )
                .await,
            Err(GuardError::Resolve(_))
        ));
    }
}
