//! The zone registry and its drivers.
//!
//! Every running Parson client (web, desktop, mobile) registers itself as a
//! zone and receives remote commands over the zone WebSocket. Sonos groups
//! come from the [`SonosController`], and the server drives them directly:
//! it maps the Parson queue onto the Sonos queue with signed media URLs and
//! follows progress by polling plus UPnP events. Linked groups fan commands
//! out to zones of any ecosystem and are clearly labelled as not
//! sample-synced.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use super::model::*;
use crate::library::state::LibraryCache;
use crate::persistence::connection::DbPool;
use crate::sonos::{SonosController, SonosGroup, Transport, TransportState};

const CLIENT_ZONE_TTL: Duration = Duration::from_secs(45);
const MAX_CLIENT_ZONES_PER_USER: usize = 32;
const MAX_PENDING_COMMANDS: usize = 32;
const MAX_LINKED_GROUPS_PER_USER: usize = 16;
const MEDIA_URL_LIFETIME_SECONDS: i64 = 12 * 60 * 60;
/// Path prefix of signed Sonos media URLs.
pub const ZONE_MEDIA_PATH: &str = "/api/v1/zones/media/";

/// Pushed to zone WebSocket subscribers.
#[derive(Clone, Debug)]
pub enum ZoneEvent {
    /// Zone list changed. `None` means every user (shared Sonos rooms).
    Changed { user_id: Option<i32> },
    /// A command for a client zone.
    Command {
        user_id: i32,
        zone_id: String,
        seq: u64,
        command: ZoneCommand,
    },
}

/// What a client reports about itself.
#[derive(Clone, Debug, Deserialize)]
pub struct ClientRegistration {
    pub kind: ZoneKind,
    pub name: String,
    #[serde(default)]
    pub device: String,
    pub status: ZoneStatus,
    /// The client's queue; omitted when unchanged since the last report.
    #[serde(default)]
    pub song_ids: Option<Vec<String>>,
    #[serde(default)]
    pub current_position: usize,
    #[serde(default)]
    pub position_ms: i64,
    #[serde(default)]
    pub duration_ms: i64,
    /// 0..=100
    pub volume: u8,
    #[serde(default)]
    pub muted: bool,
    /// Highest command sequence number already applied.
    #[serde(default)]
    pub acknowledged_seq: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct PendingCommand {
    pub seq: u64,
    pub command: ZoneCommand,
}

#[derive(Clone, Debug, Serialize)]
pub struct RegistrationResult {
    pub zone_id: String,
    pub commands: Vec<PendingCommand>,
    /// Ask the client to resend its full queue next time.
    pub needs_queue: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct ZoneSettings {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub max_volume: Option<u8>,
}

struct ClientZone {
    user_id: i32,
    kind: ZoneKind,
    name: String,
    device: String,
    status: ZoneStatus,
    song_ids: Vec<String>,
    has_queue: bool,
    current_position: usize,
    position_ms: i64,
    duration_ms: i64,
    reported_at_ms: i64,
    volume: u8,
    muted: bool,
    last_seen: Instant,
    commands: VecDeque<PendingCommand>,
    next_seq: u64,
}

struct SonosQueue {
    /// Coordinator uuid at load time; part of the media URL signature.
    media_key: String,
    song_ids: Vec<String>,
    /// Index into `song_ids` of the first track in the Sonos queue.
    window_start: usize,
    window_len: usize,
}

struct LinkedGroup {
    user_id: i32,
    members: Vec<String>,
}

struct SleepTimer {
    ends_at_ms: i64,
    finishing_track: bool,
}

#[derive(Default)]
struct State {
    clients: HashMap<String, ClientZone>,
    sonos_queues: HashMap<String, SonosQueue>,
    linked: HashMap<String, LinkedGroup>,
    settings: HashMap<String, ZoneSettings>,
    sleep: HashMap<String, SleepTimer>,
    /// Last UPnP event per player and whether a coalesced poll is pending,
    /// so NOTIFY floods cost at most four polls a second.
    sonos_events: HashMap<String, (Instant, bool)>,
}

/// Server-side zones.
pub struct ZoneService {
    state: Mutex<State>,
    events: broadcast::Sender<ZoneEvent>,
    sonos: Option<Arc<SonosController>>,
    pool: Option<DbPool>,
    port: AtomicU16,
}

pub(crate) fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn client_zone_id(device_id: &str) -> Option<String> {
    let valid = (8..=64).contains(&device_id.len())
        && device_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    valid.then(|| format!("client:{device_id}"))
}

fn sonos_zone_id(uuid: &str) -> String {
    format!("sonos:{uuid}")
}

fn clean_label(value: &str, fallback: &str) -> String {
    let cleaned: String = value
        .chars()
        .filter(|character| !character.is_control())
        .take(80)
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

fn status_for(transport: TransportState, has_media: bool) -> ZoneStatus {
    match transport {
        TransportState::Playing => ZoneStatus::Playing,
        TransportState::Paused => ZoneStatus::Paused,
        TransportState::Transitioning => ZoneStatus::Buffering,
        TransportState::Stopped if has_media => ZoneStatus::Stopped,
        TransportState::Stopped | TransportState::Unknown => ZoneStatus::Idle,
    }
}

fn now_playing_for(cache: Option<&LibraryCache>, song_id: &str) -> Option<ZoneNowPlaying> {
    let cache = cache?;
    let song = cache.song(song_id)?;
    let album = cache
        .song_map
        .get(song_id)
        .and_then(|(_, album_id)| cache.album(album_id));
    Some(ZoneNowPlaying {
        song_id: Some(song.id.clone()),
        title: song.name.clone(),
        artist: song.artist.clone(),
        album: album.map(|album| album.name.clone()).unwrap_or_default(),
        album_id: album.map(|album| album.id.clone()),
        cover_url: album
            .map(|album| album.cover_url.clone())
            .filter(|cover| !cover.trim().is_empty()),
        duration_ms: (song.duration.max(0.0) * 1_000.0).round() as i64,
    })
}

/// Bounded window of `song_ids` mapped onto a Sonos queue.
fn sonos_window(length: usize, current: usize) -> (usize, usize) {
    if length <= MAX_SONOS_QUEUE_ITEMS {
        return (0, length);
    }
    // Keep a few previous tracks so "previous" works natively on Sonos.
    let start = current
        .saturating_sub(5)
        .min(length - MAX_SONOS_QUEUE_ITEMS);
    (start, MAX_SONOS_QUEUE_ITEMS)
}

impl ZoneService {
    pub fn new(sonos: Option<Arc<SonosController>>, pool: Option<DbPool>) -> Self {
        Self {
            state: Mutex::new(State::default()),
            events: broadcast::channel(512).0,
            sonos,
            pool,
            port: AtomicU16::new(crate::settings::DEFAULT_PORT),
        }
    }

    pub fn set_port(&self, port: u16) {
        self.port.store(port, Ordering::Relaxed);
    }

    pub fn port(&self) -> u16 {
        self.port.load(Ordering::Relaxed)
    }

    pub fn sonos(&self) -> Option<&Arc<SonosController>> {
        self.sonos.as_ref()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ZoneEvent> {
        self.events.subscribe()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn changed(&self, user_id: Option<i32>) {
        let _ = self.events.send(ZoneEvent::Changed { user_id });
    }

    /// Loads persisted zone settings (names, volume limits).
    pub fn load_settings(&self) {
        let Some(pool) = &self.pool else {
            return;
        };
        let Ok(mut connection) = pool.get() else {
            return;
        };
        match load_settings(&mut connection) {
            Ok(settings) => self.lock().settings = settings,
            Err(error) => tracing::warn!(%error, "could not load zone settings"),
        }
    }

    // ----- client zones -------------------------------------------------

    /// Registers or refreshes a client zone and hands back pending commands.
    pub fn register_client(
        &self,
        user_id: i32,
        device_id: &str,
        report: ClientRegistration,
    ) -> Result<RegistrationResult, ZoneError> {
        let zone_id = client_zone_id(device_id).ok_or(ZoneError::Invalid("invalid_device_id"))?;
        if !report.kind.is_client() {
            return Err(ZoneError::Invalid("invalid_zone_kind"));
        }
        if report.volume > 100 || report.position_ms < 0 || report.duration_ms < 0 {
            return Err(ZoneError::Invalid("invalid_zone_state"));
        }
        if let Some(song_ids) = &report.song_ids
            && (song_ids.len() > MAX_ZONE_QUEUE_ITEMS
                || song_ids.iter().any(|id| id.is_empty() || id.len() > 256))
        {
            return Err(ZoneError::Invalid("invalid_zone_queue"));
        }
        let now = now_ms();
        let (result, changed) = {
            let mut state = self.lock();
            let owned = state
                .clients
                .values()
                .filter(|zone| zone.user_id == user_id)
                .count();
            let exists = state
                .clients
                .get(&zone_id)
                .is_some_and(|zone| zone.user_id == user_id);
            if !exists && owned >= MAX_CLIENT_ZONES_PER_USER {
                return Err(ZoneError::Invalid("too_many_zones"));
            }
            if state
                .clients
                .get(&zone_id)
                .is_some_and(|zone| zone.user_id != user_id)
            {
                // The device signed in as someone else.
                state.clients.remove(&zone_id);
            }
            let zone = state
                .clients
                .entry(zone_id.clone())
                .or_insert_with(|| ClientZone {
                    user_id,
                    kind: report.kind,
                    name: String::new(),
                    device: String::new(),
                    status: ZoneStatus::Idle,
                    song_ids: Vec::new(),
                    has_queue: false,
                    current_position: 0,
                    position_ms: 0,
                    duration_ms: 0,
                    reported_at_ms: now,
                    volume: 100,
                    muted: false,
                    last_seen: Instant::now(),
                    commands: VecDeque::new(),
                    next_seq: 1,
                });
            let fallback = match report.kind {
                ZoneKind::Mobile => "Phone",
                ZoneKind::Desktop => "Desktop app",
                _ => "Web browser",
            };
            let name = clean_label(&report.name, fallback);
            let device = clean_label(&report.device, "");
            let mut changed = !exists
                || zone.name != name
                || zone.status != report.status
                || zone.current_position != report.current_position
                || zone.volume != report.volume
                || zone.muted != report.muted
                || zone.kind != report.kind;
            let expected = extrapolate_position(
                zone.position_ms,
                zone.reported_at_ms,
                now,
                zone.status.is_playing(),
                zone.duration_ms,
            );
            if (expected - report.position_ms).abs() > 2_500 {
                changed = true;
            }
            if let Some(song_ids) = report.song_ids {
                changed |= song_ids != zone.song_ids;
                zone.song_ids = song_ids;
                zone.has_queue = true;
            }
            zone.kind = report.kind;
            zone.name = name;
            zone.device = device;
            zone.status = report.status;
            zone.current_position = report.current_position;
            zone.position_ms = report.position_ms;
            zone.duration_ms = report.duration_ms;
            zone.reported_at_ms = now;
            zone.volume = report.volume;
            zone.muted = report.muted;
            zone.last_seen = Instant::now();
            while zone
                .commands
                .front()
                .is_some_and(|command| command.seq <= report.acknowledged_seq)
            {
                zone.commands.pop_front();
            }
            let result = RegistrationResult {
                zone_id: zone_id.clone(),
                commands: zone.commands.iter().cloned().collect(),
                needs_queue: !zone.has_queue,
            };
            (result, changed)
        };
        if changed {
            self.changed(Some(user_id));
        }
        Ok(result)
    }

    pub fn unregister_client(&self, user_id: i32, device_id: &str) -> bool {
        let Some(zone_id) = client_zone_id(device_id) else {
            return false;
        };
        let removed = {
            let mut state = self.lock();
            let owned = state
                .clients
                .get(&zone_id)
                .is_some_and(|zone| zone.user_id == user_id);
            if owned {
                state.clients.remove(&zone_id);
                state.sleep.remove(&zone_id);
                for group in state.linked.values_mut() {
                    group.members.retain(|member| member != &zone_id);
                }
                state.linked.retain(|_, group| group.members.len() >= 2);
            }
            owned
        };
        if removed {
            self.changed(Some(user_id));
        }
        removed
    }

    fn queue_client_command(
        &self,
        user_id: i32,
        zone_id: &str,
        command: ZoneCommand,
    ) -> Result<(), ZoneError> {
        let seq = {
            let mut state = self.lock();
            let zone = state
                .clients
                .get_mut(zone_id)
                .filter(|zone| zone.user_id == user_id)
                .ok_or(ZoneError::NotFound)?;
            let seq = zone.next_seq;
            zone.next_seq += 1;
            if matches!(command, ZoneCommand::Load { .. }) {
                // A new queue supersedes anything still pending.
                zone.commands.clear();
            }
            zone.commands.push_back(PendingCommand {
                seq,
                command: command.clone(),
            });
            while zone.commands.len() > MAX_PENDING_COMMANDS {
                zone.commands.pop_front();
            }
            seq
        };
        let _ = self.events.send(ZoneEvent::Command {
            user_id,
            zone_id: zone_id.to_string(),
            seq,
            command,
        });
        Ok(())
    }

    /// Drops client zones that stopped reporting.
    pub fn expire_clients(&self) {
        let expired: Vec<i32> = {
            let mut state = self.lock();
            let expired: Vec<(String, i32)> = state
                .clients
                .iter()
                .filter(|(_, zone)| zone.last_seen.elapsed() > CLIENT_ZONE_TTL)
                .map(|(id, zone)| (id.clone(), zone.user_id))
                .collect();
            for (id, _) in &expired {
                state.clients.remove(id);
                state.sleep.remove(id);
                for group in state.linked.values_mut() {
                    group.members.retain(|member| member != id);
                }
            }
            state.linked.retain(|_, group| group.members.len() >= 2);
            expired.into_iter().map(|(_, user)| user).collect()
        };
        for user_id in expired {
            self.changed(Some(user_id));
        }
    }

    // ----- listing ------------------------------------------------------

    /// Every zone `user_id` can see.
    pub fn list(&self, user_id: i32, cache: Option<&LibraryCache>) -> Vec<Zone> {
        let now = now_ms();
        let mut zones = Vec::new();
        let state = self.lock();
        let mut clients: Vec<(&String, &ClientZone)> = state
            .clients
            .iter()
            .filter(|(_, zone)| zone.user_id == user_id)
            .collect();
        clients.sort_by(|left, right| left.1.name.cmp(&right.1.name).then(left.0.cmp(right.0)));
        for (id, zone) in clients {
            zones.push(self.client_zone_view(&state, id, zone, cache, now));
        }
        if let Some(sonos) = &self.sonos {
            for group in sonos.groups() {
                zones.push(self.sonos_zone_view(&state, sonos, &group, cache, now));
            }
        }
        let mut linked: Vec<(&String, &LinkedGroup)> = state
            .linked
            .iter()
            .filter(|(_, group)| group.user_id == user_id)
            .collect();
        linked.sort_by(|left, right| left.0.cmp(right.0));
        let mut linked_views = Vec::new();
        for (id, group) in linked {
            if let Some(view) = linked_zone_view(&state, id, group, &zones) {
                linked_views.push(view);
            }
        }
        for zone in &mut zones {
            zone.grouped_into = state
                .linked
                .iter()
                .find(|(_, group)| {
                    group.user_id == user_id && group.members.iter().any(|m| m == &zone.id)
                })
                .map(|(id, _)| id.clone());
        }
        zones.extend(linked_views);
        zones
    }

    pub fn zone(&self, user_id: i32, zone_id: &str, cache: Option<&LibraryCache>) -> Option<Zone> {
        self.list(user_id, cache)
            .into_iter()
            .find(|zone| zone.id == zone_id)
    }

    fn client_zone_view(
        &self,
        state: &State,
        id: &str,
        zone: &ClientZone,
        cache: Option<&LibraryCache>,
        now: i64,
    ) -> Zone {
        let settings = state.settings.get(id).cloned().unwrap_or_default();
        let current = zone.song_ids.get(zone.current_position);
        Zone {
            id: id.to_string(),
            kind: zone.kind,
            ecosystem: zone.kind.ecosystem(),
            name: settings.name.clone().unwrap_or_else(|| zone.name.clone()),
            device: zone.device.clone(),
            status: zone.status,
            source: if current.is_some() {
                ZoneSource::Parson
            } else {
                ZoneSource::None
            },
            now_playing: current.and_then(|song_id| now_playing_for(cache, song_id)),
            queue: ZoneQueue {
                song_ids: zone.song_ids.clone(),
                current_position: zone.current_position,
            },
            position_ms: extrapolate_position(
                zone.position_ms,
                zone.reported_at_ms,
                now,
                zone.status == ZoneStatus::Playing,
                zone.duration_ms,
            ),
            duration_ms: zone.duration_ms,
            volume: Some(ZoneVolume::new(
                zone.volume,
                zone.muted,
                settings.max_volume,
            )),
            group: None,
            grouped_into: None,
            capabilities: ZoneCapabilities {
                transport: true,
                seek: true,
                volume: true,
                mute: true,
                queue: true,
                group: true,
                transfer_target: true,
                sleep_timer: true,
            },
            sleep_timer: sleep_view(state, id),
            notes: Vec::new(),
        }
    }

    fn sonos_zone_view(
        &self,
        state: &State,
        sonos: &SonosController,
        group: &SonosGroup,
        cache: Option<&LibraryCache>,
        now: i64,
    ) -> Zone {
        let id = sonos_zone_id(&group.coordinator);
        let settings = state.settings.get(&id).cloned().unwrap_or_default();
        let coordinator = sonos.player(&group.coordinator);
        let ours = group.track_uri.contains(ZONE_MEDIA_PATH);
        let queue = state.sonos_queues.get(&group.coordinator).filter(|_| ours);
        let current_position = queue
            .map(|queue| {
                (queue.window_start + group.track_number.saturating_sub(1) as usize)
                    .min(queue.song_ids.len().saturating_sub(1))
            })
            .unwrap_or(0);
        let source = if queue.is_some() {
            ZoneSource::Parson
        } else if group.track_uri.is_empty() {
            ZoneSource::None
        } else {
            // Another app's stream, or a Parson queue from before a restart.
            ZoneSource::External
        };
        let now_playing = match source {
            ZoneSource::Parson => queue
                .and_then(|queue| queue.song_ids.get(current_position))
                .and_then(|song_id| now_playing_for(cache, song_id)),
            ZoneSource::External => Some(ZoneNowPlaying {
                song_id: None,
                title: group
                    .track_title
                    .clone()
                    .unwrap_or_else(|| "Playing from the Sonos app".to_string()),
                artist: String::new(),
                album: String::new(),
                album_id: None,
                cover_url: None,
                duration_ms: group.duration_ms,
            }),
            ZoneSource::None => None,
        };
        let members: Vec<ZoneGroupMember> = group
            .members
            .iter()
            .filter_map(|member| sonos.player(member))
            .map(|player| ZoneGroupMember {
                id: sonos_zone_id(&player.uuid),
                name: player.room_name.clone(),
                kind: ZoneKind::Sonos,
                volume: Some(ZoneVolume::new(
                    player.volume,
                    player.muted,
                    state
                        .settings
                        .get(&sonos_zone_id(&player.uuid))
                        .and_then(|settings| settings.max_volume),
                )),
            })
            .collect();
        let mut notes = vec![
            "Sonos plays up to 48 kHz/24-bit. MP3, AAC, ALAC, FLAC and WAV within that limit stream untouched; anything else is converted to 24-bit FLAC at 44.1 or 48 kHz.".to_string(),
        ];
        if source == ZoneSource::External {
            notes.push(
                "Another app is playing on this Sonos zone. Playing from Parson replaces it."
                    .to_string(),
            );
        }
        Zone {
            id,
            kind: ZoneKind::Sonos,
            ecosystem: "sonos",
            name: settings.name.clone().unwrap_or_else(|| group.name.clone()),
            device: coordinator
                .map(|player| player.model_name)
                .unwrap_or_else(|| "Sonos".to_string()),
            status: status_for(group.transport, source != ZoneSource::None),
            source,
            now_playing,
            queue: ZoneQueue {
                song_ids: queue
                    .map(|queue| queue.song_ids.clone())
                    .unwrap_or_default(),
                current_position,
            },
            position_ms: extrapolate_position(
                group.position_ms,
                group.polled_at_ms,
                now,
                group.transport == TransportState::Playing,
                group.duration_ms,
            ),
            duration_ms: if source == ZoneSource::None {
                0
            } else {
                group.duration_ms
            },
            volume: Some(ZoneVolume::new(
                group.group_volume,
                group.group_muted,
                settings.max_volume,
            )),
            group: (members.len() > 1).then(|| ZoneGroupInfo {
                id: sonos_zone_id(&group.coordinator),
                synced: true,
                sync_note: SONOS_SYNC_NOTE,
                members,
            }),
            grouped_into: None,
            capabilities: ZoneCapabilities {
                transport: source != ZoneSource::None,
                seek: source == ZoneSource::Parson,
                volume: true,
                mute: true,
                queue: source == ZoneSource::Parson,
                group: true,
                transfer_target: true,
                sleep_timer: true,
            },
            sleep_timer: sleep_view(state, &sonos_zone_id(&group.coordinator)),
            notes,
        }
    }

    // ----- commands -----------------------------------------------------

    /// Current queue and position of a zone, for transfers.
    pub fn snapshot(&self, user_id: i32, zone_id: &str) -> Result<PlaybackSnapshot, ZoneError> {
        let zone = self
            .zone(user_id, zone_id, None)
            .ok_or(ZoneError::NotFound)?;
        if zone.source != ZoneSource::Parson || zone.queue.song_ids.is_empty() {
            return Err(ZoneError::Unsupported("zone_has_no_parson_queue"));
        }
        Ok(PlaybackSnapshot {
            song_ids: zone.queue.song_ids,
            current_position: zone.queue.current_position,
            position_ms: zone.position_ms,
            playing: zone.status.is_playing(),
        })
    }

    /// Sends `command` to a zone (fan-out for linked groups).
    pub async fn command(
        &self,
        user_id: i32,
        zone_id: &str,
        command: ZoneCommand,
        cache: Option<&LibraryCache>,
    ) -> Result<(), ZoneError> {
        command.validate().map_err(ZoneError::Invalid)?;
        if let Some(group_id) = zone_id.strip_prefix("linked:").map(|_| zone_id) {
            let members = {
                let state = self.lock();
                let group = state
                    .linked
                    .get(group_id)
                    .filter(|group| group.user_id == user_id)
                    .ok_or(ZoneError::NotFound)?;
                group.members.clone()
            };
            if let Some(member) = command.member_id() {
                if !members.iter().any(|candidate| candidate == member) {
                    return Err(ZoneError::Invalid("member_not_in_group"));
                }
                let mut command = command.clone();
                strip_member(&mut command);
                return Box::pin(self.command(user_id, member, command, cache)).await;
            }
            let mut first_error = None;
            for member in members {
                if let Err(error) =
                    Box::pin(self.command(user_id, &member, command.clone(), cache)).await
                {
                    tracing::warn!(%error, member, "linked zone member command failed");
                    first_error.get_or_insert(error);
                }
            }
            return first_error.map_or(Ok(()), Err);
        }
        if let Some(uuid) = zone_id.strip_prefix("sonos:") {
            return self.sonos_command(user_id, uuid, command, cache).await;
        }
        if zone_id.starts_with("client:") {
            let command = self.clamp_client_command(user_id, zone_id, command)?;
            return self.queue_client_command(user_id, zone_id, command);
        }
        Err(ZoneError::NotFound)
    }

    fn max_volume(&self, zone_id: &str) -> u8 {
        self.lock()
            .settings
            .get(zone_id)
            .and_then(|settings| settings.max_volume)
            .unwrap_or(100)
    }

    fn clamp_client_command(
        &self,
        user_id: i32,
        zone_id: &str,
        command: ZoneCommand,
    ) -> Result<ZoneCommand, ZoneError> {
        let limit = self.max_volume(zone_id);
        Ok(match command {
            ZoneCommand::SetVolume { volume, .. } => ZoneCommand::SetVolume {
                volume: volume.min(limit),
                member_id: None,
            },
            ZoneCommand::AdjustVolume { delta, .. } => {
                let current = self
                    .lock()
                    .clients
                    .get(zone_id)
                    .filter(|zone| zone.user_id == user_id)
                    .map(|zone| zone.volume)
                    .ok_or(ZoneError::NotFound)?;
                ZoneCommand::SetVolume {
                    volume: (i16::from(current) + i16::from(delta)).clamp(0, i16::from(limit))
                        as u8,
                    member_id: None,
                }
            }
            ZoneCommand::SetMute { muted, .. } => ZoneCommand::SetMute {
                muted,
                member_id: None,
            },
            other => other,
        })
    }

    async fn sonos_command(
        &self,
        user_id: i32,
        uuid: &str,
        command: ZoneCommand,
        cache: Option<&LibraryCache>,
    ) -> Result<(), ZoneError> {
        let sonos = self
            .sonos
            .clone()
            .ok_or(ZoneError::Unsupported("sonos_disabled"))?;
        let group = sonos.group_of(uuid).ok_or(ZoneError::NotFound)?;
        let coordinator = group.coordinator.clone();
        let device = |error: crate::sonos::soap::SoapError| ZoneError::Device(error.to_string());
        let zone_id = sonos_zone_id(&coordinator);
        let _ = user_id;
        match command {
            ZoneCommand::Play => sonos
                .transport(&coordinator, Transport::Play)
                .await
                .map_err(device),
            ZoneCommand::Pause => sonos
                .transport(&coordinator, Transport::Pause)
                .await
                .map_err(device),
            ZoneCommand::PlayPause => {
                let action = if group.transport == TransportState::Playing {
                    Transport::Pause
                } else {
                    Transport::Play
                };
                sonos.transport(&coordinator, action).await.map_err(device)
            }
            ZoneCommand::Stop => sonos
                .transport(&coordinator, Transport::Stop)
                .await
                .map_err(device),
            ZoneCommand::Next => self.sonos_step(&sonos, &group, 1, cache).await,
            ZoneCommand::Previous => self.sonos_step(&sonos, &group, -1, cache).await,
            ZoneCommand::Seek { position_ms } => sonos
                .transport(&coordinator, Transport::SeekTime(position_ms))
                .await
                .map_err(device),
            ZoneCommand::Jump { queue_position } => {
                let window = {
                    let state = self.lock();
                    state
                        .sonos_queues
                        .get(&coordinator)
                        .map(|queue| (queue.window_start, queue.window_len, queue.song_ids.clone()))
                };
                let Some((start, length, song_ids)) = window else {
                    return Err(ZoneError::Unsupported("zone_has_no_parson_queue"));
                };
                if queue_position >= song_ids.len() {
                    return Err(ZoneError::Invalid("invalid_queue_position"));
                }
                if queue_position >= start && queue_position < start + length {
                    sonos
                        .transport(
                            &coordinator,
                            Transport::SeekTrack((queue_position - start) as u32),
                        )
                        .await
                        .map_err(device)?;
                    sonos
                        .transport(&coordinator, Transport::Play)
                        .await
                        .map_err(device)
                } else {
                    let cache = cache.ok_or(ZoneError::Unsupported("library_unavailable"))?;
                    self.load_sonos(
                        &sonos,
                        &coordinator,
                        song_ids,
                        queue_position,
                        0,
                        true,
                        cache,
                    )
                    .await
                }
            }
            ZoneCommand::SetVolume { volume, member_id } => match member_id {
                Some(member) => {
                    let member_uuid = member
                        .strip_prefix("sonos:")
                        .filter(|member| group.members.iter().any(|m| m == member))
                        .ok_or(ZoneError::Invalid("member_not_in_group"))?;
                    let limit = self.max_volume(&member);
                    sonos
                        .set_player_volume(member_uuid, volume.min(limit))
                        .await
                        .map_err(device)
                }
                None => {
                    let limit = self.max_volume(&zone_id);
                    sonos
                        .set_group_volume(&coordinator, volume.min(limit))
                        .await
                        .map_err(device)
                }
            },
            ZoneCommand::AdjustVolume { delta, member_id } => {
                let (current, target_zone) = match &member_id {
                    Some(member) => {
                        let member_uuid = member
                            .strip_prefix("sonos:")
                            .filter(|member| group.members.iter().any(|m| m == member))
                            .ok_or(ZoneError::Invalid("member_not_in_group"))?;
                        (
                            sonos.player(member_uuid).map(|p| p.volume).unwrap_or(0),
                            member.clone(),
                        )
                    }
                    None => (group.group_volume, zone_id.clone()),
                };
                let limit = self.max_volume(&target_zone);
                let volume =
                    (i16::from(current) + i16::from(delta)).clamp(0, i16::from(limit)) as u8;
                Box::pin(self.sonos_command(
                    user_id,
                    uuid,
                    ZoneCommand::SetVolume { volume, member_id },
                    cache,
                ))
                .await
            }
            ZoneCommand::SetMute { muted, member_id } => match member_id {
                Some(member) => {
                    let member_uuid = member
                        .strip_prefix("sonos:")
                        .filter(|member| group.members.iter().any(|m| m == member))
                        .ok_or(ZoneError::Invalid("member_not_in_group"))?;
                    sonos
                        .set_player_mute(member_uuid, muted)
                        .await
                        .map_err(device)
                }
                None => sonos
                    .set_group_mute(&coordinator, muted)
                    .await
                    .map_err(device),
            },
            ZoneCommand::Load {
                song_ids,
                current_position,
                position_ms,
                play,
            } => {
                let cache = cache.ok_or(ZoneError::Unsupported("library_unavailable"))?;
                self.load_sonos(
                    &sonos,
                    &coordinator,
                    song_ids,
                    current_position,
                    position_ms,
                    play,
                    cache,
                )
                .await
            }
        }
    }

    /// Next/previous; reloads the window when the Sonos queue edge is reached.
    async fn sonos_step(
        &self,
        sonos: &SonosController,
        group: &SonosGroup,
        step: i32,
        cache: Option<&LibraryCache>,
    ) -> Result<(), ZoneError> {
        let device = |error: crate::sonos::soap::SoapError| ZoneError::Device(error.to_string());
        let window = {
            let state = self.lock();
            state
                .sonos_queues
                .get(&group.coordinator)
                .map(|queue| (queue.window_start, queue.window_len, queue.song_ids.clone()))
        };
        if let Some((start, length, song_ids)) = window {
            let index = group.track_number.saturating_sub(1) as usize;
            let at_edge = (step > 0 && index + 1 >= length) || (step < 0 && index == 0);
            if at_edge && let Some(cache) = cache {
                let absolute = start + index;
                let target = if step > 0 {
                    absolute + 1
                } else {
                    absolute.saturating_sub(1)
                };
                if target < song_ids.len() && target != absolute {
                    return self
                        .load_sonos(sonos, &group.coordinator, song_ids, target, 0, true, cache)
                        .await;
                }
                if step > 0 {
                    return Err(ZoneError::Invalid("end_of_queue"));
                }
            }
        }
        let action = if step > 0 {
            Transport::Next
        } else {
            Transport::Previous
        };
        sonos
            .transport(&group.coordinator, action)
            .await
            .map_err(device)
    }

    #[allow(clippy::too_many_arguments)]
    async fn load_sonos(
        &self,
        sonos: &SonosController,
        coordinator: &str,
        song_ids: Vec<String>,
        current_position: usize,
        position_ms: i64,
        play: bool,
        cache: &LibraryCache,
    ) -> Result<(), ZoneError> {
        if song_ids.iter().any(|song_id| cache.song(song_id).is_none()) {
            return Err(ZoneError::Invalid("invalid_zone_queue"));
        }
        let origin = sonos
            .local_origin_for(coordinator, self.port())
            .ok_or_else(|| ZoneError::Device("no network route to the Sonos player".into()))?;
        let (window_start, window_len) = sonos_window(song_ids.len(), current_position);
        let expires = chrono::Utc::now().timestamp() + MEDIA_URL_LIFETIME_SECONDS;
        let tracks: Vec<(String, String)> = song_ids[window_start..window_start + window_len]
            .iter()
            .filter_map(|song_id| sonos_track(cache, &origin, coordinator, song_id, expires))
            .collect();
        if tracks.len() != window_len {
            return Err(ZoneError::Invalid("invalid_zone_queue"));
        }
        // Record the queue first so media requests that arrive during
        // loading are authorised.
        {
            let mut state = self.lock();
            state.sonos_queues.insert(
                coordinator.to_string(),
                SonosQueue {
                    media_key: coordinator.to_string(),
                    song_ids,
                    window_start,
                    window_len,
                },
            );
        }
        let loaded = sonos
            .load_queue(
                coordinator,
                &tracks,
                (current_position - window_start) as u32,
                position_ms,
                play,
            )
            .await;
        self.changed(None);
        loaded.map_err(|error| ZoneError::Device(error.to_string()))
    }

    /// Whether a signed Sonos media URL for `key` may stream `song_id`.
    pub fn media_allowed(&self, key: &str, song_id: &str) -> bool {
        self.lock().sonos_queues.values().any(|queue| {
            queue.media_key == key
                && queue.song_ids[queue.window_start..queue.window_start + queue.window_len]
                    .iter()
                    .any(|candidate| candidate == song_id)
        })
    }

    // ----- transfer and grouping ---------------------------------------

    /// Moves queue and position from `from` to `to`, then stops `from`.
    pub async fn transfer(
        &self,
        user_id: i32,
        from: &str,
        to: &str,
        cache: Option<&LibraryCache>,
    ) -> Result<(), ZoneError> {
        if from == to {
            return Err(ZoneError::Invalid("same_zone"));
        }
        let snapshot = self.snapshot(user_id, from)?;
        self.transfer_snapshot(user_id, from, to, snapshot, cache)
            .await
    }

    /// Loads `snapshot` on `to`, then pauses `from` (when it is a zone here).
    pub async fn transfer_snapshot(
        &self,
        user_id: i32,
        from: &str,
        to: &str,
        snapshot: PlaybackSnapshot,
        cache: Option<&LibraryCache>,
    ) -> Result<(), ZoneError> {
        let target = self.zone(user_id, to, None).ok_or(ZoneError::NotFound)?;
        if !target.capabilities.transfer_target {
            return Err(ZoneError::Unsupported("zone_cannot_receive_transfer"));
        }
        self.command(
            user_id,
            to,
            ZoneCommand::Load {
                song_ids: snapshot.song_ids,
                current_position: snapshot.current_position,
                position_ms: snapshot.position_ms,
                play: true,
            },
            cache,
        )
        .await?;
        if self.zone(user_id, from, None).is_some() {
            let stop = if from.starts_with("client:") {
                ZoneCommand::Pause
            } else {
                ZoneCommand::Stop
            };
            if let Err(error) = self.command(user_id, from, stop, cache).await {
                tracing::warn!(%error, from, "could not stop the transfer source");
            }
        }
        self.changed(Some(user_id));
        Ok(())
    }

    /// Groups zones. All-Sonos selections become a native, sample-synced
    /// Sonos group; anything else becomes a linked (not sample-synced) group.
    pub async fn group(&self, user_id: i32, zone_ids: &[String]) -> Result<String, ZoneError> {
        let mut unique: Vec<String> = Vec::new();
        for id in zone_ids {
            if !unique.contains(id) {
                unique.push(id.clone());
            }
        }
        if unique.len() < 2 || unique.len() > 16 {
            return Err(ZoneError::Invalid("group_needs_two_to_sixteen_zones"));
        }
        let zones = self.list(user_id, None);
        for id in &unique {
            let zone = zones
                .iter()
                .find(|zone| &zone.id == id)
                .ok_or(ZoneError::NotFound)?;
            if zone.kind == ZoneKind::Linked {
                return Err(ZoneError::Invalid("cannot_nest_groups"));
            }
            if zone.kind == ZoneKind::Chromecast {
                return Err(ZoneError::Unsupported(
                    "chromecast_groups_are_made_in_google_home",
                ));
            }
        }
        if unique.iter().all(|id| id.starts_with("sonos:")) {
            let sonos = self
                .sonos
                .clone()
                .ok_or(ZoneError::Unsupported("sonos_disabled"))?;
            let coordinator = unique[0].trim_start_matches("sonos:").to_string();
            for id in &unique[1..] {
                let uuid = id.trim_start_matches("sonos:");
                let members = sonos
                    .group_of(uuid)
                    .map(|group| group.members)
                    .unwrap_or_else(|| vec![uuid.to_string()]);
                for member in members {
                    sonos
                        .join(&member, &coordinator)
                        .await
                        .map_err(|error| ZoneError::Device(error.to_string()))?;
                }
            }
            sonos.refresh_topology().await.map_err(ZoneError::Device)?;
            let _ = sonos.poll_group(&coordinator).await;
            self.changed(None);
            return Ok(sonos_zone_id(&coordinator));
        }
        let id = format!("linked:{}", uuid::Uuid::new_v4());
        {
            let mut state = self.lock();
            if state
                .linked
                .values()
                .filter(|group| group.user_id == user_id)
                .count()
                >= MAX_LINKED_GROUPS_PER_USER
            {
                return Err(ZoneError::Invalid("too_many_groups"));
            }
            // A zone lives in one linked group at a time.
            for group in state.linked.values_mut() {
                if group.user_id == user_id {
                    group.members.retain(|member| !unique.contains(member));
                }
            }
            state.linked.retain(|_, group| group.members.len() >= 2);
            state.linked.insert(
                id.clone(),
                LinkedGroup {
                    user_id,
                    members: unique.clone(),
                },
            );
        }
        // Start every member on the leader's queue so they play the same thing.
        if let Ok(snapshot) = self.snapshot(user_id, &unique[0]) {
            for member in &unique[1..] {
                let _ = self
                    .command(
                        user_id,
                        member,
                        ZoneCommand::Load {
                            song_ids: snapshot.song_ids.clone(),
                            current_position: snapshot.current_position,
                            position_ms: snapshot.position_ms,
                            play: snapshot.playing,
                        },
                        None,
                    )
                    .await;
            }
        }
        self.changed(Some(user_id));
        Ok(id)
    }

    /// Ungroups a linked or Sonos group, or removes one member when given.
    pub async fn ungroup(
        &self,
        user_id: i32,
        group_id: &str,
        member: Option<&str>,
    ) -> Result<(), ZoneError> {
        if group_id.starts_with("linked:") {
            {
                let mut state = self.lock();
                let group = state
                    .linked
                    .get_mut(group_id)
                    .filter(|group| group.user_id == user_id)
                    .ok_or(ZoneError::NotFound)?;
                match member {
                    Some(member) => group.members.retain(|candidate| candidate != member),
                    None => group.members.clear(),
                }
                state.linked.retain(|_, group| group.members.len() >= 2);
            }
            self.changed(Some(user_id));
            return Ok(());
        }
        let uuid = group_id.strip_prefix("sonos:").ok_or(ZoneError::NotFound)?;
        let sonos = self
            .sonos
            .clone()
            .ok_or(ZoneError::Unsupported("sonos_disabled"))?;
        let group = sonos.group_of(uuid).ok_or(ZoneError::NotFound)?;
        let leaving: Vec<String> = match member {
            Some(member) => {
                let member = member.trim_start_matches("sonos:").to_string();
                if !group.members.contains(&member) {
                    return Err(ZoneError::Invalid("member_not_in_group"));
                }
                vec![member]
            }
            None => group
                .members
                .iter()
                .filter(|member| **member != group.coordinator)
                .cloned()
                .collect(),
        };
        for member in leaving {
            sonos
                .leave(&member)
                .await
                .map_err(|error| ZoneError::Device(error.to_string()))?;
        }
        sonos.refresh_topology().await.map_err(ZoneError::Device)?;
        self.changed(None);
        Ok(())
    }

    // ----- settings and sleep timer -------------------------------------

    pub async fn update_settings(
        &self,
        user_id: i32,
        zone_id: &str,
        settings: ZoneSettings,
    ) -> Result<ZoneSettings, ZoneError> {
        let known = self.zone(user_id, zone_id, None).is_some()
            || self.lock().linked.contains_key(zone_id)
            || zone_id
                .strip_prefix("sonos:")
                .zip(self.sonos.as_ref())
                .is_some_and(|(uuid, sonos)| sonos.knows_player(uuid));
        if !known {
            return Err(ZoneError::NotFound);
        }
        let cleaned = ZoneSettings {
            name: settings
                .name
                .map(|name| clean_label(&name, ""))
                .filter(|name| !name.is_empty()),
            max_volume: settings.max_volume.map(|volume| volume.clamp(1, 100)),
        };
        self.lock()
            .settings
            .insert(zone_id.to_string(), cleaned.clone());
        if let Some(pool) = self.pool.clone() {
            let zone_id = zone_id.to_string();
            let stored = cleaned.clone();
            let saved = tokio::task::spawn_blocking(move || {
                let mut connection = pool.get().map_err(|error| error.to_string())?;
                save_setting(&mut connection, &zone_id, &stored).map_err(|error| error.to_string())
            })
            .await;
            if !matches!(saved, Ok(Ok(()))) {
                tracing::warn!(?saved, "could not persist zone settings");
            }
        }
        let shared = zone_id.starts_with("sonos:");
        self.changed(if shared { None } else { Some(user_id) });
        Ok(cleaned)
    }

    pub fn set_sleep_timer(
        &self,
        user_id: i32,
        zone_id: &str,
        minutes: Option<u32>,
    ) -> Result<(), ZoneError> {
        let zone = self
            .zone(user_id, zone_id, None)
            .ok_or(ZoneError::NotFound)?;
        if !zone.capabilities.sleep_timer {
            return Err(ZoneError::Unsupported("sleep_timer_unsupported"));
        }
        {
            let mut state = self.lock();
            match minutes {
                Some(minutes) if (1..=24 * 60).contains(&minutes) => {
                    state.sleep.insert(
                        zone_id.to_string(),
                        SleepTimer {
                            ends_at_ms: now_ms() + i64::from(minutes) * 60_000,
                            finishing_track: false,
                        },
                    );
                }
                Some(_) => return Err(ZoneError::Invalid("invalid_sleep_minutes")),
                None => {
                    state.sleep.remove(zone_id);
                }
            }
        }
        self.changed(if zone_id.starts_with("sonos:") {
            None
        } else {
            Some(user_id)
        });
        Ok(())
    }

    /// Fires due sleep timers. A timer that runs out within five minutes of a
    /// track's end lets that track finish first, as Roon does.
    pub async fn run_sleep_timers(&self, user_for_zone: impl Fn(&str) -> Option<i32>) {
        let now = now_ms();
        let due: Vec<(String, bool)> = {
            let state = self.lock();
            state
                .sleep
                .iter()
                .filter(|(_, timer)| timer.ends_at_ms <= now)
                .map(|(id, timer)| (id.clone(), timer.finishing_track))
                .collect()
        };
        for (zone_id, finishing) in due {
            let Some(user_id) = self.owner_of(&zone_id).or_else(|| user_for_zone(&zone_id)) else {
                self.lock().sleep.remove(&zone_id);
                continue;
            };
            let zone = self.zone(user_id, &zone_id, None);
            if !finishing
                && let Some(zone) = &zone
                && zone.status == ZoneStatus::Playing
                && zone.duration_ms > 0
            {
                let remaining = zone.duration_ms - zone.position_ms;
                if remaining > 1_000 && remaining <= SLEEP_FINISH_TRACK_WINDOW_MS {
                    if let Some(timer) = self.lock().sleep.get_mut(&zone_id) {
                        timer.ends_at_ms = now + remaining - 500;
                        timer.finishing_track = true;
                    }
                    self.changed(Some(user_id));
                    continue;
                }
            }
            self.lock().sleep.remove(&zone_id);
            if zone.is_some() {
                let _ = self
                    .command(user_id, &zone_id, ZoneCommand::Pause, None)
                    .await;
            }
            self.changed(if zone_id.starts_with("sonos:") {
                None
            } else {
                Some(user_id)
            });
        }
    }

    #[cfg(test)]
    pub fn force_sleep_due(&self, zone_id: &str) {
        if let Some(timer) = self.lock().sleep.get_mut(zone_id) {
            timer.ends_at_ms = 0;
        }
    }

    fn owner_of(&self, zone_id: &str) -> Option<i32> {
        let state = self.lock();
        state
            .clients
            .get(zone_id)
            .map(|zone| zone.user_id)
            .or_else(|| state.linked.get(zone_id).map(|group| group.user_id))
    }

    // ----- Sonos background work ---------------------------------------

    /// Background loop: discovery, topology, polling, events, timers and
    /// client expiry.
    pub async fn run(self: Arc<Self>) {
        let mut tick: u64 = 0;
        let mut sonos_changes = self.sonos.as_ref().map(|sonos| sonos.subscribe_changes());
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = interval.tick() => {}
                changed = async {
                    match sonos_changes.as_mut() {
                        Some(receiver) => receiver.recv().await.ok(),
                        None => std::future::pending().await,
                    }
                } => {
                    if changed.is_some() {
                        self.changed(None);
                    }
                    continue;
                }
            }
            tick += 1;
            self.expire_clients();
            self.run_sleep_timers(|_| None).await;
            let Some(sonos) = self.sonos.clone() else {
                continue;
            };
            if tick == 1 || tick.is_multiple_of(60) {
                if let Err(error) = sonos.discover().await {
                    tracing::debug!(%error, "Sonos discovery found no topology");
                }
                sonos.ensure_subscriptions(self.port()).await;
            } else if tick.is_multiple_of(10) {
                let _ = sonos.refresh_topology().await;
            }
            let active: Vec<String> = sonos
                .groups()
                .into_iter()
                .filter(|group| {
                    matches!(
                        group.transport,
                        TransportState::Playing | TransportState::Transitioning
                    )
                })
                .map(|group| group.coordinator)
                .collect();
            if tick.is_multiple_of(5) {
                sonos.poll_all().await;
            } else {
                for coordinator in active {
                    let _ = sonos.poll_group(&coordinator).await;
                }
            }
        }
    }

    /// A UPnP NOTIFY arrived for `uuid`: poll its group now.
    pub async fn sonos_event(&self, uuid: &str) {
        let Some(sonos) = &self.sonos else {
            return;
        };
        const WINDOW: Duration = Duration::from_millis(250);
        let wait = {
            let mut state = self.lock();
            if state.sonos_events.len() > 256 {
                state.sonos_events.clear();
            }
            match state.sonos_events.get(uuid).copied() {
                Some((_, true)) => return,
                Some((at, false)) if at.elapsed() < WINDOW => {
                    state.sonos_events.insert(uuid.to_string(), (at, true));
                    Some(WINDOW - at.elapsed())
                }
                _ => {
                    state
                        .sonos_events
                        .insert(uuid.to_string(), (Instant::now(), false));
                    None
                }
            }
        };
        if let Some(wait) = wait {
            tokio::time::sleep(wait).await;
            self.lock()
                .sonos_events
                .insert(uuid.to_string(), (Instant::now(), false));
        }
        if let Some(group) = sonos.group_of(uuid) {
            let _ = sonos.poll_group(&group.coordinator).await;
        } else if sonos.knows_player(uuid) {
            let _ = sonos.refresh_topology().await;
        }
    }
}

fn strip_member(command: &mut ZoneCommand) {
    match command {
        ZoneCommand::SetVolume { member_id, .. }
        | ZoneCommand::AdjustVolume { member_id, .. }
        | ZoneCommand::SetMute { member_id, .. } => *member_id = None,
        _ => {}
    }
}

fn sleep_view(state: &State, zone_id: &str) -> Option<ZoneSleepTimer> {
    state.sleep.get(zone_id).map(|timer| ZoneSleepTimer {
        ends_at_ms: timer.ends_at_ms,
        finishing_track: timer.finishing_track,
    })
}

fn linked_zone_view(state: &State, id: &str, group: &LinkedGroup, zones: &[Zone]) -> Option<Zone> {
    let members: Vec<&Zone> = group
        .members
        .iter()
        .filter_map(|member| zones.iter().find(|zone| &zone.id == member))
        .collect();
    let leader = *members.first()?;
    let settings = state.settings.get(id).cloned().unwrap_or_default();
    let volumes: Vec<u32> = members
        .iter()
        .filter_map(|zone| zone.volume.as_ref().map(|volume| u32::from(volume.level)))
        .collect();
    let level = if volumes.is_empty() {
        0
    } else {
        (volumes.iter().sum::<u32>() / volumes.len() as u32) as u8
    };
    let muted = members
        .iter()
        .all(|zone| zone.volume.as_ref().is_some_and(|volume| volume.muted));
    Some(Zone {
        id: id.to_string(),
        kind: ZoneKind::Linked,
        ecosystem: "linked",
        name: settings.name.clone().unwrap_or_else(|| {
            members
                .iter()
                .map(|zone| zone.name.as_str())
                .collect::<Vec<_>>()
                .join(" + ")
        }),
        device: format!("{} linked zones", members.len()),
        status: leader.status,
        source: leader.source,
        now_playing: leader.now_playing.clone(),
        queue: leader.queue.clone(),
        position_ms: leader.position_ms,
        duration_ms: leader.duration_ms,
        volume: Some(ZoneVolume::new(level, muted, settings.max_volume)),
        group: Some(ZoneGroupInfo {
            id: id.to_string(),
            synced: false,
            sync_note: LINKED_SYNC_NOTE,
            members: members
                .iter()
                .map(|zone| ZoneGroupMember {
                    id: zone.id.clone(),
                    name: zone.name.clone(),
                    kind: zone.kind,
                    volume: zone.volume.clone(),
                })
                .collect(),
        }),
        grouped_into: None,
        capabilities: ZoneCapabilities {
            transport: true,
            seek: true,
            volume: true,
            mute: true,
            queue: true,
            group: false,
            transfer_target: true,
            sleep_timer: true,
        },
        sleep_timer: sleep_view(state, id),
        notes: vec![LINKED_SYNC_NOTE.to_string()],
    })
}

/// Signed URL and DIDL-Lite for one track on a Sonos zone.
fn sonos_track(
    cache: &LibraryCache,
    origin: &str,
    media_key: &str,
    song_id: &str,
    expires: i64,
) -> Option<(String, String)> {
    let song = cache.song(song_id)?;
    let album = cache
        .song_map
        .get(song_id)
        .and_then(|(_, album_id)| cache.album(album_id));
    let plan = crate::api::library::sonos_stream_plan(song);
    let signature =
        crate::api::cast::sign_media_url(&format!("zone:{media_key}"), song_id, expires);
    let uri = format!(
        "{origin}{ZONE_MEDIA_PATH}{}/stream?zone={}&expires={expires}&signature={signature}",
        crate::api::cast::encoded(song_id),
        crate::api::cast::encoded(media_key),
    );
    let album_art_uri = album
        .filter(|album| !album.cover_url.trim().is_empty())
        .map(|album| {
            let image_signature = crate::api::image::sign_image_path(&album.cover_url, expires);
            format!(
                "{origin}/media/images/{}?raw=true&expires={expires}&image_signature={image_signature}",
                crate::api::cast::encoded(&album.cover_url)
            )
        });
    let metadata = crate::sonos::didl::render(&crate::sonos::didl::DidlTrack {
        item_id: format!("parson:{song_id}"),
        title: song.name.clone(),
        artist: song.artist.clone(),
        album: album.map(|album| album.name.clone()).unwrap_or_default(),
        album_art_uri,
        duration_ms: (song.duration.max(0.0) * 1_000.0).round() as i64,
        uri: uri.clone(),
        mime_type: plan.mime_type.to_string(),
    });
    Some((uri, metadata))
}

// ----- persistence -------------------------------------------------------

#[derive(diesel::QueryableByName)]
struct SettingRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    zone_id: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    name: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
    max_volume: Option<i32>,
}

fn load_settings(
    connection: &mut diesel::sqlite::SqliteConnection,
) -> Result<HashMap<String, ZoneSettings>, diesel::result::Error> {
    use diesel::RunQueryDsl;
    let rows = diesel::sql_query("SELECT zone_id, name, max_volume FROM zone_setting")
        .load::<SettingRow>(connection)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.zone_id,
                ZoneSettings {
                    name: row.name,
                    max_volume: row.max_volume.and_then(|volume| u8::try_from(volume).ok()),
                },
            )
        })
        .collect())
}

fn save_setting(
    connection: &mut diesel::sqlite::SqliteConnection,
    zone_id: &str,
    settings: &ZoneSettings,
) -> Result<(), diesel::result::Error> {
    use diesel::RunQueryDsl;
    use diesel::sql_types::{Integer, Nullable, Text};
    diesel::sql_query(
        "INSERT INTO zone_setting (zone_id, name, max_volume, updated_at)
         VALUES (?, ?, ?, CURRENT_TIMESTAMP)
         ON CONFLICT(zone_id) DO UPDATE SET name = excluded.name,
             max_volume = excluded.max_volume, updated_at = CURRENT_TIMESTAMP",
    )
    .bind::<Text, _>(zone_id)
    .bind::<Nullable<Text>, _>(settings.name.as_deref())
    .bind::<Nullable<Integer>, _>(settings.max_volume.map(i32::from))
    .execute(connection)?;
    Ok(())
}
