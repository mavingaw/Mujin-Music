//! The background enrichment worker. It consumes `metadata_task` rows only
//! while the administrator has turned enrichment on.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use diesel::sqlite::SqliteConnection;
use serde_json::{Value, json};
use tokio::sync::Notify;

use super::fetch::{BoxError, Fetch, FetchResponse, HttpFetch, MAX_IMAGE_BYTES, MAX_JSON_BYTES};
use super::musicbrainz as mb;
use super::store::{
    self, CreditRow, EditorialText, EnrichmentSettings, ImageRow, Outcome, ProfileRow, Stage, Task,
    TrackWorkRow, WorkRow,
};
use super::wikimedia as wiki;
use super::works::WorkInfo;
use crate::library::identity::{normalize_artist_identity, normalize_song_identity};
use crate::library::state::LibraryLifecycle;
use crate::persistence::connection::DbPool;
use crate::settings::data_path;

const IDLE_POLL: Duration = Duration::from_secs(60);
const MIN_THROTTLE: Duration = Duration::from_secs(30);
const MAX_THROTTLE: Duration = Duration::from_secs(15 * 60);

/// How long one host's queue pauses after that host said it is overloaded
/// (MusicBrainz answers 503 when a client goes over its rate limit,
/// Wikimedia answers 429). Doubles on each consecutive refusal and resets
/// after a success.
pub fn next_throttle(current: Duration, outcome: &Outcome) -> Duration {
    match outcome {
        Outcome::Retry(error) if error == "HTTP 429" || error == "HTTP 503" => {
            (current * 2).clamp(MIN_THROTTLE, MAX_THROTTLE)
        }
        Outcome::Done | Outcome::Skipped(_) => Duration::ZERO,
        Outcome::Retry(_) => current,
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct HostBackoff {
    delay: Duration,
    until: Option<Instant>,
}

/// A separate back-off for each host. A 429 from Wikimedia Commons pauses
/// only the Commons queue; MusicBrainz, Wikidata and Wikipedia carry on.
#[derive(Debug, Default)]
pub struct Backoffs {
    hosts: Mutex<HashMap<Stage, HostBackoff>>,
}

impl Backoffs {
    /// Records a stage's outcome and returns how long its queue now pauses.
    pub fn record(&self, stage: Stage, outcome: &Outcome, now: Instant) -> Duration {
        let mut hosts = self.hosts.lock().unwrap_or_else(|error| error.into_inner());
        let entry = hosts.entry(stage).or_default();
        entry.delay = next_throttle(entry.delay, outcome);
        let busy =
            matches!(outcome, Outcome::Retry(error) if error == "HTTP 429" || error == "HTTP 503");
        entry.until = match (busy, entry.delay.is_zero()) {
            (true, false) => Some(now + entry.delay),
            (_, true) => None,
            (false, false) => entry.until,
        };
        if busy { entry.delay } else { Duration::ZERO }
    }

    /// How much longer the stage's host is paused.
    pub fn paused_for(&self, stage: Stage, now: Instant) -> Duration {
        let hosts = self.hosts.lock().unwrap_or_else(|error| error.into_inner());
        hosts
            .get(&stage)
            .and_then(|entry| entry.until)
            .map(|until| until.saturating_duration_since(now))
            .unwrap_or_default()
    }
}

/// The worker's back-offs, shared with the settings API so it can show
/// which service is busy.
pub fn backoffs() -> &'static Backoffs {
    static BACKOFFS: std::sync::OnceLock<Backoffs> = std::sync::OnceLock::new();
    BACKOFFS.get_or_init(Backoffs::default)
}

/// One wake-up signal per stage queue.
pub struct Wake {
    stages: [Notify; 4],
}

impl Wake {
    /// Wakes every stage (settings changed, an entity was refreshed, or a
    /// stage queued work for the next one).
    pub fn notify_all(&self) {
        for notify in &self.stages {
            notify.notify_one();
        }
    }

    fn stage(&self, stage: Stage) -> &Notify {
        let index = Stage::ALL
            .iter()
            .position(|candidate| *candidate == stage)
            .unwrap_or(0);
        &self.stages[index]
    }
}

/// Wakes the worker when settings change or an entity is refreshed.
pub fn wake() -> &'static Wake {
    static WAKE: std::sync::OnceLock<Wake> = std::sync::OnceLock::new();
    WAKE.get_or_init(|| Wake {
        stages: [Notify::new(), Notify::new(), Notify::new(), Notify::new()],
    })
}

/// What one turn of a stage's queue did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// The host is backing off for this long.
    Paused(Duration),
    /// Nothing is due in this queue.
    Idle,
    /// A task ran.
    Ran(Task, Outcome),
}

/// What the Wikidata stage stored for the later stages.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct WikidataLinks {
    article: Option<(String, String)>,
    image: Option<String>,
}

fn wikidata_links(profile: &ProfileRow) -> WikidataLinks {
    let value = serde_json::from_str::<Value>(&profile.profile_json).unwrap_or_default();
    let wikidata = value.get(WIKIDATA_KEY);
    let text = |key: &str| {
        wikidata
            .and_then(|links| links.get(key))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    WikidataLinks {
        article: text("article_language").zip(text("article_title")),
        image: text("image"),
    }
}

/// The profile key holding what Wikidata linked to. It is internal and is
/// not shown as a fact.
pub const WIKIDATA_KEY: &str = "wikidata";

pub fn images_directory() -> PathBuf {
    data_path(&["Editorial", "Images"])
}

/// What the worker needs to know about a local album.
#[derive(Debug, Clone, Default)]
pub struct AlbumInput {
    pub id: String,
    pub title: String,
    pub artist_id: String,
    pub artist_name: String,
    /// Release MBID from tags, when present.
    pub musicbrainz_id: Option<String>,
    pub tracks: Vec<TrackInput>,
    /// Local artists by normalized name, for linking external credits.
    pub known_artists: Vec<(String, String)>,
    /// Release year from tags, to choose between recordings.
    pub year: Option<String>,
    /// The release was chosen by a person or named in tags, so tracks may
    /// be matched by position even when the track counts differ.
    pub release_trusted: bool,
}

#[derive(Debug, Clone, Default)]
pub struct TrackInput {
    pub id: String,
    pub title: String,
    pub track_number: u16,
}

#[derive(Debug, Clone, Default)]
pub struct ArtistInput {
    pub id: String,
    pub name: String,
}

pub struct Enricher<F> {
    pub fetch: F,
    pub pool: DbPool,
    pub images: PathBuf,
}

async fn db<T: Send + 'static>(
    pool: &DbPool,
    work: impl FnOnce(&mut SqliteConnection) -> diesel::QueryResult<T> + Send + 'static,
) -> Result<T, BoxError> {
    let pool = pool.clone();
    tokio::task::spawn_blocking(move || {
        let mut connection = pool.get()?;
        Ok::<_, BoxError>(work(&mut connection)?)
    })
    .await?
}

fn transient(response: &FetchResponse) -> Option<Outcome> {
    match response.status {
        429 | 500..=599 => Some(Outcome::Retry(format!("HTTP {}", response.status))),
        _ => None,
    }
}

fn json_body(response: &FetchResponse) -> Option<Value> {
    response
        .is_success()
        .then(|| serde_json::from_slice(&response.body).ok())
        .flatten()
}

/// The file a stored image is written to; ids are URL-safe hashes but are
/// checked again so a crafted id can never escape the directory.
pub fn image_file_name(entity_type: &str, entity_id: &str, mime_type: &str) -> Option<String> {
    let extension = match mime_type {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        _ => return None,
    };
    let safe = |value: &str| {
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    };
    (safe(entity_type) && safe(entity_id)).then(|| format!("{entity_type}-{entity_id}.{extension}"))
}

fn sniff_image(bytes: &[u8], mime_type: &str) -> bool {
    match mime_type {
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/webp" => bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
        _ => false,
    }
}

impl<F: Fetch> Enricher<F> {
    async fn get_json(&self, url: &str) -> Result<Result<Option<Value>, Outcome>, BoxError> {
        let response = self.fetch.get(url, MAX_JSON_BYTES).await?;
        if let Some(outcome) = transient(&response) {
            return Ok(Err(outcome));
        }
        Ok(Ok(json_body(&response)))
    }

    /// The MusicBrainz stage for an album: the release, its credits, works
    /// and the release group's Wikidata item. Text comes later from the
    /// Wikidata and Wikipedia queues.
    pub async fn resolve_album(&self, album: &AlbumInput) -> Result<Outcome, BoxError> {
        let album_id = album.id.clone();
        let identified = db(&self.pool, move |connection| {
            store::load_identification(connection, &album_id)
        })
        .await?
        .map(|pinned| pinned.release_mbid)
        .filter(|id| mb::is_mbid(id));
        let tagged = album.musicbrainz_id.clone().filter(|id| mb::is_mbid(id));
        let release_mbid = match identified.clone().or(tagged) {
            Some(mbid) => mbid,
            None => {
                if album.artist_name.trim().is_empty() || album.title.trim().is_empty() {
                    return Ok(Outcome::Skipped("the album has no title or artist".into()));
                }
                if mb::is_placeholder_artist(&album.artist_name) {
                    // "Unknown Artist" would match a real namesake; only a
                    // person can say which release this is.
                    let album_id = album.id.clone();
                    db(&self.pool, move |connection| {
                        store::clear_editorial(connection, "album", &album_id)?;
                        drop_later_stages(connection, "album", &album_id)
                    })
                    .await?;
                    return Ok(Outcome::Skipped(
                        "the album artist is a placeholder; use Identify album".into(),
                    ));
                }
                let body = match self
                    .get_json(&mb::release_search_url(&album.title, &album.artist_name))
                    .await?
                {
                    Ok(Some(body)) => body,
                    Ok(None) => return Ok(Outcome::Retry("release search failed".into())),
                    Err(outcome) => return Ok(outcome),
                };
                let candidates = mb::parse_release_search(&body);
                match mb::choose_release_in_year(
                    &album.title,
                    &album.artist_name,
                    album.tracks.len(),
                    album.year.as_deref(),
                    &candidates,
                ) {
                    Some(release) => release.mbid,
                    None => {
                        return Ok(Outcome::Skipped(
                            "no confident MusicBrainz release match".into(),
                        ));
                    }
                }
            }
        };
        let release = match self
            .get_json(&mb::release_lookup_url(&release_mbid))
            .await?
        {
            Ok(Some(body)) => mb::parse_release(&body),
            Ok(None) => None,
            Err(outcome) => return Ok(outcome),
        };
        let Some(release) = release else {
            return Ok(Outcome::Skipped(
                "the MusicBrainz release was not found".into(),
            ));
        };

        let mut wikidata_id = None;
        if let Some(group) = &release.release_group_mbid {
            match self.get_json(&mb::release_group_lookup_url(group)).await? {
                Ok(Some(body)) => wikidata_id = mb::wikidata_relation(&body),
                Ok(None) => {}
                Err(outcome) => return Ok(outcome),
            }
        }

        let trusted = identified.as_deref() == Some(release.mbid.as_str())
            || album.musicbrainz_id.as_deref() == Some(release.mbid.as_str());
        let exact = trusted || release.tracks.len() == album.tracks.len();
        let album = &AlbumInput {
            release_trusted: trusted,
            ..album.clone()
        };
        let credits = album_credit_rows(album, &release);
        let links = artist_links(album, &release);
        let (works, work_links) = album_work_rows(album, &release);
        let profile = ProfileRow {
            musicbrainz_id: Some(release.mbid.clone()),
            release_group_mbid: release.release_group_mbid.clone(),
            wikidata_id,
            profile_json: json!({
                // Edition facts (date, country, catalog number, barcode) are
                // only shown when this exact release is known to be the
                // local one: a tagged id or the same track list.
                "exact": exact,
                "date": release.date,
                "first_release_date": release.first_release_date,
                "country": release.country,
                "barcode": release.barcode,
                "release_type": release.release_group_type,
                "labels": release.labels.iter().map(|(name, catalog_number)| json!({
                    "name": name, "catalog_number": catalog_number
                })).collect::<Vec<_>>(),
            })
            .to_string(),
        };
        let album_id = album.id.clone();
        db(&self.pool, move |connection| {
            let linked = profile.wikidata_id.is_some();
            store::save_profile(connection, "album", &album_id, &profile)?;
            store::replace_album_credits(connection, &album_id, "musicbrainz", &credits)?;
            store::replace_album_works(connection, &album_id, &works, &work_links)?;
            for (artist_id, mbid) in &links {
                store::link_artist_mbid(connection, artist_id, mbid)?;
            }
            queue_wikidata(connection, "album", &album_id, linked)
        })
        .await?;
        Ok(Outcome::Done)
    }

    /// The MusicBrainz stage for an artist: who they are and their Wikidata
    /// item. The biography and photo come from the later queues.
    pub async fn resolve_artist(&self, artist: &ArtistInput) -> Result<Outcome, BoxError> {
        if artist.name.trim().is_empty() {
            return Ok(Outcome::Skipped("the artist has no name".into()));
        }
        let wanted = normalize_artist_identity(&artist.name);
        if mb::is_placeholder_artist(&artist.name) {
            // Drop anything an earlier version matched for this name.
            let artist_id = artist.id.clone();
            db(&self.pool, move |connection| {
                store::clear_editorial(connection, "artist", &artist_id)?;
                drop_later_stages(connection, "artist", &artist_id)
            })
            .await?;
            return Ok(Outcome::Skipped(format!(
                "\"{}\" is a placeholder, not an artist",
                artist.name.trim()
            )));
        }
        let artist_id = artist.id.clone();
        let known = db(&self.pool, move |connection| {
            store::linked_artist_mbids(connection, &artist_id)
        })
        .await?;

        // A tagged or release-linked id is used when its name matches.
        let mut profile = None;
        for mbid in known.iter().filter(|mbid| mb::is_mbid(mbid)) {
            match self.get_json(&mb::artist_lookup_url(mbid)).await? {
                Ok(Some(body)) => {
                    if let Some(found) = mb::parse_artist(&body)
                        && normalize_artist_identity(&found.name) == wanted
                    {
                        profile = Some(found);
                        break;
                    }
                }
                Ok(None) => {}
                Err(outcome) => return Ok(outcome),
            }
        }
        if profile.is_none() {
            let body = match self.get_json(&mb::artist_search_url(&artist.name)).await? {
                Ok(Some(body)) => body,
                Ok(None) => return Ok(Outcome::Retry("artist search failed".into())),
                Err(outcome) => return Ok(outcome),
            };
            let Some(candidate) =
                mb::choose_artist(&artist.name, &mb::parse_artist_search(&body), &known)
            else {
                return Ok(Outcome::Skipped(
                    "no unambiguous MusicBrainz artist match".into(),
                ));
            };
            profile = match self
                .get_json(&mb::artist_lookup_url(&candidate.mbid))
                .await?
            {
                Ok(Some(body)) => mb::parse_artist(&body),
                Ok(None) => None,
                Err(outcome) => return Ok(outcome),
            };
        }
        let Some(profile) = profile else {
            return Ok(Outcome::Skipped(
                "the MusicBrainz artist was not found".into(),
            ));
        };

        let row = ProfileRow {
            musicbrainz_id: Some(profile.mbid.clone()),
            release_group_mbid: None,
            wikidata_id: profile.wikidata_id.clone(),
            profile_json: json!({
                "name": profile.name,
                "kind": profile.kind,
                "area": profile.area,
                "begin_area": profile.begin_area,
                "begin": profile.begin,
                "end": profile.end,
                "disambiguation": profile.disambiguation,
            })
            .to_string(),
        };
        let artist_id = artist.id.clone();
        db(&self.pool, move |connection| {
            let linked = row.wikidata_id.is_some();
            store::save_profile(connection, "artist", &artist_id, &row)?;
            queue_wikidata(connection, "artist", &artist_id, linked)
        })
        .await?;
        Ok(Outcome::Done)
    }

    /// The Wikidata stage: the Wikipedia article in the preferred language,
    /// life dates and the P18 image, then the Wikipedia and Commons queues.
    pub async fn wikidata_stage(
        &self,
        entity_type: &str,
        entity_id: &str,
        settings: &EnrichmentSettings,
    ) -> Result<Outcome, BoxError> {
        let Some(mut profile) = self.profile(entity_type, entity_id).await? else {
            return Ok(Outcome::Skipped("not matched on MusicBrainz yet".into()));
        };
        let Some(wikidata_id) = profile.wikidata_id.clone() else {
            return Ok(Outcome::Skipped("no Wikidata item".into()));
        };
        let entity = match self
            .get_json(&wiki::wikidata_entity_url(&wikidata_id))
            .await?
        {
            Ok(Some(body)) => wiki::parse_wikidata_entity(&body),
            Ok(None) => None,
            Err(outcome) => return Ok(outcome),
        };
        let Some(entity) = entity else {
            return Ok(Outcome::Skipped("the Wikidata item was not found".into()));
        };
        let article = entity.article(&settings.language);
        let image = (entity_type == "artist")
            .then(|| entity.image.clone())
            .flatten();
        let mut facts = serde_json::from_str::<Value>(&profile.profile_json)
            .ok()
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        if entity_type == "artist" {
            if let Some(begin) = entity.born.clone().or(entity.formed.clone()) {
                facts["begin"] = json!(begin);
            }
            if let Some(end) = entity.died.clone().or(entity.dissolved.clone()) {
                facts["end"] = json!(end);
            }
        }
        facts[WIKIDATA_KEY] = json!({
            "article_language": article.as_ref().map(|(language, _)| language),
            "article_title": article.as_ref().map(|(_, title)| title),
            "image": image,
        });
        profile.profile_json = facts.to_string();
        let entity_type = entity_type.to_string();
        let entity_id = entity_id.to_string();
        let wants_image = image.is_some() && settings.artist_images;
        db(&self.pool, move |connection| {
            store::save_profile(connection, &entity_type, &entity_id, &profile)?;
            if article.is_some() {
                store::queue_stage(connection, Stage::Wikipedia, &entity_type, &entity_id)?;
            } else {
                store::replace_texts(connection, &entity_type, &entity_id, &[])?;
                store::drop_stage(connection, Stage::Wikipedia, &entity_type, &entity_id)?;
            }
            if wants_image {
                store::queue_stage(connection, Stage::Commons, &entity_type, &entity_id)?;
            } else {
                store::drop_stage(connection, Stage::Commons, &entity_type, &entity_id)?;
            }
            Ok(())
        })
        .await?;
        Ok(Outcome::Done)
    }

    /// The Wikipedia stage: an artist biography, or an album's background
    /// and critical reception, with CC BY-SA attribution.
    pub async fn wikipedia_stage(
        &self,
        entity_type: &str,
        entity_id: &str,
    ) -> Result<Outcome, BoxError> {
        let Some(profile) = self.profile(entity_type, entity_id).await? else {
            return Ok(Outcome::Skipped("not matched on MusicBrainz yet".into()));
        };
        let Some((language, title)) = wikidata_links(&profile).article else {
            return Ok(Outcome::Skipped("no Wikipedia article".into()));
        };
        let article = match self
            .get_json(&wiki::wikipedia_extract_url(&language, &title))
            .await?
        {
            Ok(Some(body)) => wiki::parse_wikipedia_extract(&body),
            Ok(None) => None,
            Err(outcome) => return Ok(outcome),
        };
        let Some(article) = article else {
            return Ok(Outcome::Skipped(
                "the Wikipedia article was not found".into(),
            ));
        };
        let mut texts = Vec::new();
        if entity_type == "artist" {
            texts.push(wikipedia_text(
                "biography",
                &language,
                &article,
                &article.lead,
            ));
        } else {
            texts.push(wikipedia_text(
                "background",
                &language,
                &article,
                &article.lead,
            ));
            if let Some(reception) = &article.reception {
                texts.push(wikipedia_text("reception", &language, &article, reception));
            }
        }
        let entity_type = entity_type.to_string();
        let entity_id = entity_id.to_string();
        db(&self.pool, move |connection| {
            store::replace_texts(connection, &entity_type, &entity_id, &texts)
        })
        .await?;
        Ok(Outcome::Done)
    }

    /// The Commons stage: the artist's freely licensed photo, cached under
    /// the data directory.
    pub async fn commons_stage(
        &self,
        entity_type: &str,
        entity_id: &str,
        settings: &EnrichmentSettings,
    ) -> Result<Outcome, BoxError> {
        if entity_type != "artist" || !settings.artist_images {
            return Ok(Outcome::Skipped("artist photos are off".into()));
        }
        let Some(profile) = self.profile(entity_type, entity_id).await? else {
            return Ok(Outcome::Skipped("not matched on MusicBrainz yet".into()));
        };
        let Some(file) = wikidata_links(&profile).image else {
            return Ok(Outcome::Skipped("no Wikidata image".into()));
        };
        let image = match self.download_image(entity_id, &file).await? {
            Ok(Some(image)) => image,
            Ok(None) => {
                return Ok(Outcome::Skipped(
                    "no freely licensed photo on Commons".into(),
                ));
            }
            Err(outcome) => return Ok(outcome),
        };
        let entity_id = entity_id.to_string();
        db(&self.pool, move |connection| {
            store::save_image(connection, "artist", &entity_id, &image)
        })
        .await?;
        Ok(Outcome::Done)
    }

    async fn profile(
        &self,
        entity_type: &str,
        entity_id: &str,
    ) -> Result<Option<ProfileRow>, BoxError> {
        let entity_type = entity_type.to_string();
        let entity_id = entity_id.to_string();
        db(&self.pool, move |connection| {
            store::load_profile(connection, &entity_type, &entity_id)
        })
        .await
    }

    /// Runs one stage for one task.
    pub async fn run_task(
        &self,
        stage: Stage,
        task: &Task,
        settings: &EnrichmentSettings,
        cache: Option<&crate::library::state::LibraryCache>,
    ) -> Result<Outcome, BoxError> {
        match (stage, task.entity_type.as_str()) {
            (Stage::Musicbrainz, "album") => {
                let Some(cache) = cache else {
                    return Ok(Outcome::Retry("the library is not ready".into()));
                };
                match album_input(cache, &task.entity_id) {
                    Some(input) => self.resolve_album(&input).await,
                    None => Ok(Outcome::Skipped(
                        "the album is no longer in the library".into(),
                    )),
                }
            }
            (Stage::Musicbrainz, _) => {
                let artist_id = task.entity_id.clone();
                match db(&self.pool, move |connection| {
                    store::artist_name(connection, &artist_id)
                })
                .await?
                {
                    Some(name) => {
                        self.resolve_artist(&ArtistInput {
                            id: task.entity_id.clone(),
                            name,
                        })
                        .await
                    }
                    None => Ok(Outcome::Skipped(
                        "the artist is no longer in the library".into(),
                    )),
                }
            }
            (Stage::Wikidata, entity_type) => {
                self.wikidata_stage(entity_type, &task.entity_id, settings)
                    .await
            }
            (Stage::Wikipedia, entity_type) => {
                self.wikipedia_stage(entity_type, &task.entity_id).await
            }
            (Stage::Commons, entity_type) => {
                self.commons_stage(entity_type, &task.entity_id, settings)
                    .await
            }
        }
    }

    /// One turn of a stage's queue: nothing while its host backs off,
    /// otherwise the next due task, recorded with its outcome. Other
    /// stages are never held back by this one.
    pub async fn step(
        &self,
        stage: Stage,
        settings: &EnrichmentSettings,
        cache: Option<&crate::library::state::LibraryCache>,
        backoffs: &Backoffs,
    ) -> Result<Step, BoxError> {
        let paused = backoffs.paused_for(stage, Instant::now());
        if !paused.is_zero() {
            return Ok(Step::Paused(paused));
        }
        let Some(task) = db(&self.pool, move |connection| {
            store::next_stage_task(connection, stage)
        })
        .await?
        else {
            return Ok(Step::Idle);
        };
        let outcome = self
            .run_task(stage, &task, settings, cache)
            .await
            .unwrap_or_else(|error| Outcome::Retry(error.to_string()));
        backoffs.record(stage, &outcome, Instant::now());
        let finished = task.clone();
        let recorded = outcome.clone();
        db(&self.pool, move |connection| {
            store::finish_task(connection, &finished, &recorded)
        })
        .await?;
        Ok(Step::Ran(task, outcome))
    }

    /// Every stage for one album in order, as the queues would run them.
    /// The first busy service's answer is returned; earlier stages' results
    /// are kept.
    #[cfg(test)]
    pub async fn enrich_album(
        &self,
        album: &AlbumInput,
        settings: &EnrichmentSettings,
    ) -> Result<Outcome, BoxError> {
        let outcome = self.resolve_album(album).await?;
        if outcome != Outcome::Done {
            return Ok(outcome);
        }
        self.later_stages("album", &album.id, settings).await
    }

    /// Every stage for one artist in order, as the queues would run them.
    #[cfg(test)]
    pub async fn enrich_artist(
        &self,
        artist: &ArtistInput,
        settings: &EnrichmentSettings,
    ) -> Result<Outcome, BoxError> {
        let outcome = self.resolve_artist(artist).await?;
        if outcome != Outcome::Done {
            return Ok(outcome);
        }
        self.later_stages("artist", &artist.id, settings).await
    }

    #[cfg(test)]
    async fn later_stages(
        &self,
        entity_type: &str,
        entity_id: &str,
        settings: &EnrichmentSettings,
    ) -> Result<Outcome, BoxError> {
        let mut result = Outcome::Done;
        for stage in &Stage::ALL[1..] {
            let queued = {
                let (entity_type, entity_id) = (entity_type.to_string(), entity_id.to_string());
                let stage = *stage;
                db(&self.pool, move |connection| {
                    Ok(
                        store::task_status(connection, stage, &entity_type, &entity_id)?.as_deref()
                            == Some("pending"),
                    )
                })
                .await?
            };
            if !queued {
                continue;
            }
            let outcome = match stage {
                Stage::Wikidata => {
                    self.wikidata_stage(entity_type, entity_id, settings)
                        .await?
                }
                Stage::Wikipedia => self.wikipedia_stage(entity_type, entity_id).await?,
                _ => self.commons_stage(entity_type, entity_id, settings).await?,
            };
            if let Outcome::Retry(_) = outcome {
                if result == Outcome::Done {
                    result = outcome;
                }
                if *stage == Stage::Wikidata {
                    break;
                }
            }
        }
        Ok(result)
    }

    /// Downloads a freely licensed Commons image. A busy Commons is an
    /// `Err(outcome)` so the artist is looked up again later.
    async fn download_image(
        &self,
        artist_id: &str,
        file: &str,
    ) -> Result<Result<Option<ImageRow>, Outcome>, BoxError> {
        let body = match self.get_json(&wiki::commons_image_info_url(file)).await? {
            Ok(Some(body)) => body,
            Ok(None) => return Ok(Ok(None)),
            Err(outcome) => return Ok(Err(outcome)),
        };
        let Some(image) = wiki::parse_commons_image(&body) else {
            return Ok(Ok(None));
        };
        let Some(file_name) = image_file_name("artist", artist_id, &image.mime_type) else {
            return Ok(Ok(None));
        };
        let response = self.fetch.get(&image.download_url, MAX_IMAGE_BYTES).await?;
        if let Some(outcome) = transient(&response) {
            return Ok(Err(outcome));
        }
        if !response.is_success() || !sniff_image(&response.body, &image.mime_type) {
            return Ok(Ok(None));
        }
        write_atomically(&self.images, &file_name, &response.body).await?;
        Ok(Ok(Some(ImageRow {
            file_name,
            mime_type: image.mime_type,
            source_url: image.download_url,
            page_url: image.page_url,
            author: image.author,
            license: image.license,
            license_url: image.license_url,
        })))
    }
}

/// Queues the Wikidata stage when MusicBrainz linked an item; otherwise
/// clears text and photo lookups that no longer apply.
fn queue_wikidata(
    connection: &mut SqliteConnection,
    entity_type: &str,
    entity_id: &str,
    linked: bool,
) -> diesel::QueryResult<()> {
    if linked {
        store::queue_stage(connection, Stage::Wikidata, entity_type, entity_id)
    } else {
        store::replace_texts(connection, entity_type, entity_id, &[])?;
        drop_later_stages(connection, entity_type, entity_id)
    }
}

fn drop_later_stages(
    connection: &mut SqliteConnection,
    entity_type: &str,
    entity_id: &str,
) -> diesel::QueryResult<()> {
    for stage in &Stage::ALL[1..] {
        store::drop_stage(connection, *stage, entity_type, entity_id)?;
    }
    Ok(())
}

async fn write_atomically(directory: &Path, file_name: &str, bytes: &[u8]) -> std::io::Result<()> {
    tokio::fs::create_dir_all(directory).await?;
    let temporary = directory.join(format!("{file_name}.tmp"));
    tokio::fs::write(&temporary, bytes).await?;
    tokio::fs::rename(&temporary, directory.join(file_name)).await
}

fn wikipedia_text(
    kind: &str,
    language: &str,
    article: &wiki::WikipediaArticle,
    body: &str,
) -> EditorialText {
    EditorialText {
        kind: kind.to_string(),
        source: "wikipedia".to_string(),
        language: language.to_string(),
        title: article.title.clone(),
        body: body.to_string(),
        url: article.url.clone(),
        license: wiki::WIKIPEDIA_LICENSE.to_string(),
        license_url: wiki::WIKIPEDIA_LICENSE_URL.to_string(),
    }
}

/// The part of a track title after the last colon ("…, Op. 125: II. Molto
/// vivace" -> "ii molto vivace"), where movements differ.
fn title_tail(title: &str) -> String {
    let tail = title.rsplit_once(':').map_or(title, |(_, tail)| tail);
    let normalized = normalize_song_identity(tail);
    // "I. Grave" and "1. Grave" are the same movement.
    let mut words = normalized.split(' ');
    let first = words.next().unwrap_or_default();
    let rest = words.collect::<Vec<_>>().join(" ");
    let mut words = match roman_number(first) {
        Some(number) => format!("{number} {rest}"),
        None => normalized.clone(),
    };
    // "Adagio sostenuto - Attacca" is the same movement as "Adagio
    // sostenuto".
    if let Some(stripped) = words.strip_suffix(" attacca") {
        words = stripped.to_string();
    }
    words.trim().to_string()
}

fn roman_number(word: &str) -> Option<u32> {
    if word.is_empty() || word.len() > 6 {
        return None;
    }
    let value = |c| match c {
        'i' => Some(1),
        'v' => Some(5),
        'x' => Some(10),
        'l' => Some(50),
        _ => None,
    };
    let digits = word.chars().map(value).collect::<Option<Vec<u32>>>()?;
    let mut total = 0;
    for (index, digit) in digits.iter().enumerate() {
        if digits.get(index + 1).is_some_and(|next| next > digit) {
            total -= *digit as i64;
        } else {
            total += *digit as i64;
        }
    }
    u32::try_from(total).ok().filter(|total| *total > 0)
}

/// Matches MusicBrainz tracks to local tracks: by title, then by the
/// movement part of the title (tags often shorten "I. Allegro ma non
/// troppo, un poco maestoso"), then by position when the track lists have
/// the same length (and, for a release with several discs, only when a
/// person chose it or tags name it).
pub fn match_tracks(album: &AlbumInput, release: &mb::ReleaseDetails) -> Vec<(usize, String)> {
    let single_disc = release.tracks.iter().all(|track| track.disc == 1);
    let same_length = release.tracks.len() == album.tracks.len();
    let mut matched: Vec<Option<usize>> = vec![None; release.tracks.len()];
    let mut used = vec![false; album.tracks.len()];
    let unique = |used: &[bool], test: &dyn Fn(&TrackInput) -> bool| {
        let found = album
            .tracks
            .iter()
            .enumerate()
            .filter(|(local, track)| !used[*local] && test(track))
            .map(|(local, _)| local)
            .collect::<Vec<_>>();
        match found.as_slice() {
            [only] => Some(*only),
            _ => None,
        }
    };
    for (index, track) in release.tracks.iter().enumerate() {
        let title = normalize_song_identity(&track.title);
        if let Some(local) = unique(&used, &|local| {
            normalize_song_identity(&local.title) == title
        }) {
            matched[index] = Some(local);
            used[local] = true;
        }
    }
    for (index, track) in release.tracks.iter().enumerate() {
        if matched[index].is_some() {
            continue;
        }
        let tail = title_tail(&track.title);
        if tail.chars().count() < 6 {
            continue;
        }
        let similar = |local: &TrackInput| {
            let local_tail = title_tail(&local.title);
            local_tail.chars().count() >= 6
                && (local_tail == tail
                    || tail.starts_with(&format!("{local_tail} "))
                    || local_tail.starts_with(&format!("{tail} ")))
        };
        if let Some(local) = unique(&used, &similar) {
            matched[index] = Some(local);
            used[local] = true;
        }
    }
    if same_length && (single_disc || album.release_trusted) {
        for (index, track) in release.tracks.iter().enumerate() {
            if matched[index].is_some() {
                continue;
            }
            if let Some(local) = unique(&used, &|local| {
                i64::from(local.track_number) == track.position
            }) {
                matched[index] = Some(local);
                used[local] = true;
            }
        }
    }
    matched
        .into_iter()
        .enumerate()
        .filter_map(|(index, local)| Some((index, album.tracks[local?].id.clone())))
        .collect()
}

fn album_credit_rows(album: &AlbumInput, release: &mb::ReleaseDetails) -> Vec<CreditRow> {
    let row = |track_id: &str, credit: &mb::ExternalCredit| CreditRow {
        album_id: album.id.clone(),
        track_id: track_id.to_string(),
        name: credit.name.clone(),
        normalized_name: normalize_artist_identity(&credit.name),
        role: credit.role.clone(),
        instrument: credit.instrument.clone().unwrap_or_default(),
        source: "musicbrainz".to_string(),
        source_artist_id: credit.artist_mbid.clone(),
    };
    let mut rows = release
        .credits
        .iter()
        .map(|credit| row("", credit))
        .collect::<Vec<_>>();
    for (index, track_id) in match_tracks(album, release) {
        rows.extend(
            release.tracks[index]
                .credits
                .iter()
                .map(|credit| row(&track_id, credit)),
        );
    }
    rows
}

fn work_row(work: &WorkInfo, parent: Option<&WorkInfo>) -> WorkRow {
    let composer = work.composers.first();
    WorkRow {
        mbid: work.mbid.clone(),
        title: work.title.clone(),
        form: work.form.clone(),
        musical_key: work.key.clone(),
        catalogue: work.catalogue.clone(),
        composed: work.composed.clone(),
        composer_name: composer.map(|composer| composer.name.clone()),
        composer_normalized: composer.map(|composer| normalize_artist_identity(&composer.name)),
        composer_mbid: composer.and_then(|composer| composer.mbid.clone()),
        language: work.language.clone(),
        parent_mbid: parent.map(|parent| parent.mbid.clone()),
        part_position: work.position,
    }
}

/// Works performed on the release (movements and the pieces they belong
/// to) and the local tracks that perform them.
pub fn album_work_rows(
    album: &AlbumInput,
    release: &mb::ReleaseDetails,
) -> (Vec<WorkRow>, Vec<TrackWorkRow>) {
    let mut works: Vec<WorkRow> = Vec::new();
    let mut add = |row: WorkRow| {
        if !works.iter().any(|known| known.mbid == row.mbid) {
            works.push(row);
        }
    };
    let mut links = Vec::new();
    for (index, track_id) in match_tracks(album, release) {
        let track = &release.tracks[index];
        for work in &track.works {
            if let Some(parent) = work.parent.as_deref() {
                add(work_row(parent, None));
            }
            add(work_row(work, work.parent.as_deref()));
            links.push(TrackWorkRow {
                track_id: track_id.clone(),
                album_id: album.id.clone(),
                work_mbid: work.mbid.clone(),
                recording_mbid: track.recording_mbid.clone(),
            });
        }
    }
    (works, links)
}

/// Local artists named on the release (as album artist or in a credit) and
/// the MusicBrainz ids MusicBrainz gives them.
fn artist_links(album: &AlbumInput, release: &mb::ReleaseDetails) -> Vec<(String, String)> {
    let mut links = Vec::new();
    let mut add = |name: &str, mbid: &str| {
        let normalized = normalize_artist_identity(name);
        let local = if normalized == normalize_artist_identity(&album.artist_name) {
            Some(album.artist_id.clone())
        } else {
            album
                .known_artists
                .iter()
                .find(|(known, _)| *known == normalized)
                .map(|(_, id)| id.clone())
        };
        if let Some(local) = local
            && !links.contains(&(local.clone(), mbid.to_string()))
        {
            links.push((local, mbid.to_string()));
        }
    };
    for (mbid, name) in &release.artists {
        add(name, mbid);
    }
    for credit in release
        .credits
        .iter()
        .chain(release.tracks.iter().flat_map(|track| &track.credits))
    {
        if let Some(mbid) = &credit.artist_mbid {
            add(&credit.name, mbid);
        }
    }
    links
}

/// Builds worker input for an album from the catalog cache.
fn album_input(cache: &crate::library::state::LibraryCache, album_id: &str) -> Option<AlbumInput> {
    let album = cache.album(album_id)?;
    let owner = cache.album_owner(album_id)?;
    let mut known_artists = Vec::new();
    for song in &album.songs {
        for credit in &song.credits {
            let normalized = normalize_artist_identity(&credit.name);
            if !known_artists.iter().any(|(known, _)| *known == normalized) {
                known_artists.push((normalized, credit.artist_id.clone()));
            }
        }
        for (name, id) in song
            .contributing_artists
            .iter()
            .zip(&song.contributing_artist_ids)
        {
            let normalized = normalize_artist_identity(name);
            if !known_artists.iter().any(|(known, _)| *known == normalized) {
                known_artists.push((normalized, id.clone()));
            }
        }
    }
    Some(AlbumInput {
        id: album.id.clone(),
        title: album.name.clone(),
        artist_id: owner.id.clone(),
        artist_name: owner.name.clone(),
        musicbrainz_id: Some(album.musicbrainz_id.clone()).filter(|id| !id.is_empty()),
        tracks: album
            .songs
            .iter()
            .map(|song| TrackInput {
                id: song.id.clone(),
                title: song.name.clone(),
                track_number: song.track_number,
            })
            .collect(),
        known_artists,
        year: album
            .first_release_date
            .get(..4)
            .filter(|year| year.bytes().all(|byte| byte.is_ascii_digit()))
            .map(str::to_string),
        release_trusted: false,
    })
}

/// Runs forever, sleeping while enrichment is off or the library is not
/// ready. Spawned once by the server; each stage gets its own queue loop so
/// a busy host only pauses its own work.
pub async fn run(pool: DbPool, library: Arc<LibraryLifecycle>) {
    let enricher = match HttpFetch::new() {
        Ok(fetch) => Arc::new(Enricher {
            fetch,
            pool: pool.clone(),
            images: images_directory(),
        }),
        Err(error) => {
            tracing::error!(%error, "metadata enrichment client could not start");
            return;
        }
    };
    let mut stages = tokio::task::JoinSet::new();
    for stage in Stage::ALL {
        stages.spawn(run_stage(
            stage,
            pool.clone(),
            library.clone(),
            enricher.clone(),
        ));
    }
    while stages.join_next().await.is_some() {}
}

async fn run_stage(
    stage: Stage,
    pool: DbPool,
    library: Arc<LibraryLifecycle>,
    enricher: Arc<Enricher<HttpFetch>>,
) {
    let signal = wake().stage(stage);
    let mut seeded = false;
    loop {
        let settings = db(&pool, store::load_settings).await.unwrap_or_default();
        if !settings.enabled {
            seeded = false;
            let _ = tokio::time::timeout(IDLE_POLL, signal.notified()).await;
            continue;
        }
        let cache = if stage == Stage::Musicbrainz {
            match library.cache().await {
                Ok(cache) => Some(cache),
                Err(_) => {
                    let _ = tokio::time::timeout(IDLE_POLL, signal.notified()).await;
                    continue;
                }
            }
        } else {
            None
        };
        if stage == Stage::Musicbrainz && !seeded {
            match db(&pool, store::seed_tasks).await {
                Ok(queued) if queued > 0 => tracing::info!(queued, "queued metadata enrichment"),
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "could not queue metadata enrichment"),
            }
            seeded = true;
        }
        match enricher
            .step(stage, &settings, cache.as_deref(), backoffs())
            .await
        {
            Ok(Step::Paused(pause)) => {
                tracing::info!(
                    host = stage.host(),
                    seconds = pause.as_secs(),
                    "metadata service is busy; pausing its queue"
                );
                tokio::time::sleep(pause).await;
            }
            Ok(Step::Idle) => {
                let _ = tokio::time::timeout(IDLE_POLL, signal.notified()).await;
            }
            Ok(Step::Ran(task, outcome)) => {
                match &outcome {
                    Outcome::Retry(error) => tracing::debug!(
                        stage = stage.provider(),
                        entity = %task.entity_id,
                        %error,
                        "metadata enrichment will retry"
                    ),
                    // A finished stage may have queued the next one.
                    Outcome::Done => wake().notify_all(),
                    Outcome::Skipped(_) => {}
                }
            }
            Err(error) => {
                tracing::warn!(stage = stage.provider(), %error, "metadata enrichment step failed");
                tokio::time::sleep(IDLE_POLL).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fetch::mock::MockFetch;
    use super::*;
    use crate::persistence::connection::migrated_memory_pool;

    const RELEASE: &str = "0aa4b5f5-2fd0-4b3c-8bd5-1c1b6b6f6a1e";
    const GROUP: &str = "8e8a594e-2b2d-4c3c-9f1d-6c5b0a1d2e3f";
    const MILES: &str = "561d854a-6a28-4aa7-8c99-323e6ce46c2a";
    const EVANS: &str = "c4b9e4a2-4f6f-4b39-9b8c-7a2e0f0a1b2c";

    fn album() -> AlbumInput {
        AlbumInput {
            id: "album-1".into(),
            title: "Kind of Blue".into(),
            artist_id: "artist-miles".into(),
            artist_name: "Miles Davis".into(),
            musicbrainz_id: None,
            tracks: vec![
                TrackInput {
                    id: "t1".into(),
                    title: "So What".into(),
                    track_number: 1,
                },
                TrackInput {
                    id: "t3".into(),
                    title: "Blue in Green".into(),
                    track_number: 3,
                },
            ],
            known_artists: vec![("bill evans".into(), "artist-evans".into())],
            year: None,
            release_trusted: false,
        }
    }

    fn mock() -> MockFetch {
        MockFetch::default()
            .json(
                &mb::release_search_url("Kind of Blue", "Miles Davis"),
                serde_json::json!({"releases": [{"id": RELEASE, "title": "Kind of Blue", "score": 100,
                    "track-count": 2, "artist-credit": [{"name": "Miles Davis"}]}]}),
            )
            .json(
                &mb::release_lookup_url(RELEASE),
                serde_json::json!({
                    "id": RELEASE, "title": "Kind of Blue", "date": "1959-08-17",
                    "label-info": [{"catalog-number": "CL 1355", "label": {"name": "Columbia"}}],
                    "release-group": {"id": GROUP, "primary-type": "Album"},
                    "artist-credit": [{"name": "Miles Davis", "artist": {"id": MILES, "name": "Miles Davis"}}],
                    "relations": [{"type": "producer", "target-type": "artist", "attributes": [],
                                   "artist": {"id": EVANS, "name": "Irving Townsend"}}],
                    "media": [{"position": 1, "tracks": [
                        {"position": 1, "title": "So What", "recording": {"id": RELEASE, "relations": []}},
                        {"position": 3, "title": "Blue in Green", "recording": {"id": RELEASE, "relations": [
                            {"type": "instrument", "target-type": "artist", "attributes": ["piano"],
                             "artist": {"id": EVANS, "name": "Bill Evans"}}
                        ]}}
                    ]}]
                }),
            )
            .json(
                &mb::release_group_lookup_url(GROUP),
                serde_json::json!({"relations": [{"type": "wikidata",
                    "url": {"resource": "https://www.wikidata.org/wiki/Q731473"}}]}),
            )
            .json(
                &wiki::wikidata_entity_url("Q731473"),
                serde_json::json!({"entities": {"Q731473": {"sitelinks": {"enwiki": {"title": "Kind of Blue"}}, "claims": {}}}}),
            )
            .json(
                &wiki::wikipedia_extract_url("en", "Kind of Blue"),
                serde_json::json!({"query": {"pages": [{"title": "Kind of Blue",
                    "fullurl": "https://en.wikipedia.org/wiki/Kind_of_Blue",
                    "extract": "Kind of Blue is a 1959 album.\n\n== Reception ==\nWidely acclaimed."}]}}),
            )
    }

    #[tokio::test]
    async fn album_enrichment_stores_background_reception_credits_and_links() {
        let pool = migrated_memory_pool();
        let enricher = Enricher {
            fetch: mock(),
            pool: pool.clone(),
            images: std::env::temp_dir(),
        };
        let outcome = enricher
            .enrich_album(
                &album(),
                &EnrichmentSettings {
                    enabled: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(outcome, Outcome::Done);
        let requests = enricher.fetch.requested();
        assert!(requests.iter().all(|url| url.starts_with("https://")));
        assert_eq!(requests.len(), 5);

        let mut connection = pool.get().unwrap();
        let texts = store::load_texts(&mut connection, "album", "album-1").unwrap();
        let kinds = texts
            .iter()
            .map(|text| text.kind.as_str())
            .collect::<Vec<_>>();
        assert_eq!(kinds, ["background", "reception"]);
        assert_eq!(texts[1].body, "Widely acclaimed.");
        assert_eq!(texts[0].license, "CC BY-SA 4.0");
        let credits = store::album_credits(&mut connection, "album-1").unwrap();
        let summary = credits
            .iter()
            .map(|credit| {
                (
                    credit.track_id.as_str(),
                    credit.name.as_str(),
                    credit.role.as_str(),
                    credit.instrument.as_str(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            summary,
            [
                ("", "Irving Townsend", "producer", ""),
                ("t3", "Bill Evans", "performer", "piano")
            ]
        );
        let profile = store::load_profile(&mut connection, "album", "album-1")
            .unwrap()
            .unwrap();
        assert_eq!(profile.wikidata_id.as_deref(), Some("Q731473"));
        assert!(profile.profile_json.contains("CL 1355"));
        assert_eq!(
            store::linked_artist_mbids(&mut connection, "artist-evans").unwrap(),
            [EVANS.to_string()]
        );
        assert_eq!(
            store::linked_artist_mbids(&mut connection, "artist-miles").unwrap(),
            [MILES.to_string()]
        );
    }

    #[tokio::test]
    async fn rate_limits_are_retried_and_unmatched_albums_are_skipped() {
        let pool = migrated_memory_pool();
        let settings = EnrichmentSettings {
            enabled: true,
            ..Default::default()
        };
        let unknown = AlbumInput {
            title: "Unknown Album".into(),
            ..album()
        };
        let enricher = Enricher {
            fetch: mock(),
            pool: pool.clone(),
            images: std::env::temp_dir(),
        };
        // The mock answers 404 for the unknown search, which is not retried
        // as "no match" but as a failed search.
        assert!(matches!(
            enricher.enrich_album(&unknown, &settings).await.unwrap(),
            Outcome::Retry(_)
        ));
        let empty = MockFetch::default().json(
            &mb::release_search_url("Unknown Album", "Miles Davis"),
            serde_json::json!({"releases": []}),
        );
        let enricher = Enricher {
            fetch: empty,
            pool: pool.clone(),
            images: std::env::temp_dir(),
        };
        assert!(matches!(
            enricher.enrich_album(&unknown, &settings).await.unwrap(),
            Outcome::Skipped(_)
        ));
        let limited = MockFetch::default().status(&mb::release_lookup_url(RELEASE), 503);
        let enricher = Enricher {
            fetch: limited,
            pool,
            images: std::env::temp_dir(),
        };
        let tagged = AlbumInput {
            musicbrainz_id: Some(RELEASE.into()),
            ..album()
        };
        assert_eq!(
            enricher.enrich_album(&tagged, &settings).await.unwrap(),
            Outcome::Retry("HTTP 503".into())
        );
        // A tagged release id skips the search request.
        assert_eq!(
            enricher.fetch.requested(),
            [mb::release_lookup_url(RELEASE)]
        );
    }

    #[tokio::test]
    async fn artist_enrichment_stores_biography_facts_and_a_licensed_image() {
        let pool = migrated_memory_pool();
        let images =
            std::env::temp_dir().join(format!("parson-editorial-{}", uuid::Uuid::new_v4()));
        let jpeg = [0xff, 0xd8, 0xff, 0xe0, 0, 0x10];
        let fetch = MockFetch::default()
            .json(
                &mb::artist_search_url("Miles Davis"),
                serde_json::json!({"artists": [{"id": MILES, "name": "Miles Davis", "score": 100}]}),
            )
            .json(
                &mb::artist_lookup_url(MILES),
                serde_json::json!({"id": MILES, "name": "Miles Davis", "type": "Person",
                    "area": {"name": "United States"}, "life-span": {"begin": "1926-05-26"},
                    "relations": [{"type": "wikidata", "url": {"resource": "https://www.wikidata.org/wiki/Q93341"}}]}),
            )
            .json(
                &wiki::wikidata_entity_url("Q93341"),
                serde_json::json!({"entities": {"Q93341": {"sitelinks": {"enwiki": {"title": "Miles Davis"}},
                    "claims": {"P18": [{"mainsnak": {"datavalue": {"value": "Miles Davis.jpg"}}}],
                               "P570": [{"mainsnak": {"datavalue": {"value": {"time": "+1991-09-28T00:00:00Z", "precision": 11}}}}]}}}}),
            )
            .json(
                &wiki::wikipedia_extract_url("en", "Miles Davis"),
                serde_json::json!({"query": {"pages": [{"title": "Miles Davis",
                    "fullurl": "https://en.wikipedia.org/wiki/Miles_Davis",
                    "extract": "Miles Dewey Davis III was an American trumpeter."}]}}),
            )
            .json(
                &wiki::commons_image_info_url("Miles Davis.jpg"),
                serde_json::json!({"query": {"pages": [{"imageinfo": [{
                    "thumburl": "https://upload.wikimedia.org/thumb/miles.jpg",
                    "descriptionurl": "https://commons.wikimedia.org/wiki/File:Miles_Davis.jpg",
                    "mime": "image/jpeg",
                    "extmetadata": {"LicenseShortName": {"value": "CC BY-SA 2.0"}, "Artist": {"value": "Tom Palumbo"}}
                }]}]}}),
            )
            .bytes("https://upload.wikimedia.org/thumb/miles.jpg", "image/jpeg", &jpeg);
        let enricher = Enricher {
            fetch,
            pool: pool.clone(),
            images: images.clone(),
        };
        let artist = ArtistInput {
            id: "artist-miles".into(),
            name: "Miles Davis".into(),
        };
        let outcome = enricher
            .enrich_artist(
                &artist,
                &EnrichmentSettings {
                    enabled: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(outcome, Outcome::Done);
        let mut connection = pool.get().unwrap();
        let texts = store::load_texts(&mut connection, "artist", "artist-miles").unwrap();
        assert_eq!(texts[0].kind, "biography");
        assert_eq!(texts[0].url, "https://en.wikipedia.org/wiki/Miles_Davis");
        let profile = store::load_profile(&mut connection, "artist", "artist-miles")
            .unwrap()
            .unwrap();
        let facts: Value = serde_json::from_str(&profile.profile_json).unwrap();
        assert_eq!(facts["kind"], "Person");
        assert_eq!(facts["begin"], "1926-05-26");
        assert_eq!(facts["end"], "1991-09-28");
        let image = store::load_image(&mut connection, "artist", "artist-miles")
            .unwrap()
            .unwrap();
        assert_eq!(image.author, "Tom Palumbo");
        assert_eq!(std::fs::read(images.join(&image.file_name)).unwrap(), jpeg);
        std::fs::remove_dir_all(images).unwrap();
    }

    #[tokio::test]
    async fn busy_wikipedia_keeps_musicbrainz_results_and_retries_the_text() {
        let pool = migrated_memory_pool();
        let fetch = mock().status(&wiki::wikipedia_extract_url("en", "Kind of Blue"), 429);
        let enricher = Enricher {
            fetch,
            pool: pool.clone(),
            images: std::env::temp_dir(),
        };
        let outcome = enricher
            .enrich_album(
                &album(),
                &EnrichmentSettings {
                    enabled: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(outcome, Outcome::Retry("HTTP 429".into()));
        let mut connection = pool.get().unwrap();
        assert_eq!(
            store::album_credits(&mut connection, "album-1")
                .unwrap()
                .len(),
            2
        );
        assert!(
            store::load_texts(&mut connection, "album", "album-1")
                .unwrap()
                .is_empty()
        );
    }

    const KARAJAN_RELEASE: &str = "77777777-7777-4777-8777-777777777777";
    const SYMPHONY: &str = "22222222-2222-4222-8222-222222222222";
    const MOVEMENT_ONE: &str = "33333333-3333-4333-8333-333333333333";
    const MOVEMENT_FOUR: &str = "44444444-4444-4444-8444-444444444444";
    const BEETHOVEN: &str = "1f9df192-a621-4f54-8850-2c5373b7eac9";

    fn karajan_album() -> AlbumInput {
        AlbumInput {
            id: "karajan-9".into(),
            title: "Beethoven: Symphony No. 9".into(),
            artist_id: "artist-karajan".into(),
            artist_name: "Herbert von Karajan".into(),
            musicbrainz_id: None,
            tracks: vec![
                TrackInput {
                    id: "k1".into(),
                    title: "Symphony No. 9 in D minor, Op. 125: I. Allegro ma non troppo".into(),
                    track_number: 1,
                },
                TrackInput {
                    id: "k4".into(),
                    title: "Symphony No. 9 in D minor, Op. 125: IV. Presto".into(),
                    track_number: 2,
                },
            ],
            known_artists: Vec::new(),
            year: None,
            release_trusted: false,
        }
    }

    fn movement(id: &str, title: &str, position: i64) -> Value {
        serde_json::json!({"type": "performance", "target-type": "work", "work": {
            "id": id, "title": title,
            "relations": [
                {"type": "composer", "target-type": "artist", "begin": "1822", "end": "1824",
                 "artist": {"id": BEETHOVEN, "name": "Ludwig van Beethoven"}},
                {"type": "parts", "target-type": "work", "direction": "backward", "ordering-key": position,
                 "work": {"id": SYMPHONY, "title": "Symphony no. 9 in D minor, op. 125", "type": "Symphony"}}
            ]
        }})
    }

    fn karajan_release() -> Value {
        serde_json::json!({
            "id": KARAJAN_RELEASE, "title": "Symphonie Nr. 9",
            "release-group": {"primary-type": "Album"},
            "artist-credit": [{"name": "Beethoven", "artist": {"id": BEETHOVEN, "name": "Ludwig van Beethoven"}}],
            "media": [{"position": 1, "tracks": [
                {"position": 1, "title": "Symphonie Nr. 9: I. Allegro ma non troppo",
                 "recording": {"id": MOVEMENT_ONE, "relations": [
                     movement(MOVEMENT_ONE, "Symphony no. 9 in D minor, op. 125: I. Allegro ma non troppo, un poco maestoso", 1)
                 ]}},
                {"position": 2, "title": "Symphonie Nr. 9: IV. Presto",
                 "recording": {"id": MOVEMENT_FOUR, "relations": [
                     movement(MOVEMENT_FOUR, "Symphony no. 9 in D minor, op. 125: IV. Presto – Allegro assai", 4),
                     {"type": "conductor", "target-type": "artist", "attributes": [],
                      "artist": {"id": KARAJAN_RELEASE, "name": "Herbert von Karajan"}}
                 ]}}
            ]}]
        })
    }

    #[tokio::test]
    async fn an_identified_album_uses_the_chosen_release_and_stores_its_works() {
        let pool = migrated_memory_pool();
        {
            let mut connection = pool.get().unwrap();
            store::set_identification(
                &mut connection,
                "karajan-9",
                Some((KARAJAN_RELEASE, "Symphonie Nr. 9", "Beethoven")),
                None,
            )
            .unwrap();
        }
        let enricher = Enricher {
            fetch: MockFetch::default()
                .json(&mb::release_lookup_url(KARAJAN_RELEASE), karajan_release()),
            pool: pool.clone(),
            images: std::env::temp_dir(),
        };
        let outcome = enricher
            .enrich_album(
                &karajan_album(),
                &EnrichmentSettings {
                    enabled: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(outcome, Outcome::Done);
        // No search: the administrator's choice is looked up directly.
        assert_eq!(
            enricher.fetch.requested(),
            [mb::release_lookup_url(KARAJAN_RELEASE)]
        );
        let mut connection = pool.get().unwrap();
        let symphony = store::load_work(&mut connection, SYMPHONY)
            .unwrap()
            .unwrap();
        assert_eq!(symphony.title, "Symphony No. 9 in D minor, Op. 125");
        assert_eq!(symphony.catalogue.as_deref(), Some("Op. 125"));
        assert_eq!(symphony.musical_key.as_deref(), Some("D minor"));
        assert_eq!(symphony.form.as_deref(), Some("Symphony"));
        assert_eq!(symphony.composed.as_deref(), Some("1822–1824"));
        assert_eq!(symphony.composer_mbid.as_deref(), Some(BEETHOVEN));
        let parts = store::work_parts(&mut connection, SYMPHONY).unwrap();
        assert_eq!(
            parts
                .iter()
                .map(|part| (part.mbid.as_str(), part.part_position))
                .collect::<Vec<_>>(),
            [(MOVEMENT_ONE, Some(1)), (MOVEMENT_FOUR, Some(4))]
        );
        let links = store::work_tracks(&mut connection, SYMPHONY).unwrap();
        assert_eq!(
            links
                .iter()
                .map(|link| (link.track_id.as_str(), link.work_mbid.as_str()))
                .collect::<Vec<_>>(),
            [("k1", MOVEMENT_ONE), ("k4", MOVEMENT_FOUR)]
        );
        let profile = store::load_profile(&mut connection, "album", "karajan-9")
            .unwrap()
            .unwrap();
        assert!(profile.profile_json.contains("\"exact\":true"));
    }

    #[test]
    fn movements_match_by_their_shortened_titles_and_trusted_positions() {
        let local = |id: &str, title: &str, number: u16| TrackInput {
            id: id.into(),
            title: title.into(),
            track_number: number,
        };
        let album = AlbumInput {
            tracks: vec![
                local(
                    "k1",
                    "Symphony No. 9 in D minor, Op. 125: I. Allegro ma non troppo",
                    1,
                ),
                local(
                    "k2",
                    "Symphony No. 9 in D minor, Op. 125: II. Molto vivace",
                    2,
                ),
                local("k3", "Adagio", 3),
                local(
                    "k4",
                    "Symphony No. 9 in D minor, Op. 125: IV. Presto – Allegro assai",
                    4,
                ),
            ],
            ..karajan_album()
        };
        let release_track = |position: i64, title: &str| mb::ReleaseTrack {
            disc: 1,
            position,
            title: title.into(),
            recording_mbid: None,
            credits: Vec::new(),
            works: Vec::new(),
        };
        let release = mb::ReleaseDetails {
            tracks: vec![
                release_track(
                    1,
                    "Symphony No. 9 in D minor, Op. 125: I. Allegro ma non troppo, un poco maestoso",
                ),
                release_track(2, "Symphony No. 9 in D minor, Op. 125: II. Molto vivace"),
                release_track(
                    3,
                    "Symphony No. 9 in D minor, Op. 125: III. Adagio molto e cantabile",
                ),
                release_track(4, "Symphony No. 9 in D minor, Op. 125: IV. Presto"),
                release_track(
                    5,
                    "Symphony No. 9 in D minor, Op. 125: IV. Presto - \"O Freunde\" - Allegro assai",
                ),
            ],
            ..mb::ReleaseDetails::default()
        };
        let summary = |album: &AlbumInput| {
            match_tracks(album, &release)
                .into_iter()
                .map(|(index, id)| (release.tracks[index].position, id))
                .collect::<Vec<_>>()
        };
        // Track counts differ, so an automatic match never guesses by
        // position; "Adagio" is too short to match on its own.
        assert_eq!(
            summary(&album),
            [
                (1, "k1".to_string()),
                (2, "k2".to_string()),
                (4, "k4".to_string())
            ]
        );
        // Even for a chosen release, positions only count when the track
        // lists have the same length: a 4-track album is not tracks 1-4 of
        // a 12-track compilation.
        let chosen = AlbumInput {
            release_trusted: true,
            ..album.clone()
        };
        assert_eq!(summary(&chosen), summary(&album));
        // Roman and Arabic movement numbers match, and "Attacca" is ignored.
        let sonatas = AlbumInput {
            tracks: vec![
                local(
                    "p1",
                    "Piano Sonata No. 8, Op. 13: I. Grave – Allegro di molto e con brio",
                    1,
                ),
                local(
                    "p2",
                    "Piano Sonata No. 14, Op. 27 No. 2: I. Adagio sostenuto",
                    2,
                ),
            ],
            ..karajan_album()
        };
        let release = mb::ReleaseDetails {
            tracks: vec![
                release_track(
                    1,
                    "Sonate no. 8 c-moll, op. 13: 1. Grave - Allegro di molto e con brio",
                ),
                release_track(2, "Sonate no. 8 c-moll, op. 13: 2. Adagio cantabile"),
                release_track(
                    4,
                    "Sonate no. 14 cis-moll, op. 27 no. 2: 1. Adagio sostenuto - Attacca",
                ),
            ],
            ..mb::ReleaseDetails::default()
        };
        assert_eq!(
            match_tracks(&sonatas, &release),
            [(0, "p1".to_string()), (2, "p2".to_string())]
        );
        assert_eq!(roman_number("iv"), Some(4));
        assert_eq!(roman_number("xiv"), Some(14));
        assert_eq!(roman_number("live"), None);
    }

    #[tokio::test]
    async fn placeholder_artists_are_never_matched_automatically() {
        let pool = migrated_memory_pool();
        {
            let mut connection = pool.get().unwrap();
            store::save_profile(
                &mut connection,
                "artist",
                "unknown",
                &ProfileRow {
                    musicbrainz_id: Some(MILES.into()),
                    profile_json: "{\"disambiguation\":\"drum and bass\"}".into(),
                    ..ProfileRow::default()
                },
            )
            .unwrap();
        }
        let enricher = Enricher {
            fetch: MockFetch::default(),
            pool: pool.clone(),
            images: std::env::temp_dir(),
        };
        let settings = EnrichmentSettings {
            enabled: true,
            ..Default::default()
        };
        for name in ["Unknown Artist", "Various Artists", "[unknown]", "  VA "] {
            let outcome = enricher
                .enrich_artist(
                    &ArtistInput {
                        id: "unknown".into(),
                        name: name.into(),
                    },
                    &settings,
                )
                .await
                .unwrap();
            assert!(matches!(outcome, Outcome::Skipped(_)), "{name}");
        }
        let album = AlbumInput {
            title: "Basement Tapes".into(),
            artist_name: "Unknown Artist".into(),
            ..album()
        };
        assert!(matches!(
            enricher.enrich_album(&album, &settings).await.unwrap(),
            Outcome::Skipped(_)
        ));
        assert!(enricher.fetch.requested().is_empty());
        let mut connection = pool.get().unwrap();
        // The wrong match an earlier version stored is gone.
        assert_eq!(
            store::load_profile(&mut connection, "artist", "unknown").unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn ambiguous_artists_get_no_biography() {
        let pool = migrated_memory_pool();
        let fetch = MockFetch::default().json(
            &mb::artist_search_url("Nirvana"),
            serde_json::json!({"artists": [
                {"id": MILES, "name": "Nirvana", "score": 100},
                {"id": EVANS, "name": "Nirvana", "score": 100}
            ]}),
        );
        let enricher = Enricher {
            fetch,
            pool: pool.clone(),
            images: std::env::temp_dir(),
        };
        let artist = ArtistInput {
            id: "nirvana".into(),
            name: "Nirvana".into(),
        };
        let outcome = enricher
            .enrich_artist(
                &artist,
                &EnrichmentSettings {
                    enabled: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(matches!(outcome, Outcome::Skipped(_)));
        assert_eq!(enricher.fetch.requested().len(), 1);
    }

    const KARAJAN: &str = "d2ced2f1-6b58-47cf-ae87-5943e2ab6d99";
    const KLEIBER: &str = "a6a3e7e4-4b0a-4a59-8b8a-8a0c8b3f6c11";

    /// Three conductors and composers whose photos live on Commons.
    fn classical_people() -> [(&'static str, &'static str, &'static str, &'static str); 3] {
        [
            ("beethoven", "Ludwig van Beethoven", BEETHOVEN, "Q255"),
            ("karajan", "Herbert von Karajan", KARAJAN, "Q78491"),
            ("kleiber", "Carlos Kleiber", KLEIBER, "Q57640"),
        ]
    }

    fn classical_mock() -> MockFetch {
        let mut fetch = MockFetch::default();
        for (_, name, mbid, item) in classical_people() {
            let file = format!("{name}.jpg");
            fetch = fetch
                .json(
                    &mb::artist_search_url(name),
                    serde_json::json!({"artists": [{"id": mbid, "name": name, "score": 100}]}),
                )
                .json(
                    &mb::artist_lookup_url(mbid),
                    serde_json::json!({"id": mbid, "name": name, "type": "Person",
                        "relations": [{"type": "wikidata",
                            "url": {"resource": format!("https://www.wikidata.org/wiki/{item}")}}]}),
                )
                .json(
                    &wiki::wikidata_entity_url(item),
                    serde_json::json!({"entities": {item: {"sitelinks": {"enwiki": {"title": name}},
                        "claims": {"P18": [{"mainsnak": {"datavalue": {"value": file}}}]}}}}),
                )
                .json(
                    &wiki::wikipedia_extract_url("en", name),
                    serde_json::json!({"query": {"pages": [{"title": name,
                        "fullurl": format!("https://en.wikipedia.org/wiki/{}", name.replace(' ', "_")),
                        "extract": format!("{name} was a musician.")}]}}),
                )
                // Commons is rate limiting this client.
                .status(&wiki::commons_image_info_url(&file), 429);
        }
        fetch
    }

    /// Runs every stage's queue in turn, as the worker's loops do, until
    /// no queue has anything it may run.
    async fn drain(
        enricher: &Enricher<MockFetch>,
        settings: &EnrichmentSettings,
        backoffs: &Backoffs,
    ) -> Vec<(Stage, String, Outcome)> {
        let mut ran = Vec::new();
        loop {
            let mut progressed = false;
            for stage in Stage::ALL {
                if let Step::Ran(task, outcome) = enricher
                    .step(stage, settings, None, backoffs)
                    .await
                    .unwrap()
                {
                    ran.push((stage, task.entity_id, outcome));
                    progressed = true;
                }
            }
            if !progressed {
                return ran;
            }
        }
    }

    #[tokio::test]
    async fn a_busy_commons_does_not_hold_back_biographies() {
        let pool = migrated_memory_pool();
        {
            let mut connection = pool.get().unwrap();
            for (id, name, _, _) in classical_people() {
                diesel::RunQueryDsl::execute(
                    diesel::sql_query(
                        "INSERT INTO artist_entity (id, name, normalized_name) VALUES (?, ?, ?)",
                    )
                    .bind::<diesel::sql_types::Text, _>(id)
                    .bind::<diesel::sql_types::Text, _>(name)
                    .bind::<diesel::sql_types::Text, _>(name.to_lowercase()),
                    &mut connection,
                )
                .unwrap();
            }
            assert_eq!(store::seed_tasks(&mut connection).unwrap(), 3);
        }
        let enricher = Enricher {
            fetch: classical_mock(),
            pool: pool.clone(),
            images: std::env::temp_dir(),
        };
        let settings = EnrichmentSettings {
            enabled: true,
            ..Default::default()
        };
        let backoffs = Backoffs::default();
        let ran = drain(&enricher, &settings, &backoffs).await;

        // Beethoven's photo request was refused first...
        assert_eq!(
            ran.iter()
                .filter(|(stage, _, _)| *stage == Stage::Commons)
                .collect::<Vec<_>>(),
            [&(
                Stage::Commons,
                "beethoven".to_string(),
                Outcome::Retry("HTTP 429".into())
            )]
        );
        let now = Instant::now();
        assert!(!backoffs.paused_for(Stage::Commons, now).is_zero());
        for stage in [Stage::Musicbrainz, Stage::Wikidata, Stage::Wikipedia] {
            assert!(backoffs.paused_for(stage, now).is_zero(), "{stage:?}");
        }
        // ...yet every biography arrived, and Commons was asked only once.
        let mut connection = pool.get().unwrap();
        for (id, name, _, _) in classical_people() {
            let texts = store::load_texts(&mut connection, "artist", id).unwrap();
            assert_eq!(texts.len(), 1, "{name}");
            assert_eq!(texts[0].kind, "biography");
            assert_eq!(texts[0].body, format!("{name} was a musician."));
            assert_eq!(texts[0].license, "CC BY-SA 4.0");
            let state = store::task_state(&mut connection, "artist", id)
                .unwrap()
                .unwrap();
            assert_eq!(
                (state.status.as_str(), state.stage.as_str()),
                ("pending", "commons")
            );
        }
        let requests = enricher.fetch.requested();
        assert_eq!(
            requests
                .iter()
                .filter(|url| url.contains("commons.wikimedia.org"))
                .count(),
            1
        );
        assert_eq!(
            requests
                .iter()
                .filter(|url| url.contains("musicbrainz.org"))
                .count(),
            6
        );
        assert_eq!(
            store::stage_counts(&mut connection, Stage::Commons)
                .unwrap()
                .pending,
            3
        );
        // The photo lookup is internal and never shown as a fact.
        let profile = store::load_profile(&mut connection, "artist", "karajan")
            .unwrap()
            .unwrap();
        assert_eq!(
            wikidata_links(&profile).image.as_deref(),
            Some("Herbert von Karajan.jpg")
        );
    }

    #[test]
    fn each_host_backs_off_on_its_own() {
        let backoffs = Backoffs::default();
        let now = Instant::now();
        let busy = Outcome::Retry("HTTP 429".into());
        assert_eq!(backoffs.record(Stage::Commons, &busy, now), MIN_THROTTLE);
        assert_eq!(backoffs.paused_for(Stage::Commons, now), MIN_THROTTLE);
        assert!(backoffs.paused_for(Stage::Wikipedia, now).is_zero());
        assert!(backoffs.paused_for(Stage::Musicbrainz, now).is_zero());
        // A second refusal doubles only Commons' pause.
        assert_eq!(
            backoffs.record(Stage::Commons, &busy, now),
            MIN_THROTTLE * 2
        );
        assert_eq!(
            backoffs.record(Stage::Musicbrainz, &Outcome::Done, now),
            Duration::ZERO
        );
        // The pause runs out, and a success resets it.
        assert!(
            backoffs
                .paused_for(Stage::Commons, now + MIN_THROTTLE * 2)
                .is_zero()
        );
        backoffs.record(Stage::Commons, &Outcome::Done, now);
        assert!(backoffs.paused_for(Stage::Commons, now).is_zero());
    }

    #[test]
    fn overload_answers_pause_the_worker_with_growing_backoff() {
        let busy = Outcome::Retry("HTTP 503".into());
        let first = next_throttle(Duration::ZERO, &busy);
        assert_eq!(first, MIN_THROTTLE);
        assert_eq!(next_throttle(first, &busy), MIN_THROTTLE * 2);
        assert_eq!(next_throttle(MAX_THROTTLE, &busy), MAX_THROTTLE);
        assert_eq!(next_throttle(first, &Outcome::Done), Duration::ZERO);
        assert_eq!(
            next_throttle(first, &Outcome::Retry("timeout".into())),
            first
        );
    }

    #[test]
    fn image_names_cannot_escape_the_directory() {
        assert_eq!(
            image_file_name("artist", "abc_-1", "image/png").as_deref(),
            Some("artist-abc_-1.png")
        );
        assert_eq!(image_file_name("artist", "../x", "image/png"), None);
        assert_eq!(image_file_name("artist", "abc", "image/gif"), None);
        assert!(sniff_image(b"\x89PNG\r\n\x1a\nrest", "image/png"));
        assert!(!sniff_image(b"<html>", "image/jpeg"));
    }
}
