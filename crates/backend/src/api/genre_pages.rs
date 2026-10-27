//! Canonical genres: chips for albums and artists, the genre tree, genre
//! pages and editing an album's genres.
//!
//! Raw genres come from the files' tags (always) and from MusicBrainz
//! (when online metadata is on). The administrator chooses which source
//! wins; each falls back to the other, and genres chosen by hand replace
//! both. Every raw name is normalised into the canonical tree in
//! [`crate::enrichment::genres`].

use std::collections::{HashMap, HashSet};

use actix_web::{HttpRequest, HttpResponse, delete, get, put, web};
use diesel::SqliteConnection;
use serde::{Deserialize, Serialize};

use crate::api::auth::authenticated_user_id;
use crate::api::editorial::{EnrichmentStatus, ExternalLink, artist_image_url};
use crate::api::error::{bad_request, internal_server_error, not_found};
use crate::enrichment::genres::{
    self, GenreRef, GenreSource, RawGenre, by_slug, children, choose_raw, lineage, resolve,
    with_ancestors,
};
use crate::enrichment::identity::{PersonIdentity, load_identity};
use crate::enrichment::store::{self, EditorialText, TaskState};
use crate::enrichment::worker::wake;
use crate::library::identity::normalize_artist_identity;
use crate::library::state::{LibraryCache, LibraryLifecycle, library_unavailable_response};
use crate::persistence::connection::DbPool;

/// Chips shown for one album or artist.
pub const MAX_CHIPS: usize = 6;
const MAX_EDITED_GENRES: usize = 12;
const MAX_GENRE_CHARS: usize = 64;
const MAX_PAGE_ALBUMS: usize = 500;
const MAX_PAGE_PEOPLE: usize = 60;

/// Effective genres of every album, and MusicBrainz genres of artists.
#[derive(Debug, Default)]
pub struct GenreIndex {
    pub preference: GenreSource,
    pub albums: HashMap<String, Vec<GenreRef>>,
    pub artist_online: HashMap<String, Vec<(String, i64)>>,
    pub edited: HashSet<String>,
}

impl GenreIndex {
    pub fn build(
        preference: GenreSource,
        album_ids: impl IntoIterator<Item = String>,
        tags: &HashMap<String, Vec<String>>,
        album_online: &HashMap<String, Vec<(String, i64)>>,
        artist_online: HashMap<String, Vec<(String, i64)>>,
        edits: &HashMap<String, Vec<String>>,
    ) -> Self {
        let mut albums = HashMap::new();
        for album_id in album_ids {
            let raw = choose_raw(
                preference,
                edits.get(&album_id).map(Vec::as_slice),
                tags.get(&album_id).map(Vec::as_slice).unwrap_or_default(),
                album_online
                    .get(&album_id)
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
            );
            let refs = resolve(&raw, MAX_CHIPS);
            if !refs.is_empty() {
                albums.insert(album_id, refs);
            }
        }
        Self {
            preference,
            albums,
            artist_online,
            edited: edits.keys().cloned().collect(),
        }
    }

    pub fn album(&self, album_id: &str) -> &[GenreRef] {
        self.albums
            .get(album_id)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// An artist's genres: MusicBrainz' when online genres are preferred
    /// and the artist has some, otherwise the genres of their albums, most
    /// albums first.
    pub fn artist(&self, artist_id: &str, album_ids: &[String]) -> Vec<GenreRef> {
        let online = self
            .artist_online
            .get(artist_id)
            .map(|genres| {
                genres
                    .iter()
                    .map(|(name, votes)| RawGenre {
                        name: name.clone(),
                        source: "musicbrainz".into(),
                        weight: *votes,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let from_albums = album_ids
            .iter()
            .flat_map(|id| self.album(id))
            .map(|genre| RawGenre {
                name: genre.name.clone(),
                source: genre.sources.first().cloned().unwrap_or_default(),
                weight: 1,
            })
            .collect::<Vec<_>>();
        let (first, second) = match self.preference {
            GenreSource::Online => (online, from_albums),
            GenreSource::Tags => (from_albums, online),
        };
        let refs = resolve(&first, MAX_CHIPS);
        if refs.is_empty() {
            resolve(&second, MAX_CHIPS)
        } else {
            refs
        }
    }

    /// Whether an album belongs on a genre's page: one of its genres is
    /// the genre or sits below it.
    pub fn album_in(&self, album_id: &str, slug: &str) -> bool {
        self.album(album_id).iter().any(|genre| {
            genre.slug == slug || (genre.canonical && genres::is_within(&genre.slug, slug))
        })
    }
}

fn online_by_entity(rows: Vec<store::GenreLinkRow>) -> HashMap<String, Vec<(String, i64)>> {
    let mut map = HashMap::<String, Vec<(String, i64)>>::new();
    for row in rows {
        map.entry(row.entity_id)
            .or_default()
            .push((row.name, row.weight));
    }
    map
}

/// Reads everything the index needs. Online genres are only used while
/// online metadata is on.
pub fn load_index(
    connection: &mut SqliteConnection,
    album_ids: Vec<String>,
) -> diesel::QueryResult<GenreIndex> {
    let settings = store::load_settings(connection)?;
    let tags = store::album_tag_genres(connection)?;
    let (album_online, artist_online) = if settings.enabled {
        (
            online_by_entity(store::genre_links(connection, "album")?),
            online_by_entity(store::genre_links(connection, "artist")?),
        )
    } else {
        (HashMap::new(), HashMap::new())
    };
    let edits = store::album_genre_edits(connection)?;
    Ok(GenreIndex::build(
        settings.genre_source,
        album_ids,
        &tags,
        &album_online,
        artist_online,
        &edits,
    ))
}

pub fn library_album_ids(cache: &LibraryCache) -> Vec<String> {
    cache
        .artists
        .iter()
        .flat_map(|artist| &artist.albums)
        .map(|album| album.id.clone())
        .collect()
}

/// Loads the index off the async runtime.
pub async fn genre_index(pool: DbPool, cache: &LibraryCache) -> Result<GenreIndex, String> {
    let album_ids = library_album_ids(cache);
    web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        load_index(&mut connection, album_ids).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[derive(Debug, Clone, Serialize)]
pub struct GenreLink {
    pub slug: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GenreCount {
    pub slug: String,
    pub name: String,
    pub album_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct GenreTreeNode {
    pub slug: String,
    pub name: String,
    /// Albums in this genre or below it.
    pub album_count: usize,
    pub children: Vec<GenreTreeNode>,
    /// Top-level genres only: covers of up to four of its albums, for the
    /// tile behind the name.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub covers: Vec<String>,
    /// Top-level genres only: photos of up to three of its artists (most
    /// albums first), shown duotone on the tile. Empty until online
    /// metadata has found photos.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub photos: Vec<String>,
    /// The genre's artists, most albums first; used to pick `photos`.
    #[serde(skip)]
    pub artist_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GenreTree {
    pub genres: Vec<GenreTreeNode>,
    /// Genres the canonical tree does not know, as they were named.
    pub other: Vec<GenreCount>,
    pub source: GenreSource,
    /// Every canonical genre, for choosing genres when editing an album.
    pub catalog: Vec<CatalogGenre>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogGenre {
    pub slug: &'static str,
    pub name: &'static str,
    pub parent: Option<&'static str>,
}

/// Album counts per canonical slug (an album counts for every ancestor of
/// its genres once) and per free genre.
fn album_counts(index: &GenreIndex) -> (HashMap<String, usize>, HashMap<String, (String, usize)>) {
    let mut canonical = HashMap::<String, usize>::new();
    let mut other = HashMap::<String, (String, usize)>::new();
    for refs in index.albums.values() {
        for slug in with_ancestors(
            refs.iter()
                .filter(|genre| genre.canonical)
                .map(|genre| genre.slug.as_str()),
        ) {
            *canonical.entry(slug).or_default() += 1;
        }
        for genre in refs.iter().filter(|genre| !genre.canonical) {
            other
                .entry(genre.slug.clone())
                .or_insert_with(|| (genre.name.clone(), 0))
                .1 += 1;
        }
    }
    (canonical, other)
}

pub fn build_tree(index: &GenreIndex) -> GenreTree {
    let (counts, other) = album_counts(index);
    fn branch(slug: &str, name: &str, counts: &HashMap<String, usize>) -> Option<GenreTreeNode> {
        let album_count = counts.get(slug).copied().unwrap_or(0);
        if album_count == 0 {
            return None;
        }
        let mut nodes = children(slug)
            .into_iter()
            .filter_map(|child| branch(child.slug, child.name, counts))
            .collect::<Vec<_>>();
        nodes.sort_by(|left, right| {
            right
                .album_count
                .cmp(&left.album_count)
                .then_with(|| left.name.cmp(&right.name))
        });
        Some(GenreTreeNode {
            slug: slug.to_string(),
            name: name.to_string(),
            album_count,
            children: nodes,
            covers: Vec::new(),
            photos: Vec::new(),
            artist_ids: Vec::new(),
        })
    }
    let mut roots = genres::TREE
        .iter()
        .filter(|node| node.parent.is_none())
        .filter_map(|node| branch(node.slug, node.name, &counts))
        .collect::<Vec<_>>();
    roots.sort_by(|left, right| {
        right
            .album_count
            .cmp(&left.album_count)
            .then_with(|| left.name.cmp(&right.name))
    });
    let mut other = other
        .into_iter()
        .map(|(slug, (name, album_count))| GenreCount {
            slug,
            name,
            album_count,
        })
        .collect::<Vec<_>>();
    other.sort_by(|left, right| {
        right
            .album_count
            .cmp(&left.album_count)
            .then_with(|| left.name.cmp(&right.name))
    });
    GenreTree {
        genres: roots,
        other,
        source: index.preference,
        catalog: genres::TREE
            .iter()
            .map(|node| CatalogGenre {
                slug: node.slug,
                name: node.name,
                parent: node.parent,
            })
            .collect(),
    }
}

/// How many covers and photos a top-level genre tile carries.
const TILE_COVERS: usize = 4;
const TILE_PHOTOS: usize = 3;

/// Gives each top-level genre its covers and its artists (most albums in
/// the genre first), so the genre browser can show imagery, not just names.
pub fn add_tile_art(tree: &mut GenreTree, index: &GenreIndex, cache: &LibraryCache) {
    for root in &mut tree.genres {
        let mut artists: Vec<(&str, usize)> = Vec::new();
        for artist in cache.artists.iter() {
            for album in &artist.albums {
                if album.songs.is_empty() || !index.album_in(&album.id, &root.slug) {
                    continue;
                }
                let cover = album.cover_url.trim();
                if root.covers.len() < TILE_COVERS
                    && !cover.is_empty()
                    && !root.covers.iter().any(|known| known == cover)
                {
                    root.covers.push(cover.to_string());
                }
                match artists.iter_mut().find(|(id, _)| *id == artist.id) {
                    Some((_, count)) => *count += 1,
                    None => artists.push((artist.id.as_str(), 1)),
                }
            }
        }
        artists.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(right.0)));
        root.artist_ids = artists.into_iter().map(|(id, _)| id.to_string()).collect();
    }
}

/// The artists whose stored photos the top-level tiles could use.
pub fn tile_photo_candidates(tree: &GenreTree) -> Vec<String> {
    let mut ids = tree
        .genres
        .iter()
        .flat_map(|root| root.artist_ids.iter().take(12).cloned())
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    ids
}

/// Picks up to `take` stored artist photos for each tile, in order. An
/// artist filed under several genres (a folk singer who is also pop) leads
/// only one tile: later tiles lead with an artist no earlier tile shows,
/// so no two tiles open on the same photo while another photo is free.
pub fn distinct_tile_photos(
    artist_lists: &[&[String]],
    with_photos: &HashSet<String>,
    take: usize,
) -> Vec<Vec<String>> {
    let mut used = HashSet::<&str>::new();
    artist_lists
        .iter()
        .map(|artists| {
            let (fresh, shown): (Vec<&str>, Vec<&str>) = artists
                .iter()
                .map(String::as_str)
                .filter(|id| with_photos.contains(*id))
                .partition(|id| !used.contains(id));
            let chosen = fresh
                .into_iter()
                .chain(shown)
                .take(take)
                .collect::<Vec<_>>();
            used.extend(chosen.iter().copied());
            chosen
                .into_iter()
                .map(crate::api::editorial::artist_image_url)
                .collect()
        })
        .collect()
}

/// Points each top-level tile at up to three stored artist photos, no two
/// tiles leading with the same one (see [`distinct_tile_photos`]).
pub fn add_tile_photos(tree: &mut GenreTree, with_photos: &HashSet<String>) {
    let lists = tree
        .genres
        .iter()
        .map(|root| root.artist_ids.as_slice())
        .collect::<Vec<_>>();
    let photos = distinct_tile_photos(&lists, with_photos, TILE_PHOTOS);
    for (root, photos) in tree.genres.iter_mut().zip(photos) {
        root.photos = photos;
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GenreAlbum {
    pub id: String,
    pub name: String,
    pub cover_url: String,
    pub year: Option<String>,
    pub artist_id: String,
    pub artist_name: String,
    pub genres: Vec<GenreLink>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GenrePerson {
    pub artist_id: String,
    pub name: String,
    /// The library's artist picture, when it has one.
    pub icon_url: Option<String>,
    /// The cached photo from Wikimedia Commons, when there is one.
    pub image_url: Option<String>,
    pub album_count: usize,
    pub track_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct GenrePage {
    pub slug: String,
    pub name: String,
    pub canonical: bool,
    /// Ancestors, from the top-level genre down.
    pub path: Vec<GenreLink>,
    /// Sub-genres with how many of the library's albums are in each.
    pub children: Vec<GenreCount>,
    /// Other genres under the same parent that the library has.
    pub related: Vec<GenreCount>,
    /// The Wikipedia article lead, with attribution.
    pub about: Option<EditorialText>,
    pub links: Vec<ExternalLink>,
    pub enrichment: EnrichmentStatus,
    pub album_count: usize,
    pub albums: Vec<GenreAlbum>,
    pub artists: Vec<GenrePerson>,
    pub composers: Vec<GenrePerson>,
    pub source: GenreSource,
}

fn year(date: &str) -> Option<String> {
    let year = date.get(..4)?;
    year.bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| year.to_string())
}

fn link(genre: &GenreRef) -> GenreLink {
    GenreLink {
        slug: genre.slug.clone(),
        name: genre.name.clone(),
    }
}

/// The library side of a genre page: albums, artists and composers.
pub fn genre_page_people(
    cache: &LibraryCache,
    index: &GenreIndex,
    slug: &str,
    external_composers: &[store::CreditRow],
    identity: &PersonIdentity,
) -> (Vec<GenreAlbum>, Vec<GenrePerson>, Vec<GenrePerson>, usize) {
    let mut albums = Vec::new();
    let mut artists = HashMap::<String, GenrePerson>::new();
    let mut composers = HashMap::<String, (GenrePerson, HashSet<String>, HashSet<String>)>::new();
    let names = cache
        .artists
        .iter()
        .map(|artist| (normalize_artist_identity(&artist.name), artist.id.clone()))
        .collect::<HashMap<_, _>>();
    let external = {
        let mut by_album = HashMap::<&str, Vec<&store::CreditRow>>::new();
        for row in external_composers {
            by_album.entry(row.album_id.as_str()).or_default().push(row);
        }
        by_album
    };
    let person = |artist_id: &str, name: &str| GenrePerson {
        artist_id: artist_id.to_string(),
        name: name.to_string(),
        icon_url: cache
            .artist(artist_id)
            .map(|artist| artist.icon_url.clone())
            .filter(|url| !url.is_empty()),
        image_url: None,
        album_count: 0,
        track_count: 0,
    };
    for artist in cache.artists.iter() {
        for album in &artist.albums {
            if !index.album_in(&album.id, slug) {
                continue;
            }
            albums.push(GenreAlbum {
                id: album.id.clone(),
                name: album.name.clone(),
                cover_url: album.cover_url.clone(),
                year: year(&album.first_release_date),
                artist_id: artist.id.clone(),
                artist_name: artist.name.clone(),
                genres: index.album(&album.id).iter().map(link).collect(),
            });
            let entry = artists
                .entry(artist.id.clone())
                .or_insert_with(|| person(&artist.id, &artist.name));
            entry.album_count += 1;
            entry.track_count += album.songs.len();
            let mut credit = |id: &str, name: &str, track: &str| {
                // One entry per person, whatever name a source uses.
                let name = identity.display_name(id).unwrap_or(name);
                let entry = composers
                    .entry(id.to_string())
                    .or_insert_with(|| (person(id, name), HashSet::new(), HashSet::new()));
                entry.1.insert(album.id.clone());
                if !track.is_empty() {
                    entry.2.insert(track.to_string());
                }
            };
            for song in &album.songs {
                for composer in song
                    .credits
                    .iter()
                    .filter(|credit| credit.role == "composer")
                {
                    if !composer.artist_id.is_empty() {
                        let id = identity.for_local(
                            &composer.artist_id,
                            &composer.name,
                            Some(&album.id),
                        );
                        credit(&id, &composer.name, &song.id);
                    }
                }
            }
            for row in external.get(album.id.as_str()).into_iter().flatten() {
                let id = crate::api::editorial::row_person(identity, &names, row);
                if let Some(id) = id {
                    credit(&id, &row.name, &row.track_id);
                }
            }
        }
    }
    let album_count = albums.len();
    albums.sort_by(|left, right| {
        left.artist_name
            .to_lowercase()
            .cmp(&right.artist_name.to_lowercase())
            .then_with(|| left.year.cmp(&right.year))
            .then_with(|| left.name.cmp(&right.name))
    });
    albums.truncate(MAX_PAGE_ALBUMS);
    let rank = |people: &mut Vec<GenrePerson>| {
        people.sort_by(|left, right| {
            right
                .album_count
                .cmp(&left.album_count)
                .then(right.track_count.cmp(&left.track_count))
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
        });
        people.truncate(MAX_PAGE_PEOPLE);
    };
    let mut artists = artists.into_values().collect::<Vec<_>>();
    rank(&mut artists);
    let mut composers = composers
        .into_values()
        .map(|(mut person, albums, tracks)| {
            person.album_count = albums.len();
            person.track_count = tracks.len();
            person
        })
        .collect::<Vec<_>>();
    rank(&mut composers);
    (albums, artists, composers, album_count)
}

#[get("/canonical")]
async fn genre_tree(
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    match genre_index(pool.get_ref().clone(), &cache).await {
        Ok(index) => {
            let mut tree = build_tree(&index);
            add_tile_art(&mut tree, &index, &cache);
            let with_photos = crate::api::editorial::credited_images(
                pool.get_ref().clone(),
                tile_photo_candidates(&tree),
            )
            .await;
            add_tile_photos(&mut tree, &with_photos);
            HttpResponse::Ok().json(tree)
        }
        Err(error) => {
            tracing::error!(%error, "could not load genres");
            internal_server_error("Could not load genres.", "genres_failed")
        }
    }
}

struct StoredGenre {
    index: GenreIndex,
    about: Option<EditorialText>,
    profile: Option<store::ProfileRow>,
    enabled: bool,
    task: Option<TaskState>,
    composers: Vec<store::CreditRow>,
    queued: bool,
}

#[get("/canonical/{slug}")]
async fn genre_page(
    slug: web::Path<String>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let slug = slug.into_inner();
    if slug.is_empty() || slug.chars().count() > MAX_GENRE_CHARS * 2 {
        return bad_request("Choose a genre.", "invalid_genre");
    }
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    let node = by_slug(&slug);
    let album_ids = library_album_ids(&cache);
    let lookup = slug.clone();
    let image_pool = pool.get_ref().clone();
    let pool = pool.get_ref().clone();
    let stored = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        let run = |connection: &mut SqliteConnection| -> diesel::QueryResult<StoredGenre> {
            let index = load_index(connection, album_ids)?;
            let enabled = store::load_settings(connection)?.enabled;
            // A canonical genre's About text is looked up the first time
            // its page is opened, when online metadata is on.
            let queued = enabled && node.is_some() && store::queue_genre(connection, &lookup)?;
            let about = store::load_texts(connection, "genre", &lookup)?
                .into_iter()
                .find(|text| text.kind == "background");
            Ok(StoredGenre {
                index,
                about,
                profile: store::load_profile(connection, "genre", &lookup)?,
                enabled,
                task: store::task_state(connection, "genre", &lookup)?,
                composers: store::credits_with_role(connection, "composer")?,
                queued,
            })
        };
        run(&mut connection).map_err(|error| error.to_string())
    })
    .await;
    let stored = match stored {
        Ok(Ok(stored)) => stored,
        _ => {
            tracing::error!("could not load the genre page");
            return internal_server_error("Could not load the genre.", "genres_failed");
        }
    };
    if stored.queued {
        wake().notify_all();
    }
    let identity = load_identity(image_pool.clone(), cache.clone()).await;
    let (albums, mut artists, mut composers, album_count) =
        genre_page_people(&cache, &stored.index, &slug, &stored.composers, &identity);
    let name = match node {
        Some(node) => node.name.to_string(),
        None => match albums
            .iter()
            .flat_map(|album| &album.genres)
            .find(|genre| genre.slug == slug)
        {
            Some(genre) => genre.name.clone(),
            None => {
                return not_found(
                    "No album in your library has this genre.",
                    "genre_not_found",
                );
            }
        },
    };
    let (counts, _) = album_counts(&stored.index);
    let count_of = |slug: &str, name: &str| GenreCount {
        slug: slug.to_string(),
        name: name.to_string(),
        album_count: counts.get(slug).copied().unwrap_or(0),
    };
    let child_counts = children(&slug)
        .into_iter()
        .map(|child| count_of(child.slug, child.name))
        .collect::<Vec<_>>();
    let related = node
        .and_then(|node| node.parent)
        .map(|parent| {
            children(parent)
                .into_iter()
                .filter(|sibling| sibling.slug != slug)
                .map(|sibling| count_of(sibling.slug, sibling.name))
                .filter(|count| count.album_count > 0)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let ids = artists
        .iter()
        .chain(composers.iter())
        .map(|person| person.artist_id.clone())
        .collect::<Vec<_>>();
    let images = crate::api::editorial::credited_images(image_pool, ids).await;
    for person in artists.iter_mut().chain(composers.iter_mut()) {
        if images.contains(&person.artist_id) {
            person.image_url = Some(artist_image_url(&person.artist_id));
        }
    }
    let path = lineage(&slug);
    HttpResponse::Ok().json(GenrePage {
        canonical: node.is_some(),
        path: path[..path.len().saturating_sub(1)]
            .iter()
            .map(|node| GenreLink {
                slug: node.slug.to_string(),
                name: node.name.to_string(),
            })
            .collect(),
        children: child_counts,
        related,
        about: stored.about,
        links: stored
            .profile
            .as_ref()
            .and_then(|profile| profile.wikidata_id.as_ref())
            .map(|id| {
                vec![ExternalLink {
                    provider: "wikidata".into(),
                    url: format!("https://www.wikidata.org/wiki/{id}"),
                }]
            })
            .unwrap_or_default(),
        enrichment: EnrichmentStatus {
            enabled: stored.enabled,
            task: stored.task,
        },
        album_count,
        albums,
        artists,
        composers,
        source: stored.index.preference,
        name,
        slug,
    })
}

#[derive(Debug, Deserialize)]
pub struct GenreEdit {
    /// The genres to show, by name or canonical slug; `null` goes back to
    /// tag and online genres.
    pub genres: Option<Vec<String>>,
}

/// Validates and canonicalises edited genre names.
pub fn clean_edit(genres: &[String]) -> Result<Vec<String>, &'static str> {
    if genres.len() > MAX_EDITED_GENRES {
        return Err("Choose at most 12 genres.");
    }
    let mut cleaned = Vec::<String>::new();
    for genre in genres {
        let genre = genre.trim();
        if genre.is_empty() {
            continue;
        }
        if genre.chars().count() > MAX_GENRE_CHARS {
            return Err("Genre names are limited to 64 characters.");
        }
        let name = by_slug(genre)
            .or_else(|| genres::canonical(genre))
            .map_or_else(|| genre.to_string(), |node| node.name.to_string());
        if !cleaned
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&name))
        {
            cleaned.push(name);
        }
    }
    Ok(cleaned)
}

#[derive(Debug, Serialize)]
pub struct AlbumGenres {
    pub album_id: String,
    pub genres: Vec<GenreRef>,
    pub edited: bool,
}

async fn save_album_genres(
    request: HttpRequest,
    album_id: String,
    genres: Option<Vec<String>>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let genres = match genres.as_deref().map(clean_edit).transpose() {
        Ok(genres) => genres,
        Err(message) => return bad_request(message, "invalid_genres"),
    };
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    if cache.album(&album_id).is_none() {
        return not_found("Album not found.", "album_not_found");
    }
    let user_id = authenticated_user_id(&request).ok();
    let pool = pool.get_ref().clone();
    let id = album_id.clone();
    let saved = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        store::set_album_genre_edit(&mut connection, &id, genres.as_deref(), user_id)
            .and_then(|()| load_index(&mut connection, vec![id.clone()]))
            .map_err(|error| error.to_string())
    })
    .await;
    match saved {
        Ok(Ok(index)) => HttpResponse::Ok().json(AlbumGenres {
            genres: index.album(&album_id).to_vec(),
            edited: index.edited.contains(&album_id),
            album_id,
        }),
        _ => internal_server_error("Could not save the genres.", "genres_failed"),
    }
}

/// Administrator: sets an album's genres by hand.
#[put("/albums/{id}/genres")]
pub async fn put_album_genres(
    request: HttpRequest,
    id: web::Path<String>,
    body: web::Json<GenreEdit>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    save_album_genres(
        request,
        id.into_inner(),
        body.into_inner().genres,
        pool,
        lifecycle,
    )
    .await
}

/// Administrator: goes back to tag and online genres for an album.
#[delete("/albums/{id}/genres")]
pub async fn delete_album_genres(
    request: HttpRequest,
    id: web::Path<String>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    save_album_genres(request, id.into_inner(), None, pool, lifecycle).await
}

pub fn configure_canonical(scope: actix_web::Scope) -> actix_web::Scope {
    scope.service(genre_tree).service(genre_page)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::domain::{Album, Artist, Credit, Song};
    use crate::library::search::SearchIndex;

    fn index(preference: GenreSource) -> GenreIndex {
        let tags = HashMap::from([
            ("kind-of-blue".to_string(), vec!["Jazz".to_string()]),
            ("blue-train".to_string(), vec!["Hard Bop".to_string()]),
            ("ninth".to_string(), vec!["Classical".to_string()]),
            ("tago-mago".to_string(), vec!["Krautrock".to_string()]),
        ]);
        let online = HashMap::from([(
            "kind-of-blue".to_string(),
            vec![("modal jazz".to_string(), 7), ("cool jazz".to_string(), 3)],
        )]);
        let artists = HashMap::from([(
            "miles".to_string(),
            vec![("jazz".to_string(), 12), ("bop".to_string(), 4)],
        )]);
        let edits = HashMap::from([("edited".to_string(), vec!["Bebop".to_string()])]);
        GenreIndex::build(
            preference,
            [
                "kind-of-blue",
                "blue-train",
                "ninth",
                "tago-mago",
                "edited",
                "none",
            ]
            .map(str::to_string),
            &tags,
            &online,
            artists,
            &edits,
        )
    }

    fn slugs(refs: &[GenreRef]) -> Vec<&str> {
        refs.iter().map(|genre| genre.slug.as_str()).collect()
    }

    #[test]
    fn albums_show_the_preferred_source_and_edits_win() {
        let online = index(GenreSource::Online);
        assert_eq!(
            slugs(online.album("kind-of-blue")),
            ["modal-jazz", "cool-jazz"]
        );
        assert_eq!(online.album("kind-of-blue")[0].sources, ["musicbrainz"]);
        assert_eq!(online.album("kind-of-blue")[0].path, ["Jazz"]);
        // No online genres: the tags are used.
        assert_eq!(slugs(online.album("blue-train")), ["hard-bop"]);
        assert_eq!(slugs(online.album("edited")), ["bebop"]);
        assert_eq!(online.album("edited")[0].sources, ["user"]);
        assert!(online.album("none").is_empty());
        let tags = index(GenreSource::Tags);
        assert_eq!(slugs(tags.album("kind-of-blue")), ["jazz"]);
        assert_eq!(tags.album("kind-of-blue")[0].sources, ["tags"]);
        assert!(online.album_in("blue-train", "jazz"));
        assert!(online.album_in("kind-of-blue", "jazz"));
        assert!(!online.album_in("ninth", "jazz"));
        assert!(online.album_in("tago-mago", "krautrock"));
    }

    #[test]
    fn artists_use_their_own_online_genres_or_their_albums() {
        let online = index(GenreSource::Online);
        assert_eq!(slugs(&online.artist("miles", &[])), ["jazz", "bebop"]);
        let albums = ["blue-train".to_string(), "edited".to_string()];
        assert_eq!(
            slugs(&online.artist("coltrane", &albums)),
            ["bebop", "hard-bop"]
        );
        let tags = index(GenreSource::Tags);
        assert_eq!(
            slugs(&tags.artist("miles", &["kind-of-blue".to_string()])),
            ["jazz"]
        );
    }

    #[test]
    fn the_tree_counts_albums_under_every_ancestor_and_keeps_unknown_genres() {
        let tree = build_tree(&index(GenreSource::Online));
        let jazz = tree.genres.iter().find(|node| node.slug == "jazz").unwrap();
        // Kind of Blue, Blue Train and the edited album.
        assert_eq!(jazz.album_count, 3);
        let children = jazz
            .children
            .iter()
            .map(|child| (child.slug.as_str(), child.album_count))
            .collect::<Vec<_>>();
        assert_eq!(
            children,
            [
                ("bebop", 1),
                ("cool-jazz", 1),
                ("hard-bop", 1),
                ("modal-jazz", 1)
            ]
        );
        assert_eq!(tree.genres[0].slug, "jazz");
        assert_eq!(tree.other.len(), 1);
        assert_eq!(tree.other[0].name, "Krautrock");
    }

    #[test]
    fn top_level_genre_tiles_carry_covers_and_their_artists_photos() {
        let mut cache = cache();
        let mut artists = (*cache.artists).clone();
        artists[0].albums[0].cover_url = "kind-of-blue.jpg".into();
        artists[1].albums[0].cover_url = "blue-train.jpg".into();
        cache.artists = Arc::new(artists);
        let index = index(GenreSource::Online);
        let mut tree = build_tree(&index);
        add_tile_art(&mut tree, &index, &cache);
        let jazz = tree.genres.iter().find(|node| node.slug == "jazz").unwrap();
        assert_eq!(jazz.covers, ["kind-of-blue.jpg", "blue-train.jpg"]);
        assert_eq!(jazz.artist_ids, ["coltrane", "miles"]);
        assert!(jazz.children.iter().all(|child| child.covers.is_empty()));
        let candidates = tile_photo_candidates(&tree);
        assert!(candidates.contains(&"miles".to_string()));
        add_tile_photos(&mut tree, &HashSet::from(["miles".to_string()]));
        let jazz = tree.genres.iter().find(|node| node.slug == "jazz").unwrap();
        assert_eq!(jazz.photos, ["/artists/miles/image"]);
        let json = serde_json::to_value(jazz).unwrap();
        assert!(json.get("artist_ids").is_none());
        // A child carries no tile art, so its JSON stays as before.
        let child = serde_json::to_value(&jazz.children[0]).unwrap();
        assert!(child.get("covers").is_none() && child.get("photos").is_none());
    }

    #[test]
    fn two_genre_tiles_never_open_on_the_same_photo() {
        let node = |slug: &str, artists: &[&str]| GenreTreeNode {
            slug: slug.into(),
            name: slug.into(),
            album_count: 1,
            children: Vec::new(),
            covers: Vec::new(),
            photos: Vec::new(),
            artist_ids: artists.iter().map(|id| id.to_string()).collect(),
        };
        // Joni Mitchell is filed under both Folk and Pop.
        let mut tree = GenreTree {
            genres: vec![
                node("folk", &["joni", "drake"]),
                node("pop", &["joni", "carole"]),
                node("rock", &["joni"]),
            ],
            other: Vec::new(),
            source: GenreSource::Online,
            catalog: Vec::new(),
        };
        let photos = ["joni", "drake", "carole"]
            .into_iter()
            .map(String::from)
            .collect::<HashSet<_>>();
        add_tile_photos(&mut tree, &photos);
        let leads = tree
            .genres
            .iter()
            .map(|node| node.photos.first().cloned().unwrap_or_default())
            .collect::<Vec<_>>();
        assert_eq!(leads[0], "/artists/joni/image");
        assert_eq!(leads[1], "/artists/carole/image");
        // Rock has no other artist: it may repeat a photo rather than
        // show none.
        assert_eq!(leads[2], "/artists/joni/image");
        assert_eq!(
            tree.genres[1].photos,
            ["/artists/carole/image", "/artists/joni/image"]
        );
    }

    #[test]
    fn edits_are_canonicalised_bounded_and_deduplicated() {
        let cleaned = clean_edit(&[
            "bop".to_string(),
            "Bebop".to_string(),
            "hard-bop".to_string(),
            " ".to_string(),
            "Krautrock".to_string(),
        ])
        .unwrap();
        assert_eq!(cleaned, ["Bebop", "Hard Bop", "Krautrock"]);
        assert!(clean_edit(&vec!["x".to_string(); 13]).is_err());
        assert!(clean_edit(&["x".repeat(65)]).is_err());
    }

    fn cache() -> LibraryCache {
        let composer = |id: &str, name: &str| Credit {
            artist_id: id.into(),
            name: name.into(),
            role: "composer".into(),
            instrument: None,
        };
        let song = |id: &str, credits: Vec<Credit>| Song {
            id: id.into(),
            name: id.into(),
            credits,
            ..Song::default()
        };
        let album = |id: &str, songs: Vec<Song>| Album {
            id: id.into(),
            name: id.into(),
            first_release_date: "1959-03-02".into(),
            songs,
            ..Album::default()
        };
        let artists = vec![
            Artist {
                id: "miles".into(),
                name: "Miles Davis".into(),
                albums: vec![album(
                    "kind-of-blue",
                    vec![
                        song("so-what", vec![composer("miles", "Miles Davis")]),
                        song("blue-in-green", vec![composer("evans", "Bill Evans")]),
                    ],
                )],
                ..Artist::default()
            },
            Artist {
                id: "coltrane".into(),
                name: "John Coltrane".into(),
                albums: vec![album(
                    "blue-train",
                    vec![song(
                        "blue-train-1",
                        vec![composer("coltrane", "John Coltrane")],
                    )],
                )],
                ..Artist::default()
            },
            Artist {
                id: "karajan".into(),
                name: "Herbert von Karajan".into(),
                albums: vec![album("ninth", vec![song("ninth-1", vec![])])],
                ..Artist::default()
            },
        ];
        let mut artist_positions = HashMap::new();
        let mut album_positions = HashMap::new();
        for (artist_index, artist) in artists.iter().enumerate() {
            artist_positions.insert(artist.id.clone(), artist_index as _);
            for (album_index, album) in artist.albums.iter().enumerate() {
                album_positions.insert(album.id.clone(), (artist_index as _, album_index as _));
            }
        }
        LibraryCache {
            search_index: SearchIndex::build(&artists).unwrap(),
            artists: Arc::new(artists),
            song_map: HashMap::new(),
            album_genres: HashMap::new(),
            artist_positions,
            album_positions,
            song_positions: HashMap::new(),
            songs_flat: Vec::new(),
            songs_by_artist: HashMap::new(),
            songs_by_genre: HashMap::new(),
            image_paths: Default::default(),
        }
    }

    #[test]
    fn a_genre_page_lists_albums_artists_and_composers_from_sub_genres_too() {
        let cache = cache();
        let index = index(GenreSource::Online);
        let external = vec![store::CreditRow {
            album_id: "ninth".into(),
            track_id: "ninth-1".into(),
            name: "Ludwig van Beethoven".into(),
            normalized_name: normalize_artist_identity("Ludwig van Beethoven"),
            role: "composer".into(),
            instrument: String::new(),
            source: "musicbrainz".into(),
            source_artist_id: Some("1f9df192-a621-4f54-8850-2c5373b7eac9".into()),
        }];
        let (albums, artists, composers, count) = genre_page_people(
            &cache,
            &index,
            "jazz",
            &external,
            &PersonIdentity::default(),
        );
        assert_eq!(count, 2);
        assert_eq!(
            albums
                .iter()
                .map(|album| album.id.as_str())
                .collect::<Vec<_>>(),
            ["blue-train", "kind-of-blue"]
        );
        assert_eq!(albums[0].year.as_deref(), Some("1959"));
        assert_eq!(albums[1].genres[0].name, "Modal Jazz");
        assert_eq!(
            artists
                .iter()
                .map(|person| person.name.as_str())
                .collect::<Vec<_>>(),
            // More tracks first when album counts tie.
            ["Miles Davis", "John Coltrane"]
        );
        assert_eq!(
            composers
                .iter()
                .map(|person| person.name.as_str())
                .collect::<Vec<_>>(),
            ["Bill Evans", "John Coltrane", "Miles Davis"]
        );
        let (_, _, classical, _) = genre_page_people(
            &cache,
            &index,
            "classical",
            &external,
            &PersonIdentity::default(),
        );
        assert_eq!(classical[0].name, "Ludwig van Beethoven");
        assert!(classical[0].artist_id.starts_with("mb:"));
        let (krautrock, _, _, _) = genre_page_people(
            &cache,
            &index,
            "krautrock",
            &external,
            &PersonIdentity::default(),
        );
        assert!(krautrock.is_empty(), "tago-mago is not in the test library");
    }
}
