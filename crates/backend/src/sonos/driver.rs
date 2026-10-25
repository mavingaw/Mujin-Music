//! Sonos controller: discovery, topology, state polling, eventing and
//! transport/volume/grouping commands over UPnP.
//!
//! The controller knows nothing about Parson queues. The zone service maps a
//! Parson queue onto a group's Sonos queue through [`SonosController::load_queue`]
//! and reads progress back from [`SonosGroup::track_number`].

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

use super::description::{DeviceDescription, parse_device_description, valid_player_uuid};
use super::soap::{Service, SoapClient, SoapError};
use super::ssdp;
use super::topology::{TopologyGroup, base_url_of, parse_zone_group_state};
use super::xml::{format_duration, parse_duration};

const SUBSCRIPTION_SECONDS: u32 = 300;
const MAX_PLAYERS: usize = 64;

/// How the controller finds players.
#[derive(Clone, Debug)]
pub struct SonosConfig {
    /// Where `M-SEARCH` goes; normally the UPnP multicast group.
    pub ssdp_targets: Vec<SocketAddr>,
    /// Players to probe directly (for networks that drop multicast), as
    /// `host` or `host:port`.
    pub static_hosts: Vec<String>,
    pub search_timeout: Duration,
}

impl SonosConfig {
    /// Reads `PARSON_SONOS` (`off`/`0`/`false` disables Sonos) and
    /// `PARSON_SONOS_HOSTS` (comma separated player addresses).
    pub fn from_env() -> Option<Self> {
        let enabled = std::env::var("PARSON_SONOS")
            .map(|value| {
                !matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "0" | "off" | "false" | "no" | "disabled"
                )
            })
            .unwrap_or(true);
        if !enabled {
            return None;
        }
        let static_hosts = std::env::var("PARSON_SONOS_HOSTS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|host| !host.is_empty())
            .map(str::to_string)
            .collect();
        Some(Self {
            ssdp_targets: vec![ssdp::SSDP_MULTICAST],
            static_hosts,
            search_timeout: Duration::from_millis(1_500),
        })
    }
}

/// UPnP transport state of a group coordinator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportState {
    Playing,
    Paused,
    Stopped,
    Transitioning,
    Unknown,
}

impl TransportState {
    pub fn parse(value: &str) -> Self {
        match value {
            "PLAYING" => Self::Playing,
            "PAUSED_PLAYBACK" => Self::Paused,
            "STOPPED" | "NO_MEDIA_PRESENT" => Self::Stopped,
            "TRANSITIONING" => Self::Transitioning,
            _ => Self::Unknown,
        }
    }
}

/// One Sonos player (a room, or the primary of a stereo pair).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SonosPlayer {
    pub uuid: String,
    pub room_name: String,
    pub model_name: String,
    pub display_name: String,
    #[serde(skip)]
    pub base_url: String,
    pub volume: u8,
    pub muted: bool,
}

/// A Sonos group as Parson sees it: coordinator plus visible members.
#[derive(Clone, Debug, PartialEq)]
pub struct SonosGroup {
    pub coordinator: String,
    pub topology_id: String,
    pub members: Vec<String>,
    pub name: String,
    pub transport: TransportState,
    /// 1-based track number in the Sonos queue; 0 when nothing is queued.
    pub track_number: u32,
    pub track_uri: String,
    pub track_title: Option<String>,
    pub position_ms: i64,
    pub duration_ms: i64,
    /// Unix milliseconds when the position was read.
    pub polled_at_ms: i64,
    pub group_volume: u8,
    pub group_muted: bool,
    /// AVTransport `PlayMode` (NORMAL, REPEAT_ALL, SHUFFLE, ...).
    pub play_mode: String,
}

/// Transport actions on a group coordinator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    SeekTime(i64),
    /// 0-based index into the Sonos queue.
    SeekTrack(u32),
}

#[derive(Default)]
struct Inner {
    players: HashMap<String, SonosPlayer>,
    groups: Vec<SonosGroup>,
    subscriptions: HashMap<(String, &'static str), Instant>,
    last_error: Option<String>,
    discovered_at: Option<Instant>,
}

/// The Sonos side of zones.
pub struct SonosController {
    config: SonosConfig,
    soap: SoapClient,
    inner: Mutex<Inner>,
    changed: tokio::sync::broadcast::Sender<()>,
}

fn unix_millis() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl SonosController {
    pub fn new(config: SonosConfig) -> Result<Self, String> {
        Ok(Self {
            config,
            soap: SoapClient::new()?,
            inner: Mutex::new(Inner::default()),
            changed: tokio::sync::broadcast::channel(64).0,
        })
    }

    /// Fires whenever players, groups or their state change.
    pub fn subscribe_changes(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.changed.subscribe()
    }

    fn notify(&self) {
        let _ = self.changed.send(());
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn players(&self) -> Vec<SonosPlayer> {
        let mut players: Vec<_> = self.lock().players.values().cloned().collect();
        players.sort_by(|left, right| left.room_name.cmp(&right.room_name));
        players
    }

    pub fn groups(&self) -> Vec<SonosGroup> {
        self.lock().groups.clone()
    }

    pub fn player(&self, uuid: &str) -> Option<SonosPlayer> {
        self.lock().players.get(uuid).cloned()
    }

    /// The group whose coordinator or member is `uuid`.
    pub fn group_of(&self, uuid: &str) -> Option<SonosGroup> {
        self.lock()
            .groups
            .iter()
            .find(|group| group.coordinator == uuid || group.members.iter().any(|m| m == uuid))
            .cloned()
    }

    pub fn last_error(&self) -> Option<String> {
        self.lock().last_error.clone()
    }

    pub fn discovered_recently(&self, within: Duration) -> bool {
        self.lock()
            .discovered_at
            .is_some_and(|at| at.elapsed() < within)
    }

    fn base_url(&self, uuid: &str) -> Result<String, SoapError> {
        self.lock()
            .players
            .get(uuid)
            .map(|player| player.base_url.clone())
            .ok_or_else(|| SoapError::Transport(format!("unknown Sonos player {uuid}")))
    }

    async fn call(
        &self,
        uuid: &str,
        service: Service,
        action: &str,
        arguments: &[(&str, &str)],
    ) -> Result<HashMap<String, String>, SoapError> {
        let base = self.base_url(uuid)?;
        let result = self.soap.call(&base, service, action, arguments).await;
        if let Err(error) = &result {
            tracing::debug!(%error, uuid, action, "Sonos SOAP call failed");
        }
        result
    }

    /// Finds players by SSDP (plus configured hosts) and reads topology.
    pub async fn discover(&self) -> Result<usize, String> {
        let mut locations: Vec<String> = Vec::new();
        match ssdp::search(&self.config.ssdp_targets, self.config.search_timeout).await {
            Ok(responses) => locations.extend(responses.into_iter().map(|answer| answer.location)),
            Err(error) => tracing::debug!(%error, "SSDP search failed"),
        }
        for host in &self.config.static_hosts {
            let authority = if host.contains(':') {
                host.clone()
            } else {
                format!("{host}:1400")
            };
            locations.push(format!("http://{authority}/xml/device_description.xml"));
        }
        let mut seen = HashSet::new();
        locations.retain(|location| seen.insert(location.clone()));
        locations.truncate(MAX_PLAYERS);
        let descriptions = futures::future::join_all(
            locations
                .iter()
                .map(|location| self.describe(location.clone())),
        )
        .await;
        let mut found = 0;
        {
            let mut inner = self.lock();
            for (description, base_url) in descriptions.into_iter().flatten() {
                found += 1;
                upsert_player(&mut inner.players, description, base_url);
            }
            inner.discovered_at = Some(Instant::now());
        }
        self.refresh_topology().await?;
        let count = self.lock().players.len();
        tracing::debug!(found, players = count, "Sonos discovery finished");
        Ok(count)
    }

    async fn describe(&self, location: String) -> Option<(DeviceDescription, String)> {
        let base_url = base_url_of(&location)?;
        let document = self.soap.get_text(&location).await.ok()?;
        match parse_device_description(&document) {
            Ok(description) => Some((description, base_url)),
            Err(error) => {
                tracing::debug!(%error, location, "ignoring non-Sonos UPnP device");
                None
            }
        }
    }

    /// Reads `GetZoneGroupState` from any reachable player.
    pub async fn refresh_topology(&self) -> Result<(), String> {
        let candidates: Vec<String> = self.lock().players.keys().cloned().collect();
        if candidates.is_empty() {
            let changed = {
                let mut inner = self.lock();
                let changed = !inner.groups.is_empty();
                inner.groups.clear();
                changed
            };
            if changed {
                self.notify();
            }
            return Ok(());
        }
        let mut topology = None;
        let mut last_error = None;
        for uuid in &candidates {
            match self
                .call(uuid, Service::ZoneGroupTopology, "GetZoneGroupState", &[])
                .await
            {
                Ok(output) => {
                    let state = output.get("ZoneGroupState").cloned().unwrap_or_default();
                    match parse_zone_group_state(&state) {
                        Ok(groups) => {
                            topology = Some(groups);
                            break;
                        }
                        Err(error) => last_error = Some(error),
                    }
                }
                Err(error) => last_error = Some(error.to_string()),
            }
        }
        let Some(topology) = topology else {
            let message = last_error.unwrap_or_else(|| "no Sonos player answered".to_string());
            self.lock().last_error = Some(message.clone());
            return Err(message);
        };
        // Players that never answered SSDP (or were asleep) still show in topology.
        let missing: Vec<String> = {
            let inner = self.lock();
            topology
                .iter()
                .flat_map(|group| group.visible_members())
                .filter(|member| !inner.players.contains_key(&member.uuid))
                .map(|member| member.location.clone())
                .collect()
        };
        let described =
            futures::future::join_all(missing.into_iter().map(|location| self.describe(location)))
                .await;
        let changed = {
            let mut inner = self.lock();
            for (description, base_url) in described.into_iter().flatten() {
                upsert_player(&mut inner.players, description, base_url);
            }
            let next = merge_groups(&inner.players, &inner.groups, &topology);
            let changed = next != inner.groups;
            inner.groups = next;
            inner.last_error = None;
            changed
        };
        if changed {
            self.notify();
        }
        Ok(())
    }

    /// Reads transport, position and volume for one group.
    pub async fn poll_group(&self, coordinator: &str) -> Result<(), SoapError> {
        let transport = self
            .call(
                coordinator,
                Service::AVTransport,
                "GetTransportInfo",
                &[("InstanceID", "0")],
            )
            .await?;
        let position = self
            .call(
                coordinator,
                Service::AVTransport,
                "GetPositionInfo",
                &[("InstanceID", "0")],
            )
            .await?;
        let group_volume = self
            .call(
                coordinator,
                Service::GroupRenderingControl,
                "GetGroupVolume",
                &[("InstanceID", "0")],
            )
            .await
            .ok()
            .and_then(|output| output.get("CurrentVolume").and_then(|v| v.parse().ok()));
        let group_muted = self
            .call(
                coordinator,
                Service::GroupRenderingControl,
                "GetGroupMute",
                &[("InstanceID", "0")],
            )
            .await
            .ok()
            .and_then(|output| output.get("CurrentMute").map(|value| value == "1"));
        let play_mode = self
            .call(
                coordinator,
                Service::AVTransport,
                "GetTransportSettings",
                &[("InstanceID", "0")],
            )
            .await
            .ok()
            .and_then(|output| output.get("PlayMode").cloned());
        let members = self
            .group_of(coordinator)
            .map(|group| group.members)
            .unwrap_or_else(|| vec![coordinator.to_string()]);
        let mut member_volumes = Vec::new();
        for member in &members {
            let volume = self
                .call(
                    member,
                    Service::RenderingControl,
                    "GetVolume",
                    &[("InstanceID", "0"), ("Channel", "Master")],
                )
                .await
                .ok()
                .and_then(|output| output.get("CurrentVolume").and_then(|v| v.parse().ok()));
            let muted = self
                .call(
                    member,
                    Service::RenderingControl,
                    "GetMute",
                    &[("InstanceID", "0"), ("Channel", "Master")],
                )
                .await
                .ok()
                .and_then(|output| output.get("CurrentMute").map(|value| value == "1"));
            member_volumes.push((member.clone(), volume, muted));
        }
        let state = transport
            .get("CurrentTransportState")
            .map(|value| TransportState::parse(value))
            .unwrap_or(TransportState::Unknown);
        let track_number = position
            .get("Track")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        let track_uri = position.get("TrackURI").cloned().unwrap_or_default();
        let track_title = position
            .get("TrackMetaData")
            .and_then(|metadata| super::didl::title(metadata));
        let position_ms = position
            .get("RelTime")
            .and_then(|value| parse_duration(value))
            .unwrap_or(0);
        let duration_ms = position
            .get("TrackDuration")
            .and_then(|value| parse_duration(value))
            .unwrap_or(0);
        let changed = {
            let mut inner = self.lock();
            for (member, volume, muted) in member_volumes {
                if let Some(player) = inner.players.get_mut(&member) {
                    if let Some(volume) = volume {
                        player.volume = volume;
                    }
                    if let Some(muted) = muted {
                        player.muted = muted;
                    }
                }
            }
            let average = average_volume(&inner.players, &members);
            let Some(group) = inner
                .groups
                .iter_mut()
                .find(|group| group.coordinator == coordinator)
            else {
                return Ok(());
            };
            let before = group.clone();
            group.transport = state;
            group.track_number = track_number;
            group.track_uri = track_uri;
            group.track_title = track_title;
            group.position_ms = position_ms;
            group.duration_ms = duration_ms;
            group.polled_at_ms = unix_millis();
            group.group_volume = group_volume.unwrap_or(average);
            group.group_muted = group_muted.unwrap_or(false);
            if let Some(play_mode) = play_mode {
                group.play_mode = play_mode;
            }
            let mut comparable = before.clone();
            comparable.polled_at_ms = group.polled_at_ms;
            // Position drift while playing is expected and not a change.
            if group.transport == TransportState::Playing {
                comparable.position_ms = group.position_ms;
            }
            comparable != *group
        };
        if changed {
            self.notify();
        }
        Ok(())
    }

    pub async fn poll_all(&self) {
        let coordinators: Vec<String> = self
            .groups()
            .into_iter()
            .map(|group| group.coordinator)
            .collect();
        for coordinator in coordinators {
            let _ = self.poll_group(&coordinator).await;
        }
    }

    /// Replaces the group's Sonos queue with `tracks` (URI and DIDL-Lite) and
    /// starts at `start_index` / `position_ms`.
    pub async fn load_queue(
        &self,
        coordinator: &str,
        tracks: &[(String, String)],
        start_index: u32,
        position_ms: i64,
        play: bool,
    ) -> Result<(), SoapError> {
        if tracks.is_empty() {
            return Err(SoapError::Transport("nothing to queue".to_string()));
        }
        self.call(
            coordinator,
            Service::AVTransport,
            "RemoveAllTracksFromQueue",
            &[("InstanceID", "0")],
        )
        .await?;
        for (uri, metadata) in tracks {
            self.call(
                coordinator,
                Service::AVTransport,
                "AddURIToQueue",
                &[
                    ("InstanceID", "0"),
                    ("EnqueuedURI", uri),
                    ("EnqueuedURIMetaData", metadata),
                    ("DesiredFirstTrackNumberEnqueued", "0"),
                    ("EnqueueAsNext", "0"),
                ],
            )
            .await?;
        }
        let queue_uri = format!("x-rincon-queue:{coordinator}#0");
        self.call(
            coordinator,
            Service::AVTransport,
            "SetAVTransportURI",
            &[
                ("InstanceID", "0"),
                ("CurrentURI", &queue_uri),
                ("CurrentURIMetaData", ""),
            ],
        )
        .await?;
        let track = (start_index.min(tracks.len() as u32 - 1) + 1).to_string();
        self.call(
            coordinator,
            Service::AVTransport,
            "Seek",
            &[
                ("InstanceID", "0"),
                ("Unit", "TRACK_NR"),
                ("Target", &track),
            ],
        )
        .await?;
        if position_ms > 1_000 {
            let target = format_duration(position_ms);
            // Some sources refuse REL_TIME before playback starts; not fatal.
            let _ = self
                .call(
                    coordinator,
                    Service::AVTransport,
                    "Seek",
                    &[
                        ("InstanceID", "0"),
                        ("Unit", "REL_TIME"),
                        ("Target", &target),
                    ],
                )
                .await;
        }
        if play {
            self.transport(coordinator, Transport::Play).await?;
        }
        let _ = self.poll_group(coordinator).await;
        Ok(())
    }

    /// Appends tracks to the end of the group's Sonos queue.
    pub async fn append_tracks(
        &self,
        coordinator: &str,
        tracks: &[(String, String)],
    ) -> Result<(), SoapError> {
        for (uri, metadata) in tracks {
            self.call(
                coordinator,
                Service::AVTransport,
                "AddURIToQueue",
                &[
                    ("InstanceID", "0"),
                    ("EnqueuedURI", uri),
                    ("EnqueuedURIMetaData", metadata),
                    ("DesiredFirstTrackNumberEnqueued", "0"),
                    ("EnqueueAsNext", "0"),
                ],
            )
            .await?;
        }
        Ok(())
    }

    /// Removes the track at 0-based `index` from the Sonos queue.
    pub async fn remove_track(&self, coordinator: &str, index: u32) -> Result<(), SoapError> {
        let object = format!("Q:0/{}", index + 1);
        self.call(
            coordinator,
            Service::AVTransport,
            "RemoveTrackFromQueue",
            &[
                ("InstanceID", "0"),
                ("ObjectID", &object),
                ("UpdateID", "0"),
            ],
        )
        .await
        .map(|_| ())
    }

    /// Removes `count` tracks starting at 0-based `start`.
    pub async fn remove_track_range(
        &self,
        coordinator: &str,
        start: u32,
        count: u32,
    ) -> Result<(), SoapError> {
        if count == 0 {
            return Ok(());
        }
        let start = (start + 1).to_string();
        let count = count.to_string();
        self.call(
            coordinator,
            Service::AVTransport,
            "RemoveTrackRangeFromQueue",
            &[
                ("InstanceID", "0"),
                ("UpdateID", "0"),
                ("StartingIndex", &start),
                ("NumberOfTracks", &count),
            ],
        )
        .await
        .map(|_| ())
    }

    /// Moves the track at 0-based `from` so it ends up at 0-based `to`.
    pub async fn move_track(&self, coordinator: &str, from: u32, to: u32) -> Result<(), SoapError> {
        if from == to {
            return Ok(());
        }
        // Sonos inserts before a 1-based position counted before removal.
        let insert_before = if to > from { to + 2 } else { to + 1 };
        let starting = (from + 1).to_string();
        let insert_before = insert_before.to_string();
        self.call(
            coordinator,
            Service::AVTransport,
            "ReorderTracksInQueue",
            &[
                ("InstanceID", "0"),
                ("StartingIndex", &starting),
                ("NumberOfTracks", "1"),
                ("InsertBefore", &insert_before),
                ("UpdateID", "0"),
            ],
        )
        .await
        .map(|_| ())
    }

    /// Sets the AVTransport play mode (repeat; Parson shuffles itself).
    pub async fn set_play_mode(&self, coordinator: &str, mode: &str) -> Result<(), SoapError> {
        self.call(
            coordinator,
            Service::AVTransport,
            "SetPlayMode",
            &[("InstanceID", "0"), ("NewPlayMode", mode)],
        )
        .await?;
        let _ = self.poll_group(coordinator).await;
        Ok(())
    }

    /// Sonos crossfade (fixed length, set per group on the coordinator).
    pub async fn set_crossfade(&self, coordinator: &str, enabled: bool) -> Result<(), SoapError> {
        self.call(
            coordinator,
            Service::AVTransport,
            "SetCrossfadeMode",
            &[
                ("InstanceID", "0"),
                ("CrossfadeMode", if enabled { "1" } else { "0" }),
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn transport(&self, coordinator: &str, action: Transport) -> Result<(), SoapError> {
        let result = match action {
            Transport::Play => {
                self.call(
                    coordinator,
                    Service::AVTransport,
                    "Play",
                    &[("InstanceID", "0"), ("Speed", "1")],
                )
                .await
            }
            Transport::Pause => {
                self.call(
                    coordinator,
                    Service::AVTransport,
                    "Pause",
                    &[("InstanceID", "0")],
                )
                .await
            }
            Transport::Stop => {
                self.call(
                    coordinator,
                    Service::AVTransport,
                    "Stop",
                    &[("InstanceID", "0")],
                )
                .await
            }
            Transport::Next => {
                self.call(
                    coordinator,
                    Service::AVTransport,
                    "Next",
                    &[("InstanceID", "0")],
                )
                .await
            }
            Transport::Previous => {
                self.call(
                    coordinator,
                    Service::AVTransport,
                    "Previous",
                    &[("InstanceID", "0")],
                )
                .await
            }
            Transport::SeekTime(milliseconds) => {
                let target = format_duration(milliseconds);
                self.call(
                    coordinator,
                    Service::AVTransport,
                    "Seek",
                    &[
                        ("InstanceID", "0"),
                        ("Unit", "REL_TIME"),
                        ("Target", &target),
                    ],
                )
                .await
            }
            Transport::SeekTrack(index) => {
                let target = (index + 1).to_string();
                self.call(
                    coordinator,
                    Service::AVTransport,
                    "Seek",
                    &[
                        ("InstanceID", "0"),
                        ("Unit", "TRACK_NR"),
                        ("Target", &target),
                    ],
                )
                .await
            }
        };
        result?;
        let _ = self.poll_group(coordinator).await;
        Ok(())
    }

    /// Group volume (Sonos keeps member ratios).
    pub async fn set_group_volume(&self, coordinator: &str, level: u8) -> Result<(), SoapError> {
        let level = level.min(100).to_string();
        self.call(
            coordinator,
            Service::GroupRenderingControl,
            "SnapshotGroupVolume",
            &[("InstanceID", "0")],
        )
        .await?;
        self.call(
            coordinator,
            Service::GroupRenderingControl,
            "SetGroupVolume",
            &[("InstanceID", "0"), ("DesiredVolume", &level)],
        )
        .await?;
        let _ = self.poll_group(coordinator).await;
        Ok(())
    }

    pub async fn set_group_mute(&self, coordinator: &str, muted: bool) -> Result<(), SoapError> {
        self.call(
            coordinator,
            Service::GroupRenderingControl,
            "SetGroupMute",
            &[
                ("InstanceID", "0"),
                ("DesiredMute", if muted { "1" } else { "0" }),
            ],
        )
        .await?;
        let _ = self.poll_group(coordinator).await;
        Ok(())
    }

    /// Volume of one room inside a group.
    pub async fn set_player_volume(&self, uuid: &str, level: u8) -> Result<(), SoapError> {
        let level = level.min(100).to_string();
        self.call(
            uuid,
            Service::RenderingControl,
            "SetVolume",
            &[
                ("InstanceID", "0"),
                ("Channel", "Master"),
                ("DesiredVolume", &level),
            ],
        )
        .await?;
        if let Some(group) = self.group_of(uuid) {
            let _ = self.poll_group(&group.coordinator).await;
        }
        Ok(())
    }

    pub async fn set_player_mute(&self, uuid: &str, muted: bool) -> Result<(), SoapError> {
        self.call(
            uuid,
            Service::RenderingControl,
            "SetMute",
            &[
                ("InstanceID", "0"),
                ("Channel", "Master"),
                ("DesiredMute", if muted { "1" } else { "0" }),
            ],
        )
        .await?;
        if let Some(group) = self.group_of(uuid) {
            let _ = self.poll_group(&group.coordinator).await;
        }
        Ok(())
    }

    /// Native Sonos grouping: `member` follows `coordinator` sample-synced.
    pub async fn join(&self, member: &str, coordinator: &str) -> Result<(), SoapError> {
        if member == coordinator {
            return Ok(());
        }
        let uri = format!("x-rincon:{coordinator}");
        self.call(
            member,
            Service::AVTransport,
            "SetAVTransportURI",
            &[
                ("InstanceID", "0"),
                ("CurrentURI", &uri),
                ("CurrentURIMetaData", ""),
            ],
        )
        .await?;
        Ok(())
    }

    /// Takes `member` out of its group.
    pub async fn leave(&self, member: &str) -> Result<(), SoapError> {
        self.call(
            member,
            Service::AVTransport,
            "BecomeCoordinatorOfStandaloneGroup",
            &[("InstanceID", "0")],
        )
        .await?;
        Ok(())
    }

    /// The address of this server as seen from `uuid` (for media URLs and
    /// event callbacks): the local interface that routes to the player.
    pub fn local_origin_for(&self, uuid: &str, port: u16) -> Option<String> {
        let base = self.base_url(uuid).ok()?;
        let authority = base.strip_prefix("http://")?;
        let target = authority.to_socket_addrs().ok()?.next()?;
        let local = local_address_towards(target)?;
        Some(match local {
            IpAddr::V4(address) => format!("http://{address}:{port}"),
            IpAddr::V6(address) => format!("http://[{address}]:{port}"),
        })
    }

    /// Subscribes (and renews) AVTransport and RenderingControl events so
    /// players push state changes instead of waiting for the next poll.
    pub async fn ensure_subscriptions(&self, port: u16) {
        let players: Vec<(String, String)> = self
            .lock()
            .players
            .values()
            .map(|player| (player.uuid.clone(), player.base_url.clone()))
            .collect();
        for (uuid, base_url) in players {
            let Some(origin) = self.local_origin_for(&uuid, port) else {
                continue;
            };
            for service in [Service::AVTransport, Service::RenderingControl] {
                let key = (uuid.clone(), service.event_path());
                let due = self
                    .lock()
                    .subscriptions
                    .get(&key)
                    .is_none_or(|renew_at| Instant::now() >= *renew_at);
                if !due {
                    continue;
                }
                let callback = format!("{origin}/api/v1/zones/sonos/notify/{uuid}");
                match self
                    .soap
                    .subscribe(&base_url, service, &callback, SUBSCRIPTION_SECONDS)
                    .await
                {
                    Ok(_sid) => {
                        let renew_at = Instant::now()
                            + Duration::from_secs(u64::from(SUBSCRIPTION_SECONDS) * 4 / 5);
                        self.lock().subscriptions.insert(key, renew_at);
                    }
                    Err(error) => {
                        tracing::debug!(%error, uuid, "Sonos event subscription failed");
                        // Retry later; polling still covers state.
                        self.lock()
                            .subscriptions
                            .insert(key, Instant::now() + Duration::from_secs(60));
                    }
                }
            }
        }
    }

    pub fn knows_player(&self, uuid: &str) -> bool {
        valid_player_uuid(uuid) && self.lock().players.contains_key(uuid)
    }
}

fn upsert_player(
    players: &mut HashMap<String, SonosPlayer>,
    description: DeviceDescription,
    base_url: String,
) {
    let entry = players
        .entry(description.uuid.clone())
        .or_insert_with(|| SonosPlayer {
            uuid: description.uuid.clone(),
            room_name: String::new(),
            model_name: String::new(),
            display_name: String::new(),
            base_url: String::new(),
            volume: 0,
            muted: false,
        });
    entry.room_name = description.room_name;
    entry.model_name = description.model_name;
    entry.display_name = description.display_name;
    entry.base_url = base_url;
}

fn average_volume(players: &HashMap<String, SonosPlayer>, members: &[String]) -> u8 {
    let volumes: Vec<u32> = members
        .iter()
        .filter_map(|member| players.get(member))
        .map(|player| u32::from(player.volume))
        .collect();
    if volumes.is_empty() {
        0
    } else {
        (volumes.iter().sum::<u32>() / volumes.len() as u32) as u8
    }
}

/// New group list from topology, keeping polled state for unchanged coordinators.
fn merge_groups(
    players: &HashMap<String, SonosPlayer>,
    previous: &[SonosGroup],
    topology: &[TopologyGroup],
) -> Vec<SonosGroup> {
    topology
        .iter()
        .filter(|group| players.contains_key(&group.coordinator))
        .filter(|group| group.visible_members().next().is_some())
        .map(|group| {
            let members: Vec<String> = group
                .visible_members()
                .map(|member| member.uuid.clone())
                .collect();
            let name = group
                .visible_members()
                .map(|member| {
                    players
                        .get(&member.uuid)
                        .map(|player| player.room_name.clone())
                        .unwrap_or_else(|| member.zone_name.clone())
                })
                .collect::<Vec<_>>()
                .join(" + ");
            let mut next = previous
                .iter()
                .find(|known| known.coordinator == group.coordinator)
                .cloned()
                .unwrap_or(SonosGroup {
                    coordinator: group.coordinator.clone(),
                    topology_id: String::new(),
                    members: Vec::new(),
                    name: String::new(),
                    transport: TransportState::Unknown,
                    track_number: 0,
                    track_uri: String::new(),
                    track_title: None,
                    position_ms: 0,
                    duration_ms: 0,
                    polled_at_ms: 0,
                    group_volume: 0,
                    group_muted: false,
                    play_mode: "NORMAL".to_string(),
                });
            next.topology_id = group.id.clone();
            next.members = members;
            next.name = name;
            next
        })
        .collect()
}

fn local_address_towards(target: SocketAddr) -> Option<IpAddr> {
    let bind: SocketAddr = if target.is_ipv4() {
        (std::net::Ipv4Addr::UNSPECIFIED, 0).into()
    } else {
        (std::net::Ipv6Addr::UNSPECIFIED, 0).into()
    };
    let socket = std::net::UdpSocket::bind(bind).ok()?;
    socket.connect(target).ok()?;
    Some(socket.local_addr().ok()?.ip())
}

#[cfg(test)]
mod tests {
    use super::TransportState;

    #[test]
    fn transport_states_map_from_upnp() {
        assert_eq!(TransportState::parse("PLAYING"), TransportState::Playing);
        assert_eq!(
            TransportState::parse("PAUSED_PLAYBACK"),
            TransportState::Paused
        );
        assert_eq!(
            TransportState::parse("NO_MEDIA_PRESENT"),
            TransportState::Stopped
        );
        assert_eq!(TransportState::parse("??"), TransportState::Unknown);
    }
}
