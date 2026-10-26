//! Deezer public API: public playlists and a profile's public playlists.
//! No sign-in, no audio.

use serde::Deserialize;

use super::http::{ProviderError, send_json};
use super::{ImportedPlaylist, ImportedTrack, MAX_IMPORT_TRACKS, RemotePlaylist};
use crate::library::matching::SourceTrack;

pub const API_BASE: &str = "https://api.deezer.com";

/// Deezer reports some errors with HTTP 200 and an `error` object.
#[derive(Deserialize)]
struct DeezerError {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
struct PlaylistResponse {
    error: Option<DeezerError>,
    title: Option<String>,
    link: Option<String>,
}

#[derive(Deserialize)]
struct TracksPage {
    error: Option<DeezerError>,
    #[serde(default)]
    data: Vec<DeezerTrack>,
    next: Option<String>,
}

#[derive(Deserialize)]
struct DeezerTrack {
    id: Option<i64>,
    title: String,
    #[serde(default)]
    title_version: Option<String>,
    duration: Option<u64>,
    link: Option<String>,
    isrc: Option<String>,
    artist: Option<Named>,
    album: Option<AlbumRef>,
}

#[derive(Deserialize)]
struct Named {
    name: String,
}

#[derive(Deserialize)]
struct AlbumRef {
    title: String,
}

#[derive(Deserialize)]
struct PlaylistsPage {
    error: Option<DeezerError>,
    #[serde(default)]
    data: Vec<DeezerPlaylist>,
    next: Option<String>,
}

#[derive(Deserialize)]
struct DeezerPlaylist {
    id: i64,
    title: String,
    nb_tracks: Option<u32>,
    picture_medium: Option<String>,
    link: Option<String>,
    creator: Option<Named>,
    #[serde(default)]
    is_loved_track: bool,
}

fn check(error: Option<DeezerError>) -> Result<(), ProviderError> {
    match error {
        None => Ok(()),
        // 800 = no data, 4 = quota.
        Some(error) if error.code == 800 => Err(ProviderError::NotFound),
        Some(error) if error.code == 4 => Err(ProviderError::RateLimited),
        Some(error) => Err(ProviderError::Upstream(format!(
            "Deezer error {}: {}",
            error.code, error.message
        ))),
    }
}

fn to_track(track: DeezerTrack) -> ImportedTrack {
    let title = match track.title_version.as_deref().map(str::trim) {
        Some(version) if !version.is_empty() && !track.title.contains(version) => {
            format!("{} {version}", track.title)
        }
        _ => track.title,
    };
    ImportedTrack {
        track: SourceTrack {
            title,
            artists: track.artist.map(|artist| artist.name).into_iter().collect(),
            album: track.album.map(|album| album.title),
            duration_ms: track.duration.filter(|s| *s > 0).map(|s| s * 1000),
            isrc: track.isrc,
            location: None,
        },
        external_id: track.id.map(|id| id.to_string()),
        external_url: track.link,
    }
}

/// Only follow `next` links that stay on the configured API host.
fn same_origin(next: &str, api_base: &str) -> bool {
    next.starts_with(&format!("{}/", api_base.trim_end_matches('/')))
}

pub async fn fetch_playlist(
    client: &reqwest::Client,
    api_base: &str,
    playlist_id: &str,
    mut progress: impl FnMut(usize),
) -> Result<ImportedPlaylist, ProviderError> {
    let meta: PlaylistResponse =
        send_json(client.get(format!("{api_base}/playlist/{playlist_id}"))).await?;
    check(meta.error)?;
    let mut tracks = Vec::new();
    let mut next = Some(format!(
        "{api_base}/playlist/{playlist_id}/tracks?limit=100"
    ));
    while let Some(url) = next.take() {
        let page: TracksPage = send_json(client.get(&url)).await?;
        check(page.error)?;
        tracks.extend(page.data.into_iter().map(to_track));
        progress(tracks.len());
        if tracks.len() >= MAX_IMPORT_TRACKS {
            break;
        }
        next = page.next.filter(|next| same_origin(next, api_base));
    }
    tracks.truncate(MAX_IMPORT_TRACKS);
    Ok(ImportedPlaylist {
        name: meta.title.unwrap_or_else(|| "Deezer playlist".to_string()),
        source_url: meta.link,
        tracks,
    })
}

pub async fn list_profile_playlists(
    client: &reqwest::Client,
    api_base: &str,
    user_id: &str,
) -> Result<Vec<RemotePlaylist>, ProviderError> {
    let mut playlists = Vec::new();
    let mut next = Some(format!("{api_base}/user/{user_id}/playlists?limit=100"));
    while let Some(url) = next.take() {
        let page: PlaylistsPage = send_json(client.get(&url)).await?;
        check(page.error)?;
        playlists.extend(page.data.into_iter().map(|playlist| RemotePlaylist {
            id: playlist.id.to_string(),
            name: if playlist.is_loved_track {
                "Favourite tracks".to_string()
            } else {
                playlist.title
            },
            track_count: playlist.nb_tracks,
            image_url: playlist.picture_medium,
            owner: playlist.creator.map(|creator| creator.name),
            url: playlist.link,
            kind: "playlist",
        }));
        if playlists.len() >= 500 {
            break;
        }
        next = page.next.filter(|next| same_origin(next, api_base));
    }
    Ok(playlists)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_keep_versions_isrc_and_links() {
        let page: TracksPage = serde_json::from_value(serde_json::json!({
            "data": [{
                "id": 3135556,
                "title": "Harder, Better, Faster, Stronger",
                "title_version": "(Live)",
                "duration": 224,
                "link": "https://www.deezer.com/track/3135556",
                "isrc": "GBDUW0000059",
                "artist": {"name": "Daft Punk"},
                "album": {"title": "Discovery"}
            }],
            "next": "https://api.deezer.com/playlist/1/tracks?index=100"
        }))
        .unwrap();
        let track = to_track(page.data.into_iter().next().unwrap());
        assert_eq!(track.track.title, "Harder, Better, Faster, Stronger (Live)");
        assert_eq!(track.track.artists, vec!["Daft Punk"]);
        assert_eq!(track.track.duration_ms, Some(224_000));
        assert_eq!(track.track.isrc.as_deref(), Some("GBDUW0000059"));
        assert_eq!(track.external_id.as_deref(), Some("3135556"));
    }

    #[test]
    fn in_band_errors_are_mapped() {
        assert!(matches!(
            check(Some(DeezerError {
                code: 800,
                message: "no data".into()
            })),
            Err(ProviderError::NotFound)
        ));
        assert!(matches!(
            check(Some(DeezerError {
                code: 4,
                message: "quota".into()
            })),
            Err(ProviderError::RateLimited)
        ));
        assert!(check(None).is_ok());
    }

    #[test]
    fn pagination_never_leaves_the_api_host() {
        assert!(same_origin(
            "https://api.deezer.com/playlist/1/tracks?index=100",
            API_BASE
        ));
        assert!(!same_origin("https://evil.example/x", API_BASE));
        assert!(!same_origin("https://api.deezer.com.evil/x", API_BASE));
    }
}
