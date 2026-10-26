//! Controller API for home automation (the Home Assistant integration and
//! anything like it): a browse tree over the library, "play this item in
//! that zone" (by browse id or by search), zone radio and thumbs up/down.
//!
//! Controllers never see a queue builder: they name an item and a zone and
//! the server resolves the songs, so every remote gets the same album
//! order, playlist order and radio as the Parson apps.

use std::collections::{BTreeMap, HashSet};

use actix_web::{HttpRequest, HttpResponse, get, post, web};
use rand::seq::{IndexedRandom, SliceRandom};
use serde::{Deserialize, Serialize};

use crate::api::auth::authenticated_user_id;
use crate::api::zones::{dispatch_command, error_response, library_cache, viewer_device};
use crate::domain::{Album, AudioProperties, Song};
use crate::library::search::normalize;
use crate::library::state::{LibraryCache, LibraryLifecycle};
use crate::persistence::connection::DbPool;
use crate::recommendation::{PlaybackEventRequest, record_playback_event};
use crate::zones::{MAX_ZONE_QUEUE_ITEMS, ZoneCommand, ZoneError, ZoneService};

/// Lists longer than this are split into A-Z buckets.
const BUCKET_THRESHOLD: usize = 300;
/// Most children one browse response carries.
const MAX_BROWSE_CHILDREN: usize = 500;
/// Radio queues: the seed plus this many picks.
const RADIO_ITEMS: usize = 60;
/// Genres offered as radio stations.
const RADIO_GENRES: usize = 24;
const MAX_SEARCH_QUERY_CHARACTERS: usize = 256;
const MAX_ITEM_ID_CHARACTERS: usize = 512;
const SEARCH_ARTISTS: usize = 8;
const SEARCH_ALBUMS: usize = 12;
const SEARCH_SONGS: usize = 25;

/// One node of the browse tree.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BrowseItem {
    /// Stable id: pass it back to `/browse?item=` or `/zones/{id}/play`.
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// Artwork path for `/media/images/{path}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// `library`, `album`, `artist`, `playlist`, `genre`, `song`, `radio`
    /// or `directory`.
    pub kind: &'static str,
    pub playable: bool,
    pub expandable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    /// Source format, e.g. "FLAC 24/96" (hi-res and lossless are worth
    /// showing on a remote).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BrowseResponse {
    pub item: BrowseItem,
    pub children: Vec<BrowseItem>,
    /// Children before the `MAX_BROWSE_CHILDREN` cut.
    pub total: usize,
}

/// A parsed browse id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemId {
    Root,
    Albums(Option<char>),
    Artists(Option<char>),
    Playlists,
    Genres,
    Radio,
    Liked,
    Album(String),
    Artist(String),
    Playlist(i32),
    Genre(String),
    Song(String),
    LibraryRadio,
    GenreRadio(String),
}

impl ItemId {
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.is_empty() || value.chars().count() > MAX_ITEM_ID_CHARACTERS {
            return None;
        }
        let bucket = |rest: &str| -> Option<Option<char>> {
            if rest.is_empty() {
                return Some(None);
            }
            let mut chars = rest.chars();
            let letter = chars.next()?;
            (chars.next().is_none() && (letter.is_ascii_uppercase() || letter == '#'))
                .then_some(Some(letter))
        };
        Some(match value {
            "root" => Self::Root,
            "playlists" => Self::Playlists,
            "genres" => Self::Genres,
            "radio" => Self::Radio,
            "liked" => Self::Liked,
            "radio:library" => Self::LibraryRadio,
            _ => {
                if let Some(rest) = value.strip_prefix("albums") {
                    Self::Albums(bucket(rest.strip_prefix('/').unwrap_or(rest))?)
                } else if let Some(rest) = value.strip_prefix("artists") {
                    Self::Artists(bucket(rest.strip_prefix('/').unwrap_or(rest))?)
                } else if let Some(genre) = value.strip_prefix("radio:genre:") {
                    Self::GenreRadio(non_empty(genre)?)
                } else if let Some(id) = value.strip_prefix("album:") {
                    Self::Album(non_empty(id)?)
                } else if let Some(id) = value.strip_prefix("artist:") {
                    Self::Artist(non_empty(id)?)
                } else if let Some(id) = value.strip_prefix("playlist:") {
                    Self::Playlist(id.parse().ok()?)
                } else if let Some(genre) = value.strip_prefix("genre:") {
                    Self::Genre(non_empty(genre)?)
                } else {
                    Self::Song(non_empty(value.strip_prefix("song:")?)?)
                }
            }
        })
    }
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

/// "FLAC 24/96", "ALAC 16/44.1", "MP3 320" or `None` when unknown.
pub fn quality_label(audio: &AudioProperties) -> Option<String> {
    let codec = audio
        .codec
        .as_deref()
        .or(audio.container.as_deref())?
        .to_ascii_uppercase();
    let codec = match codec.as_str() {
        "VORBIS" => "Ogg Vorbis".to_string(),
        "OPUS" => "Opus".to_string(),
        "PCM" => audio
            .container
            .as_deref()
            .map(str::to_ascii_uppercase)
            .unwrap_or(codec.clone()),
        _ => codec,
    };
    if audio.lossless == Some(true) {
        let rate = audio.sample_rate.map(|rate| {
            let khz = f64::from(rate) / 1000.0;
            if (khz - khz.round()).abs() < 0.05 {
                format!("{}", khz.round() as u32)
            } else {
                format!("{khz:.1}")
            }
        });
        return Some(match (audio.bit_depth, rate) {
            (Some(depth), Some(rate)) => format!("{codec} {depth}/{rate}"),
            (None, Some(rate)) => format!("{codec} {rate} kHz"),
            _ => codec,
        });
    }
    Some(match audio.bitrate_kbps {
        Some(bitrate) => format!("{codec} {bitrate}"),
        None => codec,
    })
}

/// The best format among an album's tracks, for album tiles.
fn album_quality(album: &Album) -> Option<String> {
    album
        .songs
        .iter()
        .filter_map(|song| song.audio.as_ref())
        .max_by_key(|audio| {
            (
                audio.lossless == Some(true),
                audio.bit_depth.unwrap_or(0),
                audio.sample_rate.unwrap_or(0),
                audio.bitrate_kbps.unwrap_or(0),
            )
        })
        .and_then(quality_label)
}

fn directory(id: &str, title: &str, playable: bool) -> BrowseItem {
    BrowseItem {
        id: id.to_string(),
        title: title.to_string(),
        subtitle: None,
        image: None,
        kind: "directory",
        playable,
        expandable: true,
        duration_ms: None,
        quality: None,
    }
}

fn image(path: &str) -> Option<String> {
    (!path.trim().is_empty()).then(|| path.to_string())
}

fn album_item(cache: &LibraryCache, album: &Album) -> BrowseItem {
    BrowseItem {
        id: format!("album:{}", album.id),
        title: album.name.clone(),
        subtitle: cache
            .album_owner(&album.id)
            .map(|artist| artist.name.clone()),
        image: image(&album.cover_url),
        kind: "album",
        playable: true,
        expandable: true,
        duration_ms: None,
        quality: album_quality(album),
    }
}

fn artist_item(artist: &crate::domain::Artist) -> BrowseItem {
    let image = image(&artist.icon_url).or_else(|| {
        artist
            .albums
            .iter()
            .find_map(|album| image(&album.cover_url))
    });
    BrowseItem {
        id: format!("artist:{}", artist.id),
        title: artist.name.clone(),
        subtitle: Some(match artist.albums.len() {
            1 => "1 album".to_string(),
            count => format!("{count} albums"),
        }),
        image,
        kind: "artist",
        playable: true,
        expandable: true,
        duration_ms: None,
        quality: None,
    }
}

fn song_item(cache: &LibraryCache, song: &Song) -> BrowseItem {
    let album = cache
        .song_map
        .get(&song.id)
        .and_then(|(_, album_id)| cache.album(album_id));
    BrowseItem {
        id: format!("song:{}", song.id),
        title: song.name.clone(),
        subtitle: Some(song.artist.clone()),
        image: album.and_then(|album| image(&album.cover_url)),
        kind: "song",
        playable: true,
        expandable: false,
        duration_ms: Some((song.duration * 1000.0).round() as i64),
        quality: song.audio.as_ref().and_then(quality_label),
    }
}

fn genre_item(genre: &str, songs: usize) -> BrowseItem {
    BrowseItem {
        id: format!("genre:{genre}"),
        title: genre.to_string(),
        subtitle: Some(match songs {
            1 => "1 song".to_string(),
            count => format!("{count} songs"),
        }),
        image: None,
        kind: "genre",
        playable: true,
        expandable: true,
        duration_ms: None,
        quality: None,
    }
}

fn radio_item(id: String, title: String, subtitle: &str) -> BrowseItem {
    BrowseItem {
        id,
        title,
        subtitle: Some(subtitle.to_string()),
        image: None,
        kind: "radio",
        playable: true,
        expandable: false,
        duration_ms: None,
        quality: None,
    }
}

/// The A-Z bucket a name sorts into (`#` for anything else).
fn bucket_of(name: &str) -> char {
    let folded = normalize(name);
    let folded = folded
        .strip_prefix("the ")
        .map(str::to_string)
        .unwrap_or(folded);
    folded
        .chars()
        .next()
        .map(|letter| letter.to_ascii_uppercase())
        .filter(char::is_ascii_uppercase)
        .unwrap_or('#')
}

fn sort_key(name: &str) -> String {
    let folded = normalize(name);
    folded
        .strip_prefix("the ")
        .map(str::to_string)
        .unwrap_or(folded)
}

/// Splits a long list into A-Z buckets, or lists one bucket.
fn bucketed(
    parent: &str,
    items: Vec<(String, BrowseItem)>,
    letter: Option<char>,
) -> Vec<BrowseItem> {
    if letter.is_none() && items.len() <= BUCKET_THRESHOLD {
        return items.into_iter().map(|(_, item)| item).collect();
    }
    match letter {
        Some(letter) => items
            .into_iter()
            .filter(|(name, _)| bucket_of(name) == letter)
            .map(|(_, item)| item)
            .collect(),
        None => {
            let mut counts = BTreeMap::<char, usize>::new();
            for (name, _) in &items {
                *counts.entry(bucket_of(name)).or_default() += 1;
            }
            let mut buckets: Vec<BrowseItem> = counts
                .into_iter()
                .map(|(letter, count)| BrowseItem {
                    subtitle: Some(count.to_string()),
                    ..directory(&format!("{parent}/{letter}"), &letter.to_string(), false)
                })
                .collect();
            // "#" sorts first in BTreeMap order; put it last like a phone book.
            if buckets.first().is_some_and(|bucket| bucket.title == "#") {
                let numbers = buckets.remove(0);
                buckets.push(numbers);
            }
            buckets
        }
    }
}

fn all_albums(cache: &LibraryCache) -> Vec<&Album> {
    cache
        .artists
        .iter()
        .flat_map(|artist| artist.albums.iter())
        .collect()
}

/// Album songs in play order (track number, then title).
fn album_songs(album: &Album) -> Vec<Song> {
    let mut album = album.clone();
    crate::library::normalize::dedupe_and_sort_album_songs(&mut album);
    album.songs
}

fn genre_counts(cache: &LibraryCache) -> Vec<(String, usize)> {
    let mut genres: Vec<(String, usize)> = cache
        .songs_by_genre
        .iter()
        .filter(|(genre, songs)| !genre.trim().is_empty() && !songs.is_empty())
        .map(|(genre, songs)| (genre.clone(), songs.len()))
        .collect();
    genres.sort_by_key(|left| sort_key(&left.0));
    genres
}

fn genre_song_ids(cache: &LibraryCache, genre: &str) -> Vec<String> {
    let key = cache
        .songs_by_genre
        .keys()
        .find(|candidate| candidate.as_str() == genre)
        .or_else(|| {
            let wanted = normalize(genre);
            cache
                .songs_by_genre
                .keys()
                .find(|candidate| normalize(candidate) == wanted)
        });
    key.and_then(|key| cache.songs_by_genre.get(key))
        .map(|indexes| {
            indexes
                .iter()
                .filter_map(|index| cache.flat_song_id(*index).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn genre_albums<'a>(cache: &'a LibraryCache, genre: &str) -> Vec<&'a Album> {
    let wanted = normalize(genre);
    let mut albums: Vec<&Album> = all_albums(cache)
        .into_iter()
        .filter(|album| {
            cache.album_genres.get(&album.id).is_some_and(|genres| {
                genres
                    .iter()
                    .any(|candidate| candidate == genre || normalize(candidate) == wanted)
            })
        })
        .collect();
    if albums.is_empty() {
        // Track-level genres only: list the albums their songs come from.
        let mut seen = HashSet::new();
        for song_id in genre_song_ids(cache, genre) {
            if let Some((_, album_id)) = cache.song_map.get(&song_id)
                && seen.insert(album_id.clone())
                && let Some(album) = cache.album(album_id)
            {
                albums.push(album);
            }
        }
    }
    albums.sort_by_key(|album| sort_key(&album.name));
    albums
}

fn artist_song_ids(cache: &LibraryCache, artist_id: &str) -> Vec<String> {
    let Some(artist) = cache.artist(artist_id) else {
        return Vec::new();
    };
    let mut albums: Vec<&Album> = artist.albums.iter().collect();
    albums.sort_by(|left, right| {
        left.first_release_date
            .cmp(&right.first_release_date)
            .then_with(|| sort_key(&left.name).cmp(&sort_key(&right.name)))
    });
    let mut ids: Vec<String> = albums
        .into_iter()
        .flat_map(album_songs)
        .map(|song| song.id)
        .collect();
    if ids.is_empty()
        && let Some(indexes) = cache.songs_by_artist.get(artist_id)
    {
        ids = indexes
            .iter()
            .filter_map(|index| cache.flat_song_id(*index).map(str::to_string))
            .collect();
    }
    ids
}

/// Playlists `user_id` owns: (id, name, song ids in order).
fn load_playlists(
    pool: &DbPool,
    user_id: i32,
    only: Option<i32>,
) -> Result<Vec<(i32, String, Vec<String>)>, String> {
    use crate::persistence::schema::_playlist_to_song::dsl as tracks;
    use crate::persistence::schema::_playlist_to_user::dsl as owners;
    use crate::persistence::schema::playlist::dsl as playlists;
    use diesel::prelude::*;

    let mut connection = pool.get().map_err(|error| error.to_string())?;
    let mut query = owners::_playlist_to_user
        .inner_join(playlists::playlist)
        .filter(owners::b.eq(user_id))
        .select((playlists::id, playlists::name))
        .order(playlists::updated_at.desc())
        .limit(500)
        .into_boxed();
    if let Some(id) = only {
        query = query.filter(playlists::id.eq(id));
    }
    let rows: Vec<(i32, String)> = query
        .load(&mut connection)
        .map_err(|error| error.to_string())?;
    let mut result = Vec::with_capacity(rows.len());
    for (id, name) in rows {
        let songs = if only.is_some() {
            tracks::_playlist_to_song
                .filter(tracks::a.eq(id))
                .order(tracks::position.asc())
                .then_order_by(tracks::rowid.asc())
                .select(tracks::b)
                .limit(MAX_ZONE_QUEUE_ITEMS as i64)
                .load::<String>(&mut connection)
                .map_err(|error| error.to_string())?
        } else {
            tracks::_playlist_to_song
                .filter(tracks::a.eq(id))
                .order(tracks::position.asc())
                .then_order_by(tracks::rowid.asc())
                .select(tracks::b)
                .limit(1)
                .load::<String>(&mut connection)
                .map_err(|error| error.to_string())?
        };
        result.push((id, name, songs));
    }
    Ok(result)
}

fn load_liked(pool: &DbPool, user_id: i32) -> Result<Vec<String>, String> {
    use crate::persistence::schema::favorite_song::dsl as favorites;
    use diesel::prelude::*;

    let mut connection = pool.get().map_err(|error| error.to_string())?;
    favorites::favorite_song
        .filter(favorites::user_id.eq(user_id))
        .order(favorites::added_at.desc())
        .select(favorites::song_id)
        .limit(MAX_ZONE_QUEUE_ITEMS as i64)
        .load::<String>(&mut connection)
        .map_err(|error| error.to_string())
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    web::block(work)
        .await
        .map_err(|error| error.to_string())
        .and_then(|result| result)
}

/// Builds one browse response.
pub async fn browse(
    cache: &LibraryCache,
    pool: &DbPool,
    user_id: i32,
    item: &ItemId,
) -> Result<BrowseResponse, ZoneError> {
    let (node, children) = match item {
        ItemId::Root => (
            BrowseItem {
                kind: "library",
                ..directory("root", &crate::settings::library_name(), false)
            },
            vec![
                directory("albums", "Albums", false),
                directory("artists", "Artists", false),
                directory("playlists", "Playlists", false),
                directory("genres", "Genres", false),
                directory("liked", "Liked songs", true),
                BrowseItem {
                    kind: "radio",
                    ..directory("radio", "Radio", false)
                },
            ],
        ),
        ItemId::Albums(letter) => {
            let mut albums: Vec<(String, BrowseItem)> = all_albums(cache)
                .into_iter()
                .map(|album| (album.name.clone(), album_item(cache, album)))
                .collect();
            albums.sort_by(|left, right| {
                sort_key(&left.0)
                    .cmp(&sort_key(&right.0))
                    .then_with(|| left.1.subtitle.cmp(&right.1.subtitle))
            });
            let id = letter.map_or("albums".to_string(), |letter| format!("albums/{letter}"));
            let title = letter.map_or("Albums".to_string(), |letter| format!("Albums: {letter}"));
            (
                directory(&id, &title, false),
                bucketed("albums", albums, *letter),
            )
        }
        ItemId::Artists(letter) => {
            let mut artists: Vec<(String, BrowseItem)> = cache
                .artists
                .iter()
                .filter(|artist| !artist.albums.is_empty())
                .map(|artist| (artist.name.clone(), artist_item(artist)))
                .collect();
            artists.sort_by_key(|left| sort_key(&left.0));
            let id = letter.map_or("artists".to_string(), |letter| format!("artists/{letter}"));
            let title = letter.map_or("Artists".to_string(), |letter| format!("Artists: {letter}"));
            (
                directory(&id, &title, false),
                bucketed("artists", artists, *letter),
            )
        }
        ItemId::Playlists => {
            let pool = pool.clone();
            let playlists = blocking(move || load_playlists(&pool, user_id, None))
                .await
                .map_err(ZoneError::Device)?;
            let children = playlists
                .into_iter()
                .map(|(id, name, first)| BrowseItem {
                    id: format!("playlist:{id}"),
                    title: name,
                    subtitle: None,
                    image: first
                        .first()
                        .and_then(|song_id| cache.song_map.get(song_id))
                        .and_then(|(_, album_id)| cache.album(album_id))
                        .and_then(|album| image(&album.cover_url)),
                    kind: "playlist",
                    playable: true,
                    expandable: true,
                    duration_ms: None,
                    quality: None,
                })
                .collect();
            (directory("playlists", "Playlists", false), children)
        }
        ItemId::Genres => (
            directory("genres", "Genres", false),
            genre_counts(cache)
                .into_iter()
                .map(|(genre, count)| {
                    let mut item = genre_item(&genre, count);
                    item.image = genre_albums(cache, &genre)
                        .first()
                        .and_then(|album| image(&album.cover_url));
                    item
                })
                .collect(),
        ),
        ItemId::Radio => {
            let mut stations = vec![radio_item(
                "radio:library".to_string(),
                "Library radio".to_string(),
                "Endless picks from your listening",
            )];
            let mut genres = genre_counts(cache);
            genres.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
            genres.truncate(RADIO_GENRES);
            genres.sort_by_key(|left| sort_key(&left.0));
            stations.extend(genres.into_iter().map(|(genre, _)| {
                radio_item(
                    format!("radio:genre:{genre}"),
                    format!("{genre} radio"),
                    "Genre station",
                )
            }));
            (
                BrowseItem {
                    kind: "radio",
                    ..directory("radio", "Radio", false)
                },
                stations,
            )
        }
        ItemId::Liked => {
            let pool = pool.clone();
            let liked = blocking(move || load_liked(&pool, user_id))
                .await
                .map_err(ZoneError::Device)?;
            (
                directory("liked", "Liked songs", true),
                liked
                    .iter()
                    .filter_map(|id| cache.song(id))
                    .map(|song| song_item(cache, song))
                    .collect(),
            )
        }
        ItemId::Album(id) => {
            let album = cache
                .album(id)
                .ok_or(ZoneError::Missing("item_not_found"))?;
            (
                album_item(cache, album),
                album_songs(album)
                    .iter()
                    .map(|song| song_item(cache, song))
                    .collect(),
            )
        }
        ItemId::Artist(id) => {
            let artist = cache
                .artist(id)
                .ok_or(ZoneError::Missing("item_not_found"))?;
            let mut albums: Vec<&Album> = artist.albums.iter().collect();
            albums.sort_by(|left, right| {
                right
                    .first_release_date
                    .cmp(&left.first_release_date)
                    .then_with(|| sort_key(&left.name).cmp(&sort_key(&right.name)))
            });
            let mut children: Vec<BrowseItem> = albums
                .into_iter()
                .map(|album| album_item(cache, album))
                .collect();
            children.push(radio_item(
                format!("radio:artist:{}", artist.id),
                format!("{} radio", artist.name),
                "Artist station",
            ));
            (artist_item(artist), children)
        }
        ItemId::Playlist(id) => {
            let pool = pool.clone();
            let playlist_id = *id;
            let mut playlists = blocking(move || load_playlists(&pool, user_id, Some(playlist_id)))
                .await
                .map_err(ZoneError::Device)?;
            let (id, name, songs) = playlists
                .pop()
                .ok_or(ZoneError::Missing("item_not_found"))?;
            let children: Vec<BrowseItem> = songs
                .iter()
                .filter_map(|song_id| cache.song(song_id))
                .map(|song| song_item(cache, song))
                .collect();
            (
                BrowseItem {
                    id: format!("playlist:{id}"),
                    title: name,
                    subtitle: None,
                    image: children.first().and_then(|song| song.image.clone()),
                    kind: "playlist",
                    playable: true,
                    expandable: true,
                    duration_ms: None,
                    quality: None,
                },
                children,
            )
        }
        ItemId::Genre(genre) => {
            let songs = genre_song_ids(cache, genre);
            if songs.is_empty() {
                return Err(ZoneError::Missing("item_not_found"));
            }
            let mut children: Vec<BrowseItem> = genre_albums(cache, genre)
                .into_iter()
                .map(|album| album_item(cache, album))
                .collect();
            children.insert(
                0,
                radio_item(
                    format!("radio:genre:{genre}"),
                    format!("{genre} radio"),
                    "Genre station",
                ),
            );
            (genre_item(genre, songs.len()), children)
        }
        ItemId::Song(id) => {
            let song = cache.song(id).ok_or(ZoneError::Missing("item_not_found"))?;
            (song_item(cache, song), Vec::new())
        }
        ItemId::LibraryRadio | ItemId::GenreRadio(_) => {
            return Err(ZoneError::Invalid("radio_is_not_browsable"));
        }
    };
    let total = children.len();
    let mut children = children;
    children.truncate(MAX_BROWSE_CHILDREN);
    Ok(BrowseResponse {
        item: node,
        children,
        total,
    })
}

/// Library search for controllers: artists, then albums, then songs.
pub fn search(cache: &LibraryCache, query: &str) -> Result<Vec<BrowseItem>, ZoneError> {
    let query = query.trim();
    if query.is_empty() || query.chars().count() > MAX_SEARCH_QUERY_CHARACTERS {
        return Err(ZoneError::Invalid("invalid_search_query"));
    }
    let normalized = normalize(query);
    if normalized.is_empty() {
        return Err(ZoneError::Invalid("invalid_search_query"));
    }
    let hits = cache
        .search_index
        .search(&normalized, 200)
        .unwrap_or_default();
    let (mut artists, mut albums, mut songs) = (Vec::new(), Vec::new(), Vec::new());
    for hit in hits {
        match hit.entity_type.as_str() {
            "artist" if artists.len() < SEARCH_ARTISTS => {
                if let Some(artist) = cache.artist(&hit.entity_id) {
                    artists.push(artist_item(artist));
                }
            }
            "album" if albums.len() < SEARCH_ALBUMS => {
                if let Some(album) = cache.album(&hit.entity_id) {
                    albums.push(album_item(cache, album));
                }
            }
            "song" if songs.len() < SEARCH_SONGS => {
                if let Some(song) = cache.song(&hit.entity_id) {
                    songs.push(song_item(cache, song));
                }
            }
            _ => {}
        }
    }
    let mut results = artists;
    results.extend(albums);
    results.extend(songs);
    Ok(results)
}

/// What to do with a resolved item.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlayAction {
    /// Replace the queue and start playing.
    #[default]
    Play,
    /// Replace the queue with the songs in random order.
    Shuffle,
    /// Play right after the current track.
    Next,
    /// Add to the end of the queue.
    Add,
    /// Start a radio station seeded by the item.
    Radio,
}

#[derive(Debug, Deserialize)]
pub struct PlayRequest {
    /// A browse id, e.g. `album:<id>`.
    pub item: Option<String>,
    /// Free text: the best library match plays (Home Assistant's
    /// "play_media" with a search, voice assistants).
    pub query: Option<String>,
    /// Limits a `query` to one kind: `artist`, `album`, `song`,
    /// `playlist` or `genre`.
    pub kind: Option<String>,
    #[serde(default)]
    pub action: PlayAction,
}

/// Turns a free-text request into a browse id.
async fn resolve_query(
    cache: &LibraryCache,
    pool: &DbPool,
    user_id: i32,
    query: &str,
    kind: Option<&str>,
) -> Result<ItemId, ZoneError> {
    let query = query.trim();
    if query.is_empty() || query.chars().count() > MAX_SEARCH_QUERY_CHARACTERS {
        return Err(ZoneError::Invalid("invalid_search_query"));
    }
    let wanted = normalize(query);
    match kind {
        Some("playlist") => {
            let pool = pool.clone();
            let playlists = blocking(move || load_playlists(&pool, user_id, None))
                .await
                .map_err(ZoneError::Device)?;
            let exact = playlists
                .iter()
                .find(|(_, name, _)| normalize(name) == wanted);
            let partial = || {
                playlists
                    .iter()
                    .find(|(_, name, _)| normalize(name).contains(&wanted))
            };
            return exact
                .or_else(partial)
                .map(|(id, _, _)| ItemId::Playlist(*id))
                .ok_or(ZoneError::Missing("no_library_match"));
        }
        Some("genre") => {
            let genres = genre_counts(cache);
            let exact = genres.iter().find(|(genre, _)| normalize(genre) == wanted);
            let partial = || {
                genres
                    .iter()
                    .find(|(genre, _)| normalize(genre).contains(&wanted))
            };
            return exact
                .or_else(partial)
                .map(|(genre, _)| ItemId::Genre(genre.clone()))
                .ok_or(ZoneError::Missing("no_library_match"));
        }
        Some("artist" | "album" | "song") | None => {}
        Some(_) => return Err(ZoneError::Invalid("invalid_search_kind")),
    }
    let hits = cache.search_index.search(&wanted, 50).unwrap_or_default();
    hits.into_iter()
        .filter(|hit| kind.is_none_or(|kind| hit.entity_type == kind))
        .find_map(|hit| match hit.entity_type.as_str() {
            "artist" => cache
                .artist(&hit.entity_id)
                .map(|artist| ItemId::Artist(artist.id.clone())),
            "album" => cache
                .album(&hit.entity_id)
                .map(|album| ItemId::Album(album.id.clone())),
            "song" => cache
                .song(&hit.entity_id)
                .map(|song| ItemId::Song(song.id.clone())),
            _ => None,
        })
        .ok_or(ZoneError::Missing("no_library_match"))
}

/// The songs an item stands for, in play order, and its title.
async fn songs_for(
    cache: &LibraryCache,
    pool: &DbPool,
    user_id: i32,
    item: &ItemId,
) -> Result<(Vec<String>, String), ZoneError> {
    let (songs, title) = match item {
        ItemId::Album(id) => {
            let album = cache
                .album(id)
                .ok_or(ZoneError::Missing("item_not_found"))?;
            (
                album_songs(album).into_iter().map(|song| song.id).collect(),
                album.name.clone(),
            )
        }
        ItemId::Artist(id) => {
            let artist = cache
                .artist(id)
                .ok_or(ZoneError::Missing("item_not_found"))?;
            (artist_song_ids(cache, id), artist.name.clone())
        }
        ItemId::Song(id) => {
            let song = cache.song(id).ok_or(ZoneError::Missing("item_not_found"))?;
            (vec![song.id.clone()], song.name.clone())
        }
        ItemId::Genre(genre) => (genre_song_ids(cache, genre), genre.clone()),
        ItemId::Playlist(id) => {
            let pool = pool.clone();
            let playlist_id = *id;
            let mut playlists = blocking(move || load_playlists(&pool, user_id, Some(playlist_id)))
                .await
                .map_err(ZoneError::Device)?;
            let (_, name, songs) = playlists
                .pop()
                .ok_or(ZoneError::Missing("item_not_found"))?;
            (songs, name)
        }
        ItemId::Liked => {
            let pool = pool.clone();
            (
                blocking(move || load_liked(&pool, user_id))
                    .await
                    .map_err(ZoneError::Device)?,
                "Liked songs".to_string(),
            )
        }
        _ => return Err(ZoneError::Invalid("item_is_not_playable")),
    };
    let mut seen = HashSet::new();
    let songs: Vec<String> = songs
        .into_iter()
        .filter(|id| cache.song(id).is_some() && seen.insert(id.clone()))
        .take(MAX_ZONE_QUEUE_ITEMS)
        .collect();
    if songs.is_empty() {
        return Err(ZoneError::Invalid("item_has_no_songs"));
    }
    Ok((songs, title))
}

/// A radio queue: the seed (when there is one), recommendations around it,
/// then songs that share its genre or artist, then the rest of the library,
/// so a station never comes back empty on a fresh profile.
async fn radio_queue(
    cache: &std::sync::Arc<LibraryCache>,
    pool: &DbPool,
    user_id: i32,
    item: &ItemId,
) -> Result<(Vec<String>, String), ZoneError> {
    let (seed, mut related, title): (Option<String>, Vec<String>, String) = match item {
        ItemId::LibraryRadio | ItemId::Radio | ItemId::Root => {
            (None, Vec::new(), "Library radio".to_string())
        }
        ItemId::GenreRadio(genre) | ItemId::Genre(genre) => {
            let mut songs = genre_song_ids(cache, genre);
            if songs.is_empty() {
                return Err(ZoneError::Missing("item_not_found"));
            }
            songs.shuffle(&mut rand::rng());
            let seed = songs.first().cloned();
            (seed, songs, format!("{genre} radio"))
        }
        ItemId::Artist(id) => {
            let artist = cache
                .artist(id)
                .ok_or(ZoneError::Missing("item_not_found"))?;
            let mut songs = artist_song_ids(cache, id);
            songs.shuffle(&mut rand::rng());
            (
                songs.first().cloned(),
                songs,
                format!("{} radio", artist.name),
            )
        }
        other => {
            let (songs, title) = songs_for(cache, pool, user_id, other).await?;
            let seed = if matches!(other, ItemId::Song(_)) {
                songs.first().cloned()
            } else {
                songs.choose(&mut rand::rng()).cloned()
            };
            (seed, songs, format!("{title} radio"))
        }
    };
    let recommendation_cache = cache.clone();
    let recommendation_pool = pool.clone();
    let recommendation_seed = seed.clone();
    let ranked = web::block(move || {
        crate::recommendation::recommend(
            user_id,
            recommendation_seed.as_deref(),
            &recommendation_cache,
            &recommendation_pool,
            RADIO_ITEMS,
        )
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| ZoneError::Device(error.to_string()))?
    .unwrap_or_else(|error| {
        tracing::warn!(%error, "radio recommendations unavailable");
        Vec::new()
    });

    let mut rng = rand::rng();
    let mut queue = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |id: String, queue: &mut Vec<String>| {
        if queue.len() <= RADIO_ITEMS && cache.song(&id).is_some() && seen.insert(id.clone()) {
            queue.push(id);
        }
    };
    if let Some(seed) = seed.clone() {
        push(seed, &mut queue);
    }
    for candidate in ranked {
        push(candidate.song_id, &mut queue);
    }
    // Same genre or artist as the seed before anything else.
    if let Some(seed) = seed.as_deref()
        && let Some((artist_id, album_id)) = cache.song_map.get(seed)
    {
        for genre in cache.album_genres.get(album_id).into_iter().flatten() {
            let mut songs = genre_song_ids(cache, genre);
            songs.shuffle(&mut rng);
            related.extend(songs);
        }
        let mut by_artist = artist_song_ids(cache, artist_id);
        by_artist.shuffle(&mut rng);
        related.extend(by_artist);
    }
    for id in related {
        push(id, &mut queue);
    }
    if queue.len() < RADIO_ITEMS {
        let mut library = cache.songs_flat.clone();
        library.shuffle(&mut rng);
        for id in library {
            push(id, &mut queue);
        }
    }
    if queue.is_empty() {
        return Err(ZoneError::Invalid("item_has_no_songs"));
    }
    Ok((queue, title))
}

#[derive(Debug, Serialize)]
pub struct PlayResult {
    pub zone_id: String,
    pub item_id: String,
    pub title: String,
    pub action: &'static str,
    pub songs: usize,
}

fn item_id_string(item: &ItemId) -> String {
    match item {
        ItemId::Root => "root".into(),
        ItemId::Albums(_) => "albums".into(),
        ItemId::Artists(_) => "artists".into(),
        ItemId::Playlists => "playlists".into(),
        ItemId::Genres => "genres".into(),
        ItemId::Radio => "radio".into(),
        ItemId::Liked => "liked".into(),
        ItemId::Album(id) => format!("album:{id}"),
        ItemId::Artist(id) => format!("artist:{id}"),
        ItemId::Playlist(id) => format!("playlist:{id}"),
        ItemId::Genre(genre) => format!("genre:{genre}"),
        ItemId::Song(id) => format!("song:{id}"),
        ItemId::LibraryRadio => "radio:library".into(),
        ItemId::GenreRadio(genre) => format!("radio:genre:{genre}"),
    }
}

/// `radio:artist:<id>` is offered on artist pages; it plays as artist radio.
fn parse_play_item(value: &str) -> Option<(ItemId, bool)> {
    if let Some(id) = value.trim().strip_prefix("radio:artist:") {
        return non_empty(id).map(|id| (ItemId::Artist(id), true));
    }
    let item = ItemId::parse(value)?;
    let radio = matches!(item, ItemId::LibraryRadio | ItemId::GenreRadio(_));
    Some((item, radio))
}

#[allow(clippy::too_many_arguments)]
async fn play_in_zone(
    user_id: i32,
    zone_id: &str,
    request: PlayRequest,
    service: &ZoneService,
    pool: &DbPool,
    lifecycle: &LibraryLifecycle,
    cache: &std::sync::Arc<LibraryCache>,
    force_radio: bool,
) -> Result<PlayResult, ZoneError> {
    let (item, radio_item) = match (&request.item, &request.query) {
        (Some(item), _) => parse_play_item(item).ok_or(ZoneError::Invalid("invalid_item_id"))?,
        (None, Some(query)) => (
            resolve_query(cache, pool, user_id, query, request.kind.as_deref()).await?,
            false,
        ),
        (None, None) if force_radio => (ItemId::LibraryRadio, true),
        (None, None) => return Err(ZoneError::Invalid("item_or_query_required")),
    };
    let action = if force_radio || radio_item {
        PlayAction::Radio
    } else {
        request.action
    };
    let (songs, title) = match action {
        PlayAction::Radio => radio_queue(cache, pool, user_id, &item).await?,
        _ => songs_for(cache, pool, user_id, &item).await?,
    };
    let count = songs.len();
    let (command, action_name) = match action {
        PlayAction::Play | PlayAction::Radio => (
            ZoneCommand::Load {
                song_ids: songs,
                current_position: 0,
                position_ms: 0,
                play: true,
            },
            if action == PlayAction::Radio {
                "radio"
            } else {
                "play"
            },
        ),
        PlayAction::Shuffle => {
            let mut songs = songs;
            songs.shuffle(&mut rand::rng());
            (
                ZoneCommand::Load {
                    song_ids: songs,
                    current_position: 0,
                    position_ms: 0,
                    play: true,
                },
                "shuffle",
            )
        }
        PlayAction::Next => (
            ZoneCommand::AddToQueue {
                song_ids: songs,
                next: true,
            },
            "next",
        ),
        PlayAction::Add => (
            ZoneCommand::AddToQueue {
                song_ids: songs,
                next: false,
            },
            "add",
        ),
    };
    command.validate().map_err(ZoneError::Invalid)?;
    dispatch_command(user_id, zone_id, command, service, pool, lifecycle).await?;
    Ok(PlayResult {
        zone_id: zone_id.to_string(),
        item_id: item_id_string(&item),
        title,
        action: action_name,
        songs: count,
    })
}

#[derive(Deserialize)]
struct BrowseQuery {
    item: Option<String>,
    query: Option<String>,
}

/// `GET /browse?item=<id>` walks the library; `GET /browse?query=<text>`
/// searches it.
#[get("")]
async fn browse_library(
    request: HttpRequest,
    query: web::Query<BrowseQuery>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    let Some(cache) = library_cache(&lifecycle).await else {
        return error_response(ZoneError::Unsupported("library_unavailable"));
    };
    if let Some(text) = query.query.as_deref() {
        return match search(&cache, text) {
            Ok(children) => HttpResponse::Ok().json(BrowseResponse {
                item: directory("search", &format!("Results for “{}”", text.trim()), false),
                total: children.len(),
                children,
            }),
            Err(error) => error_response(error),
        };
    }
    let Some(item) = ItemId::parse(query.item.as_deref().unwrap_or("root")) else {
        return error_response(ZoneError::Invalid("invalid_item_id"));
    };
    match browse(&cache, pool.get_ref(), user_id, &item).await {
        Ok(response) => HttpResponse::Ok().json(response),
        Err(error) => error_response(error),
    }
}

async fn play_route(
    request: HttpRequest,
    zone_id: String,
    form: PlayRequest,
    service: web::Data<ZoneService>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
    force_radio: bool,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    if !service.visible_to(&zone_id, viewer_device(&request).view()) {
        return error_response(ZoneError::NotFound);
    }
    let Some(cache) = library_cache(&lifecycle).await else {
        return error_response(ZoneError::Unsupported("library_unavailable"));
    };
    match play_in_zone(
        user_id,
        &zone_id,
        form,
        &service,
        pool.get_ref(),
        &lifecycle,
        &cache,
        force_radio,
    )
    .await
    {
        Ok(result) => HttpResponse::Ok().json(result),
        Err(error) => error_response(error),
    }
}

/// `POST /zones/{zone_id}/play`: plays a browse item or the best search
/// match in a zone.
#[post("/{zone_id}/play")]
pub(crate) async fn play_item(
    request: HttpRequest,
    zone_id: web::Path<String>,
    form: web::Json<PlayRequest>,
    service: web::Data<ZoneService>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    play_route(
        request,
        zone_id.into_inner(),
        form.into_inner(),
        service,
        pool,
        lifecycle,
        false,
    )
    .await
}

/// `POST /zones/{zone_id}/radio`: starts a station seeded by an item (or
/// library radio when no item is named).
#[post("/{zone_id}/radio")]
pub(crate) async fn start_radio(
    request: HttpRequest,
    zone_id: web::Path<String>,
    form: Option<web::Json<PlayRequest>>,
    service: web::Data<ZoneService>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let form = form.map(web::Json::into_inner).unwrap_or(PlayRequest {
        item: None,
        query: None,
        kind: None,
        action: PlayAction::Radio,
    });
    play_route(
        request,
        zone_id.into_inner(),
        form,
        service,
        pool,
        lifecycle,
        true,
    )
    .await
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Rating {
    Up,
    Down,
}

#[derive(Debug, Deserialize)]
pub struct RatingRequest {
    pub rating: Rating,
    /// Defaults to the zone's current track.
    pub song_id: Option<String>,
}

/// `POST /zones/{zone_id}/rating`: thumbs up or down. Thumbs up teaches
/// radio to play more like it; thumbs down teaches it to stay away and,
/// for the playing track, skips it at once.
#[post("/{zone_id}/rating")]
pub(crate) async fn rate_track(
    request: HttpRequest,
    zone_id: web::Path<String>,
    form: web::Json<RatingRequest>,
    service: web::Data<ZoneService>,
    pool: web::Data<DbPool>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    let viewer = viewer_device(&request);
    if !service.visible_to(&zone_id, viewer.view()) {
        return error_response(ZoneError::NotFound);
    }
    let Some(cache) = library_cache(&lifecycle).await else {
        return error_response(ZoneError::Unsupported("library_unavailable"));
    };
    let zones = crate::api::zones::all_zones(
        &service,
        pool.get_ref(),
        user_id,
        viewer.view(),
        Some(&cache),
    )
    .await;
    let Some(zone) = zones.iter().find(|zone| zone.id == *zone_id) else {
        return error_response(ZoneError::NotFound);
    };
    let current = zone
        .now_playing
        .as_ref()
        .and_then(|now| now.song_id.clone());
    let form = form.into_inner();
    let Some(song_id) = form.song_id.clone().or(current.clone()) else {
        return error_response(ZoneError::Invalid("nothing_playing"));
    };
    if cache.song(&song_id).is_none() {
        return error_response(ZoneError::Invalid("invalid_song_id"));
    }
    let event = PlaybackEventRequest {
        event_key: format!("rating:{}", uuid::Uuid::new_v4()),
        song_id: song_id.clone(),
        event_type: match form.rating {
            Rating::Up => "thumbs_up",
            Rating::Down => "disliked",
        }
        .to_string(),
        session_id: None,
        queue_id: None,
        source: Some("zone".to_string()),
        position_seconds: Some(zone.position_ms.max(0) as f64 / 1000.0),
        duration_seconds: Some(zone.duration_ms.max(0) as f64 / 1000.0),
    };
    let event_cache = cache.clone();
    let event_pool = pool.get_ref().clone();
    let recorded = web::block(move || {
        record_playback_event(user_id, &event, &event_cache, &event_pool)
            .map_err(|error| error.to_string())
    })
    .await;
    if !matches!(recorded, Ok(Ok(_))) {
        tracing::warn!(?recorded, "zone rating not recorded");
        return error_response(ZoneError::Device("rating_not_recorded".to_string()));
    }
    let skip = form.rating == Rating::Down
        && current.as_deref() == Some(song_id.as_str())
        && zone.capabilities.transport;
    let skipped = skip
        && dispatch_command(
            user_id,
            &zone_id,
            ZoneCommand::Next,
            &service,
            pool.get_ref(),
            &lifecycle,
        )
        .await
        .is_ok();
    HttpResponse::Ok().json(serde_json::json!({
        "zone_id": zone_id.as_str(),
        "song_id": song_id,
        "rating": match form.rating { Rating::Up => "up", Rating::Down => "down" },
        "skipped": skipped,
    }))
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/browse").service(browse_library));
}

#[cfg(test)]
mod tests;
