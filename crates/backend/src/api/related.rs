//! Related people: the discovery graph on person pages.
//!
//! - Collaborators: people who made music with the person, ranked by the
//!   releases and tracks they share in the library's tag and MusicBrainz
//!   credits.
//! - Top performers (for someone whose music others record): the
//!   conductors, ensembles and soloists on the most recordings of their
//!   works.
//! - Performing the music of: the reverse, the composers whose works the
//!   person records.
//! - Influences and followers: Wikidata's "influenced by" (P737) and its
//!   reverse, when online metadata is on.
//!
//! Everything but influences is computed from what the library already
//! holds, so it works with online metadata off (from tags alone) and gets
//! richer once MusicBrainz credits and works are stored.

use std::collections::{BTreeSet, HashMap, HashSet};

use actix_web::{HttpResponse, get, web};
use serde::Serialize;

use crate::api::discography::{Resolved, resolve_local, resolve_row};
use crate::api::editorial::{EXTERNAL_PREFIX, artist_image_url};
use crate::api::error::internal_server_error;
use crate::domain::Album;
use crate::enrichment::identity::{PersonIdentity, PersonMembers, load_identity};
use crate::enrichment::musicbrainz::is_placeholder_artist;
use crate::enrichment::store::{self, CreditRow, InfluenceRow, TrackWorkRow, WorkRow};
use crate::library::credits::ARTIST_ROLES;
use crate::library::focus::looks_like_ensemble;
use crate::library::identity::normalize_artist_identity;
use crate::library::state::{LibraryCache, LibraryLifecycle, library_unavailable_response};
use crate::persistence::connection::DbPool;

/// At most this many people per section.
const MAX_RELATED: usize = 24;
/// At most this many albums are read to build one person's graph.
const MAX_GRAPH_ALBUMS: usize = 600;

/// What someone does on a track, as far as relations go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Part {
    /// Composer, lyricist or arranger.
    Writer,
    Conductor,
    Ensemble,
    Soloist,
    /// The album or track artist.
    Main,
    /// Producer, engineer, mixer or remixer.
    Production,
}

impl Part {
    fn performs(self) -> bool {
        matches!(
            self,
            Part::Conductor | Part::Ensemble | Part::Soloist | Part::Main
        )
    }

    fn makes_the_record(self) -> bool {
        self != Part::Writer
    }

    fn kind(self) -> &'static str {
        match self {
            Part::Writer => "composer",
            Part::Conductor => "conductor",
            Part::Ensemble => "ensemble",
            Part::Soloist => "soloist",
            Part::Main => "artist",
            Part::Production => "production",
        }
    }
}

/// The part a credit role plays; `None` for roles relations ignore.
pub fn part_for_role(role: &str, name: &str) -> Option<Part> {
    Some(match role {
        "composer" | "lyricist" | "arranger" => Part::Writer,
        "conductor" => Part::Conductor,
        "ensemble" => Part::Ensemble,
        "performer" | "primary" | "featured" if looks_like_ensemble(name) => Part::Ensemble,
        "performer" => Part::Soloist,
        "primary" | "featured" => Part::Main,
        "producer" | "remixer" | "mixer" | "engineer" => Part::Production,
        _ => return None,
    })
}

/// One person on one track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Participant {
    /// Canonical person id, or `None` when the credit cannot be browsed.
    pub id: Option<String>,
    pub name: String,
    pub part: Part,
    /// What to call their part: the instrument, or the role.
    pub label: String,
}

impl Participant {
    fn key(&self) -> String {
        self.id
            .clone()
            .unwrap_or_else(|| format!("name:{}", normalize_artist_identity(&self.name)))
    }
}

/// Everyone on each track of one album, and the work each track records.
#[derive(Debug, Clone, Default)]
pub struct AlbumGraph {
    pub album_id: String,
    pub tracks: Vec<TrackGraph>,
}

#[derive(Debug, Clone, Default)]
pub struct TrackGraph {
    pub track_id: String,
    /// The work (the piece, not the movement) the track records.
    pub work: Option<String>,
    pub people: Vec<Participant>,
}

/// What the database holds about one album, for its graph.
#[derive(Debug, Clone, Default)]
pub struct AlbumData {
    pub credits: Vec<CreditRow>,
    pub track_works: Vec<TrackWorkRow>,
    pub works: Vec<WorkRow>,
}

fn label_for(role: &str, instrument: Option<&str>) -> String {
    match instrument.map(str::trim).filter(|value| !value.is_empty()) {
        Some(instrument) => instrument.to_lowercase(),
        None => match role {
            "primary" | "featured" => "artist".to_string(),
            other => other.to_string(),
        },
    }
}

fn add(people: &mut Vec<Participant>, resolved: Resolved, part: Part, label: String) {
    if is_placeholder_artist(&resolved.name) {
        return;
    }
    let participant = Participant {
        id: resolved.id,
        name: resolved.name,
        part,
        label,
    };
    if !people
        .iter()
        .any(|known| known.key() == participant.key() && known.part == part)
    {
        people.push(participant);
    }
}

/// Builds who did what on each track of an album, from tags, MusicBrainz
/// credits and the composers of the works the tracks record.
pub fn album_graph(
    cache: &LibraryCache,
    identity: &PersonIdentity,
    known: &HashMap<String, String>,
    album: &Album,
    data: &AlbumData,
) -> AlbumGraph {
    let works = data
        .works
        .iter()
        .map(|work| (work.mbid.as_str(), work))
        .collect::<HashMap<_, _>>();
    let mut graph = AlbumGraph {
        album_id: album.id.clone(),
        tracks: Vec::new(),
    };
    let owner = cache
        .album_owner(&album.id)
        .filter(|owner| !is_placeholder_artist(&owner.name));
    for song in &album.songs {
        let mut people = Vec::new();
        if let Some(owner) = owner {
            let part = part_for_role("primary", &owner.name).unwrap_or(Part::Main);
            add(
                &mut people,
                resolve_local(identity, &owner.id, &owner.name, &album.id),
                part,
                label_for("primary", None),
            );
        }
        // The track's own artist, when it is not the album's ("Wiener
        // Philharmoniker" on Kleiber's album).
        let track_artist = song.artist.trim();
        if !track_artist.is_empty() && owner.is_none_or(|owner| owner.name != track_artist) {
            let resolved = match known.get(&normalize_artist_identity(track_artist)) {
                Some(id) => resolve_local(identity, id, track_artist, &album.id),
                None => Resolved {
                    id: None,
                    name: track_artist.to_string(),
                },
            };
            add(
                &mut people,
                resolved,
                part_for_role("primary", track_artist).unwrap_or(Part::Main),
                label_for("primary", None),
            );
        }
        for (name, id) in song
            .contributing_artists
            .iter()
            .zip(&song.contributing_artist_ids)
        {
            let part = part_for_role("primary", name).unwrap_or(Part::Main);
            add(
                &mut people,
                resolve_local(identity, id, name, &album.id),
                part,
                label_for("primary", None),
            );
        }
        for credit in &song.credits {
            let Some(part) = part_for_role(&credit.role, &credit.name) else {
                continue;
            };
            let role = if ARTIST_ROLES.contains(&credit.role.as_str()) {
                "primary"
            } else {
                credit.role.as_str()
            };
            add(
                &mut people,
                resolve_local(identity, &credit.artist_id, &credit.name, &album.id),
                part,
                label_for(role, credit.instrument.as_deref()),
            );
        }
        for row in data
            .credits
            .iter()
            .filter(|row| row.track_id.is_empty() || row.track_id == song.id)
        {
            let Some(part) = part_for_role(&row.role, &row.name) else {
                continue;
            };
            add(
                &mut people,
                resolve_row(identity, known, row),
                part,
                label_for(&row.role, Some(row.instrument.as_str())),
            );
        }
        let mut work = None;
        for link in data
            .track_works
            .iter()
            .filter(|link| link.track_id == song.id)
        {
            let Some(found) = works.get(link.work_mbid.as_str()) else {
                continue;
            };
            let top = found
                .parent_mbid
                .as_deref()
                .and_then(|parent| works.get(parent))
                .copied()
                .unwrap_or(found);
            work.get_or_insert_with(|| top.mbid.clone());
            let Some(name) = top.composer_name.as_deref() else {
                continue;
            };
            let resolved = match top.composer_mbid.as_deref() {
                Some(mbid) => {
                    let id = identity.for_mbid(mbid);
                    Resolved {
                        name: identity.display_name(&id).unwrap_or(name).to_string(),
                        id: Some(id),
                    }
                }
                None => match known.get(&normalize_artist_identity(name)) {
                    Some(id) => resolve_local(identity, id, name, &album.id),
                    None => Resolved {
                        id: None,
                        name: name.to_string(),
                    },
                },
            };
            add(&mut people, resolved, Part::Writer, "composer".to_string());
        }
        graph.tracks.push(TrackGraph {
            track_id: song.id.clone(),
            // Without a MusicBrainz work, "Symphony No. 5 in C minor, Op.
            // 67: I. Allegro con brio" names its piece before the colon.
            work: work.or_else(|| piece_from_title(&song.name)),
            people,
        });
    }
    graph
}

/// The piece a movement title names before its colon, as a work key.
fn piece_from_title(title: &str) -> Option<String> {
    let (piece, movement) = title.split_once(": ")?;
    let piece = normalize_artist_identity(piece);
    (!piece.is_empty() && !movement.trim().is_empty()).then(|| format!("title:{piece}"))
}

/// A related person with what connects them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelatedPerson {
    /// Their page (`None` when only a name is known).
    pub artist_id: Option<String>,
    pub name: String,
    pub image_url: Option<String>,
    /// What they did on the shared music: instruments and roles, most
    /// frequent first ("piano", "producer", "conductor").
    pub roles: Vec<String>,
    /// For top performers: `conductor`, `ensemble`, `soloist` or `artist`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<&'static str>,
    pub album_count: usize,
    pub track_count: usize,
    /// Distinct works (or, without work links, tracks) connecting them.
    pub work_count: usize,
    /// Up to three shared albums, most shared music first.
    pub albums: Vec<RelatedAlbum>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelatedAlbum {
    pub id: String,
    pub name: String,
    pub cover_url: String,
}

/// Names what a top performer mostly is, from how often each part occurs.
type KindOf = fn(&HashMap<Part, usize>) -> &'static str;

#[derive(Default)]
struct Tally {
    id: Option<String>,
    name: String,
    albums: Vec<String>,
    tracks: HashSet<String>,
    works: HashSet<String>,
    labels: HashMap<String, usize>,
    parts: HashMap<Part, usize>,
}

impl Tally {
    fn count(&mut self, album_id: &str, track: &TrackGraph, parts: &[&Participant]) {
        if !self.albums.iter().any(|id| id == album_id) {
            self.albums.push(album_id.to_string());
        }
        self.tracks.insert(track.track_id.clone());
        self.works.insert(
            track
                .work
                .clone()
                .unwrap_or_else(|| format!("track:{}", track.track_id)),
        );
        for participant in parts {
            *self.labels.entry(participant.label.clone()).or_default() += 1;
            *self.parts.entry(participant.part).or_default() += 1;
        }
    }

    fn finish(self, cache: &LibraryCache, kind: Option<KindOf>) -> RelatedPerson {
        let mut labels = self.labels.into_iter().collect::<Vec<_>>();
        labels.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
        let albums = self
            .albums
            .iter()
            .filter_map(|id| cache.album(id))
            .take(3)
            .map(|album| RelatedAlbum {
                id: album.id.clone(),
                name: album.name.clone(),
                cover_url: album.cover_url.clone(),
            })
            .collect();
        RelatedPerson {
            kind: kind.map(|kind| kind(&self.parts)),
            artist_id: self.id,
            name: self.name,
            image_url: None,
            roles: labels.into_iter().take(3).map(|(label, _)| label).collect(),
            album_count: self.albums.len(),
            track_count: self.tracks.len(),
            work_count: self.works.len(),
            albums,
        }
    }
}

/// Whether someone plays on a track: as conductor, ensemble or soloist,
/// or as its artist. A composer billed as the album or track artist (the
/// usual way classical files are tagged) does not perform by that alone.
fn really_performs(parts: &BTreeSet<Part>) -> bool {
    parts
        .iter()
        .any(|part| matches!(part, Part::Conductor | Part::Ensemble | Part::Soloist))
        || (parts.contains(&Part::Main) && !parts.contains(&Part::Writer))
}

/// The three graph sections for one person.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GraphSections {
    pub collaborators: Vec<RelatedPerson>,
    pub top_performers: Vec<RelatedPerson>,
    pub performing_the_music_of: Vec<RelatedPerson>,
}

fn performer_kind(parts: &HashMap<Part, usize>) -> &'static str {
    for part in [Part::Conductor, Part::Ensemble, Part::Soloist] {
        if parts.contains_key(&part) {
            return part.kind();
        }
    }
    Part::Main.kind()
}

fn ranked(
    tallies: HashMap<String, Tally>,
    cache: &LibraryCache,
    kind: Option<KindOf>,
    by_works: bool,
) -> Vec<RelatedPerson> {
    let mut people = tallies
        .into_values()
        .map(|tally| tally.finish(cache, kind))
        .collect::<Vec<_>>();
    people.sort_by(|left, right| {
        let first = if by_works {
            (right.album_count, right.work_count, right.track_count).cmp(&(
                left.album_count,
                left.work_count,
                left.track_count,
            ))
        } else {
            (right.album_count, right.track_count).cmp(&(left.album_count, left.track_count))
        };
        first
            .then_with(|| right.artist_id.is_some().cmp(&left.artist_id.is_some()))
            .then_with(|| left.name.cmp(&right.name))
    });
    people.truncate(MAX_RELATED);
    people
}

/// Collaborators, top performers and the composers a person performs,
/// from the graphs of the albums they are on.
pub fn graph_sections(cache: &LibraryCache, person: &str, graphs: &[AlbumGraph]) -> GraphSections {
    let mut collaborators: HashMap<String, Tally> = HashMap::new();
    let mut performers: HashMap<String, Tally> = HashMap::new();
    let mut composers: HashMap<String, Tally> = HashMap::new();
    for graph in graphs {
        for track in &graph.tracks {
            let own = track
                .people
                .iter()
                .filter(|participant| participant.id.as_deref() == Some(person))
                .map(|participant| participant.part)
                .collect::<BTreeSet<_>>();
            if own.is_empty() {
                continue;
            }
            let writes = own.contains(&Part::Writer);
            let performs = really_performs(&own);
            let records = performs || own.contains(&Part::Production);
            let mut others: Vec<(String, Vec<&Participant>)> = Vec::new();
            for participant in track
                .people
                .iter()
                .filter(|participant| participant.id.as_deref() != Some(person))
            {
                let key = participant.key();
                match others.iter_mut().find(|(known, _)| *known == key) {
                    Some((_, parts)) => parts.push(participant),
                    None => others.push((key, vec![participant])),
                }
            }
            for (key, parts) in others {
                let first = parts[0];
                let tally = |map: &mut HashMap<String, Tally>, chosen: &[&Participant]| {
                    let entry = map.entry(key.clone()).or_insert_with(|| Tally {
                        id: first.id.clone(),
                        name: first.name.clone(),
                        ..Tally::default()
                    });
                    entry.count(&graph.album_id, track, chosen);
                };
                let theirs = parts
                    .iter()
                    .map(|other| other.part)
                    .collect::<BTreeSet<_>>();
                let they_write = theirs.contains(&Part::Writer);
                let they_perform = really_performs(&theirs);
                let they_record = they_perform || theirs.contains(&Part::Production);
                // People who made the record together, or wrote together.
                let made = records && they_record;
                let wrote = writes && they_write;
                if made || wrote {
                    let together = parts
                        .iter()
                        .copied()
                        .filter(|other| {
                            (made && other.part.makes_the_record())
                                || (wrote && other.part == Part::Writer)
                        })
                        .collect::<Vec<_>>();
                    tally(&mut collaborators, &together);
                }
                // Others performing the person's music: not their own
                // band on their own record, which is collaboration.
                if writes && !performs && they_perform {
                    let playing = parts
                        .iter()
                        .copied()
                        .filter(|other| other.part.performs())
                        .collect::<Vec<_>>();
                    tally(&mut performers, &playing);
                }
                if performs && they_write && !they_perform {
                    let writing = parts
                        .iter()
                        .copied()
                        .filter(|other| other.part == Part::Writer)
                        .collect::<Vec<_>>();
                    tally(&mut composers, &writing);
                }
            }
        }
    }
    GraphSections {
        collaborators: ranked(collaborators, cache, None, false),
        top_performers: ranked(performers, cache, Some(performer_kind), true),
        performing_the_music_of: ranked(composers, cache, None, true),
    }
}

/// Someone Wikidata relates to the person by influence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InfluencePerson {
    /// Their page on this server, when there is one to open.
    pub artist_id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub image_url: Option<String>,
    /// Whether they have music in this library.
    pub in_library: bool,
    pub wikidata_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Attribution {
    pub source: &'static str,
    pub url: &'static str,
    pub license: &'static str,
    pub license_url: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct PersonRelations {
    pub artist_id: String,
    pub requested_id: String,
    #[serde(flatten)]
    pub sections: GraphSections,
    pub influences: Vec<InfluencePerson>,
    pub followers: Vec<InfluencePerson>,
    /// Where influences come from, shown under them.
    pub influence_source: Option<Attribution>,
    /// Whether online metadata (and so influences) is on.
    pub enrichment_enabled: bool,
}

/// Resolves influence rows to people on this server.
pub fn influence_people(
    cache: &LibraryCache,
    identity: &PersonIdentity,
    rows: &[InfluenceRow],
    by_wikidata: &HashMap<String, String>,
    direction: &str,
    own: &str,
) -> Vec<InfluencePerson> {
    let mut seen = HashSet::new();
    let mut people = Vec::new();
    let mut rows = rows
        .iter()
        .filter(|row| row.direction == direction)
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| row.position);
    for row in rows {
        if !seen.insert(row.wikidata_id.clone()) {
            continue;
        }
        let canonical = row
            .musicbrainz_id
            .as_deref()
            .map(|mbid| identity.for_mbid(mbid))
            .filter(|id| !id.starts_with(EXTERNAL_PREFIX))
            .or_else(|| {
                by_wikidata
                    .get(&row.wikidata_id)
                    .map(|id| identity.members(id).canonical)
            })
            .or_else(|| {
                row.musicbrainz_id
                    .as_deref()
                    .map(|mbid| identity.for_mbid(mbid))
            });
        if canonical.as_deref() == Some(own) {
            continue;
        }
        let in_library = canonical.as_deref().is_some_and(|id| {
            !id.starts_with(EXTERNAL_PREFIX) || person_has_music(cache, identity, id)
        });
        // A MusicBrainz-only page exists once their lookup named them.
        let artist_id = canonical
            .filter(|id| !id.starts_with(EXTERNAL_PREFIX) || identity.display_name(id).is_some());
        people.push(InfluencePerson {
            artist_id,
            name: row.name.clone(),
            description: row.description.clone(),
            image_url: None,
            in_library,
            wikidata_url: format!("https://www.wikidata.org/wiki/{}", row.wikidata_id),
        });
    }
    people.sort_by_key(|person| !person.in_library);
    people
}

fn person_has_music(cache: &LibraryCache, identity: &PersonIdentity, id: &str) -> bool {
    let members = identity.members(id);
    members
        .local_ids
        .iter()
        .any(|local| cache.artist(local).is_some())
}

/// Ids a person's photo may be stored under: their canonical id, then
/// their other ids.
fn image_candidates(identity: &PersonIdentity, id: &str) -> Vec<String> {
    let members = identity.members(id);
    let mut ids = vec![id.to_string()];
    for other in members.local_ids.iter().cloned().chain(
        members
            .mbids
            .iter()
            .map(|mbid| format!("{EXTERNAL_PREFIX}{mbid}")),
    ) {
        if !ids.contains(&other) {
            ids.push(other);
        }
    }
    ids
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

/// Albums a person's graph is built from: albums they are on, and albums
/// recording works MusicBrainz says they composed.
fn graph_albums(
    cache: &LibraryCache,
    identity: &PersonIdentity,
    members: &PersonMembers,
    rows: &[CreditRow],
    composed: Vec<String>,
) -> Vec<String> {
    let mut ids = crate::api::discography::candidate_albums(cache, identity, members, rows);
    for id in composed {
        if cache.album(&id).is_some() && !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids.truncate(MAX_GRAPH_ALBUMS);
    ids
}

#[get("/{id}/related")]
pub async fn get_related(
    id: web::Path<String>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let requested = id.into_inner();
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    let pool = pool.get_ref().clone();
    let identity = load_identity(pool.clone(), cache.clone()).await;
    let members = identity.members(&requested);
    let canonical = members.canonical.clone();
    let names = members.names.clone();
    let mbids = members.mbids.clone();
    let mut stored_ids = vec![canonical.clone()];
    stored_ids.extend(members.local_ids.iter().cloned());
    stored_ids.extend(mbids.iter().map(|mbid| format!("{EXTERNAL_PREFIX}{mbid}")));
    stored_ids.dedup();
    let loaded = blocking(&pool, move |connection| {
        let rows = store::people_credits(connection, &names, &mbids)?;
        let composed = store::composer_work_albums(connection, &names, &mbids)?;
        let influences = store::load_influences(connection, &stored_ids)?;
        let wikidata = influences
            .iter()
            .map(|row| row.wikidata_id.clone())
            .collect::<Vec<_>>();
        let by_wikidata = store::artists_by_wikidata(connection, &wikidata)?;
        let enabled = store::load_settings(connection)?.enabled;
        Ok((rows, composed, influences, by_wikidata, enabled))
    })
    .await;
    let (rows, composed, influences, by_wikidata, enabled) = match loaded {
        Ok(loaded) => loaded,
        Err(error) => {
            tracing::error!(%error, "could not load related people");
            return internal_server_error("Could not load related people.", "related_failed");
        }
    };
    let album_ids = graph_albums(&cache, &identity, &members, &rows, composed);
    let data = blocking(&pool, move |connection| {
        album_ids
            .into_iter()
            .map(|album_id| {
                let credits = store::album_credits(connection, &album_id)?;
                let (track_works, works) =
                    crate::api::works::load_album_track_works(connection, &album_id)?;
                Ok((
                    album_id,
                    AlbumData {
                        credits,
                        track_works,
                        works,
                    },
                ))
            })
            .collect::<diesel::QueryResult<Vec<_>>>()
    })
    .await;
    let data = match data {
        Ok(data) => data,
        Err(error) => {
            tracing::error!(%error, "could not load related people");
            return internal_server_error("Could not load related people.", "related_failed");
        }
    };
    let known = crate::api::works::person_index(&cache);
    let graphs = data
        .iter()
        .filter_map(|(album_id, data)| {
            let album = cache.album(album_id)?;
            Some(album_graph(&cache, &identity, &known, album, data))
        })
        .collect::<Vec<_>>();
    let sections = graph_sections(&cache, &canonical, &graphs);
    let by_wikidata = by_wikidata
        .into_iter()
        .map(|row| (row.wikidata_id, row.entity_id))
        .collect::<HashMap<_, _>>();
    let mut influences_out = influence_people(
        &cache,
        &identity,
        &influences,
        &by_wikidata,
        "influence",
        &canonical,
    );
    let mut followers = influence_people(
        &cache,
        &identity,
        &influences,
        &by_wikidata,
        "follower",
        &canonical,
    );
    let mut relations = PersonRelations {
        artist_id: canonical,
        requested_id: requested,
        sections,
        influence_source: (!influences.is_empty()).then_some(Attribution {
            source: "Wikidata",
            url: "https://www.wikidata.org/wiki/Property:P737",
            license: "CC0 1.0",
            license_url: "https://creativecommons.org/publicdomain/zero/1.0/",
        }),
        influences: Vec::new(),
        followers: Vec::new(),
        enrichment_enabled: enabled,
    };
    // Photos, stored under any of each person's ids.
    let mut wanted: Vec<(String, Vec<String>)> = Vec::new();
    {
        let sections = &relations.sections;
        for id in sections
            .collaborators
            .iter()
            .chain(&sections.top_performers)
            .chain(&sections.performing_the_music_of)
            .filter_map(|person| person.artist_id.as_deref())
            .chain(
                influences_out
                    .iter()
                    .chain(&followers)
                    .filter_map(|person| person.artist_id.as_deref()),
            )
        {
            if !wanted.iter().any(|(known, _)| known == id) {
                wanted.push((id.to_string(), image_candidates(&identity, id)));
            }
        }
    }
    let all = wanted
        .iter()
        .flat_map(|(_, ids)| ids.iter().cloned())
        .collect::<Vec<_>>();
    let images = crate::api::editorial::credited_images(pool, all).await;
    let photo = |id: Option<&str>| -> Option<String> {
        let (_, candidates) = wanted
            .iter()
            .find(|(known, _)| Some(known.as_str()) == id)?;
        candidates
            .iter()
            .find(|candidate| images.contains(*candidate))
            .map(|candidate| artist_image_url(candidate))
    };
    let sections = &mut relations.sections;
    for person in sections
        .collaborators
        .iter_mut()
        .chain(sections.top_performers.iter_mut())
        .chain(sections.performing_the_music_of.iter_mut())
    {
        person.image_url = photo(person.artist_id.as_deref());
    }
    for person in influences_out.iter_mut().chain(followers.iter_mut()) {
        person.image_url = photo(person.artist_id.as_deref());
    }
    relations.influences = influences_out;
    relations.followers = followers;
    HttpResponse::Ok().json(relations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Artist, Credit, Song};
    use crate::enrichment::store::AliasRow;

    fn credit(id: &str, name: &str, role: &str, instrument: Option<&str>) -> Credit {
        Credit {
            artist_id: id.into(),
            name: name.into(),
            role: role.into(),
            instrument: instrument.map(str::to_string),
        }
    }

    fn song(id: &str, credits: Vec<Credit>, artists: &[(&str, &str)]) -> Song {
        Song {
            id: id.into(),
            name: id.into(),
            credits,
            contributing_artists: artists.iter().map(|(_, name)| name.to_string()).collect(),
            contributing_artist_ids: artists.iter().map(|(id, _)| id.to_string()).collect(),
            ..Song::default()
        }
    }

    fn album(id: &str, songs: Vec<Song>) -> Album {
        Album {
            id: id.into(),
            name: id.into(),
            songs,
            ..Album::default()
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
            source_artist_id: Some(mbid.into()).filter(|mbid: &String| !mbid.is_empty()),
        }
    }

    const DIAMOND: &str = "0b3d0a5d-7c5e-4b1e-9a43-5a8c6d4a1f10";
    const TEO: &str = "3c2f0b0e-0f1c-4d6c-9b1e-0a7b5c3d2e1f";

    /// Kind of Blue (Miles with Bill Evans on piano, Teo Macero producing
    /// the whole release), two Beethoven recordings (Karajan with the
    /// Philharmonia, Kleiber with the Wiener Philharmoniker) and a Miles
    /// tune recorded by Bill Evans' own trio.
    fn cache() -> LibraryCache {
        let miles = Artist {
            id: "miles".into(),
            name: "Miles Davis".into(),
            albums: vec![album(
                "kind-of-blue",
                vec![
                    song(
                        "so-what",
                        vec![
                            credit("miles", "Miles Davis", "composer", None),
                            credit("miles", "Miles Davis", "performer", Some("Trumpet")),
                            credit("evans", "Bill Evans", "performer", Some("Piano")),
                        ],
                        &[],
                    ),
                    song(
                        "blue-in-green",
                        vec![
                            credit("miles", "Miles Davis", "composer", None),
                            credit("miles", "Miles Davis", "performer", Some("Trumpet")),
                            credit("evans", "Bill Evans", "composer", None),
                            credit("evans", "Bill Evans", "performer", Some("Piano")),
                        ],
                        &[],
                    ),
                ],
            )],
            ..Artist::default()
        };
        let evans = Artist {
            id: "evans".into(),
            name: "Bill Evans".into(),
            albums: vec![album(
                "portrait-in-jazz",
                vec![song(
                    "blue-in-green-trio",
                    vec![
                        credit("miles", "Miles Davis", "composer", None),
                        credit("evans", "Bill Evans", "composer", None),
                        credit("evans", "Bill Evans", "performer", Some("Piano")),
                    ],
                    &[],
                )],
            )],
            ..Artist::default()
        };
        let karajan = Artist {
            id: "karajan".into(),
            name: "Herbert von Karajan".into(),
            albums: vec![album(
                "karajan-9",
                vec![song(
                    "k9-1",
                    vec![
                        credit("beethoven", "Ludwig van Beethoven", "composer", None),
                        credit("karajan", "Herbert von Karajan", "conductor", None),
                        // An orchestra tagged as a performer is an ensemble.
                        credit("philharmonia", "Philharmonia Orchestra", "performer", None),
                    ],
                    &[],
                )],
            )],
            ..Artist::default()
        };
        let kleiber = Artist {
            id: "kleiber".into(),
            name: "Carlos Kleiber".into(),
            albums: vec![album(
                "kleiber-5",
                vec![song(
                    "k5-1",
                    vec![
                        credit("beethoven", "Ludwig van Beethoven", "composer", None),
                        credit("kleiber", "Carlos Kleiber", "conductor", None),
                    ],
                    &[("vienna", "Wiener Philharmoniker")],
                )],
            )],
            ..Artist::default()
        };
        LibraryCache::from_artists(vec![miles, evans, karajan, kleiber])
    }

    fn graphs(
        cache: &LibraryCache,
        identity: &PersonIdentity,
        rows: &[CreditRow],
    ) -> Vec<AlbumGraph> {
        let known = crate::api::works::person_index(cache);
        cache
            .artists
            .iter()
            .flat_map(|artist| &artist.albums)
            .map(|album| {
                let data = AlbumData {
                    credits: rows
                        .iter()
                        .filter(|row| row.album_id == album.id)
                        .cloned()
                        .collect(),
                    ..AlbumData::default()
                };
                album_graph(cache, identity, &known, album, &data)
            })
            .collect()
    }

    fn names(people: &[RelatedPerson]) -> Vec<&str> {
        people.iter().map(|person| person.name.as_str()).collect()
    }

    #[test]
    fn collaborators_are_ranked_by_shared_releases_and_tracks() {
        let cache = cache();
        let identity = PersonIdentity::build(&cache, &[], &[], &[]);
        let rows = vec![row("kind-of-blue", "", "Teo Macero", "producer", "", TEO)];
        let sections = graph_sections(&cache, "evans", &graphs(&cache, &identity, &rows));
        assert_eq!(
            names(&sections.collaborators),
            ["Miles Davis", "Teo Macero"]
        );
        let miles = &sections.collaborators[0];
        assert_eq!(miles.artist_id.as_deref(), Some("miles"));
        // Both Kind of Blue tracks plus the tune they wrote together.
        assert_eq!((miles.album_count, miles.track_count), (2, 3));
        assert_eq!(miles.roles[0], "artist");
        let teo = &sections.collaborators[1];
        assert_eq!(teo.artist_id.as_deref(), Some(&*format!("mb:{TEO}")));
        assert_eq!((teo.album_count, teo.track_count), (1, 2));
        assert_eq!(teo.roles, ["producer"]);
    }

    #[test]
    fn composers_get_their_top_performers_and_performers_their_composers() {
        let cache = cache();
        let identity = PersonIdentity::build(&cache, &[], &[], &[]);
        let graphs = graphs(&cache, &identity, &[]);
        let beethoven = graph_sections(&cache, "beethoven", &graphs);
        assert_eq!(
            names(&beethoven.top_performers),
            [
                "Carlos Kleiber",
                "Herbert von Karajan",
                "Philharmonia Orchestra",
                "Wiener Philharmoniker"
            ]
        );
        let kinds = beethoven
            .top_performers
            .iter()
            .map(|person| person.kind.unwrap())
            .collect::<Vec<_>>();
        assert_eq!(kinds, ["conductor", "conductor", "ensemble", "ensemble"]);
        // A composer is never their performers' collaborator.
        assert!(beethoven.collaborators.is_empty());

        let karajan = graph_sections(&cache, "karajan", &graphs);
        assert_eq!(
            names(&karajan.performing_the_music_of),
            ["Ludwig van Beethoven"]
        );
        assert_eq!(names(&karajan.collaborators), ["Philharmonia Orchestra"]);

        // Bill Evans' trio records a tune Miles wrote: Miles is among the
        // composers he performs, and Evans among Miles' performers. Playing
        // on Miles' own record is collaboration, not "his music".
        let evans = graph_sections(&cache, "evans", &graphs);
        assert_eq!(names(&evans.performing_the_music_of), ["Miles Davis"]);
        assert_eq!(evans.performing_the_music_of[0].album_count, 1);
        let miles = graph_sections(&cache, "miles", &graphs);
        assert_eq!(names(&miles.top_performers), ["Bill Evans"]);
        assert_eq!(miles.top_performers[0].kind, Some("soloist"));
        assert_eq!(miles.top_performers[0].album_count, 1);
    }

    #[test]
    fn a_person_credited_under_two_names_is_one_related_person() {
        // MusicBrainz credits Diamond D as composer under his legal name
        // and as a performer under his stage name, with one id.
        let cache = LibraryCache::from_artists(vec![Artist {
            id: "atcq".into(),
            name: "A Tribe Called Quest".into(),
            albums: vec![album("low-end", vec![song("t1", Vec::new(), &[])])],
            ..Artist::default()
        }]);
        let identity = PersonIdentity::build(
            &cache,
            &[],
            &[AliasRow {
                mbid: DIAMOND.into(),
                normalized_name: normalize_artist_identity("Diamond D"),
                name: "Diamond D".into(),
                kind: "name".into(),
            }],
            &[],
        );
        let rows = vec![
            row("low-end", "t1", "Joseph Kirkland", "composer", "", DIAMOND),
            row("low-end", "t1", "Diamond D", "performer", "vocals", DIAMOND),
        ];
        let graphs = graphs(&cache, &identity, &rows);
        let sections = graph_sections(&cache, "atcq", &graphs);
        assert_eq!(names(&sections.collaborators), ["Diamond D"]);
        assert_eq!(sections.collaborators[0].roles, ["vocals"]);
        // He performs what he wrote with them: collaboration, not a
        // composer they perform.
        assert!(sections.performing_the_music_of.is_empty());
    }

    #[test]
    fn a_composer_billed_as_the_album_artist_still_has_performers() {
        // Classical files are often tagged with the composer as album
        // artist; the conductor and orchestra are still the performers.
        let cache = LibraryCache::from_artists(vec![Artist {
            id: "beethoven".into(),
            name: "Ludwig van Beethoven".into(),
            albums: vec![album(
                "kleiber-5",
                vec![song(
                    "k5-1",
                    vec![
                        credit("beethoven", "Ludwig van Beethoven", "composer", None),
                        credit("kleiber", "Carlos Kleiber", "conductor", None),
                    ],
                    &[("vienna", "Wiener Philharmoniker")],
                )],
            )],
            ..Artist::default()
        }]);
        let identity = PersonIdentity::build(&cache, &[], &[], &[]);
        let graphs = graphs(&cache, &identity, &[]);
        let beethoven = graph_sections(&cache, "beethoven", &graphs);
        assert_eq!(
            names(&beethoven.top_performers),
            ["Carlos Kleiber", "Wiener Philharmoniker"]
        );
        // Movements of one piece are one work without MusicBrainz links.
        assert_eq!(
            piece_from_title("Symphony No. 5: I. Allegro"),
            piece_from_title("Symphony No. 5: II. Andante")
        );
        assert_eq!(piece_from_title("So What"), None);
        assert!(beethoven.collaborators.is_empty());
        let kleiber = graph_sections(&cache, "kleiber", &graphs);
        assert_eq!(
            names(&kleiber.performing_the_music_of),
            ["Ludwig van Beethoven"]
        );
        assert_eq!(names(&kleiber.collaborators), ["Wiener Philharmoniker"]);
    }

    #[test]
    fn influences_link_to_library_people_and_skip_the_person() {
        let cache = cache();
        let identity = PersonIdentity::build(&cache, &[], &[], &[]);
        let influence = |direction: &str, item: &str, name: &str, position: i32| InfluenceRow {
            artist_id: "evans".into(),
            direction: direction.into(),
            wikidata_id: item.into(),
            name: name.into(),
            description: None,
            musicbrainz_id: None,
            image: None,
            position,
        };
        let rows = vec![
            influence("influence", "Q1", "Claude Debussy", 1),
            influence("influence", "Q93341", "Miles Davis", 2),
            influence("follower", "Q2", "Herbie Hancock", 1),
            influence("follower", "Q3", "Bill Evans", 2),
        ];
        let by_wikidata = HashMap::from([
            ("Q93341".to_string(), "miles".to_string()),
            ("Q3".to_string(), "evans".to_string()),
        ]);
        let influences =
            influence_people(&cache, &identity, &rows, &by_wikidata, "influence", "evans");
        assert_eq!(influences[0].name, "Miles Davis");
        assert!(influences[0].in_library);
        assert_eq!(influences[0].artist_id.as_deref(), Some("miles"));
        assert_eq!(influences[1].artist_id, None);
        assert_eq!(
            influences[1].wikidata_url,
            "https://www.wikidata.org/wiki/Q1"
        );
        let followers =
            influence_people(&cache, &identity, &rows, &by_wikidata, "follower", "evans");
        assert_eq!(followers.len(), 1);
        assert_eq!(followers[0].name, "Herbie Hancock");
    }
}
