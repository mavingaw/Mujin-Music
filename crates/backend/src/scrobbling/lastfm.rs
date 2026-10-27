//! Last.fm API client (official API 2.0).
//!
//! Implements the web authentication flow (`/api/auth` + `auth.getSession`),
//! `track.updateNowPlaying`, batched `track.scrobble` and
//! `user.getRecentTracks` for history import. Write methods are signed with
//! `api_sig` (md5 of the sorted parameters plus the shared secret) and sent as
//! form POSTs; responses are requested as JSON.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ItemOutcome, Listen, ServiceError};
use crate::providers::md5;
use crate::providers::transport::{ApiRequest, Transport};

pub const API_URL: &str = "https://ws.audioscrobbler.com/2.0/";
pub const AUTH_URL: &str = "https://www.last.fm/api/auth/";

/// The server owner's Last.fm API account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credentials {
    pub api_key: String,
    pub api_secret: String,
}

impl Credentials {
    /// Last.fm keys and secrets are 32 hexadecimal characters.
    pub fn plausible(value: &str) -> bool {
        value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
    }

    /// "1a2b…9f0e", safe to show in settings.
    pub fn key_hint(&self) -> String {
        let key = &self.api_key;
        if key.len() < 8 {
            return "…".to_string();
        }
        format!("{}…{}", &key[..4], &key[key.len() - 4..])
    }
}

/// An authorized Last.fm session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub name: String,
    pub key: String,
}

/// One page of `user.getRecentTracks`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecentPage {
    pub listens: Vec<RecentListen>,
    pub page: u32,
    pub total_pages: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecentListen {
    pub artist: String,
    pub track: String,
    pub album: Option<String>,
    pub listened_at: i64,
}

/// `api_sig` for a parameter list: every parameter except `format` and
/// `callback`, sorted by name, concatenated as name+value, then the secret.
pub fn sign(params: &[(String, String)], secret: &str) -> String {
    let mut sorted = params
        .iter()
        .filter(|(name, _)| name != "format" && name != "callback")
        .collect::<Vec<_>>();
    sorted.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    let mut base = String::new();
    for (name, value) in sorted {
        base.push_str(name);
        base.push_str(value);
    }
    base.push_str(secret);
    md5::hex_digest(&base)
}

/// The page a listener opens to approve Parson. Last.fm sends the browser
/// back to `callback` with `token` appended.
pub fn authorize_url(auth_base: &str, api_key: &str, callback: &str) -> String {
    format!(
        "{auth_base}?api_key={}&cb={}",
        percent_encoding::utf8_percent_encode(api_key, percent_encoding::NON_ALPHANUMERIC),
        percent_encoding::utf8_percent_encode(callback, percent_encoding::NON_ALPHANUMERIC),
    )
}

/// Maps a Last.fm error code to how the caller should react.
pub fn classify_error(code: i64, message: &str) -> ServiceError {
    let message = if message.trim().is_empty() {
        format!("Last.fm error {code}")
    } else {
        message.trim().to_string()
    };
    match code {
        // Invalid session key, unauthorized or expired token, auth failed.
        4 | 9 | 14 | 15 => ServiceError::AuthRevoked(message),
        // Invalid or suspended API key.
        10 | 26 => ServiceError::BadCredentials(message),
        // Operation failed, service offline, temporarily unavailable.
        8 | 11 | 16 => ServiceError::Unavailable(message),
        29 => ServiceError::RateLimited(message),
        _ => ServiceError::Rejected(message),
    }
}

fn number(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn text(value: Option<&Value>) -> Option<String> {
    let value = value?;
    let text = match value {
        Value::String(text) => text.clone(),
        Value::Object(object) => object.get("#text")?.as_str()?.to_string(),
        _ => return None,
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Turns a reply into JSON or a [`ServiceError`].
pub fn parse_reply(status: u16, body: &[u8]) -> Result<Value, ServiceError> {
    let parsed = serde_json::from_slice::<Value>(body).ok();
    if let Some(error) = parsed.as_ref().and_then(|value| number(value.get("error"))) {
        let message = parsed
            .as_ref()
            .and_then(|value| value.get("message"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        return Err(classify_error(error, message));
    }
    match (status, parsed) {
        (200..=299, Some(value)) => Ok(value),
        (429, _) => Err(ServiceError::RateLimited(
            "Last.fm is limiting requests.".to_string(),
        )),
        (500..=599, _) | (200..=299, None) => Err(ServiceError::Unavailable(format!(
            "Last.fm answered with HTTP {status}."
        ))),
        _ => Err(ServiceError::Rejected(format!(
            "Last.fm refused the request (HTTP {status})."
        ))),
    }
}

/// Reads the per-scrobble results of `track.scrobble`. Last.fm returns a
/// single object for one scrobble and an array for several.
pub fn scrobble_outcomes(value: &Value, submitted: usize) -> Vec<ItemOutcome> {
    let scrobbles = value
        .get("scrobbles")
        .and_then(|value| value.get("scrobble"));
    let items: Vec<&Value> = match scrobbles {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(item @ Value::Object(_)) => vec![item],
        _ => Vec::new(),
    };
    (0..submitted)
        .map(|index| {
            let Some(item) = items.get(index) else {
                return ItemOutcome::Accepted;
            };
            let ignored = item.get("ignoredMessage");
            let code = number(ignored.and_then(|value| value.get("code"))).unwrap_or(0);
            let reason = text(ignored).unwrap_or_else(|| match code {
                1 => "Last.fm ignored the artist.".to_string(),
                2 => "Last.fm ignored the track.".to_string(),
                3 => "Too old for Last.fm (older than 14 days).".to_string(),
                4 => "The timestamp is too far in the future.".to_string(),
                5 => "Daily scrobble limit reached.".to_string(),
                _ => format!("Last.fm ignored this scrobble (code {code})."),
            });
            match code {
                0 => ItemOutcome::Accepted,
                5 => ItemOutcome::Deferred(reason),
                _ => ItemOutcome::Ignored(reason),
            }
        })
        .collect()
}

pub fn parse_recent_tracks(value: &Value) -> Result<RecentPage, ServiceError> {
    let recent = value
        .get("recenttracks")
        .ok_or_else(|| ServiceError::Unavailable("Last.fm sent no recent tracks.".into()))?;
    let attributes = recent.get("@attr");
    let tracks: Vec<&Value> = match recent.get("track") {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(item @ Value::Object(_)) => vec![item],
        _ => Vec::new(),
    };
    let listens = tracks
        .into_iter()
        .filter_map(|track| {
            // The track playing right now has no date yet.
            if track
                .get("@attr")
                .and_then(|attr| attr.get("nowplaying"))
                .is_some()
            {
                return None;
            }
            Some(RecentListen {
                artist: text(track.get("artist"))
                    .or_else(|| text(track.get("artist").and_then(|a| a.get("name"))))?,
                track: text(track.get("name"))?,
                album: text(track.get("album")),
                listened_at: number(track.get("date").and_then(|date| date.get("uts")))?,
            })
        })
        .collect();
    Ok(RecentPage {
        listens,
        page: number(attributes.and_then(|attr| attr.get("page"))).unwrap_or(1) as u32,
        total_pages: number(attributes.and_then(|attr| attr.get("totalPages"))).unwrap_or(1) as u32,
    })
}

pub struct Client<'a> {
    pub transport: &'a dyn Transport,
    pub base: &'a str,
    pub credentials: &'a Credentials,
}

impl Client<'_> {
    fn signed(&self, method: &str, mut params: Vec<(String, String)>) -> Vec<(String, String)> {
        params.push(("method".to_string(), method.to_string()));
        params.push(("api_key".to_string(), self.credentials.api_key.clone()));
        let signature = sign(&params, &self.credentials.api_secret);
        params.push(("api_sig".to_string(), signature));
        params.push(("format".to_string(), "json".to_string()));
        params
    }

    async fn get(&self, params: Vec<(String, String)>) -> Result<Value, ServiceError> {
        let mut request = ApiRequest::get(self.base);
        for (key, value) in params {
            request = request.query(&key, value);
        }
        let reply = self.transport.send(request).await?;
        parse_reply(reply.status, &reply.body)
    }

    async fn post(&self, params: Vec<(String, String)>) -> Result<Value, ServiceError> {
        let mut request = ApiRequest::post(self.base);
        for (key, value) in params {
            request = request.form(&key, value);
        }
        let reply = self.transport.send(request).await?;
        parse_reply(reply.status, &reply.body)
    }

    /// `auth.getToken`: proves the API key and secret work.
    pub async fn get_token(&self) -> Result<String, ServiceError> {
        let value = self.get(self.signed("auth.getToken", Vec::new())).await?;
        value
            .get("token")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| ServiceError::Unavailable("Last.fm sent no token.".into()))
    }

    /// `auth.getSession`: exchanges an approved token for a session key.
    pub async fn get_session(&self, token: &str) -> Result<Session, ServiceError> {
        let value = self
            .get(self.signed(
                "auth.getSession",
                vec![("token".to_string(), token.to_string())],
            ))
            .await?;
        let session = value
            .get("session")
            .ok_or_else(|| ServiceError::Unavailable("Last.fm sent no session.".into()))?;
        Ok(Session {
            name: text(session.get("name"))
                .ok_or_else(|| ServiceError::Unavailable("Last.fm sent no user name.".into()))?,
            key: text(session.get("key"))
                .ok_or_else(|| ServiceError::Unavailable("Last.fm sent no session key.".into()))?,
        })
    }

    pub async fn update_now_playing(
        &self,
        session_key: &str,
        listen: &Listen,
    ) -> Result<(), ServiceError> {
        let mut params = vec![
            ("artist".to_string(), listen.artist.clone()),
            ("track".to_string(), listen.track.clone()),
            ("sk".to_string(), session_key.to_string()),
        ];
        push_optional(&mut params, "album", listen.album.as_deref());
        push_optional(&mut params, "albumArtist", listen.album_artist.as_deref());
        if let Some(duration) = listen.duration_seconds {
            params.push(("duration".to_string(), duration.to_string()));
        }
        if let Some(number) = listen.track_number {
            params.push(("trackNumber".to_string(), number.to_string()));
        }
        self.post(self.signed("track.updateNowPlaying", params))
            .await
            .map(|_| ())
    }

    /// `track.scrobble` for up to 50 listens.
    pub async fn scrobble(
        &self,
        session_key: &str,
        listens: &[Listen],
    ) -> Result<Vec<ItemOutcome>, ServiceError> {
        let mut params = vec![("sk".to_string(), session_key.to_string())];
        for (index, listen) in listens.iter().take(50).enumerate() {
            let field = |name: &str| format!("{name}[{index}]");
            params.push((field("artist"), listen.artist.clone()));
            params.push((field("track"), listen.track.clone()));
            params.push((field("timestamp"), listen.listened_at.to_string()));
            if let Some(album) = listen.album.as_deref() {
                params.push((field("album"), album.to_string()));
            }
            if let Some(album_artist) = listen.album_artist.as_deref() {
                params.push((field("albumArtist"), album_artist.to_string()));
            }
            if let Some(duration) = listen.duration_seconds {
                params.push((field("duration"), duration.to_string()));
            }
            if let Some(number) = listen.track_number {
                params.push((field("trackNumber"), number.to_string()));
            }
            params.push((
                field("chosenByUser"),
                if listen.chosen_by_user { "1" } else { "0" }.to_string(),
            ));
        }
        let value = self.post(self.signed("track.scrobble", params)).await?;
        Ok(scrobble_outcomes(&value, listens.len().min(50)))
    }

    /// One page of a user's scrobbles, newest first.
    pub async fn recent_tracks(
        &self,
        user: &str,
        page: u32,
        limit: u32,
    ) -> Result<RecentPage, ServiceError> {
        let params = vec![
            ("method".to_string(), "user.getRecentTracks".to_string()),
            ("user".to_string(), user.to_string()),
            ("api_key".to_string(), self.credentials.api_key.clone()),
            ("limit".to_string(), limit.clamp(1, 200).to_string()),
            ("page".to_string(), page.max(1).to_string()),
            ("format".to_string(), "json".to_string()),
        ];
        parse_recent_tracks(&self.get(params).await?)
    }
}

fn push_optional(params: &mut Vec<(String, String)>, name: &str, value: Option<&str>) {
    if let Some(value) = value {
        params.push((name.to_string(), value.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::transport::{FixtureTransport, Method};
    use crate::scrobbling::sandbox;

    fn credentials() -> Credentials {
        Credentials {
            api_key: "0123456789abcdef0123456789abcdef".into(),
            api_secret: "fedcba9876543210fedcba9876543210".into(),
        }
    }

    fn listen(track: &str, at: i64) -> Listen {
        Listen {
            artist: "Ilse Marten".into(),
            track: track.into(),
            album: Some("Glass Orchard".into()),
            album_artist: Some("Ilse Marten".into()),
            duration_seconds: Some(214),
            track_number: Some(3),
            listened_at: at,
            chosen_by_user: true,
            ..Listen::default()
        }
    }

    #[test]
    fn signatures_follow_the_documented_recipe() {
        // Sorted by name, format excluded, secret appended, md5 hex.
        let params = vec![
            ("token".to_string(), "abc".to_string()),
            ("method".to_string(), "auth.getSession".to_string()),
            ("api_key".to_string(), "key".to_string()),
            ("format".to_string(), "json".to_string()),
        ];
        assert_eq!(
            sign(&params, "secret"),
            md5::hex_digest("api_keykeymethodauth.getSessiontokenabcsecret")
        );
        // Array parameters sort by their full name, brackets included.
        let params = vec![
            ("track[1]".to_string(), "b".to_string()),
            ("track[0]".to_string(), "a".to_string()),
        ];
        assert_eq!(sign(&params, "s"), md5::hex_digest("track[0]atrack[1]bs"));
    }

    #[test]
    fn authorize_url_encodes_the_callback() {
        let url = authorize_url(
            AUTH_URL,
            "abc",
            "http://192.168.1.4:1993/connect/lastfm?state=x y",
        );
        assert_eq!(
            url,
            "https://www.last.fm/api/auth/?api_key=abc&cb=http%3A%2F%2F192%2E168%2E1%2E4%3A1993%2Fconnect%2Flastfm%3Fstate%3Dx%20y"
        );
    }

    #[test]
    fn recorded_errors_map_to_reactions() {
        // Recorded from ws.audioscrobbler.com with an unregistered key.
        let error = parse_reply(
            403,
            br#"{"message":"Invalid API key - You must be granted a valid key by last.fm","error":10}"#,
        )
        .expect_err("invalid key");
        assert!(matches!(error, ServiceError::BadCredentials(_)));
        assert!(matches!(
            parse_reply(
                403,
                br#"{"error":9,"message":"Invalid session key - Please re-authenticate"}"#
            ),
            Err(ServiceError::AuthRevoked(_))
        ));
        assert!(matches!(
            parse_reply(200, br#"{"error":11,"message":"Service Offline"}"#),
            Err(ServiceError::Unavailable(_))
        ));
        assert!(matches!(
            parse_reply(429, br#"{"error":29,"message":"Rate Limit Exceeded"}"#),
            Err(ServiceError::RateLimited(_))
        ));
        assert!(matches!(
            parse_reply(502, b"<html>Bad gateway</html>"),
            Err(ServiceError::Unavailable(_))
        ));
        assert!(matches!(
            parse_reply(400, br#"{"error":6,"message":"Invalid parameters"}"#),
            Err(ServiceError::Rejected(_))
        ));
    }

    #[test]
    fn scrobble_results_cover_single_and_batched_replies() {
        let single: Value = serde_json::from_str(
            r##"{"scrobbles":{"scrobble":{"ignoredMessage":{"code":"0","#text":""}},"@attr":{"accepted":1,"ignored":0}}}"##,
        )
        .unwrap();
        assert_eq!(scrobble_outcomes(&single, 1), vec![ItemOutcome::Accepted]);
        let batch: Value = serde_json::from_str(
            r##"{"scrobbles":{"scrobble":[
                {"ignoredMessage":{"code":"0","#text":""}},
                {"ignoredMessage":{"code":"3","#text":"Timestamp too old"}},
                {"ignoredMessage":{"code":"5","#text":""}}
            ],"@attr":{"accepted":1,"ignored":2}}}"##,
        )
        .unwrap();
        assert_eq!(
            scrobble_outcomes(&batch, 3),
            vec![
                ItemOutcome::Accepted,
                ItemOutcome::Ignored("Timestamp too old".into()),
                ItemOutcome::Deferred("Daily scrobble limit reached.".into()),
            ]
        );
    }

    #[actix_web::test]
    async fn web_auth_now_playing_and_batched_scrobbles_against_recorded_replies() {
        let transport = FixtureTransport::from_json(&[sandbox::LASTFM_FIXTURES]).unwrap();
        let credentials = credentials();
        let client = Client {
            transport: &transport,
            base: API_URL,
            credentials: &credentials,
        };
        let session = client.get_session("approved-token").await.expect("session");
        assert_eq!(session.name, "parson-listener");
        assert_eq!(session.key, "recorded-session-key");

        client
            .update_now_playing(&session.key, &listen("Lanterns", 1_700_000_000))
            .await
            .expect("now playing");
        let outcomes = client
            .scrobble(
                &session.key,
                &[
                    listen("Lanterns", 1_700_000_000),
                    listen("Copper Sky", 1_700_000_300),
                ],
            )
            .await
            .expect("scrobble");
        assert_eq!(outcomes, vec![ItemOutcome::Accepted, ItemOutcome::Accepted]);

        let requests = transport.requests();
        let session_request = &requests[0];
        assert_eq!(session_request.method, Method::Get);
        assert_eq!(
            session_request.query_value("method"),
            Some("auth.getSession")
        );
        assert_eq!(session_request.query_value("token"), Some("approved-token"));
        assert_eq!(session_request.query_value("format"), Some("json"));
        let expected = sign(
            &session_request
                .query
                .iter()
                .filter(|(name, _)| name != "api_sig")
                .cloned()
                .collect::<Vec<_>>(),
            &credentials.api_secret,
        );
        assert_eq!(
            session_request.query_value("api_sig"),
            Some(expected.as_str())
        );

        let now_playing = &requests[1];
        assert_eq!(now_playing.method, Method::Post);
        assert_eq!(
            now_playing.form_value("method"),
            Some("track.updateNowPlaying")
        );
        assert_eq!(now_playing.form_value("sk"), Some("recorded-session-key"));
        assert_eq!(now_playing.form_value("duration"), Some("214"));
        assert!(now_playing.form_value("timestamp").is_none());

        let scrobble = &requests[2];
        assert_eq!(scrobble.form_value("method"), Some("track.scrobble"));
        assert_eq!(scrobble.form_value("track[1]"), Some("Copper Sky"));
        assert_eq!(scrobble.form_value("timestamp[0]"), Some("1700000000"));
        assert_eq!(scrobble.form_value("albumArtist[0]"), Some("Ilse Marten"));
        assert_eq!(scrobble.form_value("chosenByUser[0]"), Some("1"));
    }

    #[actix_web::test]
    async fn recent_tracks_skip_the_track_playing_now() {
        let transport = FixtureTransport::from_json(&[sandbox::LASTFM_FIXTURES]).unwrap();
        let credentials = credentials();
        let client = Client {
            transport: &transport,
            base: API_URL,
            credentials: &credentials,
        };
        let page = client
            .recent_tracks("parson-listener", 1, 200)
            .await
            .expect("recent tracks");
        assert_eq!(page.page, 1);
        assert_eq!(page.total_pages, 2);
        assert!(page.listens.iter().all(|listen| listen.listened_at > 0));
        assert_eq!(page.listens[0].artist, "Ilse Marten");
        assert_eq!(page.listens[0].album.as_deref(), Some("Glass Orchard"));
        let request = &transport.requests()[0];
        assert_eq!(request.query_value("method"), Some("user.getRecentTracks"));
        assert!(request.query_value("api_sig").is_none());
    }
}
