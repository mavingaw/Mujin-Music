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
    Linked,
}

impl ZoneKind {
    /// Zones only group natively within one ecosystem.
    pub fn ecosystem(self) -> &'static str {
        match self {
            Self::Web | Self::Desktop | Self::Mobile => "parson",
            Self::Chromecast => "chromecast",
            Self::Sonos => "sonos",
            Self::Linked => "linked",
        }
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

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ZoneVolume {
    /// 0..=100.
    pub level: u8,
    pub muted: bool,
    pub min: u8,
    /// Safety limit set in zone settings (100 when unset).
    pub max: u8,
    pub step: u8,
}

impl ZoneVolume {
    pub fn new(level: u8, muted: bool, max: Option<u8>) -> Self {
        Self {
            level: level.min(100),
            muted,
            min: 0,
            max: max.unwrap_or(100).min(100),
            step: 2,
        }
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
    pub queue: bool,
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
}

/// A place music plays: a Parson app, a Chromecast session, a Sonos group or
/// a linked group of zones.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Zone {
    pub id: String,
    pub kind: ZoneKind,
    pub ecosystem: &'static str,
    pub name: String,
    /// Device description, e.g. "Sonos Play:5" or "Chrome on macOS".
    pub device: String,
    pub status: ZoneStatus,
    pub source: ZoneSource,
    pub now_playing: Option<ZoneNowPlaying>,
    pub queue: ZoneQueue,
    pub position_ms: i64,
    pub duration_ms: i64,
    pub volume: Option<ZoneVolume>,
    pub group: Option<ZoneGroupInfo>,
    /// The linked group this zone currently belongs to.
    pub grouped_into: Option<String>,
    pub capabilities: ZoneCapabilities,
    pub sleep_timer: Option<ZoneSleepTimer>,
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
    SetVolume {
        volume: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        member_id: Option<String>,
    },
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
            _ => Ok(()),
        }
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
    use super::{ZoneCommand, extrapolate_position};

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
                member_id: None
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
        })
        .expect("serialize");
        assert_eq!(
            volume,
            serde_json::json!({"command": "set_volume", "volume": 30})
        );
    }

    #[test]
    fn positions_extrapolate_only_while_playing_and_clamp_to_duration() {
        assert_eq!(extrapolate_position(1_000, 0, 500, true, 10_000), 1_500);
        assert_eq!(extrapolate_position(1_000, 0, 500, false, 10_000), 1_000);
        assert_eq!(extrapolate_position(9_000, 0, 5_000, true, 10_000), 10_000);
        assert_eq!(extrapolate_position(1_000, 900, 500, true, 0), 1_000);
    }
}
