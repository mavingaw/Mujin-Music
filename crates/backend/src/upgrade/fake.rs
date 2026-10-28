//! Fake FETCH and fake enhancer worker for tests: small HTTP servers on
//! loopback that implement exactly the documented contracts, so the real
//! clients (reqwest, auth headers, JSON shapes) are exercised end to end.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use actix_web::{App, HttpRequest, HttpResponse, HttpServer, web};
use serde_json::{Value, json};

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn bearer(request: &HttpRequest) -> Option<String> {
    request
        .headers()
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::to_string)
}

struct Running {
    handle: actix_web::dev::ServerHandle,
    url: String,
}

async fn serve(
    state: Arc<dyn Fn(HttpRequest, web::Bytes) -> HttpResponse + Send + Sync>,
) -> Running {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind");
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let server = HttpServer::new(move || {
        let state = state.clone();
        App::new()
            .app_data(web::PayloadConfig::new(64 * 1024 * 1024))
            .default_service(web::to(move |request: HttpRequest, body: web::Bytes| {
                let state = state.clone();
                async move { state(request, body) }
            }))
    })
    .workers(1)
    .listen(listener)
    .expect("listen")
    .run();
    let handle = server.handle();
    actix_web::rt::spawn(server);
    Running { handle, url }
}

/* ------------------------------------------------------------------ */
/* FETCH                                                               */
/* ------------------------------------------------------------------ */

#[derive(Default)]
struct FetchState {
    jobs: BTreeMap<String, Value>,
    submitted: Vec<String>,
    access_seen: bool,
    next_id: u64,
    /// Answer every `/api/*` call with this status (outage tests).
    failing: Option<u16>,
    polls: u64,
}

pub struct FakeFetch {
    running: Running,
    state: Arc<Mutex<FetchState>>,
}

impl FakeFetch {
    pub async fn start(token: &str) -> Self {
        let state = Arc::new(Mutex::new(FetchState::default()));
        let token = token.to_string();
        let shared = state.clone();
        let running = serve(Arc::new(move |request: HttpRequest, body: web::Bytes| {
            let mut state = lock(&shared);
            let path = request.path().to_string();
            let method = request.method().as_str().to_string();
            if path == "/api/health" {
                return HttpResponse::Ok().json(json!({"ok": true, "version": "9.9.9-fake"}));
            }
            if path == "/openapi.json" {
                return HttpResponse::Ok().json(json!({
                    "openapi": "3.1.0",
                    "info": {"title": "FETCH", "version": "9.9.9-fake"},
                    "paths": {
                        "/api/health": {}, "/api/me": {}, "/api/login": {}, "/api/fetch": {},
                        "/api/jobs": {}, "/api/jobs/{job_id}": {},
                        "/api/jobs/{job_id}/cancel": {}, "/api/jobs/{job_id}/retry": {},
                        "/api/browse/search": {}
                    }
                }));
            }
            if !path.starts_with("/api/") {
                return HttpResponse::NotFound().finish();
            }
            if bearer(&request).as_deref() != Some(token.as_str()) {
                return HttpResponse::Unauthorized().json(json!({"error": "bad token"}));
            }
            if request.headers().contains_key("cf-access-client-id")
                && request.headers().contains_key("cf-access-client-secret")
            {
                state.access_seen = true;
            }
            if let Some(status) = state.failing {
                return HttpResponse::build(
                    actix_web::http::StatusCode::from_u16(status).unwrap(),
                )
                .finish();
            }
            let parts: Vec<&str> = path.trim_start_matches("/api/").split('/').collect();
            match (method.as_str(), parts.as_slice()) {
                ("GET", ["me"]) => HttpResponse::Ok().json(json!({"name": "Mujin Music", "kind": "agent"})),
                ("POST", ["fetch"]) => {
                    let text = serde_json::from_slice::<Value>(&body)
                        .ok()
                        .and_then(|value| value.get("text")?.as_str().map(str::to_string))
                        .unwrap_or_default();
                    state.submitted.push(text.clone());
                    if text.contains("nothing") {
                        return HttpResponse::Ok().json(json!({"jobs": []}));
                    }
                    state.next_id += 1;
                    let id = format!("job-{}", state.next_id);
                    let job = json!({
                        "id": id, "created": "2026-10-27T10:00:00Z", "updated": "2026-10-27T10:00:00Z",
                        "input": text, "engine": "soulseek", "status": "queued", "progress": 0,
                        "speed": "", "eta": "", "title": text, "dest": "", "error": "",
                        "note": "", "size": "", "plan": {"kind": "album"}, "unknown_field": 1
                    });
                    state.jobs.insert(id, job.clone());
                    HttpResponse::Ok().json(json!({"jobs": [job]}))
                }
                ("GET", ["jobs"]) => {
                    state.polls += 1;
                    HttpResponse::Ok().json(json!({
                        "jobs": state.jobs.values().cloned().collect::<Vec<_>>(),
                        "disk": {"free": 812_000_000_000u64}
                    }))
                }
                ("GET", ["jobs", id]) => match state.jobs.get(*id) {
                    Some(job) => HttpResponse::Ok().json(job),
                    None => HttpResponse::NotFound().finish(),
                },
                ("DELETE", ["jobs", id]) => match state.jobs.remove(*id) {
                    Some(_) => HttpResponse::NoContent().finish(),
                    None => HttpResponse::NotFound().finish(),
                },
                ("POST", ["jobs", id, action]) => {
                    let Some(job) = state.jobs.get_mut(*id) else {
                        return HttpResponse::NotFound().finish();
                    };
                    job["status"] = json!(match *action {
                        "cancel" => "cancelled",
                        "retry" => "queued",
                        _ => return HttpResponse::NotFound().finish(),
                    });
                    HttpResponse::Ok().json(job.clone())
                }
                ("GET", ["browse", "search"]) => {
                    let query = request.query_string().to_string();
                    if query.contains("nothing") {
                        HttpResponse::Ok().json(json!({"results": []}))
                    } else {
                        HttpResponse::Ok().json(json!({"sources": [
                            {"source": "soulseek", "results": [{"title": "Endgame (FLAC)", "format": "flac"}]}
                        ]}))
                    }
                }
                _ => HttpResponse::NotFound().finish(),
            }
        }))
        .await;
        Self { running, state }
    }

    pub fn url(&self) -> String {
        self.running.url.clone()
    }

    pub fn submitted(&self) -> Vec<String> {
        lock(&self.state).submitted.clone()
    }

    pub fn saw_access_headers(&self) -> bool {
        lock(&self.state).access_seen
    }

    pub fn polls(&self) -> u64 {
        lock(&self.state).polls
    }

    pub fn set_failing(&self, status: Option<u16>) {
        lock(&self.state).failing = status;
    }

    pub fn set_job(&self, id: &str, status: &str, progress: f64, dest: &str) {
        let mut state = lock(&self.state);
        if let Some(job) = state.jobs.get_mut(id) {
            job["status"] = json!(status);
            job["progress"] = json!(progress);
            job["dest"] = json!(dest);
            job["speed"] = json!("3.2 MB/s");
            job["eta"] = json!("1m 10s");
        }
    }

    pub fn set_job_field(&self, id: &str, field: &str, value: Value) {
        if let Some(job) = lock(&self.state).jobs.get_mut(id) {
            job[field] = value;
        }
    }

    pub async fn stop(self) {
        self.running.handle.stop(false).await;
    }
}

/* ------------------------------------------------------------------ */
/* Enhancer worker                                                     */
/* ------------------------------------------------------------------ */

#[derive(Default)]
struct EnhancerState {
    jobs: BTreeMap<String, (String, Vec<u8>)>,
    uploads: Vec<usize>,
    paths: Vec<String>,
    realtime_calls: u64,
}

pub struct FakeEnhancer {
    running: Running,
    state: Arc<Mutex<EnhancerState>>,
    slow: Arc<AtomicBool>,
}

static NEXT_ENHANCE: AtomicU64 = AtomicU64::new(1);

impl FakeEnhancer {
    /// `rtf` is the real-time factor the worker reports; `result` the
    /// bytes every finished job returns (a FLAC in the tests).
    pub async fn start(secret: &str, rtf: f64, result: Vec<u8>) -> Self {
        let state = Arc::new(Mutex::new(EnhancerState::default()));
        let slow = Arc::new(AtomicBool::new(false));
        let secret = secret.to_string();
        let shared = state.clone();
        let slow_flag = slow.clone();
        let result = Arc::new(result);
        let running = serve(Arc::new(move |request: HttpRequest, body: web::Bytes| {
            let path = request.path().to_string();
            let method = request.method().as_str().to_string();
            if path == "/v1/health" {
                return HttpResponse::Ok().json(json!({"ok": true}));
            }
            if bearer(&request).as_deref() != Some(secret.as_str()) {
                return HttpResponse::Unauthorized().json(json!({"error": "bad secret"}));
            }
            let mut state = lock(&shared);
            let parts: Vec<&str> = path.trim_start_matches("/v1/").split('/').collect();
            match (method.as_str(), parts.as_slice()) {
                ("GET", ["info"]) => HttpResponse::Ok().json(json!({
                    "worker_version": "0.1.0", "model": "apollo", "model_version": "1.0",
                    "tag": "apollo-1.0", "device": "cuda", "device_name": "Fake GPU",
                    "sample_rate": 44100, "realtime_factor": rtf,
                    "license": "CC BY-SA 4.0", "shared_roots": ["/srv/music"], "extra": true
                })),
                ("POST", ["jobs"]) => {
                    let id = format!("e{}", NEXT_ENHANCE.fetch_add(1, Ordering::Relaxed));
                    let is_json = request
                        .headers()
                        .get("content-type")
                        .and_then(|value| value.to_str().ok())
                        .is_some_and(|value| value.starts_with("application/json"));
                    if is_json {
                        let path = serde_json::from_slice::<Value>(&body)
                            .ok()
                            .and_then(|value| value.get("path")?.as_str().map(str::to_string))
                            .unwrap_or_default();
                        // Like the worker: only files under its shared roots.
                        if !path.starts_with("/srv/music/") {
                            return HttpResponse::Forbidden()
                                .json(json!({"detail": "path outside the shared folders"}));
                        }
                        state.paths.push(path);
                    } else {
                        state.uploads.push(body.len());
                    }
                    state
                        .jobs
                        .insert(id.clone(), ("done".to_string(), result.as_ref().clone()));
                    HttpResponse::Accepted().json(json!({"id": id, "state": "queued"}))
                }
                ("GET", ["jobs", id]) => match state.jobs.get(*id) {
                    Some((job_state, _)) => HttpResponse::Ok().json(json!({
                        "id": id, "state": job_state, "progress": 1.0, "message": ""
                    })),
                    None => HttpResponse::NotFound().finish(),
                },
                ("GET", ["jobs", id, "result"]) => match state.jobs.get(*id) {
                    Some((_, bytes)) => HttpResponse::Ok()
                        .content_type("audio/flac")
                        .body(bytes.clone()),
                    None => HttpResponse::NotFound().finish(),
                },
                ("DELETE", ["jobs", id]) => {
                    state.jobs.remove(*id);
                    HttpResponse::NoContent().finish()
                }
                ("POST", ["realtime"]) => {
                    state.realtime_calls += 1;
                    drop(state);
                    if slow_flag.load(Ordering::Relaxed) {
                        std::thread::sleep(std::time::Duration::from_millis(400));
                    }
                    // "Enhance": a tiny, recognisable gain, same length.
                    let samples: Vec<u8> = body
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .flat_map(|chunk| (f32::from_le_bytes(*chunk) * 0.5).to_le_bytes())
                        .collect();
                    HttpResponse::Ok()
                        .content_type("application/octet-stream")
                        .body(samples)
                }
                _ => HttpResponse::NotFound().finish(),
            }
        }))
        .await;
        Self {
            running,
            state,
            slow,
        }
    }

    pub fn url(&self) -> String {
        self.running.url.clone()
    }

    pub fn uploads(&self) -> Vec<usize> {
        lock(&self.state).uploads.clone()
    }

    pub fn paths(&self) -> Vec<String> {
        lock(&self.state).paths.clone()
    }

    pub fn realtime_calls(&self) -> u64 {
        lock(&self.state).realtime_calls
    }

    /// Makes every real-time chunk take 400 ms.
    pub fn set_slow(&self, slow: bool) {
        self.slow.store(slow, Ordering::Relaxed);
    }

    pub async fn stop(self) {
        self.running.handle.stop(false).await;
    }
}
