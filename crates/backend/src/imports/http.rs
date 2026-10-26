//! A bounded HTTP client shared by the service importers.

use std::time::Duration;

use reqwest::{Client, RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug)]
pub enum ProviderError {
    /// The service rejected the credentials (expired or revoked).
    Unauthorized,
    /// The service says the resource does not exist or is private.
    NotFound,
    /// The service is throttling; retry later.
    RateLimited,
    /// Spotify Connect needs Premium, or another permission is missing.
    Forbidden(String),
    /// The person's input or stored configuration is unusable.
    Invalid(String),
    /// Network or unexpected response.
    Upstream(String),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::Unauthorized => {
                formatter.write_str("The service rejected the saved sign-in. Connect it again.")
            }
            ProviderError::NotFound => {
                formatter.write_str("The service could not find that playlist, or it is private.")
            }
            ProviderError::RateLimited => {
                formatter.write_str("The service is rate limiting requests. Try again shortly.")
            }
            ProviderError::Forbidden(message)
            | ProviderError::Invalid(message)
            | ProviderError::Upstream(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for ProviderError {}

impl From<reqwest::Error> for ProviderError {
    fn from(error: reqwest::Error) -> Self {
        if error.is_timeout() {
            ProviderError::Upstream("The service did not answer in time.".to_string())
        } else {
            ProviderError::Upstream(format!("Could not reach the service: {error}"))
        }
    }
}

pub fn client() -> Result<Client, ProviderError> {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .user_agent(concat!("Parson/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(ProviderError::from)
}

pub fn status_error(status: StatusCode, body: &str) -> ProviderError {
    match status {
        StatusCode::UNAUTHORIZED => ProviderError::Unauthorized,
        StatusCode::NOT_FOUND => ProviderError::NotFound,
        StatusCode::TOO_MANY_REQUESTS => ProviderError::RateLimited,
        StatusCode::FORBIDDEN => ProviderError::Forbidden(
            if body.contains("PREMIUM_REQUIRED") || body.to_lowercase().contains("premium") {
                "This needs Spotify Premium.".to_string()
            } else if body.contains("quotaExceeded") {
                "The YouTube API key has used up its daily quota.".to_string()
            } else {
                "The service refused access to that resource.".to_string()
            },
        ),
        status if status.is_client_error() => {
            ProviderError::Invalid(format!("The service rejected the request ({status})."))
        }
        status => ProviderError::Upstream(format!("The service failed ({status}).")),
    }
}

/// Whether `url` may carry credentials: HTTPS, or plain HTTP to this
/// machine only (local test services). Anything else would send tokens,
/// codes and keys in clear text over the network.
pub fn secure_url(url: &str) -> Result<reqwest::Url, ProviderError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|error| ProviderError::Invalid(format!("Invalid service address: {error}")))?;
    let loopback = parsed.host_str().is_some_and(|host| {
        let host = host.trim_start_matches('[').trim_end_matches(']');
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    });
    match parsed.scheme() {
        "https" => Ok(parsed),
        "http" if loopback => Ok(parsed),
        _ => Err(ProviderError::Invalid(
            "Service addresses must use HTTPS so sign-in tokens never travel in clear text."
                .to_string(),
        )),
    }
}

/// Sends a request and reads a bounded body. Every request goes to an
/// address [`secure_url`] accepts.
pub async fn send_bytes(request: RequestBuilder) -> Result<(StatusCode, Vec<u8>), ProviderError> {
    let (client, request) = request.build_split();
    let request = request?;
    secure_url(request.url().as_str())?;
    let mut response = client.execute(request).await?;
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(ProviderError::Upstream(
            "The service response was too large.".to_string(),
        ));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(ProviderError::Upstream(
                "The service response was too large.".to_string(),
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok((status, body))
}

/// Sends a request and decodes a bounded JSON body, mapping HTTP errors.
pub async fn send_json<T: DeserializeOwned>(request: RequestBuilder) -> Result<T, ProviderError> {
    let (status, body) = send_bytes(request).await?;
    if !status.is_success() {
        return Err(status_error(status, &String::from_utf8_lossy(&body)));
    }
    serde_json::from_slice(&body)
        .map_err(|error| ProviderError::Upstream(format!("Unexpected service response: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_only_travel_over_https_or_to_this_machine() {
        for allowed in [
            "https://accounts.spotify.com/api/token",
            "http://127.0.0.1:41234/api/token",
            "http://localhost:8080/v1",
            "http://[::1]:9000/api",
        ] {
            assert!(secure_url(allowed).is_ok(), "{allowed}");
        }
        for refused in [
            "http://accounts.spotify.com/api/token",
            "http://192.168.1.20/api/token",
            "ftp://example.com/token",
            "not a url",
        ] {
            assert!(secure_url(refused).is_err(), "{refused}");
        }
    }

    #[tokio::test]
    async fn requests_to_clear_text_addresses_are_never_sent() {
        let client = client().expect("client");
        let error = send_bytes(
            client
                .post("http://accounts.example.com/api/token")
                .body("refresh_token=secret"),
        )
        .await
        .expect_err("refused before sending");
        assert!(error.to_string().contains("HTTPS"), "{error}");
    }

    #[test]
    fn premium_and_quota_errors_are_explained() {
        assert_eq!(
            status_error(
                StatusCode::FORBIDDEN,
                r#"{"error":{"reason":"PREMIUM_REQUIRED"}}"#
            )
            .to_string(),
            "This needs Spotify Premium."
        );
        assert!(
            status_error(StatusCode::FORBIDDEN, "quotaExceeded")
                .to_string()
                .contains("quota")
        );
        assert!(matches!(
            status_error(StatusCode::UNAUTHORIZED, ""),
            ProviderError::Unauthorized
        ));
        assert!(matches!(
            status_error(StatusCode::BAD_GATEWAY, ""),
            ProviderError::Upstream(_)
        ));
    }
}
