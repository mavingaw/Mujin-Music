use std::collections::HashSet;

use actix_web::{HttpRequest, HttpResponse, get, patch, post, put, web};
use diesel::deserialize::QueryableByName;
use diesel::prelude::*;
use diesel::sql_types::{Double, Integer, Text};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::auth::authenticated_user_id;
use crate::library::state::LibraryCache;
use crate::library::state::{LibraryLifecycle, library_unavailable_response};
use crate::persistence::connection::DbPool;
use crate::recommendation::{RadioSeed, RankedCandidate, catalog_genre, recommend_radio};

const DEFAULT_GENERATED_ITEMS: usize = 20;
const MAX_QUEUE_ITEMS: usize = 100;
const MAX_PERSISTED_QUEUES_PER_USER: i64 = 100;
const MAX_QUEUE_SOURCE_CHARACTERS: usize = 64;
const MAX_QUEUE_SONG_ID_CHARACTERS: usize = 256;
const MAX_SEED_GENRE_CHARACTERS: usize = 128;
const DEFAULT_RADIO_PICKS: usize = 5;
const MAX_RADIO_PICKS: usize = 20;
const MAX_RADIO_PICK_EXCLUSIONS: usize = 500;

/// One of these may name what a Radio station is built around.
#[derive(Default, Deserialize)]
struct SeedFields {
    seed_song_id: Option<String>,
    seed_album_id: Option<String>,
    seed_artist_id: Option<String>,
    seed_genre: Option<String>,
}

#[derive(Debug, PartialEq)]
enum SeedError {
    Invalid,
    Missing,
}

impl SeedFields {
    /// Parses the request's seed without consulting the catalog.
    fn parse(&self) -> Result<Option<RadioSeed>, SeedError> {
        let fields = [
            ("song", &self.seed_song_id),
            ("album", &self.seed_album_id),
            ("artist", &self.seed_artist_id),
            ("genre", &self.seed_genre),
        ];
        let mut provided = fields
            .iter()
            .filter_map(|(kind, value)| value.as_deref().map(|value| (*kind, value)));
        let Some((kind, value)) = provided.next() else {
            return Ok(None);
        };
        if provided.next().is_some() {
            return Err(SeedError::Invalid);
        }
        let valid = if kind == "genre" {
            !value.trim().is_empty() && value.chars().count() <= MAX_SEED_GENRE_CHARACTERS
        } else {
            valid_song_id(value)
        };
        if !valid {
            return Err(SeedError::Invalid);
        }
        Ok(RadioSeed::from_parts(kind, value))
    }

    /// Parses the seed and resolves it against the catalog. Genres are
    /// normalised to the catalog's spelling.
    fn resolve(&self, cache: &LibraryCache) -> Result<Option<RadioSeed>, SeedError> {
        let Some(seed) = self.parse()? else {
            return Ok(None);
        };
        let resolved = match &seed {
            RadioSeed::Song(id) => cache.song(id).map(|_| seed.clone()),
            RadioSeed::Album(id) => cache.album(id).map(|_| seed.clone()),
            RadioSeed::Artist(id) => cache.artist(id).map(|_| seed.clone()),
            RadioSeed::Genre(name) => {
                catalog_genre(cache, name).map(|genre| RadioSeed::Genre(genre.to_owned()))
            }
        };
        resolved.map(Some).ok_or(SeedError::Missing)
    }
}

#[derive(Debug, PartialEq, Serialize)]
struct QueueSeed {
    kind: &'static str,
    id: String,
    name: String,
}

fn queue_seed(seed: &RadioSeed, cache: &LibraryCache) -> Option<QueueSeed> {
    Some(QueueSeed {
        kind: seed.kind(),
        id: seed.id().to_owned(),
        name: seed.display_name(cache)?,
    })
}

#[derive(Deserialize)]
struct CreateQueueRequest {
    #[serde(flatten)]
    seed: SeedFields,
    #[serde(default)]
    explicit_song_ids: Vec<String>,
    #[serde(default)]
    exclude_song_ids: Vec<String>,
    generated_items: Option<usize>,
    source: Option<String>,
}

#[derive(Serialize)]
struct QueueSong {
    id: String,
    name: String,
    artist: String,
    contributing_artists: Vec<String>,
    contributing_artist_ids: Vec<String>,
    track_number: u16,
    duration: f64,
    album_object: QueueAlbum,
    artist_object: QueueArtist,
    origin: String,
    queue_position: i32,
    /// Technical properties for signal-path displays; omitted when unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    audio: Option<crate::domain::AudioProperties>,
}

#[derive(Serialize)]
struct QueueAlbum {
    id: String,
    name: String,
    cover_url: String,
}

#[derive(Serialize)]
struct QueueArtist {
    id: String,
    name: String,
    icon_url: String,
}

#[derive(Serialize)]
struct QueueResponse {
    id: String,
    revision: i32,
    current_position: i32,
    /// Why the queue exists (`radio`, `manual_selection`, ...).
    source: String,
    /// What a Radio queue was built around; omitted for profile-only Radio.
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<QueueSeed>,
    items: Vec<QueueSong>,
}

#[derive(QueryableByName)]
struct QueueMetaRow {
    #[diesel(sql_type = Integer)]
    revision: i32,
    #[diesel(sql_type = Integer)]
    current_position: i32,
    #[diesel(sql_type = Text)]
    source: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<Text>)]
    seed_kind: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Nullable<Text>)]
    seed_id: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Nullable<Text>)]
    seed_song_id: Option<String>,
}

impl QueueMetaRow {
    fn seed(&self) -> Option<RadioSeed> {
        match (self.seed_kind.as_deref(), self.seed_id.as_deref()) {
            (Some(kind), Some(id)) => RadioSeed::from_parts(kind, id),
            // Queues created before seeds were recorded only know the song.
            _ => self
                .seed_song_id
                .as_ref()
                .map(|id| RadioSeed::Song(id.clone())),
        }
    }
}

#[derive(Deserialize)]
struct UpdateQueueRequest {
    current_position: i32,
    revision: i32,
}

#[derive(QueryableByName)]
struct QueueItemRow {
    #[diesel(sql_type = Integer)]
    position: i32,
    #[diesel(sql_type = Text)]
    song_id: String,
    #[diesel(sql_type = Text)]
    origin: String,
    #[diesel(sql_type = Double)]
    score: f64,
    #[diesel(sql_type = Text)]
    reason: String,
}

#[derive(Debug, PartialEq, QueryableByName)]
struct QueueStateRow {
    #[diesel(sql_type = Integer)]
    revision: i32,
    #[diesel(sql_type = Integer)]
    current_position: i32,
}

#[derive(Debug, PartialEq)]
enum UpdateQueueResult {
    Updated(QueueStateRow),
    Missing,
    InvalidPosition,
    Conflict(QueueStateRow),
}

fn update_queue_position(
    connection: &mut diesel::sqlite::SqliteConnection,
    queue_id: &str,
    user_id: i32,
    current_position: i32,
    revision: i32,
) -> Result<UpdateQueueResult, diesel::result::Error> {
    connection.transaction(|connection| {
        let state = diesel::sql_query(
            "SELECT revision, current_position FROM playback_queue
             WHERE id = ? AND user_id = ?",
        )
        .bind::<Text, _>(queue_id)
        .bind::<Integer, _>(user_id)
        .get_result::<QueueStateRow>(connection)
        .optional()?;
        let Some(state) = state else {
            return Ok(UpdateQueueResult::Missing);
        };
        let item_count = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM playback_queue_item WHERE queue_id = ?",
        )
        .bind::<Text, _>(queue_id)
        .get_result::<QueueCountRow>(connection)?
        .count;
        if current_position < 0 || i64::from(current_position) >= item_count {
            return Ok(UpdateQueueResult::InvalidPosition);
        }
        if revision != state.revision {
            return Ok(UpdateQueueResult::Conflict(state));
        }
        diesel::sql_query(
            "UPDATE playback_queue
             SET current_position = ?, revision = revision + 1, updated_at = CURRENT_TIMESTAMP
             WHERE id = ? AND user_id = ? AND revision = ?",
        )
        .bind::<Integer, _>(current_position)
        .bind::<Text, _>(queue_id)
        .bind::<Integer, _>(user_id)
        .bind::<Integer, _>(revision)
        .execute(connection)?;
        Ok(UpdateQueueResult::Updated(QueueStateRow {
            revision: revision + 1,
            current_position,
        }))
    })
}

#[derive(QueryableByName)]
struct QueueCountRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    count: i64,
}

fn hydrate_queue_item(
    row: QueueItemRow,
    cache: &crate::library::state::LibraryCache,
) -> Option<QueueSong> {
    let song = cache.song(&row.song_id)?;
    let (artist_id, album_id) = cache.song_map.get(&row.song_id)?;
    let artist = cache.artist(artist_id)?;
    let album = cache.album(album_id)?;
    Some(QueueSong {
        id: song.id.clone(),
        name: song.name.clone(),
        artist: song.artist.clone(),
        contributing_artists: song.contributing_artists.clone(),
        contributing_artist_ids: song.contributing_artist_ids.clone(),
        track_number: song.track_number,
        duration: song.duration,
        album_object: QueueAlbum {
            id: album.id.clone(),
            name: album.name.clone(),
            cover_url: album.cover_url.clone(),
        },
        artist_object: QueueArtist {
            id: artist.id.clone(),
            name: artist.name.clone(),
            icon_url: artist.icon_url.clone(),
        },
        origin: row.origin,
        queue_position: row.position,
        audio: song.audio.clone(),
    })
}

fn valid_song_id(value: &str) -> bool {
    !value.is_empty() && value.chars().count() <= MAX_QUEUE_SONG_ID_CHARACTERS
}

fn prune_old_queues(
    connection: &mut diesel::sqlite::SqliteConnection,
    user_id: i32,
    protected_queue_id: &str,
) -> Result<usize, diesel::result::Error> {
    diesel::sql_query(
        "DELETE FROM playback_queue
         WHERE user_id = ? AND id <> ? AND id IN (
           SELECT id FROM playback_queue
           WHERE user_id = ? AND id <> ?
           ORDER BY updated_at DESC, created_at DESC, id DESC
           LIMIT -1 OFFSET ?
         )",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(protected_queue_id)
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(protected_queue_id)
    .bind::<diesel::sql_types::BigInt, _>(MAX_PERSISTED_QUEUES_PER_USER - 1)
    .execute(connection)
}

#[post("")]
async fn create_queue(
    request: HttpRequest,
    form: web::Json<CreateQueueRequest>,
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
    if form.explicit_song_ids.len() > MAX_QUEUE_ITEMS
        || form.exclude_song_ids.len() > MAX_QUEUE_ITEMS
        || form.seed.parse().is_err()
        || form
            .explicit_song_ids
            .iter()
            .chain(&form.exclude_song_ids)
            .any(|value| !valid_song_id(value))
        || form.source.as_deref().is_some_and(|source| {
            source.trim().is_empty() || source.chars().count() > MAX_QUEUE_SOURCE_CHARACTERS
        })
    {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "invalid_queue_request"
        }));
    }
    let Ok(seed) = form.seed.resolve(cache.as_ref()) else {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "invalid_queue_seed"
        }));
    };

    let generated_limit = form
        .generated_items
        .unwrap_or(DEFAULT_GENERATED_ITEMS)
        .min(MAX_QUEUE_ITEMS);
    let ranked = if generated_limit == 0 {
        Vec::new()
    } else {
        let exclude = form
            .exclude_song_ids
            .iter()
            .chain(&form.explicit_song_ids)
            .take(MAX_QUEUE_ITEMS * 2)
            .cloned()
            .collect::<HashSet<_>>();
        match radio_candidates(
            user_id,
            seed.clone(),
            exclude,
            cache.clone(),
            pool.get_ref().clone(),
            generated_limit,
        )
        .await
        {
            Some(items) => items,
            None => return HttpResponse::InternalServerError().finish(),
        }
    };

    let queue_id = Uuid::new_v4().to_string();
    let mut rows = Vec::<QueueItemRow>::new();
    let mut seen = form
        .exclude_song_ids
        .iter()
        .take(MAX_QUEUE_ITEMS)
        .filter(|song_id| cache.song(song_id).is_some())
        .cloned()
        .collect::<HashSet<_>>();
    let mut explicit_seen = HashSet::new();
    for song_id in form.explicit_song_ids.iter().take(MAX_QUEUE_ITEMS) {
        if cache.song(song_id).is_some() && explicit_seen.insert(song_id.clone()) {
            seen.insert(song_id.clone());
            rows.push(QueueItemRow {
                position: rows.len() as i32,
                song_id: song_id.clone(),
                origin: "manual".into(),
                score: 0.0,
                reason: "added by you".into(),
            });
        }
    }
    for candidate in ranked {
        if rows.len() >= MAX_QUEUE_ITEMS {
            break;
        }
        if seen.insert(candidate.song_id.clone()) {
            rows.push(QueueItemRow {
                position: rows.len() as i32,
                song_id: candidate.song_id,
                origin: "generated".into(),
                score: candidate.score,
                reason: candidate.reason,
            });
        }
    }
    if rows.is_empty() {
        return HttpResponse::UnprocessableEntity().json(serde_json::json!({
            "error": "queue_empty"
        }));
    }

    let persistence_pool = pool.get_ref().clone();
    let persistence_queue_id = queue_id.clone();
    let persistence_seed = match &seed {
        Some(RadioSeed::Song(id)) => Some(id.clone()),
        _ => None,
    };
    let persistence_seed_kind = seed.as_ref().map(RadioSeed::kind);
    let persistence_seed_id = seed.as_ref().map(|seed| seed.id().to_owned());
    let persistence_source = form
        .source
        .as_deref()
        .map(str::trim)
        .unwrap_or("radio")
        .to_owned();
    let response_source = persistence_source.clone();
    let persistence_rows = rows
        .iter()
        .map(|row| {
            (
                row.song_id.clone(),
                row.origin.clone(),
                row.score,
                row.reason.clone(),
            )
        })
        .collect::<Vec<_>>();
    let persisted = web::block(move || -> Result<(), String> {
        let mut connection = persistence_pool.get().map_err(|error| error.to_string())?;
        connection
            .transaction::<_, diesel::result::Error, _>(|connection| {
                diesel::sql_query(
                    "INSERT INTO playback_queue
                     (id, user_id, seed_song_id, source, seed_kind, seed_id)
                     VALUES (?, ?, ?, ?, ?, ?)",
                )
                .bind::<Text, _>(&persistence_queue_id)
                .bind::<Integer, _>(user_id)
                .bind::<diesel::sql_types::Nullable<Text>, _>(persistence_seed.as_deref())
                .bind::<Text, _>(&persistence_source)
                .bind::<diesel::sql_types::Nullable<Text>, _>(persistence_seed_kind)
                .bind::<diesel::sql_types::Nullable<Text>, _>(persistence_seed_id.as_deref())
                .execute(connection)?;
                for (position, row) in persistence_rows.iter().enumerate() {
                    diesel::sql_query(
                        "INSERT INTO playback_queue_item
                         (queue_id, position, song_id, origin, score, reason)
                         VALUES (?, ?, ?, ?, ?, ?)",
                    )
                    .bind::<Text, _>(&persistence_queue_id)
                    .bind::<Integer, _>(position as i32)
                    .bind::<Text, _>(&row.0)
                    .bind::<Text, _>(&row.1)
                    .bind::<Double, _>(row.2)
                    .bind::<Text, _>(&row.3)
                    .execute(connection)?;
                }
                prune_old_queues(connection, user_id, &persistence_queue_id)?;
                Ok(())
            })
            .map_err(|error| error.to_string())
    })
    .await;
    if !matches!(persisted, Ok(Ok(()))) {
        tracing::error!(?persisted, "queue persistence failed");
        return HttpResponse::InternalServerError().finish();
    }

    let items = rows
        .into_iter()
        .filter_map(|row| hydrate_queue_item(row, cache.as_ref()))
        .collect();
    HttpResponse::Ok().json(QueueResponse {
        id: queue_id,
        revision: 1,
        current_position: 0,
        source: response_source,
        seed: seed.and_then(|seed| queue_seed(&seed, cache.as_ref())),
        items,
    })
}

#[get("/{queue_id}")]
async fn get_queue(
    request: HttpRequest,
    queue_id: web::Path<String>,
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
    let queue_id = queue_id.into_inner();
    if Uuid::parse_str(&queue_id).is_err() {
        return HttpResponse::NotFound().finish();
    }
    let query_queue_id = queue_id.clone();
    let query_pool = pool.get_ref().clone();
    let queried = web::block(
        move || -> Result<Option<(QueueMetaRow, Vec<QueueItemRow>)>, String> {
            let mut connection = query_pool.get().map_err(|error| error.to_string())?;
            let state = diesel::sql_query(
                "SELECT revision, current_position, source, seed_kind, seed_id, seed_song_id
             FROM playback_queue WHERE id = ? AND user_id = ?",
            )
            .bind::<Text, _>(&query_queue_id)
            .bind::<Integer, _>(user_id)
            .get_result::<QueueMetaRow>(&mut connection)
            .optional()
            .map_err(|error| error.to_string())?;
            let Some(state) = state else { return Ok(None) };
            let rows = diesel::sql_query(
                "SELECT qi.position, qi.song_id, qi.origin, qi.score, qi.reason
             FROM playback_queue_item qi
             JOIN playback_queue q ON q.id = qi.queue_id
             WHERE qi.queue_id = ? AND q.user_id = ? ORDER BY qi.position ASC",
            )
            .bind::<Text, _>(&query_queue_id)
            .bind::<Integer, _>(user_id)
            .load::<QueueItemRow>(&mut connection)
            .map_err(|error| error.to_string())?;
            Ok(Some((state, rows)))
        },
    )
    .await;
    let (state, rows) = match queried {
        Ok(Ok(Some(result))) => result,
        Ok(Ok(None)) => return HttpResponse::NotFound().finish(),
        Ok(Err(error)) => {
            tracing::error!(%error, "queue lookup failed");
            return HttpResponse::InternalServerError().finish();
        }
        Err(error) => {
            tracing::error!(%error, "queue lookup worker failed");
            return HttpResponse::InternalServerError().finish();
        }
    };
    let items = rows
        .into_iter()
        .filter_map(|row| hydrate_queue_item(row, cache.as_ref()))
        .collect();
    let seed = state
        .seed()
        .and_then(|seed| queue_seed(&seed, cache.as_ref()));
    HttpResponse::Ok().json(QueueResponse {
        id: queue_id,
        revision: state.revision,
        current_position: state.current_position,
        source: state.source,
        seed,
        items,
    })
}

#[patch("/{queue_id}")]
async fn update_queue(
    request: HttpRequest,
    queue_id: web::Path<String>,
    form: web::Json<UpdateQueueRequest>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    if form.revision < 1 {
        return HttpResponse::BadRequest().finish();
    }
    let update_pool = pool.get_ref().clone();
    let update_queue_id = queue_id.into_inner();
    if Uuid::parse_str(&update_queue_id).is_err() {
        return HttpResponse::NotFound().json(serde_json::json!({
            "error": "queue_not_found"
        }));
    }
    let current_position = form.current_position;
    let revision = form.revision;
    let updated = web::block(move || -> Result<UpdateQueueResult, String> {
        let mut connection = update_pool.get().map_err(|error| error.to_string())?;
        update_queue_position(
            &mut connection,
            &update_queue_id,
            user_id,
            current_position,
            revision,
        )
        .map_err(|error| error.to_string())
    })
    .await;
    match updated {
        Ok(Ok(UpdateQueueResult::Updated(state))) => HttpResponse::Ok().json(serde_json::json!({
            "revision": state.revision,
            "current_position": state.current_position,
        })),
        Ok(Ok(UpdateQueueResult::Missing)) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "queue_not_found"
        })),
        Ok(Ok(UpdateQueueResult::InvalidPosition)) => {
            HttpResponse::BadRequest().json(serde_json::json!({
                "error": "queue_position_invalid"
            }))
        }
        Ok(Ok(UpdateQueueResult::Conflict(state))) => {
            HttpResponse::Conflict().json(serde_json::json!({
                "error": "queue_revision_conflict",
                "revision": state.revision,
                "current_position": state.current_position,
            }))
        }
        Ok(Err(error)) => {
            tracing::error!(%error, "queue position update failed");
            HttpResponse::InternalServerError().finish()
        }
        Err(error) => {
            tracing::error!(%error, "queue position update worker failed");
            HttpResponse::InternalServerError().finish()
        }
    }
}

async fn radio_candidates(
    user_id: i32,
    seed: Option<RadioSeed>,
    exclude: HashSet<String>,
    cache: std::sync::Arc<LibraryCache>,
    pool: DbPool,
    limit: usize,
) -> Option<Vec<RankedCandidate>> {
    match web::block(move || {
        recommend_radio(
            user_id,
            seed.as_ref(),
            cache.as_ref(),
            &pool,
            limit,
            &exclude,
        )
        .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(items)) => Some(items),
        Ok(Err(error)) => {
            tracing::error!(%error, "radio recommendation failed");
            None
        }
        Err(error) => {
            tracing::error!(%error, "radio recommendation worker failed");
            None
        }
    }
}

#[derive(Deserialize)]
struct RadioPicksRequest {
    #[serde(flatten)]
    seed: SeedFields,
    #[serde(default)]
    exclude_song_ids: Vec<String>,
    limit: Option<usize>,
}

#[derive(Serialize)]
struct RadioPick {
    #[serde(flatten)]
    song: QueueSong,
    /// Short, listener-facing reason for the pick ("from this artist").
    reason: String,
}

#[derive(Serialize)]
struct RadioPicksResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<QueueSeed>,
    items: Vec<RadioPick>,
}

/// Fresh Radio picks without creating a queue: used to extend the Radio
/// section of a client-side queue and to replace a thumbed-down pick.
#[post("/picks")]
async fn radio_picks(
    request: HttpRequest,
    form: web::Json<RadioPicksRequest>,
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
    let limit = form.limit.unwrap_or(DEFAULT_RADIO_PICKS);
    if !(1..=MAX_RADIO_PICKS).contains(&limit)
        || form.exclude_song_ids.len() > MAX_RADIO_PICK_EXCLUSIONS
        || form
            .exclude_song_ids
            .iter()
            .any(|value| !valid_song_id(value))
        || form.seed.parse().is_err()
    {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "invalid_radio_request"
        }));
    }
    let Ok(seed) = form.seed.resolve(cache.as_ref()) else {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "invalid_queue_seed"
        }));
    };
    let mut exclude = form
        .exclude_song_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    if let Some(RadioSeed::Song(id)) = &seed {
        exclude.insert(id.clone());
    }
    let Some(ranked) = radio_candidates(
        user_id,
        seed.clone(),
        exclude,
        cache.clone(),
        pool.get_ref().clone(),
        limit,
    )
    .await
    else {
        return HttpResponse::InternalServerError().finish();
    };
    HttpResponse::Ok().json(RadioPicksResponse {
        seed: seed.and_then(|seed| queue_seed(&seed, cache.as_ref())),
        items: radio_pick_items(ranked, cache.as_ref()),
    })
}

fn radio_pick_items(ranked: Vec<RankedCandidate>, cache: &LibraryCache) -> Vec<RadioPick> {
    ranked
        .into_iter()
        .enumerate()
        .filter_map(|(position, candidate)| {
            let reason = candidate.reason.clone();
            hydrate_queue_item(
                QueueItemRow {
                    position: position as i32,
                    song_id: candidate.song_id,
                    origin: "generated".into(),
                    score: candidate.score,
                    reason: String::new(),
                },
                cache,
            )
            .map(|song| RadioPick { song, reason })
        })
        .collect()
}

#[derive(Debug, PartialEq, Serialize, QueryableByName)]
struct RadioSettings {
    /// Continue with Radio picks once the queue runs out.
    #[diesel(sql_type = diesel::sql_types::Bool)]
    enabled: bool,
    /// Show a "Coming up" notice with thumbs before a Radio pick plays.
    #[diesel(sql_type = diesel::sql_types::Bool)]
    coming_up_notice: bool,
}

impl Default for RadioSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            coming_up_notice: true,
        }
    }
}

#[derive(Deserialize)]
struct RadioSettingsUpdate {
    enabled: Option<bool>,
    coming_up_notice: Option<bool>,
}

fn load_radio_settings(
    connection: &mut diesel::sqlite::SqliteConnection,
    user_id: i32,
) -> Result<RadioSettings, diesel::result::Error> {
    Ok(diesel::sql_query(
        "SELECT enabled, coming_up_notice FROM user_radio_settings WHERE user_id = ?",
    )
    .bind::<Integer, _>(user_id)
    .get_result::<RadioSettings>(connection)
    .optional()?
    .unwrap_or_default())
}

fn save_radio_settings(
    connection: &mut diesel::sqlite::SqliteConnection,
    user_id: i32,
    update: &RadioSettingsUpdate,
) -> Result<RadioSettings, diesel::result::Error> {
    connection.transaction(|connection| {
        let current = load_radio_settings(connection, user_id)?;
        let next = RadioSettings {
            enabled: update.enabled.unwrap_or(current.enabled),
            coming_up_notice: update.coming_up_notice.unwrap_or(current.coming_up_notice),
        };
        diesel::sql_query(
            "INSERT INTO user_radio_settings (user_id, enabled, coming_up_notice)
             VALUES (?, ?, ?)
             ON CONFLICT(user_id) DO UPDATE SET
               enabled = excluded.enabled,
               coming_up_notice = excluded.coming_up_notice,
               updated_at = CURRENT_TIMESTAMP",
        )
        .bind::<Integer, _>(user_id)
        .bind::<diesel::sql_types::Bool, _>(next.enabled)
        .bind::<diesel::sql_types::Bool, _>(next.coming_up_notice)
        .execute(connection)?;
        Ok(next)
    })
}

#[get("/settings")]
async fn get_radio_settings(request: HttpRequest, pool: web::Data<DbPool>) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let query_pool = pool.get_ref().clone();
    match web::block(move || -> Result<RadioSettings, String> {
        let mut connection = query_pool.get().map_err(|error| error.to_string())?;
        load_radio_settings(&mut connection, user_id).map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(settings)) => HttpResponse::Ok().json(settings),
        Ok(Err(error)) => {
            tracing::error!(%error, "radio settings lookup failed");
            HttpResponse::InternalServerError().finish()
        }
        Err(error) => {
            tracing::error!(%error, "radio settings worker failed");
            HttpResponse::InternalServerError().finish()
        }
    }
}

#[put("/settings")]
async fn update_radio_settings(
    request: HttpRequest,
    form: web::Json<RadioSettingsUpdate>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let update = form.into_inner();
    let update_pool = pool.get_ref().clone();
    match web::block(move || -> Result<RadioSettings, String> {
        let mut connection = update_pool.get().map_err(|error| error.to_string())?;
        save_radio_settings(&mut connection, user_id, &update).map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(settings)) => HttpResponse::Ok().json(settings),
        Ok(Err(error)) => {
            tracing::error!(%error, "radio settings update failed");
            HttpResponse::InternalServerError().finish()
        }
        Err(error) => {
            tracing::error!(%error, "radio settings update worker failed");
            HttpResponse::InternalServerError().finish()
        }
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/playback/queues")
            .service(create_queue)
            .service(get_queue)
            .service(update_queue),
    )
    .service(
        web::scope("/playback/radio")
            .service(radio_picks)
            .service(get_radio_settings)
            .service(update_radio_settings),
    );
}

#[cfg(test)]
mod tests {
    use diesel::Connection;
    use diesel::connection::SimpleConnection;

    use super::{
        MAX_PERSISTED_QUEUES_PER_USER, QueueAlbum, QueueArtist, QueueCountRow, QueueSong,
        UpdateQueueResult, prune_old_queues, update_queue_position,
    };
    use diesel::RunQueryDsl;

    #[test]
    fn queue_position_updates_distinguish_conflict_bounds_and_missing_state() {
        let mut connection = diesel::sqlite::SqliteConnection::establish(":memory:")
            .expect("in-memory queue database");
        connection
            .batch_execute(
                "CREATE TABLE playback_queue (
                    id TEXT PRIMARY KEY, user_id INTEGER NOT NULL,
                    revision INTEGER NOT NULL, current_position INTEGER NOT NULL,
                    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
                 );
                 CREATE TABLE playback_queue_item (
                    queue_id TEXT NOT NULL, position INTEGER NOT NULL
                 );
                 INSERT INTO playback_queue (id, user_id, revision, current_position)
                 VALUES ('queue-1', 9, 3, 0);
                 INSERT INTO playback_queue_item (queue_id, position)
                 VALUES ('queue-1', 0), ('queue-1', 1);",
            )
            .expect("queue fixture");

        let conflict =
            update_queue_position(&mut connection, "queue-1", 9, 1, 2).expect("queue conflict");
        assert!(matches!(
            conflict,
            UpdateQueueResult::Conflict(state)
                if state.revision == 3 && state.current_position == 0
        ));
        assert_eq!(
            update_queue_position(&mut connection, "queue-1", 9, 2, 3)
                .expect("invalid queue position"),
            UpdateQueueResult::InvalidPosition,
        );
        assert_eq!(
            update_queue_position(&mut connection, "missing", 9, 0, 1).expect("missing queue"),
            UpdateQueueResult::Missing,
        );
        let updated =
            update_queue_position(&mut connection, "queue-1", 9, 1, 3).expect("queue update");
        assert!(matches!(
            updated,
            UpdateQueueResult::Updated(state)
                if state.revision == 4 && state.current_position == 1
        ));
    }

    #[test]
    fn queue_retention_keeps_the_new_queue_and_cascades_old_items() {
        let mut connection = diesel::sqlite::SqliteConnection::establish(":memory:")
            .expect("in-memory queue retention database");
        connection
            .batch_execute(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE playback_queue (
                    id TEXT PRIMARY KEY, user_id INTEGER NOT NULL,
                    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
                 );
                 CREATE TABLE playback_queue_item (
                    queue_id TEXT NOT NULL, position INTEGER NOT NULL,
                    FOREIGN KEY(queue_id) REFERENCES playback_queue(id) ON DELETE CASCADE
                 );
                 WITH RECURSIVE sequence(value) AS (
                    SELECT 1 UNION ALL SELECT value + 1 FROM sequence WHERE value < 105
                 )
                 INSERT INTO playback_queue (id, user_id)
                 SELECT printf('old-%03d', value), 9 FROM sequence;
                 INSERT INTO playback_queue_item (queue_id, position)
                 SELECT id, 0 FROM playback_queue;
                 INSERT INTO playback_queue (id, user_id) VALUES ('new-queue', 9);
                 INSERT INTO playback_queue_item (queue_id, position) VALUES ('new-queue', 0);",
            )
            .expect("queue retention fixture");

        assert_eq!(
            prune_old_queues(&mut connection, 9, "new-queue").expect("queue pruning"),
            6,
        );
        let queues =
            diesel::sql_query("SELECT COUNT(*) AS count FROM playback_queue WHERE user_id = 9")
                .get_result::<QueueCountRow>(&mut connection)
                .expect("retained queue count")
                .count;
        let items = diesel::sql_query("SELECT COUNT(*) AS count FROM playback_queue_item")
            .get_result::<QueueCountRow>(&mut connection)
            .expect("retained queue item count")
            .count;
        assert_eq!(queues, MAX_PERSISTED_QUEUES_PER_USER);
        assert_eq!(items, MAX_PERSISTED_QUEUES_PER_USER);
        let protected = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM playback_queue WHERE id = 'new-queue'",
        )
        .get_result::<QueueCountRow>(&mut connection)
        .expect("protected queue count")
        .count;
        assert_eq!(protected, 1);
    }

    #[test]
    fn queue_items_do_not_embed_library_graphs_or_filesystem_paths() {
        let item = QueueSong {
            id: "song-1".into(),
            name: "Song".into(),
            artist: "Artist".into(),
            contributing_artists: Vec::new(),
            contributing_artist_ids: Vec::new(),
            track_number: 1,
            duration: 120.0,
            album_object: QueueAlbum {
                id: "album-1".into(),
                name: "Album".into(),
                cover_url: "cover.jpg".into(),
            },
            artist_object: QueueArtist {
                id: "artist-1".into(),
                name: "Artist".into(),
                icon_url: "artist.jpg".into(),
            },
            origin: "manual".into(),
            queue_position: 0,
            audio: None,
        };
        let json = serde_json::to_value(item).expect("queue item JSON");
        assert!(json.get("path").is_none());
        assert!(json.get("score").is_none());
        assert!(json.get("reason").is_none());
        assert!(json["album_object"].get("songs").is_none());
        assert!(json["artist_object"].get("albums").is_none());
        assert!(serde_json::to_vec(&json).unwrap().len() < 512);
    }

    use super::{
        QueueMetaRow, QueueSeed, RadioSettings, RadioSettingsUpdate, SeedError, SeedFields,
        load_radio_settings, queue_seed, radio_pick_items, save_radio_settings,
    };
    use crate::recommendation::test_support::{radio_pool, radio_test_cache};
    use crate::recommendation::{RadioSeed, RankedCandidate};

    fn seed(fields: SeedFields) -> Result<Option<RadioSeed>, SeedError> {
        fields.resolve(&radio_test_cache())
    }

    #[test]
    fn radio_seeds_accept_exactly_one_catalog_entity() {
        assert_eq!(seed(SeedFields::default()), Ok(None));
        assert_eq!(
            seed(SeedFields {
                seed_album_id: Some("album-jazz".into()),
                ..SeedFields::default()
            }),
            Ok(Some(RadioSeed::Album("album-jazz".into())))
        );
        assert_eq!(
            seed(SeedFields {
                seed_genre: Some(" jAzZ ".into()),
                ..SeedFields::default()
            }),
            Ok(Some(RadioSeed::Genre("Jazz".into())))
        );
        assert_eq!(
            seed(SeedFields {
                seed_song_id: Some("r1".into()),
                seed_artist_id: Some("artist-jazz".into()),
                ..SeedFields::default()
            }),
            Err(SeedError::Invalid)
        );
        assert_eq!(
            seed(SeedFields {
                seed_genre: Some("x".repeat(129)),
                ..SeedFields::default()
            }),
            Err(SeedError::Invalid)
        );
        assert_eq!(
            seed(SeedFields {
                seed_artist_id: Some("missing".into()),
                ..SeedFields::default()
            }),
            Err(SeedError::Missing)
        );
    }

    #[test]
    fn queue_seeds_carry_display_names_and_fall_back_to_legacy_song_seeds() {
        let cache = radio_test_cache();
        assert_eq!(
            queue_seed(&RadioSeed::Artist("artist-jazz".into()), &cache),
            Some(QueueSeed {
                kind: "artist",
                id: "artist-jazz".into(),
                name: "artist-jazz name".into(),
            })
        );
        assert_eq!(queue_seed(&RadioSeed::Song("gone".into()), &cache), None);
        let legacy = QueueMetaRow {
            revision: 1,
            current_position: 0,
            source: "radio".into(),
            seed_kind: None,
            seed_id: None,
            seed_song_id: Some("r1".into()),
        };
        assert_eq!(legacy.seed(), Some(RadioSeed::Song("r1".into())));
        let genre = QueueMetaRow {
            seed_kind: Some("genre".into()),
            seed_id: Some("Rock".into()),
            seed_song_id: None,
            ..legacy
        };
        assert_eq!(genre.seed(), Some(RadioSeed::Genre("Rock".into())));
    }

    #[test]
    fn radio_picks_explain_themselves_as_generated_items() {
        let cache = radio_test_cache();
        let picks = radio_pick_items(
            vec![
                RankedCandidate {
                    song_id: "j1".into(),
                    score: 20.0,
                    reason: "from this artist".into(),
                },
                RankedCandidate {
                    song_id: "deleted".into(),
                    score: 19.0,
                    reason: "played before".into(),
                },
            ],
            &cache,
        );
        assert_eq!(picks.len(), 1);
        let json = serde_json::to_value(&picks[0]).expect("radio pick JSON");
        assert_eq!(json["id"], "j1");
        assert_eq!(json["origin"], "generated");
        assert_eq!(json["reason"], "from this artist");
        assert!(json.get("score").is_none());
        assert!(json.get("path").is_none());
    }

    #[test]
    fn radio_settings_default_on_and_update_per_listener() {
        let pool = radio_pool();
        let mut connection = pool.get().expect("settings connection");
        assert_eq!(
            load_radio_settings(&mut connection, 1).expect("default settings"),
            RadioSettings::default()
        );
        let saved = save_radio_settings(
            &mut connection,
            1,
            &RadioSettingsUpdate {
                enabled: Some(false),
                coming_up_notice: None,
            },
        )
        .expect("save settings");
        assert_eq!(
            saved,
            RadioSettings {
                enabled: false,
                coming_up_notice: true,
            }
        );
        let saved = save_radio_settings(
            &mut connection,
            1,
            &RadioSettingsUpdate {
                enabled: None,
                coming_up_notice: Some(false),
            },
        )
        .expect("save notice");
        assert!(!saved.enabled && !saved.coming_up_notice);
        assert_eq!(
            load_radio_settings(&mut connection, 1).expect("reloaded"),
            saved
        );
        assert_eq!(
            load_radio_settings(&mut connection, 2).expect("other listener"),
            RadioSettings::default()
        );
    }
}
