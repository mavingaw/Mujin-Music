//! Albumizer: finds loose tracks (each alone on an album of one) and
//! proposes albums for them by folder, the way they were usually ripped or
//! downloaded together.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

use crate::domain::Artist;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct LooseTrack {
    pub song_id: String,
    pub title: String,
    pub artist: String,
    pub album_id: String,
    pub album: String,
    pub track_number: u16,
    pub duration: f64,
    pub cover_url: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct AlbumSuggestion {
    /// Folder the tracks share.
    pub folder: String,
    /// Proposed album title (the folder name, without "Artist - " prefixes).
    pub title: String,
    /// Proposed album artist: the artist most tracks share.
    pub artist: String,
    pub tracks: Vec<LooseTrack>,
}

/// Album title proposed from a folder name: "Artist - Title (2019)" gives
/// "Title".
pub fn title_from_folder(folder: &str, artist: &str) -> String {
    let name = Path::new(folder)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(folder)
        .trim();
    let mut title = name.to_string();
    for separator in [" - ", " – ", " — "] {
        if let Some((left, right)) = title.split_once(separator)
            && (left.trim().eq_ignore_ascii_case(artist.trim())
                || left.trim().chars().all(|c| c.is_ascii_digit()))
            && !right.trim().is_empty()
        {
            title = right.trim().to_string();
        }
    }
    // A trailing "(2019)" or "[2019]" year.
    let chars: Vec<char> = title.trim_end().chars().collect();
    if chars.len() > 7 {
        let tail = &chars[chars.len() - 6..];
        if matches!((tail[0], tail[5]), ('(', ')') | ('[', ']'))
            && tail[1..5].iter().all(char::is_ascii_digit)
        {
            title = chars[..chars.len() - 6]
                .iter()
                .collect::<String>()
                .trim_end()
                .to_string();
        }
    }
    if title.is_empty() {
        name.to_string()
    } else {
        title
    }
}

/// Loose tracks grouped by folder, largest groups first.
pub fn suggestions(library: &[Artist]) -> Vec<AlbumSuggestion> {
    let mut by_folder: BTreeMap<String, Vec<LooseTrack>> = BTreeMap::new();
    for artist in library {
        for album in &artist.albums {
            if album.songs.len() != 1 {
                continue;
            }
            let song = &album.songs[0];
            let folder = Path::new(&song.path)
                .parent()
                .map(|parent| parent.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            by_folder.entry(folder).or_default().push(LooseTrack {
                song_id: song.id.clone(),
                title: song.name.clone(),
                artist: song.artist.clone(),
                album_id: album.id.clone(),
                album: album.name.clone(),
                track_number: song.track_number,
                duration: song.duration,
                cover_url: album.cover_url.clone(),
            });
        }
    }
    let mut result: Vec<AlbumSuggestion> = by_folder
        .into_iter()
        .map(|(folder, mut tracks)| {
            tracks.sort_by(|a, b| {
                a.track_number
                    .cmp(&b.track_number)
                    .then(a.title.cmp(&b.title))
            });
            let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
            for track in &tracks {
                *counts.entry(track.artist.as_str()).or_default() += 1;
            }
            let artist = if counts.len() > 2 && tracks.len() >= 4 {
                "Various Artists".to_string()
            } else {
                counts
                    .iter()
                    .max_by_key(|(_, count)| **count)
                    .map(|(artist, _)| artist.to_string())
                    .unwrap_or_default()
            };
            AlbumSuggestion {
                title: title_from_folder(&folder, &artist),
                folder,
                artist,
                tracks,
            }
        })
        .collect();
    result.sort_by(|a, b| {
        b.tracks
            .len()
            .cmp(&a.tracks.len())
            .then(a.folder.cmp(&b.folder))
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Album, Song};

    #[test]
    fn folder_names_become_album_titles() {
        assert_eq!(
            title_from_folder("/m/Example Artist - Night Drive (2019)", "Example Artist"),
            "Night Drive"
        );
        assert_eq!(title_from_folder("/m/03 - Demos", "Someone"), "Demos");
        assert_eq!(title_from_folder("/m/Bootlegs [1998]", "x"), "Bootlegs");
        assert_eq!(title_from_folder("/m/Singles", "x"), "Singles");
    }

    #[test]
    fn loose_tracks_are_grouped_by_folder() {
        let loose = |id: &str, path: &str, number: u16| Album {
            id: format!("album-{id}"),
            name: format!("Loose {id}"),
            songs: vec![Song {
                id: id.into(),
                name: format!("Track {id}"),
                artist: "Example Artist".into(),
                path: path.into(),
                track_number: number,
                ..Song::default()
            }],
            ..Album::default()
        };
        let mut full = loose("full", "/m/Full/01.flac", 1);
        full.songs.push(Song {
            id: "full-2".into(),
            path: "/m/Full/02.flac".into(),
            ..Song::default()
        });
        let library = vec![Artist {
            id: "artist".into(),
            name: "Example Artist".into(),
            albums: vec![
                loose("b", "/m/Example Artist - Demos/02.mp3", 2),
                loose("a", "/m/Example Artist - Demos/01.mp3", 1),
                loose("c", "/m/Other/x.mp3", 0),
                full,
            ],
            ..Artist::default()
        }];
        let found = suggestions(&library);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].title, "Demos");
        assert_eq!(found[0].artist, "Example Artist");
        assert_eq!(
            found[0]
                .tracks
                .iter()
                .map(|t| t.song_id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
    }
}
