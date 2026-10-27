//! The library-tools layer over the catalog: which copy of a track plays,
//! albums made with the Albumizer, hidden or merged copies and hidden album
//! versions. Applied every time the catalog is built, so rescans keep the
//! user's choices and nothing is ever written to the music files.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use diesel::QueryResult;
use diesel::sqlite::SqliteConnection;

use super::duplicates::TrackFacts;
use super::quality::{Quality, grade};
use super::store::{self, UserAlbumRow, UserAlbumTrackRow};
use super::versions::{AlbumFacts, VersionChoice, group_versions};
use crate::domain::{Album, Artist, AudioProperties, Song};
use crate::library::identity::{hash_artist, normalize_artist_identity};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PreferredFile {
    pub path: String,
    pub audio: Option<AudioProperties>,
}

#[derive(Clone, Debug, Default)]
pub struct ToolLayers {
    /// Files hidden or merged into another copy.
    pub hidden_paths: HashSet<String>,
    /// The copy that plays, by track id.
    pub preferred_files: HashMap<String, PreferredFile>,
    pub user_albums: Vec<(UserAlbumRow, Vec<UserAlbumTrackRow>)>,
    pub version_members: HashMap<String, Option<String>>,
    pub version_choices: HashMap<String, VersionChoice>,
    pub release_groups: HashMap<String, String>,
}

impl ToolLayers {
    pub fn load(connection: &mut SqliteConnection) -> QueryResult<Self> {
        let hidden_paths = store::visibility_rows(connection)?
            .into_iter()
            .map(|row| row.path)
            .collect();
        let mut preferred_files = HashMap::new();
        for preference in store::file_preferences(connection)? {
            if let Some(copy) = store::track_copies(connection, &preference.track_id)?
                .into_iter()
                .find(|copy| copy.path == preference.path)
            {
                preferred_files.insert(
                    preference.track_id,
                    PreferredFile {
                        audio: copy.audio(),
                        path: copy.path,
                    },
                );
            }
        }
        let (albums, tracks) = store::user_albums(connection)?;
        let mut tracks_by_album: HashMap<String, Vec<UserAlbumTrackRow>> = HashMap::new();
        for track in tracks {
            tracks_by_album
                .entry(track.album_id.clone())
                .or_default()
                .push(track);
        }
        let user_albums = albums
            .into_iter()
            .map(|album| {
                let tracks = tracks_by_album.remove(&album.id).unwrap_or_default();
                (album, tracks)
            })
            .collect();
        Ok(Self {
            hidden_paths,
            preferred_files,
            user_albums,
            version_members: store::version_members(connection)?,
            version_choices: store::version_choices(connection)?,
            release_groups: store::release_groups(connection)?,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.hidden_paths.is_empty()
            && self.preferred_files.is_empty()
            && self.user_albums.is_empty()
            && !self
                .version_choices
                .values()
                .any(|choice| choice.hide_others)
    }
}

fn extension(path: &str) -> String {
    Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Quality of the file a song plays.
pub fn song_quality(song: &Song) -> Quality {
    grade(song.audio.as_ref(), &extension(&song.path))
}

/// Quality most of an album's tracks have, and whether they differ.
pub fn album_quality(album: &Album) -> (Quality, bool) {
    let mut counts: HashMap<String, (usize, Quality)> = HashMap::new();
    for song in &album.songs {
        let quality = song_quality(song);
        counts
            .entry(quality.label.clone())
            .or_insert((0, quality))
            .0 += 1;
    }
    let mixed = counts.len() > 1;
    let quality = counts
        .into_values()
        .max_by(|a, b| a.0.cmp(&b.0).then(a.1.score.cmp(&b.1.score)))
        .map(|(_, quality)| quality)
        .unwrap_or_else(|| grade(None, ""));
    (quality, mixed)
}

pub fn year_of(date: &str) -> String {
    date.chars().take(4).filter(char::is_ascii_digit).collect()
}

/// Album facts for version grouping.
pub fn album_facts(
    library: &[Artist],
    release_groups: &HashMap<String, String>,
) -> Vec<AlbumFacts> {
    library
        .iter()
        .flat_map(|artist| {
            artist.albums.iter().map(move |album| {
                let (quality, mixed) = album_quality(album);
                AlbumFacts {
                    album_id: album.id.clone(),
                    title: album.name.clone(),
                    artist_id: artist.id.clone(),
                    artist_name: artist.name.clone(),
                    year: year_of(&album.first_release_date),
                    release_group_id: release_groups.get(&album.id).cloned(),
                    track_count: album.songs.len(),
                    duration: album.songs.iter().map(|song| song.duration).sum(),
                    quality,
                    mixed,
                }
            })
        })
        .collect()
}

/// Track facts for duplicate detection.
pub fn track_facts(library: &[Artist]) -> Vec<TrackFacts> {
    library
        .iter()
        .flat_map(|artist| artist.albums.iter())
        .flat_map(|album| {
            album.songs.iter().map(move |song| TrackFacts {
                song_id: song.id.clone(),
                path: song.path.clone(),
                title: song.name.clone(),
                artist: song.artist.clone(),
                album_id: album.id.clone(),
                album_title: album.name.clone(),
                album_track_count: album.songs.len(),
                duration: song.duration,
                isrc: song.isrc.clone(),
                quality: song_quality(song),
            })
        })
        .collect()
}

/// Applies the layer. Returns the albums hidden as non-preferred versions.
pub fn apply(library: &mut Vec<Artist>, layers: &ToolLayers) -> HashSet<String> {
    if layers.is_empty() {
        return HashSet::new();
    }
    let had_albums: HashSet<String> = library
        .iter()
        .filter(|artist| !artist.albums.is_empty())
        .map(|artist| artist.id.clone())
        .collect();

    // 1. The copy that plays.
    if !layers.preferred_files.is_empty() {
        for song in library
            .iter_mut()
            .flat_map(|artist| artist.albums.iter_mut())
            .flat_map(|album| album.songs.iter_mut())
        {
            if let Some(preferred) = layers.preferred_files.get(&song.id) {
                song.path = preferred.path.clone();
                song.audio = preferred.audio.clone();
            }
        }
    }

    // 2. Albumizer: move tracks into the albums the user made.
    if !layers.user_albums.is_empty() {
        let mut placement: HashMap<&str, (&str, i32, i32)> = HashMap::new();
        for (album, tracks) in &layers.user_albums {
            for track in tracks {
                placement.insert(
                    track.path.as_str(),
                    (album.id.as_str(), track.disc, track.position),
                );
            }
        }
        let mut gathered: HashMap<String, Vec<(i32, i32, Song, String)>> = HashMap::new();
        for album in library
            .iter_mut()
            .flat_map(|artist| artist.albums.iter_mut())
        {
            let cover = album.cover_url.clone();
            album
                .songs
                .retain(|song| match placement.get(song.path.as_str()) {
                    Some((user_album, disc, position)) => {
                        gathered
                            .entry((*user_album).to_string())
                            .or_default()
                            .push((*disc, *position, song.clone(), cover.clone()));
                        false
                    }
                    None => true,
                });
        }
        for (row, _) in &layers.user_albums {
            let Some(mut songs) = gathered.remove(&row.id) else {
                continue;
            };
            songs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
            let cover_url = if !row.cover_path.is_empty() {
                row.cover_path.clone()
            } else {
                songs
                    .iter()
                    .map(|(_, _, _, cover)| cover.clone())
                    .find(|cover| !cover.is_empty())
                    .unwrap_or_default()
            };
            let songs: Vec<Song> = songs
                .into_iter()
                .enumerate()
                .map(|(index, (_, _, mut song, _))| {
                    song.track_number = u16::try_from(index + 1).unwrap_or(u16::MAX);
                    song
                })
                .collect();
            let album = Album {
                id: row.id.clone(),
                name: row.title.clone(),
                cover_url,
                songs,
                first_release_date: row.year.clone(),
                primary_type: row.primary_type.clone(),
                ..Album::default()
            };
            let by_id = row
                .artist_id
                .as_deref()
                .and_then(|id| library.iter().position(|artist| artist.id == id));
            let by_name = || {
                let wanted = normalize_artist_identity(&row.artist_name);
                library
                    .iter()
                    .position(|artist| normalize_artist_identity(&artist.name) == wanted)
            };
            match by_id.or_else(by_name) {
                Some(index) => library[index].albums.push(album),
                None => library.push(Artist {
                    id: hash_artist(&row.artist_name),
                    name: row.artist_name.clone(),
                    albums: vec![album],
                    ..Artist::default()
                }),
            }
        }
    }

    // 3. Hidden and merged copies.
    if !layers.hidden_paths.is_empty() {
        for album in library
            .iter_mut()
            .flat_map(|artist| artist.albums.iter_mut())
        {
            album
                .songs
                .retain(|song| !layers.hidden_paths.contains(&song.path));
        }
    }
    for artist in library.iter_mut() {
        artist.albums.retain(|album| !album.songs.is_empty());
    }

    // 4. Versions hidden behind the preferred one.
    let mut hidden_albums = HashSet::new();
    if layers
        .version_choices
        .values()
        .any(|choice| choice.hide_others)
    {
        let facts = album_facts(library, &layers.release_groups);
        for group in group_versions(&facts, &layers.version_members, &layers.version_choices) {
            if group.hide_others {
                hidden_albums.extend(
                    group
                        .versions
                        .iter()
                        .filter(|version| !version.preferred)
                        .map(|version| version.album_id.clone()),
                );
            }
        }
        for artist in library.iter_mut() {
            artist
                .albums
                .retain(|album| !hidden_albums.contains(&album.id));
        }
    }

    library.retain(|artist| !artist.albums.is_empty() || !had_albums.contains(&artist.id));
    hidden_albums
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: &str, path: &str, number: u16) -> Song {
        Song {
            id: id.into(),
            name: format!("Song {id}"),
            artist: "Example Artist".into(),
            path: path.into(),
            track_number: number,
            duration: 200.0,
            ..Song::default()
        }
    }

    fn library() -> Vec<Artist> {
        vec![Artist {
            id: "artist".into(),
            name: "Example Artist".into(),
            albums: vec![
                Album {
                    id: "album".into(),
                    name: "Fixture Album".into(),
                    cover_url: "/m/album/cover.jpg".into(),
                    songs: vec![
                        song("a", "/m/album/a.flac", 1),
                        song("b", "/m/album/b.flac", 2),
                    ],
                    ..Album::default()
                },
                Album {
                    id: "loose-1".into(),
                    name: "Loose 1".into(),
                    cover_url: "/m/loose/one.jpg".into(),
                    songs: vec![song("c", "/m/loose/c.mp3", 1)],
                    ..Album::default()
                },
                Album {
                    id: "loose-2".into(),
                    name: "Loose 2".into(),
                    songs: vec![song("d", "/m/loose/d.mp3", 1)],
                    ..Album::default()
                },
            ],
            ..Artist::default()
        }]
    }

    #[test]
    fn albumizer_groups_loose_tracks_in_order_and_hidden_copies_leave() {
        let mut catalog = library();
        let layers = ToolLayers {
            hidden_paths: HashSet::from(["/m/album/b.flac".to_string()]),
            user_albums: vec![(
                UserAlbumRow {
                    id: "ua-1".into(),
                    title: "Made Album".into(),
                    artist_id: None,
                    artist_name: "Another Artist".into(),
                    year: "2024".into(),
                    primary_type: "ep".into(),
                    cover_path: String::new(),
                },
                vec![
                    UserAlbumTrackRow {
                        path: "/m/loose/d.mp3".into(),
                        album_id: "ua-1".into(),
                        position: 1,
                        disc: 1,
                    },
                    UserAlbumTrackRow {
                        path: "/m/loose/c.mp3".into(),
                        album_id: "ua-1".into(),
                        position: 2,
                        disc: 1,
                    },
                ],
            )],
            ..ToolLayers::default()
        };
        apply(&mut catalog, &layers);
        assert_eq!(catalog.len(), 2);
        let original = &catalog[0];
        assert_eq!(original.albums.len(), 1, "emptied loose albums are gone");
        assert_eq!(original.albums[0].songs.len(), 1, "the hidden copy left");
        let made = &catalog[1];
        assert_eq!(made.name, "Another Artist");
        let album = &made.albums[0];
        assert_eq!(album.id, "ua-1");
        assert_eq!(album.cover_url, "/m/loose/one.jpg");
        assert_eq!(
            album
                .songs
                .iter()
                .map(|song| (song.id.as_str(), song.track_number))
                .collect::<Vec<_>>(),
            vec![("d", 1), ("c", 2)]
        );
    }

    #[test]
    fn preferred_copies_play_and_hidden_versions_leave() {
        let mut catalog = library();
        catalog[0].albums[1].name = "Fixture Album (2019 Remaster)".into();
        let facts = album_facts(&catalog, &HashMap::new());
        let group = group_versions(&facts, &HashMap::new(), &HashMap::new());
        assert_eq!(group.len(), 1);
        let layers = ToolLayers {
            preferred_files: HashMap::from([(
                "a".to_string(),
                PreferredFile {
                    path: "/m/album/a.wav".into(),
                    audio: None,
                },
            )]),
            version_choices: HashMap::from([(
                group[0].key.clone(),
                VersionChoice {
                    preferred_album_id: "album".into(),
                    hide_others: true,
                },
            )]),
            ..ToolLayers::default()
        };
        let hidden = apply(&mut catalog, &layers);
        assert_eq!(hidden, HashSet::from(["loose-1".to_string()]));
        let albums: Vec<&str> = catalog[0].albums.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(albums, vec!["album", "loose-2"]);
        assert_eq!(catalog[0].albums[0].songs[0].path, "/m/album/a.wav");
    }
}
