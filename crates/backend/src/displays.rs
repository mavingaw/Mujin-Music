//! Displays: a TV, tablet, Chromecast tab or spare browser that shows what a
//! zone plays, full screen and without controls (Roon's "Displays").
//!
//! A display is paired once (a six-digit code shown on the screen, approved
//! from a signed-in app) or opened from a link the app makes. Either way it
//! receives a long-lived key that can only read the now-playing state of its
//! zone, or of whichever of the listener's zones is playing: never the
//! library, the queue editor, the account or any other zone's controls.
//! Only a hash of the key is stored; removing the display revokes it.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use diesel::deserialize::QueryableByName;
use diesel::prelude::*;
use diesel::sql_types::{Integer, Nullable, Text};
use diesel::sqlite::SqliteConnection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::zones::{Zone, ZoneStatus};

/// Request header that carries a display's key.
pub const DISPLAY_KEY_HEADER: &str = "x-parson-display";
/// Every display key starts with this, so a key is recognisable in logs and
/// password managers and never mistaken for a session token.
pub const DISPLAY_KEY_PREFIX: &str = "mjd_";
/// Displays one listener can hold.
pub const MAX_DISPLAYS_PER_USER: i64 = 32;
/// A pairing code stays valid this long.
pub const PAIRING_LIFETIME: Duration = Duration::from_secs(10 * 60);
const MAX_PENDING_PAIRINGS: usize = 64;
const MAX_NAME_CHARS: usize = 60;
/// A display that asked for its state this recently counts as online.
pub const ONLINE_WITHIN_SECONDS: i64 = 30;
/// `last_seen_at` is written at most this often per display.
const TOUCH_INTERVAL_SECONDS: i64 = 10;

/// Whether the display shows a track's timed lyrics over the artwork.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LyricsMode {
    /// Timed lyrics take over the screen when the track has them.
    #[default]
    Auto,
    /// Never: artwork, artist photo and track facts only.
    Off,
}

impl LyricsMode {
    pub fn as_str(self) -> &'static str {
        match self {
            LyricsMode::Auto => "auto",
            LyricsMode::Off => "off",
        }
    }

    pub fn parse(value: &str) -> Self {
        if value == "off" {
            LyricsMode::Off
        } else {
            LyricsMode::Auto
        }
    }
}

/// One stored display.
#[derive(Clone, Debug, PartialEq, QueryableByName)]
pub struct DisplayRow {
    #[diesel(sql_type = Text)]
    pub id: String,
    #[diesel(sql_type = Integer)]
    pub user_id: i32,
    #[diesel(sql_type = Text)]
    pub name: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub zone_id: Option<String>,
    #[diesel(sql_type = Text)]
    pub lyrics: String,
    #[diesel(sql_type = Text)]
    pub created_at: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub last_seen_at: Option<String>,
}

/// A display as the apps list it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DisplayView {
    pub id: String,
    pub name: String,
    /// `None` follows whichever zone is playing.
    pub zone_id: Option<String>,
    pub lyrics: LyricsMode,
    pub created_at: String,
    pub last_seen_at: Option<String>,
    /// Asked for its state within the last half minute.
    pub online: bool,
}

impl DisplayRow {
    pub fn view(&self, now: chrono::NaiveDateTime) -> DisplayView {
        let online = self
            .last_seen_at
            .as_deref()
            .and_then(parse_timestamp)
            .is_some_and(|seen| (now - seen).num_seconds() <= ONLINE_WITHIN_SECONDS);
        DisplayView {
            id: self.id.clone(),
            name: self.name.clone(),
            zone_id: self.zone_id.clone(),
            lyrics: LyricsMode::parse(&self.lyrics),
            created_at: self.created_at.clone(),
            last_seen_at: self.last_seen_at.clone(),
            online,
        }
    }
}

fn parse_timestamp(value: &str) -> Option<chrono::NaiveDateTime> {
    chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S").ok()
}

pub fn now_utc() -> chrono::NaiveDateTime {
    chrono::Utc::now().naive_utc()
}

/// A display name: trimmed, one line, at most 60 characters.
pub fn clean_name(value: &str) -> Option<String> {
    let name = value.trim();
    if name.is_empty() || name.chars().any(char::is_control) {
        return None;
    }
    Some(name.chars().take(MAX_NAME_CHARS).collect())
}

/// A zone id as clients send it: short and printable.
pub fn valid_zone_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 200 && !value.chars().any(char::is_control)
}

/// A new display key: the prefix and 32 random bytes.
pub fn new_key() -> String {
    let mut bytes = [0_u8; 32];
    rand::fill(&mut bytes);
    format!("{DISPLAY_KEY_PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes))
}

/// What is stored for a key: its SHA-256, hex.
pub fn key_hash(key: &str) -> String {
    Sha256::digest(key.trim().as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn plausible_key(key: &str) -> bool {
    key.starts_with(DISPLAY_KEY_PREFIX) && (20..=120).contains(&key.len())
}

const COLUMNS: &str = "id, user_id, name, zone_id, lyrics,
    CAST(created_at AS TEXT) AS created_at, CAST(last_seen_at AS TEXT) AS last_seen_at";

#[derive(Debug)]
pub enum DisplayError {
    TooMany,
    Database(diesel::result::Error),
}

impl From<diesel::result::Error> for DisplayError {
    fn from(error: diesel::result::Error) -> Self {
        DisplayError::Database(error)
    }
}

/// Stores a new display and returns it with its key (shown once).
pub fn create_display(
    connection: &mut SqliteConnection,
    user_id: i32,
    name: &str,
    zone_id: Option<&str>,
    lyrics: LyricsMode,
) -> Result<(DisplayRow, String), DisplayError> {
    #[derive(QueryableByName)]
    struct Count {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        count: i64,
    }
    let held = diesel::sql_query("SELECT COUNT(*) AS count FROM zone_display WHERE user_id = ?")
        .bind::<Integer, _>(user_id)
        .get_result::<Count>(connection)?
        .count;
    if held >= MAX_DISPLAYS_PER_USER {
        return Err(DisplayError::TooMany);
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    let key = new_key();
    diesel::sql_query(
        "INSERT INTO zone_display (id, user_id, name, zone_id, lyrics, key_hash)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind::<Text, _>(&id)
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(name)
    .bind::<Nullable<Text>, _>(zone_id)
    .bind::<Text, _>(lyrics.as_str())
    .bind::<Text, _>(key_hash(&key))
    .execute(connection)?;
    let row = display(connection, user_id, &id)?.ok_or(diesel::result::Error::NotFound)?;
    Ok((row, key))
}

pub fn list_displays(
    connection: &mut SqliteConnection,
    user_id: i32,
) -> QueryResult<Vec<DisplayRow>> {
    diesel::sql_query(format!(
        "SELECT {COLUMNS} FROM zone_display WHERE user_id = ? ORDER BY created_at, id"
    ))
    .bind::<Integer, _>(user_id)
    .load(connection)
}

pub fn display(
    connection: &mut SqliteConnection,
    user_id: i32,
    id: &str,
) -> QueryResult<Option<DisplayRow>> {
    diesel::sql_query(format!(
        "SELECT {COLUMNS} FROM zone_display WHERE user_id = ? AND id = ?"
    ))
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(id)
    .get_result(connection)
    .optional()
}

/// What a settings change may touch.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct DisplayPatch {
    pub name: Option<String>,
    /// `Some(None)` (JSON `null`) switches to following the playing zone.
    #[serde(default, deserialize_with = "double_option")]
    pub zone_id: Option<Option<String>>,
    pub lyrics: Option<LyricsMode>,
}

fn double_option<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

pub fn update_display(
    connection: &mut SqliteConnection,
    user_id: i32,
    id: &str,
    patch: &DisplayPatch,
) -> QueryResult<Option<DisplayRow>> {
    let Some(current) = display(connection, user_id, id)? else {
        return Ok(None);
    };
    let name = patch.name.clone().unwrap_or(current.name);
    let zone_id = match &patch.zone_id {
        Some(zone_id) => zone_id.clone(),
        None => current.zone_id,
    };
    let lyrics = patch
        .lyrics
        .unwrap_or_else(|| LyricsMode::parse(&current.lyrics));
    diesel::sql_query(
        "UPDATE zone_display SET name = ?, zone_id = ?, lyrics = ? WHERE user_id = ? AND id = ?",
    )
    .bind::<Text, _>(&name)
    .bind::<Nullable<Text>, _>(zone_id.as_deref())
    .bind::<Text, _>(lyrics.as_str())
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(id)
    .execute(connection)?;
    display(connection, user_id, id)
}

/// Removes a display; its key stops working at once.
pub fn delete_display(
    connection: &mut SqliteConnection,
    user_id: i32,
    id: &str,
) -> QueryResult<bool> {
    diesel::sql_query("DELETE FROM zone_display WHERE user_id = ? AND id = ?")
        .bind::<Integer, _>(user_id)
        .bind::<Text, _>(id)
        .execute(connection)
        .map(|deleted| deleted > 0)
}

/// The display a key belongs to, if it is still paired.
pub fn display_for_key(
    connection: &mut SqliteConnection,
    key: &str,
) -> QueryResult<Option<DisplayRow>> {
    if !plausible_key(key) {
        return Ok(None);
    }
    diesel::sql_query(format!(
        "SELECT {COLUMNS} FROM zone_display WHERE key_hash = ?"
    ))
    .bind::<Text, _>(key_hash(key))
    .get_result(connection)
    .optional()
}

/// Records that a display asked for its state (at most every few seconds).
pub fn touch_display(connection: &mut SqliteConnection, id: &str) -> QueryResult<()> {
    diesel::sql_query(format!(
        "UPDATE zone_display SET last_seen_at = CURRENT_TIMESTAMP
         WHERE id = ? AND (last_seen_at IS NULL
           OR last_seen_at < datetime('now', '-{TOUCH_INTERVAL_SECONDS} seconds'))"
    ))
    .bind::<Text, _>(id)
    .execute(connection)
    .map(|_| ())
}

// ---------------------------------------------------------------------------
// Pairing: the screen shows a code, a signed-in app approves it.

#[derive(Clone, Debug)]
struct PendingPairing {
    code: String,
    name: String,
    secret_hash: String,
    expires_at: Instant,
    approving: bool,
    approved: Option<(DisplayView, String)>,
}

fn pairings() -> &'static Mutex<HashMap<String, PendingPairing>> {
    static PAIRINGS: OnceLock<Mutex<HashMap<String, PendingPairing>>> = OnceLock::new();
    PAIRINGS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_pairings() -> std::sync::MutexGuard<'static, HashMap<String, PendingPairing>> {
    pairings()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A pairing the screen started.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PairingStart {
    pub pairing_id: String,
    /// Proves the poller is the screen that started it.
    pub secret: String,
    /// Six digits, shown large on the screen.
    pub code: String,
    pub expires_in: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PairingError {
    Busy,
    NotFound,
}

/// Six digits from random bytes, never one already waiting.
fn fresh_code(taken: &HashMap<String, PendingPairing>) -> String {
    loop {
        let mut bytes = [0_u8; 4];
        rand::fill(&mut bytes);
        let code = format!("{:06}", u32::from_le_bytes(bytes) % 1_000_000);
        if !taken.values().any(|pending| pending.code == code) {
            return code;
        }
    }
}

/// `123 456` and `123-456` read as `123456`; anything but six digits is no code.
pub fn normalized_code(value: &str) -> Option<String> {
    let digits = value
        .chars()
        .filter(char::is_ascii_digit)
        .collect::<String>();
    let only_digits_and_spacing = value
        .chars()
        .all(|character| character.is_ascii_digit() || matches!(character, ' ' | '-'));
    (digits.len() == 6 && only_digits_and_spacing).then_some(digits)
}

pub fn start_pairing(name: &str, now: Instant) -> Result<PairingStart, PairingError> {
    let mut pending = lock_pairings();
    pending.retain(|_, pairing| pairing.expires_at > now);
    if pending.len() >= MAX_PENDING_PAIRINGS {
        return Err(PairingError::Busy);
    }
    let pairing_id = uuid::Uuid::new_v4().simple().to_string();
    let secret = new_key();
    let code = fresh_code(&pending);
    pending.insert(
        pairing_id.clone(),
        PendingPairing {
            code: code.clone(),
            name: name.to_string(),
            secret_hash: key_hash(&secret),
            expires_at: now + PAIRING_LIFETIME,
            approving: false,
            approved: None,
        },
    );
    Ok(PairingStart {
        pairing_id,
        secret,
        code,
        expires_in: PAIRING_LIFETIME.as_secs(),
    })
}

/// Where a screen's pairing stands.
#[derive(Clone, Debug, PartialEq)]
pub enum PairingStatus {
    Pending {
        name: String,
    },
    /// Delivered once; the pairing is forgotten afterwards.
    Approved {
        display: DisplayView,
        key: String,
    },
}

pub fn pairing_status(
    pairing_id: &str,
    secret: &str,
    now: Instant,
) -> Result<PairingStatus, PairingError> {
    let mut pending = lock_pairings();
    pending.retain(|_, pairing| pairing.expires_at > now);
    let Some(pairing) = pending.get(pairing_id) else {
        return Err(PairingError::NotFound);
    };
    if pairing.secret_hash != key_hash(secret) {
        return Err(PairingError::NotFound);
    }
    match pairing.approved.clone() {
        Some((display, key)) => {
            pending.remove(pairing_id);
            Ok(PairingStatus::Approved { display, key })
        }
        None => Ok(PairingStatus::Pending {
            name: pairing.name.clone(),
        }),
    }
}

/// Reserves the pairing showing `code` for approval; returns its id and the
/// name the screen suggested.
pub fn claim_code(code: &str, now: Instant) -> Option<(String, String)> {
    let mut pending = lock_pairings();
    pending.retain(|_, pairing| pairing.expires_at > now);
    pending.iter_mut().find_map(|(id, pairing)| {
        if pairing.code != code || pairing.approving || pairing.approved.is_some() {
            return None;
        }
        pairing.approving = true;
        Some((id.clone(), pairing.name.clone()))
    })
}

/// Hands the new display's key to the waiting screen.
pub fn complete_pairing(pairing_id: &str, display: DisplayView, key: String) -> bool {
    let mut pending = lock_pairings();
    let Some(pairing) = pending.get_mut(pairing_id) else {
        return false;
    };
    pairing.approving = false;
    pairing.approved = Some((display, key));
    true
}

/// Releases a claimed code after a failed approval.
pub fn release_code(pairing_id: &str) {
    if let Some(pairing) = lock_pairings().get_mut(pairing_id) {
        pairing.approving = false;
    }
}

// ---------------------------------------------------------------------------
// Which zone a display shows.

/// The zone a display shows, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ZoneChoice {
    /// Show this zone.
    Zone(String),
    /// The bound zone is gone (switched off, private, or forgotten).
    Missing,
    /// Following, and no zone has anything to show.
    Nothing,
}

fn has_track(zone: &Zone) -> bool {
    zone.now_playing.is_some()
        && !matches!(
            zone.status,
            ZoneStatus::Unavailable | ZoneStatus::Standby | ZoneStatus::Idle
        )
}

fn is_playing(zone: &Zone) -> bool {
    zone.status.is_playing() && zone.now_playing.is_some()
}

/// A bound display shows its zone. A following display keeps the zone it
/// showed while that zone still has a track and nothing else started, then
/// moves to a playing zone (a linked group before its members), then to a
/// paused one.
pub fn choose_zone(zones: &[Zone], bound: Option<&str>, previous: Option<&str>) -> ZoneChoice {
    if let Some(bound) = bound {
        return if zones.iter().any(|zone| zone.id == bound) {
            ZoneChoice::Zone(bound.to_string())
        } else {
            ZoneChoice::Missing
        };
    }
    // A member of a linked group plays what the group plays: show the group.
    let candidates = zones
        .iter()
        .filter(|zone| zone.grouped_into.is_none())
        .collect::<Vec<_>>();
    let previous = previous.and_then(|id| candidates.iter().find(|zone| zone.id == id));
    if let Some(previous) = previous
        && is_playing(previous)
    {
        return ZoneChoice::Zone(previous.id.clone());
    }
    if let Some(playing) = candidates.iter().find(|zone| is_playing(zone)) {
        return ZoneChoice::Zone(playing.id.clone());
    }
    if let Some(previous) = previous
        && has_track(previous)
    {
        return ZoneChoice::Zone(previous.id.clone());
    }
    candidates
        .iter()
        .find(|zone| has_track(zone))
        .map(|zone| ZoneChoice::Zone(zone.id.clone()))
        .unwrap_or(ZoneChoice::Nothing)
}

/// The zone each following display showed last, so it does not jump
/// between two paused zones from one poll to the next.
fn followed() -> &'static Mutex<HashMap<String, String>> {
    static FOLLOWED: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    FOLLOWED.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn last_followed(display_id: &str) -> Option<String> {
    followed()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(display_id)
        .cloned()
}

pub fn remember_followed(display_id: &str, zone_id: Option<&str>) {
    let mut followed = followed()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match zone_id {
        Some(zone_id) => {
            if followed.len() > 4_096 {
                followed.clear();
            }
            followed.insert(display_id.to_string(), zone_id.to_string());
        }
        None => {
            followed.remove(display_id);
        }
    }
}

// ---------------------------------------------------------------------------
// Format chip.

/// How a file's resolution reads on the quality chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FormatTier {
    /// Lossless above 48 kHz or above 16-bit.
    HiRes,
    /// Lossless at 44.1/48 kHz and 16-bit.
    Lossless,
    Lossy,
    Unknown,
}

/// The playing file's format, for the chip on the display.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TrackFormat {
    /// e.g. `FLAC 96 kHz/24-bit`, `MP3 320 kbps`.
    pub label: String,
    pub tier: FormatTier,
    pub codec: Option<String>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u8>,
    pub channels: Option<u8>,
}

fn codec_name(codec: &str) -> String {
    match codec {
        "flac" => "FLAC".into(),
        "alac" => "ALAC".into(),
        "aac" => "AAC".into(),
        "mp3" => "MP3".into(),
        "vorbis" => "Ogg Vorbis".into(),
        "opus" => "Opus".into(),
        "pcm" | "pcm_float" => "PCM".into(),
        "wavpack" => "WavPack".into(),
        "ape" => "APE".into(),
        "dsd" | "dst" => "DSD".into(),
        other => other.to_ascii_uppercase(),
    }
}

/// `44.1 kHz`, `96 kHz`.
fn rate_text(rate: u32) -> String {
    let khz = f64::from(rate) / 1000.0;
    if (khz - khz.round()).abs() < 1e-9 {
        format!("{} kHz", khz.round() as u64)
    } else {
        format!("{khz:.1} kHz")
    }
}

pub fn track_format(audio: Option<&crate::domain::AudioProperties>) -> Option<TrackFormat> {
    let audio = audio.filter(|audio| !audio.is_empty())?;
    let codec = audio.codec.as_deref().map(codec_name);
    let lossless = audio.lossless.or_else(|| {
        audio
            .codec
            .as_deref()
            .and_then(crate::domain::codec_is_lossless)
    });
    let tier = match lossless {
        Some(true) => {
            let high_rate = audio.sample_rate.is_some_and(|rate| rate > 48_000);
            let high_depth = audio.bit_depth.is_some_and(|bits| bits > 16);
            if high_rate || high_depth {
                FormatTier::HiRes
            } else {
                FormatTier::Lossless
            }
        }
        Some(false) => FormatTier::Lossy,
        None => FormatTier::Unknown,
    };
    let mut label = codec.clone().unwrap_or_else(|| "Audio".to_string());
    if lossless == Some(false) {
        if let Some(kbps) = audio.bitrate_kbps.filter(|kbps| *kbps > 0) {
            label.push_str(&format!(" {kbps} kbps"));
        } else if let Some(rate) = audio.sample_rate {
            label.push_str(&format!(" {}", rate_text(rate)));
        }
    } else {
        match (audio.sample_rate, audio.bit_depth) {
            (Some(rate), Some(bits)) => label.push_str(&format!(" {}/{bits}-bit", rate_text(rate))),
            (Some(rate), None) => label.push_str(&format!(" {}", rate_text(rate))),
            (None, Some(bits)) => label.push_str(&format!(" {bits}-bit")),
            (None, None) => {}
        }
    }
    Some(TrackFormat {
        label,
        tier,
        codec: audio.codec.clone(),
        sample_rate: audio.sample_rate,
        bit_depth: audio.bit_depth,
        channels: audio.channels,
    })
}

/// The release year from a `YYYY[-MM[-DD]]` date.
pub fn release_year(date: &str) -> Option<String> {
    let year = date.trim().get(..4)?;
    (year.chars().all(|character| character.is_ascii_digit()) && year != "0000")
        .then(|| year.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::AudioProperties;
    use crate::zones::{
        RepeatMode, ZoneCapabilities, ZoneKind, ZoneNowPlaying, ZoneQueue, ZoneSettingsView,
        ZoneSource,
    };
    use diesel::connection::SimpleConnection;

    fn pool() -> crate::persistence::connection::DbPool {
        crate::recommendation::test_support::radio_pool()
    }

    #[test]
    fn displays_are_per_listener_and_keys_resolve_until_removed() {
        let pool = pool();
        let mut connection = pool.get().unwrap();
        let (living, key) =
            create_display(&mut connection, 1, "Living room TV", None, LyricsMode::Auto).unwrap();
        assert!(key.starts_with(DISPLAY_KEY_PREFIX));
        assert_eq!(living.zone_id, None);
        create_display(
            &mut connection,
            2,
            "Kitchen",
            Some("client:x"),
            LyricsMode::Off,
        )
        .unwrap();

        assert_eq!(list_displays(&mut connection, 1).unwrap().len(), 1);
        let found = display_for_key(&mut connection, &key).unwrap().unwrap();
        assert_eq!(found.id, living.id);
        assert_eq!(found.user_id, 1);
        // Only the hash is stored.
        #[derive(QueryableByName)]
        struct Stored {
            #[diesel(sql_type = Text)]
            key_hash: String,
        }
        let stored = diesel::sql_query("SELECT key_hash FROM zone_display WHERE id = ?")
            .bind::<Text, _>(&living.id)
            .get_result::<Stored>(&mut connection)
            .unwrap();
        assert_ne!(stored.key_hash, key);
        assert_eq!(stored.key_hash, key_hash(&key));

        // Another listener cannot touch it.
        assert!(!delete_display(&mut connection, 2, &living.id).unwrap());
        assert!(
            update_display(&mut connection, 2, &living.id, &DisplayPatch::default())
                .unwrap()
                .is_none()
        );
        assert!(delete_display(&mut connection, 1, &living.id).unwrap());
        assert!(display_for_key(&mut connection, &key).unwrap().is_none());
        assert!(
            display_for_key(&mut connection, "not-a-key")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn patches_bind_follow_and_rename() {
        let pool = pool();
        let mut connection = pool.get().unwrap();
        let (display, _) =
            create_display(&mut connection, 1, "Tablet", None, LyricsMode::Auto).unwrap();
        let bound: DisplayPatch =
            serde_json::from_value(serde_json::json!({"zone_id": "client:abc", "lyrics": "off"}))
                .unwrap();
        let updated = update_display(&mut connection, 1, &display.id, &bound)
            .unwrap()
            .unwrap();
        assert_eq!(updated.zone_id.as_deref(), Some("client:abc"));
        assert_eq!(updated.lyrics, "off");
        assert_eq!(updated.name, "Tablet");

        let renamed: DisplayPatch =
            serde_json::from_value(serde_json::json!({"name": "Den"})).unwrap();
        let updated = update_display(&mut connection, 1, &display.id, &renamed)
            .unwrap()
            .unwrap();
        assert_eq!(updated.zone_id.as_deref(), Some("client:abc"));
        assert_eq!(updated.name, "Den");

        let follow: DisplayPatch =
            serde_json::from_value(serde_json::json!({"zone_id": null})).unwrap();
        let updated = update_display(&mut connection, 1, &display.id, &follow)
            .unwrap()
            .unwrap();
        assert_eq!(updated.zone_id, None);
    }

    #[test]
    fn a_listener_holds_a_bounded_number_of_displays() {
        let pool = pool();
        let mut connection = pool.get().unwrap();
        for index in 0..MAX_DISPLAYS_PER_USER {
            create_display(
                &mut connection,
                1,
                &format!("D{index}"),
                None,
                LyricsMode::Auto,
            )
            .unwrap();
        }
        assert!(matches!(
            create_display(&mut connection, 1, "One more", None, LyricsMode::Auto),
            Err(DisplayError::TooMany)
        ));
    }

    #[test]
    fn online_means_seen_in_the_last_half_minute() {
        let pool = pool();
        let mut connection = pool.get().unwrap();
        let (display, _) =
            create_display(&mut connection, 1, "TV", None, LyricsMode::Auto).unwrap();
        assert!(!display.view(now_utc()).online);
        touch_display(&mut connection, &display.id).unwrap();
        let seen = super::display(&mut connection, 1, &display.id)
            .unwrap()
            .unwrap();
        assert!(seen.view(now_utc()).online);
        connection
            .batch_execute("UPDATE zone_display SET last_seen_at = datetime('now', '-5 minutes');")
            .unwrap();
        let stale = super::display(&mut connection, 1, &display.id)
            .unwrap()
            .unwrap();
        assert!(!stale.view(now_utc()).online);
    }

    #[test]
    fn pairing_codes_are_claimed_once_and_deliver_the_key_once() {
        let now = Instant::now();
        let started = start_pairing("Bedroom TV", now).unwrap();
        assert_eq!(started.code.len(), 6);
        assert_eq!(
            pairing_status(&started.pairing_id, "wrong", now),
            Err(PairingError::NotFound)
        );
        assert_eq!(
            pairing_status(&started.pairing_id, &started.secret, now),
            Ok(PairingStatus::Pending {
                name: "Bedroom TV".into()
            })
        );
        let (claimed, name) = claim_code(&started.code, now).unwrap();
        assert_eq!(claimed, started.pairing_id);
        assert_eq!(name, "Bedroom TV");
        // A second approver cannot claim the same code.
        assert!(claim_code(&started.code, now).is_none());
        let view = DisplayView {
            id: "d1".into(),
            name: "Bedroom TV".into(),
            zone_id: None,
            lyrics: LyricsMode::Auto,
            created_at: "2026-10-27 10:00:00".into(),
            last_seen_at: None,
            online: false,
        };
        assert!(complete_pairing(&claimed, view.clone(), "mjd_key".into()));
        assert_eq!(
            pairing_status(&started.pairing_id, &started.secret, now),
            Ok(PairingStatus::Approved {
                display: view,
                key: "mjd_key".into()
            })
        );
        // Delivered once.
        assert_eq!(
            pairing_status(&started.pairing_id, &started.secret, now),
            Err(PairingError::NotFound)
        );
    }

    #[test]
    fn pairing_codes_expire() {
        let now = Instant::now();
        let started = start_pairing("Old TV", now).unwrap();
        let later = now + PAIRING_LIFETIME + Duration::from_secs(1);
        assert!(claim_code(&started.code, later).is_none());
        assert_eq!(
            pairing_status(&started.pairing_id, &started.secret, later),
            Err(PairingError::NotFound)
        );
    }

    #[test]
    fn codes_accept_spacing_only() {
        assert_eq!(normalized_code("123 456").as_deref(), Some("123456"));
        assert_eq!(normalized_code("123-456").as_deref(), Some("123456"));
        assert_eq!(normalized_code("12345"), None);
        assert_eq!(normalized_code("12a456"), None);
        assert_eq!(normalized_code("1234567"), None);
    }

    fn zone(id: &str, status: ZoneStatus, playing: bool) -> Zone {
        Zone {
            id: id.to_string(),
            kind: ZoneKind::Web,
            ecosystem: "parson",
            name: id.to_string(),
            icon: "speaker".to_string(),
            device: String::new(),
            status,
            source: ZoneSource::Parson,
            now_playing: playing.then(|| ZoneNowPlaying {
                song_id: Some(format!("{id}-song")),
                title: "Song".into(),
                artist: "Artist".into(),
                album: "Album".into(),
                album_id: None,
                cover_url: None,
                duration_ms: 1_000,
            }),
            queue: ZoneQueue {
                song_ids: Vec::new(),
                current_position: 0,
            },
            repeat: RepeatMode::Off,
            shuffle: false,
            position_ms: 0,
            duration_ms: 1_000,
            volume: None,
            group: None,
            grouped_into: None,
            capabilities: ZoneCapabilities::default(),
            sleep_timer: None,
            settings: ZoneSettingsView::unsupported(id),
            output: None,
            endpoints: Vec::new(),
            notes: Vec::new(),
        }
    }

    #[test]
    fn a_bound_display_shows_its_zone_or_says_it_is_gone() {
        let zones = [
            zone("a", ZoneStatus::Paused, true),
            zone("b", ZoneStatus::Playing, true),
        ];
        assert_eq!(
            choose_zone(&zones, Some("a"), None),
            ZoneChoice::Zone("a".into())
        );
        assert_eq!(choose_zone(&zones, Some("gone"), None), ZoneChoice::Missing);
    }

    #[test]
    fn a_following_display_prefers_playing_and_does_not_flap() {
        let zones = [
            zone("paused", ZoneStatus::Paused, true),
            zone("playing", ZoneStatus::Playing, true),
            zone("empty", ZoneStatus::Idle, false),
        ];
        assert_eq!(
            choose_zone(&zones, None, None),
            ZoneChoice::Zone("playing".into())
        );
        // A paused zone it showed gives way to one that plays.
        assert_eq!(
            choose_zone(&zones, None, Some("paused")),
            ZoneChoice::Zone("playing".into())
        );
        // With nothing playing it stays on the paused zone it showed.
        let quiet = [
            zone("first", ZoneStatus::Paused, true),
            zone("second", ZoneStatus::Paused, true),
        ];
        assert_eq!(
            choose_zone(&quiet, None, Some("second")),
            ZoneChoice::Zone("second".into())
        );
        assert_eq!(
            choose_zone(&quiet, None, None),
            ZoneChoice::Zone("first".into())
        );
        assert_eq!(
            choose_zone(&[zone("empty", ZoneStatus::Idle, false)], None, None),
            ZoneChoice::Nothing
        );
    }

    #[test]
    fn a_following_display_shows_a_linked_group_not_its_members() {
        let mut member = zone("member", ZoneStatus::Playing, true);
        member.grouped_into = Some("linked:1".into());
        let group = zone("linked:1", ZoneStatus::Playing, true);
        assert_eq!(
            choose_zone(&[member, group], None, None),
            ZoneChoice::Zone("linked:1".into())
        );
    }

    #[test]
    fn the_format_chip_reads_like_a_spec_sheet() {
        let hires = AudioProperties {
            codec: Some("flac".into()),
            sample_rate: Some(96_000),
            bit_depth: Some(24),
            channels: Some(2),
            lossless: Some(true),
            ..AudioProperties::default()
        };
        let chip = track_format(Some(&hires)).unwrap();
        assert_eq!(chip.label, "FLAC 96 kHz/24-bit");
        assert_eq!(chip.tier, FormatTier::HiRes);

        let cd = AudioProperties {
            codec: Some("flac".into()),
            sample_rate: Some(44_100),
            bit_depth: Some(16),
            ..AudioProperties::default()
        };
        let chip = track_format(Some(&cd)).unwrap();
        assert_eq!(chip.label, "FLAC 44.1 kHz/16-bit");
        assert_eq!(chip.tier, FormatTier::Lossless);

        let mp3 = AudioProperties {
            codec: Some("mp3".into()),
            sample_rate: Some(44_100),
            bitrate_kbps: Some(320),
            lossless: Some(false),
            ..AudioProperties::default()
        };
        let chip = track_format(Some(&mp3)).unwrap();
        assert_eq!(chip.label, "MP3 320 kbps");
        assert_eq!(chip.tier, FormatTier::Lossy);
        assert!(track_format(Some(&AudioProperties::default())).is_none());
        assert!(track_format(None).is_none());
    }

    #[test]
    fn release_years_come_from_dates() {
        assert_eq!(release_year("2008-05-05").as_deref(), Some("2008"));
        assert_eq!(release_year("1925").as_deref(), Some("1925"));
        assert_eq!(release_year(""), None);
        assert_eq!(release_year("0000-00-00"), None);
        assert_eq!(release_year("n/a"), None);
    }

    #[test]
    fn names_are_one_trimmed_line() {
        assert_eq!(
            clean_name("  Living room  ").as_deref(),
            Some("Living room")
        );
        assert_eq!(clean_name(""), None);
        assert_eq!(clean_name("a\nb"), None);
        assert_eq!(clean_name(&"x".repeat(100)).unwrap().chars().count(), 60);
    }
}
