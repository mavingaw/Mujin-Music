//! `/key-service/v1/*`: the Parson key service, served by a server that
//! holds TIDAL and Qobuz keys for other Parson servers (see
//! `providers::key_service`). Mounted only when `PARSON_KEY_SERVICE_PUBLIC_URL`
//! and at least one key are set; no account is needed to call it, because
//! the callers are other servers and TIDAL's redirect.

use std::sync::Mutex;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpResponse, web};
use serde_json::json;

use crate::providers::key_service::{ServiceKeys, ServiceRefusal, relay_location, sign_file_url};
use crate::providers::unix_now;

/// Signatures handed out per minute, across all callers.
const SIGNATURES_PER_MINUTE: u32 = 1_200;

pub struct KeyServiceApp {
    keys: ServiceKeys,
    window: Mutex<(i64, u32)>,
}

impl KeyServiceApp {
    pub fn new(keys: ServiceKeys) -> Self {
        Self {
            keys,
            window: Mutex::new((0, 0)),
        }
    }

    fn admit(&self) -> bool {
        let minute = unix_now() / 60;
        let mut window = self
            .window
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if window.0 != minute {
            *window = (minute, 0);
        }
        window.1 += 1;
        window.1 <= SIGNATURES_PER_MINUTE
    }
}

pub fn configure(config: &mut web::ServiceConfig, app: web::Data<KeyServiceApp>) {
    config.service(
        web::scope("/key-service/v1")
            .app_data(app)
            .route("/apps", web::get().to(apps))
            .route("/qobuz/sign", web::post().to(sign))
            .route("/tidal/callback", web::get().to(tidal_callback)),
    );
}

fn refusal(refusal: ServiceRefusal) -> HttpResponse {
    match refusal {
        ServiceRefusal::NotOffered => {
            HttpResponse::NotFound().json(json!({"error": "not_offered"}))
        }
        ServiceRefusal::Invalid(message) => {
            HttpResponse::BadRequest().json(json!({"error": "invalid", "message": message}))
        }
    }
}

async fn apps(app: web::Data<KeyServiceApp>) -> HttpResponse {
    HttpResponse::Ok()
        .insert_header((header::CACHE_CONTROL, "public, max-age=3600"))
        .json(app.keys.manifest())
}

async fn sign(
    app: web::Data<KeyServiceApp>,
    form: web::Form<Vec<(String, String)>>,
) -> HttpResponse {
    if !app.admit() {
        return HttpResponse::TooManyRequests().json(json!({"error": "rate_limited"}));
    }
    match sign_file_url(&app.keys, &form.into_inner(), unix_now()) {
        Ok(signature) => HttpResponse::Ok()
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(json!({"request_sig": signature})),
        Err(error) => refusal(error),
    }
}

async fn tidal_callback(
    query: web::Query<Vec<(String, String)>>,
    app: web::Data<KeyServiceApp>,
) -> HttpResponse {
    if app.keys.tidal_client_id.is_none() {
        return refusal(ServiceRefusal::NotOffered);
    }
    match relay_location(&query.into_inner()) {
        Ok(location) => HttpResponse::build(StatusCode::FOUND)
            .insert_header((header::LOCATION, location))
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .insert_header((header::REFERRER_POLICY, "no-referrer"))
            .finish(),
        Err(error) => refusal(error),
    }
}

#[cfg(test)]
mod tests {
    use actix_web::{App, http::StatusCode, test, web};

    use super::{KeyServiceApp, configure};
    use crate::providers::key_service::{ServiceKeys, parse_manifest, relay_state};
    use crate::providers::qobuz::file_url_signature;
    use crate::providers::tidal::random_state;
    use crate::providers::unix_now;

    fn keys() -> ServiceKeys {
        ServiceKeys {
            issuer: "Parson".into(),
            public_url: "https://keys.parson.test/key-service".into(),
            tidal_client_id: Some("parson-tidal".into()),
            tidal_device_code: false,
            qobuz_app_id: Some("123456789".into()),
            qobuz_app_secret: Some("partner-secret".into()),
        }
    }

    #[actix_web::test]
    async fn routes_publish_ids_sign_streams_and_relay_tidal() {
        let data = web::Data::new(KeyServiceApp::new(keys()));
        let service =
            test::init_service(App::new().configure(|config| configure(config, data.clone())))
                .await;

        let apps = test::call_service(
            &service,
            test::TestRequest::get()
                .uri("/key-service/v1/apps")
                .to_request(),
        )
        .await;
        assert_eq!(apps.status(), StatusCode::OK);
        let body = test::read_body(apps).await;
        assert!(!String::from_utf8_lossy(&body).contains("partner-secret"));
        let manifest = parse_manifest(&body).expect("manifest");
        assert_eq!(
            manifest.tidal.expect("tidal").redirect_uri,
            "https://keys.parson.test/key-service/v1/tidal/callback"
        );

        let now = unix_now();
        let signed = test::call_service(
            &service,
            test::TestRequest::post()
                .uri("/key-service/v1/qobuz/sign")
                .set_form([
                    ("app_id", "123456789".to_string()),
                    ("track_id", "7700001".to_string()),
                    ("format_id", "27".to_string()),
                    ("intent", "stream".to_string()),
                    ("request_ts", now.to_string()),
                ])
                .to_request(),
        )
        .await;
        assert_eq!(signed.status(), StatusCode::OK);
        let body: serde_json::Value = test::read_body_json(signed).await;
        assert_eq!(
            body["request_sig"],
            file_url_signature(27, "7700001", now, "partner-secret")
        );

        let refused = test::call_service(
            &service,
            test::TestRequest::post()
                .uri("/key-service/v1/qobuz/sign")
                .set_form([("app_id", "123456789"), ("intent", "download")])
                .to_request(),
        )
        .await;
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST);

        let back = "https://music.example.com/settings/services/tidal-callback";
        let nonce = random_state();
        let state = relay_state(&nonce, back);
        let relayed = test::call_service(
            &service,
            test::TestRequest::get()
                .uri(&format!(
                    "/key-service/v1/tidal/callback?code=abc&state={state}"
                ))
                .to_request(),
        )
        .await;
        assert_eq!(relayed.status(), StatusCode::FOUND);
        let location = relayed
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        assert!(location.starts_with(&format!("{back}?code=abc&state={nonce}.")));
        assert_eq!(
            relayed
                .headers()
                .get("referrer-policy")
                .and_then(|value| value.to_str().ok()),
            Some("no-referrer")
        );

        let foreign = relay_state(&random_state(), "https://evil.test/steal");
        let refused = test::call_service(
            &service,
            test::TestRequest::get()
                .uri(&format!(
                    "/key-service/v1/tidal/callback?code=abc&state={foreign}"
                ))
                .to_request(),
        )
        .await;
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    }
}
