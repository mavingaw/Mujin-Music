//! Client for the AI enhancer worker (`tools/enhancer/`), which runs
//! on a PC with a GPU.
//!
//! The worker restores lossy music with a generative model (Apollo by
//! default). It invents plausible treble the encoder threw away; it never
//! recovers the original master, so what it makes is labelled
//! "AI-enhanced" and never "lossless".
//!
//! Contract (every call but health carries `Authorization: Bearer
//! <shared secret>`, the secret the worker prints on first start):
//!
//! - `GET    /v1/health`                 `{"ok": true}`
//! - `GET    /v1/info`                   model, device, real-time factor
//! - `POST   /v1/jobs`                   the file's bytes (`?filename=`), or
//!   `{"path": ...}` when the worker sees the same disk
//! - `GET    /v1/jobs/{id}`              `{"state", "progress" (0..1), "message"}`
//! - `GET    /v1/jobs/{id}/result`       the 24-bit FLAC
//! - `DELETE /v1/jobs/{id}`              cancels and forgets
//! - `POST   /v1/realtime`               raw `f32le` interleaved PCM in
//!   (`X-Sample-Rate`, `X-Channels`), the same length enhanced out

use std::path::Path;
use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde::Serialize;
use serde_json::Value;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const MAX_JSON_BYTES: usize = 1024 * 1024;
/// Largest file uploaded or downloaded (a long 24-bit FLAC is big).
pub const MAX_FILE_BYTES: usize = 1024 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnhancerError {
    Unreachable(String),
    Unauthorized,
    NotFound,
    Http(u16),
    Invalid(String),
    Io(String),
}

impl std::fmt::Display for EnhancerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable(detail) => {
                write!(f, "The enhancer could not be reached ({detail}). Is it running?")
            }
            Self::Unauthorized => f.write_str(
                "The enhancer did not accept the shared secret. Copy it from the enhancer's window again.",
            ),
            Self::NotFound => f.write_str("The enhancer does not know that job."),
            Self::Http(status) => write!(f, "The enhancer answered with HTTP {status}."),
            Self::Invalid(detail) => write!(f, "The enhancer sent an unexpected answer ({detail})."),
            Self::Io(detail) => write!(f, "A file could not be read or written ({detail})."),
        }
    }
}

impl std::error::Error for EnhancerError {}

#[derive(Clone)]
pub struct EnhancerConfig {
    pub base_url: String,
    pub token: Option<String>,
}

impl std::fmt::Debug for EnhancerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnhancerConfig")
            .field("base_url", &self.base_url)
            .field("token", &self.token.as_ref().map(|_| "<saved>"))
            .finish()
    }
}

/// What the worker says about itself.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EnhancerInfo {
    pub worker_version: String,
    /// e.g. `apollo`.
    pub model: String,
    pub model_version: String,
    /// The tag written into files, e.g. `apollo-1.0`.
    pub tag: String,
    /// `cuda` or `cpu`.
    pub device: String,
    pub device_name: String,
    pub sample_rate: u32,
    /// Seconds of processing per second of audio (lower is faster).
    pub realtime_factor: Option<f64>,
    pub license: String,
}

/// Real time needs the worker at least twice as fast as playback.
pub const MAX_REALTIME_FACTOR: f64 = 0.5;

impl EnhancerInfo {
    pub fn realtime_capable(&self) -> bool {
        self.realtime_factor
            .is_some_and(|rtf| rtf.is_finite() && rtf > 0.0 && rtf < MAX_REALTIME_FACTOR)
    }

    /// `Apollo` for the signal path and labels.
    pub fn model_label(&self) -> String {
        crate::library::tools::marks::model_label(&self.model)
    }
}

pub fn parse_info(value: &Value) -> Option<EnhancerInfo> {
    let text = |key: &str| {
        value
            .get(key)
            .and_then(|value| match value {
                Value::String(text) => Some(text.clone()),
                Value::Number(number) => Some(number.to_string()),
                _ => None,
            })
            .unwrap_or_default()
    };
    let model = text("model");
    if model.is_empty() {
        return None;
    }
    let model_version = text("model_version");
    let tag = match text("tag") {
        tag if !tag.is_empty() => tag,
        _ if model_version.is_empty() => model.clone(),
        _ => format!("{model}-{model_version}"),
    };
    Some(EnhancerInfo {
        worker_version: text("worker_version"),
        model,
        model_version,
        tag,
        device: text("device"),
        device_name: text("device_name"),
        sample_rate: value
            .get("sample_rate")
            .and_then(Value::as_u64)
            .and_then(|rate| u32::try_from(rate).ok())
            .unwrap_or(44_100),
        realtime_factor: value.get("realtime_factor").and_then(Value::as_f64),
        license: text("license"),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkerJobState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
    Other(String),
}

impl WorkerJobState {
    pub fn parse(value: &str) -> Self {
        match value {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "done" => Self::Done,
            "failed" => Self::Failed,
            "cancelled" | "canceled" => Self::Cancelled,
            other => Self::Other(other.to_string()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkerJob {
    pub id: String,
    pub state: WorkerJobState,
    /// 0.0 to 1.0.
    pub progress: f64,
    pub message: String,
}

fn parse_job(value: &Value) -> Option<WorkerJob> {
    let id = match value.get("id")? {
        Value::String(id) => id.clone(),
        Value::Number(id) => id.to_string(),
        _ => return None,
    };
    Some(WorkerJob {
        id,
        state: WorkerJobState::parse(value.get("state").and_then(Value::as_str).unwrap_or("")),
        progress: value
            .get("progress")
            .and_then(Value::as_f64)
            .filter(|p| p.is_finite())
            .map_or(0.0, |p| p.clamp(0.0, 1.0)),
        message: value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    })
}

pub struct EnhancerClient {
    http: reqwest::Client,
    config: EnhancerConfig,
}

impl EnhancerClient {
    pub fn new(config: EnhancerConfig) -> Result<Self, EnhancerError> {
        // The worker is on the local network: never through a system proxy.
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("MujinMusic/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| EnhancerError::Unreachable(error.to_string()))?;
        Ok(Self { http, config })
    }

    fn request(
        &self,
        method: Method,
        path: &str,
    ) -> Result<reqwest::RequestBuilder, EnhancerError> {
        let token = self
            .config
            .token
            .as_deref()
            .ok_or(EnhancerError::Unauthorized)?;
        Ok(self
            .http
            .request(method, format!("{}{path}", self.config.base_url))
            .bearer_auth(token)
            .timeout(REQUEST_TIMEOUT))
    }

    async fn send(request: reqwest::RequestBuilder) -> Result<reqwest::Response, EnhancerError> {
        let response = request.send().await.map_err(|error| {
            EnhancerError::Unreachable(if error.is_timeout() {
                "timed out".into()
            } else if error.is_connect() {
                "connection refused or no route".into()
            } else {
                "request failed".into()
            })
        })?;
        match response.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(EnhancerError::Unauthorized),
            StatusCode::NOT_FOUND => Err(EnhancerError::NotFound),
            status if !status.is_success() => Err(EnhancerError::Http(status.as_u16())),
            _ => Ok(response),
        }
    }

    async fn json(request: reqwest::RequestBuilder) -> Result<Value, EnhancerError> {
        let bytes = Self::send(request)
            .await?
            .bytes()
            .await
            .map_err(|_| EnhancerError::Invalid("the answer was cut off".into()))?;
        if bytes.len() > MAX_JSON_BYTES {
            return Err(EnhancerError::Invalid("the answer is too large".into()));
        }
        serde_json::from_slice(&bytes).map_err(|_| EnhancerError::Invalid("not JSON".into()))
    }

    pub async fn health(&self) -> Result<(), EnhancerError> {
        Self::send(
            self.http
                .get(format!("{}/v1/health", self.config.base_url))
                .timeout(REQUEST_TIMEOUT),
        )
        .await
        .map(|_| ())
    }

    pub async fn info(&self) -> Result<EnhancerInfo, EnhancerError> {
        let value = Self::json(self.request(Method::GET, "/v1/info")?).await?;
        parse_info(&value).ok_or_else(|| EnhancerError::Invalid("no model named".into()))
    }

    /// Uploads `path` and starts a job.
    pub async fn submit_upload(&self, path: &Path) -> Result<WorkerJob, EnhancerError> {
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|error| EnhancerError::Io(error.kind().to_string()))?;
        if metadata.len() as usize > MAX_FILE_BYTES {
            return Err(EnhancerError::Io("the file is too large".into()));
        }
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|error| EnhancerError::Io(error.kind().to_string()))?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("track");
        let request = self
            .request(
                Method::POST,
                &format!(
                    "/v1/jobs?filename={}",
                    percent_encoding::utf8_percent_encode(name, percent_encoding::NON_ALPHANUMERIC)
                ),
            )?
            .timeout(TRANSFER_TIMEOUT)
            .header("content-type", "application/octet-stream")
            .body(bytes);
        let value = Self::json(request).await?;
        parse_job(&value).ok_or_else(|| EnhancerError::Invalid("no job id".into()))
    }

    /// Starts a job on a file the worker can read at `path` itself.
    pub async fn submit_path(&self, path: &str) -> Result<WorkerJob, EnhancerError> {
        let request = self
            .request(Method::POST, "/v1/jobs")?
            .json(&serde_json::json!({ "path": path }));
        let value = Self::json(request).await?;
        parse_job(&value).ok_or_else(|| EnhancerError::Invalid("no job id".into()))
    }

    pub async fn job(&self, id: &str) -> Result<WorkerJob, EnhancerError> {
        let value = Self::json(self.request(
            Method::GET,
            &format!("/v1/jobs/{}", super::fetch_segment(id)),
        )?)
        .await?;
        parse_job(&value).ok_or_else(|| EnhancerError::Invalid("no job id".into()))
    }

    pub async fn cancel(&self, id: &str) -> Result<(), EnhancerError> {
        Self::send(self.request(
            Method::DELETE,
            &format!("/v1/jobs/{}", super::fetch_segment(id)),
        )?)
        .await
        .map(|_| ())
    }

    /// Downloads a finished job's FLAC to `target`, which must not exist:
    /// it is written beside it first and moved into place, so a failed
    /// download never leaves a half file, and nothing is ever overwritten.
    pub async fn download(&self, id: &str, target: &Path) -> Result<u64, EnhancerError> {
        use tokio::io::AsyncWriteExt;
        if tokio::fs::try_exists(target).await.unwrap_or(true) {
            return Err(EnhancerError::Io("the target file already exists".into()));
        }
        let mut response = Self::send(
            self.request(
                Method::GET,
                &format!("/v1/jobs/{}/result", super::fetch_segment(id)),
            )?
            .timeout(TRANSFER_TIMEOUT),
        )
        .await?;
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| EnhancerError::Io(error.kind().to_string()))?;
        }
        let partial = target.with_extension("flac.part");
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
            .await
            .map_err(|error| EnhancerError::Io(error.kind().to_string()))?;
        let mut written = 0u64;
        let result = async {
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| EnhancerError::Invalid("the download was cut off".into()))?
            {
                written += chunk.len() as u64;
                if written as usize > MAX_FILE_BYTES {
                    return Err(EnhancerError::Invalid("the result is too large".into()));
                }
                file.write_all(&chunk)
                    .await
                    .map_err(|error| EnhancerError::Io(error.kind().to_string()))?;
            }
            file.flush()
                .await
                .map_err(|error| EnhancerError::Io(error.kind().to_string()))?;
            Ok(())
        }
        .await;
        drop(file);
        if let Err(error) = result {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(error);
        }
        if written < 4 {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(EnhancerError::Invalid("the result is empty".into()));
        }
        // Hard-link then unlink: fails instead of replacing a file that
        // appeared meanwhile.
        match tokio::fs::hard_link(&partial, target).await {
            Ok(()) => {
                let _ = tokio::fs::remove_file(&partial).await;
            }
            Err(_) if !tokio::fs::try_exists(target).await.unwrap_or(true) => {
                tokio::fs::rename(&partial, target)
                    .await
                    .map_err(|error| EnhancerError::Io(error.kind().to_string()))?;
            }
            Err(error) => {
                let _ = tokio::fs::remove_file(&partial).await;
                return Err(EnhancerError::Io(error.kind().to_string()));
            }
        }
        Ok(written)
    }

    /// Enhances one block of interleaved `f32` samples in real time.
    pub async fn realtime(
        &self,
        samples: &[f32],
        channels: u8,
        sample_rate: u32,
        timeout: Duration,
    ) -> Result<Vec<f32>, EnhancerError> {
        let body: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let request = self
            .request(Method::POST, "/v1/realtime")?
            .timeout(timeout)
            .header("content-type", "application/octet-stream")
            .header("x-sample-rate", sample_rate.to_string())
            .header("x-channels", channels.to_string())
            .body(body);
        let bytes = Self::send(request)
            .await?
            .bytes()
            .await
            .map_err(|_| EnhancerError::Invalid("the answer was cut off".into()))?;
        if bytes.len() != samples.len() * 4 {
            return Err(EnhancerError::Invalid(
                "the enhanced block has a different length".into(),
            ));
        }
        Ok(bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| f32::from_le_bytes(*chunk))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upgrade::fake::FakeEnhancer;

    #[test]
    fn info_parses_tolerantly_and_gates_real_time() {
        let info = parse_info(&serde_json::json!({
            "model": "apollo", "model_version": "1.0", "device": "cuda",
            "realtime_factor": 0.2, "unknown": [1, 2]
        }))
        .unwrap();
        assert_eq!(info.tag, "apollo-1.0");
        assert_eq!(info.model_label(), "Apollo");
        assert!(info.realtime_capable());
        let slow =
            parse_info(&serde_json::json!({"model": "apollo", "realtime_factor": 0.8})).unwrap();
        assert!(!slow.realtime_capable(), "needs RTF < 0.5");
        let unknown = parse_info(&serde_json::json!({"model": "apollo"})).unwrap();
        assert!(!unknown.realtime_capable());
        assert!(parse_info(&serde_json::json!({"device": "cpu"})).is_none());
    }

    #[actix_web::test]
    async fn uploads_paths_downloads_and_real_time_against_a_fake_worker() {
        let fake = FakeEnhancer::start("shared-secret", 0.2, b"fLaC-fake-result".to_vec()).await;
        let client = EnhancerClient::new(EnhancerConfig {
            base_url: fake.url(),
            token: Some("shared-secret".into()),
        })
        .unwrap();
        client.health().await.unwrap();
        let info = client.info().await.unwrap();
        assert_eq!(info.device_name, "Fake GPU");
        assert!(info.realtime_capable());

        let dir = std::env::temp_dir().join(format!("mujin-enhancer-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("a.mp3");
        std::fs::write(&source, vec![7u8; 1234]).unwrap();
        let job = client.submit_upload(&source).await.unwrap();
        assert_eq!(fake.uploads(), vec![1234]);
        assert_eq!(
            client.job(&job.id).await.unwrap().state,
            WorkerJobState::Done
        );
        let target = dir.join("out").join("a.flac");
        assert_eq!(client.download(&job.id, &target).await.unwrap(), 16);
        assert_eq!(std::fs::read(&target).unwrap(), b"fLaC-fake-result");
        // Never overwrites.
        assert!(matches!(
            client.download(&job.id, &target).await,
            Err(EnhancerError::Io(_))
        ));
        assert_eq!(
            std::fs::read(&source).unwrap(),
            vec![7u8; 1234],
            "source untouched"
        );

        let by_path = client.submit_path("/srv/music/a.mp3").await.unwrap();
        assert_eq!(fake.paths(), vec!["/srv/music/a.mp3".to_string()]);
        client.cancel(&by_path.id).await.unwrap();
        assert_eq!(client.job(&by_path.id).await, Err(EnhancerError::NotFound));

        let block = vec![0.5f32; 4410 * 2];
        let enhanced = client
            .realtime(&block, 2, 44_100, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(enhanced.len(), block.len());
        assert_eq!(enhanced[0], 0.25);

        let wrong = EnhancerClient::new(EnhancerConfig {
            base_url: fake.url(),
            token: Some("nope".into()),
        })
        .unwrap();
        assert_eq!(wrong.info().await, Err(EnhancerError::Unauthorized));
        let _ = std::fs::remove_dir_all(&dir);
        fake.stop().await;
    }
}
