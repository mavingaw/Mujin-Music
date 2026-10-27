//! HTTP routes for per-profile scrobbling to Last.fm and ListenBrainz.

use actix_web::{HttpRequest, HttpResponse, delete, get, patch, post, put, web};
use serde::Deserialize;

use crate::api::auth::{authenticated_user_id, clear_auth_attempts, record_auth_attempt};
use crate::api::error::{
    bad_request, conflict, forbidden, internal_server_error, json_error, not_found,
    service_unavailable,
};
use crate::library::state::{LibraryLifecycle, library_unavailable_response};
use crate::persistence::connection::DbPool;
use crate::scrobbling::hub::HubError;
use crate::scrobbling::store::SettingsPatch;
use crate::scrobbling::{ScrobbleHub, Service, ServiceError, import};

const COMPLETE_ATTEMPTS_PER_MINUTE: usize = 20;

fn error_response(error: HubError) -> HttpResponse {
    let message = error.to_string();
    match error {
        HubError::NotConfigured => service_unavailable(message, "lastfm_not_configured"),
        HubError::NotConnected => not_found(message, "scrobbling_not_connected"),
        HubError::Invalid(_) => bad_request(message, "scrobbling_invalid"),
        HubError::Expired => json_error(
            actix_web::http::StatusCode::GONE,
            "gone",
            message,
            "scrobbling_authorization_expired",
        ),
        HubError::Busy => conflict(message, "scrobbling_import_running"),
        HubError::Service(ServiceError::AuthRevoked(_)) => {
            bad_request(message, "scrobbling_service_refused")
        }
        HubError::Service(_) => json_error(
            actix_web::http::StatusCode::BAD_GATEWAY,
            "bad_gateway",
            message,
            "scrobbling_service_unavailable",
        ),
        HubError::Storage(error) => {
            tracing::error!(%error, "scrobbling storage failed");
            internal_server_error(
                "Scrobbling is temporarily unavailable.",
                "scrobbling_unavailable",
            )
        }
    }
}

fn service_from_path(value: &str) -> Result<Service, HttpResponse> {
    Service::parse(value)
        .ok_or_else(|| not_found("Unknown scrobbling service.", "unknown_scrobbling_service"))
}

async fn require_admin(request: &HttpRequest, pool: &web::Data<DbPool>) -> Option<HttpResponse> {
    match crate::api::auth::request_has_current_admin(request, pool.get_ref().clone()).await {
        Ok(true) => None,
        Ok(false) => Some(forbidden(
            "Administrator access is required.",
            "admin_required",
        )),
        Err(error) => {
            tracing::error!(%error, "admin lookup failed");
            Some(internal_server_error(
                "Could not check your permissions.",
                "admin_lookup_failed",
            ))
        }
    }
}

#[get("")]
async fn get_status(hub: web::Data<ScrobbleHub>, request: HttpRequest) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    match hub.status(user_id).await {
        Ok(status) => HttpResponse::Ok().json(status),
        Err(error) => error_response(error),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LastfmServerCredentials {
    api_key: String,
    api_secret: String,
}

#[put("/lastfm/server")]
async fn put_lastfm_server(
    body: web::Json<LastfmServerCredentials>,
    hub: web::Data<ScrobbleHub>,
    pool: web::Data<DbPool>,
    request: HttpRequest,
) -> HttpResponse {
    if let Some(response) = require_admin(&request, &pool).await {
        return response;
    }
    let admin = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    match hub
        .set_lastfm_credentials(admin, &body.api_key, &body.api_secret)
        .await
    {
        Ok(setup) => HttpResponse::Ok().json(setup),
        Err(error) => error_response(error),
    }
}

#[delete("/lastfm/server")]
async fn delete_lastfm_server(
    hub: web::Data<ScrobbleHub>,
    pool: web::Data<DbPool>,
    request: HttpRequest,
) -> HttpResponse {
    if let Some(response) = require_admin(&request, &pool).await {
        return response;
    }
    match hub.clear_lastfm_credentials().await {
        Ok(setup) => HttpResponse::Ok().json(setup),
        Err(error) => error_response(error),
    }
}

#[derive(Deserialize)]
struct AuthorizeBody {
    /// The address this client reaches the server at.
    origin: String,
}

#[post("/lastfm/authorize")]
async fn authorize_lastfm(
    body: web::Json<AuthorizeBody>,
    hub: web::Data<ScrobbleHub>,
    request: HttpRequest,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    match hub.start_lastfm_authorization(user_id, &body.origin).await {
        Ok((url, expires_in)) => HttpResponse::Ok().json(serde_json::json!({
            "authorizeUrl": url,
            "expiresIn": expires_in,
        })),
        Err(error) => error_response(error),
    }
}

#[derive(Deserialize)]
pub struct CompleteBody {
    state: String,
    token: String,
}

/// Finishes Last.fm's web authentication. Not behind a session: the
/// browser Last.fm returns to may not be signed in to Parson (a phone's
/// browser, the system browser of the desktop app). The one-time `state`
/// identifies the profile that started connecting.
pub async fn complete_lastfm(
    body: web::Json<CompleteBody>,
    hub: web::Data<ScrobbleHub>,
    request: HttpRequest,
) -> HttpResponse {
    let attempt_key =
        match record_auth_attempt(&request, "lastfm-complete", COMPLETE_ATTEMPTS_PER_MINUTE) {
            Ok(key) => key,
            Err(response) => return response,
        };
    match hub
        .complete_lastfm_authorization(body.state.trim(), &body.token)
        .await
    {
        Ok((_, account)) => {
            clear_auth_attempts(&attempt_key);
            HttpResponse::Ok().json(serde_json::json!({
                "service": "lastfm",
                "accountName": account,
            }))
        }
        Err(error) => error_response(error),
    }
}

#[derive(Deserialize)]
struct ListenbrainzBody {
    token: String,
}

#[put("/listenbrainz")]
async fn connect_listenbrainz(
    body: web::Json<ListenbrainzBody>,
    hub: web::Data<ScrobbleHub>,
    request: HttpRequest,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    match hub.connect_listenbrainz(user_id, &body.token).await {
        Ok(account) => HttpResponse::Ok().json(serde_json::json!({
            "service": "listenbrainz",
            "accountName": account,
        })),
        Err(error) => error_response(error),
    }
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SettingsBody {
    enabled: Option<bool>,
    send_now_playing: Option<bool>,
    scrobble_radio: Option<bool>,
}

#[patch("/{service}")]
async fn update_settings(
    path: web::Path<String>,
    body: web::Json<SettingsBody>,
    hub: web::Data<ScrobbleHub>,
    request: HttpRequest,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let service = match service_from_path(&path) {
        Ok(service) => service,
        Err(response) => return response,
    };
    let patch = SettingsPatch {
        enabled: body.enabled,
        send_now_playing: body.send_now_playing,
        scrobble_radio: body.scrobble_radio,
    };
    if let Err(error) = hub.update_settings(user_id, service, patch).await {
        return error_response(error);
    }
    match hub.status(user_id).await {
        Ok(status) => HttpResponse::Ok().json(status),
        Err(error) => error_response(error),
    }
}

#[delete("/{service}")]
async fn disconnect(
    path: web::Path<String>,
    hub: web::Data<ScrobbleHub>,
    request: HttpRequest,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let service = match service_from_path(&path) {
        Ok(service) => service,
        Err(response) => return response,
    };
    match hub.disconnect(user_id, service).await {
        Ok(()) => HttpResponse::NoContent().finish(),
        Err(error) => error_response(error),
    }
}

#[post("/retry")]
async fn retry(hub: web::Data<ScrobbleHub>, request: HttpRequest) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    match hub.retry(user_id).await {
        Ok(count) => HttpResponse::Ok().json(serde_json::json!({ "queued": count })),
        Err(error) => error_response(error),
    }
}

#[post("/{service}/import")]
async fn start_import(
    path: web::Path<String>,
    hub: web::Data<ScrobbleHub>,
    lifecycle: web::Data<LibraryLifecycle>,
    request: HttpRequest,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let service = match service_from_path(&path) {
        Ok(service) => service,
        Err(response) => return response,
    };
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    match import::start(hub.into_inner(), user_id, service, cache).await {
        Ok(status) => HttpResponse::Accepted().json(status),
        Err(error) => error_response(error),
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/scrobbling")
            .service(get_status)
            .service(put_lastfm_server)
            .service(delete_lastfm_server)
            .service(authorize_lastfm)
            .service(connect_listenbrainz)
            .service(retry)
            .service(start_import)
            .service(update_settings)
            .service(disconnect),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::auth::Claims;
    use crate::scrobbling::hub::tests::{fixtures, hub_with, pool};
    use actix_web::{App, HttpMessage, http::StatusCode, test as actix_test};
    use serde_json::{Value, json};

    fn claims(user_id: i32) -> Claims {
        Claims {
            sub: user_id.to_string(),
            exp: usize::MAX,
            username: format!("user-{user_id}"),
            bitrate: 0,
            token_type: "access".into(),
            role: "user".into(),
            token_version: 0,
            session_id: None,
        }
    }

    macro_rules! signed_in {
        ($builder:expr, $user:expr) => {{
            let request = $builder.to_request();
            request.extensions_mut().insert(claims($user));
            request
        }};
    }

    #[actix_web::test]
    async fn a_profile_connects_both_services_through_the_api() {
        let pool = pool();
        let hub = web::Data::new(hub_with(pool.clone(), fixtures()));
        let app = actix_test::init_service(
            App::new()
                .app_data(hub.clone())
                .app_data(web::Data::new(pool.clone()))
                .route("/complete", web::post().to(complete_lastfm))
                .configure(configure),
        )
        .await;

        let authorize: Value = actix_test::call_and_read_body_json(
            &app,
            signed_in!(
                actix_test::TestRequest::post()
                    .uri("/scrobbling/lastfm/authorize")
                    .set_json(json!({"origin": "http://192.168.1.20:1993"})),
                1
            ),
        )
        .await;
        let url = authorize["authorizeUrl"].as_str().unwrap();
        let state = url.split("state%3D").nth(1).unwrap();
        // The callback page completes without a Parson session.
        let completed = actix_test::call_service(
            &app,
            actix_test::TestRequest::post()
                .uri("/complete")
                .set_json(json!({"state": state, "token": "approved-token"}))
                .to_request(),
        )
        .await;
        assert_eq!(completed.status(), StatusCode::OK);
        let reused = actix_test::call_service(
            &app,
            actix_test::TestRequest::post()
                .uri("/complete")
                .set_json(json!({"state": state, "token": "approved-token"}))
                .to_request(),
        )
        .await;
        assert_eq!(reused.status(), StatusCode::GONE);

        let connected = actix_test::call_service(
            &app,
            signed_in!(
                actix_test::TestRequest::put()
                    .uri("/scrobbling/listenbrainz")
                    .set_json(json!({"token": "6c4b6d2e-6f4c-4d2b-9a43-0f6f7a2b9c11"})),
                1
            ),
        )
        .await;
        assert_eq!(connected.status(), StatusCode::OK);
        let bad = actix_test::call_service(
            &app,
            signed_in!(
                actix_test::TestRequest::put()
                    .uri("/scrobbling/listenbrainz")
                    .set_json(json!({"token": "not-a-token"})),
                1
            ),
        )
        .await;
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

        let status: Value = actix_test::call_and_read_body_json(
            &app,
            signed_in!(
                actix_test::TestRequest::patch()
                    .uri("/scrobbling/lastfm")
                    .set_json(json!({"scrobbleRadio": false, "sendNowPlaying": false})),
                1
            ),
        )
        .await;
        let lastfm = &status["services"][0];
        assert_eq!(lastfm["connection"]["accountName"], "parson-listener");
        assert_eq!(lastfm["connection"]["scrobbleRadio"], false);
        assert_eq!(lastfm["connection"]["sendNowPlaying"], false);
        assert_eq!(status["services"][1]["connection"]["enabled"], true);
        assert_eq!(status["lastfmSetup"]["configured"], true);

        // Another profile sees none of it.
        let other: Value = actix_test::call_and_read_body_json(
            &app,
            signed_in!(actix_test::TestRequest::get().uri("/scrobbling"), 2),
        )
        .await;
        assert!(other["services"][0]["connection"].is_null());

        let removed = actix_test::call_service(
            &app,
            signed_in!(
                actix_test::TestRequest::delete().uri("/scrobbling/listenbrainz"),
                1
            ),
        )
        .await;
        assert_eq!(removed.status(), StatusCode::NO_CONTENT);
        let unknown = actix_test::call_service(
            &app,
            signed_in!(
                actix_test::TestRequest::delete().uri("/scrobbling/spotify"),
                1
            ),
        )
        .await;
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    }

    #[actix_web::test]
    async fn only_administrators_set_the_last_fm_api_key() {
        let pool = pool();
        let hub = web::Data::new(hub_with(pool.clone(), fixtures()));
        let app = actix_test::init_service(
            App::new()
                .app_data(hub)
                .app_data(web::Data::new(pool.clone()))
                .configure(configure),
        )
        .await;
        let response = actix_test::call_service(
            &app,
            signed_in!(
                actix_test::TestRequest::put()
                    .uri("/scrobbling/lastfm/server")
                    .set_json(json!({
                        "apiKey": "0123456789abcdef0123456789abcdef",
                        "apiSecret": "fedcba9876543210fedcba9876543210"
                    })),
                1
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
