//! Better versions, mounted at `/upgrades`.
//!
//! Everyone signed in (status only):
//! - `GET  /upgrades/albums/{id}`        a request's progress for one album
//! - `GET  /upgrades/active`             requests on their way
//!
//! Admins only (`/upgrades/admin`):
//! - `GET|PUT  /settings`                FETCH and enhancer settings
//! - `PUT|DELETE /secrets/{name}`        `{value}`; never returned
//! - `POST /fetch/check`, `POST /enhancer/check`
//! - `GET  /candidates`                  upgrade candidates with requests
//! - `POST /candidates/{id}/search`      does FETCH find it?
//! - `POST /requests` `{album_ids}`; `POST /requests/{id}/cancel|retry`;
//!   `DELETE /requests/{id}`
//! - `POST /lossless-check` `{album_ids?}` the fake-lossless check (job)
//! - `GET  /enhance`; `POST /enhance` `{album_ids, song_ids}`;
//!   `POST /enhance/unfound`; `POST /enhance/cancel`
//! - `GET  /realtime`                    real-time enhancement status

use std::path::Path;
use std::sync::Arc;

use actix_web::{HttpRequest, HttpResponse, delete, get, post, put, web};
use serde::{Deserialize, Serialize};

use crate::api::auth::authenticated_user_id;
use crate::api::error::{bad_request, conflict, internal_server_error, json_error, not_found};
use crate::library::state::LibraryLifecycle;
use crate::library::tools::jobs::{self, JobHandle, JobKind};
use crate::library::tools::spectral;
use crate::persistence::connection::DbPool;
use crate::upgrade::UpgradeService;
use crate::upgrade::service::ServiceError;
use crate::upgrade::settings::{SecretName, UpgradeSettings};
use crate::upgrade::store;

const MAX_IDS: usize = 500;

fn respond(error: ServiceError) -> HttpResponse {
    match error {
        ServiceError::NotConfigured(message) => conflict(message, "upgrade_not_configured"),
        ServiceError::Invalid(message) => bad_request(message, "upgrade_invalid"),
        ServiceError::Fetch(error) => json_error(
            actix_web::http::StatusCode::BAD_GATEWAY,
            "bad_gateway",
            error.to_string(),
            "fetch_unavailable",
        ),
        ServiceError::Enhancer(error) => json_error(
            actix_web::http::StatusCode::BAD_GATEWAY,
            "bad_gateway",
            error.to_string(),
            "enhancer_unavailable",
        ),
        ServiceError::NotFound => not_found("Nothing like that exists.", "upgrade_not_found"),
        ServiceError::Storage(error) => {
            tracing::error!(%error, "Better versions storage failed");
            internal_server_error(
                "The Better versions database could not be read.",
                "upgrade_storage_failed",
            )
        }
    }
}

fn valid_ids(ids: &[String]) -> bool {
    !ids.is_empty()
        && ids.len() <= MAX_IDS
        && ids.iter().all(|id| !id.is_empty() && id.len() <= 256)
}

// ---------------------------------------------------------------------------
// Status, for everyone
// ---------------------------------------------------------------------------

#[get("/albums/{id}")]
async fn album_status(id: web::Path<String>, service: web::Data<UpgradeService>) -> HttpResponse {
    let id = id.into_inner();
    if id.is_empty() || id.len() > 256 {
        return bad_request("Unknown album.", "invalid_album");
    }
    match service.album_status(id).await {
        Ok(status) => HttpResponse::Ok().json(status),
        Err(error) => respond(error),
    }
}

#[get("/active")]
async fn active_requests(service: web::Data<UpgradeService>) -> HttpResponse {
    match service.requests(200).await {
        Ok(requests) => HttpResponse::Ok().json(serde_json::json!({
            "requests": requests
                .into_iter()
                .map(|request| request.request)
                .filter(|request| request.state.is_open())
                .collect::<Vec<_>>()
        })),
        Err(error) => respond(error),
    }
}

// ---------------------------------------------------------------------------
// Settings and secrets
// ---------------------------------------------------------------------------

#[get("/settings")]
async fn get_settings(service: web::Data<UpgradeService>) -> HttpResponse {
    match service.settings_view().await {
        Ok(view) => HttpResponse::Ok().json(view),
        Err(error) => respond(error),
    }
}

#[put("/settings")]
async fn put_settings(
    body: web::Json<UpgradeSettings>,
    service: web::Data<UpgradeService>,
) -> HttpResponse {
    match service.save_settings(body.into_inner()).await {
        Ok(_) => match service.settings_view().await {
            Ok(view) => HttpResponse::Ok().json(view),
            Err(error) => respond(error),
        },
        Err(error) => respond(error),
    }
}

#[derive(Deserialize)]
struct SecretBody {
    value: String,
}

fn secret_name(value: &str) -> Option<SecretName> {
    SecretName::ALL
        .into_iter()
        .find(|name| name.as_str() == value)
}

#[put("/secrets/{name}")]
async fn put_secret(
    name: web::Path<String>,
    body: web::Json<SecretBody>,
    request: HttpRequest,
    service: web::Data<UpgradeService>,
) -> HttpResponse {
    let Some(name) = secret_name(&name) else {
        return not_found("Unknown secret.", "unknown_secret");
    };
    let admin = match authenticated_user_id(&request) {
        Ok(admin) => admin,
        Err(response) => return response,
    };
    match service.set_secret(name, &body.value, admin).await {
        // The value is never echoed back: only that it is saved.
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "saved": true })),
        Err(error) => respond(error),
    }
}

#[delete("/secrets/{name}")]
async fn delete_secret(
    name: web::Path<String>,
    service: web::Data<UpgradeService>,
) -> HttpResponse {
    let Some(name) = secret_name(&name) else {
        return not_found("Unknown secret.", "unknown_secret");
    };
    match service.clear_secret(name).await {
        Ok(()) => HttpResponse::NoContent().finish(),
        Err(error) => respond(error),
    }
}

#[post("/fetch/check")]
async fn check_fetch(service: web::Data<UpgradeService>) -> HttpResponse {
    match service.check_fetch().await {
        Ok(check) => HttpResponse::Ok().json(check),
        Err(error) => respond(error),
    }
}

#[post("/enhancer/check")]
async fn check_enhancer(service: web::Data<UpgradeService>) -> HttpResponse {
    match service.check_enhancer().await {
        Ok(check) => HttpResponse::Ok().json(check),
        Err(error) => respond(error),
    }
}

// ---------------------------------------------------------------------------
// Candidates and requests
// ---------------------------------------------------------------------------

#[get("/candidates")]
async fn candidates(service: web::Data<UpgradeService>) -> HttpResponse {
    let candidates = match service.candidates().await {
        Ok(candidates) => candidates,
        Err(error) => return respond(error),
    };
    let requests = match service.requests(100).await {
        Ok(requests) => requests,
        Err(error) => return respond(error),
    };
    let settings = match service.settings().await {
        Ok(settings) => settings,
        Err(error) => return respond(error),
    };
    let check_job = jobs::list()
        .into_iter()
        .find(|job| job.kind == JobKind::LosslessCheck);
    HttpResponse::Ok().json(serde_json::json!({
        "candidates": candidates,
        "requests": requests,
        "fetch_enabled": settings.fetch.enabled,
        "enhancer_enabled": settings.enhancer.enabled,
        "auto_request": settings.fetch.auto_request,
        "lossless_check": check_job,
    }))
}

#[post("/candidates/{id}/search")]
async fn search_candidate(
    id: web::Path<String>,
    service: web::Data<UpgradeService>,
) -> HttpResponse {
    match service.search(&id).await {
        Ok(outcome) => HttpResponse::Ok().json(outcome),
        Err(error) => respond(error),
    }
}

#[derive(Deserialize)]
struct AlbumIdsBody {
    album_ids: Vec<String>,
}

#[post("/requests")]
async fn create_requests(
    body: web::Json<AlbumIdsBody>,
    request: HttpRequest,
    lifecycle: web::Data<LibraryLifecycle>,
    service: web::Data<UpgradeService>,
) -> HttpResponse {
    if !valid_ids(&body.album_ids) {
        return bad_request("Choose between 1 and 500 albums.", "invalid_album_ids");
    }
    let admin = match authenticated_user_id(&request) {
        Ok(admin) => admin,
        Err(response) => return response,
    };
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(_) => return conflict("The library is not ready yet.", "library_not_ready"),
    };
    match service
        .request_albums(&cache.artists, &body.album_ids, Some(admin), false)
        .await
    {
        Ok(ids) => HttpResponse::Ok().json(serde_json::json!({ "requested": ids })),
        Err(error) => respond(error),
    }
}

#[post("/requests/{id}/cancel")]
async fn cancel_request(id: web::Path<i64>, service: web::Data<UpgradeService>) -> HttpResponse {
    match service.cancel_request(id.into_inner()).await {
        Ok(()) => HttpResponse::NoContent().finish(),
        Err(error) => respond(error),
    }
}

#[post("/requests/{id}/retry")]
async fn retry_request(id: web::Path<i64>, service: web::Data<UpgradeService>) -> HttpResponse {
    match service.retry_request(id.into_inner()).await {
        Ok(()) => HttpResponse::NoContent().finish(),
        Err(error) => respond(error),
    }
}

#[delete("/requests/{id}")]
async fn delete_request(id: web::Path<i64>, service: web::Data<UpgradeService>) -> HttpResponse {
    match service.delete_request(id.into_inner()).await {
        Ok(()) => HttpResponse::NoContent().finish(),
        Err(error) => respond(error),
    }
}

// ---------------------------------------------------------------------------
// The fake-lossless check
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct CheckBody {
    #[serde(default)]
    album_ids: Option<Vec<String>>,
    /// Check files again even when this version was checked.
    #[serde(default)]
    force: bool,
}

async fn run_lossless_check(
    job: JobHandle,
    pool: DbPool,
    service: Arc<UpgradeService>,
    album_ids: Option<Vec<String>>,
    force: bool,
) {
    job.set_current("Finding lossless files");
    let files = {
        let pool = pool.clone();
        web::block(move || {
            let mut connection = pool.get().map_err(|error| error.to_string())?;
            let files =
                crate::upgrade::files::lossless_files(&mut connection, album_ids.as_deref())
                    .map_err(|error| error.to_string())?;
            let checked =
                store::spectral_versions(&mut connection).map_err(|error| error.to_string())?;
            Ok::<_, String>((files, checked))
        })
        .await
    };
    let Ok(Ok((files, checked))) = files else {
        return job.fail("The library database could not be read.");
    };
    let todo: Vec<_> = files
        .into_iter()
        .filter_map(|file| {
            let version = crate::upgrade::files::file_version(Path::new(&file.path))?;
            (force || checked.get(&file.path) != Some(&version)).then_some((file, version))
        })
        .collect();
    job.set_total(todo.len() as u64);
    let mut fakes = 0usize;
    for (file, version) in todo {
        if job.is_cancelled() {
            break;
        }
        job.set_current(
            Path::new(&file.path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
        let rate = file.sample_rate.and_then(|rate| u32::try_from(rate).ok());
        match spectral::check_file(Path::new(&file.path), rate).await {
            Ok(result) => {
                if result.is_confident_transcode() {
                    fakes += 1;
                }
                let pool = pool.clone();
                let path = file.path.clone();
                let saved = web::block(move || {
                    let mut connection = pool.get().map_err(|error| error.to_string())?;
                    store::save_spectral(&mut connection, &path, &version, &result)
                        .map_err(|error| error.to_string())
                })
                .await;
                job.advance(matches!(saved, Ok(Ok(()))));
            }
            Err(_) => job.advance(false),
        }
    }
    // The catalog grades fake lossless files as the lossy copies they are.
    service.rebuild_catalog().await;
    job.finish(match fakes {
        0 => "No fake lossless files found.".to_string(),
        1 => "1 file looks like it was made from a lossy copy.".to_string(),
        count => format!("{count} files look like they were made from lossy copies."),
    });
}

#[post("/lossless-check")]
async fn lossless_check(
    body: Option<web::Json<CheckBody>>,
    pool: web::Data<DbPool>,
    service: web::Data<UpgradeService>,
) -> HttpResponse {
    let CheckBody { album_ids, force } = body.map(web::Json::into_inner).unwrap_or_default();
    if let Some(ids) = &album_ids
        && !valid_ids(ids)
    {
        return bad_request("Choose between 1 and 500 albums.", "invalid_album_ids");
    }
    let scope = match &album_ids {
        Some(ids) if ids.len() == 1 => "One album".to_string(),
        Some(ids) => format!("{} albums", ids.len()),
        None => "Whole library".to_string(),
    };
    match jobs::start(JobKind::LosslessCheck, scope) {
        Ok(job) => {
            let status = jobs::get(job.id());
            tokio::spawn(run_lossless_check(
                job,
                pool.get_ref().clone(),
                service.into_inner(),
                album_ids,
                force,
            ));
            HttpResponse::Accepted().json(status)
        }
        Err(running) => HttpResponse::Conflict().json(serde_json::json!({
            "error": "conflict",
            "code": "job_running",
            "message": "This check is already running.",
            "job": running,
        })),
    }
}

// ---------------------------------------------------------------------------
// Enhance
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct EnhanceBody {
    #[serde(default)]
    album_ids: Vec<String>,
    #[serde(default)]
    song_ids: Vec<String>,
}

#[derive(Serialize)]
struct EnhanceView {
    counts: store::EnhanceCounts,
    jobs: Vec<store::EnhanceJobRow>,
}

#[get("/enhance")]
async fn enhance_queue(service: web::Data<UpgradeService>) -> HttpResponse {
    match service
        .db(|connection| {
            Ok(EnhanceView {
                counts: store::enhance_counts(connection)?,
                jobs: store::enhance_jobs(connection, 100)?,
            })
        })
        .await
    {
        Ok(view) => HttpResponse::Ok().json(view),
        Err(error) => respond(error),
    }
}

#[post("/enhance")]
async fn enhance(body: web::Json<EnhanceBody>, service: web::Data<UpgradeService>) -> HttpResponse {
    let body = body.into_inner();
    let count = body.album_ids.len() + body.song_ids.len();
    if count == 0
        || count > MAX_IDS
        || !body
            .album_ids
            .iter()
            .chain(&body.song_ids)
            .all(|id| !id.is_empty() && id.len() <= 256)
    {
        return bad_request("Choose between 1 and 500 albums or tracks.", "invalid_ids");
    }
    match service
        .queue_enhance(&body.album_ids, &body.song_ids, "manual")
        .await
    {
        Ok(queued) => HttpResponse::Ok().json(serde_json::json!({ "queued": queued })),
        Err(error) => respond(error),
    }
}

#[post("/enhance/unfound")]
async fn enhance_unfound(service: web::Data<UpgradeService>) -> HttpResponse {
    match service.queue_enhance_unfound().await {
        Ok(queued) => HttpResponse::Ok().json(serde_json::json!({ "queued": queued })),
        Err(error) => respond(error),
    }
}

#[post("/enhance/cancel")]
async fn cancel_enhance(service: web::Data<UpgradeService>) -> HttpResponse {
    match service.cancel_enhance().await {
        Ok(cancelled) => HttpResponse::Ok().json(serde_json::json!({ "cancelled": cancelled })),
        Err(error) => respond(error),
    }
}

#[derive(Deserialize)]
struct OriginalsBody {
    originals: bool,
}

#[post("/albums/{id}/originals")]
async fn use_originals(
    id: web::Path<String>,
    body: web::Json<OriginalsBody>,
    service: web::Data<UpgradeService>,
) -> HttpResponse {
    match service.use_originals(&id, body.originals).await {
        Ok(tracks) => HttpResponse::Ok().json(serde_json::json!({ "tracks": tracks })),
        Err(error) => respond(error),
    }
}

#[get("/realtime")]
async fn realtime_status(service: web::Data<UpgradeService>) -> HttpResponse {
    let backend = crate::upgrade::realtime::backend();
    HttpResponse::Ok().json(serde_json::json!({
        "available": backend.is_some(),
        "model": backend.as_ref().map(|backend| backend.model_label.clone()),
        "realtime_factor": backend.as_ref().map(|backend| backend.realtime_factor),
        "max_realtime_factor": crate::upgrade::enhancer::MAX_REALTIME_FACTOR,
        "status": service.realtime_stats().snapshot(),
    }))
}

/// Routes for everyone signed in.
pub fn configure_status(cfg: &mut web::ServiceConfig) {
    cfg.service(album_status)
        .service(active_requests)
        .service(realtime_status);
}

/// Routes for admins.
pub fn configure_admin(cfg: &mut web::ServiceConfig) {
    cfg.service(get_settings)
        .service(put_settings)
        .service(put_secret)
        .service(delete_secret)
        .service(check_fetch)
        .service(check_enhancer)
        .service(candidates)
        .service(search_candidate)
        .service(create_requests)
        .service(cancel_request)
        .service(retry_request)
        .service(delete_request)
        .service(lossless_check)
        .service(enhance_queue)
        .service(enhance)
        .service(enhance_unfound)
        .service(cancel_enhance)
        .service(use_originals);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::auth::Claims;
    use crate::providers::vault::TokenVault;
    use actix_web::{App, HttpMessage, http::StatusCode, test as actix_test};
    use serde_json::{Value, json};

    fn claims(user_id: i32, role: &str) -> Claims {
        Claims {
            sub: user_id.to_string(),
            exp: usize::MAX,
            username: format!("user-{user_id}"),
            bitrate: 0,
            token_type: "access".into(),
            role: role.into(),
            token_version: 0,
            session_id: None,
        }
    }

    #[actix_web::test]
    async fn secrets_are_saved_but_never_returned() {
        let pool = crate::persistence::connection::migrated_memory_pool();
        let service = web::Data::new(UpgradeService::new(pool.clone(), TokenVault::new([5; 32])));
        let app = actix_test::init_service(
            App::new()
                .app_data(service.clone())
                .app_data(web::Data::new(pool))
                .service(web::scope("/upgrades/admin").configure(configure_admin))
                .service(web::scope("/upgrades").configure(configure_status)),
        )
        .await;
        let request = actix_test::TestRequest::put()
            .uri("/upgrades/admin/secrets/fetch_token")
            .set_json(json!({"value": "agent-token-from-fetch"}))
            .to_request();
        request.extensions_mut().insert(claims(1, "admin"));
        let saved: Value = actix_test::call_and_read_body_json(&app, request).await;
        assert_eq!(saved, json!({"saved": true}));

        let view: Value = actix_test::call_and_read_body_json(
            &app,
            actix_test::TestRequest::get()
                .uri("/upgrades/admin/settings")
                .to_request(),
        )
        .await;
        assert_eq!(view["saved"], json!(["fetch_token"]));
        assert!(!view.to_string().contains("agent-token-from-fetch"));
        assert_eq!(
            view["settings"]["fetch"]["base_url"],
            "http://192.168.1.58:8490"
        );

        let unknown = actix_test::call_service(
            &app,
            actix_test::TestRequest::delete()
                .uri("/upgrades/admin/secrets/password")
                .to_request(),
        )
        .await;
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
        let bad = actix_test::call_service(
            &app,
            actix_test::TestRequest::put()
                .uri("/upgrades/admin/settings")
                .set_json(json!({"fetch": {"base_url": "file:///etc"}}))
                .to_request(),
        )
        .await;
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
        let status = actix_test::call_service(
            &app,
            actix_test::TestRequest::get()
                .uri("/upgrades/albums/endgame")
                .to_request(),
        )
        .await;
        assert_eq!(status.status(), StatusCode::OK);
    }
}
