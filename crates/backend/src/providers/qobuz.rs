//! Qobuz: app-credential + user login, catalog, favorites and signed stream
//! URL resolution (`track/getFileUrl`).
//!
//! Qobuz issues app ids and secrets only to partners. The secret comes from
//! the listener or the server owner, or stays with a Parson key service that
//! signs stream requests (see `key_service`). Parson never extracts either
//! from Qobuz's apps.

use std::sync::Arc;

use futures::future::BoxFuture;
use serde::{Deserialize, Deserializer};

use super::artwork::ArtworkMode;
use super::md5::hex_digest;
use super::transport::{ApiRequest, Transport};
use super::{
    AlbumRef, ArtistRef, ArtistSummary, AudioFormat, BrowsePage, BrowseSection, ClientCredentials,
    Credit, EditorialSection, EditorialText, ItemKind, MAX_PLAYLIST_TRACKS, PlaylistPage,
    ProviderAlbum, ProviderArtist, ProviderContext, ProviderError, ProviderGenre, ProviderId,
    ProviderPlaylist, ProviderResult, ProviderSession, ProviderTrack, QualityOption, Radio,
    RadioOrigin, RadioSeed, ResolvedStream, SearchResults, StreamQuality, StreamSource,
    StreamingProvider, decode_json, entity_id, interleave_tracks, plain_text, push_credit,
    status_error, unix_now,
};

/// Similar artists mixed into a Qobuz radio, besides the seed artist.
const RADIO_SIMILAR_ARTISTS: usize = 5;

pub const API_BASE: &str = "https://www.qobuz.com/api.json/0.2";

static QUALITY_OPTIONS: [QualityOption; 4] = [
    QualityOption {
        id: StreamQuality::HiResMax,
        label: "Hi-Res 24-Bit / up to 192 kHz",
        detail: "FLAC, the best each album is available in",
    },
    QualityOption {
        id: StreamQuality::HiRes96,
        label: "Hi-Res 24-Bit / up to 96 kHz",
        detail: "FLAC, capped at 96 kHz",
    },
    QualityOption {
        id: StreamQuality::Lossless,
        label: "CD 16-Bit / 44.1 kHz",
        detail: "FLAC, CD quality",
    },
    QualityOption {
        id: StreamQuality::High,
        label: "MP3 320 kbps",
        detail: "Lossy, for slow connections",
    },
];

static EDITORIAL_SECTIONS: [EditorialSection; 4] = [
    EditorialSection {
        id: BrowseSection::NewReleases,
        label: "New releases",
        by_genre: true,
    },
    EditorialSection {
        id: BrowseSection::EditorsPicks,
        label: "Editor's picks",
        by_genre: true,
    },
    EditorialSection {
        id: BrowseSection::Popular,
        label: "Most streamed",
        by_genre: true,
    },
    EditorialSection {
        id: BrowseSection::PressAwards,
        label: "Press awards",
        by_genre: true,
    },
];

pub struct QobuzClient {
    transport: Arc<dyn Transport>,
    artwork: ArtworkMode,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct QGenre {
    #[serde(deserialize_with = "de_id")]
    id: String,
    name: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct QGenreList {
    genres: QPage<QGenre>,
}

fn de_id<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::String(value) => value,
        serde_json::Value::Number(value) => value.to_string(),
        _ => String::new(),
    })
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct QImage {
    large: Option<String>,
    small: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct QNamed {
    name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct QArtist {
    #[serde(deserialize_with = "de_id")]
    id: String,
    name: String,
    image: Option<QImage>,
    picture: Option<String>,
    albums: Option<QPage<QAlbum>>,
    tracks: Option<QPage<QTrack>>,
    biography: Option<QBiography>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct QBiography {
    summary: Option<String>,
    content: Option<String>,
    source: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct QAward {
    name: Option<String>,
    publication_name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct QSimilar {
    artists: QPage<QArtist>,
}

/// Qobuz credits: `Name, Role, Role - Name, Role`.
fn parse_performers(performers: &str) -> Vec<Credit> {
    let mut credits = Vec::new();
    for entry in performers.split(" - ") {
        let mut parts = entry.split(',').map(str::trim);
        let Some(name) = parts.next().filter(|name| !name.is_empty()) else {
            continue;
        };
        for role in parts {
            push_credit(&mut credits, role, name);
        }
    }
    credits
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct QAlbum {
    #[serde(deserialize_with = "de_id")]
    id: String,
    title: String,
    version: Option<String>,
    artist: Option<QArtist>,
    artists: Vec<QArtist>,
    image: Option<QImage>,
    release_date_original: Option<String>,
    tracks_count: u32,
    duration: f64,
    maximum_bit_depth: Option<u8>,
    maximum_sampling_rate: Option<f64>,
    streamable: Option<bool>,
    parental_warning: bool,
    label: Option<QNamed>,
    genre: Option<QNamed>,
    copyright: Option<String>,
    url: Option<String>,
    tracks: Option<QPage<QTrack>>,
    description: Option<String>,
    awards: Vec<QAward>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct QTrack {
    #[serde(deserialize_with = "de_id")]
    id: String,
    title: String,
    version: Option<String>,
    track_number: u32,
    media_number: u32,
    duration: f64,
    performer: Option<QArtist>,
    isrc: Option<String>,
    maximum_bit_depth: Option<u8>,
    maximum_sampling_rate: Option<f64>,
    streamable: Option<bool>,
    parental_warning: bool,
    album: Option<Box<QAlbum>>,
    performers: Option<String>,
}

#[derive(Debug, Deserialize)]
struct QPage<T> {
    #[serde(default = "Vec::new")]
    items: Vec<T>,
    #[serde(default)]
    total: Option<u32>,
}

impl<T> Default for QPage<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            total: None,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct QSearch {
    albums: QPage<QAlbum>,
    tracks: QPage<QTrack>,
    artists: QPage<QArtist>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct QOwner {
    #[serde(deserialize_with = "de_id")]
    id: String,
    name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct QPlaylist {
    #[serde(deserialize_with = "de_id")]
    id: String,
    name: String,
    description: Option<String>,
    owner: Option<QOwner>,
    tracks_count: u32,
    duration: f64,
    updated_at: Option<i64>,
    images300: Vec<String>,
    image_rectangle: Vec<String>,
    tracks: Option<QPage<QTrack>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct QPlaylists {
    playlists: QPage<QPlaylist>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct QFavorites {
    albums: QPage<QAlbum>,
    tracks: QPage<QTrack>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct QCredential {
    label: Option<String>,
    parameters: Option<QCredentialParameters>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct QCredentialParameters {
    short_label: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct QUser {
    #[serde(deserialize_with = "de_id")]
    id: String,
    login: Option<String>,
    display_name: Option<String>,
    country_code: Option<String>,
    credential: Option<QCredential>,
}

#[derive(Debug, Deserialize)]
struct QLogin {
    user: QUser,
    user_auth_token: String,
}

#[derive(Debug, Deserialize)]
struct QFileUrl {
    url: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    sampling_rate: Option<f64>,
    #[serde(default)]
    bit_depth: Option<u8>,
    #[serde(default)]
    sample: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct QError {
    message: Option<String>,
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn hz(khz: Option<f64>) -> Option<u32> {
    khz.filter(|value| *value > 0.0)
        .map(|value| (value * 1000.0).round() as u32)
}

fn catalog_format(bit_depth: Option<u8>, sampling_rate: Option<f64>) -> AudioFormat {
    AudioFormat::lossless(
        "flac",
        bit_depth.or(Some(16)),
        hz(sampling_rate).or(Some(44_100)),
    )
}

/// The `type` of `album/getFeatured` for each editorial list.
fn featured_type(section: BrowseSection) -> &'static str {
    match section {
        BrowseSection::Popular => "most-streamed",
        BrowseSection::EditorsPicks => "editor-picks",
        BrowseSection::PressAwards => "press-awards",
        _ => "new-releases-full",
    }
}

/// Qobuz format ids: 5 = MP3 320, 6 = FLAC 16/44.1, 7 = FLAC 24/<=96,
/// 27 = FLAC 24/<=192.
pub fn format_id(quality: StreamQuality) -> u8 {
    match quality {
        StreamQuality::HiResMax => 27,
        StreamQuality::HiRes96 => 7,
        StreamQuality::Lossless => 6,
        StreamQuality::High | StreamQuality::Low => 5,
    }
}

/// `request_sig` for `track/getFileUrl`: the MD5 of the method name, the
/// sorted parameters, the timestamp and the app secret.
pub fn file_url_signature(format_id: u8, track_id: &str, timestamp: i64, secret: &str) -> String {
    hex_digest(&format!(
        "trackgetFileUrlformat_id{format_id}intentstreamtrack_id{track_id}{timestamp}{secret}"
    ))
}

impl QobuzClient {
    pub fn new(transport: Arc<dyn Transport>, artwork: ArtworkMode) -> Self {
        Self { transport, artwork }
    }

    fn artist_ref(artist: &QArtist) -> ArtistRef {
        ArtistRef {
            id: entity_id(ProviderId::Qobuz, &artist.id),
            name: artist.name.clone(),
        }
    }

    fn image(&self, image: Option<&QImage>) -> Option<String> {
        let image = image?;
        self.artwork
            .qobuz(image.large.as_deref().or(image.small.as_deref()))
    }

    fn normalize_album(&self, raw: QAlbum) -> ProviderAlbum {
        let artists = raw
            .artists
            .iter()
            .filter(|artist| !artist.id.is_empty())
            .map(Self::artist_ref)
            .collect::<Vec<_>>();
        let artist = raw
            .artist
            .as_ref()
            .map(Self::artist_ref)
            .or_else(|| artists.first().cloned())
            .unwrap_or_default();
        let cover_url = self.image(raw.image.as_ref());
        let album_ref = AlbumRef {
            id: entity_id(ProviderId::Qobuz, &raw.id),
            title: raw.title.clone(),
            cover_url: cover_url.clone(),
        };
        let tracks = raw
            .tracks
            .map(|page| {
                page.items
                    .into_iter()
                    .map(|track| self.normalize_track(track, Some(&album_ref), Some(&artist)))
                    .collect()
            })
            .unwrap_or_default();
        ProviderAlbum {
            id: album_ref.id.clone(),
            provider: ProviderId::Qobuz,
            format: Some(catalog_format(
                raw.maximum_bit_depth,
                raw.maximum_sampling_rate,
            )),
            title: raw.title,
            version: non_empty(raw.version),
            artist,
            artists,
            cover_url,
            release_date: non_empty(raw.release_date_original),
            track_count: raw.tracks_count,
            duration: raw.duration,
            explicit: raw.parental_warning,
            label: raw.label.and_then(|label| non_empty(label.name)),
            genre: raw.genre.and_then(|genre| non_empty(genre.name)),
            copyright: non_empty(raw.copyright),
            web_url: raw
                .url
                .filter(|url| url.starts_with("https://"))
                .or_else(|| Some(format!("https://open.qobuz.com/album/{}", raw.id))),
            tracks,
            in_library: false,
            review: raw
                .description
                .as_deref()
                .and_then(plain_text)
                .map(|text| EditorialText {
                    text,
                    source: Some("Qobuz".to_string()),
                }),
            awards: raw
                .awards
                .into_iter()
                .filter_map(|award| {
                    let name = non_empty(award.name)?;
                    Some(match non_empty(award.publication_name) {
                        Some(publication) if !name.contains(&publication) => {
                            format!("{name} ({publication})")
                        }
                        _ => name,
                    })
                })
                .collect(),
        }
    }

    fn normalize_track(
        &self,
        raw: QTrack,
        album: Option<&AlbumRef>,
        album_artist: Option<&ArtistRef>,
    ) -> ProviderTrack {
        let artist = raw
            .performer
            .as_ref()
            .map(Self::artist_ref)
            .or_else(|| album_artist.cloned())
            .or_else(|| {
                raw.album
                    .as_ref()
                    .and_then(|album| album.artist.as_ref())
                    .map(Self::artist_ref)
            })
            .unwrap_or_default();
        let album = album.cloned().or_else(|| {
            raw.album.as_ref().map(|album| AlbumRef {
                id: entity_id(ProviderId::Qobuz, &album.id),
                title: album.title.clone(),
                cover_url: self.image(album.image.as_ref()),
            })
        });
        ProviderTrack {
            id: entity_id(ProviderId::Qobuz, &raw.id),
            provider: ProviderId::Qobuz,
            format: Some(catalog_format(
                raw.maximum_bit_depth,
                raw.maximum_sampling_rate,
            )),
            title: raw.title,
            version: non_empty(raw.version),
            artists: vec![artist.clone()],
            artist,
            album,
            track_number: raw.track_number,
            disc_number: raw.media_number.max(1),
            duration: raw.duration,
            explicit: raw.parental_warning,
            isrc: non_empty(raw.isrc),
            streamable: raw.streamable.unwrap_or(true),
            in_library: false,
            credits: raw
                .performers
                .as_deref()
                .map(parse_performers)
                .unwrap_or_default(),
        }
    }

    fn artist_summary(&self, raw: &QArtist) -> ArtistSummary {
        ArtistSummary {
            id: entity_id(ProviderId::Qobuz, &raw.id),
            provider: ProviderId::Qobuz,
            name: raw.name.clone(),
            picture_url: self
                .image(raw.image.as_ref())
                .or_else(|| self.artwork.qobuz(raw.picture.as_deref())),
        }
    }

    fn request(&self, credentials: &ClientCredentials, path: &str) -> ApiRequest {
        ApiRequest::get(format!("{API_BASE}/{path}"))
            .header("x-app-id", credentials.client_id.clone())
    }

    async fn get(
        &self,
        context: &ProviderContext,
        path: &str,
        query: &[(&str, String)],
    ) -> ProviderResult<Vec<u8>> {
        let mut request = self
            .request(&context.credentials, path)
            .header("x-user-auth-token", context.session.access_token.clone());
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

    /// Asks a Parson key service to sign one `track/getFileUrl` request.
    /// It receives the app id, track id, format and time, never the
    /// listener's session token.
    async fn service_signature(
        &self,
        service: &str,
        app_id: &str,
        format: u8,
        raw_id: &str,
        timestamp: i64,
    ) -> ProviderResult<String> {
        let request = ApiRequest::post(format!("{service}/v1/qobuz/sign"))
            .form("app_id", app_id)
            .form("track_id", raw_id)
            .form("format_id", format.to_string())
            .form("intent", "stream")
            .form("request_ts", timestamp.to_string());
        let reply = self.transport.send(request).await.map_err(|error| {
            ProviderError::Unavailable(format!(
                "Mujin Music's key service could not be reached to sign this Qobuz stream ({error})."
            ))
        })?;
        match reply.status {
            200..=299 => {}
            400 | 401 | 403 | 404 => {
                return Err(ProviderError::InvalidCredentials(
                    "Mujin Music's key service declined to sign this Qobuz stream.".into(),
                ));
            }
            429 => return Err(ProviderError::RateLimited),
            status => {
                return Err(ProviderError::Unavailable(format!(
                    "Mujin Music's key service is unavailable (HTTP {status}). Try again shortly."
                )));
            }
        }
        #[derive(Deserialize)]
        struct Signed {
            request_sig: String,
        }
        let signed: Signed = decode_json(&reply.body)?;
        let well_formed = signed.request_sig.len() == 32
            && signed
                .request_sig
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit());
        if !well_formed {
            return Err(ProviderError::Decode(
                "the key service sent a malformed signature".into(),
            ));
        }
        Ok(signed.request_sig.to_ascii_lowercase())
    }

    /// Signs in with the listener's Qobuz email or username and password.
    /// The password is sent once as Qobuz's protocol requires and is never
    /// stored; only the resulting user token is kept (sealed).
    pub async fn login(
        &self,
        credentials: &ClientCredentials,
        username: &str,
        password: &str,
    ) -> ProviderResult<ProviderSession> {
        let identity_field = if username.contains('@') {
            "email"
        } else {
            "username"
        };
        let request = self
            .request(credentials, "user/login")
            .query(identity_field, username)
            .query("password", hex_digest(password))
            .query("app_id", credentials.client_id.clone());
        let reply = self.transport.send(request).await?;
        match reply.status {
            200..=299 => {}
            400 => {
                let error: QError = serde_json::from_slice(&reply.body).unwrap_or_default();
                return Err(ProviderError::InvalidCredentials(
                    error
                        .message
                        .filter(|message| message.to_ascii_lowercase().contains("app"))
                        .map(|_| "Qobuz did not accept this app ID.".to_string())
                        .unwrap_or_else(|| "Qobuz rejected the sign-in request.".to_string()),
                ));
            }
            401 => {
                return Err(ProviderError::InvalidCredentials(
                    "Qobuz did not accept this email or username and password.".into(),
                ));
            }
            status => return Err(status_error(status)),
        }
        let login: QLogin = decode_json(&reply.body)?;
        let credential = login.user.credential.unwrap_or_default();
        Ok(ProviderSession {
            access_token: login.user_auth_token,
            refresh_token: None,
            expires_at: None,
            account_name: non_empty(login.user.display_name)
                .or_else(|| non_empty(login.user.login))
                .unwrap_or_else(|| username.to_string()),
            user_id: login.user.id,
            country_code: non_empty(login.user.country_code),
            plan: credential
                .parameters
                .and_then(|parameters| non_empty(parameters.short_label))
                .or_else(|| non_empty(credential.label)),
        })
    }

    fn normalize_playlist(&self, raw: QPlaylist, user_id: &str) -> ProviderPlaylist {
        let owner = raw.owner.unwrap_or_default();
        let cover = raw
            .image_rectangle
            .first()
            .or_else(|| raw.images300.first())
            .map(String::as_str);
        let tracks = raw
            .tracks
            .map(|page| {
                page.items
                    .into_iter()
                    .map(|track| self.normalize_track(track, None, None))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        ProviderPlaylist {
            id: entity_id(ProviderId::Qobuz, &raw.id),
            provider: ProviderId::Qobuz,
            title: raw.name,
            description: non_empty(raw.description),
            own: !owner.id.is_empty() && owner.id == user_id,
            owner: non_empty(owner.name),
            track_count: raw.tracks_count.max(tracks.len() as u32),
            duration: raw.duration,
            cover_url: self.artwork.qobuz(cover),
            updated_at: raw.updated_at.filter(|at| *at > 0),
            web_url: Some(format!("https://open.qobuz.com/playlist/{}", raw.id)),
            tracks,
        }
    }

    /// `album/getFeatured`: Qobuz's editorial album lists, optionally
    /// narrowed to one genre.
    async fn featured(
        &self,
        context: &ProviderContext,
        section: BrowseSection,
        genre_id: Option<&str>,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<BrowsePage> {
        let mut query = vec![
            ("type", featured_type(section).to_string()),
            ("offset", offset.to_string()),
            ("limit", limit.to_string()),
        ];
        if let Some(genre_id) = genre_id {
            query.push(("genre_id", genre_id.to_string()));
        }
        let body = self.get(context, "album/getFeatured", &query).await?;
        let page: QFavorites = decode_json(&body)?;
        Ok(BrowsePage {
            total: page.albums.total,
            albums: page
                .albums
                .items
                .into_iter()
                .map(|album| self.normalize_album(album))
                .collect(),
            tracks: Vec::new(),
        })
    }

    async fn favorites(
        &self,
        context: &ProviderContext,
        kind: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<QFavorites> {
        let body = self
            .get(
                context,
                "favorite/getUserFavorites",
                &[
                    ("type", kind.to_string()),
                    ("offset", offset.to_string()),
                    ("limit", limit.to_string()),
                ],
            )
            .await?;
        decode_json(&body)
    }
}

impl QobuzClient {
    /// `artist/get` with albums and top tracks, plus (when asked) the
    /// biography's similar artists from `artist/getSimilarArtists`.
    async fn artist_page(
        &self,
        context: &ProviderContext,
        raw_id: &str,
        with_similar: bool,
    ) -> ProviderResult<ProviderArtist> {
        let artist_query = [
            ("artist_id", raw_id.to_string()),
            ("extra", "albums,tracks".to_string()),
            ("limit", "50".to_string()),
        ];
        let similar_query = [
            ("artist_id", raw_id.to_string()),
            ("limit", "12".to_string()),
            ("offset", "0".to_string()),
        ];
        let (body, similar) =
            futures::join!(self.get(context, "artist/get", &artist_query), async {
                if with_similar {
                    self.get(context, "artist/getSimilarArtists", &similar_query)
                        .await
                        .ok()
                } else {
                    None
                }
            });
        let artist: QArtist = decode_json(&body?)?;
        let summary = self.artist_summary(&artist);
        let artist_ref = Self::artist_ref(&artist);
        let similar: QSimilar = similar
            .and_then(|body| serde_json::from_slice(&body).ok())
            .unwrap_or_default();
        let bio = artist.biography.and_then(|biography| {
            let text = biography
                .content
                .as_deref()
                .and_then(plain_text)
                .or_else(|| biography.summary.as_deref().and_then(plain_text))?;
            Some(EditorialText {
                text,
                source: non_empty(biography.source).or_else(|| Some("Qobuz".to_string())),
            })
        });
        Ok(ProviderArtist {
            id: summary.id,
            provider: ProviderId::Qobuz,
            name: summary.name,
            picture_url: summary.picture_url,
            web_url: Some(format!("https://open.qobuz.com/artist/{}", artist.id)),
            albums: artist
                .albums
                .unwrap_or_default()
                .items
                .into_iter()
                .map(|album| self.normalize_album(album))
                .collect(),
            top_tracks: artist
                .tracks
                .unwrap_or_default()
                .items
                .into_iter()
                .take(10)
                .map(|track| self.normalize_track(track, None, Some(&artist_ref)))
                .collect(),
            bio,
            similar: similar
                .artists
                .items
                .iter()
                .filter(|artist| !artist.id.is_empty() && artist.id != raw_id)
                .map(|artist| self.artist_summary(artist))
                .collect(),
        })
    }
}

impl StreamingProvider for QobuzClient {
    fn id(&self) -> ProviderId {
        ProviderId::Qobuz
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
                    "catalog/search",
                    &[("query", query.to_string()), ("limit", limit.to_string())],
                )
                .await?;
            let search: QSearch = decode_json(&body)?;
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
                    .map(|track| self.normalize_track(track, None, None))
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
            let body = self
                .get(
                    context,
                    "album/get",
                    &[
                        ("album_id", raw_id.to_string()),
                        ("limit", "100".to_string()),
                    ],
                )
                .await?;
            Ok(self.normalize_album(decode_json(&body)?))
        })
    }

    fn artist<'a>(
        &'a self,
        context: &'a ProviderContext,
        raw_id: &'a str,
    ) -> BoxFuture<'a, ProviderResult<ProviderArtist>> {
        Box::pin(self.artist_page(context, raw_id, true))
    }

    fn track<'a>(
        &'a self,
        context: &'a ProviderContext,
        raw_id: &'a str,
    ) -> BoxFuture<'a, ProviderResult<ProviderTrack>> {
        Box::pin(async move {
            let body = self
                .get(context, "track/get", &[("track_id", raw_id.to_string())])
                .await?;
            Ok(self.normalize_track(decode_json(&body)?, None, None))
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
            match section {
                BrowseSection::NewReleases
                | BrowseSection::Popular
                | BrowseSection::EditorsPicks
                | BrowseSection::PressAwards => {
                    self.featured(context, section, None, offset, limit).await
                }
                BrowseSection::FavoriteAlbums => {
                    let page = self.favorites(context, "albums", offset, limit).await?;
                    Ok(BrowsePage {
                        total: page.albums.total,
                        albums: page
                            .albums
                            .items
                            .into_iter()
                            .map(|album| self.normalize_album(album))
                            .collect(),
                        tracks: Vec::new(),
                    })
                }
                BrowseSection::FavoriteTracks => {
                    let page = self.favorites(context, "tracks", offset, limit).await?;
                    Ok(BrowsePage {
                        total: page.tracks.total,
                        albums: Vec::new(),
                        tracks: page
                            .tracks
                            .items
                            .into_iter()
                            .map(|track| self.normalize_track(track, None, None))
                            .collect(),
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
            let body = self
                .get(context, "genre/list", &[("limit", "100".to_string())])
                .await?;
            let list: QGenreList = decode_json(&body)?;
            Ok(list
                .genres
                .items
                .into_iter()
                .filter(|genre| super::valid_raw_id(&genre.id) && !genre.name.trim().is_empty())
                .map(|genre| ProviderGenre {
                    id: genre.id,
                    provider: ProviderId::Qobuz,
                    name: genre.name.trim().to_string(),
                    image_url: None,
                })
                .collect())
        })
    }

    fn genre_albums<'a>(
        &'a self,
        context: &'a ProviderContext,
        genre_id: &'a str,
        section: BrowseSection,
        offset: u32,
        limit: u32,
    ) -> BoxFuture<'a, ProviderResult<BrowsePage>> {
        Box::pin(async move {
            let section = if section.is_editorial() {
                section
            } else {
                BrowseSection::NewReleases
            };
            self.featured(context, section, Some(genre_id), offset, limit)
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
            let field = match kind {
                ItemKind::Album => "album_ids",
                ItemKind::Track => "track_ids",
            };
            let path = if favorite {
                "favorite/create"
            } else {
                "favorite/delete"
            };
            self.get(context, path, &[(field, raw_id.to_string())])
                .await
                .map(|_| ())
        })
    }

    fn resolve_stream<'a>(
        &'a self,
        context: &'a ProviderContext,
        raw_id: &'a str,
        quality: StreamQuality,
    ) -> BoxFuture<'a, ProviderResult<ResolvedStream>> {
        Box::pin(async move {
            let format = format_id(quality);
            let timestamp = unix_now();
            let secret = context
                .credentials
                .client_secret
                .as_deref()
                .filter(|secret| !secret.is_empty());
            let signature = match (secret, context.credentials.signing_service.as_deref()) {
                (Some(secret), _) => file_url_signature(format, raw_id, timestamp, secret),
                (None, Some(service)) => {
                    self.service_signature(
                        service,
                        &context.credentials.client_id,
                        format,
                        raw_id,
                        timestamp,
                    )
                    .await?
                }
                (None, None) => {
                    return Err(ProviderError::InvalidCredentials(
                        "Add your Qobuz app secret to play music.".into(),
                    ));
                }
            };
            let body = self
                .get(
                    context,
                    "track/getFileUrl",
                    &[
                        ("request_ts", timestamp.to_string()),
                        ("request_sig", signature),
                        ("track_id", raw_id.to_string()),
                        ("format_id", format.to_string()),
                        ("intent", "stream".to_string()),
                    ],
                )
                .await
                .map_err(|error| match error {
                    ProviderError::Upstream(400) => ProviderError::InvalidCredentials(
                        "Qobuz rejected the request signature. Check the app secret.".into(),
                    ),
                    other => other,
                })?;
            let file: QFileUrl = decode_json(&body)?;
            if file.sample {
                return Err(ProviderError::Unavailable(
                    "This Qobuz account can only play 30-second samples. Check the subscription."
                        .into(),
                ));
            }
            let url = file
                .url
                .filter(|url| url.starts_with("https://"))
                .ok_or_else(|| ProviderError::Decode("no HTTPS stream URL".into()))?;
            let mime = file.mime_type.unwrap_or_else(|| "audio/flac".to_string());
            let delivered = if mime.contains("flac") {
                AudioFormat::lossless("flac", file.bit_depth, hz(file.sampling_rate))
            } else {
                AudioFormat::lossy("mp3", 320)
            };
            Ok(ResolvedStream {
                source: StreamSource::Progressive { url, mime },
                format: delivered,
                requested: quality,
                stream_seconds: None,
            })
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
            // Qobuz publishes no radio endpoint, so Parson mixes the
            // artist's top tracks with those of Qobuz's similar artists.
            let seed_track = match seed {
                RadioSeed::Artist => None,
                RadioSeed::Track | RadioSeed::Album => Some(self.track(context, raw_id).await?),
            };
            let artist_raw = match &seed_track {
                Some(track) => super::parse_entity_id(&track.artist.id)
                    .map(|(_, raw)| raw)
                    .ok_or(ProviderError::NotFound)?,
                None => raw_id.to_string(),
            };
            let artist = self.artist_page(context, &artist_raw, true).await?;
            let similar_ids = artist
                .similar
                .iter()
                .filter_map(|similar| super::parse_entity_id(&similar.id).map(|(_, raw)| raw))
                .take(RADIO_SIMILAR_ARTISTS)
                .collect::<Vec<_>>();
            let similar_pages = futures::future::join_all(
                similar_ids
                    .iter()
                    .map(|id| self.artist_page(context, id, false)),
            )
            .await;
            let mut lists = Vec::with_capacity(similar_pages.len() + 2);
            if let Some(track) = seed_track {
                lists.push(vec![track]);
            }
            lists.push(artist.top_tracks);
            lists.extend(
                similar_pages
                    .into_iter()
                    .filter_map(Result::ok)
                    .map(|page| page.top_tracks),
            );
            let tracks = interleave_tracks(lists, limit as usize)
                .into_iter()
                .filter(|track| track.streamable)
                .collect();
            Ok(Radio {
                tracks,
                origin: RadioOrigin::SimilarArtists,
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
            let body = self
                .get(
                    context,
                    "playlist/getUserPlaylists",
                    &[("offset", offset.to_string()), ("limit", limit.to_string())],
                )
                .await?;
            let page: QPlaylists = decode_json(&body)?;
            Ok(PlaylistPage {
                total: page.playlists.total,
                playlists: page
                    .playlists
                    .items
                    .into_iter()
                    .map(|playlist| self.normalize_playlist(playlist, &context.session.user_id))
                    .collect(),
            })
        })
    }

    fn playlist<'a>(
        &'a self,
        context: &'a ProviderContext,
        raw_id: &'a str,
    ) -> BoxFuture<'a, ProviderResult<ProviderPlaylist>> {
        Box::pin(async move {
            let mut playlist: Option<ProviderPlaylist> = None;
            let mut offset = 0;
            while offset < MAX_PLAYLIST_TRACKS {
                let body = self
                    .get(
                        context,
                        "playlist/get",
                        &[
                            ("playlist_id", raw_id.to_string()),
                            ("extra", "tracks".to_string()),
                            ("offset", offset.to_string()),
                            ("limit", "500".to_string()),
                        ],
                    )
                    .await?;
                let raw: QPlaylist = decode_json(&body)?;
                let total = raw.tracks.as_ref().and_then(|page| page.total);
                let page = self.normalize_playlist(raw, &context.session.user_id);
                let received = page.tracks.len() as u32;
                match playlist.as_mut() {
                    Some(playlist) => playlist.tracks.extend(page.tracks),
                    None => playlist = Some(page),
                }
                offset += received;
                if received < 500 || total.is_some_and(|total| offset >= total) {
                    break;
                }
            }
            playlist.ok_or(ProviderError::NotFound)
        })
    }

    fn refresh<'a>(
        &'a self,
        _credentials: &'a ClientCredentials,
        _session: &'a ProviderSession,
    ) -> BoxFuture<'a, ProviderResult<Option<ProviderSession>>> {
        // Qobuz user tokens have no refresh flow; an expired token means the
        // listener signs in again.
        Box::pin(async { Ok(None) })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::parse_performers;
    use super::{QobuzClient, file_url_signature};
    use crate::providers::artwork::ArtworkMode;
    use crate::providers::md5::hex_digest;
    use crate::providers::transport::FixtureTransport;
    use crate::providers::{
        BrowseSection, ClientCredentials, Credit, ItemKind, ProviderContext, ProviderError,
        ProviderSession, RadioOrigin, RadioSeed, StreamQuality, StreamSource, StreamingProvider,
    };
    use crate::test_support::generated_password;

    const FIXTURES: &str = include_str!("fixtures/qobuz.json");
    const BROWSE_FIXTURES: &str = include_str!("fixtures/qobuz_browse.json");
    const NOTES_FIXTURES: &str = include_str!("fixtures/qobuz_notes.json");

    fn client() -> (QobuzClient, Arc<FixtureTransport>) {
        let transport = Arc::new(
            FixtureTransport::from_json(&[BROWSE_FIXTURES, NOTES_FIXTURES, FIXTURES])
                .expect("fixtures"),
        );
        (
            QobuzClient::new(transport.clone(), ArtworkMode::Live),
            transport,
        )
    }

    fn credentials() -> ClientCredentials {
        ClientCredentials {
            client_id: "100000001".into(),
            client_secret: Some("listener-app-secret".into()),
            ..Default::default()
        }
    }

    fn context() -> ProviderContext {
        ProviderContext {
            credentials: credentials(),
            session: ProviderSession {
                access_token: "recorded-user-auth-token".into(),
                refresh_token: None,
                expires_at: None,
                user_id: "2468013".into(),
                account_name: "Recorded Listener".into(),
                country_code: Some("FR".into()),
                plan: Some("Studio".into()),
            },
        }
    }

    #[test]
    fn performers_strings_become_credits_by_role() {
        assert_eq!(
            parse_performers(
                "Ilse Marten, MainArtist, Composer - Ines Calder, Producer - , Nobody - Theo Barras, Mixing Engineer"
            ),
            vec![
                Credit {
                    role: "MainArtist".into(),
                    names: vec!["Ilse Marten".into()]
                },
                Credit {
                    role: "Composer".into(),
                    names: vec!["Ilse Marten".into()]
                },
                Credit {
                    role: "Producer".into(),
                    names: vec!["Ines Calder".into()]
                },
                Credit {
                    role: "Mixing Engineer".into(),
                    names: vec!["Theo Barras".into()]
                },
            ]
        );
    }

    #[actix_web::test]
    async fn album_pages_carry_the_description_awards_and_credits() {
        let (client, _) = client();
        let album = client
            .album(&context(), "q0glassorch01")
            .await
            .expect("album");
        let review = album.review.expect("description");
        assert_eq!(review.source.as_deref(), Some("Qobuz"));
        assert!(
            review
                .text
                .starts_with("Ilse Marten\u{2019}s Glass Orchard is a three-movement suite"),
            "{}",
            review.text
        );
        assert_eq!(review.text.matches("\n\n").count(), 1);
        assert_eq!(album.awards, ["Qobuzissime"]);
        let roles = album.tracks[0]
            .credits
            .iter()
            .map(|credit| credit.role.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            roles,
            [
                "MainArtist",
                "Composer",
                "Producer",
                "Mixing Engineer",
                "Mastering Engineer"
            ]
        );
    }

    #[actix_web::test]
    async fn artist_pages_carry_the_biography_and_similar_artists() {
        let (client, _) = client();
        let artist = client.artist(&context(), "51003").await.expect("artist");
        let bio = artist.bio.expect("biography");
        assert_eq!(bio.text, "Sora Ueda is a fictional pianist from Kanazawa.");
        assert_eq!(bio.source.as_deref(), Some("Mujin Music sandbox"));
        assert_eq!(artist.similar.len(), 3);
        assert!(
            artist
                .similar
                .iter()
                .all(|similar| similar.id != "qobuz:51003")
        );
    }

    #[actix_web::test]
    async fn radios_mix_the_seed_with_similar_artists_top_tracks() {
        let (client, _) = client();
        let radio = client
            .radio(&context(), RadioSeed::Track, "7700202", 50)
            .await
            .expect("radio");
        assert_eq!(radio.origin, RadioOrigin::SimilarArtists);
        let ids = radio
            .tracks
            .iter()
            .map(|track| track.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids[0], "qobuz:7700202", "the seed plays first");
        let unique = ids.iter().collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), ids.len(), "no repeats");
        // Seed artist, then each similar artist in turn.
        let artists = radio
            .tracks
            .iter()
            .take(5)
            .map(|track| track.artist.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            artists,
            [
                "Sora Ueda",
                "Sora Ueda",
                "Ilse Marten",
                "Northbound Brass",
                "Mara Vell"
            ]
        );
        assert_eq!(ids.len(), 14, "every top track of four artists, once");

        let short = client
            .radio(&context(), RadioSeed::Artist, "51001", 4)
            .await
            .expect("artist radio");
        assert_eq!(short.tracks.len(), 4);
        assert_eq!(short.tracks[0].artist.name, "Ilse Marten");
    }

    #[actix_web::test]
    async fn login_sends_a_hashed_password_and_keeps_only_the_token() {
        let (client, transport) = client();
        let password = generated_password();
        let session = client
            .login(&credentials(), "listener@example.test", &password)
            .await
            .expect("login");
        assert_eq!(session.access_token, "recorded-user-auth-token");
        assert_eq!(session.account_name, "Recorded Listener");
        assert_eq!(session.plan.as_deref(), Some("Studio"));
        let request = transport.requests().pop().expect("login request");
        assert_eq!(request.query_value("email"), Some("listener@example.test"));
        assert_eq!(
            request.query_value("password"),
            Some(hex_digest(&password).as_str())
        );
        assert!(request.query.iter().all(|(_, value)| *value != password));
        assert_eq!(request.header_value("x-app-id"), Some("100000001"));
    }

    #[actix_web::test]
    async fn catalog_pages_carry_hi_res_formats_labels_and_genres() {
        let (client, transport) = client();
        let album = client
            .album(&context(), "q0glassorch01")
            .await
            .expect("album");
        assert_eq!(album.id, "qobuz:q0glassorch01");
        assert_eq!(
            album.format.as_ref().map(|format| format.label.as_str()),
            Some("24/192 FLAC")
        );
        assert_eq!(album.label.as_deref(), Some("Nordlicht Records"));
        assert_eq!(album.genre.as_deref(), Some("Classical"));
        assert_eq!(album.tracks.len(), 3);
        assert_eq!(
            album.tracks[0].album.as_ref().map(|a| a.id.as_str()),
            Some("qobuz:q0glassorch01")
        );
        assert_eq!(album.tracks[0].artist.name, "Ilse Marten");
        assert_eq!(
            transport
                .requests()
                .pop()
                .expect("request")
                .header_value("x-user-auth-token"),
            Some("recorded-user-auth-token")
        );

        let search = client
            .search(&context(), "harbor", 10)
            .await
            .expect("search");
        assert!(
            search
                .albums
                .iter()
                .any(|album| album.title == "Harbor Lights")
        );
        assert!(search.tracks.iter().all(|track| track.album.is_some()));

        let artist = client.artist(&context(), "51003").await.expect("artist");
        assert_eq!(artist.name, "Sora Ueda");
        assert_eq!(artist.albums.len(), 1);

        let favorites = client
            .browse(&context(), BrowseSection::FavoriteAlbums, 0, 50)
            .await
            .expect("favorites");
        assert_eq!(favorites.albums[0].title, "Kites Over Kanazawa");
    }

    #[actix_web::test]
    async fn stream_urls_are_signed_for_the_chosen_format() {
        let (client, transport) = client();
        let stream = client
            .resolve_stream(&context(), "7700001", StreamQuality::HiResMax)
            .await
            .expect("stream");
        assert!(matches!(
            &stream.source,
            StreamSource::Progressive { url, mime } if url.ends_with("fmt=27") && mime == "audio/flac"
        ));
        assert_eq!(stream.format.label, "24/192 FLAC");
        let request = transport.requests().pop().expect("file url request");
        let timestamp: i64 = request
            .query_value("request_ts")
            .and_then(|value| value.parse().ok())
            .expect("timestamp");
        assert_eq!(request.query_value("format_id"), Some("27"));
        assert_eq!(request.query_value("intent"), Some("stream"));
        assert_eq!(
            request.query_value("request_sig"),
            Some(file_url_signature(27, "7700001", timestamp, "listener-app-secret").as_str())
        );
        assert_eq!(
            file_url_signature(27, "7700001", 1_700_000_000, "secret"),
            hex_digest("trackgetFileUrlformat_id27intentstreamtrack_id77000011700000000secret")
        );
    }

    #[actix_web::test]
    async fn downgraded_and_sample_only_streams_are_reported_honestly() {
        let (client, _) = client();
        let cd = client
            .resolve_stream(&context(), "7700001", StreamQuality::Lossless)
            .await
            .expect("cd stream");
        assert_eq!(cd.format.label, "16/44.1 FLAC");
        assert!(matches!(
            client
                .resolve_stream(&context(), "7700001", StreamQuality::High)
                .await,
            Err(ProviderError::Unavailable(_))
        ));
        let mut no_secret = context();
        no_secret.credentials.client_secret = None;
        assert!(matches!(
            client
                .resolve_stream(&no_secret, "7700001", StreamQuality::HiResMax)
                .await,
            Err(ProviderError::InvalidCredentials(_))
        ));
    }

    #[actix_web::test]
    async fn genres_and_editorial_lists_use_album_get_featured() {
        let (client, transport) = client();
        let genres = client.genres(&context()).await.expect("genres");
        assert_eq!(
            genres
                .iter()
                .map(|genre| (genre.id.as_str(), genre.name.as_str()))
                .collect::<Vec<_>>(),
            [
                ("112", "Pop/Rock"),
                ("80", "Jazz"),
                ("10", "Classical"),
                ("64", "Electronic")
            ]
        );

        let jazz = client
            .genre_albums(&context(), "80", BrowseSection::EditorsPicks, 0, 24)
            .await
            .expect("jazz");
        assert_eq!(jazz.albums.len(), 1);
        assert_eq!(jazz.albums[0].title, "Brass for Late Trains");
        assert_eq!(jazz.albums[0].genre.as_deref(), Some("Jazz"));
        let request = transport.requests().pop().expect("request");
        assert!(request.url.ends_with("/album/getFeatured"));
        assert_eq!(request.query_value("genre_id"), Some("80"));
        assert_eq!(request.query_value("type"), Some("editor-picks"));
        assert_eq!(request.query_value("limit"), Some("24"));

        // A favorites section is not an editorial list: genre pages fall
        // back to new releases.
        client
            .genre_albums(&context(), "80", BrowseSection::FavoriteAlbums, 0, 24)
            .await
            .expect("fallback");
        assert_eq!(
            transport
                .requests()
                .pop()
                .expect("request")
                .query_value("type"),
            Some("new-releases-full")
        );

        for (section, kind, first) in [
            (BrowseSection::EditorsPicks, "editor-picks", "Glass Orchard"),
            (BrowseSection::Popular, "most-streamed", "Harbor Lights"),
            (BrowseSection::PressAwards, "press-awards", "Glass Orchard"),
        ] {
            let page = client
                .browse(&context(), section, 0, 50)
                .await
                .expect("editorial");
            assert_eq!(page.albums[0].title, first, "{kind}");
            let request = transport.requests().pop().expect("request");
            assert_eq!(request.query_value("type"), Some(kind));
            assert_eq!(request.query_value("genre_id"), None);
        }
        assert_eq!(client.editorial_sections().len(), 4);
        assert!(client.editorial_sections().iter().all(|s| s.by_genre));
    }

    #[actix_web::test]
    async fn favorites_use_the_qobuz_favorite_endpoints() {
        let (client, transport) = client();
        client
            .set_favorite(&context(), ItemKind::Album, "q0brasslate02", true)
            .await
            .expect("favorite");
        let request = transport.requests().pop().expect("request");
        assert!(request.url.ends_with("/favorite/create"));
        assert_eq!(request.query_value("album_ids"), Some("q0brasslate02"));
        assert!(
            client
                .refresh(&credentials(), &context().session)
                .await
                .expect("refresh")
                .is_none()
        );
    }
}
