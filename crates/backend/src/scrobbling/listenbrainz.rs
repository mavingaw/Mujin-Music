//! ListenBrainz API client.
//!
//! A listener pastes their user token (listenbrainz.org/settings). Parson
//! validates it, then submits `playing_now`, `single` and `import` listens
//! and can read the listener's history for import. The API root is
//! configurable so self-hosted ListenBrainz servers work too.

use serde_json::{Value, json};

use super::{Listen, ServiceError};
use crate::providers::transport::{ApiRequest, Transport};

pub const API_URL: &str = "https://api.listenbrainz.org";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListenType {
    PlayingNow,
    Single,
    Import,
}

impl ListenType {
    fn as_str(self) -> &'static str {
        match self {
            Self::PlayingNow => "playing_now",
            Self::Single => "single",
            Self::Import => "import",
        }
    }
}

/// One listen read back from ListenBrainz.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteListen {
    pub artist: String,
    pub track: String,
    pub album: Option<String>,
    pub listened_at: i64,
}

/// ListenBrainz user tokens are UUIDs.
pub fn plausible_token(token: &str) -> bool {
    uuid::Uuid::parse_str(token.trim()).is_ok()
}

/// The JSON body of a submission. `playing_now` carries no timestamp.
pub fn submission(kind: ListenType, listens: &[Listen]) -> Value {
    let payload = listens
        .iter()
        .map(|listen| {
            let mut info = serde_json::Map::new();
            info.insert("media_player".into(), json!("Mujin Music"));
            info.insert("submission_client".into(), json!("Mujin Music"));
            info.insert(
                "submission_client_version".into(),
                json!(env!("CARGO_PKG_VERSION")),
            );
            if let Some(duration) = listen.duration_seconds {
                info.insert("duration_ms".into(), json!(u64::from(duration) * 1000));
            }
            if let Some(number) = listen.track_number {
                info.insert("tracknumber".into(), json!(number));
            }
            if let Some(mbid) = listen.release_mbid.as_deref() {
                info.insert("release_mbid".into(), json!(mbid));
            }
            if let Some(isrc) = listen.isrc.as_deref() {
                info.insert("isrc".into(), json!(isrc));
            }
            if let Some(album_artist) = listen.album_artist.as_deref()
                && album_artist != listen.artist
            {
                info.insert("album_artist".into(), json!(album_artist));
            }
            let mut metadata = json!({
                "artist_name": listen.artist,
                "track_name": listen.track,
                "additional_info": Value::Object(info),
            });
            if let Some(album) = listen.album.as_deref() {
                metadata["release_name"] = json!(album);
            }
            let mut item = json!({ "track_metadata": metadata });
            if kind != ListenType::PlayingNow {
                item["listened_at"] = json!(listen.listened_at);
            }
            item
        })
        .collect::<Vec<_>>();
    json!({ "listen_type": kind.as_str(), "payload": payload })
}

pub fn parse_reply(status: u16, body: &[u8]) -> Result<Value, ServiceError> {
    let parsed = serde_json::from_slice::<Value>(body).ok();
    let message = parsed
        .as_ref()
        .and_then(|value| value.get("error").or_else(|| value.get("message")))
        .and_then(Value::as_str)
        .map(str::to_string);
    match status {
        200..=299 => parsed.ok_or_else(|| {
            ServiceError::Unavailable("ListenBrainz sent an unreadable reply.".into())
        }),
        401 => Err(ServiceError::AuthRevoked(message.unwrap_or_else(|| {
            "ListenBrainz no longer accepts this token.".into()
        }))),
        429 => {
            Err(ServiceError::RateLimited(message.unwrap_or_else(|| {
                "ListenBrainz is limiting requests.".into()
            })))
        }
        500..=599 => {
            Err(ServiceError::Unavailable(message.unwrap_or_else(|| {
                format!("ListenBrainz answered with HTTP {status}.")
            })))
        }
        _ => Err(ServiceError::Rejected(message.unwrap_or_else(|| {
            format!("ListenBrainz refused the request (HTTP {status}).")
        }))),
    }
}

pub fn parse_listens(value: &Value) -> Vec<RemoteListen> {
    value
        .get("payload")
        .and_then(|payload| payload.get("listens"))
        .and_then(Value::as_array)
        .map(|listens| {
            listens
                .iter()
                .filter_map(|listen| {
                    let metadata = listen.get("track_metadata")?;
                    let text = |key: &str| {
                        metadata
                            .get(key)
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(str::to_string)
                    };
                    Some(RemoteListen {
                        artist: text("artist_name")?,
                        track: text("track_name")?,
                        album: text("release_name"),
                        listened_at: listen.get("listened_at")?.as_i64()?,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub struct Client<'a> {
    pub transport: &'a dyn Transport,
    pub base: &'a str,
}

impl Client<'_> {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base.trim_end_matches('/'))
    }

    /// Returns the ListenBrainz user name the token belongs to.
    pub async fn validate_token(&self, token: &str) -> Result<String, ServiceError> {
        let reply = self
            .transport
            .send(
                ApiRequest::get(self.url("/1/validate-token"))
                    .header("Authorization", format!("Token {}", token.trim())),
            )
            .await?;
        let value = parse_reply(reply.status, &reply.body)?;
        if value.get("valid").and_then(Value::as_bool) != Some(true) {
            return Err(ServiceError::AuthRevoked(
                "ListenBrainz did not recognise that token.".into(),
            ));
        }
        value
            .get("user_name")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| ServiceError::Unavailable("ListenBrainz sent no user name.".into()))
    }

    pub async fn submit(
        &self,
        token: &str,
        kind: ListenType,
        listens: &[Listen],
    ) -> Result<(), ServiceError> {
        let reply = self
            .transport
            .send(
                ApiRequest::post(self.url("/1/submit-listens"))
                    .header("Authorization", format!("Token {}", token.trim()))
                    .json(submission(kind, listens)),
            )
            .await?;
        parse_reply(reply.status, &reply.body).map(|_| ())
    }

    /// Listens strictly older than `max_ts` (all when `None`), newest first.
    pub async fn listens(
        &self,
        token: Option<&str>,
        user: &str,
        max_ts: Option<i64>,
        count: u32,
    ) -> Result<Vec<RemoteListen>, ServiceError> {
        let encoded = percent_encoding::utf8_percent_encode(user, super::PATH_SEGMENT);
        let mut request = ApiRequest::get(self.url(&format!("/1/user/{encoded}/listens")))
            .query("count", count.clamp(1, 1000).to_string());
        if let Some(max_ts) = max_ts {
            request = request.query("max_ts", max_ts.to_string());
        }
        if let Some(token) = token {
            request = request.header("Authorization", format!("Token {}", token.trim()));
        }
        let reply = self.transport.send(request).await?;
        Ok(parse_listens(&parse_reply(reply.status, &reply.body)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::transport::{FixtureTransport, Method};
    use crate::scrobbling::sandbox;

    fn listen() -> Listen {
        Listen {
            artist: "Ilse Marten".into(),
            track: "Lanterns".into(),
            album: Some("Glass Orchard".into()),
            album_artist: Some("Ilse Marten".into()),
            duration_seconds: Some(214),
            track_number: Some(3),
            release_mbid: Some("f1418001-7f1e-46af-bfdb-95faeded8841".into()),
            isrc: Some("GBAYE0601498".into()),
            listened_at: 1_700_000_000,
            chosen_by_user: true,
        }
    }

    #[test]
    fn playing_now_omits_the_timestamp_and_listens_carry_metadata() {
        let now = submission(ListenType::PlayingNow, &[listen()]);
        assert_eq!(now["listen_type"], "playing_now");
        assert!(now["payload"][0].get("listened_at").is_none());
        let single = submission(ListenType::Single, &[listen()]);
        let item = &single["payload"][0];
        assert_eq!(item["listened_at"], 1_700_000_000);
        assert_eq!(item["track_metadata"]["release_name"], "Glass Orchard");
        let info = &item["track_metadata"]["additional_info"];
        assert_eq!(info["duration_ms"], 214_000);
        assert_eq!(info["tracknumber"], 3);
        assert_eq!(info["media_player"], "Mujin Music");
        assert_eq!(info["isrc"], "GBAYE0601498");
        // Same album artist as track artist is not repeated.
        assert!(info.get("album_artist").is_none());
    }

    #[test]
    fn recorded_errors_map_to_reactions() {
        // Recorded from api.listenbrainz.org with an unknown token.
        assert!(matches!(
            parse_reply(401, br#"{"code":401,"error":"Invalid authorization token."}"#),
            Err(ServiceError::AuthRevoked(message)) if message == "Invalid authorization token."
        ));
        assert!(matches!(
            parse_reply(404, br#"{"code":404,"error":"Cannot find user: nobody"}"#),
            Err(ServiceError::Rejected(_))
        ));
        assert!(matches!(
            parse_reply(429, b""),
            Err(ServiceError::RateLimited(_))
        ));
        assert!(matches!(
            parse_reply(503, b"down"),
            Err(ServiceError::Unavailable(_))
        ));
        assert!(plausible_token("00000000-0000-0000-0000-000000000000"));
        assert!(!plausible_token("not a token"));
    }

    #[actix_web::test]
    async fn token_validation_submission_and_history_against_recorded_replies() {
        let transport = FixtureTransport::from_json(&[sandbox::LISTENBRAINZ_FIXTURES]).unwrap();
        let client = Client {
            transport: &transport,
            base: API_URL,
        };
        let token = "6c4b6d2e-6f4c-4d2b-9a43-0f6f7a2b9c11";
        assert_eq!(
            client.validate_token(token).await.expect("valid"),
            "parson-listener"
        );
        client
            .submit(token, ListenType::Import, &[listen(), listen()])
            .await
            .expect("submitted");
        let listens = client
            .listens(Some(token), "parson-listener", None, 1000)
            .await
            .expect("history");
        assert!(!listens.is_empty());
        assert_eq!(listens[0].artist, "Ilse Marten");

        let requests = transport.requests();
        assert_eq!(
            requests[0].header_value("authorization"),
            Some(&*format!("Token {token}"))
        );
        assert_eq!(requests[1].method, Method::Post);
        assert_eq!(requests[1].json.as_ref().unwrap()["listen_type"], "import");
        assert_eq!(requests[2].query_value("count"), Some("1000"));
    }

    #[actix_web::test]
    async fn an_invalid_token_is_reported() {
        let transport = FixtureTransport::from_json(&[r#"[{
            "method": "GET",
            "url": "https://api.listenbrainz.org/1/validate-token",
            "replies": [{"status": 200, "body": {"code":200,"message":"Token invalid.","valid":false}}]
        }]"#])
        .unwrap();
        let client = Client {
            transport: &transport,
            base: API_URL,
        };
        assert!(matches!(
            client
                .validate_token("00000000-0000-0000-0000-000000000000")
                .await,
            Err(ServiceError::AuthRevoked(_))
        ));
    }
}
