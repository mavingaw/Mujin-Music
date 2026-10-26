//! Where each album fact comes from: the file tags, the matched online
//! release, or a value a person typed.
//!
//! Every field has a library-wide default source and may have its own
//! choice per album. The preferred source falls back to the other one when
//! it has nothing. A typed value (or an edited credits list) is frozen: it
//! wins over both sources and is kept through "Look up online metadata
//! again" and Identify album, because lookups only ever rewrite the online
//! tables, never these.

use std::collections::{HashMap, HashSet};

use diesel::prelude::*;
use diesel::sql_types::{Integer, Nullable, Text};
use diesel::sqlite::SqliteConnection;
use serde::{Deserialize, Serialize};

use crate::domain::{Artist, Credit};
use crate::library::credits::{ARTIST_ROLES, CREDIT_ROLES};
use crate::library::identity::{hash_artist, normalize_artist_identity};

/// The two sources a fact can come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FactSource {
    /// The album's own file tags (and artwork in its folder or files).
    Tags,
    /// The matched MusicBrainz release (and the Cover Art Archive).
    Online,
}

impl FactSource {
    pub fn as_str(self) -> &'static str {
        match self {
            FactSource::Tags => "tags",
            FactSource::Online => "online",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "tags" => Some(FactSource::Tags),
            "online" => Some(FactSource::Online),
            _ => None,
        }
    }

    fn other(self) -> Self {
        match self {
            FactSource::Tags => FactSource::Online,
            FactSource::Online => FactSource::Tags,
        }
    }
}

/// The album facts a person can choose a source for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlbumField {
    /// When this music was first released (the hero year).
    OriginalDate,
    /// The date of this edition.
    ReleaseDate,
    Label,
    CatalogNumber,
    Country,
    Barcode,
    Cover,
    /// Performer, ensemble and conductor credits.
    Performers,
    /// Composer, producer, engineer and the other credits.
    Credits,
}

impl AlbumField {
    pub const ALL: [AlbumField; 9] = [
        AlbumField::OriginalDate,
        AlbumField::ReleaseDate,
        AlbumField::Label,
        AlbumField::CatalogNumber,
        AlbumField::Country,
        AlbumField::Barcode,
        AlbumField::Cover,
        AlbumField::Performers,
        AlbumField::Credits,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AlbumField::OriginalDate => "original_date",
            AlbumField::ReleaseDate => "release_date",
            AlbumField::Label => "label",
            AlbumField::CatalogNumber => "catalog_number",
            AlbumField::Country => "country",
            AlbumField::Barcode => "barcode",
            AlbumField::Cover => "cover",
            AlbumField::Performers => "performers",
            AlbumField::Credits => "credits",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|field| field.as_str() == value)
    }

    /// Fields a person can type a value for. The cover is replaced by
    /// uploading artwork and credits are edited as a list.
    pub fn is_text(self) -> bool {
        !matches!(
            self,
            AlbumField::Cover | AlbumField::Performers | AlbumField::Credits
        )
    }
}

/// Library-wide default source per field. File tags win for edition facts
/// and artwork (a carefully tagged library is right about its own
/// pressing); credits merge in what MusicBrainz knows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldSources {
    #[serde(default = "tags")]
    pub original_date: FactSource,
    #[serde(default = "tags")]
    pub release_date: FactSource,
    #[serde(default = "tags")]
    pub label: FactSource,
    #[serde(default = "tags")]
    pub catalog_number: FactSource,
    #[serde(default = "tags")]
    pub country: FactSource,
    #[serde(default = "tags")]
    pub barcode: FactSource,
    #[serde(default = "tags")]
    pub cover: FactSource,
    #[serde(default = "online")]
    pub performers: FactSource,
    #[serde(default = "online")]
    pub credits: FactSource,
}

fn tags() -> FactSource {
    FactSource::Tags
}

fn online() -> FactSource {
    FactSource::Online
}

impl Default for FieldSources {
    fn default() -> Self {
        FieldSources {
            original_date: tags(),
            release_date: tags(),
            label: tags(),
            catalog_number: tags(),
            country: tags(),
            barcode: tags(),
            cover: tags(),
            performers: online(),
            credits: online(),
        }
    }
}

impl FieldSources {
    pub fn get(&self, field: AlbumField) -> FactSource {
        match field {
            AlbumField::OriginalDate => self.original_date,
            AlbumField::ReleaseDate => self.release_date,
            AlbumField::Label => self.label,
            AlbumField::CatalogNumber => self.catalog_number,
            AlbumField::Country => self.country,
            AlbumField::Barcode => self.barcode,
            AlbumField::Cover => self.cover,
            AlbumField::Performers => self.performers,
            AlbumField::Credits => self.credits,
        }
    }

    pub fn set(&mut self, field: AlbumField, source: FactSource) {
        let slot = match field {
            AlbumField::OriginalDate => &mut self.original_date,
            AlbumField::ReleaseDate => &mut self.release_date,
            AlbumField::Label => &mut self.label,
            AlbumField::CatalogNumber => &mut self.catalog_number,
            AlbumField::Country => &mut self.country,
            AlbumField::Barcode => &mut self.barcode,
            AlbumField::Cover => &mut self.cover,
            AlbumField::Performers => &mut self.performers,
            AlbumField::Credits => &mut self.credits,
        };
        *slot = source;
    }
}

/// One album's own choices and frozen values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlbumPreferences {
    pub sources: HashMap<AlbumField, FactSource>,
    pub edits: HashMap<AlbumField, String>,
}

/// A fact as the album shows it, with both candidates so a person can see
/// what they are choosing between.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedFact {
    pub field: AlbumField,
    /// What the album shows.
    pub value: Option<String>,
    /// `edited`, `tags` or `online`; `None` when no source has a value.
    pub source: Option<&'static str>,
    /// The source this album prefers (its own choice or the default).
    pub preferred: FactSource,
    /// This album's own choice; `None` follows the library default.
    pub choice: Option<FactSource>,
    pub library_default: FactSource,
    pub tags: Option<String>,
    pub online: Option<String>,
    /// The value a person typed. While present the field is frozen.
    pub edited: Option<String>,
    /// The preferred source had nothing, so the other one filled in.
    pub fallback: bool,
}

/// Chooses the value for one field.
pub fn resolve(
    field: AlbumField,
    library: &FieldSources,
    album: &AlbumPreferences,
    tags: Option<String>,
    online: Option<String>,
) -> ResolvedFact {
    let clean = |value: Option<String>| {
        value
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let tags = clean(tags);
    let online = clean(online);
    let edited = clean(album.edits.get(&field).cloned());
    let choice = album.sources.get(&field).copied();
    let library_default = library.get(field);
    let preferred = choice.unwrap_or(library_default);
    let pick = |source: FactSource| match source {
        FactSource::Tags => tags.clone(),
        FactSource::Online => online.clone(),
    };
    let (value, source, fallback) = if let Some(edited) = &edited {
        (Some(edited.clone()), Some("edited"), false)
    } else if let Some(value) = pick(preferred) {
        (Some(value), Some(preferred.as_str()), false)
    } else if let Some(value) = pick(preferred.other()) {
        (Some(value), Some(preferred.other().as_str()), true)
    } else {
        (None, None, false)
    };
    ResolvedFact {
        field,
        value,
        source,
        preferred,
        choice,
        library_default,
        tags,
        online,
        edited,
        fallback,
    }
}

/// Whether a credit role counts as a performer for [`AlbumField::Performers`].
pub fn is_performer_role(role: &str) -> bool {
    matches!(role, "performer" | "ensemble" | "conductor")
}

/// The field governing a credit role.
pub fn credit_field(role: &str) -> AlbumField {
    if is_performer_role(role) {
        AlbumField::Performers
    } else {
        AlbumField::Credits
    }
}

/// A credit in an edited list. An empty `track_ids` means the whole album.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditedCredit {
    pub name: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instrument: Option<String>,
    #[serde(default)]
    pub track_ids: Vec<String>,
    /// The person's id when the list was edited from a linked credit
    /// (a library artist or `mb:<MusicBrainz id>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist_id: Option<String>,
}

pub const MAX_EDITED_CREDITS: usize = 500;
const MAX_CREDIT_TEXT: usize = 200;

/// Checks and tidies an edited credits list: known roles only, trimmed
/// names, lower-case instruments, tracks of this album only, no
/// duplicates. Returns a message for the first problem.
pub fn clean_credit_edit(
    credits: Vec<EditedCredit>,
    album_track_ids: &[String],
) -> Result<Vec<EditedCredit>, String> {
    if credits.len() > MAX_EDITED_CREDITS {
        return Err(format!(
            "An album can have at most {MAX_EDITED_CREDITS} credits."
        ));
    }
    let tracks = album_track_ids.iter().collect::<HashSet<_>>();
    let mut cleaned: Vec<EditedCredit> = Vec::new();
    for credit in credits {
        let name = credit.name.trim().to_string();
        if name.is_empty() || name.chars().count() > MAX_CREDIT_TEXT {
            return Err("Every credit needs a name of at most 200 characters.".into());
        }
        if normalize_artist_identity(&name).is_empty() {
            return Err(format!("\"{name}\" is not a name."));
        }
        let role = credit.role.trim().to_lowercase();
        if !CREDIT_ROLES.contains(&role.as_str()) {
            return Err(format!("\"{role}\" is not a credit role."));
        }
        let instrument = credit
            .instrument
            .map(|instrument| instrument.trim().to_lowercase())
            .filter(|instrument| !instrument.is_empty() && role == "performer");
        if instrument
            .as_ref()
            .is_some_and(|instrument| instrument.chars().count() > MAX_CREDIT_TEXT)
        {
            return Err("An instrument can have at most 200 characters.".into());
        }
        let mut track_ids = Vec::new();
        for id in credit.track_ids {
            if !tracks.contains(&id) {
                return Err("A credit names a track that is not on this album.".into());
            }
            if !track_ids.contains(&id) {
                track_ids.push(id);
            }
        }
        // Every track is the whole album.
        if track_ids.len() == album_track_ids.len() {
            track_ids.clear();
        }
        let artist_id = credit
            .artist_id
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty() && id.len() <= 128);
        let key = (
            normalize_artist_identity(&name),
            role.clone(),
            instrument.clone(),
        );
        match cleaned.iter_mut().find(|existing| {
            (
                normalize_artist_identity(&existing.name),
                existing.role.clone(),
                existing.instrument.clone(),
            ) == key
        }) {
            Some(existing) => {
                if existing.track_ids.is_empty() || track_ids.is_empty() {
                    existing.track_ids.clear();
                } else {
                    for id in track_ids {
                        if !existing.track_ids.contains(&id) {
                            existing.track_ids.push(id);
                        }
                    }
                }
            }
            None => cleaned.push(EditedCredit {
                name,
                role,
                instrument,
                track_ids,
                artist_id,
            }),
        }
    }
    Ok(cleaned)
}

/// The library id an edited credit links to: its own id when it still
/// names this person, a library artist with the same name, or a new id
/// from the name (a person page that lists just these credits).
fn credit_artist_id(credit: &EditedCredit, known: &HashMap<String, String>) -> String {
    if let Some(id) = &credit.artist_id {
        return id.clone();
    }
    known
        .get(&normalize_artist_identity(&credit.name))
        .cloned()
        .unwrap_or_else(|| hash_artist(&credit.name))
}

/// Replaces the tag credits of albums whose credits a person edited, so
/// album pages, person pages, discographies and filters all read the
/// edited list. Artist roles (primary, featured) are left alone.
pub fn apply_credit_edits(library: &mut [Artist], edits: &HashMap<String, Vec<EditedCredit>>) {
    if edits.is_empty() {
        return;
    }
    let mut known: HashMap<String, String> = HashMap::new();
    for artist in library.iter() {
        known
            .entry(normalize_artist_identity(&artist.name))
            .or_insert_with(|| artist.id.clone());
        for credit in artist
            .albums
            .iter()
            .flat_map(|album| &album.songs)
            .flat_map(|song| &song.credits)
        {
            known
                .entry(normalize_artist_identity(&credit.name))
                .or_insert_with(|| credit.artist_id.clone());
        }
    }
    for album in library
        .iter_mut()
        .flat_map(|artist| artist.albums.iter_mut())
    {
        let Some(edited) = edits.get(&album.id) else {
            continue;
        };
        for song in &mut album.songs {
            song.credits
                .retain(|credit| ARTIST_ROLES.contains(&credit.role.as_str()));
            for credit in edited
                .iter()
                .filter(|credit| credit.track_ids.is_empty() || credit.track_ids.contains(&song.id))
            {
                song.credits.push(Credit {
                    artist_id: credit_artist_id(credit, &known),
                    name: credit.name.clone(),
                    role: credit.role.clone(),
                    instrument: credit.instrument.clone(),
                });
            }
            song.credits
                .sort_by_key(|credit| crate::library::credits::role_rank(&credit.role));
        }
    }
}

/// Replaces album covers according to each album's cover source: artwork
/// fetched from the Cover Art Archive is shown instead of the album's own
/// when the album (or the library) prefers online covers, and in place of
/// missing artwork either way.
pub fn apply_cover_sources(
    library: &mut [Artist],
    fetched: &HashMap<String, String>,
    library_default: FactSource,
    album_sources: &HashMap<String, FactSource>,
    edited_covers: &HashSet<String>,
) {
    if fetched.is_empty() {
        return;
    }
    for album in library
        .iter_mut()
        .flat_map(|artist| artist.albums.iter_mut())
    {
        let Some(path) = fetched.get(&album.id) else {
            continue;
        };
        let own_art = !album.cover_url.trim().is_empty();
        let prefers_online = album_sources
            .get(&album.id)
            .copied()
            .unwrap_or(library_default)
            == FactSource::Online;
        // A cover a person uploaded is frozen like any other edit.
        if own_art && (!prefers_online || edited_covers.contains(&album.id)) {
            continue;
        }
        if std::path::Path::new(path).is_file() {
            album.cover_url = path.clone();
        }
    }
}

// ---------------------------------------------------------------- storage

pub const SETTINGS_KEY: &str = "metadata_sources";

#[derive(QueryableByName)]
struct FieldRow {
    #[diesel(sql_type = Text)]
    album_id: String,
    #[diesel(sql_type = Text)]
    field: String,
    #[diesel(sql_type = Text)]
    value: String,
}

#[derive(QueryableByName)]
struct ValueRow {
    #[diesel(sql_type = Text)]
    value: String,
}

/// The library-wide default source per field.
pub fn load_library_sources(connection: &mut SqliteConnection) -> QueryResult<FieldSources> {
    let rows = diesel::sql_query("SELECT value_json AS value FROM server_setting WHERE key = ?")
        .bind::<Text, _>(SETTINGS_KEY)
        .load::<ValueRow>(connection)?;
    Ok(rows
        .first()
        .and_then(|row| serde_json::from_str(&row.value).ok())
        .unwrap_or_default())
}

pub fn save_library_sources(
    connection: &mut SqliteConnection,
    sources: &FieldSources,
) -> QueryResult<()> {
    let value = serde_json::to_string(sources).unwrap_or_else(|_| "{}".to_string());
    diesel::sql_query(
        "INSERT INTO server_setting (key, value_json) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = CURRENT_TIMESTAMP",
    )
    .bind::<Text, _>(SETTINGS_KEY)
    .bind::<Text, _>(value)
    .execute(connection)?;
    Ok(())
}

pub fn load_album_preferences(
    connection: &mut SqliteConnection,
    album_id: &str,
) -> QueryResult<AlbumPreferences> {
    let mut preferences = AlbumPreferences::default();
    for row in diesel::sql_query(
        "SELECT album_id, field, source AS value FROM album_field_preference WHERE album_id = ?",
    )
    .bind::<Text, _>(album_id)
    .load::<FieldRow>(connection)?
    {
        if let (Some(field), Some(source)) =
            (AlbumField::parse(&row.field), FactSource::parse(&row.value))
        {
            preferences.sources.insert(field, source);
        }
    }
    for row in diesel::sql_query(
        "SELECT album_id, field, value_json AS value FROM album_field_edit WHERE album_id = ?",
    )
    .bind::<Text, _>(album_id)
    .load::<FieldRow>(connection)?
    {
        if let (Some(field), Ok(value)) = (
            AlbumField::parse(&row.field),
            serde_json::from_str::<String>(&row.value),
        ) {
            preferences.edits.insert(field, value);
        }
    }
    Ok(preferences)
}

/// Sets (or with `None` clears) an album's own source for a field.
pub fn save_field_source(
    connection: &mut SqliteConnection,
    album_id: &str,
    field: AlbumField,
    source: Option<FactSource>,
    user_id: Option<i32>,
) -> QueryResult<()> {
    match source {
        Some(source) => diesel::sql_query(
            "INSERT INTO album_field_preference (album_id, field, source, updated_by)
             VALUES (?, ?, ?, ?)
             ON CONFLICT(album_id, field) DO UPDATE SET source = excluded.source,
                updated_by = excluded.updated_by, updated_at = CURRENT_TIMESTAMP",
        )
        .bind::<Text, _>(album_id)
        .bind::<Text, _>(field.as_str())
        .bind::<Text, _>(source.as_str())
        .bind::<Nullable<Integer>, _>(user_id)
        .execute(connection)?,
        None => {
            diesel::sql_query("DELETE FROM album_field_preference WHERE album_id = ? AND field = ?")
                .bind::<Text, _>(album_id)
                .bind::<Text, _>(field.as_str())
                .execute(connection)?
        }
    };
    Ok(())
}

/// Freezes (or with `None` unfreezes) a typed value for a text field.
pub fn save_field_edit(
    connection: &mut SqliteConnection,
    album_id: &str,
    field: AlbumField,
    value: Option<&str>,
    user_id: Option<i32>,
) -> QueryResult<()> {
    match value {
        Some(value) => diesel::sql_query(
            "INSERT INTO album_field_edit (album_id, field, value_json, edited_by)
             VALUES (?, ?, ?, ?)
             ON CONFLICT(album_id, field) DO UPDATE SET value_json = excluded.value_json,
                edited_by = excluded.edited_by, updated_at = CURRENT_TIMESTAMP",
        )
        .bind::<Text, _>(album_id)
        .bind::<Text, _>(field.as_str())
        .bind::<Text, _>(serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into()))
        .bind::<Nullable<Integer>, _>(user_id)
        .execute(connection)?,
        None => diesel::sql_query("DELETE FROM album_field_edit WHERE album_id = ? AND field = ?")
            .bind::<Text, _>(album_id)
            .bind::<Text, _>(field.as_str())
            .execute(connection)?,
    };
    Ok(())
}

#[derive(QueryableByName)]
struct CreditEditRow {
    #[diesel(sql_type = Text)]
    album_id: String,
    #[diesel(sql_type = Text)]
    credits_json: String,
}

/// Every edited credits list, by album.
pub fn load_credit_edits(
    connection: &mut SqliteConnection,
) -> QueryResult<HashMap<String, Vec<EditedCredit>>> {
    Ok(
        diesel::sql_query("SELECT album_id, credits_json FROM album_credit_edit")
            .load::<CreditEditRow>(connection)?
            .into_iter()
            .filter_map(|row| {
                serde_json::from_str::<Vec<EditedCredit>>(&row.credits_json)
                    .ok()
                    .map(|credits| (row.album_id, credits))
            })
            .collect(),
    )
}

pub fn load_credit_edit(
    connection: &mut SqliteConnection,
    album_id: &str,
) -> QueryResult<Option<Vec<EditedCredit>>> {
    Ok(
        diesel::sql_query(
            "SELECT album_id, credits_json FROM album_credit_edit WHERE album_id = ?",
        )
        .bind::<Text, _>(album_id)
        .load::<CreditEditRow>(connection)?
        .into_iter()
        .next()
        .and_then(|row| serde_json::from_str(&row.credits_json).ok()),
    )
}

/// Stores (or with `None` reverts) an album's edited credits list.
pub fn save_credit_edit(
    connection: &mut SqliteConnection,
    album_id: &str,
    credits: Option<&[EditedCredit]>,
    user_id: Option<i32>,
) -> QueryResult<()> {
    match credits {
        Some(credits) => diesel::sql_query(
            "INSERT INTO album_credit_edit (album_id, credits_json, edited_by) VALUES (?, ?, ?)
             ON CONFLICT(album_id) DO UPDATE SET credits_json = excluded.credits_json,
                edited_by = excluded.edited_by, updated_at = CURRENT_TIMESTAMP",
        )
        .bind::<Text, _>(album_id)
        .bind::<Text, _>(serde_json::to_string(credits).unwrap_or_else(|_| "[]".into()))
        .bind::<Nullable<Integer>, _>(user_id)
        .execute(connection)?,
        None => diesel::sql_query("DELETE FROM album_credit_edit WHERE album_id = ?")
            .bind::<Text, _>(album_id)
            .execute(connection)?,
    };
    Ok(())
}

/// Albums with their own cover source, for the library cache.
pub fn load_cover_sources(
    connection: &mut SqliteConnection,
) -> QueryResult<HashMap<String, FactSource>> {
    Ok(diesel::sql_query(
        "SELECT album_id, field, source AS value FROM album_field_preference WHERE field = 'cover'",
    )
    .load::<FieldRow>(connection)?
    .into_iter()
    .filter_map(|row| FactSource::parse(&row.value).map(|source| (row.album_id, source)))
    .collect())
}

/// Albums whose cover a person uploaded (a frozen cover).
pub fn load_edited_covers(connection: &mut SqliteConnection) -> QueryResult<HashSet<String>> {
    Ok(diesel::sql_query(
        "SELECT entity_id AS value FROM metadata_override
         WHERE entity_type = 'album' AND field_name = 'cover_url'",
    )
    .load::<ValueRow>(connection)?
    .into_iter()
    .map(|row| row.value)
    .collect())
}

/// Albums whose external credits must not be shown: their credits list
/// was edited, or both credit fields prefer tags and the tags have
/// credits of that kind. Returned per album as the fields to drop.
pub fn hidden_external_credits(
    connection: &mut SqliteConnection,
    album_ids: &[String],
) -> QueryResult<HashMap<String, HashSet<AlbumField>>> {
    let mut hidden: HashMap<String, HashSet<AlbumField>> = HashMap::new();
    if album_ids.is_empty() {
        return Ok(hidden);
    }
    let library = load_library_sources(connection)?;
    let wanted = album_ids.iter().collect::<HashSet<_>>();
    for album_id in diesel::sql_query("SELECT album_id AS value FROM album_credit_edit")
        .load::<ValueRow>(connection)?
        .into_iter()
        .map(|row| row.value)
        .filter(|id| wanted.contains(id))
    {
        hidden.insert(
            album_id,
            HashSet::from([AlbumField::Performers, AlbumField::Credits]),
        );
    }
    // Which albums prefer tags for each credit field.
    let mut overrides: HashMap<(String, AlbumField), FactSource> = HashMap::new();
    for row in diesel::sql_query(
        "SELECT album_id, field, source AS value FROM album_field_preference
         WHERE field IN ('performers', 'credits')",
    )
    .load::<FieldRow>(connection)?
    {
        if let (Some(field), Some(source)) =
            (AlbumField::parse(&row.field), FactSource::parse(&row.value))
        {
            overrides.insert((row.album_id, field), source);
        }
    }
    let prefers_tags = |album_id: &str, field: AlbumField| {
        overrides
            .get(&(album_id.to_string(), field))
            .copied()
            .unwrap_or(library.get(field))
            == FactSource::Tags
    };
    let candidates = album_ids
        .iter()
        .filter(|id| !hidden.contains_key(*id))
        .filter(|id| {
            prefers_tags(id, AlbumField::Performers) || prefers_tags(id, AlbumField::Credits)
        })
        .cloned()
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Ok(hidden);
    }
    // Which kinds of credit the tags have, per album. A field that prefers
    // tags still shows online credits when the tags have none (fallback).
    #[derive(QueryableByName)]
    struct RoleRow {
        #[diesel(sql_type = Text)]
        album_id: String,
        #[diesel(sql_type = Text)]
        role: String,
    }
    let mut tagged: HashMap<String, HashSet<AlbumField>> = HashMap::new();
    for chunk in candidates.chunks(500) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let sql = format!(
            "SELECT DISTINCT track.album_id, credit.role FROM track_artist credit
             JOIN track_entity track ON track.id = credit.track_id
             WHERE credit.role NOT IN ('primary', 'featured') AND track.album_id IN ({placeholders})"
        );
        let mut query = diesel::sql_query(sql).into_boxed();
        for id in chunk {
            query = query.bind::<Text, _>(id.clone());
        }
        for row in query.load::<RoleRow>(connection)? {
            let (role, _) = crate::library::credits::split_role_key(&row.role);
            tagged
                .entry(row.album_id)
                .or_default()
                .insert(credit_field(role));
        }
    }
    for album_id in candidates {
        for field in [AlbumField::Performers, AlbumField::Credits] {
            if prefers_tags(&album_id, field)
                && tagged
                    .get(&album_id)
                    .is_some_and(|fields| fields.contains(&field))
            {
                hidden.entry(album_id.clone()).or_default().insert(field);
            }
        }
    }
    Ok(hidden)
}

/// Drops external credit rows the album's sources hide.
pub fn filter_external_credits(
    connection: &mut SqliteConnection,
    rows: Vec<crate::enrichment::store::CreditRow>,
) -> QueryResult<Vec<crate::enrichment::store::CreditRow>> {
    let mut album_ids = rows
        .iter()
        .map(|row| row.album_id.clone())
        .collect::<Vec<_>>();
    album_ids.sort();
    album_ids.dedup();
    let hidden = hidden_external_credits(connection, &album_ids)?;
    if hidden.is_empty() {
        return Ok(rows);
    }
    Ok(rows
        .into_iter()
        .filter(|row| {
            hidden
                .get(&row.album_id)
                .is_none_or(|fields| !fields.contains(&credit_field(&row.role)))
        })
        .collect())
}

/// Release facts tagged on an album's files, one entry per tagged file.
pub fn album_release_tags(
    connection: &mut SqliteConnection,
    album_id: &str,
) -> QueryResult<Vec<crate::library::credits::ReleaseTags>> {
    Ok(diesel::sql_query(
        "SELECT rfm.release_json AS value
         FROM track_entity track
         JOIN track_file tf ON tf.track_id = track.id AND tf.is_primary = true
         JOIN raw_file_metadata rfm ON rfm.file_id = tf.file_id
         WHERE track.album_id = ? AND rfm.release_json IS NOT NULL",
    )
    .bind::<Text, _>(album_id)
    .load::<ValueRow>(connection)?
    .into_iter()
    .filter_map(|row| serde_json::from_str(&row.value).ok())
    .collect())
}

/// The most common value among an album's files (ties go to the first).
pub fn majority<'a>(values: impl Iterator<Item = Option<&'a str>>) -> Option<String> {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for value in values
        .flatten()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        match counts.iter_mut().find(|(known, _)| *known == value) {
            Some((_, count)) => *count += 1,
            None => counts.push((value, 1)),
        }
    }
    let best = counts.iter().map(|(_, count)| *count).max()?;
    counts
        .into_iter()
        .find(|(_, count)| *count == best)
        .map(|(value, _)| value.to_string())
}

/// How many people each source credits on an album, per credit field:
/// the tags (as indexed, before any edit) and MusicBrainz.
pub fn credit_counts(
    connection: &mut SqliteConnection,
    album_id: &str,
) -> QueryResult<(HashMap<AlbumField, usize>, HashMap<AlbumField, usize>)> {
    #[derive(QueryableByName)]
    struct PersonRow {
        #[diesel(sql_type = Text)]
        person: String,
        #[diesel(sql_type = Text)]
        role: String,
    }
    let count = |rows: Vec<PersonRow>| {
        let mut people: HashMap<AlbumField, HashSet<String>> = HashMap::new();
        for row in rows {
            let (role, _) = crate::library::credits::split_role_key(&row.role);
            people
                .entry(credit_field(role))
                .or_default()
                .insert(row.person);
        }
        people
            .into_iter()
            .map(|(field, people)| (field, people.len()))
            .collect::<HashMap<_, _>>()
    };
    let tags = diesel::sql_query(
        "SELECT DISTINCT credit.artist_id AS person, credit.role FROM track_artist credit
         JOIN track_entity track ON track.id = credit.track_id
         WHERE track.album_id = ? AND credit.role NOT IN ('primary', 'featured')",
    )
    .bind::<Text, _>(album_id)
    .load::<PersonRow>(connection)?;
    let online = diesel::sql_query(
        "SELECT DISTINCT COALESCE(source_artist_id, normalized_name) AS person, role
         FROM enriched_credit WHERE album_id = ? AND source = 'musicbrainz'",
    )
    .bind::<Text, _>(album_id)
    .load::<PersonRow>(connection)?;
    Ok((count(tags), count(online)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Album, Song};

    fn preferences(
        sources: &[(AlbumField, FactSource)],
        edits: &[(AlbumField, &str)],
    ) -> AlbumPreferences {
        AlbumPreferences {
            sources: sources.iter().copied().collect(),
            edits: edits
                .iter()
                .map(|(field, value)| (*field, value.to_string()))
                .collect(),
        }
    }

    #[test]
    fn the_preferred_source_wins_and_the_other_fills_in() {
        let library = FieldSources::default();
        let none = AlbumPreferences::default();
        // Kind of Blue tagged "Columbia", matched to a CBS Europe reissue.
        let label = resolve(
            AlbumField::Label,
            &library,
            &none,
            Some("Columbia".into()),
            Some("CBS".into()),
        );
        assert_eq!(label.value.as_deref(), Some("Columbia"));
        assert_eq!(label.source, Some("tags"));
        assert!(!label.fallback);
        assert_eq!(label.choice, None);

        // Nothing tagged: MusicBrainz fills in, and says so.
        let catalog = resolve(
            AlbumField::CatalogNumber,
            &library,
            &none,
            Some("  ".into()),
            Some("32109".into()),
        );
        assert_eq!(catalog.value.as_deref(), Some("32109"));
        assert_eq!(catalog.source, Some("online"));
        assert!(catalog.fallback);

        // The album chose online for its label.
        let online = preferences(&[(AlbumField::Label, FactSource::Online)], &[]);
        let label = resolve(
            AlbumField::Label,
            &library,
            &online,
            Some("Columbia".into()),
            Some("CBS".into()),
        );
        assert_eq!(label.value.as_deref(), Some("CBS"));
        assert_eq!(label.preferred, FactSource::Online);
        assert_eq!(label.choice, Some(FactSource::Online));
        assert_eq!(label.library_default, FactSource::Tags);

        let empty = resolve(AlbumField::Country, &library, &none, None, None);
        assert_eq!(empty.value, None);
        assert_eq!(empty.source, None);
    }

    #[test]
    fn a_typed_value_is_frozen_over_both_sources() {
        let library = FieldSources::default();
        let edited = preferences(
            &[(AlbumField::ReleaseDate, FactSource::Online)],
            &[(AlbumField::ReleaseDate, "1959-08-17")],
        );
        let date = resolve(
            AlbumField::ReleaseDate,
            &library,
            &edited,
            Some("1997".into()),
            Some("1987".into()),
        );
        assert_eq!(date.value.as_deref(), Some("1959-08-17"));
        assert_eq!(date.source, Some("edited"));
        assert_eq!(date.edited.as_deref(), Some("1959-08-17"));
        // Both candidates stay visible for the editor.
        assert_eq!(date.tags.as_deref(), Some("1997"));
        assert_eq!(date.online.as_deref(), Some("1987"));
    }

    #[test]
    fn library_defaults_keep_edition_facts_from_tags_and_merge_credits() {
        let defaults = FieldSources::default();
        assert_eq!(defaults.get(AlbumField::Label), FactSource::Tags);
        assert_eq!(defaults.get(AlbumField::Cover), FactSource::Tags);
        assert_eq!(defaults.get(AlbumField::Credits), FactSource::Online);
        // Settings saved before a field existed read its default.
        let stored: FieldSources = serde_json::from_str(r#"{"label":"online"}"#).unwrap();
        assert_eq!(stored.label, FactSource::Online);
        assert_eq!(stored.performers, FactSource::Online);
        let mut changed = stored.clone();
        changed.set(AlbumField::Performers, FactSource::Tags);
        assert_eq!(changed.get(AlbumField::Performers), FactSource::Tags);
        for field in AlbumField::ALL {
            assert_eq!(AlbumField::parse(field.as_str()), Some(field));
        }
        assert!(AlbumField::Barcode.is_text());
        assert!(!AlbumField::Credits.is_text());
    }

    fn edit(name: &str, role: &str, instrument: Option<&str>, tracks: &[&str]) -> EditedCredit {
        EditedCredit {
            name: name.into(),
            role: role.into(),
            instrument: instrument.map(str::to_string),
            track_ids: tracks.iter().map(|id| id.to_string()).collect(),
            artist_id: None,
        }
    }

    #[test]
    fn edited_credit_lists_are_checked_and_tidied() {
        let tracks = vec!["t1".to_string(), "t2".to_string()];
        let cleaned = clean_credit_edit(
            vec![
                edit(" Bill Evans ", "Performer", Some("Piano"), &["t1"]),
                edit("bill evans", "performer", Some("piano"), &["t2"]),
                edit("Teo Macero", "producer", Some("desk"), &[]),
            ],
            &tracks,
        )
        .unwrap();
        assert_eq!(cleaned.len(), 2);
        assert_eq!(cleaned[0].name, "Bill Evans");
        assert_eq!(cleaned[0].instrument.as_deref(), Some("piano"));
        // Both tracks of a two-track album is the whole album.
        assert_eq!(cleaned[0].track_ids, vec!["t1", "t2"]);
        // Only performers have instruments.
        assert_eq!(cleaned[1].instrument, None);

        assert!(clean_credit_edit(vec![edit("X", "drummer", None, &[])], &tracks).is_err());
        assert!(clean_credit_edit(vec![edit("  ", "composer", None, &[])], &tracks).is_err());
        assert!(clean_credit_edit(vec![edit("X", "composer", None, &["other"])], &tracks).is_err());

        let whole =
            clean_credit_edit(vec![edit("Y", "composer", None, &["t1", "t2"])], &tracks).unwrap();
        assert!(whole[0].track_ids.is_empty());
    }

    fn song(id: &str, credits: Vec<Credit>) -> Song {
        Song {
            id: id.into(),
            name: id.into(),
            credits,
            ..Default::default()
        }
    }

    fn credit(id: &str, name: &str, role: &str) -> Credit {
        Credit {
            artist_id: id.into(),
            name: name.into(),
            role: role.into(),
            instrument: None,
        }
    }

    #[test]
    fn edited_credits_replace_tag_credits_on_the_chosen_tracks() {
        let mut library = vec![Artist {
            id: "miles".into(),
            name: "Miles Davis".into(),
            albums: vec![Album {
                id: "kob".into(),
                songs: vec![
                    song(
                        "t1",
                        vec![
                            credit("miles", "Miles Davis", "primary"),
                            credit("wrong", "Wrong Person", "composer"),
                        ],
                    ),
                    song("t2", vec![credit("evans", "Bill Evans", "composer")]),
                ],
                ..Default::default()
            }],
            ..Default::default()
        }];
        let edits = HashMap::from([(
            "kob".to_string(),
            vec![
                edit("Bill Evans", "performer", Some("piano"), &["t2"]),
                edit("Miles Davis", "composer", None, &[]),
                EditedCredit {
                    artist_id: Some("mb:producer".into()),
                    ..edit("Teo Macero", "producer", None, &[])
                },
            ],
        )]);
        apply_credit_edits(&mut library, &edits);
        let songs = &library[0].albums[0].songs;
        let t1 = &songs[0].credits;
        assert!(t1.iter().any(|credit| credit.role == "primary"));
        assert!(!t1.iter().any(|credit| credit.name == "Wrong Person"));
        // Linked to the library artist with the same name.
        assert!(
            t1.iter()
                .any(|credit| credit.role == "composer" && credit.artist_id == "miles")
        );
        assert!(t1.iter().any(|credit| credit.artist_id == "mb:producer"));
        assert!(!t1.iter().any(|credit| credit.name == "Bill Evans"));
        let t2 = &songs[1].credits;
        let evans = t2
            .iter()
            .find(|credit| credit.name == "Bill Evans")
            .unwrap();
        assert_eq!(evans.artist_id, "evans");
        assert_eq!(evans.instrument.as_deref(), Some("piano"));
        // The old composer credit is gone.
        assert!(
            !t2.iter()
                .any(|credit| credit.role == "composer" && credit.name == "Bill Evans")
        );
    }

    #[test]
    fn the_most_common_tagged_value_wins() {
        assert_eq!(
            majority([Some("CL 1355"), None, Some("CL 1355"), Some("CS 8163")].into_iter())
                .as_deref(),
            Some("CL 1355")
        );
        assert_eq!(majority([Some(" "), None].into_iter()), None);
    }

    #[test]
    fn covers_follow_each_albums_source() {
        let directory = std::env::temp_dir().join(format!("parson-cover-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let fetched_path = directory.join("caa.jpg");
        std::fs::write(&fetched_path, b"jpeg").unwrap();
        let fetched_path = fetched_path.to_string_lossy().into_owned();
        let album = |id: &str, cover: &str| Album {
            id: id.into(),
            cover_url: cover.into(),
            ..Default::default()
        };
        let mut library = vec![Artist {
            albums: vec![
                album("own", "/music/own/cover.jpg"),
                album("online", "/music/online/cover.jpg"),
                album("bare", ""),
                album("uploaded", "/data/uploaded.jpg"),
            ],
            ..Default::default()
        }];
        let fetched = ["own", "online", "bare", "uploaded"]
            .into_iter()
            .map(|id| (id.to_string(), fetched_path.clone()))
            .collect::<HashMap<_, _>>();
        let sources = HashMap::from([
            ("online".to_string(), FactSource::Online),
            ("uploaded".to_string(), FactSource::Online),
        ]);
        let edited = HashSet::from(["uploaded".to_string()]);
        apply_cover_sources(&mut library, &fetched, FactSource::Tags, &sources, &edited);
        let covers = library[0]
            .albums
            .iter()
            .map(|album| album.cover_url.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            covers,
            [
                "/music/own/cover.jpg",
                fetched_path.as_str(),
                fetched_path.as_str(),
                "/data/uploaded.jpg"
            ]
        );
        std::fs::remove_dir_all(&directory).ok();
    }
}
