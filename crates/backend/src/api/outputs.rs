//! `/outputs/native`: the server's own audio devices and its native
//! playback session.
//!
//! - `GET  /outputs/native` lists devices with their bit-perfect format
//!   support and settings.
//! - `POST /outputs/native/admin/refresh` re-probes the devices (admin).
//! - `PUT  /outputs/native/admin/settings` changes one device's Exclusive
//!   Mode, max sample rate and max bit depth (admin).
//! - `PUT  /outputs/native/dsp` saves one device's DSP chain (headroom,
//!   parametric EQ, crossfeed, sample rate), which the server runs in
//!   64-bit float before the device. A track playing there restarts at its
//!   position with the new chain.
//! - `GET  /outputs/native/session` returns what is playing, or `null`.
//! - `POST /outputs/native/session` plays a song on a device.
//! - `POST /outputs/native/session/commands` pauses, resumes, seeks or
//!   stops it.

use std::sync::Arc;

use actix_web::{HttpRequest, HttpResponse, get, post, put, web};
use actix_web_httpauth::middleware::HttpAuthentication;
use serde::Deserialize;

use crate::api::auth::{admin_guard, authenticated_user_id};
use crate::api::error::{bad_request, internal_server_error, not_found, service_unavailable};
use crate::library::state::LibraryLifecycle;
use crate::native_output::{LevelingRequest, NativeError, NativeOutputs};
use parson_output::OutputSettings;
use parson_output::dsp::DspSettings;

/// Longest accepted device id (WASAPI endpoint ids are about 55 chars).
const MAX_DEVICE_ID: usize = 512;

fn outputs(data: Option<web::Data<NativeOutputs>>) -> Arc<NativeOutputs> {
    data.map(|data| data.into_inner())
        .unwrap_or_else(crate::native_output::global)
}

fn error_response(error: &NativeError) -> HttpResponse {
    let message = error.to_string();
    match error {
        NativeError::UnknownDevice | NativeError::NoSession => not_found(message, error.code()),
        NativeError::Plan(_) | NativeError::UnknownFormat => bad_request(message, error.code()),
        NativeError::Device(_) => service_unavailable(message, error.code()),
    }
}

fn valid_device_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_DEVICE_ID && !id.chars().any(char::is_control)
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, HttpResponse> {
    web::block(work).await.map_err(|_| {
        internal_server_error(
            "The audio output task stopped.",
            "native_output_task_failed",
        )
    })
}

#[get("")]
async fn list_devices(data: Option<web::Data<NativeOutputs>>) -> HttpResponse {
    let outputs = outputs(data);
    match blocking(move || outputs.devices(false)).await {
        Ok(view) => HttpResponse::Ok().json(view),
        Err(response) => response,
    }
}

#[post("/refresh")]
async fn refresh_devices(data: Option<web::Data<NativeOutputs>>) -> HttpResponse {
    let outputs = outputs(data);
    match blocking(move || outputs.devices(true)).await {
        Ok(view) => HttpResponse::Ok().json(view),
        Err(response) => response,
    }
}

#[derive(Deserialize)]
struct SettingsRequest {
    device_id: String,
    settings: OutputSettings,
}

#[put("/settings")]
async fn update_settings(
    body: web::Json<SettingsRequest>,
    data: Option<web::Data<NativeOutputs>>,
) -> HttpResponse {
    let body = body.into_inner();
    if !valid_device_id(&body.device_id) {
        return bad_request("Invalid device id.", "invalid_device_id");
    }
    let outputs = outputs(data);
    match blocking(move || outputs.set_settings(&body.device_id, body.settings)).await {
        Ok(Ok(view)) => HttpResponse::Ok().json(view),
        Ok(Err(message)) => bad_request(message, "invalid_output_settings"),
        Err(response) => response,
    }
}

#[derive(Deserialize)]
struct DspRequest {
    device_id: String,
    settings: DspSettings,
}

#[put("/dsp")]
async fn update_dsp(
    body: web::Json<DspRequest>,
    data: Option<web::Data<NativeOutputs>>,
) -> HttpResponse {
    let body = body.into_inner();
    if !valid_device_id(&body.device_id) {
        return bad_request("Invalid device id.", "invalid_device_id");
    }
    let outputs = outputs(data);
    match blocking(move || outputs.set_dsp(&body.device_id, body.settings)).await {
        Ok(Ok(view)) => HttpResponse::Ok().json(view),
        Ok(Err(message)) => not_found(message, "native_output_device_not_found"),
        Err(response) => response,
    }
}

#[get("/session")]
async fn current_session(data: Option<web::Data<NativeOutputs>>) -> HttpResponse {
    HttpResponse::Ok().json(outputs(data).current())
}

#[derive(Deserialize)]
struct LevelingBody {
    gain_db: f64,
    source: String,
}

#[derive(Deserialize)]
struct PlayRequest {
    device_id: String,
    song_id: String,
    #[serde(default)]
    position_ms: u64,
    /// Volume leveling to apply on the server, in dB.
    #[serde(default)]
    leveling: Option<LevelingBody>,
}

#[post("/session")]
async fn play(
    request: HttpRequest,
    body: web::Json<PlayRequest>,
    lifecycle: web::Data<LibraryLifecycle>,
    data: Option<web::Data<NativeOutputs>>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    let body = body.into_inner();
    if !valid_device_id(&body.device_id) {
        return bad_request("Invalid device id.", "invalid_device_id");
    }
    let leveling = body.leveling.map(|leveling| LevelingRequest {
        gain_db: leveling.gain_db,
        source: leveling.source,
    });
    if let Some(Err(message)) = leveling.as_ref().map(LevelingRequest::validate) {
        return bad_request(message, "invalid_leveling");
    }
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return crate::library::state::library_unavailable_response(readiness),
    };
    let Some(song) = cache.song(&body.song_id).cloned() else {
        return not_found("Song not found.", "song_not_found");
    };
    let outputs = outputs(data);
    match blocking(move || {
        outputs.play(user_id, &body.device_id, &song, body.position_ms, leveling)
    })
    .await
    {
        Ok(Ok(view)) => HttpResponse::Ok().json(view),
        Ok(Err(error)) => error_response(&error),
        Err(response) => response,
    }
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum SessionCommand {
    Pause,
    Resume,
    Seek,
    Stop,
}

#[derive(Deserialize)]
struct CommandRequest {
    command: SessionCommand,
    position_ms: Option<u64>,
}

#[post("/session/commands")]
async fn command(
    body: web::Json<CommandRequest>,
    data: Option<web::Data<NativeOutputs>>,
) -> HttpResponse {
    let outputs = outputs(data);
    let body = body.into_inner();
    let result = match body.command {
        SessionCommand::Pause => outputs.pause(),
        SessionCommand::Resume => outputs.resume(),
        SessionCommand::Seek => match body.position_ms {
            Some(position) => outputs.seek(position),
            None => return bad_request("Seek needs a position.", "invalid_position"),
        },
        SessionCommand::Stop => {
            return match blocking(move || outputs.stop()).await {
                Ok(Ok(())) => HttpResponse::NoContent().finish(),
                Ok(Err(error)) => error_response(&error),
                Err(response) => response,
            };
        }
    };
    match result {
        Ok(view) => HttpResponse::Ok().json(view),
        Err(error) => error_response(&error),
    }
}

/// Mounted under the authenticated music scope.
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/outputs/native")
            .service(
                web::scope("/admin")
                    .wrap(HttpAuthentication::with_fn(admin_guard))
                    .service(refresh_devices)
                    .service(update_settings),
            )
            .service(list_devices)
            .service(update_dsp)
            .service(current_session)
            .service(play)
            .service(command),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Album, Artist, AudioProperties, Song};
    use actix_web::{App, HttpMessage, test};
    use parson_output::player::PcmSource;
    use parson_output::simulated::{SIMULATED_DEVICE_ID, SimulatedBackend};
    use std::collections::HashMap;

    fn cache() -> crate::library::state::LibraryCache {
        let song = Song {
            id: "song-1".into(),
            name: "Fixture".into(),
            path: "/music/fixture.flac".into(),
            duration: 180.0,
            audio: Some(AudioProperties {
                codec: Some("flac".into()),
                container: Some("flac".into()),
                sample_rate: Some(352_800),
                bit_depth: Some(24),
                channels: Some(2),
                bitrate_kbps: Some(9_000),
                lossless: Some(true),
            }),
            ..Song::default()
        };
        let album = Album {
            id: "album-1".into(),
            songs: vec![song],
            ..Album::default()
        };
        let artist = Artist {
            id: "artist-1".into(),
            albums: vec![album],
            ..Artist::default()
        };
        crate::library::state::LibraryCache {
            search_index: crate::library::search::SearchIndex::build(std::slice::from_ref(&artist))
                .expect("search index"),
            artists: Arc::new(vec![artist]),
            song_map: HashMap::from([("song-1".into(), ("artist-1".into(), "album-1".into()))]),
            album_genres: HashMap::new(),
            artist_positions: HashMap::from([("artist-1".into(), 0)]),
            album_positions: HashMap::from([("album-1".into(), (0, 0))]),
            song_positions: HashMap::from([("song-1".into(), (0, 0, 0))]),
            songs_flat: vec!["song-1".into()],
            songs_by_artist: HashMap::new(),
            songs_by_genre: HashMap::new(),
            image_paths: Default::default(),
        }
    }

    fn native() -> NativeOutputs {
        NativeOutputs::new(
            vec![Arc::new(SimulatedBackend)],
            true,
            None,
            Arc::new(|_, _, _, _, _| Ok(Box::new(std::io::repeat(0)) as PcmSource)),
            "test resampler",
        )
        .with_float_decoder(Arc::new(|_, _, _| {
            Ok(Box::new(std::io::repeat(0)) as PcmSource)
        }))
    }

    #[actix_web::test]
    async fn native_output_routes_list_play_and_stop() {
        let lifecycle = LibraryLifecycle::new();
        lifecycle.set_ready(cache()).await;
        let outputs = web::Data::new(native());
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(lifecycle))
                .app_data(outputs.clone())
                .wrap_fn(|request, service| {
                    request.extensions_mut().insert(crate::api::auth::Claims {
                        sub: "3".into(),
                        exp: usize::MAX,
                        username: "listener".into(),
                        bitrate: 0,
                        token_type: "access".into(),
                        role: "user".into(),
                        token_version: 0,
                        session_id: None,
                    });
                    actix_web::dev::Service::call(service, request)
                })
                .configure(configure),
        )
        .await;

        let listed: serde_json::Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::get().uri("/outputs/native").to_request(),
        )
        .await;
        assert_eq!(listed["simulated"], true);
        let device = &listed["devices"][0];
        assert_eq!(device["id"], SIMULATED_DEVICE_ID);
        assert_eq!(device["mode_label"], "Simulated exclusive output");
        assert_eq!(device["formats"]["sample_rates"][6]["value"], 352_800);
        assert_eq!(device["formats"]["sample_rates"][6]["supported"], false);

        let empty: serde_json::Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::get()
                .uri("/outputs/native/session")
                .to_request(),
        )
        .await;
        assert!(empty.is_null());

        let started: serde_json::Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::post()
                .uri("/outputs/native/session")
                .set_json(serde_json::json!({
                    "device_id": SIMULATED_DEVICE_ID,
                    "song_id": "song-1",
                    "position_ms": 1000,
                }))
                .to_request(),
        )
        .await;
        assert_eq!(started["song_id"], "song-1");
        assert_eq!(started["user_id"], 3);
        assert_eq!(started["duration_ms"], 180_000);
        // 352.8 kHz is not supported: converted to 176.4 kHz, same family.
        assert_eq!(started["plan"]["sample_rate"], 176_400);
        assert_eq!(started["signal_path"]["quality"], "high_quality");
        assert_eq!(started["signal_path"]["target"], "native");
        let stages = started["signal_path"]["stages"].as_array().unwrap();
        assert!(stages.iter().any(|stage| {
            stage["id"] == "native-resample"
                && stage["detail"]
                    .as_str()
                    .unwrap()
                    .starts_with("352.8kHz to 176.4kHz")
        }));
        let output = stages.last().unwrap();
        assert_eq!(output["kind"], "output");
        assert_eq!(output["quality"], "lossless");
        assert_eq!(
            output["detail"],
            "Simulated exclusive output, 176.4kHz 24bit 2ch"
        );

        let paused = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/outputs/native/session/commands")
                .set_json(serde_json::json!({"command": "pause"}))
                .to_request(),
        )
        .await;
        assert!(paused.status().is_success());
        let bad_seek = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/outputs/native/session/commands")
                .set_json(serde_json::json!({"command": "seek"}))
                .to_request(),
        )
        .await;
        assert_eq!(bad_seek.status(), 400);
        let stopped = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/outputs/native/session/commands")
                .set_json(serde_json::json!({"command": "stop"}))
                .to_request(),
        )
        .await;
        assert_eq!(stopped.status(), 204);
        let again = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/outputs/native/session/commands")
                .set_json(serde_json::json!({"command": "stop"}))
                .to_request(),
        )
        .await;
        assert_eq!(again.status(), 404);

        for (body, status) in [
            (
                serde_json::json!({"device_id": "nope:0", "song_id": "song-1"}),
                404,
            ),
            (
                serde_json::json!({"device_id": SIMULATED_DEVICE_ID, "song_id": "missing"}),
                404,
            ),
            (
                serde_json::json!({"device_id": "", "song_id": "song-1"}),
                400,
            ),
        ] {
            let response = test::call_service(
                &app,
                test::TestRequest::post()
                    .uri("/outputs/native/session")
                    .set_json(body)
                    .to_request(),
            )
            .await;
            assert_eq!(response.status(), status);
        }
    }

    #[actix_web::test]
    async fn settings_route_validates_and_saves() {
        let outputs = web::Data::new(native());
        let app = test::init_service(
            App::new()
                .app_data(outputs.clone())
                .service(update_settings),
        )
        .await;
        let saved: serde_json::Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::put()
                .uri("/settings")
                .set_json(serde_json::json!({
                    "device_id": SIMULATED_DEVICE_ID,
                    "settings": {"exclusive": false, "max_sample_rate": 96000, "max_bit_depth": 24},
                }))
                .to_request(),
        )
        .await;
        assert_eq!(saved["mode_label"], "Simulated shared output");
        assert_eq!(saved["settings"]["max_sample_rate"], 96_000);
        assert_eq!(saved["formats"]["sample_rates"][4]["allowed"], false);
        let bad = test::call_service(
            &app,
            test::TestRequest::put()
                .uri("/settings")
                .set_json(serde_json::json!({
                    "device_id": SIMULATED_DEVICE_ID,
                    "settings": {"exclusive": true, "max_sample_rate": 50000},
                }))
                .to_request(),
        )
        .await;
        assert_eq!(bad.status(), 400);
    }

    #[actix_web::test]
    async fn dsp_route_saves_the_chain_and_the_server_runs_it() {
        let lifecycle = LibraryLifecycle::new();
        lifecycle.set_ready(cache()).await;
        let outputs = web::Data::new(native());
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(lifecycle))
                .app_data(outputs.clone())
                .wrap_fn(|request, service| {
                    request.extensions_mut().insert(crate::api::auth::Claims {
                        sub: "3".into(),
                        exp: usize::MAX,
                        username: "listener".into(),
                        bitrate: 0,
                        token_type: "access".into(),
                        role: "user".into(),
                        token_version: 0,
                        session_id: None,
                    });
                    actix_web::dev::Service::call(service, request)
                })
                .configure(configure),
        )
        .await;
        let saved: serde_json::Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::put()
                .uri("/outputs/native/dsp")
                .set_json(serde_json::json!({
                    "device_id": SIMULATED_DEVICE_ID,
                    "settings": {
                        "enabled": true,
                        "headroom": {"enabled": true, "gainDb": -3},
                        "eq": {"enabled": true, "preset": "bass", "bands": [
                            {"id": "band-1", "enabled": true, "type": "lowshelf", "frequency": 80, "gainDb": 5, "q": 0.71}
                        ]},
                        "crossfeed": {"enabled": true, "strength": "light"},
                        "sampleRate": {"enabled": true, "mode": 96000}
                    },
                }))
                .to_request(),
        )
        .await;
        assert_eq!(saved["dsp_customized"], true);
        assert_eq!(saved["dsp"]["eq"]["bands"][0]["gainDb"], 5.0);
        assert_eq!(saved["dsp_support"]["available"], true);
        assert_eq!(saved["dsp_support"]["crossfeed"], true);

        let started: serde_json::Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::post()
                .uri("/outputs/native/session")
                .set_json(serde_json::json!({
                    "device_id": SIMULATED_DEVICE_ID,
                    "song_id": "song-1",
                }))
                .to_request(),
        )
        .await;
        assert_eq!(started["plan"]["sample_rate"], 96_000);
        assert_eq!(started["plan"]["bit_perfect"], false);
        let path = &started["signal_path"];
        assert_eq!(path["quality"], "enhanced");
        let ids: Vec<&str> = path["stages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|stage| stage["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            [
                "source",
                "native-decode",
                "native-float-in",
                "native-headroom",
                "native-eq",
                "native-crossfeed",
                "native-resample",
                "native-depth",
                "native-transport",
                "native-output",
            ]
        );
        assert_eq!(path["stages"][6]["quality"], "enhanced");
        assert_eq!(path["stages"][8]["quality"], "lossless");
        assert_eq!(path["stages"][9]["quality"], "lossless");
        assert_eq!(path["processing"]["engine"], "Parson DSP, 64-bit float");

        // The live session reports processing speed and clipping.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let live = loop {
            let live: serde_json::Value = test::call_and_read_body_json(
                &app,
                test::TestRequest::get()
                    .uri("/outputs/native/session")
                    .to_request(),
            )
            .await;
            if live["signal_path"]["processing"]["status"]["realtime_factor"].is_number()
                || std::time::Instant::now() > deadline
            {
                break live;
            }
            actix_web::rt::time::sleep(std::time::Duration::from_millis(20)).await;
        };
        let status = &live["signal_path"]["processing"]["status"];
        assert!(status["realtime_factor"].as_f64().unwrap() > 1.0);
        assert_eq!(status["clipping"], false);

        let missing = test::call_service(
            &app,
            test::TestRequest::put()
                .uri("/outputs/native/dsp")
                .set_json(serde_json::json!({"device_id": "nope:0", "settings": {}}))
                .to_request(),
        )
        .await;
        assert_eq!(missing.status(), 404);
        outputs.stop().unwrap();
    }
}
