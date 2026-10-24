//! HTTP transport for provider API calls.
//!
//! Provider clients build [`ApiRequest`]s and never talk to `reqwest`
//! directly, so the same client code runs against the live services or
//! against recorded fixtures ([`FixtureTransport`]) in tests and in the
//! server's sandbox mode.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use futures::future::BoxFuture;
use serde::Deserialize;

/// Upper bound for any provider API response body.
pub const MAX_API_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    Get,
    Post,
    Delete,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiRequest {
    pub method: Method,
    pub url: String,
    pub query: Vec<(String, String)>,
    pub form: Option<Vec<(String, String)>>,
    pub headers: Vec<(String, String)>,
}

impl ApiRequest {
    pub fn new(method: Method, url: impl Into<String>) -> Self {
        Self {
            method,
            url: url.into(),
            query: Vec::new(),
            form: None,
            headers: Vec::new(),
        }
    }

    pub fn get(url: impl Into<String>) -> Self {
        Self::new(Method::Get, url)
    }

    pub fn post(url: impl Into<String>) -> Self {
        Self::new(Method::Post, url)
    }

    pub fn delete(url: impl Into<String>) -> Self {
        Self::new(Method::Delete, url)
    }

    pub fn query(mut self, key: &str, value: impl Into<String>) -> Self {
        self.query.push((key.to_string(), value.into()));
        self
    }

    pub fn form(mut self, key: &str, value: impl Into<String>) -> Self {
        self.form
            .get_or_insert_with(Vec::new)
            .push((key.to_string(), value.into()));
        self
    }

    pub fn header(mut self, key: &str, value: impl Into<String>) -> Self {
        self.headers.push((key.to_string(), value.into()));
        self
    }

    pub fn query_value(&self, key: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    pub fn form_value(&self, key: &str) -> Option<&str> {
        self.form
            .as_ref()?
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    pub fn header_value(&self, key: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiReply {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug)]
pub enum TransportError {
    Timeout,
    Connect(String),
    TooLarge,
    Unmatched(String),
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => formatter.write_str("the streaming service did not answer in time"),
            Self::Connect(message) => write!(formatter, "could not reach the service: {message}"),
            Self::TooLarge => formatter.write_str("the service response exceeded the size limit"),
            Self::Unmatched(route) => write!(formatter, "no recorded response for {route}"),
        }
    }
}

impl std::error::Error for TransportError {}

pub trait Transport: Send + Sync {
    fn send(&self, request: ApiRequest) -> BoxFuture<'_, Result<ApiReply, TransportError>>;
}

pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub fn new() -> Result<Self, reqwest::Error> {
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(20))
                .pool_idle_timeout(Duration::from_secs(90))
                .user_agent(concat!("Parson/", env!("CARGO_PKG_VERSION")))
                .build()?,
        })
    }
}

impl Transport for ReqwestTransport {
    fn send(&self, request: ApiRequest) -> BoxFuture<'_, Result<ApiReply, TransportError>> {
        Box::pin(async move {
            let mut builder = match request.method {
                Method::Get => self.client.get(&request.url),
                Method::Post => self.client.post(&request.url),
                Method::Delete => self.client.delete(&request.url),
            };
            if !request.query.is_empty() {
                builder = builder.query(&request.query);
            }
            for (name, value) in &request.headers {
                builder = builder.header(name.as_str(), value.as_str());
            }
            if let Some(form) = &request.form {
                let body = form
                    .iter()
                    .map(|(key, value)| format!("{}={}", form_encode(key), form_encode(value)))
                    .collect::<Vec<_>>()
                    .join("&");
                builder = builder
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(body);
            }
            let mut response = builder.send().await.map_err(|error| {
                if error.is_timeout() {
                    TransportError::Timeout
                } else {
                    TransportError::Connect(error.without_url().to_string())
                }
            })?;
            let status = response.status().as_u16();
            if response
                .content_length()
                .is_some_and(|length| length > MAX_API_RESPONSE_BYTES as u64)
            {
                return Err(TransportError::TooLarge);
            }
            let mut body = Vec::new();
            loop {
                match response.chunk().await {
                    Ok(Some(chunk)) => {
                        if body.len().saturating_add(chunk.len()) > MAX_API_RESPONSE_BYTES {
                            return Err(TransportError::TooLarge);
                        }
                        body.extend_from_slice(&chunk);
                    }
                    Ok(None) => break,
                    Err(error) if error.is_timeout() => return Err(TransportError::Timeout),
                    Err(error) => {
                        return Err(TransportError::Connect(error.without_url().to_string()));
                    }
                }
            }
            Ok(ApiReply { status, body })
        })
    }
}

/// `application/x-www-form-urlencoded` encoding for one key or value.
pub fn form_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'*' => {
                encoded.push(byte as char)
            }
            b' ' => encoded.push('+'),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// One recorded route: a method, URL (a trailing `*` matches any suffix) and
/// required query/form values, and the replies it gives in order (the last
/// reply repeats).
#[derive(Debug, Deserialize)]
pub struct FixtureRoute {
    pub method: Method,
    pub url: String,
    #[serde(default)]
    pub query: Vec<(String, String)>,
    #[serde(default)]
    pub form: Vec<(String, String)>,
    pub replies: VecDeque<FixtureReply>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct FixtureReply {
    pub status: u16,
    pub body: serde_json::Value,
}

impl FixtureRoute {
    fn matches(&self, request: &ApiRequest) -> bool {
        let url_matches = match self.url.strip_suffix('*') {
            Some(prefix) => request.url.starts_with(prefix),
            None => self.url == request.url,
        };
        self.method == request.method
            && url_matches
            && self
                .query
                .iter()
                .all(|(key, value)| request.query_value(key) == Some(value.as_str()))
            && self
                .form
                .iter()
                .all(|(key, value)| request.form_value(key) == Some(value.as_str()))
    }
}

/// Replays recorded provider responses. Routes are tried in order, so more
/// specific routes (with query requirements) go first in a fixture file.
#[derive(Default)]
pub struct FixtureTransport {
    routes: Mutex<Vec<FixtureRoute>>,
    log: Mutex<Vec<ApiRequest>>,
}

impl FixtureTransport {
    pub fn from_json(fixtures: &[&str]) -> Result<Self, serde_json::Error> {
        let mut routes = Vec::new();
        for fixture in fixtures {
            routes.extend(serde_json::from_str::<Vec<FixtureRoute>>(fixture)?);
        }
        Ok(Self {
            routes: Mutex::new(routes),
            log: Mutex::new(Vec::new()),
        })
    }

    /// Adds a route ahead of the recorded ones.
    pub fn prepend(&self, route: FixtureRoute) {
        self.routes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(0, route);
    }

    /// Requests seen so far, for assertions.
    pub fn requests(&self) -> Vec<ApiRequest> {
        self.log
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl Transport for FixtureTransport {
    fn send(&self, request: ApiRequest) -> BoxFuture<'_, Result<ApiReply, TransportError>> {
        Box::pin(async move {
            self.log
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(request.clone());
            let mut routes = self
                .routes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(route) = routes.iter_mut().find(|route| route.matches(&request)) else {
                return Err(TransportError::Unmatched(format!(
                    "{:?} {}",
                    request.method, request.url
                )));
            };
            let reply = if route.replies.len() > 1 {
                route.replies.pop_front()
            } else {
                route.replies.front().cloned()
            }
            .ok_or_else(|| TransportError::Unmatched(request.url.clone()))?;
            Ok(ApiReply {
                status: reply.status,
                body: serde_json::to_vec(&reply.body).unwrap_or_default(),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{ApiRequest, FixtureTransport, Transport, form_encode};

    #[test]
    fn form_values_are_encoded_for_oauth_bodies() {
        assert_eq!(
            form_encode("urn:ietf:params:oauth:grant-type:device_code"),
            "urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code"
        );
        assert_eq!(form_encode("r_usr w_usr"), "r_usr+w_usr");
    }

    #[actix_web::test]
    async fn fixture_routes_match_on_query_and_replay_in_order() {
        let transport = FixtureTransport::from_json(&[r#"[
            {"method":"GET","url":"https://x.test/a","query":[["k","1"]],
             "replies":[{"status":400,"body":{"n":1}},{"status":200,"body":{"n":2}}]},
            {"method":"GET","url":"https://x.test/a","replies":[{"status":200,"body":{"n":0}}]}
        ]"#])
        .expect("fixtures");
        let first = transport
            .send(ApiRequest::get("https://x.test/a").query("k", "1"))
            .await
            .expect("first");
        assert_eq!(first.status, 400);
        for _ in 0..2 {
            let repeated = transport
                .send(ApiRequest::get("https://x.test/a").query("k", "1"))
                .await
                .expect("repeat");
            assert_eq!(repeated.status, 200);
        }
        let fallback = transport
            .send(ApiRequest::get("https://x.test/a"))
            .await
            .expect("fallback");
        assert_eq!(fallback.body, br#"{"n":0}"#);
        assert!(
            transport
                .send(ApiRequest::get("https://x.test/missing"))
                .await
                .is_err()
        );
        assert_eq!(transport.requests().len(), 5);
    }
}
