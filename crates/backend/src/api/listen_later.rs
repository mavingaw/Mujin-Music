//! Listen Later routes, mounted under `/library/listen-later`.

use std::cmp::Ordering;

use actix_web::{HttpRequest, HttpResponse, get, post, put, web};
use serde::{Deserialize, Serialize};

use crate::api::auth::authenticated_user_id;
use crate::api::error::{bad_request, internal_server_error};
use crate::app::{
    LocalCatalogArtist, LocalFocusAlbum, LocalFocusSong, focus_album, focus_artist, focus_song,
};
use crate::library::state::{LibraryCache, LibraryLifecycle, library_unavailable_response};
use crate::listen_later::{
    ListenLaterEntity, ListenLaterRow, MAX_LISTEN_LATER_BATCH, add, list, remove,
    remove_when_played, saved_ids, set_remove_when_played,
};
use crate::persistence::connection::DbPool;

type BoxError = Box<dyn std::error::Error + Send + Sync>;
const MAX_ENTITY_ID_CHARACTERS: usize = 256;
const MAX_PAGE: usize = 1_000;
const MAX_TEXT_CHARACTERS: usize = 200;

fn worker_failed(context: &str, error: impl std::fmt::Display) -> HttpResponse {
    tracing::error!(%error, "{context}");
    internal_server_error(
        "The request could not be completed.",
        "listen_later_request_failed",
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListenLaterSort {
    #[default]
    Added,
    Title,
    Artist,
    Year,
    Duration,
    Quality,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Direction {
    Asc,
    Desc,
}

#[derive(Debug, Default, Deserialize)]
pub struct ListenLaterQuery {
    /// `album`, `track`, `artist` or `all`.
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    sort: ListenLaterSort,
    #[serde(default)]
    direction: Option<Direction>,
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenLaterItem {
    pub entity_type: ListenLaterEntity,
    pub entity_id: String,
    pub added_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<LocalFocusAlbum>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub song: Option<LocalFocusSong>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist: Option<LocalCatalogArtist>,
}

impl ListenLaterItem {
    fn title(&self) -> &str {
        if let Some(album) = &self.album {
            &album.album.name
        } else if let Some(song) = &self.song {
            &song.song.name
        } else {
            self.artist.as_ref().map_or("", |artist| &artist.name)
        }
    }

    fn artist_name(&self) -> &str {
        if let Some(album) = &self.album {
            &album.album.artist_name
        } else if let Some(song) = &self.song {
            &song.song.artist_name
        } else {
            self.artist.as_ref().map_or("", |artist| &artist.name)
        }
    }

    fn year(&self, cache: &LibraryCache) -> Option<i32> {
        let album_id = match (&self.album, &self.song) {
            (Some(album), _) => album.album.id.as_str(),
            (_, Some(song)) => song.song.album_id.as_str(),
            _ => return None,
        };
        cache
            .album(album_id)?
            .first_release_date
            .get(..4)?
            .parse()
            .ok()
    }

    fn duration(&self, cache: &LibraryCache) -> f64 {
        if let Some(song) = &self.song {
            return song.song.duration_seconds;
        }
        if let Some(album) = &self.album {
            return cache.album(&album.album.id).map_or(0.0, |album| {
                album.songs.iter().map(|song| song.duration.max(0.0)).sum()
            });
        }
        0.0
    }

    fn quality_rank(&self) -> u8 {
        let quality = self
            .album
            .as_ref()
            .and_then(|album| album.quality)
            .or_else(|| self.song.as_ref().and_then(|song| song.quality));
        match quality {
            Some("hires") => 3,
            Some("cd") => 2,
            Some("lossy") => 1,
            _ => 0,
        }
    }
}

#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub struct ListenLaterCounts {
    pub all: usize,
    pub album: usize,
    pub track: usize,
    pub artist: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenLaterPage {
    /// Items that match the type and text filter.
    pub total: usize,
    pub offset: usize,
    /// Available items per type, before the type filter.
    pub counts: ListenLaterCounts,
    /// Saved items the library no longer has.
    pub unavailable: usize,
    pub remove_when_played: bool,
    pub items: Vec<ListenLaterItem>,
}

/// Joins saved rows to the library, dropping what it no longer has.
pub fn hydrate(rows: Vec<ListenLaterRow>, cache: &LibraryCache) -> (Vec<ListenLaterItem>, usize) {
    let mut missing = 0;
    let items = rows
        .into_iter()
        .filter_map(|row| {
            let entity = ListenLaterEntity::parse(&row.entity_type)?;
            let mut item = ListenLaterItem {
                entity_type: entity,
                entity_id: row.entity_id,
                added_at: row.added_at,
                album: None,
                song: None,
                artist: None,
            };
            let found = match entity {
                ListenLaterEntity::Album => {
                    item.album = focus_album(cache, &item.entity_id);
                    item.album.is_some()
                }
                ListenLaterEntity::Track => {
                    item.song = focus_song(cache, &item.entity_id);
                    item.song.is_some()
                }
                ListenLaterEntity::Artist => {
                    item.artist = focus_artist(cache, &item.entity_id);
                    item.artist.is_some()
                }
            };
            if !found {
                missing += 1;
            }
            found.then_some(item)
        })
        .collect();
    (items, missing)
}

fn text_matches(item: &ListenLaterItem, needle: &str) -> bool {
    let haystack = [
        item.title(),
        item.artist_name(),
        item.song
            .as_ref()
            .map_or("", |song| song.song.album_name.as_str()),
    ];
    haystack
        .iter()
        .any(|value| value.to_lowercase().contains(needle))
}

/// Filters, sorts and pages hydrated items.
pub fn page(
    items: Vec<ListenLaterItem>,
    query: &ListenLaterQuery,
    cache: &LibraryCache,
) -> (ListenLaterCounts, usize, Vec<ListenLaterItem>) {
    let mut counts = ListenLaterCounts {
        all: items.len(),
        ..ListenLaterCounts::default()
    };
    for item in &items {
        match item.entity_type {
            ListenLaterEntity::Album => counts.album += 1,
            ListenLaterEntity::Track => counts.track += 1,
            ListenLaterEntity::Artist => counts.artist += 1,
        }
    }
    let kind = query.kind.as_deref().and_then(ListenLaterEntity::parse);
    let needle = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_lowercase);
    let mut filtered = items
        .into_iter()
        .filter(|item| kind.is_none_or(|kind| item.entity_type == kind))
        .filter(|item| {
            needle
                .as_deref()
                .is_none_or(|needle| text_matches(item, needle))
        })
        .collect::<Vec<_>>();
    let direction = query.direction.unwrap_or(match query.sort {
        ListenLaterSort::Added | ListenLaterSort::Year | ListenLaterSort::Quality => {
            Direction::Desc
        }
        _ => Direction::Asc,
    });
    let text = |value: &str| value.to_lowercase();
    filtered.sort_by(|left, right| {
        let ordering = match query.sort {
            ListenLaterSort::Added => left.added_at.cmp(&right.added_at),
            ListenLaterSort::Title => text(left.title()).cmp(&text(right.title())),
            ListenLaterSort::Artist => text(left.artist_name())
                .cmp(&text(right.artist_name()))
                .then_with(|| left.year(cache).cmp(&right.year(cache)))
                .then_with(|| text(left.title()).cmp(&text(right.title()))),
            ListenLaterSort::Year => left.year(cache).cmp(&right.year(cache)),
            ListenLaterSort::Duration => left
                .duration(cache)
                .partial_cmp(&right.duration(cache))
                .unwrap_or(Ordering::Equal),
            ListenLaterSort::Quality => left.quality_rank().cmp(&right.quality_rank()),
        };
        let ordering = if direction == Direction::Desc {
            ordering.reverse()
        } else {
            ordering
        };
        ordering.then_with(|| right.added_at.cmp(&left.added_at))
    });
    let total = filtered.len();
    let offset = query.offset.unwrap_or(0).min(total);
    let limit = query.limit.unwrap_or(500).clamp(1, MAX_PAGE);
    let items = filtered.into_iter().skip(offset).take(limit).collect();
    (counts, total, items)
}

#[get("")]
async fn listen_later_page(
    request: HttpRequest,
    query: web::Query<ListenLaterQuery>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let query = query.into_inner();
    if query
        .q
        .as_deref()
        .is_some_and(|text| text.chars().count() > MAX_TEXT_CHARACTERS)
    {
        return bad_request("The filter text is too long.", "listen_later_query_invalid");
    }
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    let pool = pool.get_ref().clone();
    match web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        let rows = list(&mut connection, user_id)?;
        let remove_when_played = remove_when_played(&mut connection, user_id)?;
        let (items, unavailable) = hydrate(rows, &cache);
        let (counts, total, items) = page(items, &query, &cache);
        Ok(ListenLaterPage {
            total,
            offset: query.offset.unwrap_or(0).min(total),
            counts,
            unavailable,
            remove_when_played,
            items,
        })
    })
    .await
    {
        Ok(Ok(page)) => HttpResponse::Ok().json(page),
        Ok(Err(error)) => worker_failed("listen later list failed", error),
        Err(error) => worker_failed("listen later list worker failed", error),
    }
}

#[derive(Deserialize)]
struct ItemsRequest {
    entity_type: ListenLaterEntity,
    entity_ids: Vec<String>,
}

fn valid_items(body: &ItemsRequest) -> bool {
    !body.entity_ids.is_empty()
        && body.entity_ids.len() <= MAX_LISTEN_LATER_BATCH
        && body
            .entity_ids
            .iter()
            .all(|id| !id.is_empty() && id.chars().count() <= MAX_ENTITY_ID_CHARACTERS)
}

fn invalid_items() -> HttpResponse {
    bad_request(
        "Listen Later items must list 1 to 500 valid identifiers.",
        "listen_later_items_invalid",
    )
}

#[post("/items")]
async fn add_items(
    request: HttpRequest,
    body: web::Json<ItemsRequest>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let body = body.into_inner();
    if !valid_items(&body) {
        return invalid_items();
    }
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    let pool = pool.get_ref().clone();
    match web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        Ok(add(
            &mut connection,
            user_id,
            body.entity_type,
            &body.entity_ids,
            &cache,
        )?)
    })
    .await
    {
        Ok(Ok(outcome)) => HttpResponse::Ok().json(outcome),
        Ok(Err(error)) => worker_failed("listen later add failed", error),
        Err(error) => worker_failed("listen later add worker failed", error),
    }
}

#[post("/items/remove")]
async fn remove_items(
    request: HttpRequest,
    body: web::Json<ItemsRequest>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let body = body.into_inner();
    if !valid_items(&body) {
        return invalid_items();
    }
    let pool = pool.get_ref().clone();
    match web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        Ok(remove(
            &mut connection,
            user_id,
            body.entity_type,
            &body.entity_ids,
        )?)
    })
    .await
    {
        Ok(Ok(removed)) => HttpResponse::Ok().json(serde_json::json!({ "removed": removed })),
        Ok(Err(error)) => worker_failed("listen later remove failed", error),
        Err(error) => worker_failed("listen later remove worker failed", error),
    }
}

/// Which of the given ids are saved, for menus and buttons.
#[post("/saved")]
async fn saved(
    request: HttpRequest,
    body: web::Json<ItemsRequest>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let body = body.into_inner();
    if !valid_items(&body) {
        return invalid_items();
    }
    let pool = pool.get_ref().clone();
    match web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        Ok(saved_ids(
            &mut connection,
            user_id,
            body.entity_type,
            &body.entity_ids,
        )?)
    })
    .await
    {
        Ok(Ok(ids)) => HttpResponse::Ok().json(serde_json::json!({ "ids": ids })),
        Ok(Err(error)) => worker_failed("listen later lookup failed", error),
        Err(error) => worker_failed("listen later lookup worker failed", error),
    }
}

#[derive(Deserialize, Serialize)]
struct Settings {
    remove_when_played: bool,
}

#[get("/settings")]
async fn settings(request: HttpRequest, pool: web::Data<DbPool>) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let pool = pool.get_ref().clone();
    match web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        Ok(remove_when_played(&mut connection, user_id)?)
    })
    .await
    {
        Ok(Ok(remove_when_played)) => HttpResponse::Ok().json(Settings { remove_when_played }),
        Ok(Err(error)) => worker_failed("listen later settings failed", error),
        Err(error) => worker_failed("listen later settings worker failed", error),
    }
}

#[put("/settings")]
async fn update_settings(
    request: HttpRequest,
    body: web::Json<Settings>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let enabled = body.remove_when_played;
    let pool = pool.get_ref().clone();
    match web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        set_remove_when_played(&mut connection, user_id, enabled)?;
        Ok(())
    })
    .await
    {
        Ok(Ok(())) => HttpResponse::Ok().json(Settings {
            remove_when_played: enabled,
        }),
        Ok(Err(error)) => worker_failed("listen later settings update failed", error),
        Err(error) => worker_failed("listen later settings worker failed", error),
    }
}

/// Listen Later routes, mounted under `/library/listen-later`.
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(listen_later_page)
        .service(add_items)
        .service(remove_items)
        .service(saved)
        .service(settings)
        .service(update_settings);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recommendation::test_support::radio_test_cache;

    fn row(kind: &str, id: &str, added: &str) -> ListenLaterRow {
        ListenLaterRow {
            entity_type: kind.into(),
            entity_id: id.into(),
            added_at: added.into(),
        }
    }

    #[test]
    fn pages_hydrate_filter_and_sort() {
        let cache = radio_test_cache();
        let rows = vec![
            row("album", "album-jazz", "2026-10-03T10:00:00Z"),
            row("track", "r4", "2026-10-02T10:00:00Z"),
            row("artist", "artist-rock-1", "2026-10-01T10:00:00Z"),
            row("album", "gone", "2026-10-04T10:00:00Z"),
        ];
        let (items, missing) = hydrate(rows, &cache);
        assert_eq!(missing, 1);
        assert_eq!(items.len(), 3);

        let (counts, total, newest) = page(
            hydrate(
                vec![
                    row("album", "album-jazz", "2026-10-03T10:00:00Z"),
                    row("track", "r4", "2026-10-02T10:00:00Z"),
                    row("artist", "artist-rock-1", "2026-10-01T10:00:00Z"),
                ],
                &cache,
            )
            .0,
            &ListenLaterQuery::default(),
            &cache,
        );
        assert_eq!(
            counts,
            ListenLaterCounts {
                all: 3,
                album: 1,
                track: 1,
                artist: 1
            }
        );
        assert_eq!(total, 3);
        assert_eq!(
            newest
                .iter()
                .map(|item| item.entity_id.as_str())
                .collect::<Vec<_>>(),
            ["album-jazz", "r4", "artist-rock-1"]
        );

        let (_, total, albums) = page(
            items,
            &ListenLaterQuery {
                kind: Some("album".into()),
                ..ListenLaterQuery::default()
            },
            &cache,
        );
        assert_eq!(total, 1);
        assert_eq!(albums[0].entity_id, "album-jazz");
        assert!(albums[0].album.is_some());
    }

    #[test]
    fn text_filter_matches_titles_and_artists() {
        let cache = radio_test_cache();
        let (items, _) = hydrate(
            vec![
                row("album", "album-jazz", "2026-10-03T10:00:00Z"),
                row("track", "r4", "2026-10-02T10:00:00Z"),
            ],
            &cache,
        );
        let (_, total, found) = page(
            items,
            &ListenLaterQuery {
                q: Some("  JAZZ ".into()),
                sort: ListenLaterSort::Title,
                ..ListenLaterQuery::default()
            },
            &cache,
        );
        assert_eq!(total, 1);
        assert_eq!(found[0].entity_id, "album-jazz");
    }
}
