//! Zone data model shared by the HTTP API and the zone service.

use serde::{Deserialize, Serialize};

/// Most songs a zone queue carries.
pub const MAX_ZONE_QUEUE_ITEMS: usize = 500;
/// Most tracks mapped onto a Sonos queue at once (a window around the
/// current position; the rest follows when the window is reloaded).
pub const MAX_SONOS_QUEUE_ITEMS: usize = 100;
/// Roon parity: a sleep timer that expires near the end of a track lets the
/// track finish when it has at most this much left.
pub const SLEEP_FINISH_TRACK_WINDOW_MS: i64 = 5 * 60 * 1_000;

pub const LINKED_SYNC_NOTE: &str = "Linked zones start together and follow the same controls, but they are not sample-synced: expect small timing differences between rooms.";
pub const SONOS_SYNC_NOTE: &str =
    "Native Sonos group: the rooms play sample-synced, coordinated by the first room.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoneKind {
    Web,
    Desktop,
    Mobile,
    Chromecast,
    Sonos,
    /// A UPnP/DLNA media renderer on the network.
    Upnp,
    /// An audio output (USB DAC, built-in, HDMI) of the machine running the
    /// Parson server, normally the desktop app's own computer.
    Device,
    /// An audio output of another machine running Parson Bridge.
    Bridge,
    /// An AirPlay receiver (RAOP).
    Airplay,
    Linked,
}

impl ZoneKind {
    /// Zones only group natively within one ecosystem.
    pub fn ecosystem(self) -> &'static str {
        match self {
            Self::Web | Self::Desktop | Self::Mobile => "parson",
            Self::Chromecast => "chromecast",
            Self::Sonos => "sonos",
            Self::Upnp => "upnp",
            Self::Device => "device",
            Self::Bridge => "bridge",
            Self::Airplay => "airplay",
            Self::Linked => "linked",
        }
    }

    /// Zones whose queue the server holds and plays track by track.
    pub fn is_hosted(self) -> bool {
        matches!(
            self,
            Self::Upnp | Self::Device | Self::Bridge | Self::Airplay
        )
    }

    pub fn is_client(self) -> bool {
        matches!(self, Self::Web | Self::Desktop | Self::Mobile)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoneStatus {
    Playing,
    Paused,
    Buffering,
    Stopped,
    Idle,
    Unavailable,
}

impl ZoneStatus {
    pub fn is_playing(self) -> bool {
        matches!(self, Self::Playing | Self::Buffering)
    }
}

/// What is playing on a zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoneSource {
    /// Nothing queued.
    None,
    /// A Parson queue.
    Parson,
    /// Something another app started (for example the Sonos app).
    External,
}

/// Repeat mode of a zone queue.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatMode {
    #[default]
    Off,
    /// Start the queue again after the last track.
    All,
    /// Repeat the current track.
    One,
}

impl RepeatMode {
    /// Sonos AVTransport `PlayMode` for this repeat mode. Parson shuffles
    /// its own queue (so Sonos and Parson agree on the order), so shuffle
    /// modes are never sent.
    pub fn sonos_play_mode(self) -> &'static str {
        match self {
            Self::Off => "NORMAL",
            Self::All => "REPEAT_ALL",
            Self::One => "REPEAT_ONE",
        }
    }

    /// Repeat mode and whether Sonos itself shuffles, from a `PlayMode`.
    pub fn from_sonos_play_mode(mode: &str) -> (Self, bool) {
        match mode {
            "REPEAT_ALL" => (Self::All, false),
            "REPEAT_ONE" => (Self::One, false),
            "SHUFFLE" => (Self::All, true),
            "SHUFFLE_NOREPEAT" => (Self::Off, true),
            "SHUFFLE_REPEAT_ONE" => (Self::One, true),
            _ => (Self::Off, false),
        }
    }
}

/// Per-zone volume limits (Roon's Volume Limits): a soft comfort limit the
/// slider stops at (the +/- buttons may go past it) and a hard safety range
/// nothing can leave.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VolumeLimits {
    pub comfort: Option<u8>,
    pub safety_min: Option<u8>,
    pub safety_max: Option<u8>,
}

impl VolumeLimits {
    pub fn min(self) -> u8 {
        self.safety_min.unwrap_or(0).min(100)
    }

    pub fn max(self) -> u8 {
        self.safety_max.unwrap_or(100).min(100).max(self.min())
    }

    /// Where an absolute (slider) change stops.
    pub fn slider_max(self) -> u8 {
        self.comfort
            .map(|comfort| comfort.clamp(self.min(), self.max()))
            .unwrap_or_else(|| self.max())
    }

    /// Clamps a requested level. `step` requests (the +/- buttons, relative
    /// changes) may pass the comfort limit; absolute ones stop at it unless
    /// `above_comfort` allows it. The safety range always applies.
    pub fn clamp(self, requested: i16, step: bool, above_comfort: bool) -> u8 {
        let ceiling = if step || above_comfort {
            self.max()
        } else {
            self.slider_max()
        };
        requested.clamp(i16::from(self.min()), i16::from(ceiling)) as u8
    }

    /// The level to move to when the current level is outside the safety
    /// range (after the limits were tightened).
    pub fn enforce(self, level: u8) -> Option<u8> {
        if level > self.max() {
            Some(self.max())
        } else if level < self.min() {
            Some(self.min())
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ZoneVolume {
    /// 0..=100.
    pub level: u8,
    pub muted: bool,
    /// Hard safety minimum (0 when unset).
    pub min: u8,
    /// Hard safety maximum (100 when unset).
    pub max: u8,
    /// Soft comfort limit: volume sliders stop here, +/- may go past it.
    pub comfort: Option<u8>,
    /// The range the device itself reports (read-only).
    pub device_min: u8,
    pub device_max: u8,
    pub step: u8,
    /// The device's own volume scale, when it reports one (a DAC's mixer in
    /// dB, a renderer's 0..30 steps).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_range: Option<DeviceVolumeRange>,
}

/// A volume scale as the device reports it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DeviceVolumeRange {
    pub min: f32,
    pub max: f32,
    pub step: f32,
    /// `dB` or `steps`.
    pub unit: &'static str,
    /// The device changes its own gain (a DAC mixer, a renderer's
    /// amplifier), so the samples Parson sends stay untouched.
    pub hardware: bool,
    /// The device's current value on this scale.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<f32>,
}

impl ZoneVolume {
    pub fn new(level: u8, muted: bool, limits: VolumeLimits) -> Self {
        Self {
            level: level.min(100),
            muted,
            min: limits.min(),
            max: limits.max(),
            comfort: limits.comfort.map(|_| limits.slider_max()),
            device_min: 0,
            device_max: 100,
            step: 2,
            device_range: None,
        }
    }
}

/// Roon's volume leveling modes. `Auto` levels by album while an album plays
/// in order and by track otherwise.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LevelingMode {
    #[default]
    Off,
    Auto,
    Track,
    Album,
}

impl LevelingMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Auto => "auto",
            Self::Track => "track",
            Self::Album => "album",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "off" => Some(Self::Off),
            "auto" => Some(Self::Auto),
            "track" => Some(Self::Track),
            "album" => Some(Self::Album),
            _ => None,
        }
    }
}

/// Icons a zone can wear (names shared with the web and mobile apps).
pub const ZONE_ICONS: &[&str] = &[
    "speaker",
    "headphones",
    "monitor",
    "laptop",
    "phone",
    "tablet",
    "tv",
    "radio",
    "turntable",
    "amplifier",
    "car",
    "bed",
    "sofa",
    "kitchen",
    "office",
    "garden",
    "music",
];

pub const CROSSFADE_MAX_SECONDS: u8 = 12;
pub const LEVELING_TARGET_RANGE: std::ops::RangeInclusive<i8> = -30..=-5;
pub const LEVELING_ADJUSTMENT_RANGE: std::ops::RangeInclusive<i8> = -12..=12;
/// ReplayGain 2.0 reference loudness.
pub const DEFAULT_LEVELING_TARGET_LUFS: i8 = -18;

/// Which settings a zone can take.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ZoneSettingsSupport {
    pub crossfade: bool,
    pub leveling: bool,
    pub volume_limits: bool,
    /// Only Parson apps can be private (they belong to one device).
    pub private: bool,
    /// Per-zone DSP (preamp, bass, treble, headphone crossfeed).
    pub dsp: bool,
    /// Exclusive (bit-perfect) access to an audio device.
    pub exclusive: bool,
    /// Volume modes this zone can use; empty when fixed by the device.
    pub volume_modes: Vec<VolumeMode>,
    /// A maximum sample rate and bit depth can be set (device setup).
    pub max_format: bool,
}

/// How a server-driven zone changes volume.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumeMode {
    /// The device's own volume control (a DAC mixer or the renderer's
    /// amplifier): samples stay bit-perfect.
    #[default]
    Hardware,
    /// Parson scales the samples: works everywhere, not bit-perfect.
    Software,
    /// No volume control: bit-perfect, set the volume on the amplifier.
    Fixed,
}

impl VolumeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hardware => "hardware",
            Self::Software => "software",
            Self::Fixed => "fixed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "hardware" => Some(Self::Hardware),
            "software" => Some(Self::Software),
            "fixed" => Some(Self::Fixed),
            _ => None,
        }
    }
}

/// Sample rates a maximum can be set to.
pub const SAMPLE_RATE_CHOICES: &[u32] = &[
    44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000,
];
pub const BIT_DEPTH_CHOICES: &[u8] = &[16, 24, 32];
pub const DSP_GAIN_RANGE: std::ops::RangeInclusive<i8> = -12..=12;
pub const DSP_PREAMP_RANGE: std::ops::RangeInclusive<i8> = -24..=0;

/// Per-zone DSP, applied server-side for zones the server plays (devices,
/// UPnP renderers) and by the web and desktop apps for their own zone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ZoneDsp {
    pub enabled: bool,
    /// Extra attenuation in dB (0 or less).
    pub preamp_db: i8,
    /// Low shelf at 100 Hz, in dB.
    pub bass_db: i8,
    /// High shelf at 10 kHz, in dB.
    pub treble_db: i8,
    /// Headphone crossfeed (Bauer-style) for less extreme stereo.
    pub crossfeed: bool,
}

impl ZoneDsp {
    /// Whether any processing happens.
    pub fn is_active(&self) -> bool {
        self.enabled
            && (self.preamp_db != 0 || self.bass_db != 0 || self.treble_db != 0 || self.crossfeed)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if !DSP_PREAMP_RANGE.contains(&self.preamp_db) {
            return Err("invalid_dsp_preamp");
        }
        if !DSP_GAIN_RANGE.contains(&self.bass_db) || !DSP_GAIN_RANGE.contains(&self.treble_db) {
            return Err("invalid_dsp_gain");
        }
        Ok(())
    }

    /// Headroom management: the level drops by the largest boost so EQ
    /// never clips, unless the preamp already lowers it further.
    pub fn effective_preamp_db(&self) -> i8 {
        let boost = self.bass_db.max(self.treble_db).max(0);
        self.preamp_db.min(-boost)
    }

    /// The ffmpeg filter chain, or `None` when nothing is processed.
    pub fn ffmpeg_filter(&self) -> Option<String> {
        if !self.is_active() {
            return None;
        }
        let mut parts = Vec::new();
        let preamp = self.effective_preamp_db();
        if preamp != 0 {
            parts.push(format!("volume={preamp}dB"));
        }
        if self.bass_db != 0 {
            parts.push(format!("bass=g={}:f=100:t=q:w=0.707", self.bass_db));
        }
        if self.treble_db != 0 {
            parts.push(format!("treble=g={}:f=10000:t=q:w=0.707", self.treble_db));
        }
        if self.crossfeed {
            parts.push("crossfeed=strength=0.3:range=0.5".to_string());
        }
        Some(parts.join(","))
    }

    /// Short description for signal paths, e.g. `Bass +3 dB, Crossfeed`.
    pub fn summary(&self) -> String {
        let signed = |value: i8| {
            if value > 0 {
                format!("+{value}")
            } else {
                format!("\u{2212}{}", value.unsigned_abs())
            }
        };
        let mut parts = Vec::new();
        let preamp = self.effective_preamp_db();
        if preamp != 0 {
            parts.push(format!("Preamp {} dB", signed(preamp)));
        }
        if self.bass_db != 0 {
            parts.push(format!("Bass {} dB", signed(self.bass_db)));
        }
        if self.treble_db != 0 {
            parts.push(format!("Treble {} dB", signed(self.treble_db)));
        }
        if self.crossfeed {
            parts.push("Crossfeed".to_string());
        }
        parts.join(", ")
    }
}

/// What a zone can play natively, and what happens to everything else.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ZoneOutputInfo {
    pub max_sample_rate: u32,
    pub max_bit_depth: u8,
    /// Formats played as-is, e.g. `FLAC`, `MP3`.
    pub formats: Vec<&'static str>,
    /// "Plays up to X kHz/Y-bit, converts the rest" in words.
    pub note: String,
    /// Device zones: exclusive (bit-perfect) access.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclusive: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_mode: Option<VolumeMode>,
    /// The track playing now, from file to device.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<ZoneStreamInfo>,
}

/// The signal path of the playing track.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ZoneStreamInfo {
    /// e.g. `FLAC 96 kHz/24-bit`.
    pub source: String,
    /// e.g. `96 kHz/24-bit to Topping E30 (exclusive)`.
    pub output: String,
    /// Nothing between the file and the device changed a sample.
    pub bit_perfect: bool,
    /// Processing on the way, in the order it happens, e.g.
    /// `Sample rate conversion 96 kHz to 48 kHz`, `Volume leveling -6.4 dB`.
    pub steps: Vec<String>,
    /// Roon-style rating of the whole path.
    pub quality: StreamQuality,
}

/// How much of the original reaches the output, worst stage first: a lossy
/// file anywhere makes the path `Low`; a conversion the output needs (rate,
/// bit depth, software volume) makes it `High`; processing the listener
/// asked for (leveling, DSP) makes it `Enhanced`; otherwise `Lossless`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamQuality {
    Lossless,
    Enhanced,
    High,
    Low,
}

impl StreamQuality {
    pub fn rate(lossless_source: bool, converted: bool, enhanced: bool) -> Self {
        if !lossless_source {
            Self::Low
        } else if converted {
            Self::High
        } else if enhanced {
            Self::Enhanced
        } else {
            Self::Lossless
        }
    }
}

/// A zone's settings as clients show them.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ZoneSettingsView {
    /// The name the device or group gives itself.
    pub default_name: String,
    /// Whether `Zone::name` is a custom name.
    pub custom_name: bool,
    /// Custom icon; `None` uses the device's icon.
    pub icon: Option<String>,
    pub crossfade_seconds: u8,
    pub leveling: LevelingMode,
    pub leveling_target_lufs: i8,
    pub leveling_adjustment_db: i8,
    pub private: bool,
    pub dsp: ZoneDsp,
    pub exclusive: bool,
    pub volume_mode: VolumeMode,
    pub max_sample_rate: Option<u32>,
    pub max_bit_depth: Option<u8>,
    pub supports: ZoneSettingsSupport,
    /// How the settings apply on this kind of zone.
    pub notes: Vec<String>,
}

impl ZoneSettingsView {
    pub fn unsupported(default_name: &str) -> Self {
        Self {
            default_name: default_name.to_string(),
            custom_name: false,
            icon: None,
            crossfade_seconds: 0,
            leveling: LevelingMode::Off,
            leveling_target_lufs: DEFAULT_LEVELING_TARGET_LUFS,
            leveling_adjustment_db: 0,
            private: false,
            dsp: ZoneDsp::default(),
            exclusive: false,
            volume_mode: VolumeMode::Hardware,
            max_sample_rate: None,
            max_bit_depth: None,
            supports: ZoneSettingsSupport::default(),
            notes: Vec::new(),
        }
    }
}

/// The icon a zone of `kind` shows by default.
pub fn default_zone_icon(kind: ZoneKind) -> &'static str {
    match kind {
        ZoneKind::Web => "monitor",
        ZoneKind::Desktop => "laptop",
        ZoneKind::Mobile => "phone",
        ZoneKind::Chromecast => "tv",
        ZoneKind::Sonos => "speaker",
        ZoneKind::Upnp => "radio",
        ZoneKind::Device | ZoneKind::Bridge => "amplifier",
        ZoneKind::Airplay => "speaker",
        ZoneKind::Linked => "music",
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ZoneGroupMember {
    pub id: String,
    pub name: String,
    pub kind: ZoneKind,
    pub volume: Option<ZoneVolume>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ZoneGroupInfo {
    pub id: String,
    /// True only for ecosystem-native groups (Sonos) that play sample-synced.
    pub synced: bool,
    pub sync_note: &'static str,
    pub members: Vec<ZoneGroupMember>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ZoneCapabilities {
    pub transport: bool,
    pub seek: bool,
    pub volume: bool,
    pub mute: bool,
    /// Lists the queue.
    pub queue: bool,
    /// Reorders, removes and clears upcoming queue items.
    pub queue_edit: bool,
    pub repeat: bool,
    pub shuffle: bool,
    pub group: bool,
    /// Can receive a transferred queue.
    pub transfer_target: bool,
    pub sleep_timer: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ZoneNowPlaying {
    pub song_id: Option<String>,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_id: Option<String>,
    pub cover_url: Option<String>,
    pub duration_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ZoneQueue {
    pub song_ids: Vec<String>,
    pub current_position: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ZoneSleepTimer {
    pub ends_at_ms: i64,
    /// The timer ran out mid-track and is letting the track finish.
    pub finishing_track: bool,
    /// The timer ran out and the zone is fading out before it pauses.
    pub fading: bool,
}

/// A place music plays: a Parson app, a Chromecast session, a Sonos group or
/// a linked group of zones.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Zone {
    pub id: String,
    pub kind: ZoneKind,
    pub ecosystem: &'static str,
    pub name: String,
    /// Icon name from [`ZONE_ICONS`]: the custom icon or the kind's default.
    pub icon: String,
    /// Device description, e.g. "Sonos Play:5" or "Chrome on macOS".
    pub device: String,
    pub status: ZoneStatus,
    pub source: ZoneSource,
    pub now_playing: Option<ZoneNowPlaying>,
    pub queue: ZoneQueue,
    pub repeat: RepeatMode,
    pub shuffle: bool,
    pub position_ms: i64,
    pub duration_ms: i64,
    pub volume: Option<ZoneVolume>,
    pub group: Option<ZoneGroupInfo>,
    /// The linked group this zone currently belongs to.
    pub grouped_into: Option<String>,
    pub capabilities: ZoneCapabilities,
    pub sleep_timer: Option<ZoneSleepTimer>,
    pub settings: ZoneSettingsView,
    /// Format limits and the signal path of the playing track.
    pub output: Option<ZoneOutputInfo>,
    /// Format limits or other facts worth showing, e.g. Sonos conversions.
    pub notes: Vec<String>,
}

/// A remote-control command for a zone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum ZoneCommand {
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    Previous,
    Seek {
        position_ms: i64,
    },
    Jump {
        queue_position: usize,
    },
    /// An absolute (slider) change: stops at the comfort limit.
    SetVolume {
        volume: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        member_id: Option<String>,
        /// Server-internal: a step or a restore that may pass the comfort
        /// limit (never accepted from or sent to clients).
        #[serde(skip)]
        above_comfort: bool,
    },
    /// A +/- step: may pass the comfort limit, never the safety limits.
    AdjustVolume {
        delta: i8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        member_id: Option<String>,
    },
    SetMute {
        muted: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        member_id: Option<String>,
    },
    Load {
        song_ids: Vec<String>,
        #[serde(default)]
        current_position: usize,
        #[serde(default)]
        position_ms: i64,
        #[serde(default = "default_true")]
        play: bool,
    },
    /// Moves an upcoming item. `song_id`, when given, must be the song at
    /// `from`, so an edit made against a stale queue is refused.
    MoveQueueItem {
        from: usize,
        to: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        song_id: Option<String>,
    },
    /// Removes one upcoming item.
    RemoveQueueItem {
        queue_position: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        song_id: Option<String>,
    },
    /// Removes everything after the current track.
    ClearUpcoming,
    /// Adds songs right after the current track (`next`, "Play next") or at
    /// the end ("Add to queue"). On a zone with no queue it starts them.
    AddToQueue {
        song_ids: Vec<String>,
        #[serde(default)]
        next: bool,
    },
    SetRepeat {
        mode: RepeatMode,
    },
    /// Shuffles the upcoming items; turning it off restores their order.
    SetShuffle {
        enabled: bool,
    },
}

fn default_true() -> bool {
    true
}

impl ZoneCommand {
    /// Rejects out-of-range arguments before anything reaches a device.
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Seek { position_ms } if *position_ms < 0 => Err("invalid_seek_position"),
            Self::SetVolume { volume, .. } if *volume > 100 => Err("invalid_volume"),
            Self::Load {
                song_ids,
                current_position,
                position_ms,
                ..
            } => {
                if song_ids.is_empty() || song_ids.len() > MAX_ZONE_QUEUE_ITEMS {
                    Err("invalid_queue_length")
                } else if *current_position >= song_ids.len() {
                    Err("invalid_queue_position")
                } else if *position_ms < 0 {
                    Err("invalid_seek_position")
                } else if song_ids.iter().any(|id| id.is_empty() || id.len() > 256) {
                    Err("invalid_song_id")
                } else {
                    Ok(())
                }
            }
            Self::AddToQueue { song_ids, .. } => {
                if song_ids.is_empty() || song_ids.len() > MAX_ZONE_QUEUE_ITEMS {
                    Err("invalid_queue_length")
                } else if song_ids.iter().any(|id| id.is_empty() || id.len() > 256) {
                    Err("invalid_song_id")
                } else {
                    Ok(())
                }
            }
            Self::MoveQueueItem { from, to, song_id } => {
                if from == to {
                    Err("invalid_queue_move")
                } else if *from >= MAX_ZONE_QUEUE_ITEMS || *to >= MAX_ZONE_QUEUE_ITEMS {
                    Err("invalid_queue_position")
                } else if song_id
                    .as_ref()
                    .is_some_and(|id| id.is_empty() || id.len() > 256)
                {
                    Err("invalid_song_id")
                } else {
                    Ok(())
                }
            }
            Self::RemoveQueueItem {
                queue_position,
                song_id,
            } => {
                if *queue_position >= MAX_ZONE_QUEUE_ITEMS {
                    Err("invalid_queue_position")
                } else if song_id
                    .as_ref()
                    .is_some_and(|id| id.is_empty() || id.len() > 256)
                {
                    Err("invalid_song_id")
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        }
    }

    /// Commands that change the order or content of the queue.
    pub fn is_queue_edit(&self) -> bool {
        matches!(
            self,
            Self::MoveQueueItem { .. }
                | Self::RemoveQueueItem { .. }
                | Self::ClearUpcoming
                | Self::AddToQueue { .. }
                | Self::SetShuffle { .. }
        )
    }

    pub fn member_id(&self) -> Option<&str> {
        match self {
            Self::SetVolume { member_id, .. }
            | Self::AdjustVolume { member_id, .. }
            | Self::SetMute { member_id, .. } => member_id.as_deref(),
            _ => None,
        }
    }
}

/// What a transfer carries from one zone to another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaybackSnapshot {
    pub song_ids: Vec<String>,
    pub current_position: usize,
    pub position_ms: i64,
    pub playing: bool,
}

/// Errors surfaced to API callers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ZoneError {
    NotFound,
    Invalid(&'static str),
    Unsupported(&'static str),
    Device(String),
}

impl std::fmt::Display for ZoneError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => formatter.write_str("zone_not_found"),
            Self::Invalid(code) | Self::Unsupported(code) => formatter.write_str(code),
            Self::Device(message) => write!(formatter, "device error: {message}"),
        }
    }
}

impl std::error::Error for ZoneError {}

/// A queue edit applied to `song_ids`, as the Sonos driver needs to mirror
/// it: which absolute positions changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueEdit {
    Moved {
        from: usize,
        to: usize,
    },
    Removed {
        position: usize,
    },
    /// `count` songs were inserted at `position` (play next, add to queue).
    Inserted {
        position: usize,
        count: usize,
    },
    /// Everything after `current` changed (clear, shuffle, unshuffle).
    UpcomingReplaced,
    Unchanged,
}

/// Applies a queue-editing command to `song_ids` with the current track at
/// `current`. Only upcoming items (after `current`) can move or be removed,
/// as in Roon: the playing track and history stay put. `original` keeps the
/// unshuffled order while shuffle is on.
pub fn apply_queue_edit(
    song_ids: &mut Vec<String>,
    current: usize,
    shuffle: &mut bool,
    original: &mut Option<Vec<String>>,
    command: &ZoneCommand,
    random: &mut dyn FnMut(usize) -> usize,
) -> Result<QueueEdit, ZoneError> {
    let guard = |position: usize, song_id: &Option<String>| -> Result<(), ZoneError> {
        match (song_ids.get(position), song_id) {
            (None, _) => Err(ZoneError::Invalid("invalid_queue_position")),
            (Some(found), Some(expected)) if found != expected => {
                Err(ZoneError::Invalid("queue_changed"))
            }
            _ => Ok(()),
        }
    };
    match command {
        ZoneCommand::MoveQueueItem { from, to, song_id } => {
            guard(*from, song_id)?;
            if *to >= song_ids.len() {
                return Err(ZoneError::Invalid("invalid_queue_position"));
            }
            if *from <= current || *to <= current {
                return Err(ZoneError::Invalid("only_upcoming_items_move"));
            }
            let item = song_ids.remove(*from);
            song_ids.insert(*to, item);
            Ok(QueueEdit::Moved {
                from: *from,
                to: *to,
            })
        }
        ZoneCommand::RemoveQueueItem {
            queue_position,
            song_id,
        } => {
            guard(*queue_position, song_id)?;
            if *queue_position <= current {
                return Err(ZoneError::Invalid("only_upcoming_items_remove"));
            }
            let removed = song_ids.remove(*queue_position);
            forget_one(original, &removed, song_ids);
            Ok(QueueEdit::Removed {
                position: *queue_position,
            })
        }
        ZoneCommand::AddToQueue {
            song_ids: added,
            next,
        } => {
            if song_ids.len() + added.len() > MAX_ZONE_QUEUE_ITEMS {
                return Err(ZoneError::Invalid("queue_full"));
            }
            let position = if *next && !song_ids.is_empty() {
                (current + 1).min(song_ids.len())
            } else {
                song_ids.len()
            };
            song_ids.splice(position..position, added.iter().cloned());
            if let Some(order) = original {
                // Turning shuffle off keeps them where they were added.
                let at = if *next {
                    song_ids
                        .get(current)
                        .and_then(|playing| order.iter().position(|id| id == playing))
                        .map_or(order.len(), |index| index + 1)
                } else {
                    order.len()
                };
                order.splice(at..at, added.iter().cloned());
            }
            Ok(QueueEdit::Inserted {
                position,
                count: added.len(),
            })
        }
        ZoneCommand::ClearUpcoming => {
            if current + 1 >= song_ids.len() {
                return Ok(QueueEdit::Unchanged);
            }
            song_ids.truncate(current + 1);
            if let Some(original) = original {
                // Keep only what is still queued, in original order.
                let mut kept = song_ids.clone();
                original.retain(|id| match kept.iter().position(|k| k == id) {
                    Some(index) => {
                        kept.remove(index);
                        true
                    }
                    None => false,
                });
            }
            Ok(QueueEdit::UpcomingReplaced)
        }
        ZoneCommand::SetShuffle { enabled } => {
            if *enabled == *shuffle {
                return Ok(QueueEdit::Unchanged);
            }
            *shuffle = *enabled;
            let start = (current + 1).min(song_ids.len());
            if *enabled {
                *original = Some(song_ids.clone());
                let upcoming = &mut song_ids[start..];
                for index in (1..upcoming.len()).rev() {
                    let target = random(index + 1).min(index);
                    upcoming.swap(index, target);
                }
            } else if let Some(order) = original.take() {
                // Upcoming items go back to their original relative order.
                let rank = |id: &String, used: &mut [bool]| -> usize {
                    order
                        .iter()
                        .enumerate()
                        .position(|(index, candidate)| candidate == id && !used[index])
                        .inspect(|&index| used[index] = true)
                        .unwrap_or(usize::MAX)
                };
                let mut used = vec![false; order.len()];
                // History and the current track claim their original slots first.
                for id in &song_ids[..start] {
                    rank(id, &mut used);
                }
                let mut upcoming: Vec<(usize, usize, String)> = song_ids[start..]
                    .iter()
                    .enumerate()
                    .map(|(index, id)| (rank(id, &mut used), index, id.clone()))
                    .collect();
                upcoming.sort();
                song_ids.truncate(start);
                song_ids.extend(upcoming.into_iter().map(|(_, _, id)| id));
            }
            Ok(QueueEdit::UpcomingReplaced)
        }
        _ => Ok(QueueEdit::Unchanged),
    }
}

/// Drops one occurrence of `removed` from the unshuffled order, so turning
/// shuffle off does not bring a removed song back.
fn forget_one(original: &mut Option<Vec<String>>, removed: &str, remaining: &[String]) {
    let Some(order) = original else {
        return;
    };
    let still_queued = remaining.iter().filter(|id| *id == removed).count();
    let in_order = order.iter().filter(|id| *id == removed).count();
    if in_order > still_queued
        && let Some(index) = order.iter().rposition(|id| id == removed)
    {
        order.remove(index);
    }
}

/// Extrapolates a reported position to `now_ms` while playing.
pub fn extrapolate_position(
    position_ms: i64,
    reported_at_ms: i64,
    now_ms: i64,
    playing: bool,
    duration_ms: i64,
) -> i64 {
    let position = if playing {
        position_ms + (now_ms - reported_at_ms).max(0)
    } else {
        position_ms
    };
    if duration_ms > 0 {
        position.clamp(0, duration_ms)
    } else {
        position.max(0)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_ZONE_QUEUE_ITEMS, QueueEdit, RepeatMode, VolumeLimits, ZoneCommand, ZoneError,
        ZoneVolume, apply_queue_edit, extrapolate_position,
    };

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn play_next_goes_after_the_current_track_and_add_to_queue_at_the_end() {
        let mut queue = ids(&["a", "b", "c"]);
        let mut shuffle = false;
        let mut original = None;
        let mut random = |_: usize| 0;
        let add = |song_ids: &[&str], next: bool| ZoneCommand::AddToQueue {
            song_ids: ids(song_ids),
            next,
        };
        assert_eq!(
            apply_queue_edit(
                &mut queue,
                1,
                &mut shuffle,
                &mut original,
                &add(&["x", "y"], true),
                &mut random
            ),
            Ok(QueueEdit::Inserted {
                position: 2,
                count: 2
            })
        );
        assert_eq!(queue, ids(&["a", "b", "x", "y", "c"]));
        assert_eq!(
            apply_queue_edit(
                &mut queue,
                1,
                &mut shuffle,
                &mut original,
                &add(&["z"], false),
                &mut random
            ),
            Ok(QueueEdit::Inserted {
                position: 5,
                count: 1
            })
        );
        assert_eq!(queue, ids(&["a", "b", "x", "y", "c", "z"]));
        // While shuffled, the unshuffled order gets them in the same place.
        let mut queue = ids(&["a", "c", "b"]);
        let mut shuffle = true;
        let mut original = Some(ids(&["a", "b", "c"]));
        apply_queue_edit(
            &mut queue,
            0,
            &mut shuffle,
            &mut original,
            &add(&["n"], true),
            &mut random,
        )
        .expect("play next while shuffled");
        assert_eq!(queue, ids(&["a", "n", "c", "b"]));
        assert_eq!(original, Some(ids(&["a", "n", "b", "c"])));
        // Never past the queue limit.
        let mut full = vec!["s".to_string(); MAX_ZONE_QUEUE_ITEMS];
        assert_eq!(
            apply_queue_edit(
                &mut full,
                0,
                &mut false,
                &mut None,
                &add(&["one-too-many"], false),
                &mut random
            ),
            Err(ZoneError::Invalid("queue_full"))
        );
        assert_eq!(
            add(&[], true).validate(),
            Err("invalid_queue_length"),
            "nothing to add"
        );
        let parsed: ZoneCommand =
            serde_json::from_str(r#"{"command":"add_to_queue","song_ids":["a"]}"#).expect("json");
        assert_eq!(parsed, add(&["a"], false), "add to the end by default");
    }

    #[test]
    fn queue_edits_touch_only_upcoming_items_and_guard_stale_queues() {
        let mut queue = ids(&["a", "b", "c", "d", "e"]);
        let mut shuffle = false;
        let mut original = None;
        let mut random = |_: usize| 0;
        let mut edit = |queue: &mut Vec<String>, command: ZoneCommand| {
            apply_queue_edit(queue, 1, &mut shuffle, &mut original, &command, &mut random)
        };
        assert_eq!(
            edit(
                &mut queue,
                ZoneCommand::MoveQueueItem {
                    from: 4,
                    to: 2,
                    song_id: Some("e".into())
                }
            ),
            Ok(QueueEdit::Moved { from: 4, to: 2 })
        );
        assert_eq!(queue, ids(&["a", "b", "e", "c", "d"]));
        assert_eq!(
            edit(
                &mut queue,
                ZoneCommand::MoveQueueItem {
                    from: 1,
                    to: 3,
                    song_id: None
                }
            ),
            Err(ZoneError::Invalid("only_upcoming_items_move"))
        );
        assert_eq!(
            edit(
                &mut queue,
                ZoneCommand::RemoveQueueItem {
                    queue_position: 3,
                    song_id: Some("d".into())
                }
            ),
            Err(ZoneError::Invalid("queue_changed")),
            "a remote acting on an old queue must not remove the wrong song"
        );
        assert_eq!(
            edit(
                &mut queue,
                ZoneCommand::RemoveQueueItem {
                    queue_position: 3,
                    song_id: Some("c".into())
                }
            ),
            Ok(QueueEdit::Removed { position: 3 })
        );
        assert_eq!(queue, ids(&["a", "b", "e", "d"]));
        assert_eq!(
            edit(
                &mut queue,
                ZoneCommand::RemoveQueueItem {
                    queue_position: 1,
                    song_id: None
                }
            ),
            Err(ZoneError::Invalid("only_upcoming_items_remove"))
        );
        assert_eq!(
            edit(&mut queue, ZoneCommand::ClearUpcoming),
            Ok(QueueEdit::UpcomingReplaced)
        );
        assert_eq!(queue, ids(&["a", "b"]));
        assert_eq!(
            edit(&mut queue, ZoneCommand::ClearUpcoming),
            Ok(QueueEdit::Unchanged)
        );
    }

    #[test]
    fn shuffle_keeps_history_and_current_and_unshuffle_restores_order() {
        let mut queue = ids(&["a", "b", "c", "d", "e", "f"]);
        let mut shuffle = false;
        let mut original = None;
        // Deterministic "random": always swap with the first upcoming item.
        let mut random = |_: usize| 0;
        apply_queue_edit(
            &mut queue,
            1,
            &mut shuffle,
            &mut original,
            &ZoneCommand::SetShuffle { enabled: true },
            &mut random,
        )
        .expect("shuffle");
        assert!(shuffle);
        assert_eq!(&queue[..2], &ids(&["a", "b"])[..]);
        assert_ne!(queue, ids(&["a", "b", "c", "d", "e", "f"]));
        let mut sorted = queue.clone();
        sorted.sort();
        assert_eq!(sorted, ids(&["a", "b", "c", "d", "e", "f"]));
        // Remove one shuffled item, then unshuffle: it stays gone.
        let removed = queue[3].clone();
        apply_queue_edit(
            &mut queue,
            1,
            &mut shuffle,
            &mut original,
            &ZoneCommand::RemoveQueueItem {
                queue_position: 3,
                song_id: Some(removed.clone()),
            },
            &mut random,
        )
        .expect("remove");
        apply_queue_edit(
            &mut queue,
            1,
            &mut shuffle,
            &mut original,
            &ZoneCommand::SetShuffle { enabled: false },
            &mut random,
        )
        .expect("unshuffle");
        assert!(!shuffle);
        assert!(original.is_none());
        let expected: Vec<String> = ids(&["a", "b", "c", "d", "e", "f"])
            .into_iter()
            .filter(|id| *id != removed)
            .collect();
        assert_eq!(queue, expected);
    }

    #[test]
    fn sonos_play_modes_map_to_repeat_and_shuffle() {
        assert_eq!(RepeatMode::All.sonos_play_mode(), "REPEAT_ALL");
        assert_eq!(
            RepeatMode::from_sonos_play_mode("SHUFFLE_REPEAT_ONE"),
            (RepeatMode::One, true)
        );
        assert_eq!(
            RepeatMode::from_sonos_play_mode("NORMAL"),
            (RepeatMode::Off, false)
        );
        let parsed: ZoneCommand =
            serde_json::from_value(serde_json::json!({"command": "set_repeat", "mode": "one"}))
                .expect("set_repeat");
        assert_eq!(
            parsed,
            ZoneCommand::SetRepeat {
                mode: RepeatMode::One
            }
        );
        assert_eq!(
            ZoneCommand::MoveQueueItem {
                from: 3,
                to: 3,
                song_id: None
            }
            .validate(),
            Err("invalid_queue_move")
        );
    }

    #[test]
    fn commands_parse_from_tagged_json_and_validate() {
        let load: ZoneCommand = serde_json::from_value(serde_json::json!({
            "command": "load",
            "song_ids": ["a", "b"],
            "current_position": 1,
            "position_ms": 42_000
        }))
        .expect("load");
        assert_eq!(
            load,
            ZoneCommand::Load {
                song_ids: vec!["a".into(), "b".into()],
                current_position: 1,
                position_ms: 42_000,
                play: true
            }
        );
        assert!(load.validate().is_ok());
        let bad_position = ZoneCommand::Load {
            song_ids: vec!["a".into()],
            current_position: 1,
            position_ms: 0,
            play: true,
        };
        assert_eq!(bad_position.validate(), Err("invalid_queue_position"));
        assert_eq!(
            ZoneCommand::SetVolume {
                volume: 101,
                member_id: None,
                above_comfort: false,
            }
            .validate(),
            Err("invalid_volume")
        );
        assert!(
            serde_json::from_value::<ZoneCommand>(serde_json::json!({"command": "explode"}))
                .is_err()
        );
        let volume = serde_json::to_value(ZoneCommand::SetVolume {
            volume: 30,
            member_id: None,
            above_comfort: true,
        })
        .expect("serialize");
        assert_eq!(
            volume,
            serde_json::json!({"command": "set_volume", "volume": 30}),
            "the server-internal comfort override never reaches clients"
        );
        // ...and cannot be set by them either.
        let parsed: ZoneCommand = serde_json::from_value(serde_json::json!({
            "command": "set_volume",
            "volume": 30,
            "above_comfort": true
        }))
        .expect("parse");
        assert_eq!(
            parsed,
            ZoneCommand::SetVolume {
                volume: 30,
                member_id: None,
                above_comfort: false,
            }
        );
    }

    #[test]
    fn comfort_limit_stops_sliders_but_not_steps_and_safety_always_holds() {
        let limits = VolumeLimits {
            comfort: Some(40),
            safety_min: Some(10),
            safety_max: Some(70),
        };
        // The slider stops at the comfort limit...
        assert_eq!(limits.clamp(90, false, false), 40);
        // ...the +/- buttons may pass it, up to the safety maximum.
        assert_eq!(limits.clamp(55, true, false), 55);
        assert_eq!(limits.clamp(90, true, false), 70);
        // The safety minimum holds both ways.
        assert_eq!(limits.clamp(2, false, false), 10);
        assert_eq!(limits.clamp(-5, true, false), 10);
        // Tightened limits pull an out-of-range level back in.
        assert_eq!(limits.enforce(85), Some(70));
        assert_eq!(limits.enforce(5), Some(10));
        assert_eq!(limits.enforce(50), None);
        let volume = ZoneVolume::new(50, false, limits);
        assert_eq!(
            (volume.min, volume.max, volume.comfort, volume.device_max),
            (10, 70, Some(40), 100)
        );
        // No limits: the whole device range, no comfort stop.
        let open = VolumeLimits::default();
        assert_eq!(open.clamp(100, false, false), 100);
        assert_eq!(ZoneVolume::new(5, false, open).comfort, None);
    }

    #[test]
    fn positions_extrapolate_only_while_playing_and_clamp_to_duration() {
        assert_eq!(extrapolate_position(1_000, 0, 500, true, 10_000), 1_500);
        assert_eq!(extrapolate_position(1_000, 0, 500, false, 10_000), 1_000);
        assert_eq!(extrapolate_position(9_000, 0, 5_000, true, 10_000), 10_000);
        assert_eq!(extrapolate_position(1_000, 900, 500, true, 0), 1_000);
    }
}
