//! Stations as Parson stores and shows them, and their quality tiers.

use serde::{Deserialize, Serialize};

/// Song ids of live stations: `radio:<station id>`.
pub const SONG_ID_PREFIX: &str = "radio:";

/// Where a station comes from.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StationSource {
    /// The community directory at radio-browser.info.
    Directory,
    /// Added by URL.
    Custom,
}

impl StationSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Directory => "directory",
            Self::Custom => "custom",
        }
    }
}

/// A station. Stored whole (as JSON) when saved to My Live Radio, so saved
/// stations keep working if the directory is unreachable.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Station {
    /// radio-browser `stationuuid`, or a UUID Parson gave a custom station.
    pub id: String,
    pub source: StationSource,
    pub name: String,
    /// The URL to play: a stream, a playlist file or an HLS playlist.
    pub stream_url: String,
    #[serde(default)]
    pub homepage: Option<String>,
    /// Remote logo. Clients never get it directly: logos go through
    /// Parson, so listeners' addresses are not sent to station hosts.
    #[serde(default)]
    pub favicon: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub country_code: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Codec as the directory or the stream's headers state it.
    #[serde(default)]
    pub codec: Option<String>,
    #[serde(default)]
    pub bitrate_kbps: Option<u32>,
    #[serde(default)]
    pub hls: bool,
    #[serde(default)]
    pub votes: Option<u64>,
    #[serde(default)]
    pub clicks: Option<u64>,
    #[serde(default)]
    pub click_trend: Option<i64>,
    /// The directory's last check reached the stream.
    #[serde(default)]
    pub online: Option<bool>,
    #[serde(default)]
    pub description: Option<String>,
    /// Allowed to reach private network addresses (set by an admin).
    #[serde(default)]
    pub allow_lan: bool,
}

impl Station {
    pub fn song_id(&self) -> String {
        song_id(&self.id)
    }
}

pub fn song_id(station_id: &str) -> String {
    format!("{SONG_ID_PREFIX}{station_id}")
}

/// Station ids are UUIDs (directory and custom alike).
pub fn valid_station_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

/// The station id inside a `radio:<id>` song id.
pub fn station_id_from_song(song_id: &str) -> Option<&str> {
    song_id
        .strip_prefix(SONG_ID_PREFIX)
        .filter(|id| valid_station_id(id))
}

pub fn is_live_song_id(song_id: &str) -> bool {
    station_id_from_song(song_id).is_some()
}

/// Normalises codec names from directories and content types.
pub fn normalize_codec(value: &str) -> Option<String> {
    let lowered = value.trim().to_ascii_lowercase();
    let codec = match lowered.as_str() {
        "" | "unknown" | "unknow" => return None,
        "mp3" | "mpeg" | "audio/mpeg" | "audio/mp3" | "mpga" => "mp3",
        "aac" | "aac+" | "aacp" | "he-aac" | "audio/aac" | "audio/aacp" | "audio/x-aac" => "aac",
        "flac" | "audio/flac" | "audio/x-flac" => "flac",
        "ogg" | "vorbis" | "audio/ogg" | "application/ogg" | "audio/vorbis" => "vorbis",
        "opus" | "audio/opus" => "opus",
        "alac" => "alac",
        "wav" | "audio/wav" | "audio/x-wav" | "pcm" => "pcm",
        "wma" => "wma",
        _ => {
            // radio-browser writes "AAC,H.264" for video streams.
            if let Some(first) = lowered.split(',').next()
                && first != lowered
            {
                return normalize_codec(first);
            }
            return Some(lowered);
        }
    };
    Some(codec.to_string())
}

pub fn codec_is_lossless(codec: &str) -> bool {
    matches!(codec, "flac" | "alac" | "pcm")
}

pub fn codec_label(codec: &str) -> String {
    match codec {
        "mp3" => "MP3".into(),
        "aac" => "AAC".into(),
        "flac" => "FLAC".into(),
        "vorbis" => "Ogg Vorbis".into(),
        "opus" => "Opus".into(),
        "alac" => "ALAC".into(),
        "pcm" => "PCM".into(),
        "wma" => "WMA".into(),
        other => other.to_ascii_uppercase(),
    }
}

/// How good a station sounds, Roon-style but honest about lossy radio.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum QualityTier {
    Unknown,
    /// Below 96 kbps lossy (or 64 kbps for AAC/Opus).
    Low,
    /// Typical lossy radio.
    Standard,
    /// Transparent-for-most lossy: MP3 256+, AAC/Vorbis 192+, Opus 128+.
    High,
    /// FLAC, ALAC or PCM.
    Lossless,
}

impl QualityTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Low => "low",
            Self::Standard => "standard",
            Self::High => "high",
            Self::Lossless => "lossless",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "low" => Some(Self::Low),
            "standard" => Some(Self::Standard),
            "high" => Some(Self::High),
            "lossless" => Some(Self::Lossless),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

/// A quality badge.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Quality {
    pub tier: QualityTier,
    /// e.g. `FLAC`, `AAC 320`, `MP3 128`.
    pub label: String,
    pub lossless: bool,
    /// Read from the station's audio by Parson, not taken from a listing.
    pub verified: bool,
}

/// Classifies a codec and bitrate.
pub fn classify(codec: Option<&str>, bitrate_kbps: Option<u32>) -> Quality {
    let codec = codec.and_then(normalize_codec);
    let bitrate = bitrate_kbps.filter(|&rate| rate > 0);
    let Some(codec) = codec else {
        return Quality {
            tier: QualityTier::Unknown,
            label: bitrate
                .map(|rate| format!("{rate} kbps"))
                .unwrap_or_else(|| "Unknown format".into()),
            lossless: false,
            verified: false,
        };
    };
    if codec_is_lossless(&codec) {
        return Quality {
            tier: QualityTier::Lossless,
            label: codec_label(&codec),
            lossless: true,
            verified: false,
        };
    }
    let label = match bitrate {
        Some(rate) => format!("{} {rate}", codec_label(&codec)),
        None => codec_label(&codec),
    };
    let (high, low) = match codec.as_str() {
        "opus" => (128, 48),
        "aac" | "vorbis" => (192, 64),
        _ => (256, 96),
    };
    let tier = match bitrate {
        None => QualityTier::Unknown,
        Some(rate) if rate >= high => QualityTier::High,
        Some(rate) if rate < low => QualityTier::Low,
        Some(_) => QualityTier::Standard,
    };
    Quality {
        tier,
        label,
        lossless: false,
        verified: false,
    }
}

/// A station as clients see it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct StationView {
    pub id: String,
    pub song_id: String,
    pub source: StationSource,
    pub name: String,
    pub stream_url: String,
    pub homepage: Option<String>,
    /// Parson's proxied logo path, when the station has a logo.
    pub logo_url: Option<String>,
    pub country: Option<String>,
    pub country_code: Option<String>,
    pub region: Option<String>,
    pub languages: Vec<String>,
    pub tags: Vec<String>,
    pub codec: Option<String>,
    pub bitrate_kbps: Option<u32>,
    pub hls: bool,
    pub quality: Quality,
    pub votes: Option<u64>,
    pub clicks: Option<u64>,
    pub click_trend: Option<i64>,
    pub online: Option<bool>,
    pub description: Option<String>,
    pub allow_lan: bool,
    /// In the listener's My Live Radio.
    pub saved: bool,
    /// What Parson heard when it last opened the station.
    pub verified_format: Option<super::relay::SourceFormat>,
}

pub fn logo_path(station_id: &str) -> String {
    format!("/api/v1/live-radio/stations/{station_id}/logo")
}

impl StationView {
    pub fn new(station: &Station, saved: bool) -> Self {
        Self {
            id: station.id.clone(),
            song_id: station.song_id(),
            source: station.source,
            name: station.name.clone(),
            stream_url: station.stream_url.clone(),
            homepage: station.homepage.clone(),
            logo_url: station
                .favicon
                .as_ref()
                .filter(|favicon| !favicon.trim().is_empty())
                .map(|_| logo_path(&station.id)),
            country: station.country.clone(),
            country_code: station.country_code.clone(),
            region: station.region.clone(),
            languages: station.languages.clone(),
            tags: station.tags.clone(),
            codec: station.codec.as_deref().and_then(normalize_codec),
            bitrate_kbps: station.bitrate_kbps,
            hls: station.hls,
            quality: classify(station.codec.as_deref(), station.bitrate_kbps),
            votes: station.votes,
            clicks: station.clicks,
            click_trend: station.click_trend,
            online: station.online,
            description: station.description.clone(),
            allow_lan: station.allow_lan,
            saved,
            verified_format: None,
        }
    }

    /// Applies what Parson heard: the badge then shows the real format.
    pub fn with_verified(mut self, format: Option<super::relay::SourceFormat>) -> Self {
        if let Some(format) = format.filter(|format| format.verified && format.codec.is_some()) {
            let mut quality = classify(
                format.codec.as_deref(),
                format.bitrate_kbps.or(self.bitrate_kbps),
            );
            quality.verified = true;
            self.codec = format.codec.clone();
            self.quality = quality;
            self.verified_format = Some(format);
        }
        self
    }
}

/// Cleans a free-text field: trimmed, bounded, `None` when empty.
pub fn clean(value: Option<&str>, limit: usize) -> Option<String> {
    let value = value?.trim();
    (!value.is_empty()).then(|| value.chars().take(limit).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_uuids_inside_radio_song_ids() {
        let id = "d1a54d2e-623e-4970-ab11-35f7b56c5ec3";
        assert!(valid_station_id(id));
        assert_eq!(station_id_from_song(&song_id(id)), Some(id));
        assert!(is_live_song_id(
            "radio:d1a54d2e-623e-4970-ab11-35f7b56c5ec3"
        ));
        for bad in [
            "radio:../etc",
            "radio:",
            "tidal:123",
            "d1a54d2e-623e-4970-ab11-35f7b56c5ec3",
            "radio:d1a54d2e_623e-4970-ab11-35f7b56c5ec3",
        ] {
            assert!(!is_live_song_id(bad), "{bad}");
        }
    }

    #[test]
    fn quality_is_lossless_only_for_lossless_codecs() {
        assert_eq!(
            classify(Some("FLAC"), Some(1000)).tier,
            QualityTier::Lossless
        );
        assert_eq!(classify(Some("flac"), None).label, "FLAC");
        assert_eq!(classify(Some("MP3"), Some(320)).tier, QualityTier::High);
        assert_eq!(classify(Some("MP3"), Some(128)).tier, QualityTier::Standard);
        assert_eq!(classify(Some("MP3"), Some(64)).tier, QualityTier::Low);
        assert_eq!(classify(Some("AAC+"), Some(64)).tier, QualityTier::Standard);
        assert_eq!(classify(Some("AAC"), Some(256)).label, "AAC 256");
        assert_eq!(classify(Some("OPUS"), Some(128)).tier, QualityTier::High);
        assert_eq!(classify(Some("MP3"), None).tier, QualityTier::Unknown);
        assert_eq!(
            classify(Some("UNKNOWN"), Some(128)).tier,
            QualityTier::Unknown
        );
        assert_eq!(classify(None, None).label, "Unknown format");
        assert_eq!(normalize_codec("AAC,H.264").as_deref(), Some("aac"));
        assert_eq!(normalize_codec("audio/aacp").as_deref(), Some("aac"));
        assert_eq!(normalize_codec("OGG").as_deref(), Some("vorbis"));
        assert!(!classify(Some("mp3"), Some(320)).lossless);
    }

    #[test]
    fn views_hide_the_remote_logo_behind_parson() {
        let station = Station {
            id: "d1a54d2e-623e-4970-ab11-35f7b56c5ec3".into(),
            source: StationSource::Directory,
            name: "Classic Vinyl HD".into(),
            stream_url: "https://icecast.example/classic".into(),
            homepage: None,
            favicon: Some("https://icecast.example/classic.jpg".into()),
            country: None,
            country_code: Some("US".into()),
            region: None,
            languages: vec!["english".into()],
            tags: vec!["jazz".into()],
            codec: Some("MP3".into()),
            bitrate_kbps: Some(320),
            hls: false,
            votes: Some(1),
            clicks: None,
            click_trend: None,
            online: Some(true),
            description: None,
            allow_lan: false,
        };
        let view = StationView::new(&station, true);
        assert_eq!(
            view.logo_url.as_deref(),
            Some("/api/v1/live-radio/stations/d1a54d2e-623e-4970-ab11-35f7b56c5ec3/logo")
        );
        let json = serde_json::to_string(&view).unwrap();
        assert!(!json.contains("classic.jpg"));
        assert_eq!(view.codec.as_deref(), Some("mp3"));
        assert_eq!(view.song_id, "radio:d1a54d2e-623e-4970-ab11-35f7b56c5ec3");
        assert!(view.saved);
    }
}
