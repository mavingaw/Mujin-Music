//! Scrobbling: per-profile Last.fm and ListenBrainz connections.
//!
//! Every profile connects its own accounts. Playback events from any client
//! (web, desktop, mobile) reach the server, which applies the scrobbling
//! rules ([`rules`]), sends "now playing" right away and queues each
//! qualifying listen in `scrobble_queue`. A background worker
//! ([`hub::ScrobbleHub::run`]) delivers the queue in batches and retries with
//! backoff, so listens made while the server or a service is offline are
//! delivered later with their original timestamps.

pub mod hub;
pub mod import;
pub mod lastfm;
pub mod listenbrainz;
pub mod rules;
pub mod sandbox;
pub mod store;

use serde::{Deserialize, Serialize};

pub use hub::ScrobbleHub;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Service {
    Lastfm,
    Listenbrainz,
}

impl Service {
    pub const ALL: [Service; 2] = [Service::Lastfm, Service::Listenbrainz];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lastfm => "lastfm",
            Self::Listenbrainz => "listenbrainz",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "lastfm" => Some(Self::Lastfm),
            "listenbrainz" => Some(Self::Listenbrainz),
            _ => None,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Lastfm => "Last.fm",
            Self::Listenbrainz => "ListenBrainz",
        }
    }

    /// Most listens one submission may carry.
    pub fn max_batch(self) -> usize {
        match self {
            // track.scrobble accepts up to 50 scrobbles per request.
            Self::Lastfm => 50,
            // "import" accepts up to 1000; smaller batches keep payloads modest.
            Self::Listenbrainz => 100,
        }
    }
}

/// One listen as the services describe it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listen {
    pub artist: String,
    pub track: String,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub duration_seconds: Option<u32>,
    pub track_number: Option<u32>,
    pub release_mbid: Option<String>,
    pub isrc: Option<String>,
    /// Unix seconds when playback started.
    pub listened_at: i64,
    /// False for tracks Radio picked rather than the listener.
    pub chosen_by_user: bool,
}

/// What a service did with one submitted listen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemOutcome {
    Accepted,
    /// The service refused this listen for good (too old, filtered artist).
    Ignored(String),
    /// The service refused it for now (Last.fm's daily scrobble limit).
    Deferred(String),
}

/// A failure that applies to a whole request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceError {
    /// The network or the service is unavailable; retry later.
    Unavailable(String),
    /// Too many requests; retry later.
    RateLimited(String),
    /// The session or token was revoked; the listener must reconnect.
    AuthRevoked(String),
    /// The server's API credentials were rejected (Last.fm API key).
    BadCredentials(String),
    /// The request itself was refused; retrying the same payload fails again.
    Rejected(String),
}

impl ServiceError {
    pub fn message(&self) -> &str {
        match self {
            Self::Unavailable(message)
            | Self::RateLimited(message)
            | Self::AuthRevoked(message)
            | Self::BadCredentials(message)
            | Self::Rejected(message) => message,
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Unavailable(_) | Self::RateLimited(_) | Self::BadCredentials(_)
        )
    }
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for ServiceError {}

impl From<crate::providers::transport::TransportError> for ServiceError {
    fn from(error: crate::providers::transport::TransportError) -> Self {
        Self::Unavailable(error.to_string())
    }
}

/// Characters escaped in a URL path segment (RFC 3986 unreserved stay).
pub(crate) const PATH_SEGMENT: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

pub(crate) fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Trims a free-text field for a service payload; empty becomes `None`.
pub(crate) fn clean(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.chars().take(512).collect())
}
