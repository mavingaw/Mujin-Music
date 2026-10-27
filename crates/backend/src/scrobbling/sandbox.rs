//! Recorded Last.fm and ListenBrainz replies.
//!
//! The error replies the clients handle were recorded from the live services
//! (`ws.audioscrobbler.com`, `api.listenbrainz.org`); the success replies
//! follow the services' recorded shapes with a fictional listener and
//! catalogue. Tests replay them through `FixtureTransport`, and a server
//! started with `PARSON_SCROBBLING_SANDBOX=1` uses them instead of the
//! network so the whole flow can be tried without accounts.

use std::sync::Arc;

use crate::providers::transport::{FixtureTransport, Transport};

pub const LASTFM_FIXTURES: &str = include_str!("fixtures/lastfm.json");
pub const LISTENBRAINZ_FIXTURES: &str = include_str!("fixtures/listenbrainz.json");

/// Last.fm API credentials the sandbox pretends the server owner entered.
pub fn credentials() -> super::lastfm::Credentials {
    super::lastfm::Credentials {
        api_key: "5a4db0cfa1e2b3c4d5e6f708192a3b4c".to_string(),
        api_secret: "0f1e2d3c4b5a69788796a5b4c3d2e1f0".to_string(),
    }
}

pub fn enabled_from_env() -> bool {
    std::env::var("PARSON_SCROBBLING_SANDBOX")
        .map(|value| matches!(value.trim(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

pub fn transport() -> Arc<dyn Transport> {
    Arc::new(
        FixtureTransport::from_json(&[LASTFM_FIXTURES, LISTENBRAINZ_FIXTURES])
            .expect("embedded scrobbling fixtures are valid JSON"),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn embedded_fixtures_load() {
        let _ = super::transport();
    }
}
