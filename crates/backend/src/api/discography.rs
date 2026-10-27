//! A person's discography with Focus: every album they made, appear on,
//! conducted, composed for or are otherwise credited on, filterable by
//! their role, the other people on the album, the works' form and period,
//! the format (bit depth, sample rate, hi-res) and the source.

use std::collections::{HashMap, HashSet};

use actix_web::{HttpResponse, get, web};
use serde::Serialize;

use crate::api::editorial::EXTERNAL_PREFIX;
use crate::api::error::internal_server_error;
use crate::api::works::PersonRef;
use crate::domain::{Album, Artist, AudioProperties};
use crate::enrichment::identity::{PersonIdentity, PersonMembers, load_identity};
use crate::enrichment::musicbrainz::is_placeholder_artist;
use crate::enrichment::store::{self, CreditRow, TrackWorkRow, WorkRow};
use crate::enrichment::works::{first_year, period_for_year};
use crate::library::credits::role_rank;
use crate::library::discography_focus::{
    FocusRecord, FocusSummary, Quality, Selection, focus, looks_like_ensemble, parse_selection,
    quality, sample_rate_label, song_source,
};
use crate::library::identity::normalize_artist_identity;
use crate::library::state::{LibraryCache, LibraryLifecycle, library_unavailable_response};
use crate::persistence::connection::DbPool;

const MAX_ALBUMS: usize = 400;

/// One album on a person's discography.
#[derive(Clone, Serialize)]
pub struct FocusAlbum {
    pub album: Album,
    pub artist: Artist,
    pub year: Option<i32>,
    /// The person's roles on the album: `main`, `featured`, or a credit
    /// role (`composer`, `conductor`, `performer`, ...), main first.
    pub roles: Vec<String>,
    pub conductors: Vec<PersonRef>,
    pub ensembles: Vec<PersonRef>,
    /// The best format on the album, e.g. "FLAC 24-bit / 96 kHz".
    pub format: Option<String>,
    /// `hires`, `lossless` or `lossy`, for the best format.
    pub quality: Option<&'static str>,
    pub sources: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct PersonDiscography {
    /// The person's canonical id (another id of theirs opens the same page).
    pub artist_id: String,
    pub requested_id: String,
    pub name: String,
    /// Other names the person is credited under in this library.
    pub credited_as: Vec<String>,
    pub albums: Vec<FocusAlbum>,
    pub focus: FocusSummary,
}

/// What the database holds about one album, for its focus record.
#[derive(Debug, Clone, Default)]
pub struct AlbumExtras {
    pub credits: Vec<CreditRow>,
    pub track_works: Vec<TrackWorkRow>,
    pub works: Vec<WorkRow>,
    /// The release group's type on MusicBrainz ("Album", "EP", ...), which
    /// wins over the type guessed from track count and length.
    pub release_type: Option<String>,
}

/// The MusicBrainz release group type stored for a matched album.
pub fn stored_release_type(profile: Option<&store::ProfileRow>) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(&profile?.profile_json).ok()?;
    value
        .get("release_type")?
        .as_str()
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
        .map(str::to_lowercase)
}

fn role_label(role: &str) -> String {
    match role {
        "main" => "Main artist".to_string(),
        "featured" => "Featured".to_string(),
        other => {
            let mut characters = other.chars();
            match characters.next() {
                Some(first) => first.to_uppercase().chain(characters).collect(),
                None => String::new(),
            }
        }
    }
}

fn codec_label(audio: &AudioProperties) -> Option<String> {
    let codec = audio.codec.as_deref()?;
    Some(match codec {
        "pcm" | "pcm_float" => match audio.container.as_deref() {
            Some("aiff") => "AIFF".to_string(),
            Some("wav") => "WAV".to_string(),
            _ => "PCM".to_string(),
        },
        "vorbis" => "Ogg Vorbis".to_string(),
        "opus" => "Opus".to_string(),
        other => other.to_uppercase(),
    })
}

/// "FLAC 24-bit / 96 kHz", "MP3 320 kbps".
pub fn format_label(audio: &AudioProperties) -> Option<String> {
    let codec = codec_label(audio)?;
    if audio.lossless == Some(true) {
        let mut parts = vec![codec];
        let mut resolution = Vec::new();
        if let Some(bits) = audio.bit_depth {
            resolution.push(format!("{bits}-bit"));
        }
        if let Some(rate) = audio.sample_rate {
            resolution.push(sample_rate_label(rate));
        }
        if !resolution.is_empty() {
            parts.push(resolution.join(" / "));
        }
        return Some(parts.join(" "));
    }
    Some(match (audio.bitrate_kbps, audio.sample_rate) {
        (Some(bitrate), _) => format!("{codec} {bitrate} kbps"),
        (None, Some(rate)) => format!("{codec} {}", sample_rate_label(rate)),
        _ => codec,
    })
}

fn format_rank(audio: &AudioProperties) -> (u8, u8, u32, u32) {
    (
        match quality(audio) {
            Some(Quality::HiRes) => 3,
            Some(Quality::Lossless) => 2,
            Some(Quality::Lossy) => 1,
            None => 0,
        },
        audio.bit_depth.unwrap_or(0),
        audio.sample_rate.unwrap_or(0),
        audio.bitrate_kbps.unwrap_or(0),
    )
}

/// Who someone is on a focus record: canonical id and display name.
pub(crate) struct Resolved {
    pub id: Option<String>,
    pub name: String,
}

pub(crate) fn resolve_local(
    identity: &PersonIdentity,
    id: &str,
    name: &str,
    album_id: &str,
) -> Resolved {
    let canonical = identity.for_local(id, name, Some(album_id));
    Resolved {
        name: identity
            .display_name(&canonical)
            .unwrap_or(name)
            .to_string(),
        id: Some(canonical),
    }
}

pub(crate) fn resolve_row(
    identity: &PersonIdentity,
    known: &HashMap<String, String>,
    row: &CreditRow,
) -> Resolved {
    let id = identity
        .for_row(row)
        .filter(|id| !id.starts_with(EXTERNAL_PREFIX))
        .or_else(|| known.get(&row.normalized_name).cloned())
        .or_else(|| identity.for_row(row));
    Resolved {
        name: id
            .as_deref()
            .and_then(|id| identity.display_name(id))
            .unwrap_or(&row.name)
            .to_string(),
        id,
    }
}

/// Local people by normalized name, for MusicBrainz credits with no id.
fn known_people(cache: &LibraryCache) -> HashMap<String, String> {
    crate::api::works::person_index(cache)
}

fn push_ref(list: &mut Vec<PersonRef>, person: &Resolved) {
    let key = person
        .id
        .clone()
        .unwrap_or_else(|| normalize_artist_identity(&person.name));
    if list.iter().any(|known| {
        known
            .artist_id
            .clone()
            .unwrap_or_else(|| normalize_artist_identity(&known.name))
            == key
    }) {
        return;
    }
    list.push(PersonRef {
        artist_id: person.id.clone(),
        name: person.name.clone(),
        instrument: None,
    });
}

/// Builds one album's focus record and card for a person (`canonical`).
/// With no person (a work's recordings) the role group is left out.
pub fn album_record(
    cache: &LibraryCache,
    identity: &PersonIdentity,
    known: &HashMap<String, String>,
    canonical: Option<&str>,
    album: &Album,
    extras: &AlbumExtras,
) -> (FocusAlbum, FocusRecord) {
    let mut record = FocusRecord::default();
    let mut roles: Vec<String> = Vec::new();
    let mut conductors = Vec::new();
    let mut ensembles = Vec::new();
    let mut performers: Vec<Resolved> = Vec::new();
    let is_person =
        |resolved: &Resolved| canonical.is_some() && resolved.id.as_deref() == canonical;
    let add_role = |roles: &mut Vec<String>, role: &str| {
        if !roles.iter().any(|known| known == role) {
            roles.push(role.to_string());
        }
    };
    let owner = cache.album_owner(&album.id);
    if let Some(owner) = owner.filter(|owner| !is_placeholder_artist(&owner.name)) {
        let resolved = resolve_local(identity, &owner.id, &owner.name, &album.id);
        if is_person(&resolved) {
            add_role(&mut roles, "main");
        } else {
            performers.push(resolved);
        }
    }
    let credit = |record: &mut FocusRecord,
                  roles: &mut Vec<String>,
                  conductors: &mut Vec<PersonRef>,
                  ensembles: &mut Vec<PersonRef>,
                  resolved: Resolved,
                  role: &str,
                  instrument: Option<&str>| {
        if let Some(instrument) = instrument.filter(|value| !value.is_empty()) {
            record.add("instrument", &instrument.to_lowercase(), instrument, None);
        }
        if is_person(&resolved) {
            add_role(roles, role);
            return;
        }
        match role {
            "conductor" => {
                push_ref(conductors, &resolved);
                record.add_person("conductor", resolved.id.as_deref(), &resolved.name);
            }
            "ensemble" => {
                push_ref(ensembles, &resolved);
                record.add_person("ensemble", resolved.id.as_deref(), &resolved.name);
            }
            // An orchestra or choir credited as a performer is an
            // ensemble, never a soloist.
            "performer" if looks_like_ensemble(&resolved.name) => {
                push_ref(ensembles, &resolved);
                record.add_person("ensemble", resolved.id.as_deref(), &resolved.name);
            }
            "performer" => {
                record.add_person("soloist", resolved.id.as_deref(), &resolved.name);
            }
            "composer" => {
                record.add_person("composer", resolved.id.as_deref(), &resolved.name);
            }
            _ => {}
        }
    };
    for song in &album.songs {
        for (name, id) in song
            .contributing_artists
            .iter()
            .zip(&song.contributing_artist_ids)
        {
            let resolved = resolve_local(identity, id, name, &album.id);
            if is_person(&resolved) {
                if !roles.iter().any(|role| role == "main") {
                    add_role(&mut roles, "featured");
                }
            } else if !is_placeholder_artist(name) {
                performers.push(resolved);
            }
        }
        for tag in &song.credits {
            let resolved = resolve_local(identity, &tag.artist_id, &tag.name, &album.id);
            credit(
                &mut record,
                &mut roles,
                &mut conductors,
                &mut ensembles,
                resolved,
                &tag.role,
                tag.instrument.as_deref(),
            );
        }
    }
    for row in &extras.credits {
        let resolved = resolve_row(identity, known, row);
        credit(
            &mut record,
            &mut roles,
            &mut conductors,
            &mut ensembles,
            resolved,
            &row.role,
            Some(row.instrument.as_str()),
        );
    }
    let works = extras
        .works
        .iter()
        .map(|work| (work.mbid.as_str(), work))
        .collect::<HashMap<_, _>>();
    for link in &extras.track_works {
        let Some(work) = works.get(link.work_mbid.as_str()) else {
            continue;
        };
        let parent = work
            .parent_mbid
            .as_deref()
            .and_then(|parent| works.get(parent));
        let top = parent.copied().unwrap_or(work);
        if let Some(form) = top.form.as_deref().or(work.form.as_deref()) {
            record.add("form", &form.to_lowercase(), form, None);
        }
        if let Some(year) = top
            .composed
            .as_deref()
            .or(work.composed.as_deref())
            .and_then(first_year)
        {
            let period = period_for_year(year);
            record.add("period", &period.to_lowercase(), period, None);
        }
        if let Some(name) = top.composer_name.as_deref() {
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
            credit(
                &mut record,
                &mut roles,
                &mut conductors,
                &mut ensembles,
                resolved,
                "composer",
                None,
            );
        }
    }
    // A conductor or orchestra billed as the track artist is listed in
    // their role, not again as a performer.
    let billed = conductors
        .iter()
        .chain(&ensembles)
        .map(|person| {
            person
                .artist_id
                .clone()
                .unwrap_or_else(|| normalize_artist_identity(&person.name))
        })
        .collect::<HashSet<_>>();
    let composers = record
        .values("composer")
        .iter()
        .map(|value| value.key.clone())
        .collect::<HashSet<_>>();
    for performer in &performers {
        let key = performer
            .id
            .clone()
            .unwrap_or_else(|| normalize_artist_identity(&performer.name));
        if billed.contains(&key) || composers.contains(&key) {
            continue;
        }
        if looks_like_ensemble(&performer.name) {
            // Tags often name the orchestra only as a track artist.
            push_ref(&mut ensembles, performer);
            record.add_person("ensemble", performer.id.as_deref(), &performer.name);
            continue;
        }
        record.add_person("performer", performer.id.as_deref(), &performer.name);
    }
    roles.sort_by_key(|role| match role.as_str() {
        "main" => 0,
        "featured" => 1,
        other => 2 + role_rank(other),
    });
    if canonical.is_some() {
        for role in &roles {
            record.add("role", role, &role_label(role), None);
        }
    }
    record.add_tracks(&album.songs);
    let best = album
        .songs
        .iter()
        .filter_map(|song| song.audio.as_ref())
        .max_by_key(|audio| format_rank(audio));
    let mut sources = album
        .songs
        .iter()
        .map(|song| song_source(&song.path))
        .collect::<Vec<_>>();
    sources.sort();
    sources.dedup();
    let mut reference = crate::api::album_reference(album);
    if let Some(kind) = &extras.release_type {
        reference.primary_type = kind.clone();
    }
    let card = FocusAlbum {
        album: reference,
        artist: owner.map(crate::api::artist_reference).unwrap_or_default(),
        year: first_year(&album.first_release_date),
        roles,
        conductors,
        ensembles,
        format: best.and_then(format_label),
        quality: best.and_then(quality).map(|quality| match quality {
            Quality::HiRes => "hires",
            Quality::Lossless => "lossless",
            Quality::Lossy => "lossy",
        }),
        sources,
    };
    (card, record)
}

/// Albums a person is on, from the library and their MusicBrainz credits.
pub fn candidate_albums(
    cache: &LibraryCache,
    identity: &PersonIdentity,
    members: &PersonMembers,
    rows: &[CreditRow],
) -> Vec<String> {
    let canonical = members.canonical.as_str();
    let local = members
        .local_ids
        .iter()
        .cloned()
        .collect::<HashSet<String>>();
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    // Only ids and names that are the person can resolve to them.
    let is_person = |id: &str, name: &str, album_id: &str| {
        (local.contains(id) || members.names.contains(&normalize_artist_identity(name)))
            && identity.for_local(id, name, Some(album_id)) == canonical
    };
    for artist in cache.artists.iter() {
        for album in &artist.albums {
            let on_album = is_person(&artist.id, &artist.name, &album.id)
                || album.songs.iter().any(|song| {
                    song.contributing_artist_ids
                        .iter()
                        .zip(&song.contributing_artists)
                        .any(|(id, name)| is_person(id, name, &album.id))
                        || song
                            .credits
                            .iter()
                            .any(|credit| is_person(&credit.artist_id, &credit.name, &album.id))
                });
            if on_album && seen.insert(album.id.clone()) {
                ids.push(album.id.clone());
            }
        }
    }
    for row in rows {
        if cache.album(&row.album_id).is_some()
            && identity.for_row(row).as_deref() == Some(canonical)
            && seen.insert(row.album_id.clone())
        {
            ids.push(row.album_id.clone());
        }
    }
    ids.truncate(MAX_ALBUMS);
    ids
}

/// The focus records and cards for a person's albums, filtered.
pub fn person_discography(
    cache: &LibraryCache,
    identity: &PersonIdentity,
    members: &PersonMembers,
    albums: &[(String, AlbumExtras)],
    selection: &Selection,
) -> (Vec<FocusAlbum>, FocusSummary) {
    let known = known_people(cache);
    let mut built = albums
        .iter()
        .filter_map(|(album_id, extras)| {
            let album = cache.album(album_id)?;
            let (card, record) = album_record(
                cache,
                identity,
                &known,
                Some(&members.canonical),
                album,
                extras,
            );
            // Only albums the person really has a role on.
            (!card.roles.is_empty()).then_some((card, record))
        })
        .collect::<Vec<_>>();
    built.sort_by(|(left, _), (right, _)| {
        right
            .year
            .cmp(&left.year)
            .then_with(|| left.album.name.cmp(&right.album.name))
    });
    let records = built
        .iter()
        .map(|(_, record)| record.clone())
        .collect::<Vec<_>>();
    let (matched, summary) = focus(&records, selection);
    let matched = matched.into_iter().collect::<HashSet<_>>();
    let cards = built
        .into_iter()
        .enumerate()
        .filter(|(index, _)| matched.contains(index))
        .map(|(_, (card, _))| card)
        .collect();
    (cards, summary)
}

/// Every name a person is credited under, other than the page's own.
pub fn credited_names(
    cache: &LibraryCache,
    identity: &PersonIdentity,
    members: &PersonMembers,
    rows: &[CreditRow],
    own: &str,
) -> Vec<String> {
    let own = normalize_artist_identity(own);
    let mut names: Vec<String> = Vec::new();
    let mut push = |name: &str| {
        let normalized = normalize_artist_identity(name);
        if normalized != own
            && !names
                .iter()
                .any(|known| normalize_artist_identity(known) == normalized)
        {
            names.push(name.to_string());
        }
    };
    for id in &members.local_ids {
        if let Some(name) = identity.display_name(id) {
            push(name);
        }
    }
    for row in rows {
        if identity.for_row(row).as_deref() == Some(members.canonical.as_str()) {
            push(&row.name);
        }
    }
    let _ = cache;
    names
}

#[get("/{id}/discography")]
pub async fn get_discography(
    id: web::Path<String>,
    query: web::Query<Vec<(String, String)>>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let requested = id.into_inner();
    let selection = parse_selection(&query.into_inner());
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    let identity = load_identity(pool.get_ref().clone(), cache.clone()).await;
    let members = identity.members(&requested);
    let names = members.names.clone();
    let mbids = members.mbids.clone();
    let rows = blocking(pool.get_ref(), move |connection| {
        store::people_credits(connection, &names, &mbids)
    })
    .await;
    let rows = match rows {
        Ok(rows) => rows,
        Err(error) => {
            tracing::error!(%error, "could not load a discography");
            return internal_server_error("Could not load the discography.", "discography_failed");
        }
    };
    let album_ids = candidate_albums(&cache, &identity, &members, &rows);
    let extras = blocking(pool.get_ref(), move |connection| {
        album_ids
            .into_iter()
            .map(|album_id| {
                let credits = store::album_credits(connection, &album_id)?;
                let (track_works, works) =
                    crate::api::works::load_album_track_works(connection, &album_id)?;
                let release_type = stored_release_type(
                    store::load_profile(connection, "album", &album_id)?.as_ref(),
                );
                Ok((
                    album_id,
                    AlbumExtras {
                        credits,
                        track_works,
                        works,
                        release_type,
                    },
                ))
            })
            .collect::<diesel::QueryResult<Vec<_>>>()
    })
    .await;
    let extras = match extras {
        Ok(extras) => extras,
        Err(error) => {
            tracing::error!(%error, "could not load a discography");
            return internal_server_error("Could not load the discography.", "discography_failed");
        }
    };
    let name = identity
        .display_name(&members.canonical)
        .map(str::to_string)
        .or_else(|| {
            cache
                .artist(&members.canonical)
                .map(|artist| artist.name.clone())
        })
        .or_else(|| {
            rows.iter()
                .find(|row| identity.for_row(row).as_deref() == Some(members.canonical.as_str()))
                .map(|row| row.name.clone())
        })
        .unwrap_or_else(|| requested.clone());
    let (albums, focus) = person_discography(&cache, &identity, &members, &extras, &selection);
    HttpResponse::Ok().json(PersonDiscography {
        credited_as: credited_names(&cache, &identity, &members, &rows, &name),
        artist_id: members.canonical.clone(),
        requested_id: requested,
        name,
        albums,
        focus,
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Credit, Song};
    use crate::library::discography_focus::parse_selection;

    fn credit(id: &str, name: &str, role: &str) -> Credit {
        Credit {
            artist_id: id.into(),
            name: name.into(),
            role: role.into(),
            instrument: None,
        }
    }

    fn audio(bits: u8, rate: u32) -> AudioProperties {
        AudioProperties {
            codec: Some("flac".into()),
            container: Some("flac".into()),
            sample_rate: Some(rate),
            bit_depth: Some(bits),
            lossless: Some(true),
            ..AudioProperties::default()
        }
    }

    fn album(
        id: &str,
        year: &str,
        composer: (&str, &str),
        conductor: (&str, &str),
        ensemble: (&str, &str),
        format: AudioProperties,
    ) -> Album {
        Album {
            id: id.into(),
            name: id.into(),
            first_release_date: year.into(),
            songs: vec![Song {
                id: format!("{id}-1"),
                name: "Symphony No. 5: I. Allegro con brio".into(),
                path: format!("/music/{id}/01.flac"),
                contributing_artists: vec![ensemble.1.into(), conductor.1.into()],
                contributing_artist_ids: vec![ensemble.0.into(), conductor.0.into()],
                audio: Some(format),
                credits: vec![
                    credit(composer.0, composer.1, "composer"),
                    credit(conductor.0, conductor.1, "conductor"),
                ],
                ..Song::default()
            }],
            ..Album::default()
        }
    }

    const BEETHOVEN: (&str, &str) = ("beethoven", "Ludwig van Beethoven");
    const BRAHMS: (&str, &str) = ("brahms", "Johannes Brahms");
    const KARAJAN: (&str, &str) = ("karajan", "Herbert von Karajan");
    const KLEIBER: (&str, &str) = ("kleiber", "Carlos Kleiber");
    const PHILHARMONIA: (&str, &str) = ("philharmonia", "Philharmonia Orchestra");
    const BERLIN: (&str, &str) = ("berlin", "Berliner Philharmoniker");
    const VIENNA: (&str, &str) = ("vienna", "Wiener Philharmoniker");

    fn cache() -> LibraryCache {
        let karajan = Artist {
            id: KARAJAN.0.into(),
            name: KARAJAN.1.into(),
            albums: vec![
                album(
                    "k-philharmonia-hires",
                    "1960",
                    BEETHOVEN,
                    KARAJAN,
                    PHILHARMONIA,
                    audio(24, 96_000),
                ),
                album(
                    "k-philharmonia-cd",
                    "1955",
                    BEETHOVEN,
                    KARAJAN,
                    PHILHARMONIA,
                    audio(16, 44_100),
                ),
                album(
                    "k-berlin-hires",
                    "1962",
                    BEETHOVEN,
                    KARAJAN,
                    BERLIN,
                    audio(24, 192_000),
                ),
                album(
                    "k-brahms",
                    "1958",
                    BRAHMS,
                    KARAJAN,
                    PHILHARMONIA,
                    audio(24, 96_000),
                ),
            ],
            ..Artist::default()
        };
        let kleiber = Artist {
            id: KLEIBER.0.into(),
            name: KLEIBER.1.into(),
            albums: vec![album(
                "kleiber-vienna",
                "1975",
                BEETHOVEN,
                KLEIBER,
                VIENNA,
                audio(24, 96_000),
            )],
            ..Artist::default()
        };
        LibraryCache::from_artists(vec![karajan, kleiber])
    }

    fn page(person: &str, pairs: &[(&str, &str)]) -> (Vec<FocusAlbum>, FocusSummary) {
        let cache = cache();
        let identity = PersonIdentity::build(&cache, &[], &[], &[]);
        let members = identity.members(person);
        let albums = candidate_albums(&cache, &identity, &members, &[])
            .into_iter()
            .map(|id| (id, AlbumExtras::default()))
            .collect::<Vec<_>>();
        let selection = parse_selection(
            &pairs
                .iter()
                .map(|(group, value)| (group.to_string(), value.to_string()))
                .collect::<Vec<_>>(),
        );
        person_discography(&cache, &identity, &members, &albums, &selection)
    }

    fn ids(albums: &[FocusAlbum]) -> Vec<&str> {
        albums.iter().map(|album| album.album.id.as_str()).collect()
    }

    #[test]
    fn a_composer_page_narrows_by_conductor_ensemble_and_bit_depth() {
        // Beethoven, conducted by Karajan, with the Philharmonia, 24-bit.
        let (albums, summary) = page(
            "beethoven",
            &[
                ("conductor", "karajan"),
                ("ensemble", "philharmonia"),
                ("bit_depth", "24"),
            ],
        );
        assert_eq!(ids(&albums), vec!["k-philharmonia-hires"]);
        assert_eq!(summary.total, 4);
        assert_eq!(albums[0].roles, vec!["composer"]);
        assert_eq!(albums[0].format.as_deref(), Some("FLAC 24-bit / 96 kHz"));
        assert_eq!(albums[0].quality, Some("hires"));
        // Without the format the 16-bit Philharmonia album comes back.
        let (albums, _) = page(
            "beethoven",
            &[("conductor", "karajan"), ("ensemble", "philharmonia")],
        );
        assert_eq!(
            ids(&albums),
            vec!["k-philharmonia-hires", "k-philharmonia-cd"]
        );
    }

    #[test]
    fn a_conductor_page_filters_the_other_way_round() {
        // On Karajan's page: his Beethoven with the Philharmonia, hi-res.
        let (albums, summary) = page(
            "karajan",
            &[
                ("composer", "beethoven"),
                ("ensemble", "philharmonia"),
                ("quality", "hires"),
            ],
        );
        assert_eq!(ids(&albums), vec!["k-philharmonia-hires"]);
        assert_eq!(summary.total, 4);
        // Their own role is a facet too.
        let (albums, _) = page("karajan", &[("role", "main")]);
        assert_eq!(albums.len(), 4);
        assert!(albums.iter().all(|album| album.roles[0] == "main"));
        let roles = summary
            .groups
            .iter()
            .find(|group| group.key == "role")
            .map(|group| group.values.len());
        // Main artist and conductor on every album: nothing to narrow.
        assert_eq!(roles, None);
        // The person is never a chip on their own page.
        assert!(
            summary
                .groups
                .iter()
                .flat_map(|group| &group.values)
                .all(|chip| chip.key != "karajan")
        );
    }

    #[test]
    fn conductors_billed_as_track_artists_are_not_performers_too() {
        let (albums, summary) = page("beethoven", &[]);
        assert_eq!(albums.len(), 4);
        let performers = summary.groups.iter().find(|group| group.key == "performer");
        assert!(performers.is_none_or(|group| {
            group
                .values
                .iter()
                .all(|chip| chip.key != "karajan" && chip.key != "kleiber")
        }));
        assert_eq!(albums[0].conductors[0].name, "Carlos Kleiber");
        // Newest first.
        assert_eq!(albums[0].album.id, "kleiber-vienna");
    }

    #[test]
    fn musicbrainz_credits_join_under_the_merged_person() {
        use crate::enrichment::identity::tests::{QTIP, identity};
        let identity = identity();
        let cache = crate::enrichment::identity::tests::cache();
        let members = identity.members(&format!("mb:{QTIP}"));
        assert_eq!(members.canonical, "tag-qtip");
        let row = CreditRow {
            album_id: "low-end".into(),
            track_id: String::new(),
            name: "Jonathan Davis".into(),
            normalized_name: "jonathan davis".into(),
            role: "producer".into(),
            instrument: String::new(),
            source: "musicbrainz".into(),
            source_artist_id: Some(QTIP.into()),
        };
        let albums = candidate_albums(&cache, &identity, &members, std::slice::from_ref(&row));
        assert_eq!(albums, vec!["low-end".to_string()]);
        let extras = AlbumExtras {
            credits: vec![row.clone()],
            ..AlbumExtras::default()
        };
        let (cards, _) = person_discography(
            &cache,
            &identity,
            &members,
            &[("low-end".into(), extras)],
            &Selection::new(),
        );
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].roles, vec!["composer", "producer"]);
        assert_eq!(
            credited_names(&cache, &identity, &members, &[row], "Q-Tip"),
            vec!["Jonathan Davis".to_string()]
        );
    }

    #[test]
    fn musicbrainz_release_types_file_albums_in_the_right_section() {
        let profile = |json: &str| store::ProfileRow {
            musicbrainz_id: None,
            release_group_mbid: None,
            wikidata_id: None,
            profile_json: json.into(),
        };
        assert_eq!(
            stored_release_type(Some(&profile(r#"{"release_type":"Album"}"#))).as_deref(),
            Some("album")
        );
        assert_eq!(
            stored_release_type(Some(&profile(r#"{"release_type":null}"#))),
            None
        );
        assert_eq!(stored_release_type(None), None);
        // A short album the scanner guessed was an EP is filed as MusicBrainz says.
        let cache = cache();
        let identity = PersonIdentity::build(&cache, &[], &[], &[]);
        let known = known_people(&cache);
        let mut album = cache.album("kleiber-vienna").unwrap().clone();
        album.primary_type = "ep".into();
        let extras = AlbumExtras {
            release_type: Some("album".into()),
            ..AlbumExtras::default()
        };
        let (card, _) = album_record(&cache, &identity, &known, Some("kleiber"), &album, &extras);
        assert_eq!(card.album.primary_type, "album");
    }

    #[test]
    fn formats_read_like_a_spec_sheet() {
        assert_eq!(
            format_label(&audio(16, 44_100)).as_deref(),
            Some("FLAC 16-bit / 44.1 kHz")
        );
        let mp3 = AudioProperties {
            codec: Some("mp3".into()),
            bitrate_kbps: Some(320),
            sample_rate: Some(44_100),
            lossless: Some(false),
            ..AudioProperties::default()
        };
        assert_eq!(format_label(&mp3).as_deref(), Some("MP3 320 kbps"));
        let wav = AudioProperties {
            codec: Some("pcm".into()),
            container: Some("wav".into()),
            sample_rate: Some(22_050),
            bit_depth: Some(16),
            lossless: Some(true),
            ..AudioProperties::default()
        };
        assert_eq!(
            format_label(&wav).as_deref(),
            Some("WAV 16-bit / 22.05 kHz")
        );
    }
}
