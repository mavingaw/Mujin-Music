//! The zone registry and its drivers.
//!
//! Every running Parson client (web, desktop, mobile) registers itself as a
//! zone and receives remote commands over the zone WebSocket. Sonos groups
//! come from the [`SonosController`], and the server drives them directly:
//! it maps the Parson queue onto the Sonos queue with signed media URLs and
//! follows progress by polling plus UPnP events. Linked groups fan commands
//! out to zones of any ecosystem and are clearly labelled as not
//! sample-synced.

mod hosted;

pub use hosted::{HostedStream, render_scope};

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use super::leveling::{LevelBy, LoudnessStore};
use super::model::*;
use crate::library::state::LibraryCache;
use crate::persistence::connection::DbPool;
use crate::sonos::{SonosController, SonosGroup, Transport, TransportState};

const CLIENT_ZONE_TTL: Duration = Duration::from_secs(45);
const MAX_CLIENT_ZONES_PER_USER: usize = 32;
const MAX_PENDING_COMMANDS: usize = 32;
const MAX_LINKED_GROUPS_PER_USER: usize = 16;
const MEDIA_URL_LIFETIME_SECONDS: i64 = 12 * 60 * 60;
/// What Sonos plays untouched and what Parson converts for it.
fn sonos_format_note() -> String {
    crate::outputs::FormatLimits::SONOS.note("Sonos")
}
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
    /// A volume change for a zone whose volume Home Assistant controls.
    /// `None` means every user (shared rooms and devices).
    VolumeRequest {
        user_id: Option<i32>,
        request: VolumeRequest,
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
    #[serde(default)]
    pub repeat: RepeatMode,
    #[serde(default)]
    pub shuffle: bool,
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

/// Stored per-zone settings, shared by every client.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ZoneSettings {
    /// Custom name; `None` uses the device's own name.
    pub name: Option<String>,
    /// Custom icon from [`ZONE_ICONS`]; `None` uses the device's icon.
    pub icon: Option<String>,
    pub crossfade_seconds: u8,
    pub leveling: LevelingMode,
    pub leveling_target_lufs: i8,
    pub leveling_adjustment_db: i8,
    pub comfort_volume: Option<u8>,
    pub safety_min_volume: Option<u8>,
    pub safety_max_volume: Option<u8>,
    /// Only the device the zone runs on can see and control it.
    pub private: bool,
    /// The zone's DSP chain.
    pub dsp: DspSettings,
    /// Device zones: open the device exclusively (bit-perfect).
    pub exclusive: bool,
    /// Server-driven zones: `None` uses the device's best mode.
    pub volume_mode: Option<VolumeMode>,
    /// Who carries out volume changes.
    pub volume_control: VolumeControl,
    /// Device setup: highest sample rate and bit depth to send.
    pub max_sample_rate: Option<u32>,
    pub max_bit_depth: Option<u8>,
    /// Device setup: how DSD reaches the device, and up to which rate.
    pub dsd_strategy: DsdStrategy,
    pub max_dsd_rate: Option<u32>,
}

impl Default for ZoneSettings {
    fn default() -> Self {
        Self {
            name: None,
            icon: None,
            crossfade_seconds: 0,
            leveling: LevelingMode::Off,
            leveling_target_lufs: DEFAULT_LEVELING_TARGET_LUFS,
            leveling_adjustment_db: 0,
            comfort_volume: None,
            safety_min_volume: None,
            safety_max_volume: None,
            private: false,
            dsp: DspSettings::default(),
            exclusive: true,
            volume_mode: None,
            volume_control: VolumeControl::Parson,
            max_sample_rate: None,
            max_bit_depth: None,
            dsd_strategy: DsdStrategy::Auto,
            max_dsd_rate: None,
        }
    }
}

impl ZoneSettings {
    pub fn limits(&self) -> VolumeLimits {
        VolumeLimits {
            comfort: self.comfort_volume,
            safety_min: self.safety_min_volume,
            safety_max: self.safety_max_volume,
        }
    }

    /// The device setup a device zone plays with.
    pub fn output(&self) -> parson_output::OutputSettings {
        parson_output::OutputSettings {
            exclusive: self.exclusive,
            max_sample_rate: self.max_sample_rate,
            max_bit_depth: self.max_bit_depth,
            dsd_strategy: self.dsd_strategy,
            max_dsd_rate: self.max_dsd_rate,
        }
    }
}

/// Distinguishes an absent field (`None`: unchanged) from `null`
/// (`Some(None)`: cleared).
fn nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// A settings change. Absent fields stay as they are, so a form that leaves
/// the name alone never resets it; only `reset_name` goes back to the
/// device's own name.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ZoneSettingsPatch {
    /// A new custom name (`null` or absent: unchanged).
    #[serde(default)]
    pub name: Option<String>,
    /// Go back to the device's own name.
    #[serde(default)]
    pub reset_name: bool,
    /// A custom icon; `null` goes back to the device's icon.
    #[serde(default, deserialize_with = "nullable")]
    pub icon: Option<Option<String>>,
    #[serde(default)]
    pub crossfade_seconds: Option<u8>,
    #[serde(default)]
    pub leveling: Option<LevelingMode>,
    #[serde(default)]
    pub leveling_target_lufs: Option<i8>,
    #[serde(default)]
    pub leveling_adjustment_db: Option<i8>,
    #[serde(default, deserialize_with = "nullable")]
    pub comfort_volume: Option<Option<u8>>,
    #[serde(default, deserialize_with = "nullable")]
    pub safety_min_volume: Option<Option<u8>>,
    #[serde(default, deserialize_with = "nullable")]
    pub safety_max_volume: Option<Option<u8>>,
    /// Older clients: the single volume cap, now the safety maximum.
    #[serde(default, deserialize_with = "nullable")]
    pub max_volume: Option<Option<u8>>,
    #[serde(default)]
    pub private: Option<bool>,
    /// The whole DSP chain (replaces it).
    #[serde(default)]
    pub dsp: Option<DspSettings>,
    #[serde(default)]
    pub exclusive: Option<bool>,
    #[serde(default)]
    pub volume_mode: Option<VolumeMode>,
    #[serde(default)]
    pub volume_control: Option<VolumeControl>,
    #[serde(default, deserialize_with = "nullable")]
    pub max_sample_rate: Option<Option<u32>>,
    #[serde(default, deserialize_with = "nullable")]
    pub max_bit_depth: Option<Option<u8>>,
    #[serde(default)]
    pub dsd_strategy: Option<DsdStrategy>,
    #[serde(default, deserialize_with = "nullable")]
    pub max_dsd_rate: Option<Option<u32>>,
}

impl ZoneSettingsPatch {
    fn touches_max_format(&self) -> bool {
        self.max_sample_rate.is_some() || self.max_bit_depth.is_some()
    }

    fn touches_dsd(&self) -> bool {
        self.dsd_strategy.is_some() || self.max_dsd_rate.is_some()
    }

    fn touches_volume_limits(&self) -> bool {
        self.comfort_volume.is_some()
            || self.safety_min_volume.is_some()
            || self.safety_max_volume.is_some()
            || self.max_volume.is_some()
    }

    fn touches_leveling(&self) -> bool {
        self.leveling.is_some()
            || self.leveling_target_lufs.is_some()
            || self.leveling_adjustment_db.is_some()
    }
}

/// Applies `patch` to `current`, validating every field. `default_name` is
/// the device's own name: choosing it again clears the custom name.
pub fn apply_settings_patch(
    current: &ZoneSettings,
    patch: &ZoneSettingsPatch,
    default_name: &str,
) -> Result<ZoneSettings, ZoneError> {
    let mut next = current.clone();
    if patch.reset_name && patch.name.is_some() {
        return Err(ZoneError::Invalid("name_and_reset_name_conflict"));
    }
    if patch.reset_name {
        next.name = None;
    } else if let Some(name) = &patch.name {
        let cleaned = clean_label(name, "");
        if cleaned.is_empty() {
            return Err(ZoneError::Invalid("zone_name_required"));
        }
        next.name = (cleaned != default_name).then_some(cleaned);
    }
    if let Some(icon) = &patch.icon {
        if let Some(icon) = icon
            && !ZONE_ICONS.contains(&icon.as_str())
        {
            return Err(ZoneError::Invalid("unknown_zone_icon"));
        }
        next.icon = icon.clone();
    }
    if let Some(seconds) = patch.crossfade_seconds {
        if seconds > CROSSFADE_MAX_SECONDS {
            return Err(ZoneError::Invalid("invalid_crossfade_seconds"));
        }
        next.crossfade_seconds = seconds;
    }
    if let Some(mode) = patch.leveling {
        next.leveling = mode;
    }
    if let Some(target) = patch.leveling_target_lufs {
        if !LEVELING_TARGET_RANGE.contains(&target) {
            return Err(ZoneError::Invalid("invalid_leveling_target"));
        }
        next.leveling_target_lufs = target;
    }
    if let Some(adjustment) = patch.leveling_adjustment_db {
        if !LEVELING_ADJUSTMENT_RANGE.contains(&adjustment) {
            return Err(ZoneError::Invalid("invalid_leveling_adjustment"));
        }
        next.leveling_adjustment_db = adjustment;
    }
    let volume = |value: Option<u8>| -> Result<Option<u8>, ZoneError> {
        match value {
            Some(level) if level > 100 => Err(ZoneError::Invalid("invalid_volume")),
            other => Ok(other),
        }
    };
    if let Some(comfort) = patch.comfort_volume {
        next.comfort_volume = volume(comfort)?;
    }
    if let Some(minimum) = patch.safety_min_volume {
        next.safety_min_volume = volume(minimum)?.filter(|level| *level > 0);
    }
    if let Some(maximum) = patch.safety_max_volume.or(patch.max_volume) {
        next.safety_max_volume = volume(maximum)?.filter(|level| *level < 100);
    }
    if next.safety_max_volume == Some(0) {
        return Err(ZoneError::Invalid("invalid_safety_limits"));
    }
    let limits = next.limits();
    if let (Some(minimum), Some(maximum)) = (next.safety_min_volume, next.safety_max_volume)
        && minimum >= maximum
    {
        return Err(ZoneError::Invalid("invalid_safety_limits"));
    }
    if let Some(comfort) = next.comfort_volume
        && (comfort < limits.min() || comfort > limits.max())
    {
        return Err(ZoneError::Invalid("comfort_outside_safety_limits"));
    }
    if let Some(private) = patch.private {
        next.private = private;
    }
    if let Some(dsp) = &patch.dsp {
        next.dsp = dsp.clone().normalized();
    }
    if let Some(exclusive) = patch.exclusive {
        next.exclusive = exclusive;
    }
    if let Some(mode) = patch.volume_mode {
        next.volume_mode = Some(mode);
    }
    if let Some(control) = patch.volume_control {
        next.volume_control = control;
    }
    if let Some(rate) = patch.max_sample_rate {
        if let Some(rate) = rate
            && !SAMPLE_RATE_CHOICES.contains(&rate)
        {
            return Err(ZoneError::Invalid("invalid_max_sample_rate"));
        }
        next.max_sample_rate = rate;
    }
    if let Some(depth) = patch.max_bit_depth {
        if let Some(depth) = depth
            && !BIT_DEPTH_CHOICES.contains(&depth)
        {
            return Err(ZoneError::Invalid("invalid_max_bit_depth"));
        }
        next.max_bit_depth = depth;
    }
    if let Some(strategy) = patch.dsd_strategy {
        next.dsd_strategy = strategy;
    }
    if let Some(rate) = patch.max_dsd_rate {
        if let Some(rate) = rate
            && !DSD_RATE_CHOICES.contains(&rate)
        {
            return Err(ZoneError::Invalid("invalid_max_dsd_rate"));
        }
        next.max_dsd_rate = rate;
    }
    Ok(next)
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
    repeat: RepeatMode,
    shuffle: bool,
}

struct SonosQueue {
    /// Coordinator uuid at load time; part of the media URL signature.
    media_key: String,
    song_ids: Vec<String>,
    /// Index into `song_ids` of the first track in the Sonos queue.
    window_start: usize,
    window_len: usize,
    /// Parson shuffles the queue itself so both sides agree on the order.
    shuffle: bool,
    /// Unshuffled order while `shuffle` is on.
    original: Option<Vec<String>>,
}

struct LinkedGroup {
    user_id: i32,
    members: Vec<String>,
}

struct SleepTimer {
    /// Who set the timer. Sonos rooms are shared by every account on the
    /// server, so the owner cannot be derived from the zone.
    user_id: i32,
    ends_at_ms: i64,
    finishing_track: bool,
    /// Queue position of the track being allowed to finish.
    finishing_position: Option<usize>,
    fading: bool,
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
    /// Queues of server-driven zones (UPnP renderers, audio devices).
    hosted: HashMap<String, hosted::HostedQueue>,
    /// Last published view of each server-driven zone, to notice changes.
    hosted_seen: HashMap<String, String>,
    /// Server-driven zones switched off with `standby`.
    standby: std::collections::HashSet<String>,
    /// Running volume estimate (level, muted) of zones whose volume Home
    /// Assistant carries out.
    delegated_volume: HashMap<String, (u8, bool)>,
}

/// Server-side zones.
pub struct ZoneService {
    state: Mutex<State>,
    events: broadcast::Sender<ZoneEvent>,
    sonos: Option<Arc<SonosController>>,
    pool: Option<DbPool>,
    port: AtomicU16,
    /// How long a sleep timer fades out before pausing.
    sleep_fade: Mutex<Duration>,
    /// Measured track loudness for volume leveling.
    loudness: Arc<LoudnessStore>,
    /// UPnP renderers and local audio devices.
    outputs: OnceLock<Arc<crate::outputs::Outputs>>,
    /// The library, for server-driven zones that move on by themselves.
    library: OnceLock<Arc<crate::library::state::LibraryLifecycle>>,
    track_serial: AtomicU64,
}

/// Fade-out length when a sleep timer pauses mid-track.
pub const SLEEP_FADE: Duration = Duration::from_secs(8);
const SLEEP_FADE_STEPS: u32 = 16;

pub(crate) fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Who is asking: the app's own zone device (header) and whether the
/// request comes from this computer (loopback).
#[derive(Clone, Copy, Debug, Default)]
pub struct Viewer<'a> {
    pub device: Option<&'a str>,
    pub local: bool,
}

impl<'a> Viewer<'a> {
    pub fn device(device: Option<&'a str>) -> Self {
        Self {
            device,
            local: false,
        }
    }

    /// The viewer is the zone's own device: the app itself, or for an
    /// audio-device zone the computer the device is attached to.
    pub fn owns(&self, zone_id: &str) -> bool {
        if zone_id.starts_with("device:") {
            return self.local;
        }
        self.device.and_then(client_zone_id).as_deref() == Some(zone_id)
    }
}

impl<'a> From<Option<&'a str>> for Viewer<'a> {
    fn from(device: Option<&'a str>) -> Self {
        Self::device(device)
    }
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
            loudness: Arc::new(LoudnessStore::new(pool.clone())),
            pool,
            port: AtomicU16::new(crate::settings::DEFAULT_PORT),
            sleep_fade: Mutex::new(SLEEP_FADE),
            outputs: OnceLock::new(),
            library: OnceLock::new(),
            track_serial: AtomicU64::new(1),
        }
    }

    /// Shortens the sleep fade (tests).
    pub fn set_sleep_fade(&self, fade: Duration) {
        *self
            .sleep_fade
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = fade;
    }

    fn sleep_fade(&self) -> Duration {
        *self
            .sleep_fade
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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

    pub fn loudness(&self) -> &Arc<LoudnessStore> {
        &self.loudness
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
            Ok(mut settings) => {
                import_legacy_outputs(&mut connection, &mut settings);
                self.lock().settings = settings;
            }
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
                    repeat: RepeatMode::Off,
                    shuffle: false,
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
                || zone.repeat != report.repeat
                || zone.shuffle != report.shuffle
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
            zone.repeat = report.repeat;
            zone.shuffle = report.shuffle;
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

    /// Zones `user_id` sees from the device `viewer` (when the request names
    /// one). Private zones only show on their own device.
    pub fn list_for(
        &self,
        user_id: i32,
        viewer: Viewer<'_>,
        cache: Option<&LibraryCache>,
    ) -> Vec<Zone> {
        let mut zones = self.list(user_id, cache);
        zones.retain(|zone| self.visible_to(&zone.id, viewer));
        zones
    }

    /// Whether `viewer` may see and control `zone_id`. A private app zone
    /// shows only on its own device; a private audio-device zone only on
    /// the computer it is attached to.
    pub fn visible_to(&self, zone_id: &str, viewer: Viewer<'_>) -> bool {
        let private = self
            .lock()
            .settings
            .get(zone_id)
            .is_some_and(|settings| settings.private);
        !private || viewer.owns(zone_id)
    }

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
        zones.extend(self.hosted_views(&state, cache, now));
        for zone in &mut zones {
            apply_standby_and_volume_control(&state, zone);
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
        fill_endpoints(&mut zones);
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
        let app_player = zone.kind != ZoneKind::Mobile;
        let mut setting_notes = Vec::new();
        if !app_player {
            setting_notes.push(
                "Crossfade and volume leveling run in the Mujin Music web and desktop apps; the phone app plays tracks back to back at their own level."
                    .to_string(),
            );
        }
        Zone {
            id: id.to_string(),
            kind: zone.kind,
            ecosystem: zone.kind.ecosystem(),
            name: settings.name.clone().unwrap_or_else(|| zone.name.clone()),
            icon: zone_icon(&settings, zone.kind),
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
            repeat: zone.repeat,
            shuffle: zone.shuffle,
            position_ms: extrapolate_position(
                zone.position_ms,
                zone.reported_at_ms,
                now,
                zone.status == ZoneStatus::Playing,
                zone.duration_ms,
            ),
            duration_ms: zone.duration_ms,
            volume: Some(ZoneVolume::new(zone.volume, zone.muted, settings.limits())),
            group: None,
            grouped_into: None,
            capabilities: ZoneCapabilities {
                transport: true,
                seek: true,
                volume: true,
                mute: true,
                queue: true,
                queue_edit: true,
                repeat: true,
                shuffle: true,
                group: !settings.private,
                transfer_target: true,
                sleep_timer: true,
                standby: false,
            },
            sleep_timer: sleep_view(state, id),
            settings: settings_view(
                &settings,
                &zone.name,
                ZoneSettingsSupport {
                    crossfade: app_player,
                    leveling: app_player,
                    volume_limits: true,
                    private: true,
                    dsp: app_player,
                    ..ZoneSettingsSupport::default()
                },
                setting_notes,
            ),
            output: None,
            endpoints: Vec::new(),
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
                        .map(ZoneSettings::limits)
                        .unwrap_or_default(),
                )),
            })
            .collect();
        let (repeat, sonos_shuffles) = RepeatMode::from_sonos_play_mode(&group.play_mode);
        let mut notes = vec![sonos_format_note()];
        if let Some(queue) = queue
            && queue.song_ids.len() > queue.window_len
        {
            notes.push(format!(
                "Sonos holds {} of the {} queued tracks at a time; Mujin Music loads the next ones as playback moves on.",
                queue.window_len,
                queue.song_ids.len()
            ));
        }
        if source == ZoneSource::External {
            notes.push(
                "Another app is playing on this Sonos zone. Playing from Mujin Music replaces it."
                    .to_string(),
            );
        }
        // The queued track's own length: the speaker's TrackDuration lags
        // a track change until the next poll.
        let duration_ms = match source {
            ZoneSource::None => 0,
            ZoneSource::Parson => now_playing
                .as_ref()
                .map(|now| now.duration_ms)
                .filter(|duration| *duration > 0)
                .unwrap_or(group.duration_ms),
            ZoneSource::External => group.duration_ms,
        };
        // The path of the track playing now, from the plan the signed media
        // URL follows; rebuilt with every view, so it follows track changes.
        let stream = queue
            .zip(cache)
            .filter(|_| source == ZoneSource::Parson)
            .and_then(|(queue, cache)| {
                let song_id = queue.song_ids.get(current_position)?;
                let song = cache.song(song_id)?;
                let players: Vec<_> = group
                    .members
                    .iter()
                    .filter_map(|member| sonos.player(member))
                    .collect();
                let rooms = players
                    .iter()
                    .map(|player| player.room_name.as_str())
                    .collect::<Vec<_>>()
                    .join(" + ");
                let mut models: Vec<&str> = players
                    .iter()
                    .map(|player| player.model_name.as_str())
                    .collect();
                models.dedup();
                let target = if players.is_empty() {
                    group.name.clone()
                } else {
                    format!("{rooms} ({})", models.join(", "))
                };
                let leveling = sonos_leveling(&settings, &queue.song_ids, current_position, cache);
                Some(hosted::served_stream_info(
                    song,
                    crate::outputs::FormatLimits::SONOS,
                    hosted::cached_leveling_gain(&self.loudness, cache, song_id, leveling),
                    &DspSettings::default(),
                    &target,
                ))
            });
        let mut setting_notes = vec![
            "Sonos crossfades with its own fixed length: any crossfade time above 0 s turns Sonos crossfade on.".to_string(),
        ];
        if settings.leveling != LevelingMode::Off {
            setting_notes.push(
                "Mujin Music levels Sonos in the audio it serves (EBU R128, peaks kept under -1 dBTP), starting with the next track.".to_string(),
            );
        }
        Zone {
            id,
            kind: ZoneKind::Sonos,
            ecosystem: "sonos",
            name: settings.name.clone().unwrap_or_else(|| group.name.clone()),
            icon: zone_icon(&settings, ZoneKind::Sonos),
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
            repeat,
            shuffle: queue.is_some_and(|queue| queue.shuffle) || sonos_shuffles,
            position_ms: extrapolate_position(
                group.position_ms,
                group.polled_at_ms,
                now,
                group.transport == TransportState::Playing,
                duration_ms,
            ),
            duration_ms,
            volume: Some(ZoneVolume::new(
                group.group_volume,
                group.group_muted,
                settings.limits(),
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
                queue_edit: source == ZoneSource::Parson,
                repeat: source != ZoneSource::None,
                shuffle: source == ZoneSource::Parson,
                group: true,
                transfer_target: true,
                sleep_timer: true,
                standby: false,
            },
            sleep_timer: sleep_view(state, &sonos_zone_id(&group.coordinator)),
            settings: settings_view(
                &settings,
                &group.name,
                ZoneSettingsSupport {
                    crossfade: true,
                    leveling: true,
                    volume_limits: true,
                    private: false,
                    ..ZoneSettingsSupport::default()
                },
                setting_notes,
            ),
            output: Some(ZoneOutputInfo {
                max_sample_rate: crate::outputs::FormatLimits::SONOS.max_sample_rate,
                max_bit_depth: crate::outputs::FormatLimits::SONOS.max_bit_depth,
                formats: crate::outputs::FormatLimits::SONOS.formats(),
                note: sonos_format_note(),
                exclusive: None,
                volume_mode: None,
                stream,
            }),
            endpoints: Vec::new(),
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
        // Adding to a zone that has nothing queued starts the songs there,
        // as in Roon.
        let command = match command {
            ZoneCommand::AddToQueue { song_ids, .. }
                if self
                    .zone(user_id, zone_id, cache)
                    .is_some_and(|zone| zone.queue.song_ids.is_empty()) =>
            {
                ZoneCommand::Load {
                    song_ids,
                    current_position: 0,
                    position_ms: 0,
                    play: true,
                }
            }
            command => command,
        };
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
            // The group's own limits apply before each member's.
            let command = match command {
                ZoneCommand::SetVolume {
                    volume,
                    member_id: None,
                    above_comfort,
                } => ZoneCommand::SetVolume {
                    volume: self
                        .limits(group_id)
                        .clamp(i16::from(volume), false, above_comfort),
                    member_id: None,
                    above_comfort,
                },
                other => other,
            };
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
        if let Some(result) = self.delegated_volume_command(user_id, zone_id, &command, cache) {
            return result;
        }
        if matches!(command, ZoneCommand::Standby | ZoneCommand::Wake) && !hosted_zone_id(zone_id) {
            return Err(ZoneError::Unsupported("zone_has_no_standby"));
        }
        if let Some(uuid) = zone_id.strip_prefix("sonos:") {
            return self.sonos_command(user_id, uuid, command, cache).await;
        }
        if hosted_zone_id(zone_id) {
            return self.hosted_command(zone_id, command, cache).await;
        }
        if zone_id.starts_with("client:") {
            let command = self.clamp_client_command(user_id, zone_id, command)?;
            self.preview_client_edit(user_id, zone_id, &command)?;
            return self.queue_client_command(user_id, zone_id, command);
        }
        Err(ZoneError::NotFound)
    }

    /// Carries out a volume command for a zone whose volume Home Assistant
    /// controls: Parson only updates its running estimate and sends the
    /// request out as an event. `None` when the zone (or the group member
    /// the command names) keeps its own volume control.
    fn delegated_volume_command(
        &self,
        user_id: i32,
        zone_id: &str,
        command: &ZoneCommand,
        cache: Option<&LibraryCache>,
    ) -> Option<Result<(), ZoneError>> {
        let target = command.member_id().unwrap_or(zone_id);
        let (limits, control) = {
            let state = self.lock();
            let settings = state.settings.get(target)?;
            (settings.limits(), settings.volume_control)
        };
        if control == VolumeControl::Parson
            || !matches!(
                command,
                ZoneCommand::SetVolume { .. }
                    | ZoneCommand::AdjustVolume { .. }
                    | ZoneCommand::SetMute { .. }
            )
        {
            return None;
        }
        let known = self.lock().delegated_volume.get(target).copied();
        let (level, muted) = match known {
            Some(current) => current,
            None => match self
                .list(user_id, cache)
                .into_iter()
                .find(|zone| zone.id == target)
            {
                Some(zone) => zone
                    .volume
                    .map_or((limits.clamp(50, false, false), false), |volume| {
                        (volume.level, volume.muted)
                    }),
                None => return Some(Err(ZoneError::NotFound)),
            },
        };
        let (action, step, level, muted) = match *command {
            ZoneCommand::SetVolume {
                volume,
                above_comfort,
                ..
            } => (
                "volume_set",
                0,
                limits.clamp(i16::from(volume), false, above_comfort),
                muted,
            ),
            ZoneCommand::AdjustVolume { delta, .. } => (
                if delta >= 0 {
                    "volume_up"
                } else {
                    "volume_down"
                },
                delta.unsigned_abs(),
                limits.clamp(i16::from(level) + i16::from(delta), true, true),
                muted,
            ),
            ZoneCommand::SetMute { muted, .. } => {
                (if muted { "mute_on" } else { "mute_off" }, 0, level, muted)
            }
            _ => return None,
        };
        self.lock()
            .delegated_volume
            .insert(target.to_string(), (level, muted));
        // App zones belong to one account; rooms and devices are shared.
        let scope = (!shared_zone(target)).then_some(user_id);
        let _ = self.events.send(ZoneEvent::VolumeRequest {
            user_id: scope,
            request: VolumeRequest {
                zone_id: target.to_string(),
                action,
                step,
                level,
                muted,
            },
        });
        self.changed(scope);
        Some(Ok(()))
    }

    /// Checks a queue edit against the client's last reported queue and
    /// shows it right away, so every remote sees the change before the
    /// client's next report. The client re-sends its real queue afterwards.
    fn preview_client_edit(
        &self,
        user_id: i32,
        zone_id: &str,
        command: &ZoneCommand,
    ) -> Result<(), ZoneError> {
        let is_mode = matches!(
            command,
            ZoneCommand::SetRepeat { .. } | ZoneCommand::SetShuffle { .. }
        );
        if !command.is_queue_edit() && !is_mode {
            return Ok(());
        }
        {
            let mut state = self.lock();
            let zone = state
                .clients
                .get_mut(zone_id)
                .filter(|zone| zone.user_id == user_id)
                .ok_or(ZoneError::NotFound)?;
            match command {
                ZoneCommand::SetRepeat { mode } => zone.repeat = *mode,
                ZoneCommand::SetShuffle { enabled } => {
                    // The client shuffles with its own order; it reports back.
                    zone.shuffle = *enabled;
                    zone.has_queue = false;
                }
                _ => {
                    let mut song_ids = zone.song_ids.clone();
                    let mut shuffle = zone.shuffle;
                    let mut original = None;
                    apply_queue_edit(
                        &mut song_ids,
                        zone.current_position,
                        &mut shuffle,
                        &mut original,
                        command,
                        &mut |_| 0,
                    )?;
                    zone.song_ids = song_ids;
                    zone.has_queue = false;
                }
            }
        }
        self.changed(Some(user_id));
        Ok(())
    }

    fn limits(&self, zone_id: &str) -> VolumeLimits {
        self.lock()
            .settings
            .get(zone_id)
            .map(ZoneSettings::limits)
            .unwrap_or_default()
    }

    fn clamp_client_command(
        &self,
        user_id: i32,
        zone_id: &str,
        command: ZoneCommand,
    ) -> Result<ZoneCommand, ZoneError> {
        let limits = self.limits(zone_id);
        Ok(match command {
            ZoneCommand::SetVolume {
                volume,
                above_comfort,
                ..
            } => ZoneCommand::SetVolume {
                volume: limits.clamp(i16::from(volume), false, above_comfort),
                member_id: None,
                above_comfort: false,
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
                    volume: limits.clamp(i16::from(current) + i16::from(delta), true, true),
                    member_id: None,
                    above_comfort: false,
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
            ZoneCommand::Standby | ZoneCommand::Wake => {
                Err(ZoneError::Unsupported("zone_has_no_standby"))
            }
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
            ZoneCommand::SetVolume {
                volume,
                member_id,
                above_comfort,
            } => match member_id {
                Some(member) => {
                    let member_uuid = member
                        .strip_prefix("sonos:")
                        .filter(|member| group.members.iter().any(|m| m == member))
                        .ok_or(ZoneError::Invalid("member_not_in_group"))?;
                    let volume =
                        self.limits(&member)
                            .clamp(i16::from(volume), false, above_comfort);
                    sonos
                        .set_player_volume(member_uuid, volume)
                        .await
                        .map_err(device)
                }
                None => {
                    let volume =
                        self.limits(&zone_id)
                            .clamp(i16::from(volume), false, above_comfort);
                    sonos
                        .set_group_volume(&coordinator, volume)
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
                let volume = self.limits(&target_zone).clamp(
                    i16::from(current) + i16::from(delta),
                    true,
                    true,
                );
                Box::pin(self.sonos_command(
                    user_id,
                    uuid,
                    ZoneCommand::SetVolume {
                        volume,
                        member_id,
                        above_comfort: true,
                    },
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
            ZoneCommand::SetRepeat { mode } => sonos
                .set_play_mode(&coordinator, mode.sonos_play_mode())
                .await
                .map_err(device),
            command @ (ZoneCommand::MoveQueueItem { .. }
            | ZoneCommand::RemoveQueueItem { .. }
            | ZoneCommand::ClearUpcoming
            | ZoneCommand::AddToQueue { .. }
            | ZoneCommand::SetShuffle { .. }) => {
                let cache = cache.ok_or(ZoneError::Unsupported("library_unavailable"))?;
                self.sonos_queue_edit(&sonos, &coordinator, &command, cache)
                    .await
            }
        }
    }

    /// Edits the Parson queue mapped onto a Sonos group and mirrors the
    /// change on the speaker's queue without interrupting the current
    /// track: native reorder/remove inside the mapped window, otherwise the
    /// upcoming part of the Sonos queue is rewritten.
    async fn sonos_queue_edit(
        &self,
        sonos: &SonosController,
        coordinator: &str,
        command: &ZoneCommand,
        cache: &LibraryCache,
    ) -> Result<(), ZoneError> {
        let device = |error: crate::sonos::soap::SoapError| ZoneError::Device(error.to_string());
        // Act on the speaker's real position, not a stale poll.
        let _ = sonos.poll_group(coordinator).await;
        let group = sonos.group_of(coordinator).ok_or(ZoneError::NotFound)?;
        if !group.track_uri.contains(ZONE_MEDIA_PATH) {
            return Err(ZoneError::Unsupported("zone_has_no_parson_queue"));
        }
        let track_index = group.track_number.saturating_sub(1) as usize;
        let (edit, song_ids, window_start, window_len, current) = {
            let mut state = self.lock();
            let queue = state
                .sonos_queues
                .get_mut(coordinator)
                .ok_or(ZoneError::Unsupported("zone_has_no_parson_queue"))?;
            let current =
                (queue.window_start + track_index).min(queue.song_ids.len().saturating_sub(1));
            let mut song_ids = queue.song_ids.clone();
            let mut shuffle = queue.shuffle;
            let mut original = queue.original.clone();
            let edit = apply_queue_edit(
                &mut song_ids,
                current,
                &mut shuffle,
                &mut original,
                command,
                &mut |bound| rand::random_range(0..bound.max(1)),
            )?;
            if edit == QueueEdit::Unchanged {
                return Ok(());
            }
            queue.shuffle = shuffle;
            queue.original = original;
            (
                edit,
                song_ids,
                queue.window_start,
                queue.window_len,
                current,
            )
        };
        let in_window =
            |position: usize| position >= window_start && position < window_start + window_len;
        let relative = |position: usize| (position - window_start) as u32;
        let mut new_len = window_len;
        // Record the new order first so the speaker may fetch new tracks.
        let commit = |len: usize| {
            let mut state = self.lock();
            if let Some(queue) = state.sonos_queues.get_mut(coordinator) {
                queue.song_ids = song_ids.clone();
                queue.window_len = len;
            }
        };
        let result: Result<(), ZoneError> = async {
            match edit {
                QueueEdit::Moved { from, to } if in_window(from) && in_window(to) => {
                    commit(window_len);
                    sonos
                        .move_track(coordinator, relative(from), relative(to))
                        .await
                        .map_err(device)
                }
                QueueEdit::Removed { position } => {
                    if in_window(position) {
                        new_len -= 1;
                        commit(new_len);
                        sonos
                            .remove_track(coordinator, relative(position))
                            .await
                            .map_err(device)?;
                    } else {
                        commit(new_len);
                    }
                    // Top the window back up from the rest of the queue.
                    let end = (window_start + MAX_SONOS_QUEUE_ITEMS).min(song_ids.len());
                    let from = window_start + new_len;
                    if from < end {
                        let tracks =
                            self.sonos_tracks(sonos, coordinator, &song_ids, from..end, cache)?;
                        new_len += tracks.len();
                        commit(new_len);
                        sonos
                            .append_tracks(coordinator, &tracks)
                            .await
                            .map_err(device)?;
                    }
                    Ok(())
                }
                QueueEdit::Inserted { position, count }
                    if position == window_start + window_len
                        && window_len + count <= MAX_SONOS_QUEUE_ITEMS =>
                {
                    // Added at the end of what the speaker holds: append
                    // natively (AddURIToQueue), nothing else moves.
                    let tracks = self.sonos_tracks(
                        sonos,
                        coordinator,
                        &song_ids,
                        position..position + count,
                        cache,
                    )?;
                    new_len = window_len + tracks.len();
                    commit(new_len);
                    sonos
                        .append_tracks(coordinator, &tracks)
                        .await
                        .map_err(device)
                }
                QueueEdit::Inserted { position, .. } if position > window_start + window_len => {
                    // Beyond the speaker's window: it reaches them later.
                    commit(window_len);
                    Ok(())
                }
                QueueEdit::Moved { .. }
                | QueueEdit::Inserted { .. }
                | QueueEdit::UpcomingReplaced => {
                    let kept = current + 1 - window_start;
                    let end = (window_start + MAX_SONOS_QUEUE_ITEMS).min(song_ids.len());
                    let tracks = self.sonos_tracks(
                        sonos,
                        coordinator,
                        &song_ids,
                        (current + 1).min(end)..end,
                        cache,
                    )?;
                    new_len = kept + tracks.len();
                    commit(new_len);
                    sonos
                        .remove_track_range(coordinator, kept as u32, (window_len - kept) as u32)
                        .await
                        .map_err(device)?;
                    sonos
                        .append_tracks(coordinator, &tracks)
                        .await
                        .map_err(device)
                }
                QueueEdit::Unchanged => Ok(()),
            }
        }
        .await;
        if matches!(command, ZoneCommand::SetShuffle { .. })
            && RepeatMode::from_sonos_play_mode(&group.play_mode).1
        {
            // Parson owns the order now; stop Sonos shuffling on top of it.
            let (repeat, _) = RepeatMode::from_sonos_play_mode(&group.play_mode);
            let _ = sonos
                .set_play_mode(coordinator, repeat.sonos_play_mode())
                .await;
        }
        if let Err(error) = result {
            // The speaker's queue no longer matches; load the edited queue.
            tracing::warn!(%error, coordinator, "Sonos queue edit failed; reloading the queue");
            let playing = group.transport == TransportState::Playing;
            let position_ms = extrapolate_position(
                group.position_ms,
                group.polled_at_ms,
                now_ms(),
                playing,
                group.duration_ms,
            );
            self.load_sonos(
                sonos,
                coordinator,
                song_ids,
                current,
                position_ms,
                playing,
                cache,
            )
            .await?;
        }
        let _ = sonos.poll_group(coordinator).await;
        self.changed(None);
        Ok(())
    }

    /// Signed URLs and DIDL-Lite for `queue[range]` on a Sonos group, with
    /// the zone's volume leveling in each URL.
    fn sonos_tracks(
        &self,
        sonos: &SonosController,
        coordinator: &str,
        queue: &[String],
        range: std::ops::Range<usize>,
        cache: &LibraryCache,
    ) -> Result<Vec<(String, String)>, ZoneError> {
        if range.is_empty() {
            return Ok(Vec::new());
        }
        let origin = sonos
            .local_origin_for(coordinator, self.port())
            .ok_or_else(|| ZoneError::Device("no network route to the Sonos player".into()))?;
        let expires = chrono::Utc::now().timestamp() + MEDIA_URL_LIFETIME_SECONDS;
        let (media_key, settings) = {
            let state = self.lock();
            (
                state
                    .sonos_queues
                    .get(coordinator)
                    .map(|queue| queue.media_key.clone())
                    .unwrap_or_else(|| coordinator.to_string()),
                state
                    .settings
                    .get(&sonos_zone_id(coordinator))
                    .cloned()
                    .unwrap_or_default(),
            )
        };
        range
            .map(|index| {
                let song_id = queue
                    .get(index)
                    .ok_or(ZoneError::Invalid("invalid_zone_queue"))?;
                let leveling = sonos_leveling(&settings, queue, index, cache);
                sonos_track(cache, &origin, &media_key, song_id, expires, leveling)
                    .ok_or(ZoneError::Invalid("invalid_zone_queue"))
            })
            .collect()
    }

    /// Re-sends the upcoming part of a Sonos queue (new leveling) without
    /// touching the playing track.
    async fn refresh_sonos_upcoming(
        &self,
        sonos: &SonosController,
        coordinator: &str,
        cache: &LibraryCache,
    ) -> Result<(), ZoneError> {
        let device = |error: crate::sonos::soap::SoapError| ZoneError::Device(error.to_string());
        let _ = sonos.poll_group(coordinator).await;
        let group = sonos.group_of(coordinator).ok_or(ZoneError::NotFound)?;
        if !group.track_uri.contains(ZONE_MEDIA_PATH) {
            return Ok(());
        }
        let Some((song_ids, window_start, window_len)) = ({
            let state = self.lock();
            state
                .sonos_queues
                .get(coordinator)
                .map(|queue| (queue.song_ids.clone(), queue.window_start, queue.window_len))
        }) else {
            return Ok(());
        };
        let kept = (group.track_number as usize).clamp(1, window_len.max(1));
        let end = window_start + window_len;
        let tracks = self.sonos_tracks(
            sonos,
            coordinator,
            &song_ids,
            window_start + kept..end,
            cache,
        )?;
        if tracks.is_empty() {
            return Ok(());
        }
        sonos
            .remove_track_range(coordinator, kept as u32, (window_len - kept) as u32)
            .await
            .map_err(device)?;
        sonos
            .append_tracks(coordinator, &tracks)
            .await
            .map_err(device)
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
        let settings = self
            .lock()
            .settings
            .get(&sonos_zone_id(coordinator))
            .cloned()
            .unwrap_or_default();
        let tracks: Vec<(String, String)> = (window_start..window_start + window_len)
            .filter_map(|index| {
                let leveling = sonos_leveling(&settings, &song_ids, index, cache);
                sonos_track(
                    cache,
                    &origin,
                    coordinator,
                    &song_ids[index],
                    expires,
                    leveling,
                )
            })
            .collect();
        if tracks.len() != window_len {
            return Err(ZoneError::Invalid("invalid_zone_queue"));
        }
        if settings.leveling != LevelingMode::Off {
            // Measure ahead so the speaker does not wait when it fetches.
            self.loudness.measure_in_background(
                song_ids[window_start..window_start + window_len]
                    .iter()
                    .filter_map(|id| cache.song(id).map(|song| (id.clone(), song.path.clone())))
                    .collect(),
            );
        }
        // A window reload of the same queue keeps its shuffle state.
        let (shuffle, original) = {
            let state = self.lock();
            match state.sonos_queues.get(coordinator) {
                Some(queue) if queue.song_ids == song_ids => {
                    (queue.shuffle, queue.original.clone())
                }
                _ => (false, None),
            }
        };
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
                    shuffle,
                    original,
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
        if loaded.is_ok()
            && settings.crossfade_seconds > 0
            && let Err(error) = sonos.set_crossfade(coordinator, true).await
        {
            tracing::warn!(%error, coordinator, "could not turn on Sonos crossfade");
        }
        self.changed(None);
        loaded.map_err(|error| ZoneError::Device(error.to_string()))
    }

    /// The leveling gain for a signed Sonos media request, measuring the
    /// track's loudness first when needed.
    pub async fn stream_gain(
        &self,
        cache: &LibraryCache,
        song_id: &str,
        leveling: StreamLeveling,
    ) -> Option<f32> {
        self.loudness
            .gain_for(
                cache,
                song_id,
                leveling.by,
                leveling.target_lufs,
                leveling.adjustment_db,
            )
            .await
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

    /// Pauses every zone `viewer` can see that is playing (Roon's
    /// "Pause all"). Returns how many zones were paused.
    pub async fn pause_all(
        &self,
        user_id: i32,
        viewer: Viewer<'_>,
        cache: Option<&LibraryCache>,
    ) -> usize {
        let zones = self.list_for(user_id, viewer, cache);
        let mut paused = 0;
        for zone in zones {
            // Members of a linked group pause through the group.
            if zone.grouped_into.is_some()
                || !zone.capabilities.transport
                || !zone.status.is_playing()
            {
                continue;
            }
            match self
                .command(user_id, &zone.id, ZoneCommand::Pause, cache)
                .await
            {
                Ok(()) => paused += 1,
                Err(error) => {
                    tracing::warn!(%error, zone = zone.id, "pause all could not pause a zone")
                }
            }
        }
        paused
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
            if zone.settings.private {
                return Err(ZoneError::Unsupported("private_zones_cannot_be_grouped"));
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
        if unique
            .iter()
            .all(|id| id.starts_with(crate::outputs::bridge::BRIDGE_PREFIX))
        {
            return self.group_bridges(user_id, &unique, &zones).await;
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

    /// Groups Parson Bridge zones into one sample-synced zone that carries
    /// on with the first zone's queue.
    async fn group_bridges(
        &self,
        user_id: i32,
        members: &[String],
        zones: &[Zone],
    ) -> Result<String, ZoneError> {
        let hub = self
            .outputs
            .get()
            .and_then(|outputs| outputs.bridges.clone())
            .ok_or(ZoneError::Unsupported("bridges_disabled"))?;
        let snapshot = self.snapshot(user_id, &members[0]).ok();
        let picked: Vec<&Zone> = members
            .iter()
            .filter_map(|id| zones.iter().find(|zone| &zone.id == id))
            .collect();
        let mut names: Vec<String> = picked.iter().map(|zone| zone.name.clone()).collect();
        let duplicates = (1..names.len()).any(|index| names[..index].contains(&names[index]));
        if duplicates {
            // Same DAC model in two rooms: name the group by machine.
            names = picked
                .iter()
                .map(|zone| {
                    zone.device
                        .split(" · ")
                        .next()
                        .unwrap_or(&zone.name)
                        .to_string()
                })
                .collect();
        }
        let name = names.join(" + ");
        for member in members {
            let _ = self.command(user_id, member, ZoneCommand::Stop, None).await;
        }
        for member in members {
            wait_until_released(hub.renderer(member)).await;
        }
        let master = picked
            .first()
            .map(|zone| zone.name.clone())
            .unwrap_or_else(|| "the first zone".to_string());
        let group_id = hub
            .create_group(members, &name, &master)
            .map_err(ZoneError::Device)?;
        if let Some(snapshot) = snapshot {
            self.load_snapshot(user_id, &group_id, snapshot).await?;
        }
        self.changed(None);
        Ok(group_id)
    }

    async fn load_snapshot(
        &self,
        user_id: i32,
        zone_id: &str,
        snapshot: PlaybackSnapshot,
    ) -> Result<(), ZoneError> {
        self.command(
            user_id,
            zone_id,
            ZoneCommand::Load {
                song_ids: snapshot.song_ids,
                current_position: snapshot.current_position,
                position_ms: snapshot.position_ms,
                play: snapshot.playing,
            },
            None,
        )
        .await
    }

    /// Ends a bridge group (its queue carries on in the first zone) or
    /// takes one zone out of it.
    async fn ungroup_bridges(
        &self,
        user_id: i32,
        group_id: &str,
        member: Option<&str>,
    ) -> Result<(), ZoneError> {
        let hub = self
            .outputs
            .get()
            .and_then(|outputs| outputs.bridges.clone())
            .ok_or(ZoneError::Unsupported("bridges_disabled"))?;
        let members = hub.group_members(group_id).ok_or(ZoneError::NotFound)?;
        let snapshot = self.snapshot(user_id, group_id).ok();
        let _ = self
            .command(user_id, group_id, ZoneCommand::Stop, None)
            .await;
        // The group lets go of every member's DAC before a member opens it
        // again; otherwise the two streams fight over the device.
        wait_until_released(hub.renderer(group_id)).await;
        let freed = hub.leave_group(group_id, member).map_err(|error| {
            if error == "member_not_in_group" {
                ZoneError::Invalid("member_not_in_group")
            } else {
                ZoneError::Device(error)
            }
        })?;
        let group_remains = hub.group_members(group_id).is_some();
        if !group_remains {
            self.lock().hosted.remove(group_id);
        }
        if let Some(snapshot) = snapshot {
            let target = if group_remains {
                group_id.to_string()
            } else {
                members
                    .iter()
                    .find(|id| freed.contains(id))
                    .cloned()
                    .unwrap_or_else(|| members[0].clone())
            };
            self.load_snapshot(user_id, &target, snapshot).await?;
        }
        self.changed(None);
        Ok(())
    }

    /// Ungroups a linked or Sonos group, or removes one member when given.
    pub async fn ungroup(
        &self,
        user_id: i32,
        group_id: &str,
        member: Option<&str>,
    ) -> Result<(), ZoneError> {
        if group_id.starts_with(crate::outputs::bridge::GROUP_PREFIX) {
            return self.ungroup_bridges(user_id, group_id, member).await;
        }
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

    /// Changes a zone's settings. Only the fields in `patch` change. A zone
    /// is made private from its own device (`viewer`); tightened volume
    /// limits apply to the current volume at once, and Sonos crossfade and
    /// leveling changes reach the speaker (leveling from the next track).
    pub async fn update_settings(
        &self,
        user_id: i32,
        zone_id: &str,
        patch: ZoneSettingsPatch,
        viewer: Viewer<'_>,
        cache: Option<&LibraryCache>,
    ) -> Result<ZoneSettings, ZoneError> {
        if !self.visible_to(zone_id, viewer) {
            return Err(ZoneError::NotFound);
        }
        let (default_name, supports) = match self.zone(user_id, zone_id, None) {
            Some(zone) => (zone.settings.default_name, zone.settings.supports),
            None => {
                // A room inside a Sonos group keeps its own name and limits.
                let room = zone_id
                    .strip_prefix("sonos:")
                    .zip(self.sonos.as_ref())
                    .and_then(|(uuid, sonos)| sonos.player(uuid))
                    .ok_or(ZoneError::NotFound)?;
                (
                    room.room_name,
                    ZoneSettingsSupport {
                        volume_limits: true,
                        ..ZoneSettingsSupport::default()
                    },
                )
            }
        };
        if patch.crossfade_seconds.is_some() && !supports.crossfade {
            return Err(ZoneError::Unsupported("crossfade_not_supported_here"));
        }
        if patch.touches_leveling() && !supports.leveling {
            return Err(ZoneError::Unsupported("leveling_not_supported_here"));
        }
        if patch.touches_volume_limits() && !supports.volume_limits {
            return Err(ZoneError::Unsupported("volume_limits_not_supported_here"));
        }
        if patch.dsp.as_ref().is_some_and(DspSettings::processes) && !supports.dsp {
            return Err(ZoneError::Unsupported("dsp_not_supported_here"));
        }
        if patch.exclusive.is_some() && !supports.exclusive {
            return Err(ZoneError::Unsupported(
                "exclusive_mode_is_for_audio_devices",
            ));
        }
        if let Some(mode) = patch.volume_mode
            && !supports.volume_modes.contains(&mode)
        {
            return Err(ZoneError::Unsupported("volume_mode_not_supported_here"));
        }
        if patch.touches_max_format() && !supports.max_format {
            return Err(ZoneError::Unsupported("max_format_not_supported_here"));
        }
        if patch.touches_dsd() && !supports.dsd {
            return Err(ZoneError::Unsupported("dsd_strategy_not_supported_here"));
        }
        if patch
            .volume_control
            .is_some_and(|control| control != VolumeControl::Parson)
            && !supports.volume_control
        {
            return Err(ZoneError::Unsupported("volume_control_not_supported_here"));
        }
        if patch.private.is_some() {
            if !supports.private {
                return Err(ZoneError::Unsupported("only_parson_apps_can_be_private"));
            }
            if !viewer.owns(zone_id) {
                return Err(ZoneError::Unsupported("set_private_on_the_device_itself"));
            }
        }
        let (before, after) = {
            let mut state = self.lock();
            let before = state.settings.get(zone_id).cloned().unwrap_or_default();
            let after = apply_settings_patch(&before, &patch, &default_name)?;
            state.settings.insert(zone_id.to_string(), after.clone());
            if after.volume_control != before.volume_control {
                // A fresh estimate starts from the zone's level next time.
                state.delegated_volume.remove(zone_id);
            }
            if after.private {
                // A private zone leaves any linked group.
                for group in state.linked.values_mut() {
                    group.members.retain(|member| member != zone_id);
                }
                state.linked.retain(|_, group| group.members.len() >= 2);
            }
            (before, after)
        };
        if let Some(pool) = self.pool.clone() {
            let zone_id = zone_id.to_string();
            let stored = after.clone();
            let saved = tokio::task::spawn_blocking(move || {
                let mut connection = pool.get().map_err(|error| error.to_string())?;
                save_setting(&mut connection, &zone_id, &stored).map_err(|error| error.to_string())
            })
            .await;
            if !matches!(saved, Ok(Ok(()))) {
                tracing::warn!(?saved, "could not persist zone settings");
            }
        }
        if before.limits() != after.limits() {
            self.enforce_limits(user_id, zone_id, after.limits()).await;
        }
        if let Some(uuid) = zone_id.strip_prefix("sonos:")
            && let Some(sonos) = self.sonos.clone()
            && let Some(group) = sonos.group_of(uuid)
            && group.coordinator == uuid
        {
            if (before.crossfade_seconds > 0) != (after.crossfade_seconds > 0)
                && let Err(error) = sonos.set_crossfade(uuid, after.crossfade_seconds > 0).await
            {
                tracing::warn!(%error, uuid, "could not change Sonos crossfade");
            }
            let leveling_changed = before.leveling != after.leveling
                || (after.leveling != LevelingMode::Off
                    && (before.leveling_target_lufs != after.leveling_target_lufs
                        || before.leveling_adjustment_db != after.leveling_adjustment_db));
            if leveling_changed
                && let Some(cache) = cache
                && let Err(error) = self.refresh_sonos_upcoming(&sonos, uuid, cache).await
            {
                tracing::warn!(%error, uuid, "could not apply leveling to the Sonos queue");
            }
        }
        if hosted_zone_id(zone_id) {
            self.hosted_settings_changed(zone_id).await;
        }
        self.changed(if shared_zone(zone_id) {
            None
        } else {
            Some(user_id)
        });
        Ok(after)
    }

    /// Brings a zone's volume back inside tightened safety limits.
    async fn enforce_limits(&self, user_id: i32, zone_id: &str, limits: VolumeLimits) {
        let mut moves: Vec<(String, Option<String>, u8)> = Vec::new();
        if let Some(uuid) = zone_id.strip_prefix("sonos:") {
            if let Some(player) = self.sonos.as_ref().and_then(|sonos| sonos.player(uuid))
                && let Some(level) = limits.enforce(player.volume)
            {
                let group = self
                    .sonos
                    .as_ref()
                    .and_then(|sonos| sonos.group_of(uuid))
                    .map(|group| sonos_zone_id(&group.coordinator))
                    .unwrap_or_else(|| zone_id.to_string());
                moves.push((group, Some(zone_id.to_string()), level));
            }
        } else if let Some(zone) = self.zone(user_id, zone_id, None)
            && let Some(volume) = &zone.volume
            && let Some(level) = limits.enforce(volume.level)
        {
            moves.push((zone_id.to_string(), None, level));
        }
        for (target, member_id, volume) in moves {
            if let Err(error) = self
                .command(
                    user_id,
                    &target,
                    ZoneCommand::SetVolume {
                        volume,
                        member_id,
                        above_comfort: true,
                    },
                    None,
                )
                .await
            {
                tracing::warn!(%error, zone_id, "could not apply the new volume limits");
            }
        }
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
                            user_id,
                            ends_at_ms: now_ms() + i64::from(minutes) * 60_000,
                            finishing_track: false,
                            finishing_position: None,
                            fading: false,
                        },
                    );
                }
                Some(_) => return Err(ZoneError::Invalid("invalid_sleep_minutes")),
                None => {
                    // Cancelling during a fade stops it and restores volume.
                    state.sleep.remove(zone_id);
                }
            }
        }
        self.sleep_changed(user_id, zone_id);
        Ok(())
    }

    fn sleep_changed(&self, user_id: i32, zone_id: &str) {
        self.changed(if shared_zone(zone_id) {
            None
        } else {
            Some(user_id)
        });
    }

    /// Fires due sleep timers. A timer that runs out within five minutes of
    /// a track's end lets that track finish (Roon parity), then pauses as
    /// the next track starts or the track ends. Otherwise the zone fades
    /// out, pauses, and gets its volume back for next time.
    pub async fn run_sleep_timers(&self) {
        let now = now_ms();
        let timers: Vec<(String, i32, i64, bool, Option<usize>, bool)> = {
            let state = self.lock();
            state
                .sleep
                .iter()
                .map(|(id, timer)| {
                    (
                        id.clone(),
                        timer.user_id,
                        timer.ends_at_ms,
                        timer.finishing_track,
                        timer.finishing_position,
                        timer.fading,
                    )
                })
                .collect()
        };
        for (zone_id, user_id, ends_at_ms, finishing, finishing_position, fading) in timers {
            if fading {
                continue;
            }
            let zone = self.zone(user_id, &zone_id, None);
            let Some(zone) = zone else {
                // The zone went away (device closed, speaker unplugged).
                self.lock().sleep.remove(&zone_id);
                self.sleep_changed(user_id, &zone_id);
                continue;
            };
            let track_moved_on = finishing
                && (finishing_position != Some(zone.queue.current_position)
                    || !zone.status.is_playing());
            if ends_at_ms > now && !track_moved_on {
                continue;
            }
            let playing = zone.status.is_playing();
            if finishing {
                self.lock().sleep.remove(&zone_id);
                if playing {
                    let _ = self
                        .command(user_id, &zone_id, ZoneCommand::Pause, None)
                        .await;
                }
                self.sleep_changed(user_id, &zone_id);
                continue;
            }
            if playing && zone.duration_ms > 0 {
                let remaining = zone.duration_ms - zone.position_ms;
                if remaining > 1_000 && remaining <= SLEEP_FINISH_TRACK_WINDOW_MS {
                    if let Some(timer) = self.lock().sleep.get_mut(&zone_id) {
                        timer.ends_at_ms = now + remaining - 250;
                        timer.finishing_track = true;
                        timer.finishing_position = Some(zone.queue.current_position);
                    }
                    self.sleep_changed(user_id, &zone_id);
                    continue;
                }
            }
            if !playing {
                self.lock().sleep.remove(&zone_id);
                self.sleep_changed(user_id, &zone_id);
                continue;
            }
            if let Some(timer) = self.lock().sleep.get_mut(&zone_id) {
                timer.fading = true;
            }
            self.sleep_changed(user_id, &zone_id);
            self.fade_and_pause(user_id, &zone).await;
            {
                let mut state = self.lock();
                if state.sleep.get(&zone_id).is_some_and(|timer| timer.fading) {
                    state.sleep.remove(&zone_id);
                }
            }
            self.sleep_changed(user_id, &zone_id);
        }
    }

    /// Volume controls a fade has to move: each Sonos room on its own (a
    /// group fade would lose the rooms' balance), each member of a linked
    /// group, or the client itself.
    fn fade_targets(&self, user_id: i32, zone: &Zone) -> Vec<(String, Option<String>, u8)> {
        let audible = |volume: &Option<ZoneVolume>| {
            volume
                .as_ref()
                .filter(|volume| !volume.muted && volume.level > 0)
                .map(|volume| volume.level)
        };
        match zone.kind {
            ZoneKind::Sonos => match &zone.group {
                Some(group) => group
                    .members
                    .iter()
                    .filter_map(|member| {
                        audible(&member.volume)
                            .map(|level| (zone.id.clone(), Some(member.id.clone()), level))
                    })
                    .collect(),
                None => audible(&zone.volume)
                    .map(|level| vec![(zone.id.clone(), Some(zone.id.clone()), level)])
                    .unwrap_or_default(),
            },
            ZoneKind::Linked => zone
                .group
                .iter()
                .flat_map(|group| group.members.iter())
                .filter_map(|member| self.zone(user_id, &member.id, None))
                .flat_map(|member| self.fade_targets(user_id, &member))
                .collect(),
            ZoneKind::Chromecast => Vec::new(),
            _ => audible(&zone.volume)
                .map(|level| vec![(zone.id.clone(), None, level)])
                .unwrap_or_default(),
        }
    }

    async fn fade_and_pause(&self, user_id: i32, zone: &Zone) {
        let targets = self.fade_targets(user_id, zone);
        let fade = self.sleep_fade();
        let still_fading = || {
            self.lock()
                .sleep
                .get(&zone.id)
                .is_some_and(|timer| timer.fading)
        };
        let mut cancelled = false;
        if !targets.is_empty() && !fade.is_zero() {
            let step = fade / SLEEP_FADE_STEPS;
            for index in 1..=SLEEP_FADE_STEPS {
                tokio::time::sleep(step).await;
                if !still_fading() {
                    cancelled = true;
                    break;
                }
                for (zone_id, member_id, level) in &targets {
                    let volume =
                        (u32::from(*level) * (SLEEP_FADE_STEPS - index) / SLEEP_FADE_STEPS) as u8;
                    let _ = self
                        .command(
                            user_id,
                            zone_id,
                            ZoneCommand::SetVolume {
                                volume,
                                member_id: member_id.clone(),
                                above_comfort: true,
                            },
                            None,
                        )
                        .await;
                }
            }
        }
        if !cancelled
            && let Err(error) = self
                .command(user_id, &zone.id, ZoneCommand::Pause, None)
                .await
        {
            tracing::warn!(%error, zone = zone.id, "sleep timer could not pause the zone");
        }
        // Put the volume back so the next play is not silent.
        for (zone_id, member_id, level) in targets {
            let _ = self
                .command(
                    user_id,
                    &zone_id,
                    ZoneCommand::SetVolume {
                        volume: level,
                        member_id,
                        above_comfort: true,
                    },
                    None,
                )
                .await;
        }
    }

    #[cfg(test)]
    pub fn force_sleep_due(&self, zone_id: &str) {
        if let Some(timer) = self.lock().sleep.get_mut(zone_id) {
            timer.ends_at_ms = 0;
        }
    }

    // ----- Sonos background work ---------------------------------------

    /// Background loop: discovery, topology, polling, events, timers and
    /// client expiry.
    pub async fn run(self: Arc<Self>) {
        // Sleep timers run on their own so a fade never delays polling.
        let sleeper = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                sleeper.run_sleep_timers().await;
            }
        });
        // Renderers and audio devices: discovery and queue following.
        if let Some(outputs) = self.outputs.get().cloned() {
            let driver = self.clone();
            tokio::spawn(async move {
                let mut changes = outputs.subscribe();
                let mut interval = tokio::time::interval(Duration::from_secs(1));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                let mut tick: u64 = 0;
                loop {
                    tokio::select! {
                        _ = interval.tick() => {}
                        changed = async {
                            match changes.as_mut() {
                                Some(receiver) => receiver.recv().await.ok(),
                                None => std::future::pending().await,
                            }
                        } => {
                            if changed.is_some() {
                                // A device moved on or stopped: follow now.
                                driver.drive_hosted(0).await;
                            }
                            continue;
                        }
                    }
                    tick += 1;
                    if tick == 1 || tick.is_multiple_of(5) {
                        outputs.maintain_bridges().await;
                    }
                    if (tick == 1 || tick.is_multiple_of(10))
                        && let Some(devices) = &outputs.devices
                    {
                        devices.refresh();
                    }
                    if (tick == 1 || tick.is_multiple_of(60))
                        && let Some(upnp) = &outputs.upnp
                        && let Err(error) = upnp.discover().await
                    {
                        tracing::debug!(%error, "UPnP discovery failed");
                    }
                    driver.drive_hosted(tick).await;
                }
            });
        }
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

/// Zones every account on the server sees (speakers, renderers, devices).
/// Waits (briefly) until a stopped renderer has closed its device stream.
async fn wait_until_released(renderer: Option<Arc<dyn crate::outputs::renderer::Renderer>>) {
    let Some(renderer) = renderer else {
        return;
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    while renderer.state().state != crate::outputs::renderer::PlayState::Stopped
        && Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn shared_zone(zone_id: &str) -> bool {
    zone_id.starts_with("sonos:") || hosted_zone_id(zone_id)
}

/// Zones the server plays itself (renderers, devices, bridges, AirPlay).
pub(crate) fn hosted_zone_id(zone_id: &str) -> bool {
    [
        "upnp:",
        "device:",
        "bridge:",
        "bridge-group:",
        "airplay:",
        "chromecast:",
    ]
    .iter()
    .any(|prefix| zone_id.starts_with(prefix))
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
        fading: timer.fading,
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
    let default_name = members
        .iter()
        .map(|zone| zone.name.as_str())
        .collect::<Vec<_>>()
        .join(" + ");
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
        name: settings
            .name
            .clone()
            .unwrap_or_else(|| default_name.clone()),
        icon: zone_icon(&settings, ZoneKind::Linked),
        device: format!("{} linked zones", members.len()),
        status: leader.status,
        source: leader.source,
        now_playing: leader.now_playing.clone(),
        queue: leader.queue.clone(),
        repeat: leader.repeat,
        shuffle: leader.shuffle,
        position_ms: leader.position_ms,
        duration_ms: leader.duration_ms,
        volume: Some(ZoneVolume::new(level, muted, settings.limits())),
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
            queue_edit: leader.capabilities.queue_edit,
            repeat: leader.capabilities.repeat,
            shuffle: leader.capabilities.shuffle,
            group: false,
            transfer_target: true,
            sleep_timer: true,
            standby: false,
        },
        sleep_timer: sleep_view(state, id),
        settings: settings_view(
            &settings,
            &default_name,
            ZoneSettingsSupport {
                crossfade: false,
                leveling: false,
                volume_limits: true,
                private: false,
                ..ZoneSettingsSupport::default()
            },
            vec![
                "Crossfade and volume leveling are set on each zone in the group. A custom group name is forgotten when the group ends."
                    .to_string(),
            ],
        ),
        output: None,
        endpoints: Vec::new(),
        notes: vec![LINKED_SYNC_NOTE.to_string()],
    })
}

/// Every zone lists the outputs it plays through: a group its members'
/// (with each member's own volume), a single output itself.
fn fill_endpoints(zones: &mut [Zone]) {
    let singles: HashMap<String, Vec<ZoneEndpoint>> = zones
        .iter()
        .filter(|zone| zone.group.is_none() && !zone.endpoints.is_empty())
        .map(|zone| (zone.id.clone(), zone.endpoints.clone()))
        .collect();
    for zone in zones.iter_mut() {
        if !zone.endpoints.is_empty() {
            continue;
        }
        zone.endpoints = match &zone.group {
            Some(group) => group
                .members
                .iter()
                .flat_map(|member| {
                    let mut endpoints = singles.get(&member.id).cloned().unwrap_or_else(|| {
                        vec![ZoneEndpoint::named(
                            &member.id,
                            &member.name,
                            member.kind.label(),
                        )]
                    });
                    if let [endpoint] = endpoints.as_mut_slice() {
                        endpoint.volume = member.volume.clone();
                    }
                    endpoints
                })
                .collect(),
            None => {
                let mut endpoint = ZoneEndpoint::named(&zone.id, &zone.name, &zone.device);
                endpoint.volume = zone.volume.clone();
                vec![endpoint]
            }
        };
    }
}

fn zone_icon(settings: &ZoneSettings, kind: ZoneKind) -> String {
    settings
        .icon
        .clone()
        .unwrap_or_else(|| default_zone_icon(kind).to_string())
}

/// Standby and Home Assistant volume control, which work the same on every
/// kind of zone: the status says standby, and a delegated volume shows
/// Parson's running estimate instead of a device reading.
fn apply_standby_and_volume_control(state: &State, zone: &mut Zone) {
    if state.standby.contains(&zone.id) {
        zone.status = ZoneStatus::Standby;
    }
    zone.settings.supports.volume_control = zone.kind != ZoneKind::Linked;
    let control = zone.settings.volume_control;
    if control == VolumeControl::Parson {
        return;
    }
    let limits = state
        .settings
        .get(&zone.id)
        .map(ZoneSettings::limits)
        .unwrap_or_default();
    let (level, muted) = state
        .delegated_volume
        .get(&zone.id)
        .copied()
        .unwrap_or_else(|| delegated_start(zone, limits));
    let mut volume = ZoneVolume::new(level, muted, limits);
    volume.control = control;
    zone.volume = Some(volume);
    zone.capabilities.volume = true;
    zone.capabilities.mute = true;
}

/// Where a delegated volume estimate starts: the zone's own level when it
/// had one, else the middle of its allowed range.
fn delegated_start(zone: &Zone, limits: VolumeLimits) -> (u8, bool) {
    zone.volume.as_ref().map_or_else(
        || (limits.clamp(50, false, false), false),
        |volume| (volume.level, volume.muted),
    )
}

fn settings_view(
    settings: &ZoneSettings,
    default_name: &str,
    supports: ZoneSettingsSupport,
    notes: Vec<String>,
) -> ZoneSettingsView {
    ZoneSettingsView {
        default_name: default_name.to_string(),
        custom_name: settings.name.is_some(),
        icon: settings.icon.clone(),
        crossfade_seconds: settings.crossfade_seconds,
        leveling: settings.leveling,
        leveling_target_lufs: settings.leveling_target_lufs,
        leveling_adjustment_db: settings.leveling_adjustment_db,
        private: settings.private,
        dsp: settings.dsp.clone(),
        exclusive: settings.exclusive,
        volume_mode: settings.volume_mode.unwrap_or_default(),
        volume_control: settings.volume_control,
        max_sample_rate: settings.max_sample_rate,
        max_bit_depth: settings.max_bit_depth,
        dsd_strategy: settings.dsd_strategy,
        max_dsd_rate: settings.max_dsd_rate,
        supports,
        notes,
    }
}

/// Leveling a Sonos zone applies to the track at `index` of its queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamLeveling {
    pub by: LevelBy,
    pub target_lufs: i8,
    pub adjustment_db: i8,
}

impl StreamLeveling {
    /// Extra query parameters on a signed media URL.
    pub fn query(self) -> String {
        format!(
            "&level={}&target={}&adjust={}",
            self.by.as_str(),
            self.target_lufs,
            self.adjustment_db
        )
    }
}

/// What a signed zone media URL is signed for: the zone key and, when
/// leveled, the leveling (so it cannot be changed in the URL).
pub fn zone_media_scope(media_key: &str, leveling: Option<StreamLeveling>) -> String {
    match leveling {
        None => format!("zone:{media_key}"),
        Some(leveling) => format!(
            "zone:{media_key};level={};target={};adjust={}",
            leveling.by.as_str(),
            leveling.target_lufs,
            leveling.adjustment_db
        ),
    }
}

fn sonos_leveling(
    settings: &ZoneSettings,
    queue: &[String],
    index: usize,
    cache: &LibraryCache,
) -> Option<StreamLeveling> {
    let by = super::leveling::resolve_mode(settings.leveling, queue, index, |song_id| {
        cache
            .song_map
            .get(song_id)
            .map(|(_, album_id)| album_id.clone())
    })?;
    Some(StreamLeveling {
        by,
        target_lufs: settings.leveling_target_lufs,
        adjustment_db: settings.leveling_adjustment_db,
    })
}

/// Signed URL and DIDL-Lite for one track on a Sonos zone.
fn sonos_track(
    cache: &LibraryCache,
    origin: &str,
    media_key: &str,
    song_id: &str,
    expires: i64,
    leveling: Option<StreamLeveling>,
) -> Option<(String, String)> {
    let song = cache.song(song_id)?;
    let album = cache
        .song_map
        .get(song_id)
        .and_then(|(_, album_id)| cache.album(album_id));
    let plan = if leveling.is_some() {
        crate::api::library::sonos_leveled_stream_plan(song)
    } else {
        crate::api::library::sonos_stream_plan(song)
    };
    let signature =
        crate::api::cast::sign_media_url(&zone_media_scope(media_key, leveling), song_id, expires);
    let uri = format!(
        "{origin}{ZONE_MEDIA_PATH}{}/stream?zone={}&expires={expires}&signature={signature}{}",
        crate::api::cast::encoded(song_id),
        crate::api::cast::encoded(media_key),
        leveling.map(StreamLeveling::query).unwrap_or_default(),
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
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    icon: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    crossfade_seconds: i32,
    #[diesel(sql_type = diesel::sql_types::Text)]
    leveling: String,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    leveling_target_lufs: i32,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    leveling_adjustment_db: i32,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
    comfort_volume: Option<i32>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
    safety_min_volume: Option<i32>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
    safety_max_volume: Option<i32>,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    private: i32,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    dsp: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    output: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Text)]
    volume_control: String,
}

/// Device setup of a server-driven zone, stored as JSON.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct StoredOutput {
    exclusive: Option<bool>,
    volume_mode: Option<VolumeMode>,
    max_sample_rate: Option<u32>,
    max_bit_depth: Option<u8>,
    dsd_strategy: Option<DsdStrategy>,
    max_dsd_rate: Option<u32>,
}

fn load_settings(
    connection: &mut diesel::sqlite::SqliteConnection,
) -> Result<HashMap<String, ZoneSettings>, diesel::result::Error> {
    use diesel::RunQueryDsl;
    let rows = diesel::sql_query(
        "SELECT zone_id, name, icon, crossfade_seconds, leveling, leveling_target_lufs,
                leveling_adjustment_db, comfort_volume, safety_min_volume, safety_max_volume,
                private, dsp, output, volume_control
         FROM zone_setting",
    )
    .load::<SettingRow>(connection)?;
    let level = |value: Option<i32>| value.and_then(|level| u8::try_from(level).ok());
    Ok(rows
        .into_iter()
        .map(|row| {
            let defaults = ZoneSettings::default();
            let dsp = row
                .dsp
                .as_deref()
                .and_then(|json| serde_json::from_str::<DspSettings>(json).ok())
                .map(DspSettings::normalized)
                .unwrap_or_default();
            let output = row
                .output
                .as_deref()
                .and_then(|json| serde_json::from_str::<StoredOutput>(json).ok())
                .unwrap_or_default();
            (
                row.zone_id,
                ZoneSettings {
                    name: row.name,
                    icon: row.icon.filter(|icon| ZONE_ICONS.contains(&icon.as_str())),
                    crossfade_seconds: u8::try_from(row.crossfade_seconds)
                        .unwrap_or(0)
                        .min(CROSSFADE_MAX_SECONDS),
                    leveling: LevelingMode::parse(&row.leveling).unwrap_or_default(),
                    leveling_target_lufs: i8::try_from(row.leveling_target_lufs)
                        .ok()
                        .filter(|target| LEVELING_TARGET_RANGE.contains(target))
                        .unwrap_or(defaults.leveling_target_lufs),
                    leveling_adjustment_db: i8::try_from(row.leveling_adjustment_db)
                        .ok()
                        .filter(|adjustment| LEVELING_ADJUSTMENT_RANGE.contains(adjustment))
                        .unwrap_or(0),
                    comfort_volume: level(row.comfort_volume),
                    safety_min_volume: level(row.safety_min_volume),
                    safety_max_volume: level(row.safety_max_volume),
                    private: row.private != 0,
                    dsp,
                    exclusive: output.exclusive.unwrap_or(true),
                    volume_mode: output.volume_mode,
                    volume_control: VolumeControl::parse(&row.volume_control).unwrap_or_default(),
                    max_sample_rate: output
                        .max_sample_rate
                        .filter(|rate| SAMPLE_RATE_CHOICES.contains(rate)),
                    max_bit_depth: output
                        .max_bit_depth
                        .filter(|depth| BIT_DEPTH_CHOICES.contains(depth)),
                    dsd_strategy: output.dsd_strategy.unwrap_or_default(),
                    max_dsd_rate: output
                        .max_dsd_rate
                        .filter(|rate| DSD_RATE_CHOICES.contains(rate)),
                },
            )
        })
        .collect())
}

/// The zone a device of the old server-output picker plays as.
fn legacy_output_zone(device_id: &str) -> String {
    if let Some(mac) = device_id.strip_prefix("airplay:") {
        format!("airplay:{}", mac.to_ascii_lowercase())
    } else if device_id.starts_with("upnp:") {
        device_id.to_string()
    } else {
        format!("device:{device_id}")
    }
}

/// Moves the old server-output picker's device setup and DSP chains into
/// the settings of the zones those devices are now, once: a zone that
/// already has settings keeps them, and the old files are retired.
fn import_legacy_outputs(
    connection: &mut diesel::sqlite::SqliteConnection,
    settings: &mut HashMap<String, ZoneSettings>,
) {
    use crate::settings::DataFile;
    let outputs = DataFile::LegacyNativeOutputs.read().map(|bytes| {
        serde_json::from_slice::<HashMap<String, parson_output::OutputSettings>>(&bytes)
            .unwrap_or_default()
    });
    let chains = DataFile::LegacyNativeDsp.read().map(|bytes| {
        serde_json::from_slice::<HashMap<String, DspSettings>>(&bytes).unwrap_or_default()
    });
    if outputs.is_none() && chains.is_none() {
        return;
    }
    let mut imported: HashMap<String, ZoneSettings> = HashMap::new();
    for (device_id, output) in outputs.iter().flatten().take(64) {
        let zone_id = legacy_output_zone(device_id);
        if settings.contains_key(&zone_id) || output.validate().is_err() {
            continue;
        }
        let entry = imported.entry(zone_id).or_default();
        entry.exclusive = output.exclusive;
        entry.max_sample_rate = output
            .max_sample_rate
            .filter(|rate| SAMPLE_RATE_CHOICES.contains(rate));
        entry.max_bit_depth = output
            .max_bit_depth
            .filter(|depth| BIT_DEPTH_CHOICES.contains(depth));
        entry.dsd_strategy = output.dsd_strategy;
        entry.max_dsd_rate = output
            .max_dsd_rate
            .filter(|rate| DSD_RATE_CHOICES.contains(rate));
    }
    for (device_id, chain) in chains.iter().flatten().take(64) {
        let zone_id = legacy_output_zone(device_id);
        if settings.contains_key(&zone_id) {
            continue;
        }
        imported.entry(zone_id).or_default().dsp = chain.clone().normalized();
    }
    let mut saved = true;
    for (zone_id, zone) in imported {
        match save_setting(connection, &zone_id, &zone) {
            Ok(()) => {
                settings.insert(zone_id, zone);
            }
            Err(error) => {
                saved = false;
                tracing::warn!(%error, zone_id, "could not import an output's settings");
            }
        }
    }
    if saved {
        for file in [DataFile::LegacyNativeOutputs, DataFile::LegacyNativeDsp] {
            if file.read().is_some()
                && let Err(error) = file.retire()
            {
                tracing::warn!(%error, ?file, "could not retire an imported settings file");
            }
        }
    }
}

fn save_setting(
    connection: &mut diesel::sqlite::SqliteConnection,
    zone_id: &str,
    settings: &ZoneSettings,
) -> Result<(), diesel::result::Error> {
    use diesel::RunQueryDsl;
    use diesel::sql_types::{Integer, Nullable, Text};
    diesel::sql_query(
        "INSERT INTO zone_setting (zone_id, name, icon, crossfade_seconds, leveling,
             leveling_target_lufs, leveling_adjustment_db, comfort_volume, safety_min_volume,
             safety_max_volume, private, dsp, output, volume_control, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
         ON CONFLICT(zone_id) DO UPDATE SET name = excluded.name, icon = excluded.icon,
             crossfade_seconds = excluded.crossfade_seconds, leveling = excluded.leveling,
             leveling_target_lufs = excluded.leveling_target_lufs,
             leveling_adjustment_db = excluded.leveling_adjustment_db,
             comfort_volume = excluded.comfort_volume,
             safety_min_volume = excluded.safety_min_volume,
             safety_max_volume = excluded.safety_max_volume, private = excluded.private,
             dsp = excluded.dsp, output = excluded.output,
             volume_control = excluded.volume_control,
             updated_at = CURRENT_TIMESTAMP",
    )
    .bind::<Text, _>(zone_id)
    .bind::<Nullable<Text>, _>(settings.name.as_deref())
    .bind::<Nullable<Text>, _>(settings.icon.as_deref())
    .bind::<Integer, _>(i32::from(settings.crossfade_seconds))
    .bind::<Text, _>(settings.leveling.as_str())
    .bind::<Integer, _>(i32::from(settings.leveling_target_lufs))
    .bind::<Integer, _>(i32::from(settings.leveling_adjustment_db))
    .bind::<Nullable<Integer>, _>(settings.comfort_volume.map(i32::from))
    .bind::<Nullable<Integer>, _>(settings.safety_min_volume.map(i32::from))
    .bind::<Nullable<Integer>, _>(settings.safety_max_volume.map(i32::from))
    .bind::<Integer, _>(i32::from(settings.private))
    .bind::<Nullable<Text>, _>(
        (settings.dsp != DspSettings::default())
            .then(|| serde_json::to_string(&settings.dsp).ok())
            .flatten(),
    )
    .bind::<Nullable<Text>, _>(
        serde_json::to_string(&StoredOutput {
            exclusive: (!settings.exclusive).then_some(false),
            volume_mode: settings.volume_mode,
            max_sample_rate: settings.max_sample_rate,
            max_bit_depth: settings.max_bit_depth,
            dsd_strategy: (settings.dsd_strategy != DsdStrategy::Auto)
                .then_some(settings.dsd_strategy),
            max_dsd_rate: settings.max_dsd_rate,
        })
        .ok(),
    )
    .bind::<Text, _>(settings.volume_control.as_str())
    .execute(connection)?;
    Ok(())
}

#[cfg(test)]
mod settings_storage_tests {
    use diesel::Connection;
    use diesel::RunQueryDsl;
    use diesel_migrations::MigrationHarness;

    use super::*;

    #[derive(diesel::QueryableByName)]
    struct DspRow {
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
        dsp: Option<String>,
    }

    /// Reverts migrations down to (and including) the DSP chain migration.
    fn revert_dsp_chain(connection: &mut diesel::sqlite::SqliteConnection) {
        const DSP_CHAIN: &str = "20261104000000";
        while connection
            .applied_migrations()
            .expect("applied migrations")
            .iter()
            .any(|version| version.to_string() == DSP_CHAIN)
        {
            connection
                .revert_last_migration(crate::persistence::connection::MIGRATIONS)
                .expect("revert");
        }
    }

    fn stored_dsp(connection: &mut diesel::sqlite::SqliteConnection) -> serde_json::Value {
        let row = diesel::sql_query("SELECT dsp FROM zone_setting WHERE zone_id = 'device:dac'")
            .get_result::<DspRow>(connection)
            .expect("row");
        serde_json::from_str(&row.dsp.expect("dsp")).expect("json")
    }

    #[test]
    fn the_old_dsp_block_moves_into_the_dsp_chain_and_back() {
        let mut connection =
            diesel::sqlite::SqliteConnection::establish(":memory:").expect("database");
        connection
            .run_pending_migrations(crate::persistence::connection::MIGRATIONS)
            .expect("migrations");
        revert_dsp_chain(&mut connection);
        diesel::sql_query(
            "INSERT INTO zone_setting (zone_id, crossfade_seconds, leveling, leveling_target_lufs,
                 leveling_adjustment_db, private, dsp)
             VALUES ('device:dac', 0, 'off', -14, 0, 0,
                 '{\"enabled\":true,\"preamp_db\":-2,\"bass_db\":3,\"treble_db\":-1,\"crossfeed\":true}')",
        )
        .execute(&mut connection)
        .expect("old row");
        connection
            .run_pending_migrations(crate::persistence::connection::MIGRATIONS)
            .expect("the DSP chain migration");
        let settings = load_settings(&mut connection).expect("settings");
        let dsp = &settings["device:dac"].dsp;
        // The largest boost (3 dB) set the old block's level.
        assert_eq!(dsp.headroom_db(), Some(-3.0));
        let bands = dsp.active_bands();
        assert_eq!(bands.len(), 2);
        assert_eq!(
            (bands[0].frequency, bands[0].gain_db),
            (100.0, 3.0),
            "bass is a 100 Hz shelf"
        );
        assert_eq!((bands[1].frequency, bands[1].gain_db), (10_000.0, -1.0));
        assert_eq!(
            dsp.crossfeed(),
            Some(parson_output::dsp::CrossfeedStrength::Strong)
        );
        assert_eq!(
            crate::zones::dsp_summary(dsp),
            "Headroom \u{2212}3.0 dB, EQ 100 Hz low shelf +3.0 dB, 10 kHz high shelf \u{2212}1.0 dB, Crossfeed (strong)"
        );
        // Back to the five knobs on the way down.
        revert_dsp_chain(&mut connection);
        assert_eq!(
            stored_dsp(&mut connection),
            serde_json::json!({"enabled": true, "preamp_db": -3, "bass_db": 3, "treble_db": -1, "crossfeed": true})
        );
    }

    #[test]
    fn the_old_server_output_picker_devices_become_zone_settings() {
        assert_eq!(legacy_output_zone("alsa:USB:0"), "device:alsa:USB:0");
        assert_eq!(
            legacy_output_zone("airplay:AABBCCDDEEFF"),
            "airplay:aabbccddeeff"
        );
        assert_eq!(legacy_output_zone("upnp:uuid-1"), "upnp:uuid-1");
        let settings = ZoneSettings {
            max_sample_rate: Some(96_000),
            dsd_strategy: DsdStrategy::Dop,
            max_dsd_rate: Some(5_644_800),
            exclusive: false,
            ..ZoneSettings::default()
        };
        assert_eq!(
            settings.output(),
            parson_output::OutputSettings {
                exclusive: false,
                max_sample_rate: Some(96_000),
                max_bit_depth: None,
                dsd_strategy: DsdStrategy::Dop,
                max_dsd_rate: Some(5_644_800),
            }
        );
        let patched = apply_settings_patch(
            &settings,
            &ZoneSettingsPatch {
                max_dsd_rate: Some(Some(1)),
                ..ZoneSettingsPatch::default()
            },
            "DAC",
        );
        assert_eq!(patched, Err(ZoneError::Invalid("invalid_max_dsd_rate")));
    }
}
