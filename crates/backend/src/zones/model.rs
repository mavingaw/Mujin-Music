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
        QueueEdit, RepeatMode, ZoneCommand, ZoneError, apply_queue_edit, extrapolate_position,
    };

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
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
