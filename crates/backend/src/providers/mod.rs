//! Streaming services (TIDAL, Qobuz) behind one [`StreamingProvider`] trait.
//!
//! Neither service issues API credentials to Parson, so every account is
//! connected with credentials the listener (or the server owner, through
//! environment variables) supplies. Nothing here embeds or scrapes keys.
//! Credentials and tokens are sealed per user by [`vault::TokenVault`].

pub mod artwork;
pub mod catalog;
pub mod hub;
pub mod key_service;
pub mod md5;
pub mod media;
pub mod probe;
pub mod qobuz;
pub mod sandbox;
pub mod sandbox_media;
pub mod store;
pub mod tidal;
pub mod transport;
pub mod vault;

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};

use transport::TransportError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    Tidal,
    Qobuz,
}

impl ProviderId {
    pub const ALL: [ProviderId; 2] = [ProviderId::Tidal, ProviderId::Qobuz];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tidal => "tidal",
            Self::Qobuz => "qobuz",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Tidal => "TIDAL",
            Self::Qobuz => "Qobuz",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "tidal" => Some(Self::Tidal),
            "qobuz" => Some(Self::Qobuz),
            _ => None,
        }
    }
}

/// A provider-scoped catalog id such as `tidal:77646169` or
/// `qobuz:0060254735180`. Library ids never contain `:` with a known
/// provider prefix, so the two namespaces cannot collide.
pub fn entity_id(provider: ProviderId, raw: &str) -> String {
    format!("{}:{raw}", provider.as_str())
}

pub fn parse_entity_id(value: &str) -> Option<(ProviderId, String)> {
    let (prefix, raw) = value.split_once(':')?;
    let provider = ProviderId::parse(prefix)?;
    valid_raw_id(raw).then(|| (provider, raw.to_string()))
}

pub fn valid_raw_id(raw: &str) -> bool {
    !raw.is_empty()
        && raw.len() <= 64
        && raw
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// Streaming quality a listener chooses per service.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamQuality {
    /// Highest available: 24-bit up to 192 kHz FLAC.
    HiResMax,
    /// Qobuz only: 24-bit up to 96 kHz FLAC.
    #[serde(rename = "hi_res_96")]
    HiRes96,
    /// CD quality, 16-bit 44.1 kHz FLAC.
    Lossless,
    /// Lossy, about 320 kbps.
    High,
    /// TIDAL only: lossy, about 96 kbps.
    Low,
}

impl StreamQuality {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HiResMax => "hi_res_max",
            Self::HiRes96 => "hi_res_96",
            Self::Lossless => "lossless",
            Self::High => "high",
            Self::Low => "low",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "hi_res_max" => Some(Self::HiResMax),
            "hi_res_96" => Some(Self::HiRes96),
            "lossless" => Some(Self::Lossless),
            "high" => Some(Self::High),
            "low" => Some(Self::Low),
            _ => None,
        }
    }

    pub fn is_lossless(self) -> bool {
        matches!(self, Self::HiResMax | Self::HiRes96 | Self::Lossless)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct QualityOption {
    pub id: StreamQuality,
    pub label: &'static str,
    pub detail: &'static str,
}

/// Technical format of a stream or of the best version a service offers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioFormat {
    pub codec: String,
    pub lossless: bool,
    pub bit_depth: Option<u8>,
    pub sample_rate: Option<u32>,
    pub bitrate_kbps: Option<u32>,
    /// Known when the format was read from the stream itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<u8>,
    /// Short badge text such as `24/96 FLAC`, `16/44.1 FLAC` or `AAC 320`.
    pub label: String,
}

impl AudioFormat {
    pub fn lossless(codec: &str, bit_depth: Option<u8>, sample_rate: Option<u32>) -> Self {
        let label = match (bit_depth, sample_rate) {
            (Some(depth), Some(rate)) => {
                format!(
                    "{depth}/{} {}",
                    format_khz(rate),
                    codec.to_ascii_uppercase()
                )
            }
            (Some(depth), None) => format!("{depth}-bit {}", codec.to_ascii_uppercase()),
            _ => codec.to_ascii_uppercase(),
        };
        Self {
            codec: codec.to_string(),
            lossless: true,
            bit_depth,
            sample_rate,
            bitrate_kbps: None,
            channels: None,
            label,
        }
    }

    pub fn lossy(codec: &str, bitrate_kbps: u32) -> Self {
        Self {
            codec: codec.to_string(),
            lossless: false,
            bit_depth: None,
            sample_rate: None,
            bitrate_kbps: Some(bitrate_kbps),
            channels: None,
            label: format!("{} {bitrate_kbps}", codec.to_ascii_uppercase()),
        }
    }

    pub fn is_hi_res(&self) -> bool {
        self.lossless
            && (self.bit_depth.is_some_and(|depth| depth > 16)
                || self.sample_rate.is_some_and(|rate| rate > 48_000))
    }
}

/// `44100` -> `44.1`, `96000` -> `96`.
pub fn format_khz(rate: u32) -> String {
    if rate.is_multiple_of(1000) {
        (rate / 1000).to_string()
    } else {
        format!("{:.1}", rate as f64 / 1000.0)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ArtistRef {
    /// Provider-scoped id, e.g. `tidal:3346`.
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AlbumRef {
    pub id: String,
    pub title: String,
    pub cover_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderTrack {
    pub id: String,
    pub provider: ProviderId,
    pub title: String,
    pub version: Option<String>,
    pub artist: ArtistRef,
    pub artists: Vec<ArtistRef>,
    pub album: Option<AlbumRef>,
    pub track_number: u32,
    pub disc_number: u32,
    pub duration: f64,
    pub explicit: bool,
    pub isrc: Option<String>,
    pub format: Option<AudioFormat>,
    pub streamable: bool,
    #[serde(default)]
    pub in_library: bool,
    /// Who played, wrote and produced it, when the service says. Filled on
    /// album pages; empty in lists.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credits: Vec<Credit>,
}

/// One credited role on a track, such as `Producer: A, B`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credit {
    pub role: String,
    pub names: Vec<String>,
}

/// Editorial text a service publishes (an album review or an artist
/// biography), reduced to plain paragraphs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorialText {
    pub text: String,
    /// Who wrote it, e.g. `TiVo` or `Qobuz`, when the service says.
    pub source: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderAlbum {
    pub id: String,
    pub provider: ProviderId,
    pub title: String,
    pub version: Option<String>,
    pub artist: ArtistRef,
    pub artists: Vec<ArtistRef>,
    pub cover_url: Option<String>,
    pub release_date: Option<String>,
    pub track_count: u32,
    pub duration: f64,
    pub explicit: bool,
    pub format: Option<AudioFormat>,
    pub label: Option<String>,
    pub genre: Option<String>,
    pub copyright: Option<String>,
    pub web_url: Option<String>,
    /// Present on album pages; empty in lists and search results.
    #[serde(default)]
    pub tracks: Vec<ProviderTrack>,
    #[serde(default)]
    pub in_library: bool,
    /// The service's review or description, on album pages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<EditorialText>,
    /// Press awards such as `Qobuzissime` or `Diapason d'or`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub awards: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderArtist {
    pub id: String,
    pub provider: ProviderId,
    pub name: String,
    pub picture_url: Option<String>,
    pub albums: Vec<ProviderAlbum>,
    pub top_tracks: Vec<ProviderTrack>,
    pub web_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bio: Option<EditorialText>,
    /// Artists the service considers similar.
    #[serde(default)]
    pub similar: Vec<ArtistSummary>,
}

/// What a radio starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RadioSeed {
    Track,
    Artist,
    Album,
}

impl RadioSeed {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "track" => Some(Self::Track),
            "artist" => Some(Self::Artist),
            "album" => Some(Self::Album),
            _ => None,
        }
    }
}

/// Who chose a radio's tracks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RadioOrigin {
    /// The service's own radio (TIDAL track and artist radio).
    Service,
    /// Parson, from the service's similar artists and their top tracks
    /// (Qobuz publishes no radio endpoint).
    SimilarArtists,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Radio {
    pub tracks: Vec<ProviderTrack>,
    pub origin: RadioOrigin,
}

/// Tracks in a radio.
pub const RADIO_TRACKS: u32 = 50;

/// A service playlist: the listener's own or one they follow. Parson reads
/// playlists and never edits them on the service.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderPlaylist {
    pub id: String,
    pub provider: ProviderId,
    pub title: String,
    pub description: Option<String>,
    /// Who made it, e.g. the listener or `TIDAL`.
    pub owner: Option<String>,
    /// Whether the listener created it (as opposed to following it).
    pub own: bool,
    pub track_count: u32,
    pub duration: f64,
    pub cover_url: Option<String>,
    /// Unix seconds of the service's last change, when it reports one.
    pub updated_at: Option<i64>,
    pub web_url: Option<String>,
    /// Present on playlist pages; empty in lists.
    #[serde(default)]
    pub tracks: Vec<ProviderTrack>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaylistPage {
    pub playlists: Vec<ProviderPlaylist>,
    pub total: Option<u32>,
}

/// Playlists longer than this are read up to this many tracks.
pub const MAX_PLAYLIST_TRACKS: u32 = 1_000;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchResults {
    pub albums: Vec<ProviderAlbum>,
    pub tracks: Vec<ProviderTrack>,
    pub artists: Vec<ArtistSummary>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtistSummary {
    pub id: String,
    pub provider: ProviderId,
    pub name: String,
    pub picture_url: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowseSection {
    NewReleases,
    /// TIDAL "Top", Qobuz "Most streamed".
    Popular,
    /// TIDAL "Recommended", Qobuz "Editor's picks".
    EditorsPicks,
    /// Qobuz only: albums with press awards.
    PressAwards,
    FavoriteAlbums,
    FavoriteTracks,
}

impl BrowseSection {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "new_releases" => Some(Self::NewReleases),
            "popular" => Some(Self::Popular),
            "editors_picks" => Some(Self::EditorsPicks),
            "press_awards" => Some(Self::PressAwards),
            "favorite_albums" => Some(Self::FavoriteAlbums),
            "favorite_tracks" => Some(Self::FavoriteTracks),
            _ => None,
        }
    }

    pub fn is_editorial(self) -> bool {
        matches!(
            self,
            Self::NewReleases | Self::Popular | Self::EditorsPicks | Self::PressAwards
        )
    }
}

/// An editorial album list a service publishes, in the order its own app
/// shows them.
#[derive(Clone, Debug, Serialize)]
pub struct EditorialSection {
    pub id: BrowseSection,
    pub label: &'static str,
    /// Whether the list can be narrowed to one genre.
    pub by_genre: bool,
}

/// A genre as the service names it. `id` is the service's own key (a number
/// on Qobuz, a path such as `Pop` on TIDAL).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderGenre {
    pub id: String,
    pub provider: ProviderId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BrowsePage {
    pub albums: Vec<ProviderAlbum>,
    pub tracks: Vec<ProviderTrack>,
    pub total: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Album,
    Track,
}

impl ItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Album => "album",
            Self::Track => "track",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "album" | "albums" => Some(Self::Album),
            "track" | "tracks" => Some(Self::Track),
            _ => None,
        }
    }
}

/// Which copy of an album plays when it is in more than one place: the
/// local library, TIDAL or Qobuz.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourcePreference {
    /// The highest bit depth and sample rate wins; local files win ties.
    #[default]
    HighestQuality,
    Local,
    Tidal,
    Qobuz,
}

impl SourcePreference {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HighestQuality => "highest_quality",
            Self::Local => "local",
            Self::Tidal => "tidal",
            Self::Qobuz => "qobuz",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "highest_quality" => Some(Self::HighestQuality),
            "local" => Some(Self::Local),
            "tidal" => Some(Self::Tidal),
            "qobuz" => Some(Self::Qobuz),
            _ => None,
        }
    }
}

/// Where the audio for a resolved stream comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum StreamSource {
    /// One HTTPS resource that supports byte ranges.
    Progressive { url: String, mime: String },
    /// A DASH representation: an initialization segment then media segments,
    /// served back-to-back as one fragmented MP4 stream.
    Segmented {
        mime: String,
        init_url: Option<String>,
        segment_urls: Vec<String>,
    },
    /// Sandbox mode: a synthesized preview tone instead of a real stream.
    SandboxTone { seed: u64, seconds: u32 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedStream {
    pub source: StreamSource,
    pub format: AudioFormat,
    pub requested: StreamQuality,
    /// The length of the media the service handed over, when its manifest
    /// lists it (a DASH `SegmentTimeline`). Byte-level lengths come from
    /// the probe instead.
    pub stream_seconds: Option<f64>,
}

/// API credentials. For TIDAL these are an OAuth client id and optional
/// secret; for Qobuz an app id and either an app secret or, for an app a
/// Parson key service holds, that service's URL (it signs stream requests so
/// the secret never reaches this server).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientCredentials {
    pub client_id: String,
    pub client_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_service: Option<String>,
}

/// A signed-in session. Only ever stored sealed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSession {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix seconds; `None` for tokens without a known lifetime.
    pub expires_at: Option<i64>,
    pub user_id: String,
    pub account_name: String,
    pub country_code: Option<String>,
    pub plan: Option<String>,
}

pub struct ProviderContext {
    pub credentials: ClientCredentials,
    pub session: ProviderSession,
}

#[derive(Debug)]
pub enum ProviderError {
    /// No credentials or session for this service.
    NotConnected,
    /// The session expired or was revoked; the listener must sign in again.
    SessionExpired,
    /// The service rejected the app or user credentials.
    InvalidCredentials(String),
    NotFound,
    /// The account or content cannot be streamed (region, plan, encryption).
    Unavailable(String),
    RateLimited,
    Upstream(u16),
    Decode(String),
    Transport(TransportError),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConnected => formatter.write_str("the service is not connected"),
            Self::SessionExpired => formatter.write_str("the service session expired"),
            Self::InvalidCredentials(message) => {
                write!(formatter, "invalid credentials: {message}")
            }
            Self::NotFound => formatter.write_str("the item was not found on the service"),
            Self::Unavailable(message) => write!(formatter, "unavailable: {message}"),
            Self::RateLimited => formatter.write_str("the service is rate limiting requests"),
            Self::Upstream(status) => write!(formatter, "the service answered HTTP {status}"),
            Self::Decode(message) => write!(formatter, "unexpected service response: {message}"),
            Self::Transport(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ProviderError {}

impl From<TransportError> for ProviderError {
    fn from(error: TransportError) -> Self {
        Self::Transport(error)
    }
}

pub type ProviderResult<T> = Result<T, ProviderError>;

/// Catalog access shared by every streaming service. Sign-in differs per
/// service (device code for TIDAL, app credentials plus password login for
/// Qobuz) and lives on the concrete clients.
pub trait StreamingProvider: Send + Sync {
    fn id(&self) -> ProviderId;

    fn quality_options(&self) -> &'static [QualityOption];

    fn default_quality(&self) -> StreamQuality {
        StreamQuality::Lossless
    }

    fn search<'a>(
        &'a self,
        context: &'a ProviderContext,
        query: &'a str,
        limit: u32,
    ) -> BoxFuture<'a, ProviderResult<SearchResults>>;

    fn album<'a>(
        &'a self,
        context: &'a ProviderContext,
        raw_id: &'a str,
    ) -> BoxFuture<'a, ProviderResult<ProviderAlbum>>;

    fn artist<'a>(
        &'a self,
        context: &'a ProviderContext,
        raw_id: &'a str,
    ) -> BoxFuture<'a, ProviderResult<ProviderArtist>>;

    fn track<'a>(
        &'a self,
        context: &'a ProviderContext,
        raw_id: &'a str,
    ) -> BoxFuture<'a, ProviderResult<ProviderTrack>>;

    fn browse<'a>(
        &'a self,
        context: &'a ProviderContext,
        section: BrowseSection,
        offset: u32,
        limit: u32,
    ) -> BoxFuture<'a, ProviderResult<BrowsePage>>;

    /// The editorial lists this service publishes (new releases, top
    /// albums, editor's picks, ...).
    fn editorial_sections(&self) -> &'static [EditorialSection];

    /// The service's top-level genres.
    fn genres<'a>(
        &'a self,
        context: &'a ProviderContext,
    ) -> BoxFuture<'a, ProviderResult<Vec<ProviderGenre>>>;

    /// Albums in one genre. `section` picks the editorial list where the
    /// service can narrow it by genre; otherwise the genre's own page.
    fn genre_albums<'a>(
        &'a self,
        context: &'a ProviderContext,
        genre_id: &'a str,
        section: BrowseSection,
        offset: u32,
        limit: u32,
    ) -> BoxFuture<'a, ProviderResult<BrowsePage>>;

    /// Adds or removes a service favorite. Parson's "Add to library" is a
    /// two-way favorite, as listeners expect from the service's own apps.
    fn set_favorite<'a>(
        &'a self,
        context: &'a ProviderContext,
        kind: ItemKind,
        raw_id: &'a str,
        favorite: bool,
    ) -> BoxFuture<'a, ProviderResult<()>>;

    fn resolve_stream<'a>(
        &'a self,
        context: &'a ProviderContext,
        raw_id: &'a str,
        quality: StreamQuality,
    ) -> BoxFuture<'a, ProviderResult<ResolvedStream>>;

    /// A radio of tracks like the seed: TIDAL's own track or artist radio,
    /// or for Qobuz a mix of the artist's and similar artists' top tracks.
    /// Album seeds are resolved by the hub to the album's first track.
    fn radio<'a>(
        &'a self,
        context: &'a ProviderContext,
        seed: RadioSeed,
        raw_id: &'a str,
        limit: u32,
    ) -> BoxFuture<'a, ProviderResult<Radio>>;

    /// The listener's playlists: their own and the ones they follow.
    fn playlists<'a>(
        &'a self,
        context: &'a ProviderContext,
        offset: u32,
        limit: u32,
    ) -> BoxFuture<'a, ProviderResult<PlaylistPage>>;

    /// One playlist with its tracks (up to [`MAX_PLAYLIST_TRACKS`]).
    fn playlist<'a>(
        &'a self,
        context: &'a ProviderContext,
        raw_id: &'a str,
    ) -> BoxFuture<'a, ProviderResult<ProviderPlaylist>>;

    /// Renews an expiring session. `None` means the service has no refresh
    /// flow and the listener must sign in again.
    fn refresh<'a>(
        &'a self,
        credentials: &'a ClientCredentials,
        session: &'a ProviderSession,
    ) -> BoxFuture<'a, ProviderResult<Option<ProviderSession>>>;
}

/// Longest editorial text kept, in characters.
const MAX_EDITORIAL_CHARS: usize = 20_000;

/// Service markup (Qobuz HTML, TIDAL `[wimpLink]` tags) to plain text with
/// paragraph breaks kept. Returns `None` for text that is empty once
/// stripped.
pub(crate) fn plain_text(markup: &str) -> Option<String> {
    let mut text = String::with_capacity(markup.len());
    let mut rest = markup;
    while let Some(start) = rest.find(['<', '[']) {
        text.push_str(&rest[..start]);
        let opener = rest.as_bytes()[start];
        let close = if opener == b'<' { '>' } else { ']' };
        let tail = &rest[start..];
        let Some(end) = tail.find(close) else {
            text.push_str(tail);
            rest = "";
            break;
        };
        let tag = tail[1..end].trim().to_ascii_lowercase();
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("");
        let known = if opener == b'<' {
            !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_alphanumeric())
        } else {
            name == "wimplink"
        };
        if known {
            if matches!(name, "br" | "p" | "div" | "li") {
                text.push('\n');
                if name != "br" {
                    text.push('\n');
                }
            }
        } else {
            text.push_str(&tail[..=end]);
        }
        rest = &tail[end + 1..];
    }
    text.push_str(rest);
    let decoded = text
        .replace("&nbsp;", " ")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&rsquo;", "\u{2019}")
        .replace("&lsquo;", "\u{2018}")
        .replace("&ldquo;", "\u{201c}")
        .replace("&rdquo;", "\u{201d}")
        .replace("&hellip;", "\u{2026}")
        .replace("&ndash;", "\u{2013}")
        .replace("&mdash;", "\u{2014}")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    let paragraphs = decoded
        .split("\n\n")
        .map(|paragraph| {
            paragraph
                .lines()
                .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|paragraph| !paragraph.is_empty())
        .collect::<Vec<_>>();
    let mut joined = paragraphs.join("\n\n");
    if joined.chars().count() > MAX_EDITORIAL_CHARS {
        joined = joined.chars().take(MAX_EDITORIAL_CHARS).collect::<String>();
        joined.push('\u{2026}');
    }
    (!joined.is_empty()).then_some(joined)
}

/// Adds `name` under `role`, keeping first-seen order and no duplicates.
pub(crate) fn push_credit(credits: &mut Vec<Credit>, role: &str, name: &str) {
    let (role, name) = (role.trim(), name.trim());
    if role.is_empty() || name.is_empty() {
        return;
    }
    match credits.iter_mut().find(|credit| credit.role == role) {
        Some(credit) => {
            if !credit.names.iter().any(|existing| existing == name) {
                credit.names.push(name.to_string());
            }
        }
        None => credits.push(Credit {
            role: role.to_string(),
            names: vec![name.to_string()],
        }),
    }
}

/// Round-robins several track lists into one radio, skipping repeats.
pub(crate) fn interleave_tracks(
    lists: Vec<Vec<ProviderTrack>>,
    limit: usize,
) -> Vec<ProviderTrack> {
    let mut seen = std::collections::HashSet::new();
    let mut iterators = lists.into_iter().map(Vec::into_iter).collect::<Vec<_>>();
    let mut tracks = Vec::new();
    loop {
        let mut advanced = false;
        for iterator in &mut iterators {
            if tracks.len() >= limit {
                return tracks;
            }
            for track in iterator.by_ref() {
                advanced = true;
                if seen.insert(track.id.clone()) {
                    tracks.push(track);
                    break;
                }
            }
        }
        if !advanced {
            return tracks;
        }
    }
}

pub(crate) fn decode_json<T: serde::de::DeserializeOwned>(body: &[u8]) -> ProviderResult<T> {
    serde_json::from_slice(body).map_err(|error| ProviderError::Decode(error.to_string()))
}

/// Maps a non-success HTTP status to a provider error.
pub(crate) fn status_error(status: u16) -> ProviderError {
    match status {
        401 => ProviderError::SessionExpired,
        403 => ProviderError::Unavailable("the service refused access to this content".into()),
        404 => ProviderError::NotFound,
        429 => ProviderError::RateLimited,
        status => ProviderError::Upstream(status),
    }
}

pub(crate) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        AudioFormat, Credit, ProviderId, entity_id, format_khz, parse_entity_id, plain_text,
        push_credit,
    };

    #[test]
    fn editorial_markup_becomes_plain_paragraphs() {
        assert_eq!(
            plain_text(
                "<p>A <strong>bold</strong> debut &amp; more.</p><p>Second&nbsp;part<br/>line two</p>"
            )
            .as_deref(),
            Some("A bold debut & more.\n\nSecond part\nline two")
        );
        assert_eq!(
            plain_text("Produced with [wimpLink artistId=\"4001\"]Mara Vell[/wimpLink].")
                .as_deref(),
            Some("Produced with Mara Vell.")
        );
        assert_eq!(
            plain_text("2 < 3 and [brackets] stay").as_deref(),
            Some("2 < 3 and [brackets] stay")
        );
        assert_eq!(plain_text("<p> </p>"), None);
    }

    #[test]
    fn credits_group_names_by_role() {
        let mut credits = Vec::new();
        push_credit(&mut credits, "Producer", "A");
        push_credit(&mut credits, "Composer", "B");
        push_credit(&mut credits, "Producer", "C");
        push_credit(&mut credits, "Producer", "A");
        push_credit(&mut credits, " ", "D");
        assert_eq!(
            credits,
            vec![
                Credit {
                    role: "Producer".into(),
                    names: vec!["A".into(), "C".into()]
                },
                Credit {
                    role: "Composer".into(),
                    names: vec!["B".into()]
                },
            ]
        );
    }

    #[test]
    fn provider_ids_never_collide_with_library_ids() {
        assert_eq!(entity_id(ProviderId::Tidal, "77646169"), "tidal:77646169");
        assert_eq!(
            parse_entity_id("qobuz:0060254735180"),
            Some((ProviderId::Qobuz, "0060254735180".to_string()))
        );
        for library_like in [
            "9f0c2d1e-0a7b-4c55-8f60-1b2c3d4e5f60",
            "spotify:123",
            "tidal:",
            "tidal:../../etc",
            "tidal:a b",
            "/music/tidal:1.flac",
        ] {
            assert_eq!(parse_entity_id(library_like), None, "{library_like}");
        }
    }

    #[test]
    fn format_labels_read_like_a_quality_badge() {
        assert_eq!(format_khz(44_100), "44.1");
        assert_eq!(format_khz(192_000), "192");
        assert_eq!(
            AudioFormat::lossless("flac", Some(24), Some(96_000)).label,
            "24/96 FLAC"
        );
        assert!(AudioFormat::lossless("flac", Some(24), Some(96_000)).is_hi_res());
        assert!(!AudioFormat::lossless("flac", Some(16), Some(44_100)).is_hi_res());
        assert_eq!(AudioFormat::lossy("aac", 320).label, "AAC 320");
    }
}
