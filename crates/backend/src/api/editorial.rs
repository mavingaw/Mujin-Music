//! Credits, biographies, album background and reviews, linked credit
//! browsing and per-user notes.
//!
//! Editorial text only exists when an administrator turned enrichment on;
//! credits read from tags are always available.

use std::collections::HashMap;

use actix_web::{HttpRequest, HttpResponse, get, put, web};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::album::album_credits as tag_album_credits;
use crate::api::auth::authenticated_user_id;
use crate::api::error::{bad_request, internal_server_error, not_found};
use crate::api::song::{ResponseSong, response_song};
use crate::domain::{Album, Artist};
use crate::enrichment::genres::GenreRef;
use crate::enrichment::musicbrainz::is_placeholder_artist;
use crate::enrichment::store::{
    self, CreditRow, EditorialText, ImageRow, MAX_NOTE_CHARS, NoteRow, TaskState,
};
use crate::enrichment::worker::{images_directory, wake};
use crate::library::credits::{ARTIST_ROLES, role_rank};
use crate::library::identity::normalize_artist_identity;
use crate::library::state::{LibraryCache, LibraryLifecycle, library_unavailable_response};
use crate::persistence::connection::DbPool;

const MAX_CREDITED_SONGS: usize = 500;
pub(crate) const EXTERNAL_PREFIX: &str = crate::enrichment::worker::EXTERNAL_PERSON_PREFIX;

/// A credit on an album page, merged across tags and external sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AboutCredit {
    /// Local artist id, `mb:<MusicBrainz id>` for a person only known from
    /// MusicBrainz, or `None` when the credit cannot be browsed.
    pub artist_id: Option<String>,
    pub name: String,
    pub role: String,
    pub instrument: Option<String>,
    /// Tracks the credit applies to; empty means the whole release.
    pub track_ids: Vec<String>,
    /// `tags` and/or `musicbrainz`.
    pub sources: Vec<String>,
    /// The person's cached photo (`/artists/{id}/image`), when there is one.
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnrichmentStatus {
    pub enabled: bool,
    pub task: Option<TaskState>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExternalLink {
    pub provider: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlbumAbout {
    pub album_id: String,
    pub credits: Vec<AboutCredit>,
    /// Background (the article lead) from Wikipedia.
    pub background: Option<EditorialText>,
    /// Critical reception text from Wikipedia.
    pub review: Option<EditorialText>,
    /// Release facts from MusicBrainz (date, labels, catalog number, ...).
    pub facts: Option<Value>,
    pub links: Vec<ExternalLink>,
    pub note: Option<NoteRow>,
    pub enrichment: EnrichmentStatus,
    /// The work (and piece) each track performs.
    pub works: Vec<crate::api::works::TrackWork>,
    /// The MusicBrainz release an administrator chose with "Identify album".
    pub identification: Option<store::Identification>,
    /// Canonical genres, from tags, MusicBrainz or an edit.
    pub genres: Vec<GenreRef>,
    /// Whether an administrator chose the genres by hand.
    pub genres_edited: bool,
    /// About the pieces on the album, from their own Wikipedia articles:
    /// what a classical album shows when the release has no article.
    pub work_texts: Vec<WorkText>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkText {
    pub work_mbid: String,
    pub title: String,
    pub text: EditorialText,
}

#[derive(Debug, Clone, Serialize)]
pub struct EditorialImage {
    pub url: String,
    pub author: String,
    pub license: String,
    pub license_url: Option<String>,
    pub page_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoleSummary {
    pub role: String,
    pub instrument: Option<String>,
    pub track_count: usize,
    pub album_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtistAbout {
    pub artist_id: String,
    pub name: String,
    pub biography: Option<EditorialText>,
    pub image: Option<EditorialImage>,
    /// Kind (Person, Group, ...), area, begin/end dates, disambiguation.
    pub facts: Option<Value>,
    pub links: Vec<ExternalLink>,
    pub roles: Vec<RoleSummary>,
    pub note: Option<NoteRow>,
    pub enrichment: EnrichmentStatus,
    /// The artist's genres, or those of the albums they are credited on.
    pub genres: Vec<GenreRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CreditRole {
    pub role: String,
    pub instrument: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct CreditedAlbum {
    pub album: Album,
    pub artist: Artist,
    pub roles: Vec<CreditRole>,
    /// Tracks on the album the person is credited on; empty for a
    /// release-wide credit.
    pub track_ids: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct CreditedSong {
    pub song: ResponseSong,
    pub roles: Vec<CreditRole>,
}

#[derive(Clone, Serialize)]
pub struct ArtistCredits {
    pub artist_id: String,
    pub name: String,
    /// Whether the person also has their own discography in the library.
    pub has_discography: bool,
    pub roles: Vec<RoleSummary>,
    pub albums: Vec<CreditedAlbum>,
    pub songs: Vec<CreditedSong>,
}

fn credit_key(name: &str, role: &str, instrument: Option<&str>) -> (String, String, String) {
    (
        normalize_artist_identity(name),
        role.to_string(),
        instrument.unwrap_or_default().to_lowercase(),
    )
}

fn external_id(row: &CreditRow) -> Option<String> {
    row.source_artist_id
        .as_ref()
        .map(|mbid| format!("{EXTERNAL_PREFIX}{mbid}"))
}

/// Merges tag credits with external credits. A person matched by name to a
/// local artist links to that artist.
pub fn merge_album_credits(
    album: &Album,
    known_artists: &HashMap<String, String>,
    external: &[CreditRow],
) -> Vec<AboutCredit> {
    let mut merged: Vec<AboutCredit> = Vec::new();
    let mut index: HashMap<(String, String, String), usize> = HashMap::new();
    for credit in tag_album_credits(&album.songs) {
        let key = credit_key(
            &credit.credit.name,
            &credit.credit.role,
            credit.credit.instrument.as_deref(),
        );
        index.insert(key, merged.len());
        merged.push(AboutCredit {
            artist_id: Some(credit.credit.artist_id),
            name: credit.credit.name,
            role: credit.credit.role,
            instrument: credit.credit.instrument,
            track_ids: credit.track_ids,
            sources: vec!["tags".into()],
            image_url: None,
        });
    }
    for row in external {
        let instrument = Some(row.instrument.clone()).filter(|value| !value.is_empty());
        let mut key = credit_key(&row.name, &row.role, instrument.as_deref());
        if !index.contains_key(&key)
            && let Some(instrument) = &instrument
        {
            // "bass" on MusicBrainz and "double bass" in tags are one credit;
            // a more specific external name stays separate.
            let instrument = instrument.to_lowercase();
            if let Some(existing) = index.keys().find(|(name, role, other)| {
                *name == key.0
                    && *role == key.1
                    && !other.is_empty()
                    && other.ends_with(&format!(" {instrument}"))
            }) {
                key = existing.clone();
            }
        }
        match index.get(&key) {
            Some(position) => {
                let existing = &mut merged[*position];
                if !existing.sources.contains(&row.source) {
                    existing.sources.push(row.source.clone());
                }
                if row.track_id.is_empty() {
                    existing.track_ids.clear();
                } else if !existing.track_ids.is_empty()
                    && !existing.track_ids.contains(&row.track_id)
                {
                    existing.track_ids.push(row.track_id.clone());
                }
            }
            None => {
                index.insert(key, merged.len());
                merged.push(AboutCredit {
                    artist_id: known_artists
                        .get(&row.normalized_name)
                        .cloned()
                        .or_else(|| external_id(row)),
                    name: row.name.clone(),
                    role: row.role.clone(),
                    instrument,
                    track_ids: if row.track_id.is_empty() {
                        Vec::new()
                    } else {
                        vec![row.track_id.clone()]
                    },
                    sources: vec![row.source.clone()],
                    image_url: None,
                });
            }
        }
    }
    // Keep album track order for track ids.
    let order = album
        .songs
        .iter()
        .enumerate()
        .map(|(position, song)| (song.id.as_str(), position))
        .collect::<HashMap<_, _>>();
    for credit in &mut merged {
        credit
            .track_ids
            .sort_by_key(|id| order.get(id.as_str()).copied().unwrap_or(usize::MAX));
        credit.track_ids.dedup();
    }
    merged.sort_by_key(|credit| role_rank(&credit.role));
    merged
}

/// Local artists named on an album (owner, track artists and credits), by
/// normalized name.
fn album_known_artists(cache: &LibraryCache, album: &Album) -> HashMap<String, String> {
    let mut known = HashMap::new();
    if let Some(owner) = cache.album_owner(&album.id) {
        known.insert(normalize_artist_identity(&owner.name), owner.id.clone());
    }
    for song in &album.songs {
        for (name, id) in song
            .contributing_artists
            .iter()
            .zip(&song.contributing_artist_ids)
        {
            known
                .entry(normalize_artist_identity(name))
                .or_insert_with(|| id.clone());
        }
        for credit in &song.credits {
            known
                .entry(normalize_artist_identity(&credit.name))
                .or_insert_with(|| credit.artist_id.clone());
        }
    }
    for (name, artist) in cache
        .artists
        .iter()
        .map(|artist| (normalize_artist_identity(&artist.name), artist))
    {
        known.entry(name).or_insert_with(|| artist.id.clone());
    }
    // Contributors credited elsewhere in the library, so a MusicBrainz
    // credit for someone already known links to their local credits.
    for credit in cache
        .artists
        .iter()
        .flat_map(|artist| &artist.albums)
        .flat_map(|album| &album.songs)
        .flat_map(|song| &song.credits)
    {
        known
            .entry(normalize_artist_identity(&credit.name))
            .or_insert_with(|| credit.artist_id.clone());
    }
    known
}

fn song_artist<'a>(cache: &'a LibraryCache, album_id: &str) -> Option<&'a Artist> {
    cache.album_owner(album_id)
}

/// Every credit for a person: tag credits by artist id, and external credits
/// by normalized name or MusicBrainz id.
pub fn collect_artist_credits(
    cache: &LibraryCache,
    artist_id: &str,
    external: &[CreditRow],
) -> Option<ArtistCredits> {
    let mut name = cache.artist(artist_id).map(|artist| artist.name.clone());
    let has_discography = cache
        .artist(artist_id)
        .is_some_and(|artist| !artist.albums.is_empty());
    let mut songs: Vec<(String, String, Vec<CreditRole>)> = Vec::new();
    let mut album_wide: Vec<(String, CreditRole)> = Vec::new();
    let add_song_role = |songs: &mut Vec<(String, String, Vec<CreditRole>)>,
                         song_id: &str,
                         album_id: &str,
                         role: CreditRole| {
        match songs.iter_mut().find(|(id, _, _)| id == song_id) {
            Some((_, _, roles)) => {
                if !roles.contains(&role) {
                    roles.push(role);
                }
            }
            None => songs.push((song_id.to_string(), album_id.to_string(), vec![role])),
        }
    };

    for artist in cache.artists.iter() {
        for album in &artist.albums {
            for song in &album.songs {
                for credit in song.credits.iter().filter(|credit| {
                    credit.artist_id == artist_id && !ARTIST_ROLES.contains(&credit.role.as_str())
                }) {
                    name.get_or_insert_with(|| credit.name.clone());
                    add_song_role(
                        &mut songs,
                        &song.id,
                        &album.id,
                        CreditRole {
                            role: credit.role.clone(),
                            instrument: credit.instrument.clone(),
                        },
                    );
                }
            }
        }
    }
    for row in external {
        name.get_or_insert_with(|| row.name.clone());
        let role = CreditRole {
            role: row.role.clone(),
            instrument: Some(row.instrument.clone()).filter(|value| !value.is_empty()),
        };
        if cache.album(&row.album_id).is_none() {
            continue;
        }
        if row.track_id.is_empty() {
            if !album_wide.contains(&(row.album_id.clone(), role.clone())) {
                album_wide.push((row.album_id.clone(), role));
            }
        } else if cache.song(&row.track_id).is_some() {
            add_song_role(&mut songs, &row.track_id, &row.album_id, role);
        }
    }
    let name = name?;

    let mut albums: Vec<CreditedAlbum> = Vec::new();
    let mut push_album = |album_id: &str, role: &CreditRole, track_id: Option<&str>| {
        let position = match albums.iter().position(|entry| entry.album.id == album_id) {
            Some(position) => position,
            None => {
                let (Some(album), Some(owner)) =
                    (cache.album(album_id), song_artist(cache, album_id))
                else {
                    return;
                };
                albums.push(CreditedAlbum {
                    album: crate::api::album_reference(album),
                    artist: crate::api::artist_reference(owner),
                    roles: Vec::new(),
                    track_ids: Vec::new(),
                });
                albums.len() - 1
            }
        };
        let entry = &mut albums[position];
        if !entry.roles.contains(role) {
            entry.roles.push(role.clone());
        }
        if let Some(track_id) = track_id
            && !entry.track_ids.iter().any(|id| id == track_id)
        {
            entry.track_ids.push(track_id.to_string());
        }
    };
    for (album_id, role) in &album_wide {
        push_album(album_id, role, None);
    }
    for (song_id, album_id, roles) in &songs {
        for role in roles {
            push_album(album_id, role, Some(song_id));
        }
    }
    for (album_id, _) in &album_wide {
        if let Some(entry) = albums.iter_mut().find(|entry| entry.album.id == *album_id) {
            entry.track_ids.clear();
        }
    }
    for entry in &mut albums {
        entry.roles.sort_by_key(|role| role_rank(&role.role));
    }
    albums.sort_by(|left, right| {
        right
            .album
            .first_release_date
            .cmp(&left.album.first_release_date)
            .then_with(|| left.album.name.cmp(&right.album.name))
    });

    let mut summaries: Vec<RoleSummary> = Vec::new();
    for entry in &albums {
        for role in &entry.roles {
            let tracks = if entry.track_ids.is_empty() {
                cache
                    .album(&entry.album.id)
                    .map_or(0, |album| album.songs.len())
            } else {
                songs
                    .iter()
                    .filter(|(song_id, album_id, roles)| {
                        album_id == &entry.album.id
                            && entry.track_ids.contains(song_id)
                            && roles.contains(role)
                    })
                    .count()
            };
            match summaries
                .iter_mut()
                .find(|summary| summary.role == role.role && summary.instrument == role.instrument)
            {
                Some(summary) => {
                    summary.album_count += 1;
                    summary.track_count += tracks;
                }
                None => summaries.push(RoleSummary {
                    role: role.role.clone(),
                    instrument: role.instrument.clone(),
                    track_count: tracks,
                    album_count: 1,
                }),
            }
        }
    }
    summaries.sort_by(|left, right| {
        role_rank(&left.role)
            .cmp(&role_rank(&right.role))
            .then_with(|| right.track_count.cmp(&left.track_count))
    });

    let songs = songs
        .into_iter()
        .take(MAX_CREDITED_SONGS)
        .filter_map(|(song_id, album_id, mut roles)| {
            let song = cache.song(&song_id)?;
            let album = cache.album(&album_id)?;
            let owner = cache.album_owner(&album_id)?;
            roles.sort_by_key(|role| role_rank(&role.role));
            Some(CreditedSong {
                song: response_song(song, album, owner),
                roles,
            })
        })
        .collect();

    Some(ArtistCredits {
        artist_id: artist_id.to_string(),
        name,
        has_discography,
        roles: summaries,
        albums,
        songs,
    })
}

fn texts_by_kind(texts: Vec<EditorialText>) -> HashMap<String, EditorialText> {
    texts
        .into_iter()
        .map(|text| (text.kind.clone(), text))
        .collect()
}

fn links(profile: &store::ProfileRow, entity: &str) -> Vec<ExternalLink> {
    let mut links = Vec::new();
    if let Some(mbid) = &profile.musicbrainz_id {
        links.push(ExternalLink {
            provider: "musicbrainz".into(),
            url: format!("https://musicbrainz.org/{entity}/{mbid}"),
        });
    }
    if let Some(wikidata) = &profile.wikidata_id {
        links.push(ExternalLink {
            provider: "wikidata".into(),
            url: format!("https://www.wikidata.org/wiki/{wikidata}"),
        });
    }
    links
}

struct Stored {
    profile: Option<store::ProfileRow>,
    texts: HashMap<String, EditorialText>,
    image: Option<ImageRow>,
    note: Option<NoteRow>,
    enabled: bool,
    task: Option<TaskState>,
    credits: Vec<CreditRow>,
}

async fn load_stored(
    pool: DbPool,
    entity_type: &'static str,
    entity_id: String,
    user_id: Option<i32>,
    person: Option<(Option<String>, Option<String>)>,
) -> Result<Stored, String> {
    web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        let run = |connection: &mut diesel::SqliteConnection| -> diesel::QueryResult<Stored> {
            let credits = match &person {
                Some((name, mbid)) => {
                    store::person_credits(connection, name.as_deref(), mbid.as_deref())?
                }
                None if entity_type == "album" => store::album_credits(connection, &entity_id)?,
                None => Vec::new(),
            };
            Ok(Stored {
                profile: store::load_profile(connection, entity_type, &entity_id)?,
                texts: texts_by_kind(store::load_texts(connection, entity_type, &entity_id)?),
                image: store::load_image(connection, entity_type, &entity_id)?,
                note: match user_id {
                    Some(user_id) => {
                        store::load_note(connection, user_id, entity_type, &entity_id)?
                    }
                    None => None,
                },
                enabled: store::load_settings(connection)?.enabled,
                task: store::task_state(connection, entity_type, &entity_id)?,
                credits,
            })
        };
        run(&mut connection).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

fn parse_facts(profile: Option<&store::ProfileRow>) -> Option<Value> {
    profile
        .and_then(|profile| serde_json::from_str::<Value>(&profile.profile_json).ok())
        .map(|mut value| {
            // What Wikidata linked to is bookkeeping for later stages.
            if let Some(object) = value.as_object_mut() {
                for key in crate::enrichment::worker::INTERNAL_KEYS {
                    object.remove(key);
                }
            }
            value
        })
        .filter(|value| value.as_object().is_some_and(|object| !object.is_empty()))
}

/// The photo URL for a credited person who has one.
pub(crate) fn artist_image_url(artist_id: &str) -> String {
    format!("/artists/{artist_id}/image")
}

/// Looks up which credited people have a stored photo.
pub(crate) async fn credited_images(
    pool: DbPool,
    ids: Vec<String>,
) -> std::collections::HashSet<String> {
    web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        store::artists_with_images(&mut connection, &ids).map_err(|error| error.to_string())
    })
    .await
    .ok()
    .and_then(Result::ok)
    .unwrap_or_default()
}

#[get("/{id}/about")]
pub async fn album_about(
    request: HttpRequest,
    id: web::Path<String>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let album_id = id.into_inner();
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    let Some(album) = cache.album(&album_id) else {
        return not_found("Album not found.", "album_not_found");
    };
    let user_id = authenticated_user_id(&request).ok();
    let stored = match load_stored(
        pool.get_ref().clone(),
        "album",
        album_id.clone(),
        user_id,
        None,
    )
    .await
    {
        Ok(stored) => stored,
        Err(error) => {
            tracing::error!(%error, "could not load album editorial");
            return internal_server_error("Could not load album details.", "editorial_failed");
        }
    };
    let lookup = album_id.clone();
    let image_pool = pool.get_ref().clone();
    let pool = pool.get_ref().clone();
    let works = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        let works = crate::api::works::load_album_track_works(&mut connection, &lookup)
            .map_err(|error| error.to_string())?;
        let identification = store::load_identification(&mut connection, &lookup)
            .map_err(|error| error.to_string())?;
        let mut work_texts = Vec::new();
        for work in works.1.iter().filter(|work| work.parent_mbid.is_none()) {
            if work_texts.len() >= 4 {
                break;
            }
            if let Some(text) = store::work_background(&mut connection, &work.mbid)
                .map_err(|error| error.to_string())?
                && !work_texts
                    .iter()
                    .any(|known: &WorkText| known.text.url == text.url)
            {
                work_texts.push(WorkText {
                    work_mbid: work.mbid.clone(),
                    title: crate::enrichment::works::display_title(&work.title),
                    text,
                });
            }
        }
        Ok::<_, String>((works, identification, work_texts))
    })
    .await;
    let ((work_links, work_rows), identification, work_texts) = match works {
        Ok(Ok(found)) => found,
        _ => {
            tracing::error!("could not load album works");
            return internal_server_error("Could not load album details.", "editorial_failed");
        }
    };
    let people = crate::api::works::person_index(&cache);
    let known = album_known_artists(&cache, album);
    let mut stored = stored;
    // An album by "Unknown Artist" is only trusted once a person identified
    // it or its tags name the release; older versions matched namesakes.
    let untrusted = identification.is_none()
        && album.musicbrainz_id.is_empty()
        && cache
            .album_owner(&album.id)
            .is_some_and(|owner| is_placeholder_artist(&owner.name));
    if untrusted {
        stored.profile = None;
        stored.texts.clear();
        stored.credits.clear();
    }
    let work_texts = if untrusted { Vec::new() } else { work_texts };
    let mut texts = stored.texts;
    let mut credits = merge_album_credits(album, &known, &stored.credits);
    let ids = credits
        .iter()
        .filter_map(|credit| credit.artist_id.clone())
        .collect::<Vec<_>>();
    let genre_index = crate::api::genre_pages::genre_index(image_pool.clone(), &cache).await;
    let images = credited_images(image_pool, ids).await;
    let (genres, genres_edited) = match &genre_index {
        Ok(index) => (
            index.album(&album.id).to_vec(),
            index.edited.contains(&album.id),
        ),
        Err(error) => {
            tracing::warn!(%error, "could not load album genres");
            (Vec::new(), false)
        }
    };
    for credit in &mut credits {
        credit.image_url = credit
            .artist_id
            .as_deref()
            .filter(|id| images.contains(*id))
            .map(artist_image_url);
    }
    HttpResponse::Ok().json(AlbumAbout {
        works: crate::api::works::album_track_works(&work_links, &work_rows, &people),
        identification,
        genres,
        genres_edited,
        work_texts,
        album_id,
        credits,
        background: texts.remove("background"),
        review: texts.remove("reception"),
        facts: parse_facts(stored.profile.as_ref()),
        links: stored
            .profile
            .as_ref()
            .map(|profile| links(profile, "release"))
            .unwrap_or_default(),
        note: stored.note,
        enrichment: EnrichmentStatus {
            enabled: stored.enabled,
            task: stored.task,
        },
    })
}

pub(crate) fn person_query(
    cache: &LibraryCache,
    artist_id: &str,
) -> (Option<String>, Option<String>) {
    if let Some(mbid) = artist_id.strip_prefix(EXTERNAL_PREFIX) {
        return (None, Some(mbid.to_string()));
    }
    let name = cache
        .artist(artist_id)
        .map(|artist| artist.name.clone())
        .or_else(|| {
            cache
                .artists
                .iter()
                .flat_map(|artist| &artist.albums)
                .flat_map(|album| &album.songs)
                .flat_map(|song| &song.credits)
                .find(|credit| credit.artist_id == artist_id)
                .map(|credit| credit.name.clone())
        });
    (name.map(|name| normalize_artist_identity(&name)), None)
}

#[get("/{id}/credits")]
pub async fn artist_credits(
    id: web::Path<String>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let artist_id = id.into_inner();
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    let person = person_query(&cache, &artist_id);
    let stored = match load_stored(
        pool.get_ref().clone(),
        "artist",
        artist_id.clone(),
        None,
        Some(person),
    )
    .await
    {
        Ok(stored) => stored,
        Err(error) => {
            tracing::error!(%error, "could not load artist credits");
            return internal_server_error("Could not load credits.", "credits_failed");
        }
    };
    match collect_artist_credits(&cache, &artist_id, &stored.credits) {
        Some(credits) => HttpResponse::Ok().json(credits),
        None => not_found("Artist not found.", "artist_not_found"),
    }
}

#[get("/{id}/about")]
pub async fn artist_about(
    request: HttpRequest,
    id: web::Path<String>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let artist_id = id.into_inner();
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    let user_id = authenticated_user_id(&request).ok();
    let person = person_query(&cache, &artist_id);
    let stored = match load_stored(
        pool.get_ref().clone(),
        "artist",
        artist_id.clone(),
        user_id,
        Some(person),
    )
    .await
    {
        Ok(stored) => stored,
        Err(error) => {
            tracing::error!(%error, "could not load artist editorial");
            return internal_server_error("Could not load artist details.", "editorial_failed");
        }
    };
    let Some(credits) = collect_artist_credits(&cache, &artist_id, &stored.credits) else {
        return not_found("Artist not found.", "artist_not_found");
    };
    let mut stored = stored;
    if is_placeholder_artist(&credits.name) {
        // "Unknown Artist" is not a person; never show a namesake's
        // biography, photo or facts for it.
        stored.profile = None;
        stored.texts.clear();
        stored.image = None;
    }
    let mut texts = stored.texts;
    let album_ids = match cache.artist(&artist_id) {
        Some(artist) if !artist.albums.is_empty() => artist
            .albums
            .iter()
            .map(|album| album.id.clone())
            .collect::<Vec<_>>(),
        _ => credits
            .albums
            .iter()
            .map(|credited| credited.album.id.clone())
            .collect(),
    };
    let genres = match crate::api::genre_pages::genre_index(pool.get_ref().clone(), &cache).await {
        Ok(index) => index.artist(&artist_id, &album_ids),
        Err(error) => {
            tracing::warn!(%error, "could not load artist genres");
            Vec::new()
        }
    };
    HttpResponse::Ok().json(ArtistAbout {
        genres,
        image: stored.image.map(|image| EditorialImage {
            url: artist_image_url(&artist_id),
            author: image.author,
            license: image.license,
            license_url: image.license_url,
            page_url: image.page_url,
        }),
        artist_id,
        name: credits.name,
        biography: texts.remove("biography"),
        facts: parse_facts(stored.profile.as_ref()),
        links: stored
            .profile
            .as_ref()
            .map(|profile| links(profile, "artist"))
            .unwrap_or_default(),
        roles: credits.roles,
        note: stored.note,
        enrichment: EnrichmentStatus {
            enabled: stored.enabled,
            task: stored.task,
        },
    })
}

#[get("/{id}/image")]
pub async fn artist_image(id: web::Path<String>, pool: web::Data<DbPool>) -> HttpResponse {
    let artist_id = id.into_inner();
    let pool = pool.get_ref().clone();
    let image = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        store::load_image(&mut connection, "artist", &artist_id).map_err(|error| error.to_string())
    })
    .await;
    let Ok(Ok(Some(image))) = image else {
        return HttpResponse::NotFound().finish();
    };
    if image.file_name.contains(['/', '\\']) || image.file_name.starts_with('.') {
        return HttpResponse::NotFound().finish();
    }
    match tokio::fs::read(images_directory().join(&image.file_name)).await {
        Ok(bytes) => HttpResponse::Ok()
            .content_type(image.mime_type)
            .insert_header(("Cache-Control", "private, max-age=86400"))
            .body(bytes),
        Err(_) => HttpResponse::NotFound().finish(),
    }
}

#[derive(Deserialize)]
pub struct NoteBody {
    pub body: String,
}

async fn save_note_response(
    request: HttpRequest,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
    entity_type: &'static str,
    entity_id: String,
    body: String,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    if body.chars().count() > MAX_NOTE_CHARS {
        return bad_request("Notes are limited to 20,000 characters.", "note_too_long");
    }
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    let exists = match entity_type {
        "album" => cache.album(&entity_id).is_some(),
        _ => cache.artist(&entity_id).is_some() || person_query(&cache, &entity_id) != (None, None),
    };
    if !exists {
        return not_found("Not found.", "entity_not_found");
    }
    let pool = pool.get_ref().clone();
    match web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        store::save_note(&mut connection, user_id, entity_type, &entity_id, &body)
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(note)) => HttpResponse::Ok().json(serde_json::json!({ "note": note })),
        _ => internal_server_error("Could not save the note.", "note_failed"),
    }
}

#[put("/{id}/note")]
pub async fn put_album_note(
    request: HttpRequest,
    id: web::Path<String>,
    body: web::Json<NoteBody>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    save_note_response(
        request,
        pool,
        lifecycle,
        "album",
        id.into_inner(),
        body.into_inner().body,
    )
    .await
}

#[put("/{id}/note")]
pub async fn put_artist_note(
    request: HttpRequest,
    id: web::Path<String>,
    body: web::Json<NoteBody>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    save_note_response(
        request,
        pool,
        lifecycle,
        "artist",
        id.into_inner(),
        body.into_inner().body,
    )
    .await
}

#[derive(Serialize)]
pub struct EnrichmentOverview {
    #[serde(flatten)]
    pub settings: store::EnrichmentSettings,
    pub counts: store::TaskCounts,
    pub sources: Vec<&'static str>,
    /// Each lookup stage's queue, with its own host and back-off.
    pub stages: Vec<StageStatus>,
}

#[derive(Serialize)]
pub struct StageStatus {
    pub stage: store::Stage,
    pub host: &'static str,
    pub counts: store::TaskCounts,
    /// Seconds until the host is asked again after it said it was busy.
    pub paused_seconds: u64,
}

async fn overview(pool: DbPool) -> HttpResponse {
    match web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        Ok::<_, String>(EnrichmentOverview {
            settings: store::load_settings(&mut connection).map_err(|error| error.to_string())?,
            counts: store::task_counts(&mut connection).map_err(|error| error.to_string())?,
            sources: store::Stage::ALL.iter().map(|stage| stage.host()).collect(),
            stages: store::Stage::ALL
                .iter()
                .map(|stage| {
                    Ok(StageStatus {
                        stage: *stage,
                        host: stage.host(),
                        counts: store::stage_counts(&mut connection, *stage)?,
                        paused_seconds: crate::enrichment::worker::backoffs()
                            .paused_for(*stage, std::time::Instant::now())
                            .as_secs(),
                    })
                })
                .collect::<diesel::QueryResult<Vec<_>>>()
                .map_err(|error| error.to_string())?,
        })
    })
    .await
    {
        Ok(Ok(overview)) => HttpResponse::Ok().json(overview),
        _ => internal_server_error("Could not read enrichment settings.", "enrichment_failed"),
    }
}

#[get("/enrichment")]
pub async fn get_enrichment(pool: web::Data<DbPool>) -> HttpResponse {
    overview(pool.get_ref().clone()).await
}

#[derive(Deserialize)]
pub struct EnrichmentPatch {
    pub enabled: Option<bool>,
    pub language: Option<String>,
    pub artist_images: Option<bool>,
    pub genre_source: Option<crate::enrichment::genres::GenreSource>,
}

#[put("/enrichment")]
pub async fn put_enrichment(
    patch: web::Json<EnrichmentPatch>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let patch = patch.into_inner();
    if patch
        .language
        .as_deref()
        .is_some_and(|language| !crate::enrichment::wikimedia::valid_language(language))
    {
        return bad_request(
            "Choose a Wikipedia language code such as en.",
            "invalid_language",
        );
    }
    let write_pool = pool.get_ref().clone();
    let saved = web::block(move || {
        let mut connection = write_pool.get().map_err(|error| error.to_string())?;
        let mut settings =
            store::load_settings(&mut connection).map_err(|error| error.to_string())?;
        let language_changed = patch
            .language
            .as_ref()
            .is_some_and(|language| *language != settings.language);
        if let Some(enabled) = patch.enabled {
            settings.enabled = enabled;
        }
        if let Some(language) = patch.language {
            settings.language = language;
        }
        if let Some(artist_images) = patch.artist_images {
            settings.artist_images = artist_images;
        }
        if let Some(genre_source) = patch.genre_source {
            settings.genre_source = genre_source;
        }
        store::save_settings(&mut connection, &settings).map_err(|error| error.to_string())?;
        if language_changed {
            // Texts are per language; look everything up again.
            diesel::RunQueryDsl::execute(
                diesel::sql_query(
                    "UPDATE metadata_task SET status = 'pending', attempts = 0, not_before = NULL
                     WHERE (provider = 'musicbrainz' AND entity_type IN ('album', 'artist'))
                        OR (provider = 'wikidata' AND entity_type = 'genre')",
                ),
                &mut connection,
            )
            .map_err(|error| error.to_string())?;
        }
        Ok::<_, String>(())
    })
    .await;
    match saved {
        Ok(Ok(())) => {
            wake().notify_all();
            overview(pool.get_ref().clone()).await
        }
        _ => internal_server_error("Could not save enrichment settings.", "enrichment_failed"),
    }
}

#[derive(Deserialize)]
pub struct RefreshRequest {
    pub entity_type: String,
    pub entity_id: String,
}

#[actix_web::post("/enrichment/refresh")]
pub async fn refresh_enrichment(
    body: web::Json<RefreshRequest>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let RefreshRequest {
        entity_type,
        entity_id,
    } = body.into_inner();
    let entity_type = match entity_type.as_str() {
        "album" => "album",
        "artist" => "artist",
        _ => return bad_request("Refresh an album or an artist.", "invalid_entity_type"),
    };
    if entity_id.is_empty() || entity_id.len() > 128 || entity_id.starts_with(EXTERNAL_PREFIX) {
        return bad_request("That item cannot be refreshed.", "invalid_entity_id");
    }
    let pool = pool.get_ref().clone();
    match web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        store::refresh_entity(&mut connection, entity_type, &entity_id)
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(())) => {
            wake().notify_all();
            HttpResponse::Accepted().finish()
        }
        _ => internal_server_error("Could not queue the refresh.", "enrichment_failed"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use super::*;
    use crate::domain::{Credit, Song};
    use crate::library::search::SearchIndex;

    fn credit(id: &str, name: &str, role: &str, instrument: Option<&str>) -> Credit {
        Credit {
            artist_id: id.into(),
            name: name.into(),
            role: role.into(),
            instrument: instrument.map(str::to_string),
        }
    }

    fn row(
        album: &str,
        track: &str,
        name: &str,
        role: &str,
        instrument: &str,
        mbid: &str,
    ) -> CreditRow {
        CreditRow {
            album_id: album.into(),
            track_id: track.into(),
            name: name.into(),
            normalized_name: normalize_artist_identity(name),
            role: role.into(),
            instrument: instrument.into(),
            source: "musicbrainz".into(),
            source_artist_id: Some(mbid.into()),
        }
    }

    fn cache() -> LibraryCache {
        let album = |id: &str, date: &str, songs: Vec<Song>| Album {
            id: id.into(),
            name: id.into(),
            first_release_date: date.into(),
            songs,
            ..Album::default()
        };
        let miles = Artist {
            id: "miles".into(),
            name: "Miles Davis".into(),
            albums: vec![
                album(
                    "kind-of-blue",
                    "1959",
                    vec![
                        Song {
                            id: "so-what".into(),
                            name: "So What".into(),
                            track_number: 1,
                            credits: vec![credit("teo", "Teo Macero", "producer", None)],
                            ..Song::default()
                        },
                        Song {
                            id: "blue-in-green".into(),
                            name: "Blue in Green".into(),
                            track_number: 2,
                            credits: vec![
                                credit("evans", "Bill Evans", "composer", None),
                                credit("evans", "Bill Evans", "performer", Some("piano")),
                            ],
                            ..Song::default()
                        },
                    ],
                ),
                album(
                    "bitches-brew",
                    "1970",
                    vec![Song {
                        id: "pharaohs-dance".into(),
                        name: "Pharaoh's Dance".into(),
                        credits: vec![credit("teo", "Teo Macero", "producer", None)],
                        ..Song::default()
                    }],
                ),
            ],
            ..Artist::default()
        };
        let artists = vec![miles];
        LibraryCache {
            search_index: SearchIndex::build(&artists).unwrap(),
            artists: Arc::new(artists),
            song_map: HashMap::new(),
            album_genres: HashMap::new(),
            artist_positions: HashMap::from([("miles".into(), 0)]),
            album_positions: HashMap::from([
                ("kind-of-blue".into(), (0, 0)),
                ("bitches-brew".into(), (0, 1)),
            ]),
            song_positions: HashMap::from([
                ("so-what".into(), (0, 0, 0)),
                ("blue-in-green".into(), (0, 0, 1)),
                ("pharaohs-dance".into(), (0, 1, 0)),
            ]),
            songs_flat: Vec::new(),
            songs_by_artist: HashMap::new(),
            songs_by_genre: HashMap::new(),
            image_paths: Default::default(),
        }
    }

    #[test]
    fn album_credits_merge_tags_with_musicbrainz_and_link_people() {
        let cache = cache();
        let album = cache.album("kind-of-blue").unwrap();
        let known = album_known_artists(&cache, album);
        let external = [
            row(
                "kind-of-blue",
                "",
                "Irving Townsend",
                "producer",
                "",
                "mb-irving",
            ),
            row(
                "kind-of-blue",
                "blue-in-green",
                "Bill Evans",
                "performer",
                "piano",
                "mb-evans",
            ),
            row(
                "kind-of-blue",
                "so-what",
                "Bill Evans",
                "performer",
                "piano",
                "mb-evans",
            ),
            row(
                "kind-of-blue",
                "so-what",
                "Miles Davis",
                "performer",
                "trumpet",
                "mb-miles",
            ),
        ];
        let merged = merge_album_credits(album, &known, &external);
        let summary = merged
            .iter()
            .map(|credit| {
                (
                    credit.name.as_str(),
                    credit.role.as_str(),
                    credit.artist_id.as_deref(),
                    credit.track_ids.clone(),
                    credit.sources.clone(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            summary,
            [
                (
                    "Bill Evans",
                    "composer",
                    Some("evans"),
                    vec!["blue-in-green".to_string()],
                    vec!["tags".to_string()]
                ),
                (
                    "Bill Evans",
                    "performer",
                    Some("evans"),
                    vec!["so-what".to_string(), "blue-in-green".to_string()],
                    vec!["tags".to_string(), "musicbrainz".to_string()]
                ),
                (
                    "Miles Davis",
                    "performer",
                    Some("miles"),
                    vec!["so-what".to_string()],
                    vec!["musicbrainz".to_string()]
                ),
                (
                    "Teo Macero",
                    "producer",
                    Some("teo"),
                    vec!["so-what".to_string()],
                    vec!["tags".to_string()]
                ),
                (
                    "Irving Townsend",
                    "producer",
                    Some("mb:mb-irving"),
                    vec![],
                    vec!["musicbrainz".to_string()]
                ),
            ]
        );
    }

    #[test]
    fn generic_external_instruments_join_the_specific_tag_credit() {
        let album = Album {
            id: "debby".into(),
            songs: vec![Song {
                id: "waltz".into(),
                credits: vec![
                    credit("lafaro", "Scott LaFaro", "performer", Some("double bass")),
                    credit("evans", "Bill Evans", "performer", Some("piano")),
                ],
                ..Song::default()
            }],
            ..Album::default()
        };
        let external = [
            row(
                "debby",
                "waltz",
                "Scott LaFaro",
                "performer",
                "bass",
                "mb-lafaro",
            ),
            row(
                "debby",
                "waltz",
                "Bill Evans",
                "performer",
                "electric piano",
                "mb-evans",
            ),
        ];
        let merged = merge_album_credits(&album, &HashMap::new(), &external);
        let instruments = merged
            .iter()
            .map(|credit| {
                (
                    credit.name.as_str(),
                    credit.instrument.as_deref(),
                    credit.sources.len(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            instruments,
            [
                ("Scott LaFaro", Some("double bass"), 2),
                ("Bill Evans", Some("piano"), 1),
                ("Bill Evans", Some("electric piano"), 1),
            ]
        );
    }

    #[test]
    fn a_producer_links_to_everything_they_produced() {
        let cache = cache();
        let credits = collect_artist_credits(&cache, "teo", &[]).unwrap();
        assert_eq!(credits.name, "Teo Macero");
        assert!(!credits.has_discography);
        assert_eq!(
            credits
                .albums
                .iter()
                .map(|entry| entry.album.id.as_str())
                .collect::<Vec<_>>(),
            ["bitches-brew", "kind-of-blue"]
        );
        assert_eq!(
            credits.roles,
            [RoleSummary {
                role: "producer".into(),
                instrument: None,
                track_count: 2,
                album_count: 2
            }]
        );
        assert_eq!(credits.songs.len(), 2);
        assert!(credits.albums[0].album.songs.is_empty());

        let evans = collect_artist_credits(&cache, "evans", &[]).unwrap();
        assert_eq!(
            evans
                .roles
                .iter()
                .map(|role| role.role.as_str())
                .collect::<Vec<_>>(),
            ["composer", "performer"]
        );
        assert_eq!(evans.songs[0].roles.len(), 2);
        assert!(collect_artist_credits(&cache, "nobody", &[]).is_none());
    }

    #[test]
    fn external_credits_add_release_wide_and_unlinked_people() {
        let cache = cache();
        let external = [row(
            "kind-of-blue",
            "",
            "Irving Townsend",
            "producer",
            "",
            "mb-irving",
        )];
        let credits = collect_artist_credits(&cache, "mb:mb-irving", &external).unwrap();
        assert_eq!(credits.name, "Irving Townsend");
        assert_eq!(credits.albums.len(), 1);
        assert!(credits.albums[0].track_ids.is_empty());
        assert_eq!(credits.roles[0].track_count, 2);
        assert!(credits.songs.is_empty());
        assert_eq!(person_query(&cache, "mb:abc"), (None, Some("abc".into())));
        assert_eq!(
            person_query(&cache, "teo"),
            (Some("teo macero".into()), None)
        );
    }
}
