//! Tag pages and bulk tagging, mounted with the other tag routes under
//! `/library/tags`.

use std::collections::HashMap;

use actix_web::{HttpRequest, HttpResponse, get, post, web};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Text};
use diesel::sqlite::SqliteConnection;
use serde::{Deserialize, Serialize};

use crate::api::auth::authenticated_user_id;
use crate::api::error::{bad_request, internal_server_error, not_found};
use crate::api::focus::{
    MAX_TAG_BATCH, TagItems, TaggedEntity, UserTag, change_tag_items, tag_by_id,
};
use crate::app::{
    LocalCatalogArtist, LocalFocusAlbum, LocalFocusSong, focus_album, focus_artist, focus_song,
};
use crate::library::focus::{FocusWork, focus_ids, work_rows};
use crate::library::state::{LibraryCache, LibraryLifecycle, library_unavailable_response};
use crate::persistence::connection::DbPool;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Members of each kind shown on a tag page.
pub const MAX_TAG_PAGE_ITEMS: usize = 500;
/// Tags one bulk request may add or remove.
const MAX_BULK_TAGS: usize = 50;
const MAX_ENTITY_ID_CHARACTERS: usize = 256;

fn worker_failed(context: &str, error: impl std::fmt::Display) -> HttpResponse {
    tracing::error!(%error, "{context}");
    internal_server_error("The request could not be completed.", "tag_request_failed")
}

#[derive(Debug, Serialize, PartialEq, QueryableByName)]
#[serde(rename_all = "camelCase")]
pub struct TaggedPlaylist {
    #[diesel(sql_type = Integer)]
    pub id: i32,
    #[diesel(sql_type = Text)]
    pub name: String,
    #[diesel(sql_type = BigInt)]
    pub song_count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TagPage {
    pub tag: UserTag,
    pub albums: Vec<LocalFocusAlbum>,
    pub songs: Vec<LocalFocusSong>,
    pub artists: Vec<LocalCatalogArtist>,
    pub works: Vec<FocusWork>,
    pub playlists: Vec<TaggedPlaylist>,
}

/// Ids of one kind on a hand-made tag, most recently tagged first.
fn manual_members(
    connection: &mut SqliteConnection,
    tag_id: i32,
    entity_type: &str,
) -> Result<Vec<String>, diesel::result::Error> {
    #[derive(QueryableByName)]
    struct IdRow {
        #[diesel(sql_type = Text)]
        entity_id: String,
    }
    Ok(diesel::sql_query(
        "SELECT entity_id FROM user_tag_item WHERE tag_id = ? AND entity_type = ? \
         ORDER BY added_at DESC, entity_id LIMIT ?",
    )
    .bind::<Integer, _>(tag_id)
    .bind::<Text, _>(entity_type)
    .bind::<BigInt, _>(MAX_TAG_PAGE_ITEMS as i64)
    .load::<IdRow>(connection)?
    .into_iter()
    .map(|row| row.entity_id)
    .collect())
}

fn tagged_playlists(
    connection: &mut SqliteConnection,
    user_id: i32,
    ids: &[String],
) -> Result<Vec<TaggedPlaylist>, diesel::result::Error> {
    let mut playlists = Vec::new();
    for id in ids {
        if let Some(playlist) = diesel::sql_query(
            "SELECT p.id, p.name, \
             (SELECT COUNT(*) FROM _playlist_to_song ps WHERE ps.a = p.id) AS song_count \
             FROM playlist p JOIN _playlist_to_user pu ON pu.a = p.id \
             WHERE CAST(p.id AS TEXT) = ? AND pu.b = ?",
        )
        .bind::<Text, _>(id)
        .bind::<Integer, _>(user_id)
        .get_result::<TaggedPlaylist>(connection)
        .optional()?
        {
            playlists.push(playlist);
        }
    }
    Ok(playlists)
}

/// Members of a tag by kind: hand-tagged items, or what a smart tag's
/// focus matches now (in the focus's own sort order).
pub(crate) fn tag_page(
    connection: &mut SqliteConnection,
    user_id: i32,
    tag: UserTag,
    cache: &LibraryCache,
) -> Result<TagPage, BoxError> {
    let mut ids: HashMap<&'static str, Vec<String>> = HashMap::new();
    if let Some(query) = &tag.query {
        let focused = focus_ids(connection, query, user_id, 0, MAX_TAG_PAGE_ITEMS)?;
        let kind = crate::library::focus::tagged_entity_type(query.entity);
        ids.insert(kind, focused.ids);
    } else {
        for kind in ["album", "track", "artist", "work", "playlist"] {
            ids.insert(kind, manual_members(connection, tag.id, kind)?);
        }
    }
    let take = |kind: &str| ids.get(kind).cloned().unwrap_or_default();
    let works = take("work");
    Ok(TagPage {
        albums: take("album")
            .iter()
            .filter_map(|id| focus_album(cache, id))
            .collect(),
        songs: take("track")
            .iter()
            .filter_map(|id| focus_song(cache, id))
            .collect(),
        artists: take("artist")
            .iter()
            .filter_map(|id| focus_artist(cache, id))
            .collect(),
        works: if works.is_empty() {
            Vec::new()
        } else {
            crate::library::works::ensure_works(connection)?;
            work_rows(connection, &works)?
        },
        playlists: tagged_playlists(connection, user_id, &take("playlist"))?,
        tag,
    })
}

#[get("/{id}")]
async fn tag_details(
    request: HttpRequest,
    id: web::Path<i32>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    let id = id.into_inner();
    let pool = pool.get_ref().clone();
    match web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        let Some(tag) = tag_by_id(&mut connection, user_id, id)? else {
            return Ok(None);
        };
        Ok(Some(tag_page(&mut connection, user_id, tag, &cache)?))
    })
    .await
    {
        Ok(Ok(Some(page))) => HttpResponse::Ok().json(page),
        Ok(Ok(None)) => not_found("Tag not found.", "tag_not_found"),
        Ok(Err(error)) => worker_failed("tag page failed", error),
        Err(error) => worker_failed("tag page worker failed", error),
    }
}

fn valid_ids(ids: &[String]) -> bool {
    !ids.is_empty()
        && ids.len() <= MAX_TAG_BATCH
        && ids
            .iter()
            .all(|id| !id.is_empty() && id.chars().count() <= MAX_ENTITY_ID_CHARACTERS)
}

#[derive(Deserialize)]
struct BulkRequest {
    entity_type: TaggedEntity,
    entity_ids: Vec<String>,
    #[serde(default)]
    add: Vec<i32>,
    #[serde(default)]
    remove: Vec<i32>,
}

#[derive(Debug, Default, Serialize, PartialEq)]
pub struct BulkOutcome {
    pub added: usize,
    pub removed: usize,
    /// Tags skipped because they are missing or smart.
    pub skipped: Vec<i32>,
}

/// Applies several tags to many items at once, and removes others, as the
/// Manage Tags dialog saves a selection.
pub(crate) fn bulk_tag(
    connection: &mut SqliteConnection,
    user_id: i32,
    entity: TaggedEntity,
    ids: &[String],
    add: &[i32],
    remove: &[i32],
) -> Result<BulkOutcome, diesel::result::Error> {
    let mut outcome = BulkOutcome::default();
    for (tags, adding) in [(add, true), (remove, false)] {
        for tag_id in tags {
            match change_tag_items(connection, user_id, *tag_id, entity, ids, adding)? {
                Some(changed) if adding => outcome.added += changed,
                Some(changed) => outcome.removed += changed,
                None => outcome.skipped.push(*tag_id),
            }
        }
    }
    Ok(outcome)
}

#[post("/bulk")]
async fn bulk(
    request: HttpRequest,
    body: web::Json<BulkRequest>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let body = body.into_inner();
    if !valid_ids(&body.entity_ids)
        || body.add.len() + body.remove.len() > MAX_BULK_TAGS
        || body.add.iter().any(|tag| body.remove.contains(tag))
    {
        return bad_request(
            "Bulk tagging takes 1 to 500 items and up to 50 distinct tags.",
            "tag_bulk_invalid",
        );
    }
    let pool = pool.get_ref().clone();
    match web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        Ok(bulk_tag(
            &mut connection,
            user_id,
            body.entity_type,
            &body.entity_ids,
            &body.add,
            &body.remove,
        )?)
    })
    .await
    {
        Ok(Ok(outcome)) => HttpResponse::Ok().json(outcome),
        Ok(Err(error)) => worker_failed("bulk tag failed", error),
        Err(error) => worker_failed("bulk tag worker failed", error),
    }
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TagCoverage {
    pub tag_id: i32,
    /// How many of the requested items carry the tag.
    pub count: i64,
}

/// For each tag, how many of the given items carry it: all, some or none,
/// which the Manage Tags dialog shows as checked, mixed or empty.
pub(crate) fn tag_coverage(
    connection: &mut SqliteConnection,
    user_id: i32,
    entity: TaggedEntity,
    ids: &[String],
) -> Result<Vec<TagCoverage>, diesel::result::Error> {
    #[derive(QueryableByName)]
    struct CoverageRow {
        #[diesel(sql_type = Integer)]
        tag_id: i32,
        #[diesel(sql_type = BigInt)]
        count: i64,
    }
    let mut unique = ids.to_vec();
    unique.sort();
    unique.dedup();
    let mut counts = HashMap::<i32, i64>::new();
    for chunk in unique.chunks(200) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let mut query = diesel::sql_query(format!(
            "SELECT i.tag_id, COUNT(DISTINCT i.entity_id) AS count FROM user_tag_item i \
             JOIN user_tag g ON g.id = i.tag_id \
             WHERE g.user_id = ? AND i.entity_type = ? AND i.entity_id IN ({placeholders}) \
             GROUP BY i.tag_id"
        ))
        .into_boxed()
        .bind::<Integer, _>(user_id)
        .bind::<Text, _>(entity.as_str());
        for id in chunk {
            query = query.bind::<Text, _>(id.clone());
        }
        for row in query.load::<CoverageRow>(connection)? {
            *counts.entry(row.tag_id).or_default() += row.count;
        }
    }
    let mut coverage = counts
        .into_iter()
        .map(|(tag_id, count)| TagCoverage { tag_id, count })
        .collect::<Vec<_>>();
    coverage.sort_by_key(|item| item.tag_id);
    Ok(coverage)
}

#[post("/assigned/bulk")]
async fn assigned_bulk(
    request: HttpRequest,
    body: web::Json<TagItems>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let body = body.into_inner();
    if !valid_ids(&body.entity_ids) {
        return bad_request(
            "Tag lookups take 1 to 500 valid identifiers.",
            "tag_items_invalid",
        );
    }
    let pool = pool.get_ref().clone();
    match web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        Ok(tag_coverage(
            &mut connection,
            user_id,
            body.entity_type,
            &body.entity_ids,
        )?)
    })
    .await
    {
        Ok(Ok(coverage)) => HttpResponse::Ok().json(coverage),
        Ok(Err(error)) => worker_failed("tag coverage failed", error),
        Err(error) => worker_failed("tag coverage worker failed", error),
    }
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TagPreview {
    pub tag_id: i32,
    /// Up to four album covers for the tag's tile.
    pub covers: Vec<String>,
}

const PREVIEW_COVERS: usize = 4;

/// Album ids that picture a tag: tagged albums, then the albums of tagged
/// tracks and artists, or the first albums a smart tag's focus matches.
fn preview_albums(
    connection: &mut SqliteConnection,
    user_id: i32,
    tag: &UserTag,
    cache: &LibraryCache,
) -> Result<Vec<String>, BoxError> {
    let mut albums = Vec::<String>::new();
    let mut push = |album: &str| {
        if !album.is_empty() && !albums.iter().any(|known| known == album) {
            albums.push(album.to_string());
        }
    };
    if let Some(query) = &tag.query {
        let focused = focus_ids(connection, query, user_id, 0, PREVIEW_COVERS * 3)?;
        for id in &focused.ids {
            match query.entity {
                crate::library::focus::FocusEntity::Albums => push(id),
                crate::library::focus::FocusEntity::Songs => {
                    if let Some((_, album)) = cache.song_map.get(id) {
                        push(album);
                    }
                }
                _ => {
                    if let Some(artist) = cache.artist(id) {
                        for album in artist.albums.iter().take(2) {
                            push(&album.id);
                        }
                    }
                }
            }
        }
    } else {
        for kind in ["album", "track", "artist"] {
            for id in manual_members(connection, tag.id, kind)?
                .iter()
                .take(PREVIEW_COVERS * 2)
            {
                match kind {
                    "album" => push(id),
                    "track" => {
                        if let Some((_, album)) = cache.song_map.get(id) {
                            push(album);
                        }
                    }
                    _ => {
                        if let Some(artist) = cache.artist(id) {
                            for album in artist.albums.iter().take(2) {
                                push(&album.id);
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(albums)
}

#[get("/previews")]
async fn previews(
    request: HttpRequest,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    let pool = pool.get_ref().clone();
    match web::block(move || -> Result<_, BoxError> {
        let mut connection = pool.get()?;
        let tags = crate::api::focus::list_tags(&mut connection, user_id)?;
        let mut previews = Vec::with_capacity(tags.len());
        for tag in &tags {
            let covers = preview_albums(&mut connection, user_id, tag, &cache)?
                .iter()
                .filter_map(|id| cache.album(id))
                .map(|album| album.cover_url.clone())
                .filter(|cover| !cover.trim().is_empty())
                .take(PREVIEW_COVERS)
                .collect();
            previews.push(TagPreview {
                tag_id: tag.id,
                covers,
            });
        }
        Ok(previews)
    })
    .await
    {
        Ok(Ok(previews)) => HttpResponse::Ok().json(previews),
        Ok(Err(error)) => worker_failed("tag previews failed", error),
        Err(error) => worker_failed("tag previews worker failed", error),
    }
}

/// Routes added to the `/library/tags` scope after the core tag routes.
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(bulk)
        .service(assigned_bulk)
        .service(previews)
        .service(tag_details);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::focus::{TagDetails, TagWrite, create_tag, create_tag_with, list_tags};
    use crate::library::focus::{Criterion, FocusEntity, FocusFilter, FocusQuery, focus_facets};
    use diesel::connection::SimpleConnection;
    use diesel_migrations::MigrationHarness;

    fn database() -> SqliteConnection {
        let mut connection = SqliteConnection::establish(":memory:").expect("tags database");
        connection
            .run_pending_migrations(crate::persistence::connection::MIGRATIONS)
            .expect("migrations");
        connection
            .batch_execute(
                "PRAGMA foreign_keys = ON;
                 INSERT INTO user(id, username, password) VALUES (1, 'one', 'x'), (2, 'two', 'x');
                 INSERT INTO album_entity(id, title, normalized_title, first_release_date) VALUES
                   ('al-1', 'First', 'first', '1999'), ('al-2', 'Second', 'second', '2005'),
                   ('al-3', 'Third', 'third', '2020');
                 INSERT INTO artist_entity(id, name, normalized_name) VALUES
                   ('ar-1', 'Artist', 'artist'), ('ar-2', 'Other', 'other');
                 INSERT INTO album_artist(album_id, artist_id, role) VALUES
                   ('al-1', 'ar-1', 'primary'), ('al-2', 'ar-1', 'primary'), ('al-3', 'ar-2', 'primary');
                 INSERT INTO track_entity(id, title, normalized_title, album_id, track_number) VALUES
                   ('t-1', 'One', 'one', 'al-1', 1), ('t-2', 'Two', 'two', 'al-2', 1),
                   ('t-3', 'Three', 'three', 'al-3', 1);
                 INSERT INTO track_artist(track_id, artist_id, role) VALUES
                   ('t-1', 'ar-1', 'primary'), ('t-2', 'ar-1', 'primary'), ('t-3', 'ar-2', 'primary');
                 INSERT INTO song(id) VALUES ('t-1'), ('t-2'), ('t-3');
                 INSERT INTO playlist(id, name) VALUES (7, 'Mix'), (8, 'Not mine');
                 INSERT INTO _playlist_to_user(a, b, role) VALUES (7, 1, 'owner'), (8, 2, 'owner');
                 INSERT INTO _playlist_to_song(a, b, position) VALUES (7, 't-3', 0);",
            )
            .expect("fixture");
        connection
    }

    fn albums_from(year: i64) -> FocusQuery {
        FocusQuery {
            entity: FocusEntity::Albums,
            filters: vec![FocusFilter {
                criterion: Criterion::Year {
                    from: Some(year),
                    to: None,
                },
                exclude: false,
            }],
            ..FocusQuery::default()
        }
    }

    fn tag_filter(tag_id: i32, entity: FocusEntity) -> FocusQuery {
        FocusQuery {
            entity,
            filters: vec![FocusFilter {
                criterion: Criterion::Tag {
                    tag_id: i64::from(tag_id),
                    name: None,
                    smart: None,
                },
                exclude: false,
            }],
            ..FocusQuery::default()
        }
    }

    fn written(write: TagWrite) -> UserTag {
        match write {
            TagWrite::Written(tag) => *tag,
            other => panic!("tag not written: {other:?}"),
        }
    }

    #[test]
    fn smart_tags_follow_their_focus_and_work_as_focus_filters() {
        let mut connection = database();
        let smart = written(
            create_tag_with(
                &mut connection,
                1,
                "Modern",
                &TagDetails {
                    color: Some("violet".into()),
                    query: Some(albums_from(2000)),
                    ..TagDetails::default()
                },
            )
            .expect("smart tag"),
        );
        assert_eq!(smart.kind, "smart");
        assert_eq!(smart.counts.album, 2);
        assert_eq!(smart.item_count, 2);
        // Smart tags cannot hold hand-tagged items.
        assert_eq!(
            change_tag_items(
                &mut connection,
                1,
                smart.id,
                TaggedEntity::Album,
                &["al-1".into()],
                true
            )
            .expect("smart items"),
            None
        );

        let albums = focus_ids(
            &mut connection,
            &tag_filter(smart.id, FocusEntity::Albums),
            1,
            0,
            10,
        )
        .expect("focus");
        let mut ids = albums.ids.clone();
        ids.sort();
        assert_eq!(ids, ["al-2", "al-3"]);
        // A smart tag of albums reaches their artists and tracks too.
        assert_eq!(
            focus_ids(
                &mut connection,
                &tag_filter(smart.id, FocusEntity::Songs),
                1,
                0,
                10
            )
            .expect("songs")
            .total,
            2
        );

        // The library changes and the tag follows without a refresh.
        connection
            .batch_execute("UPDATE album_entity SET first_release_date = '2001' WHERE id = 'al-1'")
            .expect("re-date");
        assert_eq!(
            focus_ids(
                &mut connection,
                &tag_filter(smart.id, FocusEntity::Albums),
                1,
                0,
                10
            )
            .expect("refocus")
            .total,
            3
        );
        // Another profile cannot use the tag.
        assert_eq!(
            focus_ids(
                &mut connection,
                &tag_filter(smart.id, FocusEntity::Albums),
                2,
                0,
                10
            )
            .expect("other")
            .total,
            0
        );

        // Facet counts include smart tags.
        let facets = focus_facets(
            &mut connection,
            &FocusQuery::default(),
            1,
            &crate::library::focus::FacetRequest::default(),
        )
        .expect("facets");
        let tag_facet = facets
            .facets
            .iter()
            .find(|facet| facet.key == "tag")
            .expect("tag facet");
        assert_eq!(tag_facet.values[0].label, "Modern");
        assert_eq!(tag_facet.values[0].count, 3);
    }

    #[test]
    fn smart_tags_can_build_on_tags_without_looping() {
        let mut connection = database();
        let manual = written(create_tag(&mut connection, 1, "Keepers").expect("manual"));
        change_tag_items(
            &mut connection,
            1,
            manual.id,
            TaggedEntity::Artist,
            &["ar-2".into()],
            true,
        )
        .expect("tag artist");
        let mut keepers_after_2000 = albums_from(2000);
        keepers_after_2000.filters.push(FocusFilter {
            criterion: Criterion::Tag {
                tag_id: i64::from(manual.id),
                name: None,
                smart: None,
            },
            exclude: false,
        });
        let smart = written(
            create_tag_with(
                &mut connection,
                1,
                "Recent keepers",
                &TagDetails {
                    query: Some(keepers_after_2000),
                    ..TagDetails::default()
                },
            )
            .expect("smart"),
        );
        assert_eq!(smart.counts.album, 1);

        // A smart tag that names itself matches nothing through itself.
        let mut looping = albums_from(1900);
        looping.filters.push(FocusFilter {
            criterion: Criterion::Tag {
                tag_id: i64::from(smart.id) + 1,
                name: None,
                smart: None,
            },
            exclude: false,
        });
        let looped = written(
            create_tag_with(
                &mut connection,
                1,
                "Loop",
                &TagDetails {
                    query: Some(looping),
                    ..TagDetails::default()
                },
            )
            .expect("loop"),
        );
        assert_eq!(looped.id, smart.id + 1);
        let tags = list_tags(&mut connection, 1).expect("tags");
        assert_eq!(tags.len(), 3);
    }

    #[test]
    fn bulk_tagging_reports_coverage_for_mixed_selections() {
        let mut connection = database();
        let first = written(create_tag(&mut connection, 1, "A").expect("a"));
        let second = written(create_tag(&mut connection, 1, "B").expect("b"));
        let theirs = written(create_tag(&mut connection, 2, "C").expect("c"));
        let albums = vec!["al-1".to_string(), "al-2".to_string(), "al-3".to_string()];
        let outcome = bulk_tag(
            &mut connection,
            1,
            TaggedEntity::Album,
            &albums[..2],
            &[first.id, theirs.id],
            &[],
        )
        .expect("bulk");
        assert_eq!(outcome.added, 2);
        assert_eq!(outcome.skipped, [theirs.id]);
        bulk_tag(
            &mut connection,
            1,
            TaggedEntity::Album,
            &albums,
            &[second.id],
            &[],
        )
        .expect("bulk second");

        let coverage =
            tag_coverage(&mut connection, 1, TaggedEntity::Album, &albums).expect("coverage");
        assert_eq!(
            coverage,
            [
                TagCoverage {
                    tag_id: first.id,
                    count: 2
                },
                TagCoverage {
                    tag_id: second.id,
                    count: 3
                }
            ]
        );
        let outcome = bulk_tag(
            &mut connection,
            1,
            TaggedEntity::Album,
            &albums,
            &[],
            &[first.id],
        )
        .expect("bulk remove");
        assert_eq!(outcome.removed, 2);
    }

    #[test]
    fn playlists_and_works_can_be_tagged_and_focused() {
        let mut connection = database();
        let tag = written(create_tag(&mut connection, 1, "Party").expect("tag"));
        assert_eq!(
            change_tag_items(
                &mut connection,
                1,
                tag.id,
                TaggedEntity::Playlist,
                &["7".into(), "8".into()],
                true
            )
            .expect("playlists"),
            Some(1),
            "only the user's own playlist is tagged"
        );
        let tag = tag_by_id(&mut connection, 1, tag.id)
            .expect("tag")
            .expect("exists");
        assert_eq!(tag.counts.playlist, 1);
        let songs = focus_ids(
            &mut connection,
            &tag_filter(tag.id, FocusEntity::Songs),
            1,
            0,
            10,
        )
        .expect("focus");
        assert_eq!(songs.ids, ["t-3"]);
        let playlists = tagged_playlists(&mut connection, 1, &["7".into()]).expect("playlists");
        assert_eq!(
            playlists,
            [TaggedPlaylist {
                id: 7,
                name: "Mix".into(),
                song_count: 1
            }]
        );
    }
}
