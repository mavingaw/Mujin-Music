use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Album {
    pub id: String,
    pub name: String,
    pub cover_url: String,
    pub songs: Vec<Song>,
    pub first_release_date: String,
    pub musicbrainz_id: String,
    pub wikidata_id: Option<String>,
    pub primary_type: String,
    pub description: String,
    pub contributing_artists: Vec<String>,
    pub contributing_artists_ids: Vec<String>,
    pub release_album: Option<ReleaseAlbum>,
    pub release_group_album: Option<ReleaseGroupAlbum>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Artist {
    pub id: String,
    pub name: String,
    pub icon_url: String,
    pub followers: u64,
    pub albums: Vec<Album>,
    pub featured_on_album_ids: Vec<String>,
    pub description: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Song {
    pub id: String,
    pub name: String,
    pub artist: String,
    pub contributing_artists: Vec<String>,
    pub contributing_artist_ids: Vec<String>,
    pub track_number: u16,
    pub path: String,
    pub duration: f64,
    /// Technical properties of the primary file, when the indexer read them.
    #[serde(default)]
    pub audio: Option<AudioProperties>,
    /// Contributor credits (composer, producer, performer, ...) from tags.
    #[serde(default)]
    pub credits: Vec<Credit>,
    #[serde(default)]
    pub isrc: Option<String>,
    /// Record label tagged on the primary file.
    #[serde(default)]
    pub label: Option<String>,
}

/// Technical properties of an audio file, as read from its stream headers.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioProperties {
    /// Codec, e.g. `flac`, `alac`, `aac`, `mp3`, `vorbis`, `opus`, `pcm`.
    pub codec: Option<String>,
    /// Container, e.g. `flac`, `mp4`, `mp3`, `ogg`, `wav`, `aiff`.
    pub container: Option<String>,
    /// Sample rate in Hz.
    pub sample_rate: Option<u32>,
    /// Bits per sample for lossless and PCM codecs.
    pub bit_depth: Option<u8>,
    pub channels: Option<u8>,
    /// Average audio bitrate in kbps.
    pub bitrate_kbps: Option<u32>,
    /// Whether the codec is lossless (`None` when the codec is unknown).
    pub lossless: Option<bool>,
}

impl AudioProperties {
    pub fn is_empty(&self) -> bool {
        self.codec.is_none()
            && self.container.is_none()
            && self.sample_rate.is_none()
            && self.bit_depth.is_none()
            && self.channels.is_none()
            && self.bitrate_kbps.is_none()
    }
}

/// Whether a codec name reported in [`AudioProperties::codec`] is lossless.
pub fn codec_is_lossless(codec: &str) -> Option<bool> {
    match codec {
        "flac" | "alac" | "pcm" | "pcm_float" | "wavpack" | "ape" | "tta" => Some(true),
        "mp3" | "aac" | "vorbis" | "opus" | "speex" | "musepack" | "ac3" | "eac3" => Some(false),
        _ => None,
    }
}

/// A contributor credit on a track.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Credit {
    pub artist_id: String,
    pub name: String,
    /// Canonical role, e.g. `composer`, `conductor`, `producer`, `performer`.
    pub role: String,
    /// Instrument for `performer` credits, lower case, when tagged.
    #[serde(default)]
    pub instrument: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ReleaseGroupAlbum {
    pub rating: Rating,
    pub artist_credit: Vec<CreditArtist>,
    pub relationships: Vec<Relationship>,
    pub releases: Vec<Information>,
    pub musicbrainz_id: String,
    pub first_release_date: String,
    pub title: String,
    pub aliases: Vec<Alias>,
    pub primary_type_id: String,
    pub annotation: String,
    pub tags: Vec<Tag>,
    pub genres: Vec<Genre>,
}

impl Default for ReleaseGroupAlbum {
    fn default() -> Self {
        ReleaseGroupAlbum {
            rating: Rating::default(),
            artist_credit: Vec::new(),
            relationships: Vec::new(),
            releases: Vec::new(),
            musicbrainz_id: String::new(),
            first_release_date: String::new(),
            title: String::new(),
            aliases: Vec::new(),
            primary_type_id: String::new(),
            annotation: String::new(),
            tags: vec![Tag::default()],
            genres: vec![Genre::default()],
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ReleaseAlbum {
    pub information: Information,
    pub tracks: Vec<Track>,
    pub labels: Vec<Label>,
    pub relationships: Vec<Relationship>,
    pub musicbrainz_id: String,
    pub first_release_date: String,
    pub title: String,
    pub aliases: Vec<Alias>,
    pub primary_type_id: String,
    pub annotation: String,
    pub tags: Vec<Tag>,
    pub genres: Vec<Genre>,
}

impl Default for ReleaseAlbum {
    fn default() -> Self {
        ReleaseAlbum {
            information: Information::default(),
            tracks: Vec::new(),
            labels: Vec::new(),
            relationships: vec![Relationship::default()],
            musicbrainz_id: String::default(),
            first_release_date: String::default(),
            title: String::default(),
            aliases: Vec::new(),
            primary_type_id: String::default(),
            annotation: String::default(),
            tags: Vec::new(),
            genres: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Information {
    pub date: String,
    pub country: String,
    pub status_id: String,
    pub title: String,
    pub barcode: String,
    pub quality: String,
    pub packaging: String,
    pub disambiguation: String,
    pub release_type: String,
    pub asin: String,
    pub music_brainz_id: String,
    pub packaging_id: String,
    pub status: String,
    pub tags: Vec<Tag>,
    pub genres: Vec<Genre>,
    pub cover_art_status: CoverArtStatus,
    pub collections: Vec<Collection>,
    pub artist_credits: Vec<CreditArtist>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct CoverArtStatus {
    pub count: u16,
    pub front: String,
    pub darkened: String,
    pub artwork: String,
    pub back: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct CreditArtist {
    pub name: String,
    pub join_phrase: String,
    pub musicbrainz_id: String,
    pub artist_type: String,
    pub disambiguation: String,
    pub genres: Vec<Genre>,
    pub aliases: Vec<Alias>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Genre {
    pub musicbrainz_id: String,
    pub disambiguation: String,
    pub name: String,
    pub count: u64,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Alias {
    pub begin: String,
    pub alias_type: String,
    pub sort_name: String,
    pub name: String,
    pub end: String,
    pub locale: String,
    pub ended: bool,
    pub type_id: String,
    pub primary: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Collection {
    pub entity_type: String,
    pub type_id: String,
    pub name: String,
    pub editor: String,
    pub release_count: u64,
    pub id: String,
    pub collection_type: String,
    pub secondary_type_ids: Vec<String>,
    pub tags: Vec<Tag>,
    pub artist_credit: Vec<CreditArtist>,
    pub aliases: Vec<String>,
    pub secondary_types: Vec<String>,
    pub disambiguation: String,
    pub first_release_date: String,
}

impl Default for Collection {
    fn default() -> Self {
        Collection {
            entity_type: String::new(),
            type_id: String::new(),
            name: String::new(),
            editor: String::new(),
            release_count: 0,
            id: String::new(),
            collection_type: String::new(),
            secondary_type_ids: Vec::new(),
            tags: Vec::new(),
            artist_credit: vec![CreditArtist::default()],
            aliases: Vec::new(),
            secondary_types: Vec::new(),
            disambiguation: String::new(),
            first_release_date: String::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Track {
    pub length: u64,
    pub artist_credit: Vec<CreditArtist>,
    pub track_name: String,
    pub position: u16,
    pub video: bool,
    pub first_release_date: String,
    pub number: String,
    pub musicbrainz_id: String,
    pub rating: Rating,
    pub tags: Vec<Tag>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Rating {
    pub votes_count: u64,
    pub value: f64,
}

impl Default for Rating {
    fn default() -> Self {
        Rating {
            votes_count: 0,
            value: 0.0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Tag {
    pub count: u64,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Label {
    pub catalog_number: String,
    pub type_id: String,
    pub name: String,
    pub sort_name: String,
    pub label_type: String,
    pub id: String,
    pub aliases: Vec<Alias>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Relationship {
    pub direction: String,
    pub type_id: String,
    pub ended: bool,
    pub begin: String,
    pub purchase_relationship_type: String,
    pub musicbrainz_id: String,
    pub target_credit: String,
    pub source_credit: String,
    pub target_type: String,
    pub end: String,
    pub url: String,
}
