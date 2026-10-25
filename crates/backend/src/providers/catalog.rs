//! One album catalog over local files and streaming library albums, with
//! Roon-style focus filters (source, genre, decade, quality) that combine,
//! and facet counts for each filter given the others.
//!
//! Pure functions over already-loaded data, so the HTTP layer only gathers
//! the local cache and the listener's streaming rows.

use std::cmp::Ordering;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::store::LibraryItemRow;
use super::{AudioFormat, ProviderAlbum, ProviderId, entity_id};
use crate::domain::{Album, Artist};
use crate::library::search::normalize;

/// Genre facets beyond this many are left out (the long tail of tags).
pub const MAX_GENRE_FACETS: usize = 60;
pub const MAX_CATALOG_PAGE: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum CatalogSource {
    Local,
    Tidal,
    Qobuz,
}

impl CatalogSource {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "local" => Some(Self::Local),
            "tidal" => Some(Self::Tidal),
            "qobuz" => Some(Self::Qobuz),
            _ => None,
        }
    }

    fn from_provider(provider: ProviderId) -> Self {
        match provider {
            ProviderId::Tidal => Self::Tidal,
            ProviderId::Qobuz => Self::Qobuz,
        }
    }
}

/// Hi-Res (above 16/48), lossless CD quality, or lossy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum QualityTier {
    HiRes,
    Lossless,
    Lossy,
}

impl QualityTier {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "hi_res" => Some(Self::HiRes),
            "lossless" => Some(Self::Lossless),
            "lossy" => Some(Self::Lossy),
            _ => None,
        }
    }

    fn of(format: &AudioFormat) -> Self {
        if format.is_hi_res() {
            Self::HiRes
        } else if format.lossless {
            Self::Lossless
        } else {
            Self::Lossy
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CatalogAlbum {
    /// Unique across sources: `local:<id>` or the provider-scoped id.
    pub key: String,
    /// Library album id, or the provider-scoped id (`qobuz:...`).
    pub id: String,
    pub name: String,
    pub artist_id: String,
    pub artist_name: String,
    /// A library cover path, or an artwork URL for streaming albums.
    pub cover: String,
    pub year: Option<u16>,
    pub genres: Vec<String>,
    pub source: CatalogSource,
    pub format_label: Option<String>,
    pub quality: Option<QualityTier>,
    /// Whether the genres were borrowed from another copy of the album
    /// (TIDAL publishes none of its own).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub genre_inferred: bool,
}

impl CatalogAlbum {
    pub fn decade(&self) -> Option<u16> {
        self.year.map(|year| year / 10 * 10)
    }
}

fn year_of(date: &str) -> Option<u16> {
    date.get(..4)
        .and_then(|year| year.parse::<u16>().ok())
        .filter(|year| (1000..=2999).contains(year))
}

/// The best format among an album's tracks.
fn local_format(album: &Album) -> Option<AudioFormat> {
    album
        .songs
        .iter()
        .filter_map(|song| song.audio.as_ref())
        .filter_map(|audio| {
            let codec = audio.codec.as_deref()?;
            Some(match audio.lossless {
                Some(false) => AudioFormat::lossy(codec, audio.bitrate_kbps.unwrap_or(0)),
                _ if audio.lossless == Some(true) || audio.bit_depth.is_some() => {
                    AudioFormat::lossless(codec, audio.bit_depth, audio.sample_rate)
                }
                _ => return None,
            })
        })
        .max_by_key(|format| {
            (
                format.lossless,
                format.bit_depth.unwrap_or(0),
                format.sample_rate.unwrap_or(0),
                format.bitrate_kbps.unwrap_or(0),
            )
        })
}

/// The Library's local albums, with the same visibility rule as its paged
/// catalog: when any album has artwork, albums without it are hidden.
pub fn local_albums(
    artists: &[Artist],
    album_genres: &HashMap<String, Vec<String>>,
) -> Vec<CatalogAlbum> {
    let has_artwork = artists
        .iter()
        .flat_map(|artist| &artist.albums)
        .any(|album| !album.cover_url.trim().is_empty());
    artists
        .iter()
        .flat_map(|artist| artist.albums.iter().map(move |album| (artist, album)))
        .filter(|(_, album)| !has_artwork || !album.cover_url.trim().is_empty())
        .map(|(artist, album)| {
            let format = local_format(album);
            CatalogAlbum {
                key: format!("local:{}", album.id),
                id: album.id.clone(),
                name: album.name.clone(),
                artist_id: artist.id.clone(),
                artist_name: artist.name.clone(),
                cover: album.cover_url.clone(),
                year: year_of(&album.first_release_date),
                genres: album_genres
                    .get(&album.id)
                    .map(|genres| clean_genres(genres.iter().map(String::as_str)))
                    .unwrap_or_default(),
                source: CatalogSource::Local,
                quality: format.as_ref().map(QualityTier::of),
                format_label: format.map(|format| format.label),
                genre_inferred: false,
            }
        })
        .collect()
}

fn clean_genres<'a>(genres: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut seen = Vec::<String>::new();
    for genre in genres.map(str::trim).filter(|genre| !genre.is_empty()) {
        if !seen
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(genre))
        {
            seen.push(genre.to_string());
        }
    }
    seen
}

/// Streaming albums in the listener's library, read from their snapshots.
pub fn streaming_albums(rows: &[LibraryItemRow]) -> Vec<CatalogAlbum> {
    rows.iter()
        .filter(|row| row.kind == "album")
        .filter_map(|row| {
            let provider = ProviderId::parse(&row.provider)?;
            let snapshot = serde_json::from_str::<ProviderAlbum>(&row.payload).ok();
            let format = snapshot.as_ref().and_then(|album| album.format.clone());
            Some(CatalogAlbum {
                key: entity_id(provider, &row.provider_item_id),
                id: entity_id(provider, &row.provider_item_id),
                name: row.title.clone(),
                artist_id: snapshot
                    .as_ref()
                    .map(|album| album.artist.id.clone())
                    .unwrap_or_default(),
                artist_name: row.artist_name.clone(),
                cover: row.cover_url.clone().unwrap_or_default(),
                year: row.release_date.as_deref().and_then(year_of),
                genres: clean_genres(
                    snapshot
                        .as_ref()
                        .and_then(|album| album.genre.as_deref())
                        .into_iter(),
                ),
                source: CatalogSource::from_provider(provider),
                quality: format.as_ref().map(QualityTier::of),
                format_label: format
                    .map(|format| format.label)
                    .or_else(|| row.format_label.clone()),
                genre_inferred: false,
            })
        })
        .collect()
}

fn identity(album: &CatalogAlbum) -> (String, String) {
    (normalize(&album.artist_name), normalize(&album.name))
}

/// Albums without genres (every TIDAL album) borrow them from another copy
/// of the same album, by artist and title, in the library or on the other
/// service.
pub fn fill_missing_genres(albums: &mut [CatalogAlbum]) {
    let mut known = HashMap::<(String, String), Vec<String>>::new();
    for album in albums.iter().filter(|album| !album.genres.is_empty()) {
        known
            .entry(identity(album))
            .or_insert_with(|| album.genres.clone());
    }
    for album in albums.iter_mut().filter(|album| album.genres.is_empty()) {
        if let Some(genres) = known.get(&identity(album)) {
            album.genres = genres.clone();
            album.genre_inferred = true;
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CatalogFilter {
    pub source: Option<CatalogSource>,
    pub genre: Option<String>,
    pub decade: Option<u16>,
    pub quality: Option<QualityTier>,
    pub text: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Facet {
    Source,
    Genre,
    Decade,
    Quality,
}

impl CatalogFilter {
    fn matches(&self, album: &CatalogAlbum, tokens: &[String], except: Option<Facet>) -> bool {
        let source = except == Some(Facet::Source)
            || self.source.is_none_or(|source| album.source == source);
        let genre = except == Some(Facet::Genre)
            || self.genre.as_deref().is_none_or(|genre| {
                album
                    .genres
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(genre))
            });
        let decade =
            except == Some(Facet::Decade) || self.decade.is_none_or(|d| album.decade() == Some(d));
        let quality =
            except == Some(Facet::Quality) || self.quality.is_none_or(|q| album.quality == Some(q));
        let text = tokens.is_empty() || {
            let haystack = normalize(&format!("{} {}", album.name, album.artist_name));
            tokens.iter().all(|token| haystack.contains(token.as_str()))
        };
        source && genre && decade && quality && text
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FacetCount<T> {
    pub value: T,
    pub count: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct CatalogFacets {
    pub sources: Vec<FacetCount<CatalogSource>>,
    pub genres: Vec<FacetCount<String>>,
    pub decades: Vec<FacetCount<u16>>,
    pub qualities: Vec<FacetCount<QualityTier>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CatalogPage {
    pub albums: Vec<CatalogAlbum>,
    pub total: usize,
    pub facets: CatalogFacets,
}

fn compare(left: &CatalogAlbum, right: &CatalogAlbum) -> Ordering {
    left.artist_name
        .to_lowercase()
        .cmp(&right.artist_name.to_lowercase())
        .then_with(|| left.year.cmp(&right.year))
        .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
        .then_with(|| left.source.cmp(&right.source))
        .then_with(|| left.key.cmp(&right.key))
}

fn counted<T: Ord + Clone>(values: impl Iterator<Item = T>) -> Vec<FacetCount<T>> {
    let mut counts = std::collections::BTreeMap::<T, usize>::new();
    for value in values {
        *counts.entry(value).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(value, count)| FacetCount { value, count })
        .collect()
}

/// Filters, sorts (artist, year, title) and pages the catalog. Each facet
/// counts the albums matching every *other* filter, so the counts say what
/// picking that value would show.
pub fn query(
    mut albums: Vec<CatalogAlbum>,
    filter: &CatalogFilter,
    offset: usize,
    limit: usize,
) -> CatalogPage {
    let tokens = filter
        .text
        .as_deref()
        .map(normalize)
        .unwrap_or_default()
        .split_whitespace()
        .take(8)
        .map(str::to_string)
        .collect::<Vec<_>>();
    let tokens = tokens.as_slice();
    let facet = |except: Facet| {
        albums
            .iter()
            .filter(move |album| filter.matches(album, tokens, Some(except)))
    };

    let sources = counted(facet(Facet::Source).map(|album| album.source));
    let decades = counted(facet(Facet::Decade).filter_map(CatalogAlbum::decade));
    let qualities = counted(facet(Facet::Quality).filter_map(|album| album.quality));
    let mut genre_counts = HashMap::<String, (String, usize)>::new();
    for album in facet(Facet::Genre) {
        for genre in &album.genres {
            genre_counts
                .entry(genre.to_lowercase())
                .or_insert_with(|| (genre.clone(), 0))
                .1 += 1;
        }
    }
    let mut genres = genre_counts
        .into_values()
        .map(|(value, count)| FacetCount { value, count })
        .collect::<Vec<_>>();
    genres.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.value.to_lowercase().cmp(&right.value.to_lowercase()))
    });
    genres.truncate(MAX_GENRE_FACETS);
    let facets = CatalogFacets {
        sources,
        genres,
        decades,
        qualities,
    };

    albums.retain(|album| filter.matches(album, tokens, None));
    albums.sort_by(compare);
    let total = albums.len();
    let albums = albums
        .into_iter()
        .skip(offset)
        .take(limit.clamp(1, MAX_CATALOG_PAGE))
        .collect();
    CatalogPage {
        albums,
        total,
        facets,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{
        CatalogFilter, CatalogSource, QualityTier, fill_missing_genres, local_albums, query,
        streaming_albums,
    };
    use crate::domain::{Album, Artist, AudioProperties, Song};
    use crate::providers::store::{LibraryItemRow, NewLibraryItem};
    use crate::providers::{AudioFormat, ProviderId};

    fn song(codec: &str, depth: Option<u8>, rate: u32, lossless: bool) -> Song {
        Song {
            audio: Some(AudioProperties {
                codec: Some(codec.into()),
                sample_rate: Some(rate),
                bit_depth: depth,
                lossless: Some(lossless),
                bitrate_kbps: (!lossless).then_some(320),
                ..AudioProperties::default()
            }),
            ..Song::default()
        }
    }

    fn library() -> (Vec<Artist>, HashMap<String, Vec<String>>) {
        let album = |id: &str, name: &str, date: &str, songs: Vec<Song>| Album {
            id: id.into(),
            name: name.into(),
            cover_url: format!("covers/{id}.jpg"),
            first_release_date: date.into(),
            songs,
            ..Album::default()
        };
        let artists = vec![
            Artist {
                id: "a-ilse".into(),
                name: "Ilse Marten".into(),
                albums: vec![album(
                    "l-glass",
                    "Glass Orchard",
                    "2025-02-14",
                    vec![song("flac", Some(24), 96_000, true)],
                )],
                ..Artist::default()
            },
            Artist {
                id: "a-north".into(),
                name: "Northbound Brass".into(),
                albums: vec![
                    album(
                        "l-brass",
                        "Late Trains Live",
                        "1994-05-01",
                        vec![song("mp3", None, 44_100, false)],
                    ),
                    album("l-nocover", "Demos", "1999", Vec::new()),
                ],
                ..Artist::default()
            },
        ];
        let mut artists = artists;
        artists[1].albums[1].cover_url.clear();
        let genres = HashMap::from([
            ("l-glass".to_string(), vec!["Classical".to_string()]),
            (
                "l-brass".to_string(),
                vec!["Jazz".to_string(), "jazz".to_string(), " ".to_string()],
            ),
        ]);
        (artists, genres)
    }

    fn row(item: &NewLibraryItem) -> LibraryItemRow {
        LibraryItemRow {
            provider: item.provider.as_str().into(),
            kind: item.kind.as_str().into(),
            provider_item_id: item.provider_item_id.clone(),
            title: item.title.clone(),
            artist_name: item.artist_name.clone(),
            album_title: None,
            cover_url: item.cover_url.clone(),
            release_date: item.release_date.clone(),
            duration: item.duration,
            track_count: item.track_count,
            format_label: item.format_label.clone(),
            payload: item.payload.clone(),
            origin: "parson".into(),
            added_at: 0,
        }
    }

    fn streaming() -> Vec<LibraryItemRow> {
        let mut harbor = crate::providers::store::tests::album(
            ProviderId::Qobuz,
            "q0harborhr04",
            "Harbor Lights",
            "Mara Vell",
        );
        harbor.genre = Some("Pop/Rock".into());
        harbor.release_date = Some("2024-03-08".into());
        harbor.format = Some(AudioFormat::lossless("flac", Some(24), Some(192_000)));
        let mut tidal_harbor = crate::providers::store::tests::album(
            ProviderId::Tidal,
            "300100",
            "Harbor Lights",
            "Mara Vell",
        );
        tidal_harbor.genre = None;
        tidal_harbor.release_date = Some("2024-03-08".into());
        tidal_harbor.format = Some(AudioFormat::lossless("flac", Some(24), None));
        let mut moons = crate::providers::store::tests::album(
            ProviderId::Tidal,
            "300200",
            "Paper Moons",
            "Mara Vell",
        );
        moons.genre = None;
        moons.release_date = Some("2021-09-17".into());
        moons.format = Some(AudioFormat::lossless("flac", Some(16), Some(44_100)));
        [harbor, tidal_harbor, moons]
            .iter()
            .map(|album| row(&NewLibraryItem::album(album)))
            .collect()
    }

    fn catalog() -> Vec<super::CatalogAlbum> {
        let (artists, genres) = library();
        let mut albums = local_albums(&artists, &genres);
        albums.extend(streaming_albums(&streaming()));
        fill_missing_genres(&mut albums);
        albums
    }

    #[test]
    fn local_and_streaming_albums_share_one_sorted_catalog() {
        let page = query(catalog(), &CatalogFilter::default(), 0, 50);
        assert_eq!(page.total, 5, "the coverless local album stays hidden");
        assert_eq!(
            page.albums
                .iter()
                .map(|album| (
                    album.artist_name.as_str(),
                    album.name.as_str(),
                    album.source
                ))
                .collect::<Vec<_>>(),
            [
                ("Ilse Marten", "Glass Orchard", CatalogSource::Local),
                ("Mara Vell", "Paper Moons", CatalogSource::Tidal),
                ("Mara Vell", "Harbor Lights", CatalogSource::Tidal),
                ("Mara Vell", "Harbor Lights", CatalogSource::Qobuz),
                ("Northbound Brass", "Late Trains Live", CatalogSource::Local),
            ]
        );
        let glass = &page.albums[0];
        assert_eq!(glass.key, "local:l-glass");
        assert_eq!(glass.format_label.as_deref(), Some("24/96 FLAC"));
        assert_eq!(glass.quality, Some(QualityTier::HiRes));
        assert_eq!(page.albums[4].quality, Some(QualityTier::Lossy));
        assert_eq!(page.albums[4].genres, ["Jazz"], "genres are de-duplicated");
        assert_eq!(page.albums[3].id, "qobuz:q0harborhr04");
        assert_eq!(page.albums[3].format_label.as_deref(), Some("24/192 FLAC"));
    }

    #[test]
    fn tidal_albums_borrow_genres_from_another_copy() {
        let albums = catalog();
        let tidal_harbor = albums
            .iter()
            .find(|album| album.id == "tidal:300100")
            .expect("tidal harbor");
        assert_eq!(tidal_harbor.genres, ["Pop/Rock"]);
        assert!(tidal_harbor.genre_inferred);
        let moons = albums
            .iter()
            .find(|album| album.id == "tidal:300200")
            .expect("moons");
        assert!(moons.genres.is_empty() && !moons.genre_inferred);
    }

    #[test]
    fn source_genre_decade_and_quality_filters_combine() {
        let filter = CatalogFilter {
            source: Some(CatalogSource::Tidal),
            genre: Some("pop/rock".into()),
            decade: Some(2020),
            ..CatalogFilter::default()
        };
        let page = query(catalog(), &filter, 0, 50);
        assert_eq!(page.total, 1);
        assert_eq!(page.albums[0].id, "tidal:300100");

        // Facets count each value with the other filters applied.
        let sources = page
            .facets
            .sources
            .iter()
            .map(|facet| (facet.value, facet.count))
            .collect::<Vec<_>>();
        assert_eq!(
            sources,
            [(CatalogSource::Tidal, 1), (CatalogSource::Qobuz, 1)],
            "Pop/Rock in the 2020s: one TIDAL and one Qobuz copy"
        );
        assert_eq!(
            page.facets
                .genres
                .iter()
                .map(|facet| (facet.value.as_str(), facet.count))
                .collect::<Vec<_>>(),
            [("Pop/Rock", 1)],
            "TIDAL albums from the 2020s with a genre"
        );
        assert_eq!(
            page.facets
                .decades
                .iter()
                .map(|facet| (facet.value, facet.count))
                .collect::<Vec<_>>(),
            [(2020, 1)]
        );

        let hi_res = query(
            catalog(),
            &CatalogFilter {
                quality: Some(QualityTier::HiRes),
                ..CatalogFilter::default()
            },
            0,
            50,
        );
        assert_eq!(hi_res.total, 3);
        let nineties = query(
            catalog(),
            &CatalogFilter {
                decade: Some(1990),
                source: Some(CatalogSource::Local),
                ..CatalogFilter::default()
            },
            0,
            50,
        );
        assert_eq!(nineties.albums[0].name, "Late Trains Live");
        assert_eq!(
            query(
                catalog(),
                &CatalogFilter {
                    text: Some("harbor".into()),
                    ..CatalogFilter::default()
                },
                0,
                50
            )
            .total,
            2
        );
    }

    #[test]
    fn pages_are_bounded() {
        let page = query(catalog(), &CatalogFilter::default(), 3, 0);
        assert_eq!(page.albums.len(), 1);
        assert_eq!(page.total, 5);
        assert!(
            query(catalog(), &CatalogFilter::default(), 99, 10)
                .albums
                .is_empty()
        );
    }
}
