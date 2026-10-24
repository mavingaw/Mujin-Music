//! YouTube Music playlists through the YouTube Data API v3.
//!
//! YouTube offers no music metadata API, so only video titles and channel
//! names are read and then parsed into artist and track for matching.

use std::sync::OnceLock;

use regex::Regex;
use serde::Deserialize;

use super::files::split_display_title;
use super::http::{ProviderError, send_json};
use super::{ImportedPlaylist, ImportedTrack, MAX_IMPORT_TRACKS};
use crate::library::matching::SourceTrack;

pub const API_BASE: &str = "https://www.googleapis.com/youtube/v3";

fn video_noise() -> &'static Regex {
    static NOISE: OnceLock<Regex> = OnceLock::new();
    NOISE.get_or_init(|| {
        Regex::new(
            r"(?i)\s*[\(\[\{]\s*(?:official\s+)?(?:music\s+|lyric\s+|lyrics\s+|hd\s+|hq\s+|4k\s+)?(?:video|audio|visuali[sz]er|lyrics?|mv|m/v|hd|hq|4k|explicit|clean|performance\s+video|color\s+coded\s+lyrics)(?:\s+(?:hd|hq|4k|version))?\s*[\)\]\}]",
        )
        .expect("youtube noise regex")
    })
}

/// Parses a music video title (and its channel) into artist and track.
///
/// * `Artist - Title (Official Video)` -> (`Artist`, `Title`)
/// * YouTube Music auto-generated uploads come from `Artist - Topic`
///   channels whose video title is the bare track name.
/// * `Artist "Title"` and `Artist 'Title'` (common for K-pop/J-pop uploads).
pub fn parse_video_title(title: &str, channel: Option<&str>) -> (Vec<String>, String) {
    static TRAILING: OnceLock<Regex> = OnceLock::new();
    let trailing = TRAILING.get_or_init(|| {
        Regex::new(r"(?i)\s+(?:official\s+(?:music\s+)?(?:video|audio|mv)|m/?v)\s*$")
            .expect("trailing video noise regex")
    });
    let cleaned = video_noise().replace_all(title, "").trim().to_string();
    let cleaned = trailing.replace(&cleaned, "").trim().to_string();
    let cleaned = cleaned
        .trim_end_matches(['|', '-', '–'])
        .trim()
        .split(" | ")
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    let topic_artist = channel
        .and_then(|channel| channel.strip_suffix(" - Topic"))
        .map(str::trim)
        .filter(|artist| !artist.is_empty());
    if let Some(artist) = topic_artist {
        return (vec![artist.to_string()], cleaned);
    }
    let (artists, track) = split_display_title(&cleaned);
    if !artists.is_empty() {
        return (artists, track);
    }
    static QUOTED: OnceLock<Regex> = OnceLock::new();
    let quoted = QUOTED.get_or_init(|| {
        Regex::new(r#"^(.+?)\s+["'“‘『「](.+?)["'”’』」]\s*$"#).expect("quoted title regex")
    });
    if let Some(captures) = quoted.captures(&cleaned) {
        return (
            vec![captures[1].trim().to_string()],
            captures[2].trim().to_string(),
        );
    }
    let channel_artist = channel
        .map(|channel| {
            channel
                .trim_end_matches("VEVO")
                .trim_end_matches(" Official")
                .trim()
                .to_string()
        })
        .filter(|channel| !channel.is_empty());
    (channel_artist.into_iter().collect(), cleaned)
}

#[derive(Deserialize)]
struct PlaylistItemsPage {
    #[serde(default)]
    items: Vec<PlaylistItem>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct PlaylistItem {
    snippet: Option<PlaylistItemSnippet>,
}

#[derive(Deserialize)]
struct PlaylistItemSnippet {
    title: String,
    #[serde(rename = "videoOwnerChannelTitle")]
    video_owner_channel_title: Option<String>,
    #[serde(rename = "resourceId")]
    resource_id: Option<ResourceId>,
}

#[derive(Deserialize)]
struct ResourceId {
    #[serde(rename = "videoId")]
    video_id: Option<String>,
}

#[derive(Deserialize)]
struct PlaylistsPage {
    #[serde(default)]
    items: Vec<PlaylistResource>,
}

#[derive(Deserialize)]
struct PlaylistResource {
    snippet: Option<PlaylistResourceSnippet>,
}

#[derive(Deserialize)]
struct PlaylistResourceSnippet {
    title: String,
}

fn tracks_from_page(page: PlaylistItemsPage) -> Vec<ImportedTrack> {
    page.items
        .into_iter()
        .filter_map(|item| item.snippet)
        // Removed and private videos keep a placeholder entry.
        .filter(|snippet| !matches!(snippet.title.as_str(), "Deleted video" | "Private video"))
        .map(|snippet| {
            let (artists, title) =
                parse_video_title(&snippet.title, snippet.video_owner_channel_title.as_deref());
            let video_id = snippet.resource_id.and_then(|resource| resource.video_id);
            ImportedTrack {
                track: SourceTrack {
                    title,
                    artists,
                    ..SourceTrack::default()
                },
                external_url: video_id
                    .as_ref()
                    .map(|id| format!("https://music.youtube.com/watch?v={id}")),
                external_id: video_id,
            }
        })
        .collect()
}

pub async fn fetch_playlist(
    client: &reqwest::Client,
    api_base: &str,
    api_key: &str,
    playlist_id: &str,
    mut progress: impl FnMut(usize),
) -> Result<ImportedPlaylist, ProviderError> {
    let meta: PlaylistsPage = send_json(client.get(format!("{api_base}/playlists")).query(&[
        ("part", "snippet"),
        ("id", playlist_id),
        ("key", api_key),
    ]))
    .await?;
    let name = meta
        .items
        .into_iter()
        .find_map(|item| item.snippet)
        .map(|snippet| snippet.title)
        .ok_or(ProviderError::NotFound)?;
    let mut tracks = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut request = client.get(format!("{api_base}/playlistItems")).query(&[
            ("part", "snippet"),
            ("maxResults", "50"),
            ("playlistId", playlist_id),
            ("key", api_key),
        ]);
        if let Some(token) = page_token.as_deref() {
            request = request.query(&[("pageToken", token)]);
        }
        let page: PlaylistItemsPage = send_json(request).await?;
        page_token = page.next_page_token.clone();
        tracks.extend(tracks_from_page(page));
        progress(tracks.len());
        if page_token.is_none() || tracks.len() >= MAX_IMPORT_TRACKS {
            break;
        }
    }
    tracks.truncate(MAX_IMPORT_TRACKS);
    Ok(ImportedPlaylist {
        name,
        source_url: Some(format!(
            "https://music.youtube.com/playlist?list={playlist_id}"
        )),
        tracks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(title: &str, channel: Option<&str>) -> (Vec<String>, String) {
        parse_video_title(title, channel)
    }

    #[test]
    fn artist_dash_title_with_video_noise() {
        assert_eq!(
            parsed(
                "Daft Punk - Get Lucky (Official Video) [HD]",
                Some("DaftPunkVEVO")
            ),
            (vec!["Daft Punk".to_string()], "Get Lucky".to_string())
        );
        assert_eq!(
            parsed("Adele - Hello (Official Music Video)", None),
            (vec!["Adele".to_string()], "Hello".to_string())
        );
        assert_eq!(
            parsed("Nirvana – Lithium (Lyrics) | Nevermind", None),
            (vec!["Nirvana".to_string()], "Lithium".to_string())
        );
    }

    #[test]
    fn topic_channels_name_the_artist() {
        assert_eq!(
            parsed(
                "Get Lucky (feat. Pharrell Williams)",
                Some("Daft Punk - Topic")
            ),
            (
                vec!["Daft Punk".to_string()],
                "Get Lucky (feat. Pharrell Williams)".to_string()
            )
        );
    }

    #[test]
    fn quoted_titles_and_channel_fallback() {
        assert_eq!(
            parsed("BTS \"Dynamite\" Official MV", None),
            (vec!["BTS".to_string()], "Dynamite".to_string())
        );
        assert_eq!(
            parsed("NewJeans 'Super Shy'", None),
            (vec!["NewJeans".to_string()], "Super Shy".to_string())
        );
        assert_eq!(
            parsed("Hello", Some("AdeleVEVO")),
            (vec!["Adele".to_string()], "Hello".to_string())
        );
    }

    #[test]
    fn placeholder_items_are_skipped() {
        let page: PlaylistItemsPage = serde_json::from_value(serde_json::json!({
            "items": [
                {"snippet": {"title": "Deleted video"}},
                {"snippet": {"title": "Song", "videoOwnerChannelTitle": "Band - Topic",
                  "resourceId": {"videoId": "abc"}}}
            ]
        }))
        .unwrap();
        let tracks = tracks_from_page(page);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].track.artists, vec!["Band"]);
        assert_eq!(
            tracks[0].external_url.as_deref(),
            Some("https://music.youtube.com/watch?v=abc")
        );
    }
}
