//! Spotify Web API: OAuth 2.0 PKCE sign-in with the person's own client id,
//! playlist and Liked Songs import, and Spotify Connect handoff to the
//! person's own devices. Parson never streams Spotify audio.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::http::{ProviderError, send_bytes, send_json, status_error};
use super::{ImportedPlaylist, ImportedTrack, MAX_IMPORT_TRACKS, RemotePlaylist};
use crate::library::matching::SourceTrack;

pub const ACCOUNTS_BASE: &str = "https://accounts.spotify.com";
pub const API_BASE: &str = "https://api.spotify.com/v1";
pub const SCOPES: &str = "playlist-read-private playlist-read-collaborative user-library-read user-read-playback-state user-modify-playback-state";
/// Refresh this long before the access token actually expires.
const EXPIRY_MARGIN_SECONDS: i64 = 60;
pub const MAX_HANDOFF_TRACKS: usize = 50;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SpotifyCredentials {
    pub client_id: String,
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds.
    pub expires_at: i64,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub product: Option<String>,
}

/// A random PKCE code verifier (RFC 7636: 43-128 unreserved characters).
pub fn code_verifier() -> String {
    let mut bytes = [0_u8; 48];
    rand::fill(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn random_state() -> String {
    let mut bytes = [0_u8; 24];
    rand::fill(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// S256 code challenge for a verifier.
pub fn code_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// `application/x-www-form-urlencoded` keeps RFC 3986 unreserved characters.
const FORM: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

fn encode(value: &str) -> String {
    utf8_percent_encode(value, FORM).to_string()
}

fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| format!("{}={}", encode(key), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Spotify client ids are 32 lowercase hex characters.
pub fn valid_client_id(client_id: &str) -> bool {
    client_id.len() == 32 && client_id.chars().all(|c| c.is_ascii_hexdigit())
}

/// Spotify only accepts https redirect URIs, or http on a loopback IP literal.
pub fn valid_redirect_uri(uri: &str) -> bool {
    if uri.len() > 512 || uri.contains(['#', ' ']) {
        return false;
    }
    if uri.starts_with("https://") {
        return true;
    }
    ["http://127.0.0.1", "http://[::1]"].iter().any(|prefix| {
        uri.strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '/']))
    })
}

pub fn authorize_url(
    accounts_base: &str,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    verifier: &str,
) -> String {
    format!(
        "{accounts_base}/authorize?{}",
        form(&[
            ("response_type", "code"),
            ("client_id", client_id),
            ("scope", SCOPES),
            ("redirect_uri", redirect_uri),
            ("state", state),
            ("code_challenge_method", "S256"),
            ("code_challenge", &code_challenge(verifier)),
        ])
    )
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: i64,
    refresh_token: Option<String>,
    #[serde(default)]
    scope: String,
}

async fn token_request(
    client: &reqwest::Client,
    accounts_base: &str,
    body: String,
) -> Result<TokenResponse, ProviderError> {
    let url = super::http::secure_url(&format!("{accounts_base}/api/token"))?;
    let (status, bytes) = send_bytes(
        client
            .post(url)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(body),
    )
    .await?;
    if status == reqwest::StatusCode::BAD_REQUEST {
        // invalid_grant: the code or refresh token was revoked or reused.
        return Err(ProviderError::Unauthorized);
    }
    if !status.is_success() {
        return Err(status_error(status, &String::from_utf8_lossy(&bytes)));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| ProviderError::Upstream(format!("Unexpected Spotify token: {error}")))
}

pub async fn exchange_code(
    client: &reqwest::Client,
    accounts_base: &str,
    client_id: &str,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
    now: i64,
) -> Result<SpotifyCredentials, ProviderError> {
    let token = token_request(
        client,
        accounts_base,
        form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", client_id),
            ("code_verifier", verifier),
        ]),
    )
    .await?;
    Ok(SpotifyCredentials {
        client_id: client_id.to_string(),
        access_token: token.access_token,
        refresh_token: token.refresh_token.unwrap_or_default(),
        expires_at: now + token.expires_in,
        scope: token.scope,
        product: None,
    })
}

/// Refreshes the access token when it is about to expire. Returns `true`
/// when the credentials changed and must be saved.
pub async fn ensure_fresh(
    client: &reqwest::Client,
    accounts_base: &str,
    credentials: &mut SpotifyCredentials,
    now: i64,
) -> Result<bool, ProviderError> {
    if credentials.expires_at - EXPIRY_MARGIN_SECONDS > now {
        return Ok(false);
    }
    if credentials.refresh_token.is_empty() {
        return Err(ProviderError::Unauthorized);
    }
    let token = token_request(
        client,
        accounts_base,
        form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &credentials.refresh_token),
            ("client_id", &credentials.client_id),
        ]),
    )
    .await?;
    credentials.access_token = token.access_token;
    credentials.expires_at = now + token.expires_in;
    if let Some(refresh) = token.refresh_token {
        credentials.refresh_token = refresh;
    }
    if !token.scope.is_empty() {
        credentials.scope = token.scope;
    }
    Ok(true)
}

#[derive(Deserialize)]
pub struct Profile {
    pub id: String,
    pub display_name: Option<String>,
    pub product: Option<String>,
}

pub async fn profile(
    client: &reqwest::Client,
    api_base: &str,
    token: &str,
) -> Result<Profile, ProviderError> {
    send_json(client.get(format!("{api_base}/me")).bearer_auth(token)).await
}

#[derive(Deserialize)]
struct Paging<T> {
    #[serde(default = "Vec::new")]
    items: Vec<T>,
    next: Option<String>,
    #[serde(default)]
    total: Option<u32>,
}

#[derive(Deserialize)]
struct Image {
    url: String,
}

#[derive(Deserialize, Default)]
struct ExternalUrls {
    spotify: Option<String>,
}

#[derive(Deserialize)]
struct Total {
    total: Option<u32>,
}

#[derive(Deserialize)]
struct SimplePlaylist {
    id: String,
    name: String,
    #[serde(default)]
    images: Option<Vec<Image>>,
    owner: Option<Owner>,
    tracks: Option<Total>,
    items: Option<Total>,
    #[serde(default)]
    external_urls: ExternalUrls,
}

#[derive(Deserialize)]
struct Owner {
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct SpotifyTrack {
    id: Option<String>,
    name: String,
    #[serde(default)]
    duration_ms: Option<u64>,
    #[serde(default)]
    artists: Vec<Named>,
    album: Option<Named>,
    #[serde(default)]
    external_ids: Option<ExternalIds>,
    #[serde(default)]
    external_urls: ExternalUrls,
    #[serde(rename = "type", default)]
    kind: Option<String>,
}

#[derive(Deserialize)]
struct Named {
    name: String,
}

#[derive(Deserialize)]
struct ExternalIds {
    isrc: Option<String>,
}

/// Playlist items carry the track under `track` (classic) or `item`.
#[derive(Deserialize)]
struct PlaylistEntry {
    track: Option<SpotifyTrack>,
    item: Option<SpotifyTrack>,
}

fn to_imported(track: SpotifyTrack) -> Option<ImportedTrack> {
    // Podcast episodes are not music; skip them.
    if track.kind.as_deref().is_some_and(|kind| kind != "track") || track.name.trim().is_empty() {
        return None;
    }
    Some(ImportedTrack {
        track: SourceTrack {
            title: track.name,
            artists: track
                .artists
                .into_iter()
                .map(|artist| artist.name)
                .filter(|name| !name.is_empty())
                .collect(),
            album: track
                .album
                .map(|album| album.name)
                .filter(|name| !name.is_empty()),
            duration_ms: track.duration_ms.filter(|ms| *ms > 0),
            isrc: track.external_ids.and_then(|ids| ids.isrc),
            location: None,
        },
        external_url: track.external_urls.spotify,
        external_id: track.id,
    })
}

fn same_origin(next: &str, api_base: &str) -> bool {
    next.starts_with(&format!("{}/", api_base.trim_end_matches('/')))
}

pub async fn list_playlists(
    client: &reqwest::Client,
    api_base: &str,
    token: &str,
) -> Result<Vec<RemotePlaylist>, ProviderError> {
    let saved: Paging<serde_json::Value> = send_json(
        client
            .get(format!("{api_base}/me/tracks?limit=1"))
            .bearer_auth(token),
    )
    .await?;
    let mut playlists = vec![RemotePlaylist {
        id: "liked".to_string(),
        name: "Liked Songs".to_string(),
        track_count: saved.total,
        image_url: None,
        owner: None,
        url: Some("https://open.spotify.com/collection/tracks".to_string()),
        kind: "saved_tracks",
    }];
    let mut next = Some(format!("{api_base}/me/playlists?limit=50"));
    while let Some(url) = next.take() {
        let page: Paging<Option<SimplePlaylist>> =
            send_json(client.get(&url).bearer_auth(token)).await?;
        playlists.extend(page.items.into_iter().flatten().map(|playlist| {
            RemotePlaylist {
                id: playlist.id,
                name: playlist.name,
                track_count: playlist
                    .items
                    .and_then(|total| total.total)
                    .or(playlist.tracks.and_then(|total| total.total)),
                image_url: playlist
                    .images
                    .and_then(|images| images.into_iter().next())
                    .map(|image| image.url),
                owner: playlist.owner.and_then(|owner| owner.display_name),
                url: playlist.external_urls.spotify,
                kind: "playlist",
            }
        }));
        if playlists.len() >= 1_000 {
            break;
        }
        next = page.next.filter(|next| same_origin(next, api_base));
    }
    Ok(playlists)
}

async fn collect_tracks<T, F>(
    client: &reqwest::Client,
    api_base: &str,
    token: &str,
    first: String,
    mut unwrap: F,
    progress: &mut impl FnMut(usize),
) -> Result<Vec<ImportedTrack>, ProviderError>
where
    T: serde::de::DeserializeOwned,
    F: FnMut(T) -> Option<SpotifyTrack>,
{
    let mut tracks = Vec::new();
    let mut next = Some(first);
    while let Some(url) = next.take() {
        let page: Paging<T> = send_json(client.get(&url).bearer_auth(token)).await?;
        tracks.extend(
            page.items
                .into_iter()
                .filter_map(&mut unwrap)
                .filter_map(to_imported),
        );
        progress(tracks.len());
        if tracks.len() >= MAX_IMPORT_TRACKS {
            break;
        }
        next = page.next.filter(|next| same_origin(next, api_base));
    }
    tracks.truncate(MAX_IMPORT_TRACKS);
    Ok(tracks)
}

#[derive(Deserialize)]
struct SavedEntry {
    track: Option<SpotifyTrack>,
}

pub async fn fetch_saved_tracks(
    client: &reqwest::Client,
    api_base: &str,
    token: &str,
    mut progress: impl FnMut(usize),
) -> Result<ImportedPlaylist, ProviderError> {
    let tracks = collect_tracks(
        client,
        api_base,
        token,
        format!("{api_base}/me/tracks?limit=50"),
        |entry: SavedEntry| entry.track,
        &mut progress,
    )
    .await?;
    Ok(ImportedPlaylist {
        name: "Liked Songs".to_string(),
        source_url: Some("https://open.spotify.com/collection/tracks".to_string()),
        tracks,
    })
}

#[derive(Deserialize)]
struct PlaylistMeta {
    name: String,
    #[serde(default)]
    external_urls: ExternalUrls,
}

pub async fn fetch_playlist(
    client: &reqwest::Client,
    api_base: &str,
    token: &str,
    playlist_id: &str,
    mut progress: impl FnMut(usize),
) -> Result<ImportedPlaylist, ProviderError> {
    let meta: PlaylistMeta = send_json(
        client
            .get(format!(
                "{api_base}/playlists/{playlist_id}?fields=name,external_urls"
            ))
            .bearer_auth(token),
    )
    .await?;
    let unwrap = |entry: PlaylistEntry| entry.item.or(entry.track);
    // Newer apps read `/items`; older ones `/tracks`. Try both.
    let tracks = match collect_tracks(
        client,
        api_base,
        token,
        format!("{api_base}/playlists/{playlist_id}/items?limit=100"),
        unwrap,
        &mut progress,
    )
    .await
    {
        Err(ProviderError::NotFound) => {
            collect_tracks(
                client,
                api_base,
                token,
                format!("{api_base}/playlists/{playlist_id}/tracks?limit=100"),
                unwrap,
                &mut progress,
            )
            .await?
        }
        other => other?,
    };
    Ok(ImportedPlaylist {
        name: meta.name,
        source_url: meta.external_urls.spotify,
        tracks,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Device {
    pub id: Option<String>,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub is_active: bool,
    #[serde(default)]
    pub is_restricted: bool,
    pub volume_percent: Option<u8>,
}

#[derive(Deserialize)]
struct Devices {
    #[serde(default)]
    devices: Vec<Device>,
}

pub async fn devices(
    client: &reqwest::Client,
    api_base: &str,
    token: &str,
) -> Result<Vec<Device>, ProviderError> {
    let response: Devices = send_json(
        client
            .get(format!("{api_base}/me/player/devices"))
            .bearer_auth(token),
    )
    .await?;
    Ok(response.devices)
}

/// What Parson knows about a local track when looking for its Spotify twin.
#[derive(Clone, Debug, Default)]
pub struct HandoffTrack {
    pub title: String,
    pub artist: String,
    pub isrc: Option<String>,
}

fn quoted(value: &str) -> String {
    value.replace('"', " ")
}

/// Search queries to try, most precise first.
pub fn search_queries(track: &HandoffTrack) -> Vec<String> {
    let mut queries = Vec::new();
    if let Some(isrc) = track.isrc.as_deref().filter(|isrc| isrc.len() == 12) {
        queries.push(format!("isrc:{isrc}"));
    }
    if !track.title.trim().is_empty() {
        if track.artist.trim().is_empty() {
            queries.push(format!("track:\"{}\"", quoted(&track.title)));
        } else {
            queries.push(format!(
                "track:\"{}\" artist:\"{}\"",
                quoted(&track.title),
                quoted(&track.artist)
            ));
        }
    }
    queries
}

#[derive(Deserialize)]
struct SearchResponse {
    tracks: Option<Paging<SearchTrack>>,
}

#[derive(Deserialize)]
struct SearchTrack {
    uri: String,
}

pub async fn find_track_uri(
    client: &reqwest::Client,
    api_base: &str,
    token: &str,
    track: &HandoffTrack,
) -> Result<Option<String>, ProviderError> {
    for query in search_queries(track) {
        let response: SearchResponse = send_json(
            client
                .get(format!("{api_base}/search"))
                .query(&[("q", query.as_str()), ("type", "track"), ("limit", "1")])
                .bearer_auth(token),
        )
        .await?;
        if let Some(uri) = response
            .tracks
            .and_then(|page| page.items.into_iter().next())
            .map(|track| track.uri)
        {
            return Ok(Some(uri));
        }
    }
    Ok(None)
}

pub async fn start_playback(
    client: &reqwest::Client,
    api_base: &str,
    token: &str,
    device_id: Option<&str>,
    uris: &[String],
    position_ms: u64,
) -> Result<(), ProviderError> {
    let mut request = client
        .put(format!("{api_base}/me/player/play"))
        .bearer_auth(token)
        .json(&serde_json::json!({ "uris": uris, "position_ms": position_ms }));
    if let Some(device_id) = device_id {
        request = request.query(&[("device_id", device_id)]);
    }
    let (status, body) = send_bytes(request).await?;
    if status.is_success() {
        Ok(())
    } else if status == reqwest::StatusCode::NOT_FOUND {
        Err(ProviderError::Invalid(
            "No active Spotify device. Open Spotify on the device first.".to_string(),
        ))
    } else {
        Err(status_error(status, &String::from_utf8_lossy(&body)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_unpadded_base64url_sha256() {
        // Cross-checked with Python: urlsafe_b64encode(sha256(verifier)).rstrip("=").
        assert_eq!(
            code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r7wW1gFWFOEjXk"),
            "bwWFMyPfdG9qreDhH2lmftFx_dFeLDalzcT1gb_j68g"
        );
        let verifier = code_verifier();
        assert!((43..=128).contains(&verifier.len()));
        assert!(
            verifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
        assert_ne!(code_verifier(), verifier);
    }

    #[test]
    fn authorize_url_carries_pkce_and_minimal_scopes() {
        let url = authorize_url(
            ACCOUNTS_BASE,
            "0123456789abcdef0123456789abcdef",
            "http://127.0.0.1:1993/imports",
            "state-1",
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r7wW1gFWFOEjXk",
        );
        assert!(url.starts_with("https://accounts.spotify.com/authorize?response_type=code&"));
        assert!(url.contains("code_challenge=bwWFMyPfdG9qreDhH2lmftFx_dFeLDalzcT1gb_j68g"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A1993%2Fimports"));
        assert!(url.contains("scope=playlist-read-private%20"));
        assert!(
            !url.contains("streaming"),
            "Mujin Music never requests audio streaming"
        );
    }

    #[test]
    fn redirect_uris_follow_spotify_rules() {
        assert!(valid_redirect_uri("https://music.example.com/imports"));
        assert!(valid_redirect_uri("http://127.0.0.1:1993/imports"));
        assert!(valid_redirect_uri("http://[::1]:1993/imports"));
        assert!(!valid_redirect_uri("http://localhost:1993/imports"));
        assert!(!valid_redirect_uri("http://192.168.1.4:1993/imports"));
        assert!(!valid_redirect_uri("http://127.0.0.1.evil.com/imports"));
        assert!(valid_client_id("0123456789abcdef0123456789abcdef"));
        assert!(!valid_client_id("not-a-client-id"));
    }

    #[test]
    fn playlist_entries_read_both_shapes_and_skip_episodes() {
        let page: Paging<PlaylistEntry> = serde_json::from_value(serde_json::json!({
            "items": [
                {"track": {"id": "1", "name": "Song", "duration_ms": 1000, "type": "track",
                  "artists": [{"name": "A"}, {"name": "B"}], "album": {"name": "Album"},
                  "external_ids": {"isrc": "USABC1234567"},
                  "external_urls": {"spotify": "https://open.spotify.com/track/1"}}},
                {"item": {"id": "2", "name": "Newer", "type": "track", "artists": []}},
                {"track": {"id": "3", "name": "Episode", "type": "episode"}},
                {"track": null},
                {"track": {"id": null, "name": "Local file", "type": "track",
                  "artists": [{"name": "Me"}], "album": {"name": ""}}}
            ],
            "next": null
        }))
        .unwrap();
        let tracks: Vec<ImportedTrack> = page
            .items
            .into_iter()
            .filter_map(|entry| entry.item.or(entry.track))
            .filter_map(to_imported)
            .collect();
        assert_eq!(tracks.len(), 3);
        assert_eq!(tracks[0].track.artists, vec!["A", "B"]);
        assert_eq!(tracks[0].track.isrc.as_deref(), Some("USABC1234567"));
        assert_eq!(tracks[1].track.title, "Newer");
        assert_eq!(tracks[2].external_id, None);
        assert_eq!(tracks[2].track.album, None);
    }

    #[test]
    fn handoff_search_prefers_isrc_and_strips_quotes() {
        let queries = search_queries(&HandoffTrack {
            title: "Say \"Hi\"".into(),
            artist: "Band".into(),
            isrc: Some("USABC1234567".into()),
        });
        assert_eq!(
            queries,
            vec![
                "isrc:USABC1234567".to_string(),
                "track:\"Say  Hi \" artist:\"Band\"".to_string()
            ]
        );
        assert!(search_queries(&HandoffTrack::default()).is_empty());
    }
}
