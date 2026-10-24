//! The background enrichment worker. It consumes `metadata_task` rows only
//! while the administrator has turned enrichment on.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use diesel::sqlite::SqliteConnection;
use serde_json::{Value, json};
use tokio::sync::Notify;

use super::fetch::{BoxError, Fetch, FetchResponse, HttpFetch, MAX_IMAGE_BYTES, MAX_JSON_BYTES};
use super::musicbrainz as mb;
use super::store::{
    self, CreditRow, EditorialText, EnrichmentSettings, ImageRow, Outcome, ProfileRow,
};
use super::wikimedia as wiki;
use crate::library::identity::{normalize_artist_identity, normalize_song_identity};
use crate::library::state::LibraryLifecycle;
use crate::persistence::connection::DbPool;
use crate::settings::data_path;

const IDLE_POLL: Duration = Duration::from_secs(60);
const MIN_THROTTLE: Duration = Duration::from_secs(30);
const MAX_THROTTLE: Duration = Duration::from_secs(15 * 60);

/// How long the whole worker pauses after a service said it is overloaded
/// (MusicBrainz answers 503 when a client goes over its rate limit). Doubles
/// on each consecutive refusal and resets after a success.
pub fn next_throttle(current: Duration, outcome: &Outcome) -> Duration {
    match outcome {
        Outcome::Retry(error) if error == "HTTP 429" || error == "HTTP 503" => {
            (current * 2).clamp(MIN_THROTTLE, MAX_THROTTLE)
        }
        Outcome::Done | Outcome::Skipped(_) => Duration::ZERO,
        Outcome::Retry(_) => current,
    }
}

/// Wakes the worker when settings change or an entity is refreshed.
pub fn wake() -> &'static Notify {
    static WAKE: std::sync::OnceLock<Notify> = std::sync::OnceLock::new();
    WAKE.get_or_init(Notify::new)
}

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

    /// Wikidata item -> Wikipedia article in the preferred language.
    async fn wikipedia(
        &self,
        wikidata_id: &str,
        language: &str,
    ) -> Result<
        Result<
            (
                Option<wiki::WikidataEntity>,
                Option<(String, wiki::WikipediaArticle)>,
            ),
            Outcome,
        >,
        BoxError,
    > {
        let entity = match self
            .get_json(&wiki::wikidata_entity_url(wikidata_id))
            .await?
        {
            Ok(Some(body)) => wiki::parse_wikidata_entity(&body),
            Ok(None) => None,
            Err(outcome) => return Ok(Err(outcome)),
        };
        let Some((article_language, title)) =
            entity.as_ref().and_then(|entity| entity.article(language))
        else {
            return Ok(Ok((entity, None)));
        };
        let article = match self
            .get_json(&wiki::wikipedia_extract_url(&article_language, &title))
            .await?
        {
            Ok(Some(body)) => wiki::parse_wikipedia_extract(&body),
            Ok(None) => None,
            Err(outcome) => return Ok(Err(outcome)),
        };
        Ok(Ok((
            entity,
            article.map(|article| (article_language, article)),
        )))
    }

    pub async fn enrich_album(
        &self,
        album: &AlbumInput,
        settings: &EnrichmentSettings,
    ) -> Result<Outcome, BoxError> {
        let release_mbid = match album.musicbrainz_id.as_deref().filter(|id| mb::is_mbid(id)) {
            Some(mbid) => mbid.to_string(),
            None => {
                if album.artist_name.trim().is_empty() || album.title.trim().is_empty() {
                    return Ok(Outcome::Skipped("the album has no title or artist".into()));
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
                match mb::choose_release(
                    &album.title,
                    &album.artist_name,
                    album.tracks.len(),
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
        // Wikimedia may be busy while MusicBrainz answered; keep what
        // MusicBrainz gave and retry the text later.
        let mut texts = Some(Vec::new());
        let mut deferred = None;
        if let Some(wikidata_id) = &wikidata_id {
            match self.wikipedia(wikidata_id, &settings.language).await? {
                Ok((_, Some((language, article)))) => {
                    let texts = texts.get_or_insert_with(Vec::new);
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
                Ok((_, None)) => {}
                Err(outcome) => {
                    texts = None;
                    deferred = Some(outcome);
                }
            }
        }

        let exact = album.musicbrainz_id.as_deref() == Some(release.mbid.as_str())
            || release.tracks.len() == album.tracks.len();
        let credits = album_credit_rows(album, &release);
        let links = artist_links(album, &release);
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
            store::save_profile(connection, "album", &album_id, &profile)?;
            if let Some(texts) = &texts {
                store::replace_texts(connection, "album", &album_id, texts)?;
            }
            store::replace_album_credits(connection, &album_id, "musicbrainz", &credits)?;
            for (artist_id, mbid) in &links {
                store::link_artist_mbid(connection, artist_id, mbid)?;
            }
            Ok(())
        })
        .await?;
        Ok(deferred.unwrap_or(Outcome::Done))
    }

    pub async fn enrich_artist(
        &self,
        artist: &ArtistInput,
        settings: &EnrichmentSettings,
    ) -> Result<Outcome, BoxError> {
        if artist.name.trim().is_empty() {
            return Ok(Outcome::Skipped("the artist has no name".into()));
        }
        let wanted = normalize_artist_identity(&artist.name);
        if wanted == "various artists" {
            return Ok(Outcome::Skipped("Various Artists is not a person".into()));
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

        let mut texts = Some(Vec::new());
        let mut entity = None;
        let mut deferred = None;
        if let Some(wikidata_id) = &profile.wikidata_id {
            match self.wikipedia(wikidata_id, &settings.language).await? {
                Ok((found, article)) => {
                    entity = found;
                    if let Some((language, article)) = article {
                        texts.get_or_insert_with(Vec::new).push(wikipedia_text(
                            "biography",
                            &language,
                            &article,
                            &article.lead,
                        ));
                    }
                }
                Err(outcome) => {
                    texts = None;
                    deferred = Some(outcome);
                }
            }
        }

        let image = match (&entity, settings.artist_images) {
            (
                Some(wiki::WikidataEntity {
                    image: Some(file), ..
                }),
                true,
            ) => match self.download_image(&artist.id, file).await? {
                Ok(image) => image,
                Err(outcome) => {
                    deferred.get_or_insert(outcome);
                    None
                }
            },
            _ => None,
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
                "begin": entity.as_ref().and_then(|entity| entity.born.clone().or(entity.formed.clone())).or(profile.begin.clone()),
                "end": entity.as_ref().and_then(|entity| entity.died.clone().or(entity.dissolved.clone())).or(profile.end.clone()),
                "disambiguation": profile.disambiguation,
            })
            .to_string(),
        };
        let artist_id = artist.id.clone();
        db(&self.pool, move |connection| {
            store::save_profile(connection, "artist", &artist_id, &row)?;
            if let Some(texts) = &texts {
                store::replace_texts(connection, "artist", &artist_id, texts)?;
            }
            if let Some(image) = &image {
                store::save_image(connection, "artist", &artist_id, image)?;
            }
            Ok(())
        })
        .await?;
        Ok(deferred.unwrap_or(Outcome::Done))
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

/// Matches MusicBrainz tracks to local tracks by title, then by position
/// when the track lists line up.
pub fn match_tracks(album: &AlbumInput, release: &mb::ReleaseDetails) -> Vec<(usize, String)> {
    let single_disc = release.tracks.iter().all(|track| track.disc == 1);
    let same_length = release.tracks.len() == album.tracks.len();
    let mut matches = Vec::new();
    for (index, track) in release.tracks.iter().enumerate() {
        let title = normalize_song_identity(&track.title);
        let by_title = album
            .tracks
            .iter()
            .filter(|local| normalize_song_identity(&local.title) == title)
            .collect::<Vec<_>>();
        let local = match by_title.as_slice() {
            [only] => Some(*only),
            _ if single_disc && same_length => album
                .tracks
                .iter()
                .find(|local| i64::from(local.track_number) == track.position),
            _ => None,
        };
        if let Some(local) = local {
            matches.push((index, local.id.clone()));
        }
    }
    matches
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
    })
}

/// Runs forever, sleeping while enrichment is off or the library is not
/// ready. Spawned once by the server.
pub async fn run(pool: DbPool, library: Arc<LibraryLifecycle>) {
    let enricher = match HttpFetch::new() {
        Ok(fetch) => Enricher {
            fetch,
            pool: pool.clone(),
            images: images_directory(),
        },
        Err(error) => {
            tracing::error!(%error, "metadata enrichment client could not start");
            return;
        }
    };
    let mut seeded = false;
    let mut throttle = Duration::ZERO;
    loop {
        let settings = db(&pool, store::load_settings).await.unwrap_or_default();
        if !settings.enabled {
            seeded = false;
            let _ = tokio::time::timeout(IDLE_POLL, wake().notified()).await;
            continue;
        }
        let Ok(cache) = library.cache().await else {
            let _ = tokio::time::timeout(IDLE_POLL, wake().notified()).await;
            continue;
        };
        if !seeded {
            match db(&pool, store::seed_tasks).await {
                Ok(queued) if queued > 0 => tracing::info!(queued, "queued metadata enrichment"),
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "could not queue metadata enrichment"),
            }
            seeded = true;
        }
        let task = match db(&pool, store::next_task).await {
            Ok(Some(task)) => task,
            Ok(None) => {
                let _ = tokio::time::timeout(IDLE_POLL, wake().notified()).await;
                continue;
            }
            Err(error) => {
                tracing::warn!(%error, "could not read metadata tasks");
                tokio::time::sleep(IDLE_POLL).await;
                continue;
            }
        };
        let result = match task.entity_type.as_str() {
            "album" => match album_input(&cache, &task.entity_id) {
                Some(input) => enricher.enrich_album(&input, &settings).await,
                None => Ok(Outcome::Skipped(
                    "the album is no longer in the library".into(),
                )),
            },
            _ => {
                let artist_id = task.entity_id.clone();
                match db(&pool, move |connection| {
                    store::artist_name(connection, &artist_id)
                })
                .await
                {
                    Ok(Some(name)) => {
                        let input = ArtistInput {
                            id: task.entity_id.clone(),
                            name,
                        };
                        enricher.enrich_artist(&input, &settings).await
                    }
                    Ok(None) => Ok(Outcome::Skipped(
                        "the artist is no longer in the library".into(),
                    )),
                    Err(error) => Err(error),
                }
            }
        };
        let outcome = result.unwrap_or_else(|error| Outcome::Retry(error.to_string()));
        if let Outcome::Retry(error) = &outcome {
            tracing::debug!(entity = %task.entity_id, %error, "metadata enrichment will retry");
        }
        throttle = next_throttle(throttle, &outcome);
        let finished = task.clone();
        if let Err(error) = db(&pool, move |connection| {
            store::finish_task(connection, &finished, &outcome)
        })
        .await
        {
            tracing::warn!(%error, "could not record a metadata task result");
            tokio::time::sleep(IDLE_POLL).await;
        }
        if !throttle.is_zero() {
            tracing::info!(
                seconds = throttle.as_secs(),
                "metadata services are busy; pausing enrichment"
            );
            let _ = tokio::time::timeout(throttle, wake().notified()).await;
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
