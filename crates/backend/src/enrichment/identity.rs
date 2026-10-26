//! One identity per person, across the names tags and MusicBrainz use.
//!
//! Tags name people as text ("Q-Tip"); MusicBrainz credits name them as a
//! release prints them ("Jonathan Davis") next to a stable id. Without a
//! merge the same person shows up twice: twice in an album's credits, twice
//! on a genre page, and with their albums split over two pages, which also
//! splits every Focus filter on those pages.
//!
//! A person's canonical id is, in order of preference:
//! 1. the local artist (or tag credit) linked to their MusicBrainz id by a
//!    release, an artist lookup or a tagged MusicBrainz artist id;
//! 2. the local person whose name is one of the names MusicBrainz knows
//!    them by (their own name, a credited-as name or an alias), as long as
//!    that name does not also belong to another MusicBrainz person;
//! 3. `mb:<MusicBrainz id>`, a page for someone only MusicBrainz credits.
//!
//! A tag credit on an album MusicBrainz also credits is resolved with that
//! album's own credits first, so "Jonathan Davis" on an A Tribe Called
//! Quest album and "Jonathan Davis" on a Korn album stay two people.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use crate::api::editorial::EXTERNAL_PREFIX;
use crate::enrichment::store::{self, AliasRow, CreditNameRow, CreditRow, PersonLinkRow};
use crate::library::identity::normalize_artist_identity;
use crate::library::state::LibraryCache;
use crate::persistence::connection::DbPool;

/// Everything needed to resolve a person, loaded once per request.
#[derive(Debug, Default, Clone)]
pub struct PersonIdentity {
    /// Local ids linked to each MusicBrainz id.
    mbid_locals: HashMap<String, BTreeSet<String>>,
    /// MusicBrainz ids linked to each local id.
    local_mbids: HashMap<String, BTreeSet<String>>,
    /// MusicBrainz people known by each normalized name.
    name_mbids: HashMap<String, BTreeSet<String>>,
    /// Each MusicBrainz person's own name, normalized.
    mbid_names: HashMap<String, String>,
    /// Every normalized name each MusicBrainz person is known by.
    mbid_all_names: HashMap<String, BTreeSet<String>>,
    /// Which MusicBrainz person a name credited on an album is.
    album_names: HashMap<(String, String), BTreeSet<String>>,
    /// Local people by normalized name.
    local_names: HashMap<String, BTreeSet<String>>,
    /// Each local person's name, as tags spell it.
    display: HashMap<String, String>,
    /// Local artists with albums of their own.
    owners: HashSet<String>,
}

/// The ids and names that make up one person.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PersonMembers {
    pub canonical: String,
    /// Local ids (artists and tag credits) that are this person.
    pub local_ids: Vec<String>,
    pub mbids: Vec<String>,
    /// Normalized names the person is known by in this library.
    pub names: Vec<String>,
}

impl PersonIdentity {
    pub fn build(
        cache: &LibraryCache,
        links: &[PersonLinkRow],
        aliases: &[AliasRow],
        credited: &[CreditNameRow],
    ) -> Self {
        let mut identity = PersonIdentity::default();
        let add_local = |identity: &mut PersonIdentity, id: &str, name: &str| {
            if id.is_empty() || id.starts_with(EXTERNAL_PREFIX) {
                return;
            }
            identity
                .local_names
                .entry(normalize_artist_identity(name))
                .or_default()
                .insert(id.to_string());
            identity
                .display
                .entry(id.to_string())
                .or_insert_with(|| name.to_string());
        };
        for artist in cache.artists.iter() {
            add_local(&mut identity, &artist.id, &artist.name);
            if !artist.albums.is_empty() {
                identity.owners.insert(artist.id.clone());
            }
            for song in artist.albums.iter().flat_map(|album| &album.songs) {
                for credit in &song.credits {
                    add_local(&mut identity, &credit.artist_id, &credit.name);
                }
                for (name, id) in song
                    .contributing_artists
                    .iter()
                    .zip(&song.contributing_artist_ids)
                {
                    add_local(&mut identity, id, name);
                }
            }
        }
        for link in links {
            if !identity.display.contains_key(&link.artist_id) {
                continue;
            }
            identity
                .mbid_locals
                .entry(link.mbid.clone())
                .or_default()
                .insert(link.artist_id.clone());
            identity
                .local_mbids
                .entry(link.artist_id.clone())
                .or_default()
                .insert(link.mbid.clone());
        }
        for alias in aliases {
            identity
                .name_mbids
                .entry(alias.normalized_name.clone())
                .or_default()
                .insert(alias.mbid.clone());
            if alias.kind == "name" {
                identity
                    .mbid_names
                    .insert(alias.mbid.clone(), alias.normalized_name.clone());
            }
        }
        for row in credited {
            identity
                .album_names
                .entry((row.album_id.clone(), row.normalized_name.clone()))
                .or_default()
                .insert(row.mbid.clone());
            identity
                .name_mbids
                .entry(row.normalized_name.clone())
                .or_default()
                .insert(row.mbid.clone());
        }
        for (name, mbids) in &identity.name_mbids {
            for mbid in mbids {
                identity
                    .mbid_all_names
                    .entry(mbid.clone())
                    .or_default()
                    .insert(name.clone());
            }
        }
        identity
    }

    /// Normalized names MusicBrainz knows a person by.
    fn names_of(&self, mbid: &str) -> Vec<&str> {
        self.mbid_all_names
            .get(mbid)
            .map(|names| names.iter().map(String::as_str).collect())
            .unwrap_or_default()
    }

    /// The local people that are a MusicBrainz person.
    fn locals_of(&self, mbid: &str) -> BTreeSet<String> {
        let mut locals = self.mbid_locals.get(mbid).cloned().unwrap_or_default();
        for name in self.names_of(mbid) {
            // A name two MusicBrainz people share proves nothing.
            if self
                .name_mbids
                .get(name)
                .is_some_and(|mbids| mbids.len() > 1)
            {
                continue;
            }
            for local in self.local_names.get(name).into_iter().flatten() {
                // A local person linked to someone else is not this one.
                if self
                    .local_mbids
                    .get(local)
                    .is_some_and(|linked| !linked.contains(mbid))
                {
                    continue;
                }
                locals.insert(local.clone());
            }
        }
        locals
    }

    /// The preferred local id among several that are one person: one with
    /// albums of their own, then one spelled like their MusicBrainz name.
    fn preferred(&self, mbid: &str, locals: &BTreeSet<String>) -> Option<String> {
        let own_name = self.mbid_names.get(mbid);
        locals
            .iter()
            .min_by_key(|id| {
                (
                    !self.owners.contains(*id),
                    own_name.is_none_or(|name| {
                        self.display
                            .get(*id)
                            .is_none_or(|display| normalize_artist_identity(display) != *name)
                    }),
                    (*id).clone(),
                )
            })
            .cloned()
    }

    /// The canonical id of a MusicBrainz person.
    pub fn for_mbid(&self, mbid: &str) -> String {
        self.preferred(mbid, &self.locals_of(mbid))
            .unwrap_or_else(|| format!("{EXTERNAL_PREFIX}{mbid}"))
    }

    /// The single MusicBrainz person a local id or name is, when known.
    fn mbid_of_local(&self, id: &str, name: &str, album_id: Option<&str>) -> Option<String> {
        let normalized = normalize_artist_identity(name);
        if let Some(album_id) = album_id
            && let Some(mbids) = self
                .album_names
                .get(&(album_id.to_string(), normalized.clone()))
            && mbids.len() == 1
        {
            return mbids.first().cloned();
        }
        if let Some(mbids) = self.local_mbids.get(id) {
            return (mbids.len() == 1).then(|| mbids.first().cloned()).flatten();
        }
        let mbids = self.name_mbids.get(&normalized)?;
        if mbids.len() != 1 {
            return None;
        }
        let mbid = mbids.first()?;
        // Only merge by name when this id is one of the locals the person
        // resolves to (a namesake linked elsewhere stays apart).
        self.locals_of(mbid).contains(id).then(|| mbid.clone())
    }

    /// The canonical id of a local artist or tag credit, optionally on an
    /// album (whose MusicBrainz credits decide first).
    pub fn for_local(&self, id: &str, name: &str, album_id: Option<&str>) -> String {
        if let Some(mbid) = id.strip_prefix(EXTERNAL_PREFIX) {
            return self.for_mbid(mbid);
        }
        match self.mbid_of_local(id, name, album_id) {
            Some(mbid) => {
                let canonical = self.for_mbid(&mbid);
                // An album credit naming someone with no local identity
                // keeps the tag credit's own page.
                if canonical.starts_with(EXTERNAL_PREFIX) && self.display.contains_key(id) {
                    id.to_string()
                } else {
                    canonical
                }
            }
            None => id.to_string(),
        }
    }

    /// The canonical id of a MusicBrainz credit row: by its MusicBrainz id,
    /// or by name when it has none.
    pub fn for_row(&self, row: &CreditRow) -> Option<String> {
        if let Some(mbid) = row.source_artist_id.as_deref().filter(|id| !id.is_empty()) {
            return Some(self.for_mbid(mbid));
        }
        let locals = self.local_names.get(&row.normalized_name)?;
        let id = locals.first()?;
        Some(self.for_local(id, &row.name, Some(&row.album_id)))
    }

    /// The local display name for a canonical id, when it is local.
    pub fn display_name(&self, id: &str) -> Option<&str> {
        self.display.get(id).map(String::as_str)
    }

    /// Every id and name that is the same person as `id`.
    pub fn members(&self, id: &str) -> PersonMembers {
        let canonical = match self.display.get(id) {
            Some(name) => self.for_local(id, name, None),
            None => match id.strip_prefix(EXTERNAL_PREFIX) {
                Some(mbid) => self.for_mbid(mbid),
                None => id.to_string(),
            },
        };
        let mut mbids: BTreeSet<String> = BTreeSet::new();
        if let Some(mbid) = canonical.strip_prefix(EXTERNAL_PREFIX) {
            mbids.insert(mbid.to_string());
        }
        if let Some(linked) = self.local_mbids.get(&canonical) {
            mbids.extend(linked.iter().cloned());
        }
        if let Some(mbid) = self
            .display
            .get(&canonical)
            .and_then(|name| self.mbid_of_local(&canonical, name, None))
        {
            mbids.insert(mbid);
        }
        let mut locals: BTreeSet<String> = BTreeSet::new();
        if self.display.contains_key(&canonical) {
            locals.insert(canonical.clone());
        }
        for mbid in &mbids {
            for local in self.locals_of(mbid) {
                if self.for_local(&local, &self.display[&local], None) == canonical {
                    locals.insert(local);
                }
            }
        }
        if self.display.contains_key(id) && self.for_local(id, &self.display[id], None) == canonical
        {
            locals.insert(id.to_string());
        }
        let mut names: BTreeSet<String> = locals
            .iter()
            .filter_map(|local| self.display.get(local))
            .map(|name| normalize_artist_identity(name))
            .collect();
        for mbid in &mbids {
            names.extend(self.names_of(mbid).into_iter().map(str::to_string));
        }
        PersonMembers {
            canonical,
            local_ids: locals.into_iter().collect(),
            mbids: mbids.into_iter().collect(),
            names: names.into_iter().collect(),
        }
    }
}

/// Loads what identity resolution needs from the database.
pub fn load(
    connection: &mut diesel::SqliteConnection,
    cache: &LibraryCache,
) -> diesel::QueryResult<PersonIdentity> {
    Ok(PersonIdentity::build(
        cache,
        &store::person_links(connection)?,
        &store::load_aliases(connection)?,
        &store::credited_names(connection)?,
    ))
}

/// Loads person identities for a request; on a database error people are
/// still resolved by their local ids.
pub async fn load_identity(pool: DbPool, cache: Arc<LibraryCache>) -> PersonIdentity {
    let fallback = cache.clone();
    let loaded = actix_web::web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        load(&mut connection, &cache).map_err(|error| error.to_string())
    })
    .await;
    match loaded {
        Ok(Ok(identity)) => identity,
        Ok(Err(error)) => {
            tracing::warn!(%error, "could not load person identities");
            PersonIdentity::build(&fallback, &[], &[], &[])
        }
        Err(error) => {
            tracing::warn!(%error, "could not load person identities");
            PersonIdentity::build(&fallback, &[], &[], &[])
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::domain::{Album, Artist, Credit, Song};

    pub const QTIP: &str = "7a9b2c5a-4e7e-4b8c-9c7a-1d2e3f4a5b6c";
    pub const KORN_JD: &str = "0c1d2e3f-4a5b-4c6d-8e7f-9a0b1c2d3e4f";

    fn credit(id: &str, name: &str, role: &str) -> Credit {
        Credit {
            artist_id: id.into(),
            name: name.into(),
            role: role.into(),
            instrument: None,
        }
    }

    fn song(id: &str, credits: Vec<Credit>) -> Song {
        Song {
            id: id.into(),
            name: id.into(),
            artist: "A Tribe Called Quest".into(),
            credits,
            ..Song::default()
        }
    }

    fn album(id: &str, songs: Vec<Song>) -> Album {
        Album {
            id: id.into(),
            name: id.into(),
            cover_url: String::new(),
            songs,
            first_release_date: "1991".into(),
            musicbrainz_id: String::new(),
            wikidata_id: None,
            primary_type: "Album".into(),
            description: String::new(),
            contributing_artists: Vec::new(),
            contributing_artists_ids: Vec::new(),
            release_album: None,
            release_group_album: None,
        }
    }

    pub fn cache() -> LibraryCache {
        let atcq = Artist {
            id: "atcq".into(),
            name: "A Tribe Called Quest".into(),
            albums: vec![album(
                "low-end",
                vec![song(
                    "check-the-rhime",
                    vec![credit("tag-qtip", "Q-Tip", "composer")],
                )],
            )],
            ..Artist::default()
        };
        let korn = Artist {
            id: "korn".into(),
            name: "Korn".into(),
            albums: vec![album(
                "korn-album",
                vec![song(
                    "blind",
                    vec![credit("tag-jd", "Jonathan Davis", "composer")],
                )],
            )],
            ..Artist::default()
        };
        LibraryCache::from_artists(vec![atcq, korn])
    }

    fn alias(mbid: &str, name: &str, kind: &str) -> AliasRow {
        AliasRow {
            mbid: mbid.into(),
            normalized_name: normalize_artist_identity(name),
            name: name.into(),
            kind: kind.into(),
        }
    }

    fn credited(album: &str, name: &str, mbid: &str) -> CreditNameRow {
        CreditNameRow {
            album_id: album.into(),
            normalized_name: normalize_artist_identity(name),
            mbid: mbid.into(),
        }
    }

    pub fn identity() -> PersonIdentity {
        PersonIdentity::build(
            &cache(),
            &[],
            &[
                alias(QTIP, "Q-Tip", "name"),
                alias(QTIP, "Jonathan Davis", "credit"),
                alias(KORN_JD, "Jonathan Davis", "name"),
            ],
            &[
                credited("low-end", "Jonathan Davis", QTIP),
                credited("korn-album", "Jonathan Davis", KORN_JD),
            ],
        )
    }

    #[test]
    fn a_credited_as_name_merges_into_the_tagged_person() {
        let identity = identity();
        // MusicBrainz credits Q-Tip as "Jonathan Davis": the same person as
        // the "Q-Tip" tag credit.
        assert_eq!(identity.for_mbid(QTIP), "tag-qtip");
        assert_eq!(identity.for_local("tag-qtip", "Q-Tip", None), "tag-qtip");
        let members = identity.members(&format!("mb:{QTIP}"));
        assert_eq!(members.canonical, "tag-qtip");
        assert_eq!(members.local_ids, vec!["tag-qtip".to_string()]);
        assert_eq!(members.mbids, vec![QTIP.to_string()]);
        assert!(members.names.contains(&"jonathan davis".to_string()));
    }

    #[test]
    fn a_shared_name_stays_two_people_by_album() {
        let identity = identity();
        // Korn's singer has the same name: on the Korn album the tag
        // credit is him, and his MusicBrainz id is a page of his own.
        assert_eq!(
            identity.for_local("tag-jd", "Jonathan Davis", Some("korn-album")),
            "tag-jd"
        );
        assert_eq!(identity.for_mbid(KORN_JD), format!("mb:{KORN_JD}"));
        assert_eq!(identity.for_mbid(QTIP), "tag-qtip");
        // On the Tribe album a "Jonathan Davis" tag credit is Q-Tip.
        assert_eq!(
            identity.for_local("tag-jd", "Jonathan Davis", Some("low-end")),
            "tag-qtip"
        );
        // Without an album the shared name proves nothing.
        assert_eq!(
            identity.for_local("tag-jd", "Jonathan Davis", None),
            "tag-jd"
        );
    }

    #[test]
    fn a_linked_mbid_wins_over_names_and_prefers_an_artist_with_albums() {
        let links = [
            PersonLinkRow {
                artist_id: "atcq".into(),
                mbid: QTIP.into(),
            },
            PersonLinkRow {
                artist_id: "tag-qtip".into(),
                mbid: QTIP.into(),
            },
        ];
        let identity = PersonIdentity::build(&cache(), &links, &[], &[]);
        // Contrived, but proves the order: an artist with albums of their
        // own is the page both ids open.
        assert_eq!(identity.for_mbid(QTIP), "atcq");
        assert_eq!(identity.for_local("tag-qtip", "Q-Tip", None), "atcq");
        let members = identity.members("tag-qtip");
        assert_eq!(members.canonical, "atcq");
        assert_eq!(
            members.local_ids,
            vec!["atcq".to_string(), "tag-qtip".to_string()]
        );
    }

    #[test]
    fn rows_resolve_by_mbid_or_by_name() {
        let identity = identity();
        let row = CreditRow {
            album_id: "low-end".into(),
            track_id: String::new(),
            name: "Jonathan Davis".into(),
            normalized_name: "jonathan davis".into(),
            role: "composer".into(),
            instrument: String::new(),
            source: "musicbrainz".into(),
            source_artist_id: Some(QTIP.into()),
        };
        assert_eq!(identity.for_row(&row).as_deref(), Some("tag-qtip"));
        let unlinked = CreditRow {
            source_artist_id: None,
            name: "Q-Tip".into(),
            normalized_name: normalize_artist_identity("Q-Tip"),
            ..row
        };
        assert_eq!(identity.for_row(&unlinked).as_deref(), Some("tag-qtip"));
    }

    #[test]
    fn nobody_known_resolves_to_themselves() {
        let identity = PersonIdentity::default();
        assert_eq!(identity.for_local("x", "Somebody", None), "x");
        assert_eq!(identity.for_mbid(QTIP), format!("mb:{QTIP}"));
        assert_eq!(identity.members("x").canonical, "x");
    }
}
