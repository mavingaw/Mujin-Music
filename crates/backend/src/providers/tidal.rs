//! TIDAL: OAuth 2.0 device authorization, catalog, favorites and playback.
//!
//! Endpoints follow TIDAL's public v1 API. The listener supplies the OAuth
//! client id (and secret, when their client has one); Parson ships none.

use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use futures::future::BoxFuture;
use regex::Regex;
use serde::{Deserialize, Deserializer, Serialize};

use super::artwork::ArtworkMode;
use super::transport::{ApiRequest, Transport};
use super::{
    AlbumRef, ArtistRef, ArtistSummary, AudioFormat, BrowsePage, BrowseSection, ClientCredentials,
    Credit, EditorialSection, EditorialText, ItemKind, MAX_PLAYLIST_TRACKS, PlaylistPage,
    ProviderAlbum, ProviderArtist, ProviderContext, ProviderError, ProviderGenre, ProviderId,
    ProviderPlaylist, ProviderResult, ProviderSession, ProviderTrack, QualityOption, Radio,
    RadioOrigin, RadioSeed, ResolvedStream, SearchResults, StreamQuality, StreamSource,
    StreamingProvider, decode_json, entity_id, plain_text, push_credit, status_error, unix_now,
};

pub const AUTH_BASE: &str = "https://auth.tidal.com/v1/oauth2";
pub const API_BASE: &str = "https://api.tidal.com/v1";
const SCOPE: &str = "r_usr w_usr w_sub";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
const MAX_DASH_SEGMENTS: usize = 4096;

static QUALITY_OPTIONS: [QualityOption; 4] = [
    QualityOption {
        id: StreamQuality::HiResMax,
        label: "Max: 24-Bit / up to 192 kHz",
        detail: "Hi-Res FLAC where the album has it, otherwise CD quality",
    },
    QualityOption {
        id: StreamQuality::Lossless,
        label: "High: 16-Bit / 44.1 kHz",
        detail: "FLAC, CD quality",
    },
    QualityOption {
        id: StreamQuality::High,
        label: "Low: AAC 320 kbps",
        detail: "Lossy, for slower connections",
    },
    QualityOption {
        id: StreamQuality::Low,
        label: "Low: AAC 96 kbps",
        detail: "Lossy, for very slow or metered connections",
    },
];

static EDITORIAL_SECTIONS: [EditorialSection; 3] = [
    EditorialSection {
        id: BrowseSection::NewReleases,
        label: "New",
        by_genre: false,
    },
    EditorialSection {
        id: BrowseSection::EditorsPicks,
        label: "Recommended",
        by_genre: false,
    },
    EditorialSection {
        id: BrowseSection::Popular,
        label: "Top",
        by_genre: false,
    },
];

pub struct TidalClient {
    transport: Arc<dyn Transport>,
    artwork: ArtworkMode,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct TGenre {
    name: String,
    path: String,
    has_albums: Option<bool>,
    image: Option<String>,
}

/// A pending device authorization. Stored sealed until the listener approves
/// it at `verification_uri`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceAuthorization {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_at: i64,
    pub interval: u64,
}

#[derive(Debug, PartialEq)]
pub enum DevicePoll {
    Pending,
    SlowDown,
    Approved(ProviderSession),
    Expired,
    Denied,
}

fn de_id<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::String(value) => value,
        serde_json::Value::Number(value) => value.to_string(),
        _ => String::new(),
    })
}

fn de_opt_id<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Ok(match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::String(value) => Some(value),
        serde_json::Value::Number(value) => Some(value.to_string()),
        _ => None,
    })
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct TArtist {
    #[serde(deserialize_with = "de_id")]
    id: String,
    name: String,
    picture: Option<String>,
    url: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TMediaMetadata {
    tags: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct TAlbum {
    #[serde(deserialize_with = "de_id")]
    id: String,
    title: String,
    version: Option<String>,
    duration: f64,
    number_of_tracks: u32,
    release_date: Option<String>,
    copyright: Option<String>,
    cover: Option<String>,
    audio_quality: Option<String>,
    media_metadata: TMediaMetadata,
    explicit: bool,
    artist: Option<TArtist>,
    artists: Vec<TArtist>,
    url: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct TAlbumRef {
    #[serde(deserialize_with = "de_id")]
    id: String,
    title: String,
    cover: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct TTrack {
    #[serde(deserialize_with = "de_id")]
    id: String,
    title: String,
    version: Option<String>,
    duration: f64,
    track_number: u32,
    volume_number: u32,
    explicit: bool,
    isrc: Option<String>,
    audio_quality: Option<String>,
    media_metadata: TMediaMetadata,
    artist: Option<TArtist>,
    artists: Vec<TArtist>,
    album: Option<TAlbumRef>,
    stream_ready: Option<bool>,
    allow_streaming: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TPage<T> {
    #[serde(default = "Vec::new")]
    items: Vec<T>,
    #[serde(default)]
    total_number_of_items: Option<u32>,
}

impl<T> Default for TPage<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            total_number_of_items: None,
        }
    }
}

/// `/albums/{id}/review` and `/artists/{id}/bio`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TEditorial {
    text: Option<String>,
    source: Option<String>,
}

impl TEditorial {
    fn into_text(self) -> Option<EditorialText> {
        Some(EditorialText {
            text: plain_text(self.text.as_deref()?)?,
            source: non_empty(self.source),
        })
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TContributor {
    name: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TCredit {
    #[serde(rename = "type")]
    role: String,
    contributors: Vec<TContributor>,
}

/// One entry of `/albums/{id}/items/credits`: a track and its credits.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TCreditedItem {
    item: TCreditedTrack,
    credits: Vec<TCredit>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TCreditedTrack {
    #[serde(deserialize_with = "de_id")]
    id: String,
}

fn credits_from(raw: Vec<TCredit>) -> Vec<Credit> {
    let mut credits = Vec::new();
    for credit in raw {
        for contributor in credit.contributors {
            push_credit(&mut credits, &credit.role, &contributor.name);
        }
    }
    credits
}

/// Favorites and some editorial lists wrap each entry as `{ "item": ... }`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum MaybeWrapped<T> {
    Wrapped { item: T },
    Bare(T),
}

impl<T> MaybeWrapped<T> {
    fn into_inner(self) -> T {
        match self {
            Self::Wrapped { item } | Self::Bare(item) => item,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TCreator {
    #[serde(deserialize_with = "de_opt_id")]
    id: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct TPlaylist {
    uuid: String,
    title: String,
    description: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    creator: Option<TCreator>,
    number_of_tracks: u32,
    duration: f64,
    square_image: Option<String>,
    image: Option<String>,
    last_updated: Option<String>,
}

/// `playlistsAndFavoritePlaylists` wraps each playlist with how the
/// listener relates to it.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TPlaylistEntry {
    Wrapped {
        #[serde(rename = "type", default)]
        relation: Option<String>,
        playlist: TPlaylist,
    },
    Bare(TPlaylist),
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TPlaylistItem {
    #[serde(rename = "type")]
    kind: Option<String>,
    item: Option<serde_json::Value>,
}

/// TIDAL's `2026-10-20T21:14:03.000+0000` timestamps.
fn tidal_time(value: Option<&str>) -> Option<i64> {
    chrono::DateTime::parse_from_str(value?, "%Y-%m-%dT%H:%M:%S%.f%z")
        .ok()
        .map(|time| time.timestamp())
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TSearch {
    artists: TPage<TArtist>,
    albums: TPage<TAlbum>,
    tracks: TPage<TTrack>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TPlayback {
    #[serde(default)]
    audio_quality: Option<String>,
    #[serde(default)]
    asset_presentation: Option<String>,
    manifest_mime_type: String,
    manifest: String,
    #[serde(default)]
    bit_depth: Option<u8>,
    #[serde(default)]
    sample_rate: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TBtsManifest {
    mime_type: String,
    #[serde(default)]
    codecs: String,
    #[serde(default)]
    encryption_type: Option<String>,
    urls: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct TUser {
    #[serde(deserialize_with = "de_opt_id")]
    user_id: Option<String>,
    username: Option<String>,
    email: Option<String>,
    country_code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    user: Option<TUser>,
    #[serde(default, deserialize_with = "de_opt_id")]
    user_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TDeviceResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: Option<String>,
    expires_in: i64,
    #[serde(default)]
    interval: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TOAuthError {
    error: String,
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// The best format an album or track is offered in.
fn catalog_format(audio_quality: Option<&str>, tags: &[String]) -> Option<AudioFormat> {
    let has = |tag: &str| tags.iter().any(|value| value == tag);
    if has("HIRES_LOSSLESS") || audio_quality == Some("HI_RES_LOSSLESS") {
        return Some(AudioFormat::lossless("flac", Some(24), None));
    }
    if has("LOSSLESS") || matches!(audio_quality, Some("LOSSLESS" | "HI_RES")) {
        return Some(AudioFormat::lossless("flac", Some(16), Some(44_100)));
    }
    match audio_quality {
        Some("HIGH") => Some(AudioFormat::lossy("aac", 320)),
        Some("LOW") => Some(AudioFormat::lossy("aac", 96)),
        _ => None,
    }
}

impl TidalClient {
    pub fn new(transport: Arc<dyn Transport>, artwork: ArtworkMode) -> Self {
        Self { transport, artwork }
    }

    fn artist_ref(&self, artist: &TArtist) -> ArtistRef {
        ArtistRef {
            id: entity_id(ProviderId::Tidal, &artist.id),
            name: artist.name.clone(),
        }
    }

    fn normalize_album(&self, raw: TAlbum) -> ProviderAlbum {
        let artists = raw
            .artists
            .iter()
            .filter(|artist| !artist.id.is_empty())
            .map(|artist| self.artist_ref(artist))
            .collect::<Vec<_>>();
        let artist = raw
            .artist
            .as_ref()
            .map(|artist| self.artist_ref(artist))
            .or_else(|| artists.first().cloned())
            .unwrap_or_default();
        ProviderAlbum {
            id: entity_id(ProviderId::Tidal, &raw.id),
            provider: ProviderId::Tidal,
            format: catalog_format(raw.audio_quality.as_deref(), &raw.media_metadata.tags),
            title: raw.title,
            version: non_empty(raw.version),
            artist,
            artists,
            cover_url: self.artwork.tidal(raw.cover.as_deref(), 640),
            release_date: non_empty(raw.release_date),
            track_count: raw.number_of_tracks,
            duration: raw.duration,
            explicit: raw.explicit,
            label: None,
            genre: None,
            copyright: non_empty(raw.copyright),
            web_url: Some(format!("https://tidal.com/browse/album/{}", raw.id)),
            tracks: Vec::new(),
            in_library: false,
            review: None,
            awards: Vec::new(),
        }
    }

    fn normalize_track(&self, raw: TTrack) -> ProviderTrack {
        let artists = raw
            .artists
            .iter()
            .filter(|artist| !artist.id.is_empty())
            .map(|artist| self.artist_ref(artist))
            .collect::<Vec<_>>();
        let artist = raw
            .artist
            .as_ref()
            .map(|artist| self.artist_ref(artist))
            .or_else(|| artists.first().cloned())
            .unwrap_or_default();
        ProviderTrack {
            id: entity_id(ProviderId::Tidal, &raw.id),
            provider: ProviderId::Tidal,
            format: catalog_format(raw.audio_quality.as_deref(), &raw.media_metadata.tags),
            title: raw.title,
            version: non_empty(raw.version),
            artist,
            artists,
            album: raw.album.as_ref().map(|album| AlbumRef {
                id: entity_id(ProviderId::Tidal, &album.id),
                title: album.title.clone(),
                cover_url: self.artwork.tidal(album.cover.as_deref(), 640),
            }),
            track_number: raw.track_number,
            disc_number: raw.volume_number.max(1),
            duration: raw.duration,
            explicit: raw.explicit,
            isrc: non_empty(raw.isrc),
            streamable: raw.allow_streaming.unwrap_or(true) && raw.stream_ready.unwrap_or(true),
            in_library: false,
            credits: Vec::new(),
        }
    }

    fn artist_summary(&self, raw: &TArtist) -> ArtistSummary {
        ArtistSummary {
            id: entity_id(ProviderId::Tidal, &raw.id),
            provider: ProviderId::Tidal,
            name: raw.name.clone(),
            picture_url: self.artwork.tidal(raw.picture.as_deref(), 480),
        }
    }

    fn client_form(request: ApiRequest, credentials: &ClientCredentials) -> ApiRequest {
        let request = request.form("client_id", credentials.client_id.clone());
        match credentials
            .client_secret
            .as_deref()
            .filter(|secret| !secret.is_empty())
        {
            Some(secret) => request.form("client_secret", secret),
            None => request,
        }
    }

    /// Starts the OAuth 2.0 device authorization flow (RFC 8628).
    pub async fn start_device_authorization(
        &self,
        credentials: &ClientCredentials,
    ) -> ProviderResult<DeviceAuthorization> {
        let request = ApiRequest::post(format!("{AUTH_BASE}/device_authorization"))
            .form("client_id", credentials.client_id.clone())
            .form("scope", SCOPE);
        let reply = self.transport.send(request).await?;
        match reply.status {
            200..=299 => {}
            400 | 401 | 403 => {
                return Err(ProviderError::InvalidCredentials(
                    "TIDAL did not accept this client ID.".into(),
                ));
            }
            status => return Err(status_error(status)),
        }
        let device: TDeviceResponse = decode_json(&reply.body)?;
        let verification_uri = if device.verification_uri.starts_with("http") {
            device.verification_uri
        } else {
            format!("https://{}", device.verification_uri)
        };
        let verification_uri_complete = device.verification_uri_complete.map(|uri| {
            if uri.starts_with("http") {
                uri
            } else {
                format!("https://{uri}")
            }
        });
        Ok(DeviceAuthorization {
            device_code: device.device_code,
            user_code: device.user_code,
            verification_uri,
            verification_uri_complete,
            expires_at: unix_now() + device.expires_in.clamp(30, 3600),
            interval: device.interval.unwrap_or(5).clamp(2, 30),
        })
    }

    pub async fn poll_device_authorization(
        &self,
        credentials: &ClientCredentials,
        pending: &DeviceAuthorization,
    ) -> ProviderResult<DevicePoll> {
        if pending.expires_at <= unix_now() {
            return Ok(DevicePoll::Expired);
        }
        let request =
            Self::client_form(ApiRequest::post(format!("{AUTH_BASE}/token")), credentials)
                .form("device_code", pending.device_code.clone())
                .form("grant_type", DEVICE_GRANT)
                .form("scope", SCOPE);
        let reply = self.transport.send(request).await?;
        if (200..300).contains(&reply.status) {
            let token: TTokenResponse = decode_json(&reply.body)?;
            return Ok(DevicePoll::Approved(session_from_token(token, None)));
        }
        if matches!(reply.status, 400 | 401) {
            let error: TOAuthError = serde_json::from_slice(&reply.body).unwrap_or_default();
            return match error.error.as_str() {
                "authorization_pending" => Ok(DevicePoll::Pending),
                "slow_down" => Ok(DevicePoll::SlowDown),
                "expired_token" => Ok(DevicePoll::Expired),
                "access_denied" => Ok(DevicePoll::Denied),
                _ => Err(ProviderError::InvalidCredentials(
                    "TIDAL rejected this client ID or secret.".into(),
                )),
            };
        }
        Err(status_error(reply.status))
    }

    async fn get(
        &self,
        context: &ProviderContext,
        path: &str,
        query: &[(&str, String)],
    ) -> ProviderResult<Vec<u8>> {
        let mut request = ApiRequest::get(format!("{API_BASE}{path}"))
            .header(
                "authorization",
                format!("Bearer {}", context.session.access_token),
            )
            .query(
                "countryCode",
                context
                    .session
                    .country_code
                    .clone()
                    .unwrap_or_else(|| "US".to_string()),
            );
        for (key, value) in query {
            request = request.query(key, value.clone());
        }
        let reply = self.transport.send(request).await?;
        if (200..300).contains(&reply.status) {
            Ok(reply.body)
        } else {
            Err(status_error(reply.status))
        }
    }

    fn normalize_playlist(
        &self,
        raw: TPlaylist,
        relation: Option<&str>,
        user_id: &str,
    ) -> ProviderPlaylist {
        let creator = raw.creator.unwrap_or_default();
        let own = relation == Some("USER_CREATED")
            || creator.id.as_deref().is_some_and(|id| id == user_id);
        let owner = non_empty(creator.name).or_else(|| match raw.kind.as_deref() {
            Some("EDITORIAL") => Some("TIDAL".to_string()),
            _ => None,
        });
        ProviderPlaylist {
            id: entity_id(ProviderId::Tidal, &raw.uuid),
            provider: ProviderId::Tidal,
            title: raw.title,
            description: non_empty(raw.description),
            owner,
            own,
            track_count: raw.number_of_tracks,
            duration: raw.duration,
            cover_url: self
                .artwork
                .tidal(raw.square_image.as_deref().or(raw.image.as_deref()), 640),
            updated_at: tidal_time(raw.last_updated.as_deref()),
            web_url: Some(format!("https://tidal.com/browse/playlist/{}", raw.uuid)),
            tracks: Vec::new(),
        }
    }

    async fn album_page(
        &self,
        context: &ProviderContext,
        path: &str,
        offset: u32,
        limit: u32,
        extra: &[(&str, String)],
    ) -> ProviderResult<BrowsePage> {
        let mut query = vec![("offset", offset.to_string()), ("limit", limit.to_string())];
        query.extend(extra.iter().cloned());
        let page: TPage<MaybeWrapped<TAlbum>> =
            decode_json(&self.get(context, path, &query).await?)?;
        Ok(BrowsePage {
            albums: page
                .items
                .into_iter()
                .map(|item| self.normalize_album(item.into_inner()))
                .collect(),
            tracks: Vec::new(),
            total: page.total_number_of_items,
        })
    }
}

fn session_from_token(
    token: TTokenResponse,
    previous: Option<&ProviderSession>,
) -> ProviderSession {
    let user = token.user.unwrap_or_default();
    let user_id = user
        .user_id
        .or(token.user_id)
        .or_else(|| previous.map(|session| session.user_id.clone()))
        .unwrap_or_default();
    let account_name = non_empty(user.username)
        .or_else(|| non_empty(user.email))
        .or_else(|| previous.map(|session| session.account_name.clone()))
        .unwrap_or_else(|| "TIDAL listener".to_string());
    ProviderSession {
        access_token: token.access_token,
        refresh_token: token
            .refresh_token
            .or_else(|| previous.and_then(|session| session.refresh_token.clone())),
        expires_at: token.expires_in.map(|seconds| unix_now() + seconds.max(0)),
        user_id,
        account_name,
        country_code: non_empty(user.country_code)
            .or_else(|| previous.and_then(|session| session.country_code.clone())),
        plan: previous.and_then(|session| session.plan.clone()),
    }
}

fn audio_quality_parameter(quality: StreamQuality) -> &'static str {
    match quality {
        StreamQuality::HiResMax | StreamQuality::HiRes96 => "HI_RES_LOSSLESS",
        StreamQuality::Lossless => "LOSSLESS",
        StreamQuality::High => "HIGH",
        StreamQuality::Low => "LOW",
    }
}

fn xml_attribute(attributes: &str, name: &str) -> Option<String> {
    let pattern = Regex::new(&format!(r#"\b{name}="([^"]*)""#)).ok()?;
    pattern
        .captures(attributes)
        .and_then(|captures| captures.get(1))
        .map(|value| {
            value
                .as_str()
                .replace("&amp;", "&")
                .replace("&quot;", "\"")
                .replace("&lt;", "<")
                .replace("&gt;", ">")
        })
}

#[derive(Debug, PartialEq)]
pub(crate) struct DashRepresentation {
    pub codecs: String,
    pub sample_rate: Option<u32>,
    pub init_url: Option<String>,
    pub segment_urls: Vec<String>,
}

/// Reads the single audio representation TIDAL returns for a track: its
/// codec and the initialization and media segment URLs of its
/// `SegmentTemplate`/`SegmentTimeline`.
pub(crate) fn parse_dash_manifest(xml: &str) -> ProviderResult<DashRepresentation> {
    let invalid = |message: &str| ProviderError::Decode(format!("DASH manifest: {message}"));
    let representation = Regex::new(r"<Representation\b([^>]*)>")
        .ok()
        .and_then(|pattern| pattern.captures(xml))
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().to_string())
        .ok_or_else(|| invalid("no Representation"))?;
    let template = Regex::new(r"<SegmentTemplate\b([^>]*)>")
        .ok()
        .and_then(|pattern| pattern.captures(xml))
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().to_string())
        .ok_or_else(|| invalid("no SegmentTemplate"))?;
    let representation_id = xml_attribute(&representation, "id").unwrap_or_default();
    let media = xml_attribute(&template, "media").ok_or_else(|| invalid("no media template"))?;
    let start_number = xml_attribute(&template, "startNumber")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1);
    let segments = Regex::new(r"<S\b([^>]*)/?>")
        .map_err(|_| invalid("timeline pattern"))?
        .captures_iter(xml)
        .filter_map(|captures| captures.get(1))
        .map(|attributes| {
            xml_attribute(attributes.as_str(), "r")
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(0)
                .max(0) as usize
                + 1
        })
        .sum::<usize>();
    if segments == 0 {
        return Err(invalid("empty SegmentTimeline"));
    }
    if segments > MAX_DASH_SEGMENTS {
        return Err(invalid("too many segments"));
    }
    let substitute = |template: &str, number: Option<usize>| {
        let value = template.replace("$RepresentationID$", &representation_id);
        match number {
            Some(number) => value.replace("$Number$", &number.to_string()),
            None => value,
        }
    };
    let init_url = xml_attribute(&template, "initialization").map(|value| substitute(&value, None));
    let segment_urls = (start_number..start_number + segments)
        .map(|number| substitute(&media, Some(number)))
        .collect::<Vec<_>>();
    if segment_urls
        .iter()
        .chain(init_url.iter())
        .any(|url| !url.starts_with("https://"))
    {
        return Err(invalid("segment URLs must use HTTPS"));
    }
    Ok(DashRepresentation {
        codecs: xml_attribute(&representation, "codecs").unwrap_or_default(),
        sample_rate: xml_attribute(&representation, "audioSamplingRate")
            .and_then(|value| value.parse().ok()),
        init_url,
        segment_urls,
    })
}

fn delivered_format(
    codecs: &str,
    bit_depth: Option<u8>,
    sample_rate: Option<u32>,
    audio_quality: Option<&str>,
) -> AudioFormat {
    let codec = codecs.to_ascii_lowercase();
    if codec.contains("flac") {
        AudioFormat::lossless("flac", bit_depth.or(Some(16)), sample_rate.or(Some(44_100)))
    } else if codec.contains("alac") {
        AudioFormat::lossless("alac", bit_depth, sample_rate)
    } else {
        AudioFormat::lossy(
            "aac",
            if audio_quality == Some("LOW") {
                96
            } else {
                320
            },
        )
    }
}

impl StreamingProvider for TidalClient {
    fn id(&self) -> ProviderId {
        ProviderId::Tidal
    }

    fn quality_options(&self) -> &'static [QualityOption] {
        &QUALITY_OPTIONS
    }

    fn default_quality(&self) -> StreamQuality {
        StreamQuality::HiResMax
    }

    fn search<'a>(
        &'a self,
        context: &'a ProviderContext,
        query: &'a str,
        limit: u32,
    ) -> BoxFuture<'a, ProviderResult<SearchResults>> {
        Box::pin(async move {
            let body = self
                .get(
                    context,
                    "/search",
                    &[
                        ("query", query.to_string()),
                        ("types", "ARTISTS,ALBUMS,TRACKS".to_string()),
                        ("limit", limit.to_string()),
                        ("offset", "0".to_string()),
                    ],
                )
                .await?;
            let search: TSearch = decode_json(&body)?;
            Ok(SearchResults {
                albums: search
                    .albums
                    .items
                    .into_iter()
                    .map(|album| self.normalize_album(album))
                    .collect(),
                tracks: search
                    .tracks
                    .items
                    .into_iter()
                    .map(|track| self.normalize_track(track))
                    .collect(),
                artists: search
                    .artists
                    .items
                    .iter()
                    .map(|artist| self.artist_summary(artist))
                    .collect(),
            })
        })
    }

    fn album<'a>(
        &'a self,
        context: &'a ProviderContext,
        raw_id: &'a str,
    ) -> BoxFuture<'a, ProviderResult<ProviderAlbum>> {
        Box::pin(async move {
            let album_path = format!("/albums/{raw_id}");
            let tracks_path = format!("/albums/{raw_id}/tracks");
            let tracks_query = [("limit", "100".to_string()), ("offset", "0".to_string())];
            let review_path = format!("/albums/{raw_id}/review");
            let credits_path = format!("/albums/{raw_id}/items/credits");
            let credits_query = [
                ("limit", "100".to_string()),
                ("offset", "0".to_string()),
                ("replace", "true".to_string()),
                ("includeContributors", "true".to_string()),
            ];
            let (album, tracks, review, credits) = futures::join!(
                self.get(context, &album_path, &[]),
                self.get(context, &tracks_path, &tracks_query),
                self.get(context, &review_path, &[]),
                self.get(context, &credits_path, &credits_query)
            );
            let mut album = self.normalize_album(decode_json::<TAlbum>(&album?)?);
            let tracks: TPage<MaybeWrapped<TTrack>> = decode_json(&tracks?)?;
            album.tracks = tracks
                .items
                .into_iter()
                .map(|track| self.normalize_track(track.into_inner()))
                .collect();
            // Many albums have no review (TIDAL answers 404) and credits are
            // a nicety: the album page works without either.
            album.review = review
                .ok()
                .and_then(|body| serde_json::from_slice::<TEditorial>(&body).ok())
                .and_then(TEditorial::into_text);
            if let Some(credited) = credits
                .ok()
                .and_then(|body| serde_json::from_slice::<TPage<TCreditedItem>>(&body).ok())
            {
                let mut by_track = credited
                    .items
                    .into_iter()
                    .map(|entry| {
                        (
                            entity_id(ProviderId::Tidal, &entry.item.id),
                            credits_from(entry.credits),
                        )
                    })
                    .collect::<std::collections::HashMap<_, _>>();
                for track in &mut album.tracks {
                    track.credits = by_track.remove(&track.id).unwrap_or_default();
                }
            }
            Ok(album)
        })
    }

    fn artist<'a>(
        &'a self,
        context: &'a ProviderContext,
        raw_id: &'a str,
    ) -> BoxFuture<'a, ProviderResult<ProviderArtist>> {
        Box::pin(async move {
            let artist_path = format!("/artists/{raw_id}");
            let albums_path = format!("/artists/{raw_id}/albums");
            let top_path = format!("/artists/{raw_id}/toptracks");
            let albums_query = [("limit", "50".to_string()), ("offset", "0".to_string())];
            let top_query = [("limit", "10".to_string()), ("offset", "0".to_string())];
            let bio_path = format!("/artists/{raw_id}/bio");
            let similar_path = format!("/artists/{raw_id}/similar");
            let similar_query = [("limit", "12".to_string()), ("offset", "0".to_string())];
            let (artist, albums, top, bio, similar) = futures::join!(
                self.get(context, &artist_path, &[]),
                self.get(context, &albums_path, &albums_query),
                self.get(context, &top_path, &top_query),
                self.get(context, &bio_path, &[]),
                self.get(context, &similar_path, &similar_query)
            );
            let bio = bio
                .ok()
                .and_then(|body| serde_json::from_slice::<TEditorial>(&body).ok())
                .and_then(TEditorial::into_text);
            let similar: TPage<TArtist> = similar
                .ok()
                .and_then(|body| serde_json::from_slice(&body).ok())
                .unwrap_or_default();
            let artist: TArtist = decode_json(&artist?)?;
            let albums: TPage<TAlbum> = decode_json(&albums?)?;
            // Top tracks are a nicety; an artist page still works without them.
            let top: TPage<TTrack> = top
                .ok()
                .and_then(|body| serde_json::from_slice(&body).ok())
                .unwrap_or_default();
            Ok(ProviderArtist {
                id: entity_id(ProviderId::Tidal, &artist.id),
                provider: ProviderId::Tidal,
                picture_url: self.artwork.tidal(artist.picture.as_deref(), 750),
                web_url: artist
                    .url
                    .clone()
                    .or_else(|| Some(format!("https://tidal.com/browse/artist/{}", artist.id))),
                name: artist.name,
                albums: albums
                    .items
                    .into_iter()
                    .map(|album| self.normalize_album(album))
                    .collect(),
                top_tracks: top
                    .items
                    .into_iter()
                    .map(|track| self.normalize_track(track))
                    .collect(),
                bio,
                similar: similar
                    .items
                    .iter()
                    .filter(|artist| !artist.id.is_empty())
                    .map(|artist| self.artist_summary(artist))
                    .collect(),
            })
        })
    }

    fn track<'a>(
        &'a self,
        context: &'a ProviderContext,
        raw_id: &'a str,
    ) -> BoxFuture<'a, ProviderResult<ProviderTrack>> {
        Box::pin(async move {
            let body = self.get(context, &format!("/tracks/{raw_id}"), &[]).await?;
            Ok(self.normalize_track(decode_json(&body)?))
        })
    }

    fn browse<'a>(
        &'a self,
        context: &'a ProviderContext,
        section: BrowseSection,
        offset: u32,
        limit: u32,
    ) -> BoxFuture<'a, ProviderResult<BrowsePage>> {
        Box::pin(async move {
            let user = &context.session.user_id;
            match section {
                BrowseSection::NewReleases => {
                    self.album_page(context, "/featured/new/albums", offset, limit, &[])
                        .await
                }
                BrowseSection::Popular => {
                    self.album_page(context, "/featured/top/albums", offset, limit, &[])
                        .await
                }
                BrowseSection::EditorsPicks => {
                    self.album_page(context, "/featured/recommended/albums", offset, limit, &[])
                        .await
                }
                BrowseSection::PressAwards => Err(ProviderError::Unavailable(
                    "TIDAL does not publish a press awards list.".into(),
                )),
                BrowseSection::FavoriteAlbums => {
                    self.album_page(
                        context,
                        &format!("/users/{user}/favorites/albums"),
                        offset,
                        limit,
                        &[
                            ("order", "DATE".to_string()),
                            ("orderDirection", "DESC".to_string()),
                        ],
                    )
                    .await
                }
                BrowseSection::FavoriteTracks => {
                    let body = self
                        .get(
                            context,
                            &format!("/users/{user}/favorites/tracks"),
                            &[
                                ("offset", offset.to_string()),
                                ("limit", limit.to_string()),
                                ("order", "DATE".to_string()),
                                ("orderDirection", "DESC".to_string()),
                            ],
                        )
                        .await?;
                    let page: TPage<MaybeWrapped<TTrack>> = decode_json(&body)?;
                    Ok(BrowsePage {
                        albums: Vec::new(),
                        tracks: page
                            .items
                            .into_iter()
                            .map(|item| self.normalize_track(item.into_inner()))
                            .collect(),
                        total: page.total_number_of_items,
                    })
                }
            }
        })
    }

    fn editorial_sections(&self) -> &'static [EditorialSection] {
        &EDITORIAL_SECTIONS
    }

    fn genres<'a>(
        &'a self,
        context: &'a ProviderContext,
    ) -> BoxFuture<'a, ProviderResult<Vec<ProviderGenre>>> {
        Box::pin(async move {
            let body = self.get(context, "/genres", &[]).await?;
            let genres: Vec<TGenre> = decode_json(&body)?;
            Ok(genres
                .into_iter()
                .filter(|genre| genre.has_albums.unwrap_or(true))
                .filter(|genre| super::valid_raw_id(&genre.path) && !genre.name.trim().is_empty())
                .map(|genre| ProviderGenre {
                    image_url: self.artwork.tidal(genre.image.as_deref(), 640),
                    id: genre.path,
                    provider: ProviderId::Tidal,
                    name: genre.name.trim().to_string(),
                })
                .collect())
        })
    }

    fn genre_albums<'a>(
        &'a self,
        context: &'a ProviderContext,
        genre_id: &'a str,
        _section: BrowseSection,
        offset: u32,
        limit: u32,
    ) -> BoxFuture<'a, ProviderResult<BrowsePage>> {
        // TIDAL's genre pages have one album list; editorial lists are not
        // split by genre.
        Box::pin(async move {
            self.album_page(
                context,
                &format!("/genres/{genre_id}/albums"),
                offset,
                limit,
                &[],
            )
            .await
        })
    }

    fn set_favorite<'a>(
        &'a self,
        context: &'a ProviderContext,
        kind: ItemKind,
        raw_id: &'a str,
        favorite: bool,
    ) -> BoxFuture<'a, ProviderResult<()>> {
        Box::pin(async move {
            let (collection, field) = match kind {
                ItemKind::Album => ("albums", "albumIds"),
                ItemKind::Track => ("tracks", "trackIds"),
            };
            let user = &context.session.user_id;
            let country = context
                .session
                .country_code
                .clone()
                .unwrap_or_else(|| "US".to_string());
            let request = if favorite {
                ApiRequest::post(format!("{API_BASE}/users/{user}/favorites/{collection}"))
                    .form(field, raw_id)
            } else {
                ApiRequest::delete(format!(
                    "{API_BASE}/users/{user}/favorites/{collection}/{raw_id}"
                ))
            }
            .query("countryCode", country)
            .header(
                "authorization",
                format!("Bearer {}", context.session.access_token),
            );
            let reply = self.transport.send(request).await?;
            match reply.status {
                200..=299 => Ok(()),
                // Removing something that is no longer a favorite is fine.
                404 if !favorite => Ok(()),
                status => Err(status_error(status)),
            }
        })
    }

    fn resolve_stream<'a>(
        &'a self,
        context: &'a ProviderContext,
        raw_id: &'a str,
        quality: StreamQuality,
    ) -> BoxFuture<'a, ProviderResult<ResolvedStream>> {
        Box::pin(async move {
            let body = self
                .get(
                    context,
                    &format!("/tracks/{raw_id}/playbackinfopostpaywall"),
                    &[
                        ("audioquality", audio_quality_parameter(quality).to_string()),
                        ("playbackmode", "STREAM".to_string()),
                        ("assetpresentation", "FULL".to_string()),
                    ],
                )
                .await?;
            let playback: TPlayback = decode_json(&body)?;
            if playback.asset_presentation.as_deref() == Some("PREVIEW") {
                return Err(ProviderError::Unavailable(
                    "This TIDAL account can only play previews. Check the subscription.".into(),
                ));
            }
            let manifest = STANDARD
                .decode(playback.manifest.trim())
                .map_err(|_| ProviderError::Decode("manifest is not base64".into()))?;
            let audio_quality = playback.audio_quality.as_deref();
            match playback.manifest_mime_type.as_str() {
                "application/vnd.tidal.bts" => {
                    let manifest: TBtsManifest = decode_json(&manifest)?;
                    if manifest
                        .encryption_type
                        .as_deref()
                        .is_some_and(|value| value != "NONE")
                    {
                        return Err(ProviderError::Unavailable(
                            "TIDAL returned an encrypted stream, which Parson cannot play.".into(),
                        ));
                    }
                    let url = manifest
                        .urls
                        .into_iter()
                        .find(|url| url.starts_with("https://"))
                        .ok_or_else(|| ProviderError::Decode("no HTTPS stream URL".into()))?;
                    Ok(ResolvedStream {
                        format: delivered_format(
                            &manifest.codecs,
                            playback.bit_depth,
                            playback.sample_rate,
                            audio_quality,
                        ),
                        source: StreamSource::Progressive {
                            url,
                            mime: manifest.mime_type,
                        },
                        requested: quality,
                    })
                }
                "application/dash+xml" => {
                    let xml = String::from_utf8(manifest)
                        .map_err(|_| ProviderError::Decode("manifest is not UTF-8".into()))?;
                    let dash = parse_dash_manifest(&xml)?;
                    Ok(ResolvedStream {
                        format: delivered_format(
                            &dash.codecs,
                            playback.bit_depth,
                            playback.sample_rate.or(dash.sample_rate),
                            audio_quality,
                        ),
                        source: StreamSource::Segmented {
                            mime: "audio/mp4".to_string(),
                            init_url: dash.init_url,
                            segment_urls: dash.segment_urls,
                        },
                        requested: quality,
                    })
                }
                other => Err(ProviderError::Unavailable(format!(
                    "unsupported TIDAL manifest type {other}"
                ))),
            }
        })
    }

    fn radio<'a>(
        &'a self,
        context: &'a ProviderContext,
        seed: RadioSeed,
        raw_id: &'a str,
        limit: u32,
    ) -> BoxFuture<'a, ProviderResult<Radio>> {
        Box::pin(async move {
            let path = match seed {
                RadioSeed::Artist => format!("/artists/{raw_id}/radio"),
                RadioSeed::Track | RadioSeed::Album => format!("/tracks/{raw_id}/radio"),
            };
            let query = [
                ("limit", limit.min(100).to_string()),
                ("offset", "0".to_string()),
            ];
            let page: TPage<MaybeWrapped<TTrack>> =
                decode_json(&self.get(context, &path, &query).await?)?;
            Ok(Radio {
                tracks: page
                    .items
                    .into_iter()
                    .map(|track| self.normalize_track(track.into_inner()))
                    .take(limit as usize)
                    .collect(),
                origin: RadioOrigin::Service,
            })
        })
    }

    fn playlists<'a>(
        &'a self,
        context: &'a ProviderContext,
        offset: u32,
        limit: u32,
    ) -> BoxFuture<'a, ProviderResult<PlaylistPage>> {
        Box::pin(async move {
            let user = &context.session.user_id;
            let body = self
                .get(
                    context,
                    &format!("/users/{user}/playlistsAndFavoritePlaylists"),
                    &[
                        ("offset", offset.to_string()),
                        ("limit", limit.to_string()),
                        ("order", "DATE".to_string()),
                        ("orderDirection", "DESC".to_string()),
                    ],
                )
                .await?;
            let page: TPage<TPlaylistEntry> = decode_json(&body)?;
            Ok(PlaylistPage {
                playlists: page
                    .items
                    .into_iter()
                    .map(|entry| match entry {
                        TPlaylistEntry::Wrapped { relation, playlist } => {
                            self.normalize_playlist(playlist, relation.as_deref(), user)
                        }
                        TPlaylistEntry::Bare(playlist) => {
                            self.normalize_playlist(playlist, None, user)
                        }
                    })
                    .collect(),
                total: page.total_number_of_items,
            })
        })
    }

    fn playlist<'a>(
        &'a self,
        context: &'a ProviderContext,
        raw_id: &'a str,
    ) -> BoxFuture<'a, ProviderResult<ProviderPlaylist>> {
        Box::pin(async move {
            let body = self
                .get(context, &format!("/playlists/{raw_id}"), &[])
                .await?;
            let mut playlist =
                self.normalize_playlist(decode_json(&body)?, None, &context.session.user_id);
            let mut offset = 0;
            while offset < MAX_PLAYLIST_TRACKS {
                let body = self
                    .get(
                        context,
                        &format!("/playlists/{raw_id}/items"),
                        &[("offset", offset.to_string()), ("limit", "100".to_string())],
                    )
                    .await?;
                let page: TPage<TPlaylistItem> = decode_json(&body)?;
                let received = page.items.len() as u32;
                // Playlists can hold videos; Parson plays the tracks.
                playlist.tracks.extend(
                    page.items
                        .into_iter()
                        .filter(|entry| entry.kind.as_deref().is_none_or(|kind| kind == "track"))
                        .filter_map(|entry| serde_json::from_value::<TTrack>(entry.item?).ok())
                        .map(|track| self.normalize_track(track)),
                );
                offset += received;
                if received < 100
                    || page
                        .total_number_of_items
                        .is_some_and(|total| offset >= total)
                {
                    break;
                }
            }
            playlist.track_count = playlist.track_count.max(playlist.tracks.len() as u32);
            Ok(playlist)
        })
    }

    fn refresh<'a>(
        &'a self,
        credentials: &'a ClientCredentials,
        session: &'a ProviderSession,
    ) -> BoxFuture<'a, ProviderResult<Option<ProviderSession>>> {
        Box::pin(async move {
            let Some(refresh_token) = session.refresh_token.as_deref() else {
                return Ok(None);
            };
            let request =
                Self::client_form(ApiRequest::post(format!("{AUTH_BASE}/token")), credentials)
                    .form("refresh_token", refresh_token)
                    .form("grant_type", "refresh_token")
                    .form("scope", SCOPE);
            let reply = self.transport.send(request).await?;
            match reply.status {
                200..=299 => {
                    let token: TTokenResponse = decode_json(&reply.body)?;
                    Ok(Some(session_from_token(token, Some(session))))
                }
                400 | 401 => Err(ProviderError::SessionExpired),
                status => Err(status_error(status)),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;

    use super::{DevicePoll, TidalClient, parse_dash_manifest};
    use crate::providers::artwork::ArtworkMode;
    use crate::providers::transport::{FixtureRoute, FixtureTransport, Method};
    use crate::providers::{
        BrowseSection, ClientCredentials, Credit, ItemKind, ProviderContext, ProviderError,
        ProviderSession, RadioOrigin, RadioSeed, StreamQuality, StreamSource, StreamingProvider,
    };

    const FIXTURES: &str = include_str!("fixtures/tidal.json");
    const BROWSE_FIXTURES: &str = include_str!("fixtures/tidal_browse.json");
    const NOTES_FIXTURES: &str = include_str!("fixtures/tidal_notes.json");

    fn client() -> (TidalClient, Arc<FixtureTransport>) {
        let transport = Arc::new(
            FixtureTransport::from_json(&[BROWSE_FIXTURES, NOTES_FIXTURES, FIXTURES])
                .expect("fixtures"),
        );
        (
            TidalClient::new(transport.clone(), ArtworkMode::Live),
            transport,
        )
    }

    fn credentials() -> ClientCredentials {
        ClientCredentials {
            client_id: "listener-client-id".into(),
            client_secret: Some("listener-client-secret".into()),
        }
    }

    fn context() -> ProviderContext {
        ProviderContext {
            credentials: credentials(),
            session: ProviderSession {
                access_token: "recorded-access-token".into(),
                refresh_token: Some("recorded-refresh-token".into()),
                expires_at: None,
                user_id: "5550101".into(),
                account_name: "recorded.listener".into(),
                country_code: Some("GB".into()),
                plan: None,
            },
        }
    }

    #[actix_web::test]
    async fn device_code_sign_in_waits_for_approval_then_returns_a_session() {
        let (client, transport) = client();
        let pending = client
            .start_device_authorization(&credentials())
            .await
            .expect("device authorization");
        assert_eq!(pending.user_code, "PRSN7");
        assert_eq!(pending.verification_uri, "https://link.tidal.com");
        assert_eq!(
            pending.verification_uri_complete.as_deref(),
            Some("https://link.tidal.com/PRSN7")
        );

        assert_eq!(
            client
                .poll_device_authorization(&credentials(), &pending)
                .await
                .expect("first poll"),
            DevicePoll::Pending
        );
        let DevicePoll::Approved(session) = client
            .poll_device_authorization(&credentials(), &pending)
            .await
            .expect("second poll")
        else {
            panic!("the recorded second poll approves the device");
        };
        assert_eq!(session.user_id, "5550101");
        assert_eq!(session.account_name, "recorded.listener");
        assert_eq!(session.country_code.as_deref(), Some("GB"));
        assert!(session.expires_at.is_some());

        let token_request = transport
            .requests()
            .into_iter()
            .rfind(|request| request.url.ends_with("/token"))
            .expect("token request");
        assert_eq!(
            token_request.form_value("grant_type"),
            Some("urn:ietf:params:oauth:grant-type:device_code")
        );
        assert_eq!(
            token_request.form_value("client_secret"),
            Some("listener-client-secret")
        );
    }

    #[actix_web::test]
    async fn genres_and_editorial_lists_follow_the_v1_featured_paths() {
        let (client, transport) = client();
        let context = context();
        let genres = client.genres(&context).await.expect("genres");
        assert_eq!(
            genres
                .iter()
                .map(|genre| genre.id.as_str())
                .collect::<Vec<_>>(),
            ["Pop", "Jazz", "Classical", "Electronic"],
            "genres without albums are left out"
        );
        assert!(
            genres[0]
                .image_url
                .as_deref()
                .is_some_and(|url| url.starts_with("https://resources.tidal.com/images/"))
        );
        assert_eq!(
            transport
                .requests()
                .pop()
                .expect("request")
                .query_value("countryCode"),
            Some("GB")
        );

        let pop = client
            .genre_albums(&context, "Pop", BrowseSection::NewReleases, 0, 24)
            .await
            .expect("pop");
        assert_eq!(
            pop.albums
                .iter()
                .map(|album| album.title.as_str())
                .collect::<Vec<_>>(),
            ["Harbor Lights", "Paper Moons"]
        );
        // TIDAL albums carry no genre of their own; the hub labels them.
        assert!(pop.albums.iter().all(|album| album.genre.is_none()));
        assert!(
            transport
                .requests()
                .pop()
                .expect("request")
                .url
                .ends_with("/v1/genres/Pop/albums")
        );

        let top = client
            .browse(&context, BrowseSection::Popular, 0, 50)
            .await
            .expect("top");
        assert_eq!(top.albums[0].title, "Night Shift Standards");
        let recommended = client
            .browse(&context, BrowseSection::EditorsPicks, 0, 50)
            .await
            .expect("recommended");
        assert_eq!(recommended.albums[0].title, "Low Tide Hymns");
        assert!(matches!(
            client
                .browse(&context, BrowseSection::PressAwards, 0, 50)
                .await,
            Err(ProviderError::Unavailable(_))
        ));
        assert!(
            client
                .editorial_sections()
                .iter()
                .all(|section| section.id != BrowseSection::PressAwards)
        );
    }

    #[actix_web::test]
    async fn album_pages_carry_the_review_and_per_track_credits() {
        let (client, transport) = client();
        let album = client.album(&context(), "300100").await.expect("album");
        let review = album.review.expect("review");
        assert_eq!(review.source.as_deref(), Some("Parson sandbox"));
        assert!(
            review
                .text
                .starts_with("Mara Vell's Harbor Lights is a quiet")
        );
        assert!(review.text.contains("\n\nWritten over one winter"));
        assert!(review.text.contains("\"porch light\""), "{}", review.text);
        let first = &album.tracks[0];
        assert_eq!(first.id, "tidal:300101");
        assert_eq!(
            first.credits[0],
            Credit {
                role: "Producer".into(),
                names: vec!["Ines Calder".into()]
            }
        );
        assert!(album.tracks.iter().all(|track| track.credits.len() == 4));
        let credits_request = transport
            .requests()
            .into_iter()
            .find(|request| request.url.ends_with("/albums/300100/items/credits"))
            .expect("credits request");
        assert_eq!(
            credits_request.query_value("includeContributors"),
            Some("true")
        );

        // No review: TIDAL answers 404 and the page still loads.
        let plain = client.album(&context(), "300200").await.expect("album");
        assert!(plain.review.is_none());
        assert_eq!(plain.tracks.len(), 3);
    }

    #[actix_web::test]
    async fn artist_pages_carry_the_bio_and_similar_artists() {
        let (client, _) = client();
        let artist = client.artist(&context(), "400200").await.expect("artist");
        assert_eq!(
            artist.bio.expect("bio").text,
            "The Lantern Choir is a twelve-voice ensemble that sings in churches and boathouses."
        );
        let similar = artist
            .similar
            .iter()
            .map(|artist| artist.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(similar, ["tidal:400100", "tidal:400300"]);
    }

    #[actix_web::test]
    async fn radios_use_tidal_track_and_artist_radio() {
        let (client, transport) = client();
        let radio = client
            .radio(&context(), RadioSeed::Track, "300101", 5)
            .await
            .expect("track radio");
        assert_eq!(radio.origin, RadioOrigin::Service);
        assert_eq!(radio.tracks.len(), 5);
        assert!(radio.tracks.iter().all(|track| track.id != "tidal:300101"));
        let request = transport.requests().pop().expect("radio request");
        assert!(request.url.ends_with("/tracks/300101/radio"));
        assert_eq!(request.query_value("limit"), Some("5"));

        let artist = client
            .radio(&context(), RadioSeed::Artist, "400300", 50)
            .await
            .expect("artist radio");
        assert_eq!(artist.tracks[0].artist.name, "Okonkwo Quartet");
        assert!(
            transport
                .requests()
                .pop()
                .expect("request")
                .url
                .ends_with("/artists/400300/radio")
        );
    }

    #[actix_web::test]
    async fn expired_device_codes_stop_polling_without_a_request() {
        let (client, transport) = client();
        let mut pending = client
            .start_device_authorization(&credentials())
            .await
            .expect("device authorization");
        pending.expires_at = 0;
        assert_eq!(
            client
                .poll_device_authorization(&credentials(), &pending)
                .await
                .expect("poll"),
            DevicePoll::Expired
        );
        assert_eq!(transport.requests().len(), 1);
    }

    #[actix_web::test]
    async fn catalog_responses_normalize_to_provider_scoped_entities() {
        let (client, transport) = client();
        let context = context();
        let results = client.search(&context, "harbor", 10).await.expect("search");
        assert!(!results.albums.is_empty());
        assert!(
            results
                .albums
                .iter()
                .all(|album| album.id.starts_with("tidal:"))
        );
        let request = transport.requests().pop().expect("search request");
        assert_eq!(request.query_value("countryCode"), Some("GB"));
        assert_eq!(
            request.header_value("authorization"),
            Some("Bearer recorded-access-token")
        );

        let album = client.album(&context, "300100").await.expect("album");
        assert_eq!(album.title, "Harbor Lights");
        assert_eq!(album.artist.id, "tidal:400100");
        assert_eq!(album.tracks.len(), 4);
        assert_eq!(album.tracks[0].id, "tidal:300101");
        assert_eq!(
            album.format.as_ref().map(|format| format.label.as_str()),
            Some("24-bit FLAC")
        );
        assert!(
            album
                .cover_url
                .as_deref()
                .is_some_and(|url| url.starts_with("https://resources.tidal.com/images/"))
        );

        let artist = client.artist(&context, "400100").await.expect("artist");
        assert_eq!(artist.name, "Mara Vell");
        assert!(!artist.albums.is_empty());
        assert!(!artist.top_tracks.is_empty());

        let favorites = client
            .browse(&context, BrowseSection::FavoriteAlbums, 0, 50)
            .await
            .expect("favorites");
        assert_eq!(favorites.total, Some(2));
        assert_eq!(favorites.albums.len(), 2);
    }

    #[actix_web::test]
    async fn add_to_library_writes_a_tidal_favorite() {
        let (client, transport) = client();
        client
            .set_favorite(&context(), ItemKind::Album, "300200", true)
            .await
            .expect("favorite");
        let request = transport.requests().pop().expect("favorite request");
        assert!(request.url.ends_with("/users/5550101/favorites/albums"));
        assert_eq!(request.form_value("albumIds"), Some("300200"));
        client
            .set_favorite(&context(), ItemKind::Album, "300200", false)
            .await
            .expect("unfavorite");
        assert!(
            transport
                .requests()
                .pop()
                .expect("delete")
                .url
                .ends_with("/favorites/albums/300200")
        );
    }

    #[actix_web::test]
    async fn lossless_streams_resolve_to_the_bts_url_at_the_chosen_quality() {
        let (client, transport) = client();
        let stream = client
            .resolve_stream(&context(), "300101", StreamQuality::Lossless)
            .await
            .expect("stream");
        assert_eq!(
            stream.source,
            StreamSource::Progressive {
                url: "https://sp-recorded.audio.tidal.test/300101.flac".into(),
                mime: "audio/flac".into(),
            }
        );
        assert_eq!(stream.format.label, "16/44.1 FLAC");
        assert_eq!(
            transport
                .requests()
                .pop()
                .expect("playback")
                .query_value("audioquality"),
            Some("LOSSLESS")
        );
    }

    #[actix_web::test]
    async fn hi_res_streams_resolve_dash_segments() {
        let (client, _) = client();
        let stream = client
            .resolve_stream(&context(), "300101", StreamQuality::HiResMax)
            .await
            .expect("stream");
        let StreamSource::Segmented {
            init_url,
            segment_urls,
            mime,
        } = stream.source
        else {
            panic!("hi-res uses DASH");
        };
        assert_eq!(mime, "audio/mp4");
        assert_eq!(
            init_url.as_deref(),
            Some("https://sp-recorded.audio.tidal.test/hr/300101/0.mp4?token=a&sig=b")
        );
        assert_eq!(segment_urls.len(), 4);
        assert_eq!(
            segment_urls[3],
            "https://sp-recorded.audio.tidal.test/hr/300101/4.mp4?token=a&sig=b"
        );
        assert_eq!(stream.format.label, "24/96 FLAC");
    }

    #[actix_web::test]
    async fn previews_and_encrypted_streams_are_refused_clearly() {
        let (client, transport) = client();
        transport.prepend(FixtureRoute {
            method: Method::Get,
            url: "https://api.tidal.com/v1/tracks/300102/playbackinfopostpaywall".into(),
            query: Vec::new(),
            form: Vec::new(),
            replies: [crate::providers::transport::FixtureReply {
                status: 200,
                body: serde_json::json!({
                    "trackId": 300102,
                    "audioQuality": "LOSSLESS",
                    "assetPresentation": "FULL",
                    "manifestMimeType": "application/vnd.tidal.bts",
                    "manifest": STANDARD.encode(br#"{"mimeType":"audio/flac","codecs":"flac","encryptionType":"OLD_AES","urls":["https://x.test/a"]}"#),
                }),
            }]
            .into(),
        });
        assert!(matches!(
            client
                .resolve_stream(&context(), "300102", StreamQuality::Lossless)
                .await,
            Err(ProviderError::Unavailable(_))
        ));
    }

    #[actix_web::test]
    async fn refresh_keeps_identity_and_rotates_tokens() {
        let (client, _) = client();
        let refreshed = client
            .refresh(&credentials(), &context().session)
            .await
            .expect("refresh")
            .expect("session");
        assert_eq!(refreshed.access_token, "recorded-access-token-2");
        assert_eq!(
            refreshed.refresh_token.as_deref(),
            Some("recorded-refresh-token")
        );
        assert_eq!(refreshed.user_id, "5550101");
    }

    #[test]
    fn dash_manifests_reject_insecure_urls_and_empty_timelines() {
        let insecure = r#"<MPD><Period><AdaptationSet><Representation id="FLAC" codecs="flac"><SegmentTemplate initialization="http://x/0.mp4" media="http://x/$Number$.mp4" startNumber="1"><SegmentTimeline><S d="1"/></SegmentTimeline></SegmentTemplate></Representation></AdaptationSet></Period></MPD>"#;
        assert!(parse_dash_manifest(insecure).is_err());
        let empty = r#"<MPD><Representation id="FLAC" codecs="flac"><SegmentTemplate media="https://x/$Number$.mp4"></SegmentTemplate></Representation></MPD>"#;
        assert!(parse_dash_manifest(empty).is_err());
    }
}
