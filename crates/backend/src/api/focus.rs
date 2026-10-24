//! Focus routes: focused library pages, facet counts, saved focuses
//! (bookmarks), user tags and smart playlists built from a focus.

use std::collections::{HashMap, HashSet};

use actix_web::{HttpRequest, HttpResponse, delete, get, patch, post, put, web};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Text};
use diesel::sqlite::SqliteConnection;
use serde::{Deserialize, Serialize};

use crate::api::auth::authenticated_user_id;
use crate::api::error::{bad_request, conflict, internal_server_error, not_found};
use crate::app::LocalApp;
use crate::library::focus::{FacetRequest, FocusQuery, MAX_PAGE_SIZE, MAX_RESOLVED_IDS, focus_ids};
use crate::library::state::{LibraryLifecycle, library_unavailable_response};
use crate::persistence::connection::DbPool;
use crate::playlist_rules::{
    MAX_PLAYLIST_DESCRIPTION_CHARACTERS, MAX_PLAYLIST_NAME_CHARACTERS, MAX_PLAYLISTS,
    MAX_TRACKS_PER_PLAYLIST, valid_optional_text,
};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

pub(crate) const MAX_SAVED_FOCUSES: i64 = 200;
pub(crate) const MAX_SAVED_FOCUS_NAME_CHARACTERS: usize = 100;
pub(crate) const MAX_TAGS: i64 = 500;
pub(crate) const MAX_TAG_NAME_CHARACTERS: usize = 100;
pub(crate) const MAX_TAG_BATCH: usize = 500;
const MAX_ENTITY_ID_CHARACTERS: usize = 256;

fn worker_failed(context: &str, error: impl std::fmt::Display) -> HttpResponse {
    tracing::error!(%error, "{context}");
    internal_server_error(
        "The request could not be completed.",
        "focus_request_failed",
    )
}

#[derive(Deserialize)]
struct FocusPageRequest {
    #[serde(default)]
    query: FocusQuery,
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

#[post("")]
async fn focus_page(
    request: HttpRequest,
    body: web::Json<FocusPageRequest>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let body = body.into_inner();
    if let Err(error) = body.query.validate() {
        return bad_request(error.to_string(), "focus_invalid");
    }
    if let Err(readiness) = lifecycle.cache().await {
        return library_unavailable_response(readiness);
    }
    let app = LocalApp {
        database: pool.get_ref().clone(),
        library: lifecycle.into_inner(),
    };
    match app
        .focus(
            user_id,
            body.query,
            body.offset.unwrap_or(0).min(1_000_000),
            body.limit.unwrap_or(100).clamp(1, MAX_PAGE_SIZE),
        )
        .await
    {
        Ok(page) => HttpResponse::Ok().json(page),
        Err(error) => worker_failed("focus query failed", error),
    }
}

#[derive(Deserialize)]
struct FocusFacetsRequest {
    #[serde(default)]
    query: FocusQuery,
    #[serde(default, flatten)]
    facets: FacetRequest,
}

#[post("/facets")]
async fn focus_facets(
    request: HttpRequest,
    body: web::Json<FocusFacetsRequest>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let body = body.into_inner();
    if let Err(error) = body.query.validate() {
        return bad_request(error.to_string(), "focus_invalid");
    }
    if body.facets.only.len() > crate::library::focus::FACET_KEYS.len()
        || body
            .facets
            .search
            .as_deref()
            .is_some_and(|search| search.chars().count() > 200)
    {
        return bad_request("The facet request is invalid.", "focus_facets_invalid");
    }
    if let Err(readiness) = lifecycle.cache().await {
        return library_unavailable_response(readiness);
    }
    let app = LocalApp {
        database: pool.get_ref().clone(),
        library: lifecycle.into_inner(),
    };
    match app.focus_facets(user_id, body.query, body.facets).await {
        Ok(facets) => HttpResponse::Ok().json(facets),
        Err(error) => worker_failed("focus facet query failed", error),
    }
}

// ---------------------------------------------------------------------------
// Saved focuses (bookmarks)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, PartialEq)]
pub(crate) struct SavedFocus {
    id: i32,
    name: String,
    view: String,
    query: FocusQuery,
    position: i64,
    created_at: String,
    updated_at: String,
}

#[derive(QueryableByName)]
struct SavedFocusRow {
    #[diesel(sql_type = Integer)]
    id: i32,
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = Text)]
    view: String,
    #[diesel(sql_type = Text)]
    query_json: String,
    #[diesel(sql_type = BigInt)]
    position: i64,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    updated_at: String,
}

impl SavedFocusRow {
    fn into_saved(self) -> Option<SavedFocus> {
        let query = serde_json::from_str::<FocusQuery>(&self.query_json).ok()?;
        Some(SavedFocus {
            id: self.id,
            name: self.name,
            view: self.view,
            query,
            position: self.position,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

const SAVED_FOCUS_COLUMNS: &str = "id, name, view, query_json, CAST(position AS BIGINT) AS position, \
     CAST(created_at AS TEXT) AS created_at, CAST(updated_at AS TEXT) AS updated_at";

pub(crate) fn list_saved_focuses(
    connection: &mut SqliteConnection,
    user_id: i32,
) -> Result<Vec<SavedFocus>, diesel::result::Error> {
    Ok(diesel::sql_query(format!(
        "SELECT {SAVED_FOCUS_COLUMNS} FROM saved_focus WHERE user_id = ? \
         ORDER BY position, id LIMIT ?"
    ))
    .bind::<Integer, _>(user_id)
    .bind::<BigInt, _>(MAX_SAVED_FOCUSES)
    .load::<SavedFocusRow>(connection)?
    .into_iter()
    .filter_map(SavedFocusRow::into_saved)
    .collect())
}

fn saved_focus(
    connection: &mut SqliteConnection,
    user_id: i32,
    id: i32,
) -> Result<Option<SavedFocus>, diesel::result::Error> {
    Ok(diesel::sql_query(format!(
        "SELECT {SAVED_FOCUS_COLUMNS} FROM saved_focus WHERE user_id = ? AND id = ?"
    ))
    .bind::<Integer, _>(user_id)
    .bind::<Integer, _>(id)
    .get_result::<SavedFocusRow>(connection)
    .optional()?
    .and_then(SavedFocusRow::into_saved))
}

#[derive(Debug, PartialEq)]
pub(crate) enum SaveFocusResult {
    Saved(SavedFocus),
    CapacityReached,
    Missing,
}

pub(crate) fn create_saved_focus(
    connection: &mut SqliteConnection,
    user_id: i32,
    name: &str,
    query: &FocusQuery,
) -> Result<SaveFocusResult, BoxError> {
    let query_json = serde_json::to_string(query)?;
    let view = query.entity.as_str();
    let name = name.trim().to_string();
    Ok(connection.immediate_transaction(|connection| {
        #[derive(QueryableByName)]
        struct Counts {
            #[diesel(sql_type = BigInt)]
            count: i64,
            #[diesel(sql_type = BigInt)]
            next: i64,
        }
        let counts = diesel::sql_query(
            "SELECT COUNT(*) AS count, COALESCE(MAX(position), -1) + 1 AS next \
             FROM saved_focus WHERE user_id = ?",
        )
        .bind::<Integer, _>(user_id)
        .get_result::<Counts>(connection)?;
        if counts.count >= MAX_SAVED_FOCUSES {
            return Ok::<_, diesel::result::Error>(SaveFocusResult::CapacityReached);
        }
        diesel::sql_query(
            "INSERT INTO saved_focus(user_id, name, view, query_json, position) VALUES (?, ?, ?, ?, ?)",
        )
        .bind::<Integer, _>(user_id)
        .bind::<Text, _>(&name)
        .bind::<Text, _>(view)
        .bind::<Text, _>(&query_json)
        .bind::<BigInt, _>(counts.next)
        .execute(connection)?;
        #[derive(QueryableByName)]
        struct Inserted {
            #[diesel(sql_type = Integer)]
            id: i32,
        }
        let id = diesel::sql_query("SELECT CAST(last_insert_rowid() AS INTEGER) AS id")
            .get_result::<Inserted>(connection)?
            .id;
        Ok(match saved_focus(connection, user_id, id)? {
            Some(saved) => SaveFocusResult::Saved(saved),
            None => SaveFocusResult::Missing,
        })
    })?)
}

pub(crate) fn update_saved_focus(
    connection: &mut SqliteConnection,
    user_id: i32,
    id: i32,
    name: Option<&str>,
    query: Option<&FocusQuery>,
) -> Result<SaveFocusResult, BoxError> {
    let query_json = query.map(serde_json::to_string).transpose()?;
    let view = query.map(|query| query.entity.as_str());
    Ok(connection.immediate_transaction(|connection| {
        let updated = diesel::sql_query(
            "UPDATE saved_focus SET name = COALESCE(?, name), view = COALESCE(?, view), \
             query_json = COALESCE(?, query_json), updated_at = CURRENT_TIMESTAMP \
             WHERE user_id = ? AND id = ?",
        )
        .bind::<diesel::sql_types::Nullable<Text>, _>(name.map(str::trim))
        .bind::<diesel::sql_types::Nullable<Text>, _>(view)
        .bind::<diesel::sql_types::Nullable<Text>, _>(query_json.as_deref())
        .bind::<Integer, _>(user_id)
        .bind::<Integer, _>(id)
        .execute(connection)?;
        if updated == 0 {
            return Ok::<_, diesel::result::Error>(SaveFocusResult::Missing);
        }
        Ok(match saved_focus(connection, user_id, id)? {
            Some(saved) => SaveFocusResult::Saved(saved),
            None => SaveFocusResult::Missing,
        })
    })?)
}

/// Reorders a user's saved focuses. The ids must be exactly their set.
pub(crate) fn reorder_saved_focuses(
    connection: &mut SqliteConnection,
    user_id: i32,
    ids: &[i32],
) -> Result<bool, diesel::result::Error> {
    connection.immediate_transaction(|connection| {
        #[derive(QueryableByName)]
        struct IdRow {
            #[diesel(sql_type = Integer)]
            id: i32,
        }
        let existing = diesel::sql_query("SELECT id FROM saved_focus WHERE user_id = ?")
            .bind::<Integer, _>(user_id)
            .load::<IdRow>(connection)?
            .into_iter()
            .map(|row| row.id)
            .collect::<HashSet<_>>();
        let requested = ids.iter().copied().collect::<HashSet<_>>();
        if requested.len() != ids.len() || requested != existing {
            return Ok(false);
        }
        for (position, id) in ids.iter().enumerate() {
            diesel::sql_query("UPDATE saved_focus SET position = ? WHERE user_id = ? AND id = ?")
                .bind::<BigInt, _>(position as i64)
                .bind::<Integer, _>(user_id)
                .bind::<Integer, _>(*id)
                .execute(connection)?;
        }
        Ok(true)
    })
}

#[derive(Deserialize)]
struct CreateSavedFocus {
    name: String,
    query: FocusQuery,
}

#[derive(Deserialize)]
struct UpdateSavedFocus {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    query: Option<FocusQuery>,
}

#[derive(Deserialize)]
struct ReorderSavedFocuses {
    ids: Vec<i32>,
}

fn valid_saved_focus_name(name: &str) -> bool {
    valid_optional_text(Some(name.trim()), MAX_SAVED_FOCUS_NAME_CHARACTERS, false)
}

#[get("/saved")]
async fn saved_focuses(request: HttpRequest, pool: web::Data<DbPool>) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let pool = pool.get_ref().clone();
    match web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        Ok(list_saved_focuses(&mut connection, user_id)?)
    })
    .await
    {
        Ok(Ok(saved)) => HttpResponse::Ok().json(saved),
        Ok(Err(error)) => worker_failed("saved focus list failed", error),
        Err(error) => worker_failed("saved focus list worker failed", error),
    }
}

fn saved_focus_response(
    result: Result<Result<SaveFocusResult, BoxError>, actix_web::error::BlockingError>,
    created: bool,
) -> HttpResponse {
    match result {
        Ok(Ok(SaveFocusResult::Saved(saved))) => {
            if created {
                HttpResponse::Created().json(saved)
            } else {
                HttpResponse::Ok().json(saved)
            }
        }
        Ok(Ok(SaveFocusResult::CapacityReached)) => conflict(
            "Saved focus capacity reached.",
            "saved_focus_capacity_reached",
        ),
        Ok(Ok(SaveFocusResult::Missing)) => {
            not_found("Saved focus not found.", "saved_focus_not_found")
        }
        Ok(Err(error)) => worker_failed("saved focus write failed", error),
        Err(error) => worker_failed("saved focus worker failed", error),
    }
}

#[post("/saved")]
async fn create_saved(
    request: HttpRequest,
    body: web::Json<CreateSavedFocus>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let body = body.into_inner();
    if !valid_saved_focus_name(&body.name) {
        return bad_request(
            "Saved focus names must contain 1 to 100 characters.",
            "saved_focus_name_invalid",
        );
    }
    if let Err(error) = body.query.validate() {
        return bad_request(error.to_string(), "focus_invalid");
    }
    let pool = pool.get_ref().clone();
    let result = web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        create_saved_focus(&mut connection, user_id, &body.name, &body.query)
    })
    .await;
    saved_focus_response(result, true)
}

#[put("/saved/order")]
async fn reorder_saved(
    request: HttpRequest,
    body: web::Json<ReorderSavedFocuses>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    if body.ids.len() as i64 > MAX_SAVED_FOCUSES {
        return bad_request("Too many saved focuses.", "saved_focus_order_invalid");
    }
    let ids = body.into_inner().ids;
    let pool = pool.get_ref().clone();
    match web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        let reordered = reorder_saved_focuses(&mut connection, user_id, &ids)?;
        Ok((reordered, list_saved_focuses(&mut connection, user_id)?))
    })
    .await
    {
        Ok(Ok((true, saved))) => HttpResponse::Ok().json(saved),
        Ok(Ok((false, _))) => bad_request(
            "The order must list every saved focus exactly once.",
            "saved_focus_order_invalid",
        ),
        Ok(Err(error)) => worker_failed("saved focus reorder failed", error),
        Err(error) => worker_failed("saved focus reorder worker failed", error),
    }
}

#[patch("/saved/{id}")]
async fn update_saved(
    request: HttpRequest,
    id: web::Path<i32>,
    body: web::Json<UpdateSavedFocus>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let body = body.into_inner();
    if body
        .name
        .as_deref()
        .is_some_and(|name| !valid_saved_focus_name(name))
    {
        return bad_request(
            "Saved focus names must contain 1 to 100 characters.",
            "saved_focus_name_invalid",
        );
    }
    if let Some(Err(error)) = body.query.as_ref().map(FocusQuery::validate) {
        return bad_request(error.to_string(), "focus_invalid");
    }
    let id = id.into_inner();
    let pool = pool.get_ref().clone();
    let result = web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        update_saved_focus(
            &mut connection,
            user_id,
            id,
            body.name.as_deref(),
            body.query.as_ref(),
        )
    })
    .await;
    saved_focus_response(result, false)
}

#[delete("/saved/{id}")]
async fn delete_saved(
    request: HttpRequest,
    id: web::Path<i32>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let id = id.into_inner();
    let pool = pool.get_ref().clone();
    match web::block(move || -> Result<usize, BoxError> {
        let mut connection = pool.get()?;
        Ok(
            diesel::sql_query("DELETE FROM saved_focus WHERE user_id = ? AND id = ?")
                .bind::<Integer, _>(user_id)
                .bind::<Integer, _>(id)
                .execute(&mut connection)?,
        )
    })
    .await
    {
        Ok(Ok(0)) => not_found("Saved focus not found.", "saved_focus_not_found"),
        Ok(Ok(_)) => HttpResponse::NoContent().finish(),
        Ok(Err(error)) => worker_failed("saved focus delete failed", error),
        Err(error) => worker_failed("saved focus delete worker failed", error),
    }
}

// ---------------------------------------------------------------------------
// Smart playlists
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct SmartPlaylistInfo {
    pub query: FocusQuery,
    pub track_limit: i64,
    pub refreshed_at: String,
}

#[derive(QueryableByName)]
struct SmartPlaylistRow {
    #[diesel(sql_type = Integer)]
    playlist_id: i32,
    #[diesel(sql_type = Text)]
    query_json: String,
    #[diesel(sql_type = BigInt)]
    track_limit: i64,
    #[diesel(sql_type = Text)]
    refreshed_at: String,
}

/// Smart playlist definitions for a user's playlists, keyed by playlist id.
pub(crate) fn smart_playlists_for_user(
    connection: &mut SqliteConnection,
    user_id: i32,
) -> Result<HashMap<i32, SmartPlaylistInfo>, diesel::result::Error> {
    Ok(diesel::sql_query(
        "SELECT smart.playlist_id, smart.query_json, CAST(smart.track_limit AS BIGINT) AS track_limit, \
         CAST(smart.refreshed_at AS TEXT) AS refreshed_at \
         FROM smart_playlist smart JOIN _playlist_to_user owners ON owners.a = smart.playlist_id \
         WHERE owners.b = ?",
    )
    .bind::<Integer, _>(user_id)
    .load::<SmartPlaylistRow>(connection)?
    .into_iter()
    .filter_map(|row| {
        let query = serde_json::from_str(&row.query_json).ok()?;
        Some((
            row.playlist_id,
            SmartPlaylistInfo {
                query,
                track_limit: row.track_limit,
                refreshed_at: row.refreshed_at,
            },
        ))
    })
    .collect())
}

fn owns_playlist(
    connection: &mut SqliteConnection,
    playlist_id: i32,
    user_id: i32,
) -> Result<bool, diesel::result::Error> {
    #[derive(QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = BigInt)]
        count: i64,
    }
    Ok(
        diesel::sql_query("SELECT COUNT(*) AS count FROM _playlist_to_user WHERE a = ? AND b = ?")
            .bind::<Integer, _>(playlist_id)
            .bind::<Integer, _>(user_id)
            .get_result::<CountRow>(connection)?
            .count
            > 0,
    )
}

/// Replaces a playlist's tracks with the songs its focus matches now.
fn write_smart_tracks(
    connection: &mut SqliteConnection,
    playlist_id: i32,
    song_ids: &[String],
) -> Result<usize, diesel::result::Error> {
    diesel::sql_query("DELETE FROM _playlist_to_song WHERE a = ?")
        .bind::<Integer, _>(playlist_id)
        .execute(connection)?;
    let mut written = 0;
    for song_id in song_ids {
        written += diesel::sql_query(
            "INSERT INTO _playlist_to_song(a, b) SELECT ?, id FROM song WHERE id = ?",
        )
        .bind::<Integer, _>(playlist_id)
        .bind::<Text, _>(song_id)
        .execute(connection)?;
    }
    diesel::sql_query("UPDATE playlist SET updated_at = CURRENT_TIMESTAMP WHERE id = ?;")
        .bind::<Integer, _>(playlist_id)
        .execute(connection)?;
    diesel::sql_query(
        "UPDATE smart_playlist SET refreshed_at = CURRENT_TIMESTAMP WHERE playlist_id = ?",
    )
    .bind::<Integer, _>(playlist_id)
    .execute(connection)?;
    Ok(written)
}

#[derive(Debug, Serialize, PartialEq)]
pub(crate) struct SmartPlaylistResult {
    pub id: i32,
    pub name: String,
    pub song_count: usize,
    pub smart: SmartPlaylistInfo,
}

#[derive(Debug, PartialEq)]
pub(crate) enum SmartPlaylistOutcome {
    Written(SmartPlaylistResult),
    CapacityReached,
    Missing,
}

fn smart_result(
    connection: &mut SqliteConnection,
    user_id: i32,
    playlist_id: i32,
    song_count: usize,
) -> Result<SmartPlaylistOutcome, diesel::result::Error> {
    #[derive(QueryableByName)]
    struct NameRow {
        #[diesel(sql_type = Text)]
        name: String,
    }
    let name = diesel::sql_query("SELECT name FROM playlist WHERE id = ?")
        .bind::<Integer, _>(playlist_id)
        .get_result::<NameRow>(connection)?
        .name;
    Ok(
        match smart_playlists_for_user(connection, user_id)?.remove(&playlist_id) {
            Some(smart) => SmartPlaylistOutcome::Written(SmartPlaylistResult {
                id: playlist_id,
                name,
                song_count,
                smart,
            }),
            None => SmartPlaylistOutcome::Missing,
        },
    )
}

/// Creates a playlist whose tracks are the songs a focus matches.
pub(crate) fn create_smart_playlist(
    connection: &mut SqliteConnection,
    user_id: i32,
    name: &str,
    description: Option<&str>,
    query: &FocusQuery,
    track_limit: i64,
) -> Result<SmartPlaylistOutcome, BoxError> {
    let songs = query.as_songs();
    let song_ids = focus_ids(connection, &songs, user_id, 0, track_limit as usize)?.ids;
    let query_json = serde_json::to_string(query)?;
    Ok(connection.immediate_transaction(|connection| {
        #[derive(QueryableByName)]
        struct CountRow {
            #[diesel(sql_type = BigInt)]
            count: i64,
        }
        let owned =
            diesel::sql_query("SELECT COUNT(*) AS count FROM _playlist_to_user WHERE b = ?")
                .bind::<Integer, _>(user_id)
                .get_result::<CountRow>(connection)?
                .count;
        if owned >= MAX_PLAYLISTS {
            return Ok::<_, diesel::result::Error>(SmartPlaylistOutcome::CapacityReached);
        }
        diesel::sql_query("INSERT INTO playlist(name, description) VALUES (?, ?)")
            .bind::<Text, _>(name.trim())
            .bind::<diesel::sql_types::Nullable<Text>, _>(description)
            .execute(connection)?;
        #[derive(QueryableByName)]
        struct Inserted {
            #[diesel(sql_type = Integer)]
            id: i32,
        }
        let playlist_id = diesel::sql_query("SELECT CAST(last_insert_rowid() AS INTEGER) AS id")
            .get_result::<Inserted>(connection)?
            .id;
        diesel::sql_query("INSERT INTO _playlist_to_user(a, b) VALUES (?, ?)")
            .bind::<Integer, _>(playlist_id)
            .bind::<Integer, _>(user_id)
            .execute(connection)?;
        diesel::sql_query(
            "INSERT INTO smart_playlist(playlist_id, query_json, track_limit) VALUES (?, ?, ?)",
        )
        .bind::<Integer, _>(playlist_id)
        .bind::<Text, _>(&query_json)
        .bind::<BigInt, _>(track_limit)
        .execute(connection)?;
        let written = write_smart_tracks(connection, playlist_id, &song_ids)?;
        smart_result(connection, user_id, playlist_id, written)
    })?)
}

/// Re-runs a smart playlist's focus and replaces its tracks.
pub(crate) fn refresh_smart_playlist(
    connection: &mut SqliteConnection,
    user_id: i32,
    playlist_id: i32,
) -> Result<SmartPlaylistOutcome, BoxError> {
    if !owns_playlist(connection, playlist_id, user_id)? {
        return Ok(SmartPlaylistOutcome::Missing);
    }
    let Some(smart) = smart_playlists_for_user(connection, user_id)?.remove(&playlist_id) else {
        return Ok(SmartPlaylistOutcome::Missing);
    };
    let song_ids = focus_ids(
        connection,
        &smart.query.as_songs(),
        user_id,
        0,
        smart.track_limit as usize,
    )?
    .ids;
    Ok(connection.immediate_transaction(|connection| {
        let written = write_smart_tracks(connection, playlist_id, &song_ids)?;
        smart_result(connection, user_id, playlist_id, written)
    })?)
}

#[derive(Deserialize)]
struct CreateSmartPlaylist {
    name: String,
    #[serde(default)]
    description: Option<String>,
    query: FocusQuery,
    #[serde(default)]
    limit: Option<i64>,
}

fn smart_playlist_response(
    result: Result<Result<SmartPlaylistOutcome, BoxError>, actix_web::error::BlockingError>,
    created: bool,
) -> HttpResponse {
    match result {
        Ok(Ok(SmartPlaylistOutcome::Written(playlist))) => {
            if created {
                HttpResponse::Created().json(playlist)
            } else {
                HttpResponse::Ok().json(playlist)
            }
        }
        Ok(Ok(SmartPlaylistOutcome::CapacityReached)) => {
            conflict("Playlist capacity reached.", "playlist_capacity_reached")
        }
        Ok(Ok(SmartPlaylistOutcome::Missing)) => {
            not_found("Smart playlist not found.", "smart_playlist_not_found")
        }
        Ok(Err(error)) => worker_failed("smart playlist write failed", error),
        Err(error) => worker_failed("smart playlist worker failed", error),
    }
}

#[post("/playlists")]
async fn create_smart(
    request: HttpRequest,
    body: web::Json<CreateSmartPlaylist>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let body = body.into_inner();
    let name = body.name.trim().to_string();
    if !valid_optional_text(Some(&name), MAX_PLAYLIST_NAME_CHARACTERS, false) {
        return bad_request(
            "Playlist name must contain 1 to 200 characters.",
            "playlist_name_invalid",
        );
    }
    if !valid_optional_text(
        body.description.as_deref(),
        MAX_PLAYLIST_DESCRIPTION_CHARACTERS,
        true,
    ) {
        return bad_request(
            "Playlist description is too long.",
            "playlist_description_invalid",
        );
    }
    if let Err(error) = body.query.validate() {
        return bad_request(error.to_string(), "focus_invalid");
    }
    let track_limit = body.limit.unwrap_or(MAX_TRACKS_PER_PLAYLIST);
    if !(1..=MAX_TRACKS_PER_PLAYLIST.min(MAX_RESOLVED_IDS as i64)).contains(&track_limit) {
        return bad_request(
            "Smart playlists hold 1 to 5000 tracks.",
            "smart_playlist_limit_invalid",
        );
    }
    let pool = pool.get_ref().clone();
    let result = web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        create_smart_playlist(
            &mut connection,
            user_id,
            &name,
            body.description.as_deref(),
            &body.query,
            track_limit,
        )
    })
    .await;
    smart_playlist_response(result, true)
}

#[post("/playlists/{id}/refresh")]
async fn refresh_smart(
    request: HttpRequest,
    id: web::Path<i32>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let id = id.into_inner();
    let pool = pool.get_ref().clone();
    let result = web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        refresh_smart_playlist(&mut connection, user_id, id)
    })
    .await;
    smart_playlist_response(result, false)
}

/// Turns a smart playlist into an ordinary one that keeps its current tracks.
#[delete("/playlists/{id}/smart")]
async fn detach_smart(
    request: HttpRequest,
    id: web::Path<i32>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let id = id.into_inner();
    let pool = pool.get_ref().clone();
    match web::block(move || -> Result<usize, BoxError> {
        let mut connection = pool.get()?;
        if !owns_playlist(&mut connection, id, user_id)? {
            return Ok(0);
        }
        Ok(
            diesel::sql_query("DELETE FROM smart_playlist WHERE playlist_id = ?")
                .bind::<Integer, _>(id)
                .execute(&mut connection)?,
        )
    })
    .await
    {
        Ok(Ok(0)) => not_found("Smart playlist not found.", "smart_playlist_not_found"),
        Ok(Ok(_)) => HttpResponse::NoContent().finish(),
        Ok(Err(error)) => worker_failed("smart playlist detach failed", error),
        Err(error) => worker_failed("smart playlist detach worker failed", error),
    }
}

// ---------------------------------------------------------------------------
// Tags
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, PartialEq, QueryableByName)]
pub(crate) struct UserTag {
    #[diesel(sql_type = Integer)]
    id: i32,
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = BigInt)]
    item_count: i64,
    #[diesel(sql_type = Text)]
    created_at: String,
}

fn normalize_tag_name(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub(crate) fn list_tags(
    connection: &mut SqliteConnection,
    user_id: i32,
) -> Result<Vec<UserTag>, diesel::result::Error> {
    diesel::sql_query(
        "SELECT g.id, g.name, (SELECT COUNT(*) FROM user_tag_item i WHERE i.tag_id = g.id) AS item_count, \
         CAST(g.created_at AS TEXT) AS created_at FROM user_tag g WHERE g.user_id = ? \
         ORDER BY g.name COLLATE NOCASE, g.id LIMIT ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<BigInt, _>(MAX_TAGS)
    .load::<UserTag>(connection)
}

#[derive(Debug, PartialEq)]
pub(crate) enum TagWrite {
    Written(UserTag),
    Duplicate,
    CapacityReached,
    Missing,
}

fn tag_by_id(
    connection: &mut SqliteConnection,
    user_id: i32,
    tag_id: i32,
) -> Result<Option<UserTag>, diesel::result::Error> {
    Ok(list_tags(connection, user_id)?
        .into_iter()
        .find(|tag| tag.id == tag_id))
}

pub(crate) fn create_tag(
    connection: &mut SqliteConnection,
    user_id: i32,
    name: &str,
) -> Result<TagWrite, diesel::result::Error> {
    let display = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let normalized = normalize_tag_name(name);
    connection.immediate_transaction(|connection| {
        #[derive(QueryableByName)]
        struct Counts {
            #[diesel(sql_type = BigInt)]
            count: i64,
            #[diesel(sql_type = BigInt)]
            duplicate: i64,
        }
        let counts = diesel::sql_query(
            "SELECT COUNT(*) AS count, COALESCE(SUM(normalized_name = ?), 0) AS duplicate \
             FROM user_tag WHERE user_id = ?",
        )
        .bind::<Text, _>(&normalized)
        .bind::<Integer, _>(user_id)
        .get_result::<Counts>(connection)?;
        if counts.duplicate > 0 {
            return Ok(TagWrite::Duplicate);
        }
        if counts.count >= MAX_TAGS {
            return Ok(TagWrite::CapacityReached);
        }
        diesel::sql_query("INSERT INTO user_tag(user_id, name, normalized_name) VALUES (?, ?, ?)")
            .bind::<Integer, _>(user_id)
            .bind::<Text, _>(&display)
            .bind::<Text, _>(&normalized)
            .execute(connection)?;
        #[derive(QueryableByName)]
        struct Inserted {
            #[diesel(sql_type = Integer)]
            id: i32,
        }
        let id = diesel::sql_query("SELECT CAST(last_insert_rowid() AS INTEGER) AS id")
            .get_result::<Inserted>(connection)?
            .id;
        Ok(match tag_by_id(connection, user_id, id)? {
            Some(tag) => TagWrite::Written(tag),
            None => TagWrite::Missing,
        })
    })
}

pub(crate) fn rename_tag(
    connection: &mut SqliteConnection,
    user_id: i32,
    tag_id: i32,
    name: &str,
) -> Result<TagWrite, diesel::result::Error> {
    let display = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let normalized = normalize_tag_name(name);
    connection.immediate_transaction(|connection| {
        #[derive(QueryableByName)]
        struct CountRow {
            #[diesel(sql_type = BigInt)]
            count: i64,
        }
        let duplicate = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM user_tag WHERE user_id = ? AND normalized_name = ? AND id <> ?",
        )
        .bind::<Integer, _>(user_id)
        .bind::<Text, _>(&normalized)
        .bind::<Integer, _>(tag_id)
        .get_result::<CountRow>(connection)?
        .count;
        if duplicate > 0 {
            return Ok(TagWrite::Duplicate);
        }
        let updated = diesel::sql_query(
            "UPDATE user_tag SET name = ?, normalized_name = ?, updated_at = CURRENT_TIMESTAMP \
             WHERE user_id = ? AND id = ?",
        )
        .bind::<Text, _>(&display)
        .bind::<Text, _>(&normalized)
        .bind::<Integer, _>(user_id)
        .bind::<Integer, _>(tag_id)
        .execute(connection)?;
        if updated == 0 {
            return Ok(TagWrite::Missing);
        }
        Ok(match tag_by_id(connection, user_id, tag_id)? {
            Some(tag) => TagWrite::Written(tag),
            None => TagWrite::Missing,
        })
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TaggedEntity {
    Track,
    Album,
    Artist,
}

impl TaggedEntity {
    fn as_str(self) -> &'static str {
        match self {
            Self::Track => "track",
            Self::Album => "album",
            Self::Artist => "artist",
        }
    }

    fn table(self) -> &'static str {
        match self {
            Self::Track => "track_entity",
            Self::Album => "album_entity",
            Self::Artist => "artist_entity",
        }
    }
}

/// Adds or removes catalog entities on a tag. Unknown ids are ignored.
/// Returns `None` when the tag does not belong to the user.
pub(crate) fn change_tag_items(
    connection: &mut SqliteConnection,
    user_id: i32,
    tag_id: i32,
    entity: TaggedEntity,
    ids: &[String],
    add: bool,
) -> Result<Option<usize>, diesel::result::Error> {
    connection.immediate_transaction(|connection| {
        if tag_by_id(connection, user_id, tag_id)?.is_none() {
            return Ok(None);
        }
        let mut changed = 0;
        for id in ids {
            changed += if add {
                diesel::sql_query(format!(
                    "INSERT INTO user_tag_item(tag_id, entity_type, entity_id) \
                     SELECT ?, ?, id FROM {} WHERE id = ? \
                     ON CONFLICT(tag_id, entity_type, entity_id) DO NOTHING",
                    entity.table()
                ))
                .bind::<Integer, _>(tag_id)
                .bind::<Text, _>(entity.as_str())
                .bind::<Text, _>(id)
                .execute(connection)?
            } else {
                diesel::sql_query(
                    "DELETE FROM user_tag_item WHERE tag_id = ? AND entity_type = ? AND entity_id = ?",
                )
                .bind::<Integer, _>(tag_id)
                .bind::<Text, _>(entity.as_str())
                .bind::<Text, _>(id)
                .execute(connection)?
            };
        }
        if changed > 0 {
            diesel::sql_query("UPDATE user_tag SET updated_at = CURRENT_TIMESTAMP WHERE id = ?")
                .bind::<Integer, _>(tag_id)
                .execute(connection)?;
        }
        Ok(Some(changed))
    })
}

pub(crate) fn tags_for_entity(
    connection: &mut SqliteConnection,
    user_id: i32,
    entity: TaggedEntity,
    entity_id: &str,
) -> Result<Vec<i32>, diesel::result::Error> {
    #[derive(QueryableByName)]
    struct IdRow {
        #[diesel(sql_type = Integer)]
        id: i32,
    }
    Ok(diesel::sql_query(
        "SELECT g.id FROM user_tag g JOIN user_tag_item i ON i.tag_id = g.id \
         WHERE g.user_id = ? AND i.entity_type = ? AND i.entity_id = ? ORDER BY g.id",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(entity.as_str())
    .bind::<Text, _>(entity_id)
    .load::<IdRow>(connection)?
    .into_iter()
    .map(|row| row.id)
    .collect())
}

#[derive(Deserialize)]
struct TagName {
    name: String,
}

#[derive(Deserialize)]
struct TagItems {
    entity_type: TaggedEntity,
    entity_ids: Vec<String>,
}

#[derive(Deserialize)]
struct TagLookup {
    entity_type: TaggedEntity,
    entity_id: String,
}

fn valid_tag_name(name: &str) -> bool {
    valid_optional_text(Some(name.trim()), MAX_TAG_NAME_CHARACTERS, false)
}

fn tag_response(
    result: Result<Result<TagWrite, BoxError>, actix_web::error::BlockingError>,
    created: bool,
) -> HttpResponse {
    match result {
        Ok(Ok(TagWrite::Written(tag))) => {
            if created {
                HttpResponse::Created().json(tag)
            } else {
                HttpResponse::Ok().json(tag)
            }
        }
        Ok(Ok(TagWrite::Duplicate)) => conflict("A tag with this name exists.", "tag_exists"),
        Ok(Ok(TagWrite::CapacityReached)) => {
            conflict("Tag capacity reached.", "tag_capacity_reached")
        }
        Ok(Ok(TagWrite::Missing)) => not_found("Tag not found.", "tag_not_found"),
        Ok(Err(error)) => worker_failed("tag write failed", error),
        Err(error) => worker_failed("tag worker failed", error),
    }
}

#[get("")]
async fn tags(request: HttpRequest, pool: web::Data<DbPool>) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let pool = pool.get_ref().clone();
    match web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        Ok(list_tags(&mut connection, user_id)?)
    })
    .await
    {
        Ok(Ok(tags)) => HttpResponse::Ok().json(tags),
        Ok(Err(error)) => worker_failed("tag list failed", error),
        Err(error) => worker_failed("tag list worker failed", error),
    }
}

#[post("")]
async fn create_tag_route(
    request: HttpRequest,
    body: web::Json<TagName>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    if !valid_tag_name(&body.name) {
        return bad_request(
            "Tag names must contain 1 to 100 characters.",
            "tag_name_invalid",
        );
    }
    let name = body.into_inner().name;
    let pool = pool.get_ref().clone();
    let result = web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        Ok(create_tag(&mut connection, user_id, &name)?)
    })
    .await;
    tag_response(result, true)
}

#[get("/assigned")]
async fn assigned_tags(
    request: HttpRequest,
    query: web::Query<TagLookup>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let lookup = query.into_inner();
    if lookup.entity_id.is_empty() || lookup.entity_id.chars().count() > MAX_ENTITY_ID_CHARACTERS {
        return bad_request("Entity identifier is invalid.", "tag_entity_invalid");
    }
    let pool = pool.get_ref().clone();
    match web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        Ok(tags_for_entity(
            &mut connection,
            user_id,
            lookup.entity_type,
            &lookup.entity_id,
        )?)
    })
    .await
    {
        Ok(Ok(ids)) => HttpResponse::Ok().json(ids),
        Ok(Err(error)) => worker_failed("tag lookup failed", error),
        Err(error) => worker_failed("tag lookup worker failed", error),
    }
}

#[patch("/{id}")]
async fn rename_tag_route(
    request: HttpRequest,
    id: web::Path<i32>,
    body: web::Json<TagName>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    if !valid_tag_name(&body.name) {
        return bad_request(
            "Tag names must contain 1 to 100 characters.",
            "tag_name_invalid",
        );
    }
    let id = id.into_inner();
    let name = body.into_inner().name;
    let pool = pool.get_ref().clone();
    let result = web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        Ok(rename_tag(&mut connection, user_id, id, &name)?)
    })
    .await;
    tag_response(result, false)
}

#[delete("/{id}")]
async fn delete_tag_route(
    request: HttpRequest,
    id: web::Path<i32>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let id = id.into_inner();
    let pool = pool.get_ref().clone();
    match web::block(move || -> Result<usize, BoxError> {
        let mut connection = pool.get()?;
        Ok(
            diesel::sql_query("DELETE FROM user_tag WHERE user_id = ? AND id = ?")
                .bind::<Integer, _>(user_id)
                .bind::<Integer, _>(id)
                .execute(&mut connection)?,
        )
    })
    .await
    {
        Ok(Ok(0)) => not_found("Tag not found.", "tag_not_found"),
        Ok(Ok(_)) => HttpResponse::NoContent().finish(),
        Ok(Err(error)) => worker_failed("tag delete failed", error),
        Err(error) => worker_failed("tag delete worker failed", error),
    }
}

async fn change_items(
    request: HttpRequest,
    id: i32,
    body: TagItems,
    pool: web::Data<DbPool>,
    add: bool,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    if body.entity_ids.is_empty()
        || body.entity_ids.len() > MAX_TAG_BATCH
        || body
            .entity_ids
            .iter()
            .any(|id| id.is_empty() || id.chars().count() > MAX_ENTITY_ID_CHARACTERS)
    {
        return bad_request(
            "Tag items must list 1 to 500 valid identifiers.",
            "tag_items_invalid",
        );
    }
    let pool = pool.get_ref().clone();
    match web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        Ok(change_tag_items(
            &mut connection,
            user_id,
            id,
            body.entity_type,
            &body.entity_ids,
            add,
        )?)
    })
    .await
    {
        Ok(Ok(Some(changed))) => HttpResponse::Ok().json(serde_json::json!({ "changed": changed })),
        Ok(Ok(None)) => not_found("Tag not found.", "tag_not_found"),
        Ok(Err(error)) => worker_failed("tag item change failed", error),
        Err(error) => worker_failed("tag item worker failed", error),
    }
}

#[post("/{id}/items")]
async fn add_tag_items(
    request: HttpRequest,
    id: web::Path<i32>,
    body: web::Json<TagItems>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    change_items(request, id.into_inner(), body.into_inner(), pool, true).await
}

#[post("/{id}/items/remove")]
async fn remove_tag_items(
    request: HttpRequest,
    id: web::Path<i32>,
    body: web::Json<TagItems>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    change_items(request, id.into_inner(), body.into_inner(), pool, false).await
}

/// Focus routes, mounted under `/library/focus`.
pub fn configure_focus(cfg: &mut web::ServiceConfig) {
    cfg.service(focus_page)
        .service(focus_facets)
        .service(saved_focuses)
        .service(create_saved)
        .service(reorder_saved)
        .service(update_saved)
        .service(delete_saved)
        .service(create_smart)
        .service(refresh_smart)
        .service(detach_smart);
}

/// Tag routes, mounted under `/library/tags`.
pub fn configure_tags(cfg: &mut web::ServiceConfig) {
    cfg.service(tags)
        .service(create_tag_route)
        .service(assigned_tags)
        .service(rename_tag_route)
        .service(delete_tag_route)
        .service(add_tag_items)
        .service(remove_tag_items);
}

#[cfg(test)]
mod tests {
    use diesel::Connection;
    use diesel::connection::SimpleConnection;
    use diesel_migrations::MigrationHarness;

    use super::*;
    use crate::library::focus::{Criterion, FocusEntity, FocusFilter};

    fn database() -> SqliteConnection {
        let mut connection = SqliteConnection::establish(":memory:").expect("focus api database");
        connection
            .run_pending_migrations(crate::persistence::connection::MIGRATIONS)
            .expect("migrations");
        connection
            .batch_execute(
                "PRAGMA foreign_keys = ON;
                 INSERT INTO user(id, username, password) VALUES (1, 'one', 'hash'), (2, 'two', 'hash');
                 INSERT INTO album_entity(id, title, normalized_title, first_release_date) VALUES
                   ('al-1', 'First', 'first', '1999'), ('al-2', 'Second', 'second', '2005');
                 INSERT INTO artist_entity(id, name, normalized_name) VALUES ('ar-1', 'Artist', 'artist');
                 INSERT INTO album_artist(album_id, artist_id, role) VALUES ('al-1', 'ar-1', 'primary'), ('al-2', 'ar-1', 'primary');
                 INSERT INTO track_entity(id, title, normalized_title, album_id, track_number) VALUES
                   ('t-1', 'One', 'one', 'al-1', 1), ('t-2', 'Two', 'two', 'al-1', 2), ('t-3', 'Three', 'three', 'al-2', 1);
                 INSERT INTO track_artist(track_id, artist_id, role) VALUES ('t-1', 'ar-1', 'primary'), ('t-2', 'ar-1', 'primary'), ('t-3', 'ar-1', 'primary');
                 INSERT INTO song(id) VALUES ('t-1'), ('t-2'), ('t-3');",
            )
            .expect("fixture");
        connection
    }

    fn nineties() -> FocusQuery {
        FocusQuery {
            entity: FocusEntity::Albums,
            filters: vec![FocusFilter {
                criterion: Criterion::Year {
                    from: Some(1990),
                    to: Some(1999),
                },
                exclude: false,
            }],
            ..FocusQuery::default()
        }
    }

    #[test]
    fn saved_focuses_are_per_user_ordered_and_round_trip() {
        let mut connection = database();
        let SaveFocusResult::Saved(first) =
            create_saved_focus(&mut connection, 1, "  90s albums ", &nineties()).expect("save")
        else {
            panic!("first focus should save");
        };
        assert_eq!(first.name, "90s albums");
        assert_eq!(first.view, "albums");
        assert_eq!(first.query, nineties());
        let SaveFocusResult::Saved(second) =
            create_saved_focus(&mut connection, 1, "Everything", &FocusQuery::default())
                .expect("save")
        else {
            panic!("second focus should save");
        };
        assert_eq!(second.position, first.position + 1);
        create_saved_focus(&mut connection, 2, "Theirs", &FocusQuery::default()).expect("save");

        let mine = list_saved_focuses(&mut connection, 1).expect("list");
        assert_eq!(
            mine.iter().map(|saved| saved.id).collect::<Vec<_>>(),
            [first.id, second.id]
        );

        assert!(!reorder_saved_focuses(&mut connection, 1, &[second.id]).expect("partial"));
        assert!(
            reorder_saved_focuses(&mut connection, 1, &[second.id, first.id]).expect("reorder")
        );
        let mine = list_saved_focuses(&mut connection, 1).expect("list");
        assert_eq!(mine[0].id, second.id);

        let SaveFocusResult::Saved(renamed) =
            update_saved_focus(&mut connection, 1, first.id, Some("Nineties"), None)
                .expect("rename")
        else {
            panic!("rename should succeed");
        };
        assert_eq!(renamed.name, "Nineties");
        assert_eq!(renamed.query, nineties());
        assert_eq!(
            update_saved_focus(&mut connection, 2, first.id, Some("Stolen"), None)
                .expect("foreign update"),
            SaveFocusResult::Missing
        );
    }

    #[test]
    fn smart_playlists_materialize_and_refresh_focused_songs() {
        let mut connection = database();
        let SmartPlaylistOutcome::Written(created) = create_smart_playlist(
            &mut connection,
            1,
            "Nineties",
            Some("Albums from the 1990s"),
            &nineties(),
            MAX_TRACKS_PER_PLAYLIST,
        )
        .expect("create smart playlist") else {
            panic!("smart playlist should be created");
        };
        assert_eq!(created.song_count, 2);
        assert_eq!(created.smart.query, nineties());
        assert!(owns_playlist(&mut connection, created.id, 1).expect("owner"));

        #[derive(QueryableByName)]
        struct SongRow {
            #[diesel(sql_type = Text)]
            b: String,
        }
        let songs = |connection: &mut SqliteConnection| {
            diesel::sql_query("SELECT b FROM _playlist_to_song WHERE a = ? ORDER BY position")
                .bind::<Integer, _>(created.id)
                .load::<SongRow>(connection)
                .expect("songs")
                .into_iter()
                .map(|row| row.b)
                .collect::<Vec<_>>()
        };
        assert_eq!(songs(&mut connection), ["t-1", "t-2"]);

        // The library changes: the second album is re-dated into the 1990s.
        connection
            .batch_execute("UPDATE album_entity SET first_release_date = '1995' WHERE id = 'al-2'")
            .expect("re-date");
        let SmartPlaylistOutcome::Written(refreshed) =
            refresh_smart_playlist(&mut connection, 1, created.id).expect("refresh")
        else {
            panic!("refresh should succeed");
        };
        assert_eq!(refreshed.song_count, 3);
        assert_eq!(songs(&mut connection).len(), 3);
        assert_eq!(
            refresh_smart_playlist(&mut connection, 2, created.id).expect("foreign refresh"),
            SmartPlaylistOutcome::Missing
        );
        assert_eq!(
            smart_playlists_for_user(&mut connection, 1)
                .expect("smart list")
                .len(),
            1
        );
        assert!(
            smart_playlists_for_user(&mut connection, 2)
                .expect("smart list")
                .is_empty()
        );
    }

    #[test]
    fn tags_are_unique_per_user_and_only_hold_catalog_entities() {
        let mut connection = database();
        let TagWrite::Written(tag) =
            create_tag(&mut connection, 1, "  Late   Night ").expect("tag")
        else {
            panic!("tag should be created");
        };
        assert_eq!(tag.name, "Late Night");
        assert_eq!(
            create_tag(&mut connection, 1, "late night").expect("duplicate"),
            TagWrite::Duplicate
        );
        assert!(matches!(
            create_tag(&mut connection, 2, "Late Night").expect("other user"),
            TagWrite::Written(_)
        ));

        let changed = change_tag_items(
            &mut connection,
            1,
            tag.id,
            TaggedEntity::Album,
            &["al-1".into(), "missing".into(), "al-1".into()],
            true,
        )
        .expect("tag albums");
        assert_eq!(changed, Some(1));
        assert_eq!(
            change_tag_items(
                &mut connection,
                2,
                tag.id,
                TaggedEntity::Album,
                &["al-2".into()],
                true
            )
            .expect("foreign tag"),
            None
        );
        assert_eq!(
            tags_for_entity(&mut connection, 1, TaggedEntity::Album, "al-1").expect("assigned"),
            [tag.id]
        );
        assert_eq!(
            list_tags(&mut connection, 1).expect("tags")[0].item_count,
            1
        );

        let TagWrite::Written(renamed) =
            rename_tag(&mut connection, 1, tag.id, "After Hours").expect("rename")
        else {
            panic!("rename should succeed");
        };
        assert_eq!(renamed.name, "After Hours");

        assert_eq!(
            change_tag_items(
                &mut connection,
                1,
                tag.id,
                TaggedEntity::Album,
                &["al-1".into()],
                false
            )
            .expect("untag"),
            Some(1)
        );

        // Deleting the user removes their tags and items.
        connection
            .batch_execute("DELETE FROM user WHERE id = 1")
            .expect("delete user");
        assert!(list_tags(&mut connection, 1).expect("tags").is_empty());
    }
}
