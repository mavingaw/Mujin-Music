//! `/outputs/convolution`: FIR filters (room correction and the like) the
//! server's DSP chain convolves with. Every zone's DSP chain names one by
//! id; the web app's own chain fetches the WAV to run it in the browser.
//!
//! - `GET    /outputs/convolution` lists the filters with their summaries.
//! - `GET    /outputs/convolution/{id}/file` returns one as WAV.
//! - `POST   /outputs/admin/convolution?name=` uploads a WAV impulse
//!   response (admin).
//! - `DELETE /outputs/admin/convolution/{id}` removes one (admin).
//!
//! The server's audio devices, bridges, AirPlay receivers and UPnP players
//! are zones (`/zones`); their device setup and DSP chain are zone settings.

use std::sync::Arc;

use actix_web::{HttpResponse, delete, get, post, web};
use actix_web_httpauth::middleware::HttpAuthentication;
use futures::StreamExt;
use serde::Deserialize;

use crate::api::auth::admin_guard;
use crate::api::error::{bad_request, internal_server_error, not_found};
use crate::outputs::pipeline::{FilterStore, MAX_FILTER_BYTES};

/// The filters: the app's own store, else the server's.
fn filters(data: Option<web::Data<FilterStore>>) -> Arc<FilterStore> {
    data.map(|data| data.into_inner())
        .unwrap_or_else(|| Arc::clone(&crate::outputs::pipeline::global().filters))
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, HttpResponse> {
    web::block(work)
        .await
        .map_err(|_| internal_server_error("The filter task stopped.", "convolution_task_failed"))
}

#[get("/convolution")]
async fn list_filters(data: Option<web::Data<FilterStore>>) -> HttpResponse {
    let filters = filters(data);
    match blocking(move || filters.list()).await {
        Ok(filters) => HttpResponse::Ok().json(filters),
        Err(response) => response,
    }
}

#[get("/convolution/{id}/file")]
async fn filter_file(
    path: web::Path<String>,
    data: Option<web::Data<FilterStore>>,
) -> HttpResponse {
    let id = path.into_inner();
    let filters = filters(data);
    match blocking(move || filters.wav(&id)).await {
        Ok(Some(bytes)) => HttpResponse::Ok()
            .content_type("audio/wav")
            .insert_header(("Cache-Control", "private, max-age=86400"))
            .body(bytes),
        Ok(None) => not_found("That filter is not on the server.", "filter_not_found"),
        Err(response) => response,
    }
}

#[derive(Deserialize)]
struct UploadFilterQuery {
    #[serde(default)]
    name: String,
    #[serde(default)]
    file_name: Option<String>,
}

#[post("/convolution")]
async fn upload_filter(
    query: web::Query<UploadFilterQuery>,
    mut payload: web::Payload,
    data: Option<web::Data<FilterStore>>,
) -> HttpResponse {
    let mut bytes = Vec::new();
    while let Some(chunk) = payload.next().await {
        let Ok(chunk) = chunk else {
            return bad_request("The upload was interrupted.", "filter_upload_failed");
        };
        if bytes.len() + chunk.len() > MAX_FILTER_BYTES {
            return HttpResponse::PayloadTooLarge().json(serde_json::json!({
                "error": "filter_too_large",
                "message": format!("A filter file can be at most {} MB.", MAX_FILTER_BYTES / (1024 * 1024)),
            }));
        }
        bytes.extend_from_slice(&chunk);
    }
    let query = query.into_inner();
    let filters = filters(data);
    match blocking(move || filters.add(&query.name, query.file_name.as_deref(), &bytes)).await {
        Ok(Ok(view)) => HttpResponse::Ok().json(view),
        Ok(Err(message)) => bad_request(message, "invalid_filter"),
        Err(response) => response,
    }
}

#[delete("/convolution/{id}")]
async fn delete_filter(
    path: web::Path<String>,
    data: Option<web::Data<FilterStore>>,
) -> HttpResponse {
    let id = path.into_inner();
    let filters = filters(data);
    match blocking(move || filters.delete(&id)).await {
        Ok(Ok(())) => HttpResponse::NoContent().finish(),
        Ok(Err(message)) => not_found(message, "filter_not_found"),
        Err(response) => response,
    }
}

/// Mounted under the authenticated music scope.
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/outputs")
            .service(
                web::scope("/admin")
                    .wrap(HttpAuthentication::with_fn(admin_guard))
                    .service(upload_filter)
                    .service(delete_filter),
            )
            .service(list_filters)
            .service(filter_file),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{App, HttpMessage, test};

    fn claims(role: &str) -> crate::api::auth::Claims {
        crate::api::auth::Claims {
            sub: "3".into(),
            exp: usize::MAX,
            username: "listener".into(),
            bitrate: 0,
            token_type: "access".into(),
            role: role.into(),
            token_version: 0,
            session_id: None,
        }
    }

    #[actix_web::test]
    async fn convolution_filters_are_listed_served_and_uploaded_by_admins() {
        let dir = std::env::temp_dir().join(format!("parson-routes-{}", uuid::Uuid::new_v4()));
        let store = FilterStore::new(Some(dir.join("convolution")));
        let wav = parson_output::convolution::write_wav_f64(48_000, &[vec![0.0, 1.0, 0.0]]);
        let filter = store.add("Room", None, &wav).unwrap();
        let store = web::Data::new(store);
        let app = test::init_service(
            App::new()
                .app_data(store.clone())
                .wrap_fn(|request, service| {
                    request.extensions_mut().insert(claims("user"));
                    actix_web::dev::Service::call(service, request)
                })
                .configure(configure),
        )
        .await;
        let listed: serde_json::Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::get()
                .uri("/outputs/convolution")
                .to_request(),
        )
        .await;
        assert_eq!(listed[0]["name"], "Room");
        assert_eq!(listed[0]["summary"]["taps"], 3);
        let file = test::call_service(
            &app,
            test::TestRequest::get()
                .uri(&format!("/outputs/convolution/{}/file", filter.record.id))
                .to_request(),
        )
        .await;
        assert_eq!(file.status(), 200);
        assert_eq!(file.headers().get("content-type").unwrap(), "audio/wav");
        assert_eq!(test::read_body(file).await, wav);
        for uri in [
            "/outputs/convolution/../../etc/file",
            "/outputs/convolution/abcdef/file",
        ] {
            let response =
                test::call_service(&app, test::TestRequest::get().uri(uri).to_request()).await;
            assert_eq!(response.status(), 404, "{uri}");
        }
        // Uploads are for administrators.
        let upload = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/outputs/admin/convolution?name=x")
                .set_payload(wav.clone())
                .to_request(),
        )
        .await;
        assert_eq!(upload.status(), 401);
        let _ = std::fs::remove_dir_all(dir);
    }
}
