//! Imports from other music services and playlist files.
//!
//! Parson never streams audio from Spotify, Apple Music, YouTube Music or
//! Deezer. What each service's developer terms allow is exposed as a set of
//! [`ProviderCapability`] values per service, and the UI shows them verbatim:
//!
//! | Service        | metadata | library-import | playback | handoff |
//! |----------------|----------|----------------|----------|---------|
//! | Spotify        | yes      | yes (OAuth PKCE)| no      | yes (Spotify Connect, Premium, the user's own devices) |
//! | Apple Music    | yes      | yes (developer token + Music User Token) | no | no |
//! | YouTube Music  | no       | yes (YouTube Data API, titles only) | no | no |
//! | Deezer         | yes      | yes (public API) | no     | no |
//! | Playlist files | no       | yes (M3U, M3U8, PLS, XSPF, CSV/TSV, iTunes XML) | no | no |

pub mod apple;
pub mod deezer;
pub mod files;
pub mod http;
pub mod spotify;
pub mod urls;
pub mod youtube;

use serde::{Deserialize, Serialize};

use crate::library::matching::SourceTrack;

/// Most tracks one import job may hold (Spotify "Liked Songs" can be large).
pub const MAX_IMPORT_TRACKS: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportService {
    Spotify,
    AppleMusic,
    YoutubeMusic,
    Deezer,
    File,
}

impl ImportService {
    pub const ALL: [ImportService; 5] = [
        ImportService::Spotify,
        ImportService::AppleMusic,
        ImportService::YoutubeMusic,
        ImportService::Deezer,
        ImportService::File,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ImportService::Spotify => "spotify",
            ImportService::AppleMusic => "apple_music",
            ImportService::YoutubeMusic => "youtube_music",
            ImportService::Deezer => "deezer",
            ImportService::File => "file",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|service| service.as_str() == value)
    }
}

/// What Parson may do with a service, per that service's developer terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderCapability {
    /// Read track/album metadata (titles, ISRCs, artwork) for matching.
    Metadata,
    /// Read the person's playlists or saved tracks and rebuild them from
    /// their own library.
    LibraryImport,
    /// Stream audio through Parson. Never granted for these services.
    Playback,
    /// Ask the service's own app/device to play (e.g. Spotify Connect).
    Handoff,
}

/// How a person connects a service.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectMethod {
    /// OAuth 2.0 Authorization Code with PKCE using the person's own client id.
    OauthPkce,
    /// A developer token and Music User Token the person supplies.
    DeveloperToken,
    /// A Google API key the person supplies.
    ApiKey,
    /// No sign-in: public data only.
    Public,
    /// Upload a file.
    Upload,
}

#[derive(Clone, Debug, Serialize)]
pub struct CapabilityGrant {
    pub capability: ProviderCapability,
    pub available: bool,
    /// Plain-language explanation shown next to the capability.
    pub detail: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct ServiceDescriptor {
    pub id: ImportService,
    pub name: &'static str,
    pub connect_method: ConnectMethod,
    pub capabilities: Vec<CapabilityGrant>,
    /// Source formats accepted (URLs or file types).
    pub accepts: Vec<&'static str>,
    pub summary: &'static str,
    pub requirements: &'static str,
    pub terms_url: &'static str,
}

const NO_PLAYBACK: &str = "Mujin Music never streams audio from this service. Imported tracks play from your own library.";

fn grant(capability: ProviderCapability, available: bool, detail: &'static str) -> CapabilityGrant {
    CapabilityGrant {
        capability,
        available,
        detail,
    }
}

pub fn service_descriptor(service: ImportService) -> ServiceDescriptor {
    use ProviderCapability::{Handoff, LibraryImport, Metadata, Playback};
    match service {
        ImportService::Spotify => ServiceDescriptor {
            id: service,
            name: "Spotify",
            connect_method: ConnectMethod::OauthPkce,
            capabilities: vec![
                grant(
                    Metadata,
                    true,
                    "Track titles, artists, albums, durations and ISRCs from the Spotify Web API.",
                ),
                grant(
                    LibraryImport,
                    true,
                    "Your playlists and Liked Songs, matched to tracks you own.",
                ),
                grant(Playback, false, NO_PLAYBACK),
                grant(
                    Handoff,
                    true,
                    "Spotify Connect: start a Mujin Music queue on one of your own Spotify devices. Requires Spotify Premium; Spotify plays it, not Mujin Music.",
                ),
            ],
            accepts: vec![
                "open.spotify.com/playlist/…",
                "spotify:playlist:…",
                "Liked Songs",
            ],
            summary: "Import playlists and Liked Songs; hand playback off to your Spotify devices.",
            requirements: "Create a free app at developer.spotify.com, add this page's callback as a Redirect URI, and paste its Client ID. Spotify accepts https or http://127.0.0.1 callbacks only.",
            terms_url: "https://developer.spotify.com/terms",
        },
        ImportService::AppleMusic => ServiceDescriptor {
            id: service,
            name: "Apple Music",
            connect_method: ConnectMethod::DeveloperToken,
            capabilities: vec![
                grant(
                    Metadata,
                    true,
                    "Catalog titles, artists, durations and ISRCs from the Apple Music API.",
                ),
                grant(
                    LibraryImport,
                    true,
                    "Your library playlists (needs a Music User Token) and public catalog playlists (developer token only).",
                ),
                grant(Playback, false, NO_PLAYBACK),
                grant(
                    Handoff,
                    false,
                    "Apple does not offer a remote-play API to third parties.",
                ),
            ],
            accepts: vec!["music.apple.com/…/playlist/…/pl.…", "Library playlists"],
            summary: "Import library and catalog playlists with your own MusicKit developer token.",
            requirements: "Requires an Apple Developer Program membership: paste a MusicKit developer token (JWT). For library playlists also paste a Music User Token from MusicKit JS authorize().",
            terms_url: "https://developer.apple.com/support/terms/apple-developer-program-license-agreement/",
        },
        ImportService::YoutubeMusic => ServiceDescriptor {
            id: service,
            name: "YouTube Music",
            connect_method: ConnectMethod::ApiKey,
            capabilities: vec![
                grant(
                    Metadata,
                    false,
                    "YouTube exposes video titles only; there is no music metadata API.",
                ),
                grant(
                    LibraryImport,
                    true,
                    "Public and unlisted playlists via the YouTube Data API. Titles are parsed into artist and track, then matched.",
                ),
                grant(Playback, false, NO_PLAYBACK),
                grant(Handoff, false, "No remote-play API is available."),
            ],
            accepts: vec![
                "music.youtube.com/playlist?list=…",
                "youtube.com/playlist?list=…",
            ],
            summary: "Import public or unlisted playlists by link (titles only, then matched).",
            requirements: "Paste a YouTube Data API v3 key from console.cloud.google.com. Private playlists and Liked Music are not readable with a key.",
            terms_url: "https://developers.google.com/youtube/terms/api-services-terms-of-service",
        },
        ImportService::Deezer => ServiceDescriptor {
            id: service,
            name: "Deezer",
            connect_method: ConnectMethod::Public,
            capabilities: vec![
                grant(
                    Metadata,
                    true,
                    "Public track, album and artist data from the Deezer API.",
                ),
                grant(
                    LibraryImport,
                    true,
                    "Public playlists by link, or every public playlist of a Deezer profile.",
                ),
                grant(Playback, false, NO_PLAYBACK),
                grant(
                    Handoff,
                    false,
                    "Deezer offers 30-second previews only to third parties; Mujin Music does not use them.",
                ),
            ],
            accepts: vec!["deezer.com/…/playlist/…", "deezer.com/…/profile/…"],
            summary: "Import public playlists without signing in.",
            requirements: "No account needed. Make the playlist public in Deezer first.",
            terms_url: "https://developers.deezer.com/termsofuse",
        },
        ImportService::File => ServiceDescriptor {
            id: service,
            name: "Playlist file",
            connect_method: ConnectMethod::Upload,
            capabilities: vec![
                grant(Metadata, false, "Uses only what the file contains."),
                grant(
                    LibraryImport,
                    true,
                    "M3U/M3U8, PLS, XSPF, CSV/TSV (Exportify, TuneMyMusic, Soundiiz, Apple Music text export) and iTunes/Music XML playlists.",
                ),
                grant(
                    Playback,
                    false,
                    "Files are matched to your library; nothing is downloaded.",
                ),
                grant(Handoff, false, "Not applicable."),
            ],
            accepts: vec![
                ".m3u", ".m3u8", ".pls", ".xspf", ".csv", ".tsv", ".txt", ".xml",
            ],
            summary: "Import any exported playlist file.",
            requirements: "Nothing to set up.",
            terms_url: "",
        },
    }
}

/// One track read from a source, plus where it came from.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ImportedTrack {
    #[serde(flatten)]
    pub track: SourceTrack,
    /// The service's own id (e.g. a Spotify track id), when known.
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub external_url: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ImportedPlaylist {
    pub name: String,
    pub source_url: Option<String>,
    pub tracks: Vec<ImportedTrack>,
}

/// A playlist the person can pick from a connected service.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RemotePlaylist {
    pub id: String,
    pub name: String,
    pub track_count: Option<u32>,
    pub image_url: Option<String>,
    pub owner: Option<String>,
    pub url: Option<String>,
    /// `playlist`, `saved_tracks` or `profile`.
    pub kind: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_streaming_service_ever_grants_playback() {
        for service in ImportService::ALL {
            let descriptor = service_descriptor(service);
            assert_eq!(descriptor.capabilities.len(), 4, "{service:?}");
            let playback = descriptor
                .capabilities
                .iter()
                .find(|grant| grant.capability == ProviderCapability::Playback)
                .expect("playback is always listed");
            assert!(!playback.available, "{service:?} must not stream audio");
            assert!(!playback.detail.is_empty());
        }
    }

    #[test]
    fn only_spotify_offers_handoff() {
        for service in ImportService::ALL {
            let handoff = service_descriptor(service)
                .capabilities
                .into_iter()
                .any(|grant| grant.capability == ProviderCapability::Handoff && grant.available);
            assert_eq!(handoff, service == ImportService::Spotify);
        }
    }

    #[test]
    fn service_ids_round_trip_and_serialize_like_the_sdk() {
        for service in ImportService::ALL {
            assert_eq!(ImportService::parse(service.as_str()), Some(service));
            assert_eq!(
                serde_json::to_value(service).unwrap(),
                serde_json::Value::String(service.as_str().to_string())
            );
        }
        assert_eq!(
            serde_json::to_value(ProviderCapability::LibraryImport).unwrap(),
            "library-import"
        );
        assert_eq!(ImportService::parse("tidal"), None);
    }
}
