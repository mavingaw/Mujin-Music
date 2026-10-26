//! The Parson key service: sign-in with only a subscription.
//!
//! TIDAL and Qobuz license API access to an application, not to each
//! listener. Whoever distributes Parson and holds that licence (a TIDAL
//! client registered for Parson, a Qobuz partner app ID and secret) runs a
//! key service. Every Parson server pointed at it lets listeners sign in with
//! only their subscription: a TIDAL login on tidal.com, or a Qobuz email and
//! password. Nobody registers anything.
//!
//! Protocol (version 1), all under the service's base URL:
//! - `GET /v1/apps` publishes the public half: TIDAL's OAuth client id (a
//!   public PKCE client, no secret), the redirect URI registered with TIDAL
//!   (the service's relay, below), whether TIDAL allows that client device
//!   sign-in, and the Qobuz app id.
//! - `POST /v1/qobuz/sign` returns the `request_sig` for one
//!   `track/getFileUrl` call, so the Qobuz app secret never leaves the
//!   service. It sees the track id, format and time only: never the
//!   listener's token or password, which go to Qobuz directly. A signature
//!   alone plays nothing; Qobuz still checks the listener's own session.
//! - `GET /v1/tidal/callback` is the redirect URI registered with TIDAL. It
//!   forwards TIDAL's code to the Parson server named in `state`. A code is
//!   useless without its PKCE verifier, which never leaves that server.
//!
//! The client half (used by every Parson server) is [`KeyServiceManifest`],
//! [`relay_state`] and the Qobuz client's signing call. The service half is
//! [`ServiceKeys`], [`sign_file_url`] and [`relay_location`], served by
//! `api::key_service` when a server holds keys, and by [`InProcessKeyService`]
//! in the sandbox and tests.
//!
//! Neither service has issued Parson such a licence, so this repository's
//! builds name no key service. A distribution that holds one sets
//! `PARSON_KEY_SERVICE_URL` at build time (or a server sets it at run time).
//! Parson never takes keys from TIDAL's or Qobuz's own apps.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};

use super::hub::TIDAL_CALLBACK_PATH;
use super::qobuz::file_url_signature;
use super::transport::{ApiReply, ApiRequest, Method, Transport, TransportError, form_encode};

pub const PROTOCOL_VERSION: u32 = 1;
/// How far a signing request's timestamp may be from the service's clock.
pub const SIGN_CLOCK_SKEW_SECONDS: i64 = 300;
/// The key service the sandbox simulates in-process.
pub const SANDBOX_KEY_SERVICE_URL: &str = "https://keys.sandbox.parson.invalid";
const MAX_URL_CHARACTERS: usize = 512;
/// Separates the random part of a relayed TIDAL `state` from the return
/// address. Not in the base64url alphabet, so the split is unambiguous.
const RELAY_STATE_SEPARATOR: char = '.';

/// What a key service publishes at `GET /v1/apps`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyServiceManifest {
    pub protocol: u32,
    /// Who holds the licence, shown to listeners ("Signs in through ...").
    pub issuer: String,
    #[serde(default)]
    pub tidal: Option<TidalApp>,
    #[serde(default)]
    pub qobuz: Option<QobuzApp>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TidalApp {
    pub client_id: String,
    /// The relay registered with TIDAL as this client's redirect URI.
    pub redirect_uri: String,
    /// Whether TIDAL lets this client use device-code sign-in.
    #[serde(default)]
    pub device_code: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QobuzApp {
    pub app_id: String,
}

/// A key service URL: `https://` (or `http://` on a loopback address, for
/// development), no query, fragment or user info. Trailing slashes are
/// dropped.
pub fn normalized_service_url(value: &str) -> Option<String> {
    let value = value.trim().trim_end_matches('/');
    if value.is_empty() || value.len() > MAX_URL_CHARACTERS {
        return None;
    }
    let rest = if let Some(rest) = value.strip_prefix("https://") {
        rest
    } else {
        let rest = value.strip_prefix("http://")?;
        let host = rest.split(['/', ':']).next().unwrap_or_default();
        if !matches!(host, "localhost" | "127.0.0.1" | "[::1]") {
            return None;
        }
        rest
    };
    let authority = rest.split('/').next().unwrap_or_default();
    let clean = !authority.is_empty()
        && !rest.contains(['?', '#', '@', '\\', ' '])
        && value.chars().all(|character| character.is_ascii_graphic());
    clean.then(|| value.to_string())
}

/// The key service this server uses: `PARSON_KEY_SERVICE_URL` at run time,
/// else the one the build was made with, else none. `off` turns it off.
pub fn configured_url() -> Option<String> {
    let runtime = std::env::var("PARSON_KEY_SERVICE_URL").ok();
    let value = runtime.or_else(|| option_env!("PARSON_KEY_SERVICE_URL").map(str::to_string))?;
    if matches!(value.trim(), "" | "off" | "none") {
        return None;
    }
    let url = normalized_service_url(&value);
    if url.is_none() {
        tracing::warn!("PARSON_KEY_SERVICE_URL is not an https:// URL; ignoring it");
    }
    url
}

/// Reads and checks a manifest reply.
pub fn parse_manifest(body: &[u8]) -> Result<KeyServiceManifest, String> {
    let manifest: KeyServiceManifest =
        serde_json::from_slice(body).map_err(|error| format!("unreadable reply: {error}"))?;
    if manifest.protocol != PROTOCOL_VERSION {
        return Err(format!(
            "it speaks protocol {}, this Parson speaks {PROTOCOL_VERSION}",
            manifest.protocol
        ));
    }
    let id_ok = |value: &str| {
        !value.is_empty()
            && value.len() <= 256
            && value.chars().all(|character| character.is_ascii_graphic())
    };
    if manifest.issuer.trim().is_empty() || manifest.issuer.len() > 80 {
        return Err("it names no issuer".into());
    }
    if let Some(tidal) = &manifest.tidal
        && (!id_ok(&tidal.client_id) || normalized_service_url(&tidal.redirect_uri).is_none())
    {
        return Err("its TIDAL client is malformed".into());
    }
    if let Some(qobuz) = &manifest.qobuz
        && !id_ok(&qobuz.app_id)
    {
        return Err("its Qobuz app is malformed".into());
    }
    Ok(manifest)
}

/// A TIDAL `state` for a sign-in that comes back through the key service's
/// relay: the random state, then where the relay should send the listener
/// (this server's own callback, already checked by the hub).
pub fn relay_state(nonce: &str, return_uri: &str) -> String {
    format!(
        "{nonce}{RELAY_STATE_SEPARATOR}{}",
        URL_SAFE_NO_PAD.encode(return_uri.as_bytes())
    )
}

/// The return address inside a relayed `state`, if it is a well-formed
/// Parson callback (`http(s)://host[:port]/settings/services/tidal-callback`).
pub fn relay_return_uri(state: &str) -> Option<String> {
    let (nonce, encoded) = state.rsplit_once(RELAY_STATE_SEPARATOR)?;
    if nonce.is_empty() || state.len() > 1024 {
        return None;
    }
    let decoded = URL_SAFE_NO_PAD.decode(encoded).ok()?;
    let uri = String::from_utf8(decoded).ok()?;
    let origin = uri.strip_suffix(TIDAL_CALLBACK_PATH)?;
    let authority = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))?;
    let well_formed = !authority.is_empty()
        && !authority.contains(['/', '?', '#', '@', '\\'])
        && !authority.starts_with(':')
        && uri.chars().all(|character| character.is_ascii_graphic());
    well_formed.then_some(uri)
}

/// The keys a key service holds. Read from the environment by a server
/// that runs one (`PARSON_KEY_SERVICE_*`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceKeys {
    pub issuer: String,
    /// The service's own public base URL, for the relay redirect URI.
    pub public_url: String,
    pub tidal_client_id: Option<String>,
    pub tidal_device_code: bool,
    pub qobuz_app_id: Option<String>,
    pub qobuz_app_secret: Option<String>,
}

impl ServiceKeys {
    /// `None` unless this server is set up to run a key service.
    pub fn from_env() -> Option<Self> {
        let read = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let public_url = normalized_service_url(&read("PARSON_KEY_SERVICE_PUBLIC_URL")?)?;
        let keys = Self {
            issuer: read("PARSON_KEY_SERVICE_ISSUER").unwrap_or_else(|| "Parson".into()),
            public_url,
            tidal_client_id: read("PARSON_KEY_SERVICE_TIDAL_CLIENT_ID"),
            tidal_device_code: read("PARSON_KEY_SERVICE_TIDAL_DEVICE_CODE")
                .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes")),
            qobuz_app_id: read("PARSON_KEY_SERVICE_QOBUZ_APP_ID"),
            qobuz_app_secret: read("PARSON_KEY_SERVICE_QOBUZ_APP_SECRET"),
        };
        (keys.tidal_client_id.is_some() || keys.qobuz_signing()).then_some(keys)
    }

    /// The simulated service the sandbox uses. Its "keys" open nothing real.
    pub fn sandbox() -> Self {
        Self {
            issuer: "Sandbox key service".into(),
            public_url: SANDBOX_KEY_SERVICE_URL.into(),
            tidal_client_id: Some("sandbox-parson-tidal-client".into()),
            tidal_device_code: true,
            qobuz_app_id: Some("100000001".into()),
            qobuz_app_secret: Some("sandbox-parson-qobuz-secret".into()),
        }
    }

    fn qobuz_signing(&self) -> bool {
        self.qobuz_app_id.is_some() && self.qobuz_app_secret.is_some()
    }

    pub fn manifest(&self) -> KeyServiceManifest {
        KeyServiceManifest {
            protocol: PROTOCOL_VERSION,
            issuer: self.issuer.clone(),
            tidal: self.tidal_client_id.as_ref().map(|client_id| TidalApp {
                client_id: client_id.clone(),
                redirect_uri: format!("{}/v1/tidal/callback", self.public_url),
                device_code: self.tidal_device_code,
            }),
            qobuz: self
                .qobuz_signing()
                .then(|| self.qobuz_app_id.clone())
                .flatten()
                .map(|app_id| QobuzApp { app_id }),
        }
    }
}

/// Why a key service refused a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceRefusal {
    /// The service holds no key for this.
    NotOffered,
    /// Malformed or out-of-policy input.
    Invalid(&'static str),
}

/// Signs one Qobuz `track/getFileUrl` request. Only streaming intents at
/// Qobuz's four format ids, for numeric track ids, within the clock skew.
pub fn sign_file_url(
    keys: &ServiceKeys,
    form: &[(String, String)],
    now: i64,
) -> Result<String, ServiceRefusal> {
    let (Some(app_id), Some(secret)) = (&keys.qobuz_app_id, &keys.qobuz_app_secret) else {
        return Err(ServiceRefusal::NotOffered);
    };
    let value = |key: &str| {
        form.iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    };
    if value("app_id") != Some(app_id.as_str()) {
        return Err(ServiceRefusal::Invalid("unknown app_id"));
    }
    if value("intent") != Some("stream") {
        return Err(ServiceRefusal::Invalid("only intent=stream is signed"));
    }
    let track_id = value("track_id")
        .filter(|id| !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit()))
        .ok_or(ServiceRefusal::Invalid(
            "track_id must be a Qobuz track number",
        ))?;
    let format_id = value("format_id")
        .and_then(|format| format.parse::<u8>().ok())
        .filter(|format| matches!(format, 5 | 6 | 7 | 27))
        .ok_or(ServiceRefusal::Invalid("format_id must be 5, 6, 7 or 27"))?;
    let timestamp = value("request_ts")
        .and_then(|ts| ts.parse::<i64>().ok())
        .filter(|ts| (ts - now).abs() <= SIGN_CLOCK_SKEW_SECONDS)
        .ok_or(ServiceRefusal::Invalid(
            "request_ts is missing or too far off",
        ))?;
    Ok(file_url_signature(format_id, track_id, timestamp, secret))
}

/// Where the TIDAL relay sends the listener: their own Parson server's
/// callback with TIDAL's `code` (or `error`) and `state`, unchanged.
pub fn relay_location(query: &[(String, String)]) -> Result<String, ServiceRefusal> {
    let value = |key: &str| {
        query
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    };
    let state = value("state").ok_or(ServiceRefusal::Invalid("state is missing"))?;
    let target =
        relay_return_uri(state).ok_or(ServiceRefusal::Invalid("state names no Parson server"))?;
    let mut parameters = vec![("state", state)];
    match (value("code"), value("error")) {
        (Some(code), _) if !code.is_empty() && code.len() <= 2048 => {
            parameters.insert(0, ("code", code))
        }
        (_, Some(error)) if !error.is_empty() && error.len() <= 200 => {
            parameters.insert(0, ("error", error))
        }
        _ => {
            return Err(ServiceRefusal::Invalid(
                "TIDAL sent neither a code nor an error",
            ));
        }
    }
    let query = parameters
        .into_iter()
        .map(|(key, value)| format!("{key}={}", form_encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    Ok(format!("{target}?{query}"))
}

/// Answers key service requests in-process and passes everything else on.
/// The sandbox uses it so its listeners see the subscriber-only sign-in, and
/// tests use it to run the real client against the real service code.
pub struct InProcessKeyService<T> {
    keys: ServiceKeys,
    inner: T,
}

impl<T> InProcessKeyService<T> {
    pub fn new(keys: ServiceKeys, inner: T) -> Self {
        Self { keys, inner }
    }

    pub fn inner(&self) -> &T {
        &self.inner
    }
}

fn json_reply(status: u16, value: serde_json::Value) -> ApiReply {
    ApiReply {
        status,
        body: serde_json::to_vec(&value).unwrap_or_default(),
    }
}

fn refusal_reply(refusal: ServiceRefusal) -> ApiReply {
    match refusal {
        ServiceRefusal::NotOffered => json_reply(404, serde_json::json!({"error": "not_offered"})),
        ServiceRefusal::Invalid(message) => json_reply(
            400,
            serde_json::json!({"error": "invalid", "message": message}),
        ),
    }
}

impl<T: Transport> Transport for InProcessKeyService<T> {
    fn send(&self, request: ApiRequest) -> BoxFuture<'_, Result<ApiReply, TransportError>> {
        let Some(path) = request.url.strip_prefix(self.keys.public_url.as_str()) else {
            return self.inner.send(request);
        };
        let reply = match (request.method, path) {
            (Method::Get, "/v1/apps") => json_reply(200, serde_json::json!(self.keys.manifest())),
            (Method::Post, "/v1/qobuz/sign") => {
                let form = request.form.clone().unwrap_or_default();
                match sign_file_url(&self.keys, &form, super::unix_now()) {
                    Ok(signature) => json_reply(200, serde_json::json!({"request_sig": signature})),
                    Err(refusal) => refusal_reply(refusal),
                }
            }
            _ => json_reply(404, serde_json::json!({"error": "not_found"})),
        };
        Box::pin(async move { Ok(reply) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> ServiceKeys {
        ServiceKeys {
            issuer: "Parson".into(),
            public_url: "https://keys.parson.test".into(),
            tidal_client_id: Some("parson-tidal".into()),
            tidal_device_code: false,
            qobuz_app_id: Some("123456789".into()),
            qobuz_app_secret: Some("partner-secret".into()),
        }
    }

    fn form(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn service_urls_must_be_https_or_loopback() {
        assert_eq!(
            normalized_service_url("https://keys.parson.app/"),
            Some("https://keys.parson.app".into())
        );
        assert_eq!(
            normalized_service_url("http://localhost:8080"),
            Some("http://localhost:8080".into())
        );
        for bad in [
            "http://keys.parson.app",
            "https://user@keys.parson.app",
            "https://keys.parson.app?x=1",
            "ftp://keys.parson.app",
            "https://",
            "",
        ] {
            assert_eq!(normalized_service_url(bad), None, "{bad}");
        }
    }

    #[test]
    fn manifest_publishes_ids_but_never_the_secret() {
        let manifest = keys().manifest();
        let json = serde_json::to_string(&manifest).expect("json");
        assert!(!json.contains("partner-secret"));
        assert_eq!(
            manifest.tidal.as_ref().map(|app| app.redirect_uri.as_str()),
            Some("https://keys.parson.test/v1/tidal/callback")
        );
        assert_eq!(parse_manifest(json.as_bytes()), Ok(manifest));
        let mut no_secret = keys();
        no_secret.qobuz_app_secret = None;
        assert_eq!(no_secret.manifest().qobuz, None);
    }

    #[test]
    fn manifests_from_another_protocol_or_malformed_are_refused() {
        assert!(parse_manifest(br#"{"protocol":2,"issuer":"x"}"#).is_err());
        assert!(parse_manifest(br#"{"protocol":1,"issuer":""}"#).is_err());
        assert!(
            parse_manifest(
                br#"{"protocol":1,"issuer":"x","tidal":{"client_id":"a","redirect_uri":"http://evil.test/cb"}}"#
            )
            .is_err()
        );
        assert!(parse_manifest(b"not json").is_err());
    }

    #[test]
    fn signing_matches_the_local_signature_and_enforces_policy() {
        let now = 1_700_000_000;
        let good = form(&[
            ("app_id", "123456789"),
            ("track_id", "7700001"),
            ("format_id", "27"),
            ("intent", "stream"),
            ("request_ts", "1700000010"),
        ]);
        assert_eq!(
            sign_file_url(&keys(), &good, now),
            Ok(file_url_signature(
                27,
                "7700001",
                1_700_000_010,
                "partner-secret"
            ))
        );
        let with = |key: &str, value: &str| {
            let mut changed = good.clone();
            for pair in &mut changed {
                if pair.0 == key {
                    pair.1 = value.into();
                }
            }
            changed
        };
        for bad in [
            with("app_id", "999"),
            with("intent", "download"),
            with("track_id", "../etc"),
            with("format_id", "99"),
            with("request_ts", "1600000000"),
        ] {
            assert!(matches!(
                sign_file_url(&keys(), &bad, now),
                Err(ServiceRefusal::Invalid(_))
            ));
        }
        let mut tidal_only = keys();
        tidal_only.qobuz_app_secret = None;
        assert_eq!(
            sign_file_url(&tidal_only, &good, now),
            Err(ServiceRefusal::NotOffered)
        );
    }

    #[test]
    fn the_relay_returns_only_to_a_parson_callback() {
        let back = "https://music.example.com/settings/services/tidal-callback";
        let state = relay_state("abc123", back);
        assert_eq!(relay_return_uri(&state).as_deref(), Some(back));
        let location =
            relay_location(&form(&[("code", "c0de/+"), ("state", &state)])).expect("relayed");
        assert_eq!(
            location,
            format!("{back}?code=c0de%2F%2B&state={}", form_encode(&state))
        );
        let declined = relay_location(&form(&[("error", "access_denied"), ("state", &state)]))
            .expect("declined relayed");
        assert!(declined.starts_with(&format!("{back}?error=access_denied&state=")));
        for target in [
            "https://evil.test/steal",
            "https://user@evil.test/settings/services/tidal-callback",
            "https://evil.test/x/settings/services/tidal-callback",
            "javascript:alert(1)/settings/services/tidal-callback",
        ] {
            let state = relay_state("n", target);
            assert!(relay_location(&form(&[("code", "c"), ("state", &state)])).is_err());
        }
        assert!(relay_location(&form(&[("state", &state)])).is_err());
        assert!(relay_location(&form(&[("code", "c"), ("state", "no-separator")])).is_err());
    }

    #[actix_web::test]
    async fn in_process_service_answers_its_own_urls_only() {
        let inner = super::super::transport::FixtureTransport::default();
        let service = InProcessKeyService::new(keys(), inner);
        let manifest = service
            .send(ApiRequest::get("https://keys.parson.test/v1/apps"))
            .await
            .expect("manifest");
        assert_eq!(manifest.status, 200);
        assert_eq!(parse_manifest(&manifest.body), Ok(keys().manifest()));
        let refused = service
            .send(ApiRequest::post("https://keys.parson.test/v1/qobuz/sign").form("intent", "x"))
            .await
            .expect("reply");
        assert_eq!(refused.status, 400);
        assert!(
            service
                .send(ApiRequest::get("https://www.qobuz.com/api.json/0.2/x"))
                .await
                .is_err(),
            "other hosts go to the inner transport"
        );
    }
}
