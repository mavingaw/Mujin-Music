//! Recognises pasted links to playlists on the supported services.

use serde::Serialize;

use super::ImportService;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Playlist,
    /// Spotify Liked Songs.
    SavedTracks,
    /// Every public playlist of a Deezer profile.
    Profile,
    /// An Apple Music library playlist (`p.…`).
    LibraryPlaylist,
    File,
}

impl SourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceKind::Playlist => "playlist",
            SourceKind::SavedTracks => "saved_tracks",
            SourceKind::Profile => "profile",
            SourceKind::LibraryPlaylist => "library_playlist",
            SourceKind::File => "file",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "playlist" => SourceKind::Playlist,
            "saved_tracks" => SourceKind::SavedTracks,
            "profile" => SourceKind::Profile,
            "library_playlist" => SourceKind::LibraryPlaylist,
            "file" => SourceKind::File,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ParsedSource {
    pub service: ImportService,
    pub kind: SourceKind,
    pub id: String,
    /// Apple Music storefront (`us`, `gb`, ...).
    pub storefront: Option<String>,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.starts_with('.')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn query_param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then_some(value)
    })
}

/// Parses a playlist link or URI. Returns `None` when it is not recognised.
pub fn parse_source_url(input: &str) -> Option<ParsedSource> {
    let input = input.trim();
    if let Some(rest) = input.strip_prefix("spotify:") {
        let mut parts = rest.split(':');
        return match (parts.next(), parts.next(), parts.next()) {
            (Some("playlist"), Some(id), None) if valid_id(id) => Some(ParsedSource {
                service: ImportService::Spotify,
                kind: SourceKind::Playlist,
                id: id.to_string(),
                storefront: None,
            }),
            (Some("user"), Some(_), Some("collection")) => Some(ParsedSource {
                service: ImportService::Spotify,
                kind: SourceKind::SavedTracks,
                id: "liked".to_string(),
                storefront: None,
            }),
            _ => None,
        };
    }

    let without_scheme = input
        .strip_prefix("https://")
        .or_else(|| input.strip_prefix("http://"))
        .unwrap_or(input);
    let (host_and_path, query) = without_scheme
        .split_once('?')
        .map_or((without_scheme, ""), |(left, right)| (left, right));
    let query = query.split('#').next().unwrap_or_default();
    let host_and_path = host_and_path.split('#').next().unwrap_or_default();
    let (host, path) = host_and_path
        .split_once('/')
        .map_or((host_and_path, ""), |(host, path)| (host, path));
    let host = host.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let segments: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();

    match host {
        "open.spotify.com" | "play.spotify.com" => {
            // Optional locale prefix: /intl-de/playlist/{id}
            let segments: Vec<&str> = segments
                .into_iter()
                .skip_while(|part| part.starts_with("intl-"))
                .collect();
            match segments.as_slice() {
                ["playlist", id, ..] if valid_id(id) => Some(ParsedSource {
                    service: ImportService::Spotify,
                    kind: SourceKind::Playlist,
                    id: (*id).to_string(),
                    storefront: None,
                }),
                ["collection", "tracks"] => Some(ParsedSource {
                    service: ImportService::Spotify,
                    kind: SourceKind::SavedTracks,
                    id: "liked".to_string(),
                    storefront: None,
                }),
                _ => None,
            }
        }
        "deezer.com" => {
            let segments: Vec<&str> = segments
                .into_iter()
                .skip_while(|part| part.len() == 2 && part.chars().all(|c| c.is_ascii_lowercase()))
                .collect();
            match segments.as_slice() {
                ["playlist", id, ..] if id.chars().all(|c| c.is_ascii_digit()) => {
                    Some(ParsedSource {
                        service: ImportService::Deezer,
                        kind: SourceKind::Playlist,
                        id: (*id).to_string(),
                        storefront: None,
                    })
                }
                ["profile", id, ..] if id.chars().all(|c| c.is_ascii_digit()) => {
                    Some(ParsedSource {
                        service: ImportService::Deezer,
                        kind: SourceKind::Profile,
                        id: (*id).to_string(),
                        storefront: None,
                    })
                }
                _ => None,
            }
        }
        "music.youtube.com" | "youtube.com" | "m.youtube.com" => {
            let list = query_param(query, "list")?;
            // "LM" (Liked Music) and "WL" (Watch Later) are private lists
            // the Data API will not return with an API key.
            if !valid_id(list) || matches!(list, "LM" | "WL" | "LL") {
                return None;
            }
            Some(ParsedSource {
                service: ImportService::YoutubeMusic,
                kind: SourceKind::Playlist,
                id: list.to_string(),
                storefront: None,
            })
        }
        "music.apple.com" | "itunes.apple.com" => match segments.as_slice() {
            ["library", "playlist", id] if valid_id(id) => Some(ParsedSource {
                service: ImportService::AppleMusic,
                kind: SourceKind::LibraryPlaylist,
                id: (*id).to_string(),
                storefront: None,
            }),
            [storefront, "playlist", .., id] if storefront.len() == 2 && valid_id(id) => {
                Some(ParsedSource {
                    service: ImportService::AppleMusic,
                    kind: if id.starts_with("p.") {
                        SourceKind::LibraryPlaylist
                    } else {
                        SourceKind::Playlist
                    },
                    id: (*id).to_string(),
                    storefront: Some(storefront.to_ascii_lowercase()),
                })
            }
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(input: &str) -> (ImportService, SourceKind, String) {
        let source = parse_source_url(input).unwrap_or_else(|| panic!("{input} should parse"));
        (source.service, source.kind, source.id)
    }

    #[test]
    fn spotify_links_and_uris() {
        assert_eq!(
            parsed("https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M?si=abc"),
            (
                ImportService::Spotify,
                SourceKind::Playlist,
                "37i9dQZF1DXcBWIGoYBM5M".into()
            )
        );
        assert_eq!(
            parsed("https://open.spotify.com/intl-de/playlist/abc123"),
            (
                ImportService::Spotify,
                SourceKind::Playlist,
                "abc123".into()
            )
        );
        assert_eq!(
            parsed("spotify:playlist:abc123"),
            (
                ImportService::Spotify,
                SourceKind::Playlist,
                "abc123".into()
            )
        );
        assert_eq!(
            parsed("https://open.spotify.com/collection/tracks"),
            (
                ImportService::Spotify,
                SourceKind::SavedTracks,
                "liked".into()
            )
        );
        assert!(parse_source_url("https://open.spotify.com/track/abc").is_none());
    }

    #[test]
    fn deezer_links_with_and_without_locale() {
        assert_eq!(
            parsed("https://www.deezer.com/en/playlist/908622995"),
            (
                ImportService::Deezer,
                SourceKind::Playlist,
                "908622995".into()
            )
        );
        assert_eq!(
            parsed("deezer.com/playlist/1"),
            (ImportService::Deezer, SourceKind::Playlist, "1".into())
        );
        assert_eq!(
            parsed("https://www.deezer.com/fr/profile/5"),
            (ImportService::Deezer, SourceKind::Profile, "5".into())
        );
        assert!(parse_source_url("https://www.deezer.com/en/playlist/abc").is_none());
    }

    #[test]
    fn youtube_playlists_but_not_private_lists() {
        assert_eq!(
            parsed("https://music.youtube.com/playlist?list=PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG"),
            (
                ImportService::YoutubeMusic,
                SourceKind::Playlist,
                "PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG".into()
            )
        );
        assert_eq!(
            parsed("https://www.youtube.com/watch?v=x&list=OLAK5uy_abc"),
            (
                ImportService::YoutubeMusic,
                SourceKind::Playlist,
                "OLAK5uy_abc".into()
            )
        );
        assert!(parse_source_url("https://music.youtube.com/playlist?list=LM").is_none());
        assert!(parse_source_url("https://youtube.com/watch?v=abc").is_none());
    }

    #[test]
    fn apple_catalog_and_library_playlists() {
        let catalog = parse_source_url(
            "https://music.apple.com/us/playlist/todays-hits/pl.f4d106fed2bd41149aaacabb233eb5eb",
        )
        .unwrap();
        assert_eq!(catalog.service, ImportService::AppleMusic);
        assert_eq!(catalog.kind, SourceKind::Playlist);
        assert_eq!(catalog.storefront.as_deref(), Some("us"));
        assert_eq!(catalog.id, "pl.f4d106fed2bd41149aaacabb233eb5eb");

        assert_eq!(
            parsed("https://music.apple.com/library/playlist/p.abc"),
            (
                ImportService::AppleMusic,
                SourceKind::LibraryPlaylist,
                "p.abc".into()
            )
        );
    }

    #[test]
    fn unknown_or_hostile_input_is_rejected() {
        for input in [
            "",
            "not a url",
            "https://evil.example/playlist/1",
            "https://open.spotify.com/playlist/../../etc",
            "spotify:playlist:a/b",
        ] {
            assert!(parse_source_url(input).is_none(), "{input}");
        }
    }
}
