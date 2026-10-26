//! Works (compositions): the work page with every recording in the
//! library, a composer's compositions, the works a track performs, and the
//! administrator's manual "Identify album" against MusicBrainz releases.

use std::collections::{HashMap, HashSet};

use actix_web::{HttpRequest, HttpResponse, delete, get, put, web};
use serde::{Deserialize, Serialize};

use crate::api::auth::authenticated_user_id;
use crate::api::editorial::{EXTERNAL_PREFIX, person_query};
use crate::api::error::{bad_request, conflict, internal_server_error, not_found};
use crate::api::song::{ResponseSong, response_song};
use crate::domain::{Album, Artist, Song};
use crate::enrichment::fetch::{Fetch, HttpFetch, MAX_JSON_BYTES};
use crate::enrichment::identity::{PersonIdentity, load_identity};
use crate::enrichment::musicbrainz::{
    self as mb, ReleaseCandidate, identify_query, is_mbid, is_placeholder_artist, release_query_url,
};
use crate::enrichment::store::{self, CreditRow, Identification, TrackWorkRow, WorkRow};
use crate::enrichment::worker::wake;
use crate::enrichment::works::{first_year, part_title, period_for_year};
use crate::library::focus::{
    FocusRecord, FocusSummary, Selection, focus, looks_like_ensemble, parse_selection,
};
use crate::library::identity::normalize_artist_identity;
use crate::library::state::{LibraryCache, LibraryLifecycle, library_unavailable_response};
use crate::persistence::connection::DbPool;

const MAX_RECORDINGS: usize = 200;
const MAX_QUERY_CHARS: usize = 300;

/// A person on a work or recording, linked to a local artist, to a
/// MusicBrainz-only person (`mb:<id>`), or not browsable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PersonRef {
    pub artist_id: Option<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instrument: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkSummary {
    pub mbid: String,
    pub title: String,
    pub form: Option<String>,
    pub key: Option<String>,
    pub catalogue: Option<String>,
    pub composed: Option<String>,
    /// Style period from the composition year (Baroque, Classical, ...).
    pub period: Option<String>,
    pub composer: Option<PersonRef>,
    pub language: Option<String>,
    pub parent_mbid: Option<String>,
    pub part_position: Option<i64>,
}

#[derive(Clone, Serialize)]
pub struct RecordingTrack {
    pub song: ResponseSong,
    pub work_mbid: String,
    /// The movement title without the work title, e.g. "IV. Presto".
    pub part_title: String,
    pub part_position: Option<i64>,
}

/// One album's recording of a work.
#[derive(Clone, Serialize)]
pub struct Recording {
    pub album: Album,
    pub artist: Artist,
    pub year: Option<i32>,
    pub conductors: Vec<PersonRef>,
    pub ensembles: Vec<PersonRef>,
    pub soloists: Vec<PersonRef>,
    /// Track artists that are not already a conductor or ensemble.
    pub artists: Vec<PersonRef>,
    pub tracks: Vec<RecordingTrack>,
    /// Whether every known part of the work is on this album.
    pub complete: bool,
    pub duration: f64,
}

#[derive(Clone, Serialize)]
pub struct WorkPage {
    pub work: WorkSummary,
    pub parent: Option<WorkSummary>,
    pub parts: Vec<WorkSummary>,
    /// The recordings the Focus selection keeps (all of them without one).
    pub recordings: Vec<Recording>,
    /// Focus chips over every recording: conductor, ensemble, soloist,
    /// instrument, performer, format and source, with counts.
    pub focus: FocusSummary,
    pub url: String,
    /// About the work, from the Wikipedia article MusicBrainz links (a
    /// movement shows its piece's text), with CC BY-SA attribution.
    pub about: Option<store::EditorialText>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComposerWork {
    pub work: WorkSummary,
    pub track_count: i64,
    pub recording_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComposerWorks {
    pub artist_id: String,
    pub name: Option<String>,
    pub works: Vec<ComposerWork>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrackWork {
    pub track_id: String,
    pub work: WorkSummary,
    pub parent: Option<WorkSummary>,
    /// The movement title under the parent work's heading.
    pub part_title: String,
}

/// Local artists and contributors by normalized name.
pub fn person_index(cache: &LibraryCache) -> HashMap<String, String> {
    let mut index = HashMap::new();
    for artist in cache.artists.iter() {
        index
            .entry(normalize_artist_identity(&artist.name))
            .or_insert_with(|| artist.id.clone());
    }
    for song in cache
        .artists
        .iter()
        .flat_map(|artist| &artist.albums)
        .flat_map(|album| &album.songs)
    {
        for credit in &song.credits {
            index
                .entry(normalize_artist_identity(&credit.name))
                .or_insert_with(|| credit.artist_id.clone());
        }
        for (name, id) in song
            .contributing_artists
            .iter()
            .zip(&song.contributing_artist_ids)
        {
            index
                .entry(normalize_artist_identity(name))
                .or_insert_with(|| id.clone());
        }
    }
    index
}

fn person(
    people: &HashMap<String, String>,
    name: &str,
    mbid: Option<&str>,
    instrument: Option<String>,
) -> PersonRef {
    PersonRef {
        artist_id: people
            .get(&normalize_artist_identity(name))
            .cloned()
            .or_else(|| mbid.map(|mbid| format!("{EXTERNAL_PREFIX}{mbid}"))),
        name: name.to_string(),
        instrument,
    }
}

pub fn summary(row: &WorkRow, people: &HashMap<String, String>) -> WorkSummary {
    WorkSummary {
        mbid: row.mbid.clone(),
        title: row.title.clone(),
        form: row.form.clone(),
        key: row.musical_key.clone(),
        catalogue: row.catalogue.clone(),
        composed: row.composed.clone(),
        period: row
            .composed
            .as_deref()
            .and_then(first_year)
            .map(|year| period_for_year(year).to_string()),
        composer: row
            .composer_name
            .as_deref()
            .map(|name| person(people, name, row.composer_mbid.as_deref(), None)),
        language: row.language.clone(),
        parent_mbid: row.parent_mbid.clone(),
        part_position: row.part_position,
    }
}

fn song_context<'a>(
    cache: &'a LibraryCache,
    id: &str,
) -> Option<(&'a Song, &'a Album, &'a Artist)> {
    let (artist, album, song) = cache.song_positions.get(id)?;
    let artist = cache.artists.get(*artist as usize)?;
    let album = artist.albums.get(*album as usize)?;
    Some((album.songs.get(*song as usize)?, album, artist))
}

fn push_person(list: &mut Vec<PersonRef>, entry: PersonRef) {
    let key = normalize_artist_identity(&entry.name);
    if !list
        .iter()
        .any(|known| normalize_artist_identity(&known.name) == key)
    {
        list.push(entry);
    }
}

/// Groups the tracks performing a work by album and names each recording's
/// conductor, ensembles, soloists and artists from tag and MusicBrainz
/// credits.
pub fn build_recordings(
    cache: &LibraryCache,
    people: &HashMap<String, String>,
    top: &WorkRow,
    parts: &[WorkRow],
    links: &[TrackWorkRow],
    external: &HashMap<String, Vec<CreditRow>>,
) -> Vec<Recording> {
    let composer = top
        .composer_normalized
        .clone()
        .or_else(|| top.composer_name.as_deref().map(normalize_artist_identity));
    let parts_by_id = parts
        .iter()
        .map(|part| (part.mbid.as_str(), part))
        .collect::<HashMap<_, _>>();
    let mut order: Vec<String> = Vec::new();
    let mut by_album: HashMap<String, Vec<&TrackWorkRow>> = HashMap::new();
    for link in links {
        if !by_album.contains_key(&link.album_id) {
            order.push(link.album_id.clone());
        }
        by_album
            .entry(link.album_id.clone())
            .or_default()
            .push(link);
    }
    let mut recordings = Vec::new();
    for album_id in order {
        let (Some(album), Some(owner)) = (cache.album(&album_id), cache.album_owner(&album_id))
        else {
            continue;
        };
        let mut conductors = Vec::new();
        let mut ensembles = Vec::new();
        let mut soloists = Vec::new();
        let mut artists = Vec::new();
        let mut tracks = Vec::new();
        let mut seen = HashSet::new();
        let album_rows = external
            .get(&album_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for link in &by_album[&album_id] {
            if !seen.insert(link.track_id.clone()) {
                continue;
            }
            let Some(song) = album.songs.iter().find(|song| song.id == link.track_id) else {
                continue;
            };
            for credit in &song.credits {
                let entry = PersonRef {
                    artist_id: Some(credit.artist_id.clone()),
                    name: credit.name.clone(),
                    instrument: credit.instrument.clone(),
                };
                match credit.role.as_str() {
                    "conductor" => push_person(&mut conductors, entry),
                    "ensemble" => push_person(&mut ensembles, entry),
                    "performer" => push_person(&mut soloists, entry),
                    _ => {}
                }
            }
            for row in album_rows
                .iter()
                .filter(|row| row.track_id.is_empty() || row.track_id == song.id)
            {
                let instrument = Some(row.instrument.clone()).filter(|value| !value.is_empty());
                let entry = person(
                    people,
                    &row.name,
                    row.source_artist_id.as_deref(),
                    instrument,
                );
                match row.role.as_str() {
                    "conductor" => push_person(&mut conductors, entry),
                    "ensemble" => push_person(&mut ensembles, entry),
                    "performer" => push_person(&mut soloists, entry),
                    _ => {}
                }
            }
            for (name, id) in song
                .contributing_artists
                .iter()
                .zip(&song.contributing_artist_ids)
            {
                push_person(
                    &mut artists,
                    PersonRef {
                        artist_id: Some(id.clone()),
                        name: name.clone(),
                        instrument: None,
                    },
                );
            }
            let (title, position) = match parts_by_id.get(link.work_mbid.as_str()) {
                Some(part) => (part_title(&top.title, &part.title), part.part_position),
                None => (song.name.clone(), None),
            };
            tracks.push(RecordingTrack {
                song: response_song(song, album, owner),
                work_mbid: link.work_mbid.clone(),
                part_title: title,
                part_position: position,
            });
        }
        if tracks.is_empty() {
            continue;
        }
        // The track artist tag on classical albums usually repeats the
        // orchestra and conductor; list them once, in their role.
        let named = conductors
            .iter()
            .chain(&ensembles)
            .chain(&soloists)
            .map(|entry| normalize_artist_identity(&entry.name))
            .collect::<HashSet<_>>();
        artists.retain(|entry| {
            let key = normalize_artist_identity(&entry.name);
            !named.contains(&key) && Some(&key) != composer.as_ref()
        });
        // An orchestra tagged only as a track artist is the ensemble.
        if ensembles.is_empty() {
            let (groups, people): (Vec<_>, Vec<_>) = artists
                .into_iter()
                .partition(|entry| looks_like_ensemble(&entry.name));
            ensembles = groups;
            artists = people;
        }
        // An orchestra or choir credited as a performer is an ensemble.
        let (groups, people): (Vec<_>, Vec<_>) = soloists
            .into_iter()
            .partition(|entry| looks_like_ensemble(&entry.name));
        soloists = people;
        for group in groups {
            push_person(&mut ensembles, group);
        }
        soloists.retain(|entry| {
            let key = normalize_artist_identity(&entry.name);
            Some(&key) != composer.as_ref()
                && !conductors
                    .iter()
                    .any(|conductor| normalize_artist_identity(&conductor.name) == key)
        });
        let covered = tracks
            .iter()
            .map(|track| track.work_mbid.as_str())
            .collect::<HashSet<_>>();
        let complete = if parts.is_empty() {
            true
        } else {
            covered.contains(top.mbid.as_str())
                || parts
                    .iter()
                    .all(|part| covered.contains(part.mbid.as_str()))
        };
        tracks.sort_by_key(|track| {
            (
                track.part_position.unwrap_or(i64::MAX),
                track.song.track_number,
            )
        });
        recordings.push(Recording {
            year: first_year(&album.first_release_date),
            duration: tracks.iter().map(|track| track.song.duration).sum(),
            album: crate::api::album_reference(album),
            artist: crate::api::artist_reference(owner),
            conductors,
            ensembles,
            soloists,
            artists,
            tracks,
            complete,
        });
        if recordings.len() >= MAX_RECORDINGS {
            break;
        }
    }
    recordings.sort_by(|left, right| {
        right
            .complete
            .cmp(&left.complete)
            .then_with(|| {
                left.year
                    .unwrap_or(i32::MAX)
                    .cmp(&right.year.unwrap_or(i32::MAX))
            })
            .then_with(|| left.album.name.cmp(&right.album.name))
    });
    recordings
}

/// Links an album's tracks to their works, for headings in the track list.
pub fn album_track_works(
    links: &[TrackWorkRow],
    works: &[WorkRow],
    people: &HashMap<String, String>,
) -> Vec<TrackWork> {
    let by_id = works
        .iter()
        .map(|work| (work.mbid.as_str(), work))
        .collect::<HashMap<_, _>>();
    let mut result: Vec<TrackWork> = Vec::new();
    for link in links {
        let Some(work) = by_id.get(link.work_mbid.as_str()) else {
            continue;
        };
        if result.iter().any(|known| known.track_id == link.track_id) {
            continue;
        }
        let parent = work
            .parent_mbid
            .as_deref()
            .and_then(|parent| by_id.get(parent));
        result.push(TrackWork {
            track_id: link.track_id.clone(),
            work: summary(work, people),
            parent: parent.map(|parent| summary(parent, people)),
            part_title: match parent {
                Some(parent) => part_title(&parent.title, &work.title),
                None => work.title.clone(),
            },
        });
    }
    result
}

/// Track-to-work links for an album, ready for the album page.
pub fn load_album_track_works(
    connection: &mut diesel::SqliteConnection,
    album_id: &str,
) -> diesel::QueryResult<(Vec<TrackWorkRow>, Vec<WorkRow>)> {
    let links = store::album_track_works(connection, album_id)?;
    let mut ids = links
        .iter()
        .map(|link| link.work_mbid.clone())
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    let mut works = store::works_by_ids(connection, &ids)?;
    let parents = works
        .iter()
        .filter_map(|work| work.parent_mbid.clone())
        .filter(|parent| !ids.contains(parent))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    works.extend(store::works_by_ids(connection, &parents)?);
    Ok((links, works))
}

struct WorkData {
    work: WorkRow,
    parent: Option<WorkRow>,
    top: WorkRow,
    parts: Vec<WorkRow>,
    links: Vec<TrackWorkRow>,
    external: HashMap<String, Vec<CreditRow>>,
    about: Option<store::EditorialText>,
}

fn load_work_data(
    connection: &mut diesel::SqliteConnection,
    mbid: &str,
) -> diesel::QueryResult<Option<WorkData>> {
    let Some(work) = store::load_work(connection, mbid)? else {
        return Ok(None);
    };
    let parent = match &work.parent_mbid {
        Some(parent) => store::load_work(connection, parent)?,
        None => None,
    };
    // A movement's page shows its own recordings; the piece's page shows
    // every recording of any of its movements.
    let top = work.clone();
    let parts = store::work_parts(connection, &top.mbid)?;
    let links = store::work_tracks(connection, &top.mbid)?;
    let mut external = HashMap::new();
    for album_id in links
        .iter()
        .map(|link| link.album_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .take(MAX_RECORDINGS)
    {
        let rows = store::album_credits(connection, &album_id)?;
        external.insert(album_id, rows);
    }
    let mut about = store::work_background(connection, &work.mbid)?;
    if about.is_none()
        && let Some(parent) = &parent
    {
        about = store::work_background(connection, &parent.mbid)?;
    }
    Ok(Some(WorkData {
        work,
        parent,
        top,
        parts,
        links,
        external,
        about,
    }))
}

async fn blocking<T: Send + 'static>(
    pool: &DbPool,
    work: impl FnOnce(&mut diesel::SqliteConnection) -> diesel::QueryResult<T> + Send + 'static,
) -> Result<T, String> {
    let pool = pool.clone();
    web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        work(&mut connection).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// One person per id in a list, after merging ids that are one person.
fn canonical_people(identity: &PersonIdentity, people: &mut Vec<PersonRef>, album_id: &str) {
    let mut seen = HashSet::new();
    people.retain_mut(|person| {
        if let Some(id) = &person.artist_id {
            let canonical = identity.for_local(id, &person.name, Some(album_id));
            if (canonical != *id || canonical.starts_with(EXTERNAL_PREFIX))
                && let Some(name) = identity.display_name(&canonical)
            {
                person.name = name.to_string();
            }
            person.artist_id = Some(canonical);
        }
        let key = person
            .artist_id
            .clone()
            .unwrap_or_else(|| normalize_artist_identity(&person.name));
        seen.insert((key, person.instrument.clone()))
    });
}

/// Resolves every person on the recordings to one identity each.
pub fn canonical_recordings(identity: &PersonIdentity, recordings: &mut [Recording]) {
    for recording in recordings {
        let album_id = recording.album.id.clone();
        for list in [
            &mut recording.conductors,
            &mut recording.ensembles,
            &mut recording.soloists,
            &mut recording.artists,
        ] {
            canonical_people(identity, list, &album_id);
        }
    }
}

/// A recording's Focus record.
pub fn recording_record(recording: &Recording, composer: Option<&PersonRef>) -> FocusRecord {
    let mut record = FocusRecord::default();
    for person in &recording.conductors {
        record.add_person("conductor", person.artist_id.as_deref(), &person.name);
    }
    for person in &recording.ensembles {
        record.add_person("ensemble", person.artist_id.as_deref(), &person.name);
    }
    for person in &recording.soloists {
        record.add_person("soloist", person.artist_id.as_deref(), &person.name);
        if let Some(instrument) = &person.instrument {
            record.add("instrument", &instrument.to_lowercase(), instrument, None);
        }
    }
    for person in &recording.artists {
        // Tags often name the orchestra only as a track artist.
        let group = if looks_like_ensemble(&person.name) {
            "ensemble"
        } else {
            "performer"
        };
        record.add_person(group, person.artist_id.as_deref(), &person.name);
    }
    if let Some(composer) = composer {
        record.add_person("composer", composer.artist_id.as_deref(), &composer.name);
    }
    for track in &recording.tracks {
        record.add_track(track.song.audio.as_ref(), &track.song.path);
    }
    record
}

/// Keeps the recordings a Focus selection keeps, with the chip counts.
pub fn focus_recordings(
    recordings: Vec<Recording>,
    composer: Option<&PersonRef>,
    selection: &Selection,
) -> (Vec<Recording>, FocusSummary) {
    let records = recordings
        .iter()
        .map(|recording| recording_record(recording, composer))
        .collect::<Vec<_>>();
    let (matched, summary) = focus(&records, selection);
    let matched = matched.into_iter().collect::<HashSet<_>>();
    let kept = recordings
        .into_iter()
        .enumerate()
        .filter(|(index, _)| matched.contains(index))
        .map(|(_, recording)| recording)
        .collect();
    (kept, summary)
}

#[get("/{mbid}")]
pub async fn get_work(
    mbid: web::Path<String>,
    query: web::Query<Vec<(String, String)>>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let mbid = mbid.into_inner();
    let selection = parse_selection(&query.into_inner());
    if !is_mbid(&mbid) {
        return not_found("Work not found.", "work_not_found");
    }
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    let lookup = mbid.clone();
    let data = match blocking(pool.get_ref(), move |connection| {
        load_work_data(connection, &lookup)
    })
    .await
    {
        Ok(Some(data)) => data,
        Ok(None) => return not_found("Work not found.", "work_not_found"),
        Err(error) => {
            tracing::error!(%error, "could not load a work");
            return internal_server_error("Could not load the work.", "work_failed");
        }
    };
    let people = person_index(&cache);
    let identity = load_identity(pool.get_ref().clone(), cache.clone()).await;
    let mut recordings = build_recordings(
        &cache,
        &people,
        &data.top,
        &data.parts,
        &data.links,
        &data.external,
    );
    canonical_recordings(&identity, &mut recordings);
    let mut work = summary(&data.work, &people);
    if let Some(composer) = work.composer.as_mut() {
        if let Some(mbid) = data.work.composer_mbid.as_deref() {
            composer.artist_id = Some(identity.for_mbid(mbid));
        } else if let Some(id) = composer.artist_id.clone() {
            composer.artist_id = Some(identity.for_local(&id, &composer.name, None));
        }
    }
    let (recordings, focus) = focus_recordings(recordings, work.composer.as_ref(), &selection);
    // Only movements someone has a recording of are listed.
    let recorded = data
        .links
        .iter()
        .map(|link| link.work_mbid.as_str())
        .collect::<HashSet<_>>();
    HttpResponse::Ok().json(WorkPage {
        work,
        focus,
        parent: data.parent.as_ref().map(|parent| summary(parent, &people)),
        parts: data
            .parts
            .iter()
            .filter(|part| recorded.contains(part.mbid.as_str()))
            .map(|part| summary(part, &people))
            .collect(),
        recordings,
        url: format!("https://musicbrainz.org/work/{mbid}"),
        about: data.about,
    })
}

#[get("/{id}/works")]
pub async fn get_artist_works(
    id: web::Path<String>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let artist_id = id.into_inner();
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    let (normalized, external) = person_query(&cache, &artist_id);
    let identity = load_identity(pool.get_ref().clone(), cache.clone()).await;
    let members = identity.members(&artist_id);
    let artist_id = members.canonical.clone();
    let name = cache
        .artist(&artist_id)
        .map(|artist| artist.name.clone())
        .or_else(|| identity.display_name(&artist_id).map(str::to_string));
    let local = members.local_ids.clone();
    let mut names = members.names.clone();
    names.extend(normalized);
    let mut mbids = members.mbids.clone();
    mbids.extend(external);
    let rows = blocking(pool.get_ref(), move |connection| {
        for id in local.iter().filter(|id| !id.starts_with(EXTERNAL_PREFIX)) {
            mbids.extend(store::linked_artist_mbids(connection, id)?);
        }
        mbids.sort();
        mbids.dedup();
        names.sort();
        names.dedup();
        let mut found: Vec<(WorkRow, i64, i64)> = Vec::new();
        let lookups = if names.is_empty() {
            vec![None]
        } else {
            names.iter().map(|name| Some(name.as_str())).collect()
        };
        for name in lookups.into_iter().take(8) {
            for row in store::composer_works(connection, name, &mbids)? {
                if !found.iter().any(|(known, _, _)| known.mbid == row.0.mbid) {
                    found.push(row);
                }
            }
        }
        found.sort_by(|left, right| left.0.title.cmp(&right.0.title));
        Ok(found)
    })
    .await;
    let mut rows = match rows {
        Ok(rows) => rows,
        Err(error) => {
            tracing::error!(%error, "could not load compositions");
            return internal_server_error("Could not load compositions.", "works_failed");
        }
    };
    // A namesake's compositions (same name, another MusicBrainz person)
    // are not this person's.
    rows.retain(|(work, _, _)| {
        work.composer_mbid
            .as_deref()
            .is_none_or(|mbid| identity.for_mbid(mbid) == artist_id)
    });
    let people = person_index(&cache);
    let name = name.or_else(|| {
        rows.first()
            .and_then(|(work, _, _)| work.composer_name.clone())
    });
    HttpResponse::Ok().json(ComposerWorks {
        artist_id,
        name,
        works: rows
            .iter()
            .filter(|(_, tracks, _)| *tracks > 0)
            .map(|(work, tracks, albums)| ComposerWork {
                work: summary(work, &people),
                track_count: *tracks,
                recording_count: *albums,
            })
            .collect(),
    })
}

#[get("/{id}/works")]
pub async fn get_song_works(
    id: web::Path<String>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let song_id = id.into_inner();
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    if song_context(&cache, &song_id).is_none() {
        return not_found("Song not found.", "song_not_found");
    }
    let lookup = song_id.clone();
    let rows = match blocking(pool.get_ref(), move |connection| {
        store::track_works(connection, &lookup)
    })
    .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::error!(%error, "could not load a track's works");
            return internal_server_error("Could not load the work.", "works_failed");
        }
    };
    let people = person_index(&cache);
    let works = rows
        .iter()
        .map(|(work, parent)| TrackWork {
            track_id: song_id.clone(),
            work: summary(work, &people),
            parent: parent.as_ref().map(|parent| summary(parent, &people)),
            part_title: match parent {
                Some(parent) => part_title(&parent.title, &work.title),
                None => work.title.clone(),
            },
        })
        .collect::<Vec<_>>();
    HttpResponse::Ok().json(serde_json::json!({ "song_id": song_id, "works": works }))
}

#[derive(Debug, Clone, Serialize)]
pub struct ReleaseMatch {
    pub mbid: String,
    pub title: String,
    pub artist: String,
    pub date: Option<String>,
    pub country: Option<String>,
    pub track_count: Option<i64>,
    pub labels: Vec<String>,
    pub catalog_numbers: Vec<String>,
    pub format: Option<String>,
    pub release_type: Option<String>,
    pub disambiguation: Option<String>,
    pub score: i64,
    /// Same number of tracks as the local album.
    pub same_track_count: bool,
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct IdentifySearch {
    pub album_id: String,
    pub query: String,
    pub title: String,
    pub artist: String,
    pub local_track_count: usize,
    pub identification: Option<Identification>,
    pub releases: Vec<ReleaseMatch>,
}

/// Orders candidates for a person to choose from: same track count first,
/// then MusicBrainz' own score.
pub fn rank_releases(candidates: Vec<ReleaseCandidate>, track_count: usize) -> Vec<ReleaseMatch> {
    let mut releases = candidates
        .into_iter()
        .map(|candidate| ReleaseMatch {
            same_track_count: candidate.track_count == Some(track_count as i64),
            url: mb::release_page_url(&candidate.mbid),
            mbid: candidate.mbid,
            title: candidate.title,
            artist: candidate.artist,
            date: candidate.date,
            country: candidate.country,
            track_count: candidate.track_count,
            labels: candidate.labels,
            catalog_numbers: candidate.catalog_numbers,
            format: candidate.format,
            release_type: candidate.release_type,
            disambiguation: candidate.disambiguation,
            score: candidate.score,
        })
        .collect::<Vec<_>>();
    releases.sort_by(|left, right| {
        right
            .same_track_count
            .cmp(&left.same_track_count)
            .then_with(|| right.score.cmp(&left.score))
    });
    releases
}

/// The default search for an album: its title, and its artist unless the
/// artist is a placeholder such as "Unknown Artist".
pub fn default_identify_query(title: &str, artist: &str) -> String {
    identify_query(
        title,
        Some(artist).filter(|artist| !is_placeholder_artist(artist)),
    )
}

pub async fn search_releases<F: Fetch>(
    fetch: &F,
    query: &str,
) -> Result<Vec<ReleaseCandidate>, String> {
    let response = fetch
        .get(&release_query_url(query), MAX_JSON_BYTES)
        .await
        .map_err(|error| error.to_string())?;
    if !response.is_success() {
        return Err(format!("MusicBrainz answered HTTP {}", response.status));
    }
    let body = serde_json::from_slice::<serde_json::Value>(&response.body)
        .map_err(|error| error.to_string())?;
    Ok(mb::parse_release_search(&body))
}

#[derive(Deserialize)]
pub struct IdentifyQuery {
    pub album_id: String,
    /// A raw MusicBrainz search (Lucene syntax).
    pub query: Option<String>,
    /// Or a title and artist to search for; default: the album's own. An
    /// empty artist searches by title alone.
    pub title: Option<String>,
    pub artist: Option<String>,
}

#[get("/identify")]
pub async fn identify_search(
    query: web::Query<IdentifyQuery>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let IdentifyQuery {
        album_id,
        query,
        title,
        artist,
    } = query.into_inner();
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    let (Some(album), Some(owner)) = (cache.album(&album_id), cache.album_owner(&album_id)) else {
        return not_found("Album not found.", "album_not_found");
    };
    let trimmed = |value: Option<String>| {
        value
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let title = trimmed(title).unwrap_or_else(|| album.name.clone());
    // An artist sent empty means "search by title alone".
    let artist = match artist {
        Some(artist) => artist.trim().to_string(),
        None if is_placeholder_artist(&owner.name) => String::new(),
        None => owner.name.clone(),
    };
    let query = trimmed(query).unwrap_or_else(|| default_identify_query(&title, &artist));
    if query.chars().count() > MAX_QUERY_CHARS {
        return bad_request("The search is too long.", "query_too_long");
    }
    let local_track_count = album.songs.len();
    let lookup = album_id.clone();
    let (settings, identification) = match blocking(pool.get_ref(), move |connection| {
        Ok((
            store::load_settings(connection)?,
            store::load_identification(connection, &lookup)?,
        ))
    })
    .await
    {
        Ok(found) => found,
        Err(error) => {
            tracing::error!(%error, "could not read enrichment settings");
            return internal_server_error("Could not search.", "identify_failed");
        }
    };
    if !settings.enabled {
        return conflict(
            "Turn on online metadata in Settings to search MusicBrainz.",
            "enrichment_disabled",
        );
    }
    let fetch = match HttpFetch::new() {
        Ok(fetch) => fetch,
        Err(error) => {
            tracing::error!(%error, "could not start the MusicBrainz client");
            return internal_server_error("Could not search.", "identify_failed");
        }
    };
    match search_releases(&fetch, &query).await {
        Ok(candidates) => HttpResponse::Ok().json(IdentifySearch {
            album_id,
            query,
            title,
            artist,
            local_track_count,
            identification,
            releases: rank_releases(candidates, local_track_count),
        }),
        Err(error) => {
            tracing::warn!(%error, "MusicBrainz release search failed");
            crate::api::error::service_unavailable(
                "MusicBrainz did not answer. Try again in a moment.",
                "musicbrainz_unavailable",
            )
        }
    }
}

#[derive(Deserialize)]
pub struct IdentifyBody {
    pub release_mbid: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
}

#[put("/identify/{album_id}")]
pub async fn identify_album(
    request: HttpRequest,
    album_id: web::Path<String>,
    body: web::Json<IdentifyBody>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let album_id = album_id.into_inner();
    let body = body.into_inner();
    if !is_mbid(&body.release_mbid) {
        return bad_request("Choose a MusicBrainz release.", "invalid_release");
    }
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    if cache.album(&album_id).is_none() {
        return not_found("Album not found.", "album_not_found");
    }
    let user_id = authenticated_user_id(&request).ok();
    let title = body.title.chars().take(500).collect::<String>();
    let artist = body.artist.chars().take(500).collect::<String>();
    let lookup = album_id.clone();
    match blocking(pool.get_ref(), move |connection| {
        store::set_identification(
            connection,
            &lookup,
            Some((&body.release_mbid, &title, &artist)),
            user_id,
        )?;
        store::load_identification(connection, &lookup)
    })
    .await
    {
        Ok(identification) => {
            wake().notify_all();
            HttpResponse::Accepted().json(serde_json::json!({ "identification": identification }))
        }
        Err(error) => {
            tracing::error!(%error, "could not identify an album");
            internal_server_error("Could not save the match.", "identify_failed")
        }
    }
}

#[delete("/identify/{album_id}")]
pub async fn clear_identification(
    album_id: web::Path<String>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let album_id = album_id.into_inner();
    if album_id.is_empty() || album_id.len() > 128 {
        return bad_request("That album cannot be changed.", "invalid_album");
    }
    match blocking(pool.get_ref(), move |connection| {
        store::set_identification(connection, &album_id, None, None)
    })
    .await
    {
        Ok(()) => {
            wake().notify_all();
            HttpResponse::Accepted().finish()
        }
        Err(error) => {
            tracing::error!(%error, "could not clear an album match");
            internal_server_error("Could not clear the match.", "identify_failed")
        }
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/works").service(get_work));
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::domain::Credit;
    use crate::enrichment::fetch::mock::MockFetch;
    use crate::library::search::SearchIndex;

    const SYMPHONY: &str = "22222222-2222-4222-8222-222222222222";
    const BEETHOVEN: &str = "1f9df192-a621-4f54-8850-2c5373b7eac9";

    fn credit(id: &str, name: &str, role: &str) -> Credit {
        Credit {
            artist_id: id.into(),
            name: name.into(),
            role: role.into(),
            instrument: None,
        }
    }

    fn song(id: &str, number: u16, credits: Vec<Credit>, artists: &[(&str, &str)]) -> Song {
        Song {
            id: id.into(),
            name: format!("Track {number}"),
            track_number: number,
            duration: 60.0,
            credits,
            contributing_artists: artists.iter().map(|(name, _)| name.to_string()).collect(),
            contributing_artist_ids: artists.iter().map(|(_, id)| id.to_string()).collect(),
            ..Song::default()
        }
    }

    fn cache() -> LibraryCache {
        let karajan = Artist {
            id: "karajan".into(),
            name: "Herbert von Karajan".into(),
            albums: vec![Album {
                id: "k-album".into(),
                name: "Beethoven: Symphony No. 9".into(),
                first_release_date: "1963".into(),
                songs: vec![
                    song(
                        "k1",
                        1,
                        vec![credit("beethoven", "Ludwig van Beethoven", "composer")],
                        &[
                            ("Berliner Philharmoniker", "bpo"),
                            ("Herbert von Karajan", "karajan"),
                        ],
                    ),
                    song(
                        "k4",
                        4,
                        vec![credit("beethoven", "Ludwig van Beethoven", "composer")],
                        &[
                            ("Berliner Philharmoniker", "bpo"),
                            ("Herbert von Karajan", "karajan"),
                        ],
                    ),
                ],
                ..Album::default()
            }],
            ..Artist::default()
        };
        let bernstein = Artist {
            id: "bernstein".into(),
            name: "Leonard Bernstein".into(),
            albums: vec![Album {
                id: "b-album".into(),
                name: "Ode to Freedom".into(),
                first_release_date: "1990".into(),
                songs: vec![song(
                    "b4",
                    1,
                    vec![credit("bernstein", "Leonard Bernstein", "conductor")],
                    &[("Leonard Bernstein", "bernstein")],
                )],
                ..Album::default()
            }],
            ..Artist::default()
        };
        let artists = vec![karajan, bernstein];
        LibraryCache {
            search_index: SearchIndex::build(&artists).unwrap(),
            artists: Arc::new(artists),
            song_map: HashMap::new(),
            album_genres: HashMap::new(),
            artist_positions: HashMap::from([("karajan".into(), 0), ("bernstein".into(), 1)]),
            album_positions: HashMap::from([
                ("k-album".into(), (0, 0)),
                ("b-album".into(), (1, 0)),
            ]),
            song_positions: HashMap::from([
                ("k1".into(), (0, 0, 0)),
                ("k4".into(), (0, 0, 1)),
                ("b4".into(), (1, 0, 0)),
            ]),
            songs_flat: Vec::new(),
            songs_by_artist: HashMap::new(),
            songs_by_genre: HashMap::new(),
            image_paths: Default::default(),
        }
    }

    fn work(mbid: &str, title: &str, parent: Option<&str>, position: Option<i64>) -> WorkRow {
        WorkRow {
            mbid: mbid.into(),
            title: title.into(),
            composed: Some("1822–1824".into()),
            composer_name: Some("Ludwig van Beethoven".into()),
            composer_normalized: Some("ludwig van beethoven".into()),
            composer_mbid: Some(BEETHOVEN.into()),
            parent_mbid: parent.map(str::to_string),
            part_position: position,
            ..WorkRow::default()
        }
    }

    fn link(track: &str, album: &str, work: &str) -> TrackWorkRow {
        TrackWorkRow {
            track_id: track.into(),
            album_id: album.into(),
            work_mbid: work.into(),
            recording_mbid: None,
        }
    }

    fn recording(
        album_id: &str,
        conductor: &str,
        orchestra: &str,
        bits: u8,
        rate: u32,
    ) -> Recording {
        let album = Album {
            id: album_id.into(),
            name: album_id.into(),
            ..Album::default()
        };
        let artist = Artist {
            id: conductor.into(),
            name: conductor.into(),
            ..Artist::default()
        };
        let song = Song {
            id: format!("{album_id}-1"),
            name: "I. Allegro con brio".into(),
            path: format!("/music/{album_id}/01.flac"),
            audio: Some(crate::domain::AudioProperties {
                codec: Some("flac".into()),
                sample_rate: Some(rate),
                bit_depth: Some(bits),
                lossless: Some(true),
                ..Default::default()
            }),
            ..Song::default()
        };
        Recording {
            year: None,
            conductors: vec![PersonRef {
                artist_id: Some(conductor.into()),
                name: conductor.into(),
                instrument: None,
            }],
            ensembles: Vec::new(),
            soloists: Vec::new(),
            artists: vec![PersonRef {
                artist_id: Some(orchestra.into()),
                name: orchestra.into(),
                instrument: None,
            }],
            tracks: vec![RecordingTrack {
                song: response_song(&song, &album, &artist),
                work_mbid: SYMPHONY.into(),
                part_title: "I. Allegro con brio".into(),
                part_position: Some(1),
            }],
            complete: true,
            duration: 0.0,
            album,
            artist,
        }
    }

    #[test]
    fn recordings_focus_on_conductor_orchestra_and_resolution() {
        let recordings = vec![
            recording(
                "karajan-po-96",
                "karajan",
                "Philharmonia Orchestra",
                24,
                96_000,
            ),
            recording(
                "karajan-po-44",
                "karajan",
                "Philharmonia Orchestra",
                16,
                44_100,
            ),
            recording(
                "karajan-bp-192",
                "karajan",
                "Berliner Philharmoniker",
                24,
                192_000,
            ),
            recording(
                "kleiber-wp-96",
                "kleiber",
                "Wiener Philharmoniker",
                24,
                96_000,
            ),
        ];
        let selection = parse_selection(&[
            ("conductor".into(), "karajan".into()),
            ("ensemble".into(), "Philharmonia Orchestra".into()),
            ("quality".into(), "hires".into()),
        ]);
        let (kept, summary) = focus_recordings(recordings, None, &selection);
        assert_eq!(
            kept.iter()
                .map(|recording| recording.album.id.as_str())
                .collect::<Vec<_>>(),
            vec!["karajan-po-96"]
        );
        assert_eq!(summary.total, 4);
        assert_eq!(summary.matched, 1);
        // The orchestra billed as a track artist is an ensemble chip.
        let ensembles = summary
            .groups
            .iter()
            .find(|group| group.key == "ensemble")
            .unwrap();
        assert!(
            ensembles
                .values
                .iter()
                .any(|chip| chip.label == "Philharmonia Orchestra")
        );
        // Every chip left would show the same one recording.
        assert!(
            summary
                .groups
                .iter()
                .all(|group| group.key != "sample_rate")
        );
        let all = vec![
            recording("a", "karajan", "Philharmonia Orchestra", 24, 96_000),
            recording("b", "karajan", "Berliner Philharmoniker", 24, 192_000),
        ];
        let (_, summary) = focus_recordings(all, None, &Selection::new());
        let rates = summary
            .groups
            .iter()
            .find(|group| group.key == "sample_rate")
            .unwrap();
        assert_eq!(rates.values[0].label, "192 kHz");
    }

    #[test]
    fn merged_people_are_listed_once_on_a_recording() {
        use crate::enrichment::identity::tests::{QTIP, identity};
        let identity = identity();
        let mut recordings = vec![recording("low-end", "x", "y", 16, 44_100)];
        recordings[0].soloists = vec![
            PersonRef {
                artist_id: Some("tag-qtip".into()),
                name: "Q-Tip".into(),
                instrument: None,
            },
            PersonRef {
                artist_id: Some(format!("mb:{QTIP}")),
                name: "Jonathan Davis".into(),
                instrument: None,
            },
        ];
        canonical_recordings(&identity, &mut recordings);
        assert_eq!(recordings[0].soloists.len(), 1);
        assert_eq!(
            recordings[0].soloists[0].artist_id.as_deref(),
            Some("tag-qtip")
        );
    }

    #[test]
    fn a_work_lists_each_album_recording_with_its_conductor_and_orchestra() {
        let cache = cache();
        let people = person_index(&cache);
        let top = work(SYMPHONY, "Symphony No. 9 in D minor, Op. 125", None, None);
        let parts = vec![
            work(
                "m1",
                "Symphony No. 9 in D minor, Op. 125: I. Allegro",
                Some(SYMPHONY),
                Some(1),
            ),
            work(
                "m4",
                "Symphony No. 9 in D minor, Op. 125: IV. Presto",
                Some(SYMPHONY),
                Some(4),
            ),
        ];
        let links = vec![
            link("k4", "k-album", "m4"),
            link("k1", "k-album", "m1"),
            link("b4", "b-album", "m4"),
        ];
        let external = HashMap::from([(
            "k-album".to_string(),
            vec![
                CreditRow {
                    album_id: "k-album".into(),
                    track_id: String::new(),
                    name: "Herbert von Karajan".into(),
                    normalized_name: "herbert von karajan".into(),
                    role: "conductor".into(),
                    instrument: String::new(),
                    source: "musicbrainz".into(),
                    source_artist_id: None,
                },
                CreditRow {
                    album_id: "k-album".into(),
                    track_id: "k4".into(),
                    name: "Berliner Philharmoniker".into(),
                    normalized_name: "berliner philharmoniker".into(),
                    role: "ensemble".into(),
                    instrument: String::new(),
                    source: "musicbrainz".into(),
                    source_artist_id: Some("33333333-3333-4333-8333-333333333333".into()),
                },
                CreditRow {
                    album_id: "k-album".into(),
                    track_id: "k4".into(),
                    name: "Gundula Janowitz".into(),
                    normalized_name: "gundula janowitz".into(),
                    role: "performer".into(),
                    instrument: "soprano vocals".into(),
                    source: "musicbrainz".into(),
                    source_artist_id: Some("44444444-4444-4444-8444-444444444444".into()),
                },
            ],
        )]);
        let recordings = build_recordings(&cache, &people, &top, &parts, &links, &external);
        assert_eq!(recordings.len(), 2);
        let karajan = &recordings[0];
        assert_eq!(karajan.album.id, "k-album");
        assert!(karajan.complete);
        assert_eq!(karajan.year, Some(1963));
        assert_eq!(
            karajan
                .tracks
                .iter()
                .map(|track| (track.song.id.as_str(), track.part_title.as_str()))
                .collect::<Vec<_>>(),
            [("k1", "I. Allegro"), ("k4", "IV. Presto")]
        );
        assert_eq!(karajan.conductors[0].artist_id.as_deref(), Some("karajan"));
        assert_eq!(karajan.ensembles[0].artist_id.as_deref(), Some("bpo"));
        assert_eq!(
            karajan.soloists[0].artist_id.as_deref(),
            Some("mb:44444444-4444-4444-8444-444444444444")
        );
        assert_eq!(
            karajan.soloists[0].instrument.as_deref(),
            Some("soprano vocals")
        );
        // Orchestra and conductor are listed in their roles, not again as
        // track artists.
        assert!(karajan.artists.is_empty());
        assert_eq!(karajan.duration, 120.0);
        let bernstein = &recordings[1];
        assert!(!bernstein.complete);
        assert_eq!(bernstein.conductors[0].name, "Leonard Bernstein");
    }

    #[test]
    fn album_tracks_carry_their_movement_and_piece() {
        let people = person_index(&cache());
        let works = vec![
            work(SYMPHONY, "Symphony No. 9 in D minor, Op. 125", None, None),
            work(
                "m1",
                "Symphony No. 9 in D minor, Op. 125: I. Allegro",
                Some(SYMPHONY),
                Some(1),
            ),
        ];
        let linked = album_track_works(&[link("k1", "k-album", "m1")], &works, &people);
        assert_eq!(linked.len(), 1);
        assert_eq!(linked[0].part_title, "I. Allegro");
        let parent = linked[0].parent.as_ref().unwrap();
        assert_eq!(parent.mbid, SYMPHONY);
        assert_eq!(parent.period.as_deref(), Some("Classical"));
        assert_eq!(
            parent
                .composer
                .as_ref()
                .and_then(|composer| composer.artist_id.as_deref()),
            Some("beethoven")
        );
    }

    #[test]
    fn a_movement_page_shows_its_piece_about_text() {
        let pool = crate::persistence::connection::migrated_memory_pool();
        let mut connection = pool.get().unwrap();
        let works = vec![
            work(SYMPHONY, "Symphony No. 9 in D minor, Op. 125", None, None),
            work(
                "m1",
                "Symphony No. 9 in D minor, Op. 125: I. Allegro",
                Some(SYMPHONY),
                Some(1),
            ),
        ];
        store::replace_album_works(
            &mut connection,
            "k-album",
            &works,
            &[link("k1", "k-album", "m1")],
        )
        .unwrap();
        assert!(
            load_work_data(&mut connection, "m1")
                .unwrap()
                .unwrap()
                .about
                .is_none()
        );
        let text = store::EditorialText {
            kind: "background".into(),
            source: "wikipedia".into(),
            language: "en".into(),
            title: "Symphony No. 9 (Beethoven)".into(),
            body: "The Symphony No. 9 is a choral symphony.".into(),
            url: "https://en.wikipedia.org/wiki/Symphony_No._9_(Beethoven)".into(),
            license: "CC BY-SA 4.0".into(),
            license_url: "https://creativecommons.org/licenses/by-sa/4.0/".into(),
        };
        store::replace_texts(
            &mut connection,
            "work",
            SYMPHONY,
            std::slice::from_ref(&text),
        )
        .unwrap();
        for mbid in [SYMPHONY, "m1"] {
            let data = load_work_data(&mut connection, mbid).unwrap().unwrap();
            assert_eq!(data.about.as_ref(), Some(&text), "{mbid}");
        }
    }

    #[test]
    fn identify_queries_leave_out_placeholder_artists() {
        assert_eq!(
            default_identify_query("Basement Tapes", "Unknown Artist"),
            "release:\"Basement Tapes\""
        );
        assert_eq!(
            default_identify_query("Beethoven: Symphony No. 9", "Herbert von Karajan"),
            "release:(\"Beethoven: Symphony No. 9\" OR \"Symphony No. 9\") AND artist:\"Herbert von Karajan\""
        );
    }

    #[tokio::test]
    async fn release_search_ranks_matching_track_counts_first() {
        let query = "beethoven 9 karajan 1963";
        let fetch = MockFetch::default().json(
            &release_query_url(query),
            serde_json::json!({"releases": [
                {"id": "55555555-5555-4555-8555-555555555555", "title": "Symphonie Nr. 9", "score": 100,
                 "track-count": 5, "country": "DE", "date": "1963",
                 "artist-credit": [{"name": "Beethoven", "joinphrase": "; "}, {"name": "Karajan"}],
                 "media": [{"format": "Vinyl"}, {"format": "Vinyl"}],
                 "label-info": [{"catalog-number": "138 807", "label": {"name": "Deutsche Grammophon"}}],
                 "release-group": {"primary-type": "Album"}},
                {"id": "66666666-6666-4666-8666-666666666666", "title": "Symphony No. 9", "score": 92,
                 "track-count": 4, "media": [{"format": "CD"}]}
            ]}),
        );
        let found = search_releases(&fetch, query).await.unwrap();
        let ranked = rank_releases(found, 4);
        assert_eq!(ranked[0].mbid, "66666666-6666-4666-8666-666666666666");
        assert!(ranked[0].same_track_count);
        let vinyl = &ranked[1];
        assert_eq!(vinyl.format.as_deref(), Some("2×Vinyl"));
        assert_eq!(vinyl.labels, ["Deutsche Grammophon"]);
        assert_eq!(vinyl.catalog_numbers, ["138 807"]);
        assert_eq!(vinyl.artist, "Beethoven; Karajan");
        assert_eq!(
            vinyl.url,
            "https://musicbrainz.org/release/55555555-5555-4555-8555-555555555555"
        );
        let failing = MockFetch::default().status(&release_query_url(query), 503);
        assert!(search_releases(&failing, query).await.is_err());
    }
}
