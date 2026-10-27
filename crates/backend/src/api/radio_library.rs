//! Radio's "never play" lists: per-listener bans (artists, albums, tracks)
//! and disliked tracks, with the endpoints to review and undo them.
//!
//! Mounted under `/playback/radio` by [`super::playback::configure`]:
//! - `GET    /bans`              every ban, hydrated for display
//! - `POST   /bans`              `{kind, id}` bans a song, album or artist
//! - `DELETE /bans/{kind}/{id}`  unbans
//! - `GET    /dislikes`          tracks Radio keeps out after a thumbs down
//! - `DELETE /dislikes/{song_id}` undoes one dislike

use actix_web::{HttpRequest, HttpResponse, delete, get, post, web};
use serde::{Deserialize, Serialize};

use super::playback::{QueueItemRow, QueueSong, hydrate_queue_item, valid_song_id};
use crate::api::auth::authenticated_user_id;
use crate::library::state::{LibraryCache, LibraryLifecycle, library_unavailable_response};
use crate::persistence::connection::DbPool;
use crate::radio_bans::{
    AddBanError, BanKind, BanRow, add_ban as store_ban, list_bans as stored_bans,
};
use crate::recommendation::{self, DislikedTrack};

const MAX_DISLIKES_LISTED: i64 = 500;

/// A ban as clients show it. `name` is `None` when the banned entity has
/// left the catalog; the ban is kept so it applies again if it returns.
#[derive(Serialize)]
pub(crate) struct BanItem {
    kind: BanKind,
    id: String,
    name: Option<String>,
    /// Artist (and album for tracks) for display.
    #[serde(skip_serializing_if = "Option::is_none")]
    subtitle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    artist_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    album_id: Option<String>,
    /// Playable track, for song bans still in the catalog.
    #[serde(skip_serializing_if = "Option::is_none")]
    song: Option<QueueSong>,
    created_at: String,
}

#[derive(Serialize)]
struct BanList {
    items: Vec<BanItem>,
}

fn non_empty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.to_owned())
}

pub(crate) fn hydrate_ban(row: BanRow, cache: &LibraryCache) -> Option<BanItem> {
    let kind = BanKind::parse(&row.kind)?;
    let mut item = BanItem {
        kind,
        id: row.entity_id,
        name: None,
        subtitle: None,
        image_url: None,
        artist_id: None,
        album_id: None,
        song: None,
        created_at: row.created_at,
    };
    match kind {
        BanKind::Song => {
            if let Some(song) = hydrate_queue_item(
                QueueItemRow {
                    position: 0,
                    song_id: item.id.clone(),
                    origin: "manual".into(),
                    score: 0.0,
                    reason: String::new(),
                },
                cache,
            ) {
                let (artist_id, album_id) = cache.song_map.get(&item.id).cloned()?;
                let album = cache.album(&album_id);
                item.name = cache.song(&item.id).map(|song| song.name.clone());
                item.subtitle = cache.song(&item.id).map(|song| match album {
                    Some(album) => format!("{} · {}", song.artist, album.name),
                    None => song.artist.clone(),
                });
                item.image_url = album.and_then(|album| non_empty(&album.cover_url));
                item.artist_id = Some(artist_id);
                item.album_id = Some(album_id);
                item.song = Some(song);
            }
        }
        BanKind::Album => {
            if let Some(album) = cache.album(&item.id) {
                item.name = Some(album.name.clone());
                item.image_url = non_empty(&album.cover_url);
                if let Some(owner) = cache.album_owner(&item.id) {
                    item.subtitle = Some(owner.name.clone());
                    item.artist_id = Some(owner.id.clone());
                }
            }
        }
        BanKind::Artist => {
            if let Some(artist) = cache.artist(&item.id) {
                item.name = Some(artist.name.clone());
                item.image_url = non_empty(&artist.icon_url);
                item.subtitle = Some("Artist".into());
            }
        }
    }
    Some(item)
}

async fn run<T: Send + 'static>(
    pool: &web::Data<DbPool>,
    label: &'static str,
    work: impl FnOnce(&mut diesel::SqliteConnection) -> Result<T, String> + Send + 'static,
) -> Result<T, HttpResponse> {
    let pool = pool.get_ref().clone();
    match web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        work(&mut connection)
    })
    .await
    {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => {
            tracing::error!(%error, label, "radio library request failed");
            Err(HttpResponse::InternalServerError().finish())
        }
        Err(error) => {
            tracing::error!(%error, label, "radio library worker failed");
            Err(HttpResponse::InternalServerError().finish())
        }
    }
}

/// The signed-in listener's bans, or none when they cannot be loaded (bans
/// then fail open rather than breaking Shuffle).
pub(crate) async fn listener_bans(
    request: &HttpRequest,
    pool: Option<&web::Data<DbPool>>,
) -> crate::radio_bans::RadioBans {
    let (Ok(user_id), Some(pool)) = (authenticated_user_id(request), pool) else {
        return crate::radio_bans::RadioBans::default();
    };
    run(pool, "load bans", move |connection| {
        crate::radio_bans::RadioBans::load(connection, user_id).map_err(|error| error.to_string())
    })
    .await
    .unwrap_or_default()
}

#[get("/bans")]
pub(crate) async fn list_bans(
    request: HttpRequest,
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    let rows = match run(&pool, "list bans", move |connection| {
        stored_bans(connection, user_id).map_err(|error| error.to_string())
    })
    .await
    {
        Ok(rows) => rows,
        Err(response) => return response,
    };
    HttpResponse::Ok().json(BanList {
        items: rows
            .into_iter()
            .filter_map(|row| hydrate_ban(row, cache.as_ref()))
            .collect(),
    })
}

#[derive(Deserialize)]
struct BanRequest {
    kind: String,
    id: String,
}

#[post("/bans")]
pub(crate) async fn add_ban(
    request: HttpRequest,
    form: web::Json<BanRequest>,
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let (Some(kind), true) = (BanKind::parse(&form.kind), valid_song_id(&form.id)) else {
        return HttpResponse::BadRequest().json(serde_json::json!({ "error": "invalid_ban" }));
    };
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    if !kind.exists(&form.id, cache.as_ref()) {
        return HttpResponse::NotFound()
            .json(serde_json::json!({ "error": "ban_target_not_found" }));
    }
    let id = form.id.clone();
    let stored = run(&pool, "add ban", move |connection| {
        Ok(store_ban(connection, user_id, kind, &id))
    })
    .await;
    match stored {
        Ok(Ok(row)) => match hydrate_ban(row, cache.as_ref()) {
            Some(item) => HttpResponse::Ok().json(item),
            None => HttpResponse::InternalServerError().finish(),
        },
        Ok(Err(AddBanError::TooMany)) => HttpResponse::UnprocessableEntity()
            .json(serde_json::json!({ "error": "too_many_bans" })),
        Ok(Err(AddBanError::Database(error))) => {
            tracing::error!(%error, "ban could not be stored");
            HttpResponse::InternalServerError().finish()
        }
        Err(response) => response,
    }
}

#[delete("/bans/{kind}/{id}")]
pub(crate) async fn remove_ban(
    request: HttpRequest,
    path: web::Path<(String, String)>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let (kind, id) = path.into_inner();
    let (Some(kind), true) = (BanKind::parse(&kind), valid_song_id(&id)) else {
        return HttpResponse::BadRequest().json(serde_json::json!({ "error": "invalid_ban" }));
    };
    match run(&pool, "remove ban", move |connection| {
        crate::radio_bans::remove_ban(connection, user_id, kind, &id)
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(_) => HttpResponse::NoContent().finish(),
        Err(response) => response,
    }
}

#[derive(Serialize)]
struct DislikedItem {
    song: QueueSong,
    #[serde(skip_serializing_if = "Option::is_none")]
    disliked_at: Option<String>,
}

#[derive(Serialize)]
struct DislikedList {
    items: Vec<DislikedItem>,
}

fn hydrate_dislike(row: DislikedTrack, cache: &LibraryCache) -> Option<DislikedItem> {
    hydrate_queue_item(
        QueueItemRow {
            position: 0,
            song_id: row.song_id,
            origin: "manual".into(),
            score: 0.0,
            reason: String::new(),
        },
        cache,
    )
    .map(|song| DislikedItem {
        song,
        disliked_at: row.disliked_at,
    })
}

#[get("/dislikes")]
pub(crate) async fn list_dislikes(
    request: HttpRequest,
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    let rows = match run(&pool, "list dislikes", move |connection| {
        recommendation::disliked_tracks(connection, user_id, MAX_DISLIKES_LISTED)
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(rows) => rows,
        Err(response) => return response,
    };
    HttpResponse::Ok().json(DislikedList {
        items: rows
            .into_iter()
            .filter_map(|row| hydrate_dislike(row, cache.as_ref()))
            .collect(),
    })
}

#[delete("/dislikes/{song_id}")]
pub(crate) async fn clear_dislike(
    request: HttpRequest,
    song_id: web::Path<String>,
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let song_id = song_id.into_inner();
    if !valid_song_id(&song_id) {
        return HttpResponse::BadRequest().json(serde_json::json!({ "error": "invalid_song" }));
    }
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    match run(&pool, "clear dislike", move |connection| {
        recommendation::clear_dislike(connection, user_id, &song_id, cache.as_ref())
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(_) => HttpResponse::NoContent().finish(),
        Err(response) => response,
    }
}

#[cfg(test)]
mod tests {
    use actix_web::http::StatusCode;
    use actix_web::{App, HttpMessage, test as actix_test, web};

    use crate::library::state::LibraryLifecycle;
    use crate::recommendation::test_support::{radio_pool, radio_test_cache};

    fn claims(user: &str) -> crate::api::auth::Claims {
        crate::api::auth::Claims {
            sub: user.to_string(),
            exp: usize::MAX,
            username: "listener".to_string(),
            bitrate: 0,
            token_type: "access".to_string(),
            role: "user".to_string(),
            token_version: 0,
            session_id: None,
        }
    }

    #[actix_web::test]
    async fn bans_can_be_added_listed_and_removed_over_http() {
        let pool = radio_pool();
        let lifecycle = LibraryLifecycle::new();
        lifecycle.set_ready(radio_test_cache()).await;
        let app = actix_test::init_service(
            App::new()
                .app_data(web::Data::new(pool.clone()))
                .app_data(web::Data::new(lifecycle))
                .configure(crate::api::playback::configure),
        )
        .await;

        let call = |request: actix_test::TestRequest, user: &str| {
            let request = request.to_request();
            request.extensions_mut().insert(claims(user));
            request
        };

        let response = actix_test::call_service(
            &app,
            call(
                actix_test::TestRequest::post()
                    .uri("/playback/radio/bans")
                    .set_json(serde_json::json!({ "kind": "artist", "id": "artist-jazz" })),
                "1",
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = actix_test::read_body_json(response).await;
        assert_eq!(body["kind"], "artist");
        assert_eq!(body["name"], "artist-jazz name");

        let response = actix_test::call_service(
            &app,
            call(
                actix_test::TestRequest::post()
                    .uri("/playback/radio/bans")
                    .set_json(serde_json::json!({ "kind": "track", "id": "r1" })),
                "1",
            ),
        )
        .await;
        let body: serde_json::Value = actix_test::read_body_json(response).await;
        assert_eq!(body["kind"], "song");
        assert_eq!(body["song"]["id"], "r1");
        assert_eq!(body["album_id"], "album-rock-1");

        for (payload, status) in [
            (
                serde_json::json!({ "kind": "album", "id": "missing" }),
                StatusCode::NOT_FOUND,
            ),
            (
                serde_json::json!({ "kind": "genre", "id": "Rock" }),
                StatusCode::BAD_REQUEST,
            ),
        ] {
            let response = actix_test::call_service(
                &app,
                call(
                    actix_test::TestRequest::post()
                        .uri("/playback/radio/bans")
                        .set_json(payload),
                    "1",
                ),
            )
            .await;
            assert_eq!(response.status(), status);
        }

        let list = |user: &'static str| {
            call(
                actix_test::TestRequest::get().uri("/playback/radio/bans"),
                user,
            )
        };
        let body: serde_json::Value =
            actix_test::read_body_json(actix_test::call_service(&app, list("1")).await).await;
        assert_eq!(body["items"].as_array().unwrap().len(), 2);
        let body: serde_json::Value =
            actix_test::read_body_json(actix_test::call_service(&app, list("2")).await).await;
        assert!(body["items"].as_array().unwrap().is_empty());

        // Radio picks skip the banned artist straight away.
        let response = actix_test::call_service(
            &app,
            call(
                actix_test::TestRequest::post()
                    .uri("/playback/radio/picks")
                    .set_json(serde_json::json!({ "seed_genre": "Jazz", "limit": 10 })),
                "1",
            ),
        )
        .await;
        let body: serde_json::Value = actix_test::read_body_json(response).await;
        let picks = body["items"].as_array().unwrap();
        assert!(!picks.is_empty(), "radio widened past the banned artist");
        assert!(picks.iter().all(|pick| {
            let id = pick["id"].as_str().unwrap();
            !id.starts_with('j') && id != "r1"
        }));

        let response = actix_test::call_service(
            &app,
            call(
                actix_test::TestRequest::delete().uri("/playback/radio/bans/artist/artist-jazz"),
                "1",
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let body: serde_json::Value =
            actix_test::read_body_json(actix_test::call_service(&app, list("1")).await).await;
        assert_eq!(body["items"].as_array().unwrap().len(), 1);
        assert_eq!(body["items"][0]["id"], "r1");
    }

    #[actix_web::test]
    async fn a_dislike_can_be_reviewed_and_undone_over_http() {
        let pool = radio_pool();
        let cache = radio_test_cache();
        crate::recommendation::record_playback_event(
            1,
            &crate::recommendation::PlaybackEventRequest {
                event_key: "down".into(),
                song_id: "r2".into(),
                event_type: "disliked".into(),
                session_id: None,
                queue_id: None,
                source: Some("generated".into()),
                position_seconds: Some(3.0),
                duration_seconds: Some(200.0),
                ..Default::default()
            },
            &cache,
            &pool,
        )
        .unwrap();
        let lifecycle = LibraryLifecycle::new();
        lifecycle.set_ready(cache).await;
        let app = actix_test::init_service(
            App::new()
                .app_data(web::Data::new(pool.clone()))
                .app_data(web::Data::new(lifecycle))
                .configure(crate::api::playback::configure),
        )
        .await;
        let request = actix_test::TestRequest::get()
            .uri("/playback/radio/dislikes")
            .to_request();
        request.extensions_mut().insert(claims("1"));
        let body: serde_json::Value =
            actix_test::read_body_json(actix_test::call_service(&app, request).await).await;
        assert_eq!(body["items"][0]["song"]["id"], "r2");

        let request = actix_test::TestRequest::delete()
            .uri("/playback/radio/dislikes/r2")
            .to_request();
        request.extensions_mut().insert(claims("1"));
        assert_eq!(
            actix_test::call_service(&app, request).await.status(),
            StatusCode::NO_CONTENT
        );
        let request = actix_test::TestRequest::get()
            .uri("/playback/radio/dislikes")
            .to_request();
        request.extensions_mut().insert(claims("1"));
        let body: serde_json::Value =
            actix_test::read_body_json(actix_test::call_service(&app, request).await).await;
        assert!(body["items"].as_array().unwrap().is_empty());
    }
}
