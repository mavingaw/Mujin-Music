//! Settings → Connections → Away from home (administrators only).

use actix_web::{HttpResponse, web};

use crate::api::error::{bad_request, service_unavailable};
use crate::remote_access::{self, RemoteAccessConfig};

fn unavailable() -> HttpResponse {
    service_unavailable(
        "Remote access is not running on this server.",
        "remote_access_unavailable",
    )
}

async fn get_status() -> HttpResponse {
    match remote_access::status().await {
        Some(status) => HttpResponse::Ok()
            .insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
            .json(status),
        None => unavailable(),
    }
}

async fn put_config(config: web::Json<RemoteAccessConfig>) -> HttpResponse {
    match remote_access::update(config.into_inner()).await {
        Ok(status) => HttpResponse::Ok().json(status),
        Err(message) => bad_request(message, "invalid_remote_access"),
    }
}

async fn run_check() -> HttpResponse {
    match remote_access::check().await {
        Some(report) => HttpResponse::Ok().json(report),
        None => unavailable(),
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.route("", web::get().to(get_status))
        .route("", web::put().to(put_config))
        .route("/check", web::post().to(run_check));
}
