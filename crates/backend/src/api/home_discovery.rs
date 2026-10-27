//! Discovery rows for the Home screen: recently added, daily picks, new for
//! you, similar artists, an artist's best years, genres and a short profile
//! of the library (lossless and high-resolution share, composers, hours).
//!
//! Everything here is a pure function over the in-memory catalog plus the
//! listener's recent history, so the rows are cheap to build on every Home
//! request and are deterministic for a given user and day.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::domain::{Album, Artist};
pub use crate::library::quality::{AlbumQuality, album_quality};
use crate::library::state::LibraryCache;

/// Albums shown in a discovery row.
pub const ROW_SIZE: usize = 18;
/// Picks in the Daily Picks band (Roon shows three to five a day).
pub const DAILY_PICK_COUNT: usize = 5;
/// Albums played this recently are not picked again.
const RECENT_ALBUM_EXCLUSION: usize = 12;
/// Genre tiles on Home.
const GENRE_TILE_COUNT: usize = 12;

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ArtistRef {
    pub id: String,
    pub name: String,
}

/// A compact album for discovery rows (no track list).
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct HomeAlbum {
    pub id: String,
    pub name: String,
    pub cover_url: String,
    pub first_release_date: String,
    pub primary_type: String,
    pub artist: ArtistRef,
    pub track_count: usize,
    pub duration_seconds: f64,
    pub quality: Option<AlbumQuality>,
    pub genres: Vec<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct DailyPick {
    pub album: HomeAlbum,
    /// "Because you played Ghosts I–IV", or "From your library".
    pub reason: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct DailyPicks {
    /// The day the picks are for, `YYYY-MM-DD`.
    pub date: String,
    pub picks: Vec<DailyPick>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct HomeArtist {
    pub id: String,
    pub name: String,
    /// A cover of theirs (the fallback picture).
    pub image_url: String,
    /// Their photo from Wikimedia Commons (`/artists/{id}/image`), when
    /// online metadata found one. Pages prefer it to the cover.
    pub photo_url: Option<String>,
    pub album_count: usize,
    /// "Similar to Nine Inch Nails", when the row was seeded by a listen.
    pub reason: Option<String>,
}

/// Roon's "In their prime": the stretch of years an artist made most of
/// the music you own (or play) by them.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct BestYears {
    pub artist: ArtistRef,
    pub image_url: String,
    /// The artist's photo, when online metadata found one.
    pub photo_url: Option<String>,
    pub from_year: i32,
    pub to_year: i32,
    pub albums: Vec<HomeAlbum>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct HomeGenre {
    pub name: String,
    /// The genre page's id (`/genre?slug=`): the canonical genre when the
    /// tree knows the name, else a URL-safe form of it.
    pub slug: String,
    pub album_count: usize,
    pub artist_count: usize,
    /// Up to four distinct covers for the tile collage.
    pub covers: Vec<String>,
    /// Photos of up to three of the genre's artists (most albums first),
    /// for the duotone tile. Empty until online metadata finds photos.
    pub photos: Vec<String>,
    /// The genre's artists, most albums first; used to pick `photos`.
    #[serde(skip)]
    pub artist_ids: Vec<String>,
}

/// What the library is made of, for the stat cards under the greeting.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct LibraryProfile {
    pub composer_count: usize,
    pub genre_count: usize,
    pub total_duration_seconds: f64,
    pub lossless_tracks: usize,
    pub hi_res_tracks: usize,
    pub hi_res_albums: usize,
}

/// The listener's recent history reduced to what the rows need.
#[derive(Default)]
pub struct ListeningContext {
    /// Most recent first, one entry per album.
    pub recent_album_ids: Vec<String>,
    /// Plays per artist id over the recent history.
    pub artist_plays: HashMap<String, usize>,
}

impl ListeningContext {
    pub fn from_history(song_ids: &[String], cache: &LibraryCache) -> Self {
        let mut context = Self::default();
        let mut seen = HashSet::new();
        for song_id in song_ids {
            let Some((artist_id, album_id)) = cache.song_map.get(song_id) else {
                continue;
            };
            *context.artist_plays.entry(artist_id.clone()).or_default() += 1;
            if seen.insert(album_id.clone()) {
                context.recent_album_ids.push(album_id.clone());
            }
        }
        context
    }
}

/// FNV-1a: a stable hash, so picks only change when the day does.
fn stable_hash(parts: &[&str]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for part in parts {
        for byte in part.as_bytes().iter().chain(std::iter::once(&0xffu8)) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    hash
}

pub fn has_cover(path: &str) -> bool {
    let path = path.trim().to_ascii_lowercase();
    !path.is_empty() && !path.ends_with("snf.png")
}

/// The four-digit year at the start of a release date.
pub fn release_year(date: &str) -> Option<i32> {
    let year = date.trim().get(..4)?;
    year.chars()
        .all(|c| c.is_ascii_digit())
        .then(|| year.parse::<i32>().ok())
        .flatten()
        .filter(|year| *year > 0)
}

pub fn home_album(artist: &Artist, album: &Album, cache: &LibraryCache) -> HomeAlbum {
    HomeAlbum {
        id: album.id.clone(),
        name: album.name.clone(),
        cover_url: album.cover_url.clone(),
        first_release_date: album.first_release_date.clone(),
        primary_type: album.primary_type.clone(),
        artist: ArtistRef {
            id: artist.id.clone(),
            name: artist.name.clone(),
        },
        track_count: album.songs.len(),
        duration_seconds: album.songs.iter().map(|song| song.duration.max(0.0)).sum(),
        quality: album_quality(album),
        genres: cache
            .album_genres
            .get(&album.id)
            .cloned()
            .unwrap_or_default(),
    }
}

/// Every album with at least one track, with its owning artist.
fn playable_albums(cache: &LibraryCache) -> impl Iterator<Item = (&Artist, &Album)> {
    cache.artists.iter().flat_map(|artist| {
        artist
            .albums
            .iter()
            .filter(|album| !album.songs.is_empty())
            .map(move |album| (artist, album))
    })
}

/// Albums in the order the catalog first saw them (newest first), as read
/// from the database; ids the cache no longer knows are skipped. Without an
/// order (no database), the catalog's own order is reversed.
pub fn recently_added(cache: &LibraryCache, newest_first: &[String]) -> Vec<HomeAlbum> {
    let mut rows = Vec::new();
    let mut seen = HashSet::new();
    for album_id in newest_first {
        if rows.len() >= ROW_SIZE {
            break;
        }
        let (Some(album), Some(artist)) = (cache.album(album_id), cache.album_owner(album_id))
        else {
            continue;
        };
        if album.songs.is_empty() || !seen.insert(album_id.as_str()) {
            continue;
        }
        rows.push(home_album(artist, album, cache));
    }
    if rows.is_empty() {
        let mut albums = playable_albums(cache).collect::<Vec<_>>();
        albums.reverse();
        rows = albums
            .into_iter()
            .take(ROW_SIZE)
            .map(|(artist, album)| home_album(artist, album, cache))
            .collect();
    }
    rows
}

fn genres_of_albums<'a>(
    cache: &'a LibraryCache,
    album_ids: impl IntoIterator<Item = &'a String>,
) -> HashMap<&'a str, usize> {
    let mut weights = HashMap::new();
    for album_id in album_ids {
        for genre in cache.album_genres.get(album_id).into_iter().flatten() {
            *weights.entry(genre.as_str()).or_default() += 1;
        }
    }
    weights
}

/// Three to five albums for today, seeded by user and date. Albums sharing a
/// genre with recent listening come first; the reason names the album that
/// led to the pick. Recently played albums are left out.
pub fn daily_picks(
    cache: &LibraryCache,
    context: &ListeningContext,
    user_key: &str,
    date: &str,
) -> DailyPicks {
    let recent: HashSet<&str> = context
        .recent_album_ids
        .iter()
        .take(RECENT_ALBUM_EXCLUSION)
        .map(String::as_str)
        .collect();
    let recent_genres = genres_of_albums(cache, context.recent_album_ids.iter().take(8));
    let mut candidates = playable_albums(cache)
        .filter(|(_, album)| !recent.contains(album.id.as_str()))
        .map(|(artist, album)| {
            let genres = cache.album_genres.get(&album.id);
            let affinity = genres
                .into_iter()
                .flatten()
                .map(|genre| recent_genres.get(genre.as_str()).copied().unwrap_or(0))
                .sum::<usize>();
            let jitter = stable_hash(&[user_key, date, &album.id]);
            (
                affinity.min(3),
                has_cover(&album.cover_url),
                jitter,
                artist,
                album,
            )
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then(right.1.cmp(&left.1))
            .then(left.2.cmp(&right.2))
    });
    let mut artists_used = HashSet::new();
    let mut picks = Vec::new();
    for (affinity, _, _, artist, album) in &candidates {
        if picks.len() >= DAILY_PICK_COUNT {
            break;
        }
        // One album per artist keeps the band varied.
        if !artists_used.insert(artist.id.as_str()) {
            continue;
        }
        let reason = if *affinity > 0 {
            because_album(cache, context, album)
        } else {
            None
        }
        .unwrap_or_else(|| "From your library".to_string());
        picks.push(DailyPick {
            album: home_album(artist, album, cache),
            reason,
        });
    }
    DailyPicks {
        date: date.to_string(),
        picks,
    }
}

/// "Because you played <album>": the most recent played album sharing a genre.
fn because_album(
    cache: &LibraryCache,
    context: &ListeningContext,
    album: &Album,
) -> Option<String> {
    let genres: HashSet<&str> = cache
        .album_genres
        .get(&album.id)?
        .iter()
        .map(String::as_str)
        .collect();
    context.recent_album_ids.iter().find_map(|played_id| {
        let shares = cache
            .album_genres
            .get(played_id)?
            .iter()
            .any(|genre| genres.contains(genre.as_str()));
        shares
            .then(|| cache.album(played_id))
            .flatten()
            .map(|played| format!("Because you played {}", played.name))
    })
}

/// The newest releases in the library you have not played lately.
pub fn new_for_you(cache: &LibraryCache, context: &ListeningContext) -> Vec<HomeAlbum> {
    let recent: HashSet<&str> = context
        .recent_album_ids
        .iter()
        .take(RECENT_ALBUM_EXCLUSION)
        .map(String::as_str)
        .collect();
    let mut albums = playable_albums(cache)
        .filter(|(_, album)| !recent.contains(album.id.as_str()))
        .filter_map(|(artist, album)| {
            release_year(&album.first_release_date).map(|_| (artist, album))
        })
        .collect::<Vec<_>>();
    albums.sort_by(|left, right| {
        right
            .1
            .first_release_date
            .cmp(&left.1.first_release_date)
            .then_with(|| left.1.name.cmp(&right.1.name))
    });
    albums
        .into_iter()
        .take(ROW_SIZE)
        .map(|(artist, album)| home_album(artist, album, cache))
        .collect()
}

fn artist_genres<'a>(cache: &'a LibraryCache, artist: &'a Artist) -> HashSet<&'a str> {
    artist
        .albums
        .iter()
        .flat_map(|album| cache.album_genres.get(&album.id).into_iter().flatten())
        .map(String::as_str)
        .collect()
}

/// An artist's photo when stored, else one of their covers.
pub fn artist_image(artist: &Artist) -> String {
    if has_cover(&artist.icon_url) {
        return artist.icon_url.clone();
    }
    artist
        .albums
        .iter()
        .map(|album| album.cover_url.as_str())
        .find(|cover| has_cover(cover))
        .unwrap_or_default()
        .to_string()
}

/// The artist the listener played most lately, else the one with the most
/// albums (ties broken by the day's seed so Home varies).
fn seed_artist<'a>(
    cache: &'a LibraryCache,
    context: &ListeningContext,
    user_key: &str,
    date: &str,
) -> Option<&'a Artist> {
    cache
        .artists
        .iter()
        .filter(|artist| artist.albums.iter().any(|album| !album.songs.is_empty()))
        .max_by(|left, right| {
            let plays =
                |artist: &Artist| context.artist_plays.get(&artist.id).copied().unwrap_or(0);
            plays(left)
                .cmp(&plays(right))
                .then(left.albums.len().cmp(&right.albums.len()))
                .then_with(|| {
                    stable_hash(&[user_key, date, &right.id])
                        .cmp(&stable_hash(&[user_key, date, &left.id]))
                })
        })
}

/// Artists sharing the most genres with the seed artist (Jaccard overlap).
/// Falls back to artists with the most albums when nothing overlaps.
pub fn similar_artists(
    cache: &LibraryCache,
    context: &ListeningContext,
    user_key: &str,
    date: &str,
) -> Vec<HomeArtist> {
    let Some(seed) = seed_artist(cache, context, user_key, date) else {
        return Vec::new();
    };
    rank_similar(cache, seed, ROW_SIZE)
}

/// Artists like the given one (shared genres), for its artist page. Only
/// artists with at least one shared genre are returned.
pub fn similar_to(cache: &LibraryCache, artist_id: &str, limit: usize) -> Vec<HomeArtist> {
    let Some(seed) = cache.artist(artist_id) else {
        return Vec::new();
    };
    rank_similar(cache, seed, limit)
        .into_iter()
        .filter(|artist| artist.reason.is_some())
        .collect()
}

fn rank_similar(cache: &LibraryCache, seed: &Artist, limit: usize) -> Vec<HomeArtist> {
    let seed_genres = artist_genres(cache, seed);
    let mut scored = cache
        .artists
        .iter()
        .filter(|artist| artist.id != seed.id)
        .filter(|artist| artist.albums.iter().any(|album| !album.songs.is_empty()))
        .map(|artist| {
            let genres = artist_genres(cache, artist);
            let shared = genres.intersection(&seed_genres).count();
            let union = genres.union(&seed_genres).count().max(1);
            let score = (shared * 1000) / union;
            (score, artist)
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then(right.1.albums.len().cmp(&left.1.albums.len()))
            .then_with(|| left.1.name.cmp(&right.1.name))
    });
    scored
        .into_iter()
        .take(limit)
        .map(|(score, artist)| HomeArtist {
            id: artist.id.clone(),
            name: artist.name.clone(),
            image_url: artist_image(artist),
            photo_url: None,
            album_count: artist
                .albums
                .iter()
                .filter(|album| !album.songs.is_empty())
                .count(),
            reason: (score > 0).then(|| format!("Similar to {}", seed.name)),
        })
        .collect()
}

/// The three-year window holding most of an artist's dated albums.
fn densest_window(years: &[i32]) -> Option<(i32, i32)> {
    let mut best: Option<(usize, i32)> = None;
    for start in years {
        let count = years
            .iter()
            .filter(|year| (*start..=*start + 2).contains(*year))
            .count();
        if best.is_none_or(|(best_count, best_start)| {
            count > best_count || (count == best_count && *start < best_start)
        }) {
            best = Some((count, *start));
        }
    }
    let (_, start) = best?;
    let inside = years
        .iter()
        .copied()
        .filter(|year| (start..=start + 2).contains(year))
        .collect::<Vec<_>>();
    Some((*inside.iter().min()?, *inside.iter().max()?))
}

/// An artist's best years: the played (or largest) artist with at least two
/// dated albums, and the albums from their densest three-year stretch.
pub fn best_years(
    cache: &LibraryCache,
    context: &ListeningContext,
    user_key: &str,
    date: &str,
) -> Option<BestYears> {
    let artist = cache
        .artists
        .iter()
        .filter(|artist| {
            artist
                .albums
                .iter()
                .filter(|album| !album.songs.is_empty())
                .filter(|album| release_year(&album.first_release_date).is_some())
                .count()
                >= 2
        })
        .max_by(|left, right| {
            let plays =
                |artist: &Artist| context.artist_plays.get(&artist.id).copied().unwrap_or(0);
            plays(left)
                .cmp(&plays(right))
                .then(left.albums.len().cmp(&right.albums.len()))
                .then_with(|| {
                    stable_hash(&[user_key, date, "prime", &right.id])
                        .cmp(&stable_hash(&[user_key, date, "prime", &left.id]))
                })
        })?;
    let years = artist
        .albums
        .iter()
        .filter(|album| !album.songs.is_empty())
        .filter_map(|album| release_year(&album.first_release_date))
        .collect::<Vec<_>>();
    let (from_year, to_year) = densest_window(&years)?;
    let mut albums = artist
        .albums
        .iter()
        .filter(|album| !album.songs.is_empty())
        .filter(|album| {
            release_year(&album.first_release_date)
                .is_some_and(|year| (from_year..=to_year).contains(&year))
        })
        .collect::<Vec<_>>();
    albums.sort_by(|left, right| left.first_release_date.cmp(&right.first_release_date));
    Some(BestYears {
        artist: ArtistRef {
            id: artist.id.clone(),
            name: artist.name.clone(),
        },
        image_url: artist_image(artist),
        photo_url: None,
        from_year,
        to_year,
        albums: albums
            .into_iter()
            .map(|album| home_album(artist, album, cache))
            .collect(),
    })
}

/// The library's biggest genres, each with a collage of its covers.
pub fn genre_tiles(cache: &LibraryCache) -> Vec<HomeGenre> {
    struct Tile<'a> {
        albums: usize,
        artists: HashMap<&'a str, usize>,
        covers: Vec<&'a str>,
    }
    let mut tiles: HashMap<&str, Tile> = HashMap::new();
    for (artist, album) in playable_albums(cache) {
        for genre in cache.album_genres.get(&album.id).into_iter().flatten() {
            let tile = tiles.entry(genre.as_str()).or_insert_with(|| Tile {
                albums: 0,
                artists: HashMap::new(),
                covers: Vec::new(),
            });
            tile.albums += 1;
            *tile.artists.entry(artist.id.as_str()).or_default() += 1;
            if has_cover(&album.cover_url)
                && tile.covers.len() < 4
                && !tile.covers.contains(&album.cover_url.as_str())
            {
                tile.covers.push(album.cover_url.as_str());
            }
        }
    }
    let mut tiles = tiles
        .into_iter()
        .filter(|(name, _)| !name.trim().is_empty())
        .map(|(name, tile)| {
            let mut artists = tile.artists.into_iter().collect::<Vec<_>>();
            artists.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(right.0)));
            HomeGenre {
                name: name.to_string(),
                slug: genre_slug(name),
                album_count: tile.albums,
                artist_count: artists.len(),
                covers: tile.covers.into_iter().map(str::to_string).collect(),
                photos: Vec::new(),
                artist_ids: artists.into_iter().map(|(id, _)| id.to_string()).collect(),
            }
        })
        .collect::<Vec<_>>();
    tiles.sort_by(|left, right| {
        right
            .album_count
            .cmp(&left.album_count)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    tiles.truncate(GENRE_TILE_COUNT);
    tiles
}

/// How many artist photos a genre tile shows.
const GENRE_TILE_PHOTOS: usize = 3;

/// Every artist whose photo Home could show, for one stored-photo lookup.
pub fn photo_candidates(
    similar: &[HomeArtist],
    best: Option<&BestYears>,
    genres: &[HomeGenre],
) -> Vec<String> {
    let mut ids = similar
        .iter()
        .map(|artist| artist.id.clone())
        .chain(best.map(|best| best.artist.id.clone()))
        .chain(
            genres
                .iter()
                .flat_map(|genre| genre.artist_ids.iter().take(12).cloned()),
        )
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    ids
}

/// Points Home's artist pictures at the stored photos, where there are
/// any: the round "Similar artists" images, the "In their prime" artist,
/// and up to three photos per genre tile.
pub fn attach_photos(
    with_photos: &HashSet<String>,
    similar: &mut [HomeArtist],
    best: Option<&mut BestYears>,
    genres: &mut [HomeGenre],
) {
    let photo = |id: &str| {
        with_photos
            .contains(id)
            .then(|| crate::api::editorial::artist_image_url(id))
    };
    for artist in similar {
        artist.photo_url = photo(&artist.id);
    }
    if let Some(best) = best {
        best.photo_url = photo(&best.artist.id);
    }
    for genre in genres {
        genre.photos = genre
            .artist_ids
            .iter()
            .filter_map(|id| photo(id))
            .take(GENRE_TILE_PHOTOS)
            .collect();
    }
}

/// The genre page slug for a raw genre name.
pub fn genre_slug(name: &str) -> String {
    crate::enrichment::genres::canonical(name)
        .map(|node| node.slug.to_string())
        .unwrap_or_else(|| crate::enrichment::genres::free_slug(name))
}

pub fn library_profile(cache: &LibraryCache) -> LibraryProfile {
    let mut profile = LibraryProfile::default();
    let mut composers = HashSet::new();
    for (_, album) in playable_albums(cache) {
        if album_quality(album).is_some_and(|quality| quality.hi_res) {
            profile.hi_res_albums += 1;
        }
        for song in &album.songs {
            profile.total_duration_seconds += song.duration.max(0.0);
            for credit in song
                .credits
                .iter()
                .filter(|credit| credit.role == "composer")
            {
                composers.insert(if credit.artist_id.is_empty() {
                    credit.name.to_lowercase()
                } else {
                    credit.artist_id.clone()
                });
            }
            let Some(audio) = song.audio.as_ref() else {
                continue;
            };
            if audio.lossless == Some(true) {
                profile.lossless_tracks += 1;
                if audio.bit_depth.unwrap_or(0) > 16 || audio.sample_rate.unwrap_or(0) > 48_000 {
                    profile.hi_res_tracks += 1;
                }
            }
        }
    }
    profile.composer_count = composers.len();
    profile.genre_count = cache
        .album_genres
        .values()
        .flatten()
        .map(|genre| genre.to_lowercase())
        .collect::<HashSet<_>>()
        .len();
    profile
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use super::*;
    use crate::domain::{AudioProperties, Credit, Song};
    use crate::library::search::SearchIndex;

    fn song(id: &str, bits: u8, rate: u32, lossless: bool) -> Song {
        Song {
            id: id.into(),
            name: id.into(),
            artist: "Artist".into(),
            duration: 60.0,
            audio: Some(AudioProperties {
                codec: Some(if lossless { "flac" } else { "mp3" }.into()),
                container: None,
                sample_rate: Some(rate),
                bit_depth: lossless.then_some(bits),
                channels: Some(2),
                bitrate_kbps: None,
                lossless: Some(lossless),
            }),
            ..Song::default()
        }
    }

    fn album(id: &str, date: &str, songs: Vec<Song>) -> Album {
        Album {
            id: id.into(),
            name: format!("Album {id}"),
            cover_url: format!("{id}.jpg"),
            first_release_date: date.into(),
            primary_type: "Album".into(),
            songs,
            ..Album::default()
        }
    }

    fn artist(id: &str, albums: Vec<Album>) -> Artist {
        Artist {
            id: id.into(),
            name: format!("Artist {id}"),
            albums,
            ..Artist::default()
        }
    }

    fn cache(artists: Vec<Artist>, genres: &[(&str, &[&str])]) -> LibraryCache {
        let mut song_map = HashMap::new();
        let mut artist_positions = HashMap::new();
        let mut album_positions = HashMap::new();
        let mut song_positions = HashMap::new();
        let mut songs_flat = Vec::new();
        for (artist_index, artist) in artists.iter().enumerate() {
            artist_positions.insert(artist.id.clone(), artist_index as _);
            for (album_index, album) in artist.albums.iter().enumerate() {
                album_positions.insert(album.id.clone(), (artist_index as _, album_index as _));
                for (song_index, song) in album.songs.iter().enumerate() {
                    song_map.insert(song.id.clone(), (artist.id.clone(), album.id.clone()));
                    song_positions.insert(
                        song.id.clone(),
                        (artist_index as _, album_index as _, song_index as _),
                    );
                    songs_flat.push(song.id.clone());
                }
            }
        }
        LibraryCache {
            search_index: SearchIndex::build(&artists).unwrap(),
            artists: Arc::new(artists),
            song_map,
            album_genres: genres
                .iter()
                .map(|(album, genres)| {
                    (
                        album.to_string(),
                        genres.iter().map(|genre| genre.to_string()).collect(),
                    )
                })
                .collect(),
            artist_positions,
            album_positions,
            song_positions,
            songs_flat,
            songs_by_artist: HashMap::new(),
            songs_by_genre: HashMap::new(),
            image_paths: Default::default(),
        }
    }

    fn fixture() -> LibraryCache {
        cache(
            vec![
                artist(
                    "nin",
                    vec![
                        album("slip", "2008-05-05", vec![song("s1", 24, 96_000, true)]),
                        album("ghosts", "2008-03-02", vec![song("g1", 24, 96_000, true)]),
                        album("old", "1994", vec![song("o1", 16, 44_100, true)]),
                    ],
                ),
                artist(
                    "bach",
                    vec![album(
                        "goldberg",
                        "2012",
                        vec![song("b1", 16, 44_100, true), song("b2", 16, 44_100, true)],
                    )],
                ),
                artist(
                    "folk",
                    vec![album("town", "2011", vec![song("f1", 0, 44_100, false)])],
                ),
                artist(
                    "ambient",
                    vec![album("leaves", "2006", vec![song("l1", 16, 44_100, true)])],
                ),
            ],
            &[
                ("slip", &["Industrial", "Rock"]),
                ("ghosts", &["Ambient", "Industrial"]),
                ("old", &["Industrial"]),
                ("goldberg", &["Classical"]),
                ("town", &["Folk", "Rock"]),
                ("leaves", &["Ambient"]),
            ],
        )
    }

    #[test]
    fn quality_reports_the_best_format_and_hi_res_only_for_lossless() {
        let cache = fixture();
        let quality = album_quality(cache.album("slip").unwrap()).unwrap();
        assert_eq!(quality.bit_depth, Some(24));
        assert_eq!(quality.sample_rate, Some(96_000));
        assert!(quality.lossless && quality.hi_res);
        let cd = album_quality(cache.album("goldberg").unwrap()).unwrap();
        assert!(cd.lossless && !cd.hi_res);
        let lossy = album_quality(cache.album("town").unwrap()).unwrap();
        assert!(!lossy.lossless && !lossy.hi_res);
        assert_eq!(album_quality(&Album::default()), None);
    }

    #[test]
    fn release_year_reads_leading_digits_only() {
        assert_eq!(release_year("2008-05-05"), Some(2008));
        assert_eq!(release_year(" 1994 "), Some(1994));
        assert_eq!(release_year("unknown"), None);
        assert_eq!(release_year("0000-00-00"), None);
        assert_eq!(release_year(""), None);
    }

    #[test]
    fn recently_added_follows_database_order_and_skips_unknown_ids() {
        let cache = fixture();
        let rows = recently_added(
            &cache,
            &[
                "leaves".into(),
                "missing".into(),
                "slip".into(),
                "leaves".into(),
            ],
        );
        assert_eq!(
            rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            ["leaves", "slip"]
        );
        assert_eq!(rows[1].quality.as_ref().unwrap().bit_depth, Some(24));
        assert_eq!(rows[1].genres, ["Industrial", "Rock"]);
        // No database order: newest catalog entries first.
        assert_eq!(recently_added(&cache, &[]).first().unwrap().id, "leaves");
    }

    #[test]
    fn daily_picks_are_stable_per_day_varied_by_artist_and_skip_recent_plays() {
        let cache = fixture();
        let context = ListeningContext::from_history(&["l1".into()], &cache);
        let picks = daily_picks(&cache, &context, "7", "2026-10-27");
        assert_eq!(picks.date, "2026-10-27");
        assert_eq!(picks, daily_picks(&cache, &context, "7", "2026-10-27"));
        let ids = picks
            .picks
            .iter()
            .map(|pick| pick.album.id.as_str())
            .collect::<Vec<_>>();
        assert!(!ids.contains(&"leaves"), "just played");
        let artists = picks
            .picks
            .iter()
            .map(|pick| pick.album.artist.id.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(artists.len(), picks.picks.len(), "one album per artist");
        // Ghosts shares Ambient with the played album, so it leads.
        assert_eq!(ids[0], "ghosts");
        assert_eq!(picks.picks[0].reason, "Because you played Album leaves");
        assert!(
            picks
                .picks
                .iter()
                .skip(1)
                .all(|pick| pick.reason == "From your library")
        );
    }

    #[test]
    fn new_for_you_orders_by_release_date_and_leaves_out_recent_plays() {
        let cache = fixture();
        let context = ListeningContext::from_history(&["b1".into()], &cache);
        let rows = new_for_you(&cache, &context);
        assert_eq!(
            rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            ["town", "slip", "ghosts", "leaves", "old"]
        );
    }

    #[test]
    fn similar_artists_rank_by_shared_genres_with_the_most_played_artist() {
        let cache = fixture();
        let context = ListeningContext::from_history(&["f1".into(), "f1".into()], &cache);
        let similar = similar_artists(&cache, &context, "7", "2026-10-27");
        assert_eq!(similar[0].id, "nin");
        assert_eq!(similar[0].reason.as_deref(), Some("Similar to Artist folk"));
        assert_eq!(similar[0].image_url, "slip.jpg");
        assert_eq!(similar[0].album_count, 3);
        let bach = similar.iter().find(|artist| artist.id == "bach").unwrap();
        assert_eq!(bach.reason, None, "no shared genre");
    }

    #[test]
    fn similar_to_keeps_only_artists_sharing_a_genre() {
        let cache = fixture();
        let similar = similar_to(&cache, "ambient", 10);
        assert_eq!(
            similar
                .iter()
                .map(|artist| artist.id.as_str())
                .collect::<Vec<_>>(),
            ["nin"]
        );
        assert!(similar_to(&cache, "missing", 10).is_empty());
    }

    #[test]
    fn best_years_pick_the_densest_three_year_window() {
        let cache = fixture();
        let prime = best_years(&cache, &ListeningContext::default(), "7", "2026-10-27").unwrap();
        assert_eq!(prime.artist.id, "nin");
        assert_eq!((prime.from_year, prime.to_year), (2008, 2008));
        assert_eq!(
            prime
                .albums
                .iter()
                .map(|album| album.id.as_str())
                .collect::<Vec<_>>(),
            ["ghosts", "slip"]
        );
        assert_eq!(
            densest_window(&[1990, 1991, 1999, 2000, 2001]),
            Some((1999, 2001))
        );
        assert_eq!(densest_window(&[]), None);
    }

    #[test]
    fn genre_tiles_count_albums_and_artists_with_distinct_covers() {
        let cache = fixture();
        let tiles = genre_tiles(&cache);
        assert_eq!(tiles[0].name, "Industrial");
        assert_eq!(tiles[0].album_count, 3);
        assert_eq!(tiles[0].artist_count, 1);
        assert_eq!(tiles[0].covers, ["slip.jpg", "ghosts.jpg", "old.jpg"]);
        let names = tiles
            .iter()
            .map(|tile| tile.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names[1..3], ["Ambient", "Rock"]);
        assert!(!tiles[0].slug.is_empty());
        assert_eq!(genre_slug("Zorb Wave"), "zorb-wave");
    }

    #[test]
    fn stored_artist_photos_replace_covers_on_home() {
        let cache = fixture();
        let mut genres = genre_tiles(&cache);
        let industrial = genres[0].artist_ids.clone();
        assert!(!industrial.is_empty());
        let mut similar = vec![
            HomeArtist {
                id: industrial[0].clone(),
                name: "With photo".into(),
                image_url: "slip.jpg".into(),
                photo_url: None,
                album_count: 3,
                reason: None,
            },
            HomeArtist {
                id: "no-photo".into(),
                name: "Without".into(),
                image_url: "cover.jpg".into(),
                photo_url: None,
                album_count: 1,
                reason: None,
            },
        ];
        let mut best = BestYears {
            artist: ArtistRef {
                id: industrial[0].clone(),
                name: "With photo".into(),
            },
            image_url: "slip.jpg".into(),
            photo_url: None,
            from_year: 2008,
            to_year: 2008,
            albums: Vec::new(),
        };
        let candidates = photo_candidates(&similar, Some(&best), &genres);
        assert!(candidates.contains(&"no-photo".to_string()));
        assert_eq!(
            candidates.iter().filter(|id| **id == industrial[0]).count(),
            1,
            "each artist is looked up once"
        );
        let with_photos = HashSet::from([industrial[0].clone()]);
        attach_photos(&with_photos, &mut similar, Some(&mut best), &mut genres);
        let photo = format!("/artists/{}/image", industrial[0]);
        assert_eq!(similar[0].photo_url.as_deref(), Some(photo.as_str()));
        assert_eq!(
            similar[0].image_url, "slip.jpg",
            "the cover stays as fallback"
        );
        assert_eq!(similar[1].photo_url, None);
        assert_eq!(best.photo_url.as_deref(), Some(photo.as_str()));
        assert_eq!(genres[0].photos, [photo]);
        // The internal artist list never reaches the client.
        let json = serde_json::to_value(&genres[0]).unwrap();
        assert!(json.get("artist_ids").is_none());
        assert!(json.get("photos").is_some());
    }

    #[test]
    fn library_profile_counts_lossless_hi_res_and_composers() {
        let mut cache = fixture();
        let mut artists = (*cache.artists).clone();
        artists[1].albums[0].songs[0].credits = vec![Credit {
            artist_id: String::new(),
            name: "J. S. Bach".into(),
            role: "composer".into(),
            instrument: None,
        }];
        artists[1].albums[0].songs[1].credits = artists[1].albums[0].songs[0].credits.clone();
        cache.artists = Arc::new(artists);
        let profile = library_profile(&cache);
        assert_eq!(profile.composer_count, 1);
        assert_eq!(profile.lossless_tracks, 6);
        assert_eq!(profile.hi_res_tracks, 2);
        assert_eq!(profile.hi_res_albums, 2);
        assert_eq!(profile.genre_count, 5);
        assert_eq!(profile.total_duration_seconds, 420.0);
    }

    #[test]
    fn stable_hash_separates_parts() {
        assert_ne!(stable_hash(&["ab", "c"]), stable_hash(&["a", "bc"]));
        assert_eq!(stable_hash(&["x"]), stable_hash(&["x"]));
    }
}
