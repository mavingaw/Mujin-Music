//! Apple Music API with a MusicKit developer token the person supplies.
//! Catalog playlists need only the developer token; library playlists also
//! need a Music User Token. Parson never streams Apple Music audio.

use serde::{Deserialize, Serialize};

use super::http::{ProviderError, send_json};
use super::{ImportedPlaylist, ImportedTrack, MAX_IMPORT_TRACKS, RemotePlaylist};
use crate::library::matching::SourceTrack;

pub const API_BASE: &str = "https://api.music.apple.com";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AppleCredentials {
    pub developer_token: String,
    #[serde(default)]
    pub music_user_token: Option<String>,
    pub storefront: String,
}

/// A developer token is an ES256 JWT: three base64url segments.
pub fn plausible_jwt(token: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    token.len() <= 4096
        && parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
}

pub fn valid_storefront(storefront: &str) -> bool {
    storefront.len() == 2 && storefront.chars().all(|c| c.is_ascii_lowercase())
}

#[derive(Deserialize)]
struct Document {
    #[serde(default)]
    data: Vec<Resource>,
    next: Option<String>,
}

#[derive(Deserialize)]
struct Resource {
    id: String,
    attributes: Option<Attributes>,
    relationships: Option<Relationships>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Attributes {
    name: Option<String>,
    artist_name: Option<String>,
    album_name: Option<String>,
    duration_in_millis: Option<u64>,
    isrc: Option<String>,
    url: Option<String>,
    artwork: Option<Artwork>,
    curator_name: Option<String>,
}

#[derive(Deserialize)]
struct Artwork {
    url: String,
}

#[derive(Deserialize, Default)]
struct Relationships {
    catalog: Option<Relationship>,
}

#[derive(Deserialize)]
struct Relationship {
    #[serde(default)]
    data: Vec<Resource>,
}

fn to_track(resource: Resource) -> Option<ImportedTrack> {
    let attributes = resource.attributes?;
    let catalog = resource
        .relationships
        .and_then(|relationships| relationships.catalog)
        .and_then(|catalog| catalog.data.into_iter().next())
        .and_then(|song| song.attributes);
    let name = attributes.name.filter(|name| !name.trim().is_empty())?;
    Some(ImportedTrack {
        track: SourceTrack {
            title: name,
            artists: attributes.artist_name.into_iter().collect(),
            album: attributes.album_name,
            duration_ms: attributes.duration_in_millis.filter(|ms| *ms > 0),
            isrc: attributes
                .isrc
                .or_else(|| catalog.as_ref().and_then(|song| song.isrc.clone())),
            location: None,
        },
        external_url: attributes.url.or_else(|| catalog.and_then(|song| song.url)),
        external_id: Some(resource.id),
    })
}

fn artwork_url(artwork: Option<Artwork>) -> Option<String> {
    artwork.map(|artwork| artwork.url.replace("{w}", "300").replace("{h}", "300"))
}

fn request(
    client: &reqwest::Client,
    url: String,
    credentials: &AppleCredentials,
    needs_user: bool,
) -> Result<reqwest::RequestBuilder, ProviderError> {
    let mut builder = client.get(url).bearer_auth(&credentials.developer_token);
    if let Some(user_token) = credentials.music_user_token.as_deref() {
        builder = builder.header("Music-User-Token", user_token);
    } else if needs_user {
        return Err(ProviderError::Invalid(
            "Library playlists need a Music User Token. Add one in the Apple Music connection."
                .to_string(),
        ));
    }
    Ok(builder)
}

/// `next` links are paths on the API host.
fn next_url(api_base: &str, next: Option<String>) -> Option<String> {
    next.filter(|path| path.starts_with("/v1/") && !path.contains("://"))
        .map(|path| format!("{api_base}{path}"))
}

pub async fn verify(
    client: &reqwest::Client,
    api_base: &str,
    credentials: &AppleCredentials,
) -> Result<(), ProviderError> {
    let _: Document = send_json(request(
        client,
        format!("{api_base}/v1/storefronts/{}", credentials.storefront),
        credentials,
        false,
    )?)
    .await?;
    if credentials.music_user_token.is_some() {
        let _: Document = send_json(request(
            client,
            format!("{api_base}/v1/me/library/playlists?limit=1"),
            credentials,
            true,
        )?)
        .await?;
    }
    Ok(())
}

pub async fn list_library_playlists(
    client: &reqwest::Client,
    api_base: &str,
    credentials: &AppleCredentials,
) -> Result<Vec<RemotePlaylist>, ProviderError> {
    let mut playlists = Vec::new();
    let mut next = Some(format!("{api_base}/v1/me/library/playlists?limit=100"));
    while let Some(url) = next.take() {
        let page: Document = send_json(request(client, url, credentials, true)?).await?;
        playlists.extend(page.data.into_iter().map(|resource| {
            let attributes = resource.attributes.unwrap_or_default();
            RemotePlaylist {
                url: Some(format!(
                    "https://music.apple.com/library/playlist/{}",
                    resource.id
                )),
                id: resource.id,
                name: attributes.name.unwrap_or_else(|| "Untitled".to_string()),
                track_count: None,
                image_url: artwork_url(attributes.artwork),
                owner: attributes.curator_name,
                kind: "library_playlist",
            }
        }));
        if playlists.len() >= 1_000 {
            break;
        }
        next = next_url(api_base, page.next);
    }
    Ok(playlists)
}

async fn collect(
    client: &reqwest::Client,
    api_base: &str,
    credentials: &AppleCredentials,
    first: String,
    needs_user: bool,
    progress: &mut impl FnMut(usize),
) -> Result<Vec<ImportedTrack>, ProviderError> {
    let mut tracks = Vec::new();
    let mut next = Some(first);
    while let Some(url) = next.take() {
        let page: Document = send_json(request(client, url, credentials, needs_user)?).await?;
        tracks.extend(page.data.into_iter().filter_map(to_track));
        progress(tracks.len());
        if tracks.len() >= MAX_IMPORT_TRACKS {
            break;
        }
        next = next_url(api_base, page.next);
    }
    tracks.truncate(MAX_IMPORT_TRACKS);
    Ok(tracks)
}

pub async fn fetch_catalog_playlist(
    client: &reqwest::Client,
    api_base: &str,
    credentials: &AppleCredentials,
    storefront: &str,
    playlist_id: &str,
    mut progress: impl FnMut(usize),
) -> Result<ImportedPlaylist, ProviderError> {
    let document: Document = send_json(request(
        client,
        format!("{api_base}/v1/catalog/{storefront}/playlists/{playlist_id}"),
        credentials,
        false,
    )?)
    .await?;
    let resource = document
        .data
        .into_iter()
        .next()
        .ok_or(ProviderError::NotFound)?;
    let attributes = resource.attributes.unwrap_or_default();
    let tracks = collect(
        client,
        api_base,
        credentials,
        format!("{api_base}/v1/catalog/{storefront}/playlists/{playlist_id}/tracks?limit=100"),
        false,
        &mut progress,
    )
    .await?;
    Ok(ImportedPlaylist {
        name: attributes
            .name
            .unwrap_or_else(|| "Apple Music playlist".to_string()),
        source_url: attributes.url,
        tracks,
    })
}

pub async fn fetch_library_playlist(
    client: &reqwest::Client,
    api_base: &str,
    credentials: &AppleCredentials,
    playlist_id: &str,
    mut progress: impl FnMut(usize),
) -> Result<ImportedPlaylist, ProviderError> {
    let document: Document = send_json(request(
        client,
        format!("{api_base}/v1/me/library/playlists/{playlist_id}"),
        credentials,
        true,
    )?)
    .await?;
    let name = document
        .data
        .into_iter()
        .next()
        .and_then(|resource| resource.attributes)
        .and_then(|attributes| attributes.name)
        .ok_or(ProviderError::NotFound)?;
    let tracks = collect(
        client,
        api_base,
        credentials,
        format!(
            "{api_base}/v1/me/library/playlists/{playlist_id}/tracks?limit=100&include=catalog"
        ),
        true,
        &mut progress,
    )
    .await?;
    Ok(ImportedPlaylist {
        name,
        source_url: Some(format!(
            "https://music.apple.com/library/playlist/{playlist_id}"
        )),
        tracks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_songs_borrow_isrc_from_their_catalog_twin() {
        let page: Document = serde_json::from_value(serde_json::json!({
            "data": [{
                "id": "i.abc",
                "type": "library-songs",
                "attributes": {"name": "Blinding Lights", "artistName": "The Weeknd",
                  "albumName": "After Hours", "durationInMillis": 200040},
                "relationships": {"catalog": {"data": [{
                    "id": "1499378615",
                    "attributes": {"isrc": "USUG11904206",
                      "url": "https://music.apple.com/us/song/1499378615"}
                }]}}
            }, {"id": "i.empty", "attributes": {"name": "  "}}],
            "next": "/v1/me/library/playlists/p.x/tracks?offset=100"
        }))
        .unwrap();
        assert_eq!(
            next_url(API_BASE, page.next.clone()).as_deref(),
            Some("https://api.music.apple.com/v1/me/library/playlists/p.x/tracks?offset=100")
        );
        let tracks: Vec<_> = page.data.into_iter().filter_map(to_track).collect();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].track.isrc.as_deref(), Some("USUG11904206"));
        assert_eq!(tracks[0].track.duration_ms, Some(200_040));
        assert_eq!(
            tracks[0].external_url.as_deref(),
            Some("https://music.apple.com/us/song/1499378615")
        );
    }

    #[test]
    fn tokens_and_storefronts_are_validated_before_use() {
        assert!(plausible_jwt("eyJhbGciOiJFUzI1NiJ9.eyJpc3MiOiJ4In0.c2ln"));
        assert!(!plausible_jwt("not a token"));
        assert!(!plausible_jwt("a.b"));
        assert!(valid_storefront("us"));
        assert!(!valid_storefront("USA"));
        assert_eq!(next_url(API_BASE, Some("https://evil/v1/x".into())), None);
        assert_eq!(
            next_url(API_BASE, Some("/v1/a?u=https://evil".into())),
            None
        );
    }

    #[test]
    fn library_requests_require_a_user_token() {
        let client = reqwest::Client::new();
        let credentials = AppleCredentials {
            developer_token: "a.b.c".into(),
            music_user_token: None,
            storefront: "us".into(),
        };
        assert!(request(&client, "https://x/v1/me".into(), &credentials, true).is_err());
        assert!(request(&client, "https://x/v1/catalog".into(), &credentials, false).is_ok());
    }
}
