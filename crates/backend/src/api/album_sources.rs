//! Where an album's facts come from (its file tags or the matched online
//! release), the values a person froze, and edited credits lists.
//!
//! Lookups never write these tables, so a choice or an edit survives "Look
//! up online metadata again" and Identify album.

use std::collections::{HashMap, HashSet};

use actix_web::{HttpRequest, HttpResponse, delete, get, put, web};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::auth::authenticated_user_id;
use crate::api::error::{bad_request, internal_server_error, not_found};
use crate::domain::Album;
use crate::enrichment::preferences::{
    self as prefs, AlbumField, AlbumPreferences, EditedCredit, FactSource, FieldSources,
    ResolvedFact,
};
use crate::enrichment::store::{self, Stage};
use crate::enrichment::worker::wake;
use crate::library::credits::{ReleaseTags, normalize_tag_date};
use crate::library::state::{LibraryCache, LibraryLifecycle, library_unavailable_response};
use crate::persistence::connection::DbPool;

const MAX_FACT_CHARS: usize = 200;

/// What the source choices are made from, read in one go.
#[derive(Debug, Clone, Default)]
pub struct SourceInputs {
    pub library: FieldSources,
    pub album: AlbumPreferences,
    pub release_tags: Vec<ReleaseTags>,
    /// People credited per field: in the tags, and on MusicBrainz.
    pub tag_credits: HashMap<AlbumField, usize>,
    pub online_credits: HashMap<AlbumField, usize>,
    /// The album's files or folder carry artwork.
    pub own_art: bool,
    /// A cover a person uploaded.
    pub uploaded_cover: bool,
    /// The Cover Art Archive front cover stored for the album.
    pub fetched_cover: bool,
    pub credit_edit: Option<Vec<EditedCredit>>,
    /// How the online release was chosen: `tagged`, `identified` or
    /// `automatic`.
    pub release_method: Option<String>,
}

/// The album's facts with their sources, for the album page and the
/// "Metadata sources" editor.
#[derive(Debug, Clone, Serialize)]
pub struct AlbumSources {
    pub facts: Vec<ResolvedFact>,
    /// The year the hero shows: the original release, not this edition.
    pub hero_year: Option<String>,
    /// Whether a person edited the credits list (it is then frozen).
    pub credits_edited: bool,
    pub edited_credits: Option<Vec<EditedCredit>>,
    /// Whether a verified online release backs the "Online" values.
    pub online_release: bool,
    /// Edition facts (date, label, catalog number, country, barcode) are
    /// only offered from a release known to be this exact edition.
    pub online_edition: bool,
    /// The release was found automatically, so its pressing is a guess:
    /// its edition facts (edition date, catalog number, country, barcode)
    /// never fill in for missing tags; choosing Online for a field, or
    /// identifying the exact release, uses them.
    pub edition_guessed: bool,
    pub library: FieldSources,
}

/// Facts that describe one pressing rather than the music.
fn is_edition_field(field: AlbumField) -> bool {
    matches!(
        field,
        AlbumField::ReleaseDate
            | AlbumField::CatalogNumber
            | AlbumField::Country
            | AlbumField::Barcode
    )
}

fn text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn people(count: usize, place: &str) -> Option<String> {
    match count {
        0 => None,
        1 => Some(format!("1 person {place}")),
        count => Some(format!("{count} people {place}")),
    }
}

/// Resolves every field. `online` is the matched release's facts, or
/// `None` when no trusted release backs the album.
pub fn album_sources(album: &Album, online: Option<&Value>, inputs: SourceInputs) -> AlbumSources {
    let exact = online
        .and_then(|facts| facts.get("exact"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // Edition facts from another pressing would be wrong for this one.
    let edition = |key: &str| {
        if exact {
            text(online.and_then(|facts| facts.get(key)))
        } else {
            None
        }
    };
    let labels = online
        .filter(|_| exact)
        .and_then(|facts| facts.get("labels"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let online_label = labels
        .iter()
        .filter_map(|label| text(label.get("name")))
        .find(|name| name != "[no label]");
    let online_catalog = {
        let mut numbers = Vec::new();
        for number in labels
            .iter()
            .filter_map(|label| text(label.get("catalog_number")))
            .filter(|number| !number.eq_ignore_ascii_case("[none]"))
        {
            if !numbers.contains(&number) {
                numbers.push(number);
            }
        }
        Some(numbers.join(", ")).filter(|numbers| !numbers.is_empty())
    };
    let tagged = |pick: fn(&ReleaseTags) -> Option<&str>| {
        prefs::majority(inputs.release_tags.iter().map(pick))
    };

    let mut preferences = inputs.album.clone();
    let credits_edited = inputs.credit_edit.is_some();
    if let Some(edited) = &inputs.credit_edit {
        let count = |field: AlbumField| {
            edited
                .iter()
                .filter(|credit| prefs::credit_field(&credit.role) == field)
                .map(|credit| crate::library::identity::normalize_artist_identity(&credit.name))
                .collect::<HashSet<_>>()
                .len()
        };
        for field in [AlbumField::Performers, AlbumField::Credits] {
            preferences.edits.insert(
                field,
                people(count(field), "in your edited list")
                    .unwrap_or_else(|| "Nobody (edited)".into()),
            );
        }
    }
    if inputs.uploaded_cover {
        preferences
            .edits
            .insert(AlbumField::Cover, "Artwork you uploaded".into());
    }

    let resolve = |field: AlbumField, tags: Option<String>, online: Option<String>| {
        prefs::resolve(field, &inputs.library, &preferences, tags, online)
    };
    let facts = vec![
        resolve(
            AlbumField::OriginalDate,
            tagged(|tags| tags.original_date.as_deref()),
            // The release group's first release holds for every edition.
            text(online.and_then(|facts| facts.get("first_release_date"))),
        ),
        resolve(
            AlbumField::ReleaseDate,
            normalize_tag_date(&album.first_release_date),
            edition("date"),
        ),
        resolve(
            AlbumField::Label,
            crate::api::album::album_label(&album.songs),
            online_label,
        ),
        resolve(
            AlbumField::CatalogNumber,
            tagged(|tags| tags.catalog_number.as_deref()),
            online_catalog,
        ),
        resolve(
            AlbumField::Country,
            tagged(|tags| tags.country.as_deref()),
            edition("country"),
        ),
        resolve(
            AlbumField::Barcode,
            tagged(|tags| tags.barcode.as_deref()),
            edition("barcode"),
        ),
        resolve(
            AlbumField::Cover,
            inputs
                .own_art
                .then(|| "Artwork in the album's files or folder".to_string()),
            inputs
                .fetched_cover
                .then(|| "Front cover from the Cover Art Archive".to_string()),
        ),
        resolve(
            AlbumField::Performers,
            people(
                inputs
                    .tag_credits
                    .get(&AlbumField::Performers)
                    .copied()
                    .unwrap_or(0),
                "in the tags",
            ),
            people(
                inputs
                    .online_credits
                    .get(&AlbumField::Performers)
                    .copied()
                    .unwrap_or(0),
                "on MusicBrainz",
            ),
        ),
        resolve(
            AlbumField::Credits,
            people(
                inputs
                    .tag_credits
                    .get(&AlbumField::Credits)
                    .copied()
                    .unwrap_or(0),
                "in the tags",
            ),
            people(
                inputs
                    .online_credits
                    .get(&AlbumField::Credits)
                    .copied()
                    .unwrap_or(0),
                "on MusicBrainz",
            ),
        ),
    ];
    let edition_guessed = online.is_some()
        && inputs
            .release_method
            .as_deref()
            .is_none_or(|method| method == "automatic");
    let mut facts = facts;
    if edition_guessed {
        // Another pressing's catalog number or barcode is worse than none.
        for fact in &mut facts {
            if is_edition_field(fact.field) && fact.fallback && fact.source == Some("online") {
                fact.value = None;
                fact.source = None;
                fact.fallback = false;
            }
        }
    }
    let year = |field: AlbumField| {
        facts
            .iter()
            .find(|fact| fact.field == field)
            .and_then(|fact| fact.value.as_deref())
            .and_then(|value| value.get(..4))
            .filter(|year| year.chars().all(|character| character.is_ascii_digit()))
            .map(str::to_string)
    };
    let hero_year = year(AlbumField::OriginalDate).or_else(|| year(AlbumField::ReleaseDate));
    AlbumSources {
        hero_year,
        credits_edited,
        edited_credits: inputs.credit_edit,
        online_release: online.is_some(),
        online_edition: exact,
        edition_guessed,
        library: inputs.library,
        facts,
    }
}

/// Reads everything [`album_sources`] needs for one album.
pub fn load_inputs(
    connection: &mut diesel::SqliteConnection,
    album_id: &str,
    trusted: bool,
) -> diesel::QueryResult<SourceInputs> {
    use diesel::prelude::*;
    #[derive(QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        count: i64,
    }
    let own_art = diesel::sql_query(
        "SELECT COUNT(*) AS count FROM track_entity track
         JOIN track_file tf ON tf.track_id = track.id AND tf.is_primary = true
         JOIN raw_file_metadata rfm ON rfm.file_id = tf.file_id
         WHERE track.album_id = ? AND rfm.cover_url <> ''",
    )
    .bind::<diesel::sql_types::Text, _>(album_id)
    .load::<CountRow>(connection)?
    .first()
    .is_some_and(|row| row.count > 0);
    let (tag_credits, online_credits) = prefs::credit_counts(connection, album_id)?;
    Ok(SourceInputs {
        library: prefs::load_library_sources(connection)?,
        album: prefs::load_album_preferences(connection, album_id)?,
        release_tags: prefs::album_release_tags(connection, album_id)?,
        tag_credits,
        online_credits: if trusted {
            online_credits
        } else {
            HashMap::new()
        },
        own_art,
        uploaded_cover: prefs::load_edited_covers(connection)?.contains(album_id),
        fetched_cover: store::load_cover_art(connection, album_id)?.is_some(),
        credit_edit: prefs::load_credit_edit(connection, album_id)?,
        release_method: store::load_release_match(connection, album_id)?.map(|found| found.method),
    })
}

/// Checks a typed value for a field and puts it in its stored form.
pub fn clean_fact(field: AlbumField, value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("Type a value, or revert the field to its sources.".into());
    }
    if value.chars().count() > MAX_FACT_CHARS {
        return Err(format!(
            "A value can have at most {MAX_FACT_CHARS} characters."
        ));
    }
    match field {
        AlbumField::OriginalDate | AlbumField::ReleaseDate => normalize_tag_date(value)
            .ok_or_else(|| "Type a date as YYYY, YYYY-MM or YYYY-MM-DD.".to_string()),
        AlbumField::Country => {
            let code = value.to_ascii_uppercase();
            if code.len() == 2
                && code
                    .chars()
                    .all(|character| character.is_ascii_alphabetic())
            {
                Ok(code)
            } else {
                Err("Type a two-letter country code such as US, GB or XE (Europe).".into())
            }
        }
        AlbumField::Barcode => {
            if (8..=14).contains(&value.len())
                && value.chars().all(|character| character.is_ascii_digit())
            {
                Ok(value.to_string())
            } else {
                Err("A barcode has 8 to 14 digits.".into())
            }
        }
        AlbumField::Label | AlbumField::CatalogNumber => Ok(value.to_string()),
        AlbumField::Cover | AlbumField::Performers | AlbumField::Credits => {
            Err("Upload a cover or edit the credits list instead.".into())
        }
    }
}

/// Reloads the catalog so covers and credits show their new source.
pub(crate) async fn reload_library(lifecycle: &LibraryLifecycle) -> Result<(), String> {
    crate::library::storage::refresh_cache().await;
    let fresh = LibraryCache::new()
        .await
        .map_err(|error| error.to_string())?;
    lifecycle.set_ready_and_persist(fresh).await;
    Ok(())
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

#[derive(Debug, Deserialize)]
pub struct SourcesPatch {
    /// Field to `tags`, `online`, or `null` for the library default.
    #[serde(default)]
    pub sources: HashMap<String, Option<FactSource>>,
    /// Field to a typed value (frozen), or `null` to unfreeze it.
    #[serde(default)]
    pub edits: HashMap<String, Option<String>>,
}

type SourceWrites = (
    Vec<(AlbumField, Option<FactSource>)>,
    Vec<(AlbumField, Option<String>)>,
);

/// Checks a patch: known fields, typed values only for text fields.
pub fn clean_patch(patch: SourcesPatch) -> Result<SourceWrites, String> {
    let mut sources = Vec::new();
    for (name, source) in patch.sources {
        let field = AlbumField::parse(&name).ok_or_else(|| format!("Unknown field {name}."))?;
        sources.push((field, source));
    }
    let mut edits = Vec::new();
    for (name, value) in patch.edits {
        let field = AlbumField::parse(&name).ok_or_else(|| format!("Unknown field {name}."))?;
        if !field.is_text() {
            return Err("Upload a cover or edit the credits list instead.".into());
        }
        let value = value.map(|value| clean_fact(field, &value)).transpose()?;
        edits.push((field, value));
    }
    if sources.is_empty() && edits.is_empty() {
        return Err("Nothing to change.".into());
    }
    Ok((sources, edits))
}

/// Administrator: chooses sources for an album's fields and freezes or
/// unfreezes typed values.
#[put("/albums/{id}/sources")]
pub async fn put_album_sources(
    request: HttpRequest,
    id: web::Path<String>,
    body: web::Json<SourcesPatch>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let album_id = id.into_inner();
    let (sources, edits) = match clean_patch(body.into_inner()) {
        Ok(patch) => patch,
        Err(message) => return bad_request(message, "invalid_sources"),
    };
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    if cache.album(&album_id).is_none() {
        return not_found("Album not found.", "album_not_found");
    }
    let user_id = authenticated_user_id(&request).ok();
    let cover_changed = sources.iter().any(|(field, _)| *field == AlbumField::Cover);
    let lookup = album_id.clone();
    let saved = blocking(pool.get_ref(), move |connection| {
        use diesel::Connection;
        connection.transaction(|connection| {
            for (field, source) in &sources {
                prefs::save_field_source(connection, &lookup, *field, *source, user_id)?;
            }
            for (field, value) in &edits {
                prefs::save_field_edit(connection, &lookup, *field, value.as_deref(), user_id)?;
            }
            if cover_changed {
                // Fetch the online cover if the album now prefers it.
                store::queue_stage(connection, Stage::CoverArt, "album", &lookup)?;
            }
            Ok(())
        })
    })
    .await;
    if let Err(error) = saved {
        tracing::error!(%error, "could not save album sources");
        return internal_server_error("Could not save the sources.", "sources_failed");
    }
    if cover_changed {
        wake().notify_all();
        if let Err(error) = reload_library(&lifecycle).await {
            tracing::warn!(%error, "could not reload the library after a cover change");
        }
    }
    HttpResponse::Ok().json(serde_json::json!({ "album_id": album_id }))
}

#[derive(Debug, Deserialize)]
pub struct CreditsEdit {
    pub credits: Vec<EditedCredit>,
}

async fn save_credits(
    request: HttpRequest,
    album_id: String,
    credits: Option<Vec<EditedCredit>>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let cache = match lifecycle.cache().await {
        Ok(cache) => cache,
        Err(readiness) => return library_unavailable_response(readiness),
    };
    let Some(album) = cache.album(&album_id) else {
        return not_found("Album not found.", "album_not_found");
    };
    let track_ids = album
        .songs
        .iter()
        .map(|song| song.id.clone())
        .collect::<Vec<_>>();
    let credits = match credits
        .map(|credits| prefs::clean_credit_edit(credits, &track_ids))
        .transpose()
    {
        Ok(credits) => credits,
        Err(message) => return bad_request(message, "invalid_credits"),
    };
    let user_id = authenticated_user_id(&request).ok();
    let lookup = album_id.clone();
    let saved = blocking(pool.get_ref(), move |connection| {
        prefs::save_credit_edit(connection, &lookup, credits.as_deref(), user_id)?;
        prefs::load_credit_edit(connection, &lookup)
    })
    .await;
    match saved {
        Ok(credits) => {
            if let Err(error) = reload_library(&lifecycle).await {
                tracing::error!(%error, "could not reload the library after a credits edit");
                return internal_server_error(
                    "The credits were saved but the library could not reload.",
                    "cache_refresh_failed",
                );
            }
            HttpResponse::Ok().json(serde_json::json!({
                "album_id": album_id,
                "edited": credits.is_some(),
                "credits": credits.unwrap_or_default(),
            }))
        }
        Err(error) => {
            tracing::error!(%error, "could not save album credits");
            internal_server_error("Could not save the credits.", "credits_failed")
        }
    }
}

/// Administrator: replaces an album's credits with an edited list. The
/// list is frozen: lookups no longer change the album's credits.
#[put("/albums/{id}/credits")]
pub async fn put_album_credits(
    request: HttpRequest,
    id: web::Path<String>,
    body: web::Json<CreditsEdit>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    save_credits(
        request,
        id.into_inner(),
        Some(body.into_inner().credits),
        pool,
        lifecycle,
    )
    .await
}

/// Administrator: goes back to tag and online credits.
#[delete("/albums/{id}/credits")]
pub async fn delete_album_credits(
    request: HttpRequest,
    id: web::Path<String>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    save_credits(request, id.into_inner(), None, pool, lifecycle).await
}

/// Administrator: the library-wide default source per field.
#[get("/sources")]
pub async fn get_library_sources(pool: web::Data<DbPool>) -> HttpResponse {
    match blocking(pool.get_ref(), prefs::load_library_sources).await {
        Ok(sources) => HttpResponse::Ok().json(sources),
        Err(error) => {
            tracing::error!(%error, "could not read metadata sources");
            internal_server_error("Could not read the sources.", "sources_failed")
        }
    }
}

/// Administrator: changes library-wide defaults. Unnamed fields keep
/// theirs.
#[put("/sources")]
pub async fn put_library_sources(
    body: web::Json<HashMap<String, FactSource>>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let mut changes = Vec::new();
    for (name, source) in body.into_inner() {
        match AlbumField::parse(&name) {
            Some(field) => changes.push((field, source)),
            None => return bad_request(format!("Unknown field {name}."), "invalid_sources"),
        }
    }
    let cover_changed = changes.iter().any(|(field, _)| *field == AlbumField::Cover);
    let saved = blocking(pool.get_ref(), move |connection| {
        let mut sources = prefs::load_library_sources(connection)?;
        let previous_cover = sources.cover;
        for (field, source) in changes {
            sources.set(field, source);
        }
        prefs::save_library_sources(connection, &sources)?;
        if previous_cover != sources.cover && sources.cover == FactSource::Online {
            // Albums with their own artwork skipped the cover stage; ask
            // the Cover Art Archive for them now.
            diesel::RunQueryDsl::execute(
                diesel::sql_query(
                    "UPDATE metadata_task SET status = 'pending', attempts = 0,
                        last_error = NULL, not_before = NULL
                     WHERE provider = 'coverart' AND status = 'skipped'",
                ),
                connection,
            )?;
        }
        Ok(sources)
    })
    .await;
    match saved {
        Ok(sources) => {
            if cover_changed {
                wake().notify_all();
                if let Err(error) = reload_library(&lifecycle).await {
                    tracing::warn!(%error, "could not reload the library after a cover change");
                }
            }
            HttpResponse::Ok().json(sources)
        }
        Err(error) => {
            tracing::error!(%error, "could not save metadata sources");
            internal_server_error("Could not save the sources.", "sources_failed")
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::domain::{Credit, Song};

    fn album() -> Album {
        let song = |id: &str, label: Option<&str>| Song {
            id: id.into(),
            name: id.into(),
            label: label.map(str::to_string),
            credits: vec![Credit {
                artist_id: "evans".into(),
                name: "Bill Evans".into(),
                role: "performer".into(),
                instrument: Some("piano".into()),
            }],
            ..Default::default()
        };
        Album {
            id: "kob".into(),
            name: "Kind of Blue".into(),
            first_release_date: "1997".into(),
            songs: vec![song("t1", Some("Columbia")), song("t2", Some("Columbia"))],
            ..Default::default()
        }
    }

    /// A CBS Europe reissue matched by its release id.
    fn reissue() -> Value {
        json!({
            "exact": true,
            "date": "1987",
            "first_release_date": "1959-08-17",
            "country": "XE",
            "barcode": "5099703109",
            "labels": [{"name": "CBS", "catalog_number": "CBS 32109"}],
        })
    }

    fn inputs() -> SourceInputs {
        SourceInputs {
            release_tags: vec![
                ReleaseTags {
                    catalog_number: Some("CK 64935".into()),
                    country: Some("US".into()),
                    ..Default::default()
                },
                ReleaseTags {
                    catalog_number: Some("CK 64935".into()),
                    ..Default::default()
                },
            ],
            tag_credits: HashMap::from([(AlbumField::Performers, 1)]),
            online_credits: HashMap::from([(AlbumField::Performers, 6), (AlbumField::Credits, 2)]),
            own_art: true,
            release_method: Some("tagged".into()),
            ..Default::default()
        }
    }

    fn fact(sources: &AlbumSources, field: AlbumField) -> &ResolvedFact {
        sources
            .facts
            .iter()
            .find(|fact| fact.field == field)
            .unwrap()
    }

    #[test]
    fn tagged_edition_facts_win_over_a_matched_reissue_by_default() {
        let online = reissue();
        let sources = album_sources(&album(), Some(&online), inputs());
        assert_eq!(
            fact(&sources, AlbumField::Label).value.as_deref(),
            Some("Columbia")
        );
        assert_eq!(
            fact(&sources, AlbumField::CatalogNumber).value.as_deref(),
            Some("CK 64935")
        );
        assert_eq!(
            fact(&sources, AlbumField::Country).value.as_deref(),
            Some("US")
        );
        assert_eq!(
            fact(&sources, AlbumField::ReleaseDate).value.as_deref(),
            Some("1997")
        );
        // Nothing tagged for the barcode: the release fills in, marked.
        let barcode = fact(&sources, AlbumField::Barcode);
        assert_eq!(barcode.source, Some("online"));
        assert!(barcode.fallback);
        // No original date tagged: the first release of the release group.
        let original = fact(&sources, AlbumField::OriginalDate);
        assert_eq!(original.value.as_deref(), Some("1959-08-17"));
        // The hero shows 1959, not the 1997 edition.
        assert_eq!(sources.hero_year.as_deref(), Some("1959"));
        assert!(sources.online_release && sources.online_edition);
        // Credits merge MusicBrainz in by default.
        let performers = fact(&sources, AlbumField::Performers);
        assert_eq!(performers.source, Some("online"));
        assert_eq!(performers.tags.as_deref(), Some("1 person in the tags"));
        assert_eq!(
            fact(&sources, AlbumField::Cover).value.as_deref(),
            Some("Artwork in the album's files or folder")
        );
    }

    #[test]
    fn choosing_online_and_typing_values_change_what_shows() {
        let online = reissue();
        let mut inputs = inputs();
        inputs
            .album
            .sources
            .insert(AlbumField::Label, FactSource::Online);
        inputs
            .album
            .edits
            .insert(AlbumField::OriginalDate, "1959-08-17".into());
        inputs.release_tags[0].original_date = Some("1958".into());
        inputs.credit_edit = Some(vec![EditedCredit {
            name: "Bill Evans".into(),
            role: "performer".into(),
            instrument: Some("piano".into()),
            track_ids: Vec::new(),
            artist_id: None,
        }]);
        let sources = album_sources(&album(), Some(&online), inputs);
        let label = fact(&sources, AlbumField::Label);
        assert_eq!(label.value.as_deref(), Some("CBS"));
        assert_eq!(label.choice, Some(FactSource::Online));
        let original = fact(&sources, AlbumField::OriginalDate);
        assert_eq!(original.source, Some("edited"));
        assert_eq!(original.tags.as_deref(), Some("1958"));
        assert!(sources.credits_edited);
        let performers = fact(&sources, AlbumField::Performers);
        assert_eq!(performers.source, Some("edited"));
        assert_eq!(
            performers.value.as_deref(),
            Some("1 person in your edited list")
        );
    }

    #[test]
    fn a_release_from_another_edition_offers_no_edition_facts() {
        let mut online = reissue();
        online["exact"] = json!(false);
        let mut inputs = inputs();
        inputs.release_tags.clear();
        let sources = album_sources(&album(), Some(&online), inputs);
        assert_eq!(fact(&sources, AlbumField::CatalogNumber).value, None);
        assert_eq!(fact(&sources, AlbumField::Country).value, None);
        // The first release of the music is the same for every edition.
        assert_eq!(sources.hero_year.as_deref(), Some("1959"));
        assert!(!sources.online_edition);

        // Without a trusted release only the tags remain.
        let sources = album_sources(&album(), None, SourceInputs::default());
        assert_eq!(sources.hero_year.as_deref(), Some("1997"));
        assert!(!sources.online_release);
    }

    #[test]
    fn a_guessed_pressing_never_fills_in_edition_facts() {
        // Blue Train matched automatically to a Japanese TOCJ pressing.
        let online = json!({
            "exact": true,
            "first_release_date": "1958-02",
            "country": "JP",
            "barcode": "4988006722200",
            "labels": [{"name": "Blue Note", "catalog_number": "TOCJ-1577"}],
        });
        let inputs = SourceInputs {
            release_method: Some("automatic".into()),
            ..Default::default()
        };
        let sources = album_sources(&album(), Some(&online), inputs.clone());
        assert!(sources.edition_guessed);
        for field in [
            AlbumField::Country,
            AlbumField::Barcode,
            AlbumField::CatalogNumber,
        ] {
            let fact = fact(&sources, field);
            assert_eq!(fact.value, None, "{field:?}");
            // Still offered as a candidate in the editor.
            assert!(fact.online.is_some());
        }
        // The music's first release holds for every pressing.
        assert_eq!(sources.hero_year.as_deref(), Some("1958"));
        // Choosing Online for a field uses it.
        let mut chosen = inputs;
        chosen
            .album
            .sources
            .insert(AlbumField::Country, FactSource::Online);
        let sources = album_sources(&album(), Some(&online), chosen);
        assert_eq!(
            fact(&sources, AlbumField::Country).value.as_deref(),
            Some("JP")
        );
    }

    #[test]
    fn typed_values_are_checked_per_field() {
        assert_eq!(
            clean_fact(AlbumField::ReleaseDate, " 1959-08-17 ").unwrap(),
            "1959-08-17"
        );
        assert!(clean_fact(AlbumField::OriginalDate, "late fifties").is_err());
        assert_eq!(clean_fact(AlbumField::Country, "gb").unwrap(), "GB");
        assert!(clean_fact(AlbumField::Country, "Germany").is_err());
        assert!(clean_fact(AlbumField::Barcode, "12ab").is_err());
        assert!(clean_fact(AlbumField::Label, "  ").is_err());
        assert!(clean_fact(AlbumField::Cover, "x").is_err());

        let patch: SourcesPatch = serde_json::from_value(json!({
            "sources": {"label": "online", "cover": null},
            "edits": {"catalog_number": "CL 1355", "country": null}
        }))
        .unwrap();
        let (sources, edits) = clean_patch(patch).unwrap();
        assert_eq!(sources.len(), 2);
        assert!(edits.contains(&(AlbumField::CatalogNumber, Some("CL 1355".into()))));
        assert!(edits.contains(&(AlbumField::Country, None)));
        let bad: SourcesPatch = serde_json::from_value(json!({"edits": {"credits": "x"}})).unwrap();
        assert!(clean_patch(bad).is_err());
        let unknown: SourcesPatch =
            serde_json::from_value(json!({"sources": {"mood": "tags"}})).unwrap();
        assert!(clean_patch(unknown).is_err());
    }
}
