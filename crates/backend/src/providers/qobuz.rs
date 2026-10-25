//! Qobuz: app-credential + user login, catalog, favorites and signed stream
//! URL resolution (`track/getFileUrl`).
//!
//! Qobuz issues app ids and secrets only to partners, so the listener (or the
//! server owner) supplies both. Parson never extracts them from Qobuz's apps.

use std::sync::Arc;

use futures::future::BoxFuture;
use serde::{Deserialize, Deserializer};

use super::artwork::ArtworkMode;
use super::md5::hex_digest;
use super::transport::{ApiRequest, Transport};
use super::{
    AlbumRef, ArtistRef, ArtistSummary, AudioFormat, BrowsePage, BrowseSection, ClientCredentials,
    ItemKind, MAX_PLAYLIST_TRACKS, PlaylistPage, ProviderAlbum, ProviderArtist, ProviderContext,
    ProviderError, ProviderId, ProviderPlaylist, ProviderResult, ProviderSession, ProviderTrack,
    QualityOption, ResolvedStream, SearchResults, StreamQuality, StreamSource, StreamingProvider,
    decode_json, entity_id, status_error, unix_now,
};

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

pub struct QobuzClient {
    transport: Arc<dyn Transport>,
    artwork: ArtworkMode,
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
        Box::pin(async move {
            let body = self
                .get(
                    context,
                    "artist/get",
                    &[
                        ("artist_id", raw_id.to_string()),
                        ("extra", "albums,tracks".to_string()),
                        ("limit", "50".to_string()),
                    ],
                )
                .await?;
            let artist: QArtist = decode_json(&body)?;
            let summary = self.artist_summary(&artist);
            let artist_ref = Self::artist_ref(&artist);
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
            })
        })
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
                BrowseSection::NewReleases => {
                    let body = self
                        .get(
                            context,
                            "album/getFeatured",
                            &[
                                ("type", "new-releases-full".to_string()),
                                ("offset", offset.to_string()),
                                ("limit", limit.to_string()),
                            ],
                        )
                        .await?;
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
            let Some(secret) = context
                .credentials
                .client_secret
                .as_deref()
                .filter(|secret| !secret.is_empty())
            else {
                return Err(ProviderError::InvalidCredentials(
                    "Add your Qobuz app secret to play music.".into(),
                ));
            };
            let format = format_id(quality);
            let timestamp = unix_now();
            let signature = file_url_signature(format, raw_id, timestamp, secret);
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

    use super::{QobuzClient, file_url_signature};
    use crate::providers::artwork::ArtworkMode;
    use crate::providers::md5::hex_digest;
    use crate::providers::transport::FixtureTransport;
    use crate::providers::{
        BrowseSection, ClientCredentials, ItemKind, ProviderContext, ProviderError,
        ProviderSession, StreamQuality, StreamSource, StreamingProvider,
    };

    const FIXTURES: &str = include_str!("fixtures/qobuz.json");

    fn client() -> (QobuzClient, Arc<FixtureTransport>) {
        let transport = Arc::new(FixtureTransport::from_json(&[FIXTURES]).expect("fixtures"));
        (
            QobuzClient::new(transport.clone(), ArtworkMode::Live),
            transport,
        )
    }

    fn credentials() -> ClientCredentials {
        ClientCredentials {
            client_id: "100000001".into(),
            client_secret: Some("listener-app-secret".into()),
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

    #[actix_web::test]
    async fn login_sends_a_hashed_password_and_keeps_only_the_token() {
        let (client, transport) = client();
        let session = client
            .login(&credentials(), "listener@example.test", "correct horse")
            .await
            .expect("login");
        assert_eq!(session.access_token, "recorded-user-auth-token");
        assert_eq!(session.account_name, "Recorded Listener");
        assert_eq!(session.plan.as_deref(), Some("Studio"));
        let request = transport.requests().pop().expect("login request");
        assert_eq!(request.query_value("email"), Some("listener@example.test"));
        assert_eq!(
            request.query_value("password"),
            Some(hex_digest("correct horse").as_str())
        );
        assert!(
            request
                .query
                .iter()
                .all(|(_, value)| value != "correct horse")
        );
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
