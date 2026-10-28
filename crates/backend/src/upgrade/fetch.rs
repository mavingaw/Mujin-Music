//! Client for FETCH, the download manager on the local network.
//!
//! Only the Mujin Music server talks to FETCH (never a browser). The
//! contract, as FETCH documents it at `/api.md` and `/openapi.json`:
//!
//! - `GET  /api/health` (no auth), `GET /api/me`
//! - `POST /api/fetch` `{"text": "..."}` → `{"jobs": [...]}`
//! - `GET  /api/jobs?limit=200` (all jobs and free disk space),
//!   `GET /api/jobs/{id}`, `POST /api/jobs/{id}/cancel`,
//!   `POST /api/jobs/{id}/retry`, `DELETE /api/jobs/{id}`
//! - `GET  /api/browse/search?cat=music&q=...`
//!
//! Every `/api/*` call carries `Authorization: Bearer <agent token>`, plus
//! Cloudflare Access service-token headers when they are configured. The
//! client is tolerant: unknown fields are ignored, unknown statuses are
//! kept as FETCH wrote them, and numbers may arrive as strings. The token
//! never appears in an error, a log line or a response.

use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde::Serialize;
use serde_json::Value;

/// Paths Mujin Music uses, as `/openapi.json` would list them.
pub const USED_PATHS: &[&str] = &[
    "/api/health",
    "/api/me",
    "/api/fetch",
    "/api/jobs",
    "/api/jobs/{}",
    "/api/jobs/{}/cancel",
    "/api/jobs/{}/retry",
    "/api/browse/search",
];
const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchError {
    /// FETCH could not be reached (wrong address, PC asleep, firewall).
    Unreachable(String),
    /// The agent token is missing or FETCH rejected it.
    Unauthorized,
    /// Cloudflare Access turned the request away.
    AccessDenied,
    NotFound,
    Http(u16),
    /// FETCH answered with something that is not the documented JSON.
    Invalid(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable(detail) => write!(f, "FETCH could not be reached ({detail})."),
            Self::Unauthorized => f.write_str(
                "FETCH did not accept the agent token. Ask FETCH for a token for Mujin Music and save it again.",
            ),
            Self::AccessDenied => f.write_str(
                "Cloudflare Access refused the request. Check the service token, or use FETCH's LAN address.",
            ),
            Self::NotFound => f.write_str("FETCH does not know that job or page."),
            Self::Http(status) => write!(f, "FETCH answered with HTTP {status}."),
            Self::Invalid(detail) => write!(f, "FETCH sent an unexpected answer ({detail})."),
        }
    }
}

impl std::error::Error for FetchError {}

/// How to reach FETCH. `Debug` never prints the secrets.
#[derive(Clone)]
pub struct FetchConfig {
    pub base_url: String,
    pub token: Option<String>,
    /// Cloudflare Access service token (client id, client secret).
    pub access: Option<(String, String)>,
}

impl std::fmt::Debug for FetchConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FetchConfig")
            .field("base_url", &self.base_url)
            .field("token", &self.token.as_ref().map(|_| "<saved>"))
            .field("access", &self.access.as_ref().map(|_| "<saved>"))
            .finish()
    }
}

/// FETCH job status. Unknown values are kept as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Starting,
    Running,
    Done,
    Failed,
    Cancelled,
    /// Another app (Lidarr and the like) took the job over.
    HandedOff,
    Other(String),
}

impl JobStatus {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "queued" | "pending" => Self::Queued,
            "starting" => Self::Starting,
            "running" | "downloading" => Self::Running,
            "done" | "completed" | "complete" => Self::Done,
            "failed" | "error" => Self::Failed,
            "cancelled" | "canceled" => Self::Cancelled,
            "handed_off" | "handed-off" | "handedoff" => Self::HandedOff,
            _ => Self::Other(value.trim().to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Queued => "queued",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::HandedOff => "handed_off",
            Self::Other(value) => value,
        }
    }

    /// Queued, starting, running or handed off.
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Starting | Self::Running | Self::HandedOff
        )
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}

/// One FETCH job. Missing fields read as empty.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FetchJob {
    pub id: String,
    pub created: String,
    pub updated: String,
    pub input: String,
    pub engine: String,
    pub status: String,
    /// 0 to 100.
    pub progress: Option<f64>,
    pub speed: String,
    pub eta: String,
    pub title: String,
    pub dest: String,
    pub error: String,
    pub note: String,
    pub size: String,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub plan: Value,
}

impl FetchJob {
    pub fn status(&self) -> JobStatus {
        JobStatus::parse(&self.status)
    }

    /// Finished and moved out of FETCH's staging area: the album is in
    /// its final folder.
    pub fn is_final(&self) -> bool {
        self.status() == JobStatus::Done && !self.dest.trim().starts_with("/staging/")
    }
}

/// Text of a JSON value that may be a string or a number.
fn text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(flag)) => flag.to_string(),
        _ => String::new(),
    }
}

/// Progress in percent from `45`, `45.5`, `"45%"` or `"45"`.
fn percent(value: Option<&Value>) -> Option<f64> {
    let number = match value? {
        Value::Number(number) => number.as_f64()?,
        Value::String(text) => text.trim().trim_end_matches('%').trim().parse().ok()?,
        _ => return None,
    };
    number.is_finite().then(|| number.clamp(0.0, 100.0))
}

/// Reads one job object; `None` without an id.
pub fn parse_job(value: &Value) -> Option<FetchJob> {
    let object = value.as_object()?;
    let id = text(object.get("id"));
    if id.is_empty() {
        return None;
    }
    let status = text(object.get("status"));
    let mut progress = percent(object.get("progress"));
    if JobStatus::parse(&status) == JobStatus::Done && progress.is_none() {
        progress = Some(100.0);
    }
    Some(FetchJob {
        id,
        created: text(object.get("created")),
        updated: text(object.get("updated")),
        input: text(object.get("input")),
        engine: text(object.get("engine")),
        status: if status.is_empty() {
            "queued".into()
        } else {
            status
        },
        progress,
        speed: text(object.get("speed")),
        eta: text(object.get("eta")),
        title: text(object.get("title")),
        dest: text(object.get("dest")),
        error: text(object.get("error")),
        note: text(object.get("note")),
        size: text(object.get("size")),
        plan: object.get("plan").cloned().unwrap_or(Value::Null),
    })
}

/// Jobs from `{"jobs": [...]}`, a bare array, or `{"job": {...}}`.
pub fn parse_jobs(value: &Value) -> Vec<FetchJob> {
    let list = match value {
        Value::Array(items) => items.as_slice(),
        Value::Object(object) => match object.get("jobs") {
            Some(Value::Array(items)) => items.as_slice(),
            _ => {
                return object
                    .get("job")
                    .and_then(parse_job)
                    .or_else(|| parse_job(value))
                    .into_iter()
                    .collect();
            }
        },
        _ => &[],
    };
    list.iter().filter_map(parse_job).collect()
}

/// Free disk space FETCH reports next to the job list, if it does.
pub fn parse_free_space(value: &Value) -> Option<String> {
    for pointer in [
        "/free",
        "/free_bytes",
        "/free_space",
        "/disk_free",
        "/disk/free",
        "/disk/free_bytes",
        "/space/free",
    ] {
        match value.pointer(pointer) {
            Some(Value::Number(number)) => {
                return number
                    .as_u64()
                    .map(format_bytes)
                    .or(Some(number.to_string()));
            }
            Some(Value::String(text)) if !text.is_empty() => return Some(text.clone()),
            _ => {}
        }
    }
    None
}

fn format_bytes(bytes: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < units.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", units[unit])
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct JobList {
    pub jobs: Vec<FetchJob>,
    pub free_space: Option<String>,
}

/// What `/api/browse/search` found, read tolerantly.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SearchOutcome {
    /// `None` when the answer had no recognisable result list.
    pub found: Option<bool>,
    pub count: usize,
    /// A few result titles, for the tooltip.
    pub titles: Vec<String>,
}

/// Finds result lists wherever FETCH put them: a bare array, `results`,
/// `items`, `hits`, or per-source lists under `sources`.
pub fn parse_search(value: &Value) -> SearchOutcome {
    fn collect<'a>(value: &'a Value, out: &mut Vec<&'a Value>, depth: usize) -> bool {
        if depth > 3 {
            return false;
        }
        match value {
            Value::Array(items) => {
                // An array of sources, each with its own results.
                let mut nested = false;
                for item in items {
                    if let Some(object) = item.as_object()
                        && ["results", "items", "hits"]
                            .iter()
                            .any(|key| object.get(*key).is_some_and(Value::is_array))
                    {
                        nested |= collect(item, out, depth + 1);
                    }
                }
                if !nested {
                    out.extend(items.iter());
                }
                true
            }
            Value::Object(object) => {
                let mut seen = false;
                for key in ["results", "items", "hits", "sources", "data"] {
                    if let Some(child) = object.get(key) {
                        seen |= collect(child, out, depth + 1);
                    }
                }
                seen
            }
            _ => false,
        }
    }
    let mut results = Vec::new();
    if !collect(value, &mut results, 0) {
        return SearchOutcome {
            found: None,
            count: 0,
            titles: Vec::new(),
        };
    }
    let titles = results
        .iter()
        .filter_map(|item| {
            let object = item.as_object()?;
            ["title", "name", "release", "album"]
                .iter()
                .find_map(|key| object.get(*key).and_then(Value::as_str))
                .map(str::to_string)
        })
        .take(5)
        .collect();
    SearchOutcome {
        found: Some(!results.is_empty()),
        count: results.len(),
        titles,
    }
}

/// What "Check connection" learned from FETCH's own description.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OpenApiReport {
    pub title: Option<String>,
    pub version: Option<String>,
    pub path_count: usize,
    /// Paths Mujin Music uses that FETCH does not list.
    pub missing: Vec<String>,
}

/// `/api/jobs/{job_id}` and `/api/jobs/{id}` compare equal.
fn normalize_template(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut in_param = false;
    for character in path.trim_end_matches('/').chars() {
        match character {
            '{' => {
                in_param = true;
                out.push('{');
            }
            '}' => {
                in_param = false;
                out.push('}');
            }
            _ if in_param => {}
            other => out.push(other),
        }
    }
    out
}

pub fn read_openapi(value: &Value) -> OpenApiReport {
    let paths: Vec<String> = value
        .get("paths")
        .and_then(Value::as_object)
        .map(|paths| paths.keys().map(|path| normalize_template(path)).collect())
        .unwrap_or_default();
    let missing = USED_PATHS
        .iter()
        .filter(|used| !paths.iter().any(|path| path == *used))
        .map(|used| used.replace("{}", "{id}"))
        .collect();
    OpenApiReport {
        title: value
            .pointer("/info/title")
            .and_then(Value::as_str)
            .map(str::to_string),
        version: value
            .pointer("/info/version")
            .and_then(Value::as_str)
            .map(str::to_string),
        path_count: paths.len(),
        missing,
    }
}

use super::fetch_segment as segment;

pub struct FetchClient {
    http: reqwest::Client,
    config: FetchConfig,
}

impl FetchClient {
    pub fn new(config: FetchConfig) -> Result<Self, FetchError> {
        // FETCH is on the local network: never through a system proxy.
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("MujinMusic/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| FetchError::Unreachable(error.to_string()))?;
        Ok(Self { http, config })
    }

    pub fn base_url(&self) -> &str {
        &self.config.base_url
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        authorized: bool,
    ) -> Result<Value, FetchError> {
        let url = format!("{}{path}", self.config.base_url);
        let mut request = self.http.request(method, &url);
        if authorized {
            let token = self
                .config
                .token
                .as_deref()
                .ok_or(FetchError::Unauthorized)?;
            request = request.bearer_auth(token);
        }
        if let Some((id, secret)) = &self.config.access {
            request = request
                .header("CF-Access-Client-Id", id)
                .header("CF-Access-Client-Secret", secret);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(|error| {
            FetchError::Unreachable(if error.is_timeout() {
                "timed out".into()
            } else if error.is_connect() {
                "connection refused or no route".into()
            } else {
                "request failed".into()
            })
        })?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED {
            return Err(FetchError::Unauthorized);
        }
        if status == StatusCode::FORBIDDEN || status.is_redirection() {
            // Cloudflare Access answers with a redirect to its login page
            // or a 403; FETCH itself answers 401 for a bad token.
            return Err(if self.config.access.is_some() || status.is_redirection() {
                FetchError::AccessDenied
            } else {
                FetchError::Unauthorized
            });
        }
        if status == StatusCode::NOT_FOUND {
            return Err(FetchError::NotFound);
        }
        if !status.is_success() {
            return Err(FetchError::Http(status.as_u16()));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|_| FetchError::Invalid("the answer was cut off".into()))?;
        if bytes.len() > MAX_BODY_BYTES {
            return Err(FetchError::Invalid("the answer is too large".into()));
        }
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&bytes).map_err(|_| FetchError::Invalid("not JSON".into()))
    }

    /// `GET /api/health`, without the token.
    pub async fn health(&self) -> Result<Value, FetchError> {
        self.call(Method::GET, "/api/health", None, false).await
    }

    /// `GET /api/me`: proves the token works.
    pub async fn me(&self) -> Result<Value, FetchError> {
        self.call(Method::GET, "/api/me", None, true).await
    }

    /// `GET /openapi.json`.
    pub async fn openapi(&self) -> Result<OpenApiReport, FetchError> {
        let value = self.call(Method::GET, "/openapi.json", None, false).await?;
        let report = read_openapi(&value);
        if !report.missing.is_empty() {
            tracing::warn!(
                missing = ?report.missing,
                version = ?report.version,
                "FETCH's OpenAPI description lacks paths Mujin Music uses; requests may fail"
            );
        }
        Ok(report)
    }

    /// `POST /api/fetch`: asks FETCH for something in plain words.
    pub async fn submit(&self, text: &str) -> Result<Vec<FetchJob>, FetchError> {
        let value = self
            .call(
                Method::POST,
                "/api/fetch",
                Some(serde_json::json!({ "text": text })),
                true,
            )
            .await?;
        Ok(parse_jobs(&value))
    }

    /// `GET /api/jobs?limit=`.
    pub async fn jobs(&self, limit: u32) -> Result<JobList, FetchError> {
        let value = self
            .call(
                Method::GET,
                &format!("/api/jobs?limit={}", limit.clamp(1, 1000)),
                None,
                true,
            )
            .await?;
        Ok(JobList {
            jobs: parse_jobs(&value),
            free_space: parse_free_space(&value),
        })
    }

    /// `GET /api/jobs/{id}`.
    pub async fn job(&self, id: &str) -> Result<FetchJob, FetchError> {
        let value = self
            .call(
                Method::GET,
                &format!("/api/jobs/{}", segment(id)),
                None,
                true,
            )
            .await?;
        parse_jobs(&value)
            .into_iter()
            .next()
            .ok_or_else(|| FetchError::Invalid("no job in the answer".into()))
    }

    pub async fn cancel(&self, id: &str) -> Result<(), FetchError> {
        self.call(
            Method::POST,
            &format!("/api/jobs/{}/cancel", segment(id)),
            None,
            true,
        )
        .await
        .map(|_| ())
    }

    pub async fn retry(&self, id: &str) -> Result<(), FetchError> {
        self.call(
            Method::POST,
            &format!("/api/jobs/{}/retry", segment(id)),
            None,
            true,
        )
        .await
        .map(|_| ())
    }

    pub async fn delete(&self, id: &str) -> Result<(), FetchError> {
        self.call(
            Method::DELETE,
            &format!("/api/jobs/{}", segment(id)),
            None,
            true,
        )
        .await
        .map(|_| ())
    }

    /// `GET /api/browse/search?cat=music&q=`: does any source have it?
    pub async fn search_music(&self, query: &str) -> Result<SearchOutcome, FetchError> {
        let path = format!(
            "/api/browse/search?cat=music&q={}",
            percent_encoding::utf8_percent_encode(query, percent_encoding::NON_ALPHANUMERIC)
        );
        let value = self.call(Method::GET, &path, None, true).await?;
        Ok(parse_search(&value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upgrade::fake::FakeFetch;

    #[test]
    fn jobs_parse_tolerantly() {
        let value = serde_json::json!({
            "jobs": [
                {"id": 7, "status": "running", "progress": "45%", "speed": "2.1 MB/s",
                 "eta": "3m", "title": "Endgame", "dest": "/staging/x", "extra": {"a": 1}},
                {"id": "b2", "status": "teleporting", "progress": 12.5},
                {"status": "running"},
                {"id": "c3", "status": "done", "dest": "Music/Rise Against/Endgame"}
            ],
            "disk": {"free": 1_500_000_000u64}
        });
        let jobs = parse_jobs(&value);
        assert_eq!(jobs.len(), 3, "the job without an id is dropped");
        assert_eq!(jobs[0].id, "7");
        assert_eq!(jobs[0].progress, Some(45.0));
        assert_eq!(jobs[0].status(), JobStatus::Running);
        assert!(!jobs[0].is_final());
        assert_eq!(
            jobs[1].status(),
            JobStatus::Other("teleporting".into()),
            "unknown statuses are kept as written"
        );
        assert_eq!(jobs[1].status().as_str(), "teleporting");
        assert!(jobs[2].is_final());
        assert_eq!(jobs[2].progress, Some(100.0));
        assert_eq!(parse_free_space(&value).as_deref(), Some("1.5 GB"));
        assert!(JobStatus::HandedOff.is_active());
        assert!(!JobStatus::HandedOff.is_terminal());
        assert!(JobStatus::parse("canceled").is_terminal());
        // A single job, bare.
        let single = parse_jobs(&serde_json::json!({"id": "x", "status": "queued"}));
        assert_eq!(single.len(), 1);
    }

    #[test]
    fn done_in_staging_is_not_final_yet() {
        let mut job =
            parse_job(&serde_json::json!({"id": 1, "status": "done", "dest": "/staging/abc"}))
                .unwrap();
        assert!(!job.is_final());
        job.dest = "/music/Rise Against/Endgame".into();
        assert!(job.is_final());
    }

    #[test]
    fn search_results_are_found_wherever_they_are() {
        assert_eq!(
            parse_search(&serde_json::json!({"results": [{"title": "Endgame"}]})).found,
            Some(true)
        );
        let nested = parse_search(&serde_json::json!({"sources": [
            {"source": "a", "results": []},
            {"source": "b", "results": [{"name": "Endgame (FLAC)"}, {"name": "Endgame"}]}
        ]}));
        assert_eq!(nested.found, Some(true));
        assert_eq!(nested.count, 2);
        assert_eq!(nested.titles[0], "Endgame (FLAC)");
        assert_eq!(parse_search(&serde_json::json!([])).found, Some(false));
        assert_eq!(parse_search(&serde_json::json!({"ok": true})).found, None);
    }

    #[test]
    fn openapi_reports_version_and_missing_paths() {
        let value = serde_json::json!({
            "info": {"title": "FETCH", "version": "2.4.0"},
            "paths": {
                "/api/health": {}, "/api/me": {}, "/api/fetch": {}, "/api/jobs": {},
                "/api/jobs/{job_id}": {}, "/api/jobs/{job_id}/cancel": {},
                "/api/browse/search": {}
            }
        });
        let report = read_openapi(&value);
        assert_eq!(report.version.as_deref(), Some("2.4.0"));
        assert_eq!(report.path_count, 7);
        assert_eq!(report.missing, vec!["/api/jobs/{id}/retry".to_string()]);
    }

    #[test]
    fn config_debug_never_prints_secrets() {
        let config = FetchConfig {
            base_url: "http://fetch".into(),
            token: Some("very-secret-token".into()),
            access: Some(("id".into(), "cf-secret".into())),
        };
        let printed = format!("{config:?}");
        assert!(!printed.contains("very-secret-token"));
        assert!(!printed.contains("cf-secret"));
        let error = FetchError::Unauthorized.to_string();
        assert!(!error.contains("very-secret-token"));
    }

    #[actix_web::test]
    async fn the_client_follows_the_contract_against_a_fake_fetch() {
        let fake = FakeFetch::start("agent-token").await;
        let client = FetchClient::new(FetchConfig {
            base_url: fake.url(),
            token: Some("agent-token".into()),
            access: Some(("cf-id".into(), "cf-secret".into())),
        })
        .unwrap();
        assert_eq!(client.health().await.unwrap()["ok"], true);
        assert_eq!(client.me().await.unwrap()["name"], "Mujin Music");
        let report = client.openapi().await.unwrap();
        assert_eq!(report.version.as_deref(), Some("9.9.9-fake"));
        assert!(report.missing.is_empty(), "{:?}", report.missing);

        let jobs = client
            .submit("the album Endgame by Rise Against as flac")
            .await
            .unwrap();
        assert_eq!(jobs.len(), 1);
        let id = jobs[0].id.clone();
        assert_eq!(
            fake.submitted(),
            vec!["the album Endgame by Rise Against as flac"]
        );
        assert!(fake.saw_access_headers(), "service-token headers are sent");

        fake.set_job(&id, "running", 45.0, "/staging/1");
        let job = client.job(&id).await.unwrap();
        assert_eq!(job.status(), JobStatus::Running);
        assert_eq!(job.progress, Some(45.0));
        let list = client.jobs(200).await.unwrap();
        assert_eq!(list.jobs.len(), 1);
        assert!(list.free_space.is_some());

        client.cancel(&id).await.unwrap();
        assert_eq!(
            client.job(&id).await.unwrap().status(),
            JobStatus::Cancelled
        );
        client.retry(&id).await.unwrap();
        assert_eq!(client.job(&id).await.unwrap().status(), JobStatus::Queued);
        let found = client.search_music("Rise Against Endgame").await.unwrap();
        assert_eq!(found.found, Some(true));
        let missing = client.search_music("nothing here").await.unwrap();
        assert_eq!(missing.found, Some(false));
        client.delete(&id).await.unwrap();
        assert_eq!(client.job(&id).await, Err(FetchError::NotFound));

        // A wrong token is refused, and the error does not echo it.
        let wrong = FetchClient::new(FetchConfig {
            base_url: fake.url(),
            token: Some("wrong-token".into()),
            access: None,
        })
        .unwrap();
        let error = wrong.me().await.unwrap_err();
        assert_eq!(error, FetchError::Unauthorized);
        assert!(!error.to_string().contains("wrong-token"));
        // Health needs no token.
        assert!(wrong.health().await.is_ok());
        fake.stop().await;

        let gone = FetchClient::new(FetchConfig {
            base_url: "http://127.0.0.1:9".into(),
            token: Some("agent-token".into()),
            access: None,
        })
        .unwrap();
        assert!(matches!(
            gone.health().await,
            Err(FetchError::Unreachable(_))
        ));
    }
}
