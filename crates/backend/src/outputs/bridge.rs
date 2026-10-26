//! Parson Bridge zones: the audio devices of other machines.
//!
//! The server connects to each bridge it finds on the local network
//! (DNS-SD `_parson-bridge._tcp`) or that someone added by address. Every
//! device a bridge lists becomes a zone (`bridge:<bridge id>:<device id>`)
//! played by the same engine as the server's own devices: each file decoded
//! at its own rate and depth, exclusive mode, hardware volume in dB, DSP,
//! leveling and the signal path. Only the last hop is the network, as
//! uncompressed PCM.
//!
//! Bridge zones group natively: a group (`bridge-group:<id>`) plays one
//! stream on every member, sample-synced to the first member's DAC clock.

use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use parson_bridge::remote::{GroupMember, RemoteBridge, SyncGroup};
use serde::Serialize;

use super::device::{DeviceManager, DeviceRenderer, Placement};
use super::renderer::{Renderer, RendererInfo, RendererSettings, RendererState, TrackRequest};
use crate::zones::ZoneKind;

pub const BRIDGE_PREFIX: &str = "bridge:";
pub const GROUP_PREFIX: &str = "bridge-group:";
const RETRY: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// What the zone picker says about bridge zones.
pub const BRIDGE_NOTE: &str = "Parson Bridge: Parson decodes each file here and sends the samples to the bridge uncompressed over your network; the bridge plays them on the DAC exclusively, as sent (ALSA on Linux, WASAPI exclusive mode on Windows, CoreAudio hog mode on macOS). Tested against simulated DACs, ALSA's null device and protocol-level fakes of WASAPI and the macOS audio HAL, not yet on real hardware.";

/// What a bridge zone says about how it plays, by the bridge's audio API,
/// including what has and has not been tested.
pub fn bridge_note(backend: Option<&str>) -> String {
    const SENT: &str = "Parson Bridge: Parson decodes each file here and sends the samples to the bridge uncompressed over your network;";
    match backend {
        Some("WASAPI") => format!(
            "{SENT} the bridge plays them in WASAPI exclusive mode at the file's own rate and depth, with no Windows mixer, resampler or volume in between. The WASAPI path is tested against a protocol-level fake of Windows' audio stack (exact formats, buffer alignment, event-driven buffers, exclusive ownership), not yet on real hardware."
        ),
        Some("CoreAudio") => format!(
            "{SENT} the bridge takes the DAC in hog mode, switches it to the file's own sample rate and sets its physical format to the file's bit depth, so 16- and 24-bit audio arrives unchanged. The CoreAudio path is tested against a protocol-level fake of the macOS audio HAL (hog mode, asynchronous rate switching, float-to-integer conversion), not yet on real hardware."
        ),
        Some("ALSA") => format!(
            "{SENT} the bridge plays them on the DAC's ALSA hw device exclusively, as sent. Tested against simulated DACs and ALSA's null device, not yet on real hardware."
        ),
        _ => BRIDGE_NOTE.to_string(),
    }
}
pub const GROUP_SYNC_NOTE: &str = "Parson Bridge group: one stream, sample-synced to the first zone's DAC clock (within a quarter millisecond). The first zone stays bit-perfect; the others follow by repeating or skipping a single frame now and then.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeSource {
    Discovered,
    Manual,
}

/// A bridge as the zone picker lists it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BridgeStatus {
    pub address: String,
    pub source: BridgeSource,
    pub connected: bool,
    pub bridge_id: Option<String>,
    pub name: Option<String>,
    pub host: Option<String>,
    pub platform: Option<String>,
    pub version: Option<String>,
    pub backend: Option<String>,
    pub devices: usize,
    /// Bridge clock minus server clock, and how sure, in microseconds.
    pub clock_offset_us: Option<i64>,
    pub clock_uncertainty_us: Option<i64>,
    pub note: Option<String>,
    pub last_error: Option<String>,
}

struct Entry {
    source: BridgeSource,
    link: Option<RemoteBridge>,
    devices: Option<Arc<DeviceManager>>,
    last_attempt: Option<Instant>,
    last_error: Option<String>,
}

/// A synced group of bridge zones.
pub struct GroupRenderer {
    inner: DeviceRenderer,
    id: String,
    name: String,
    /// The clock master's zone name.
    master: String,
    members: Vec<String>,
}

impl Renderer for GroupRenderer {
    fn info(&self) -> RendererInfo {
        let mut info = self.inner.info();
        info.id = self.id.clone();
        info.name = self.name.clone();
        info.notes.insert(0, GROUP_SYNC_NOTE.to_string());
        info
    }

    fn state(&self) -> RendererState {
        self.inner.state()
    }

    fn configure(&self, settings: RendererSettings) {
        self.inner.configure(settings);
    }

    fn load(
        &self,
        track: TrackRequest,
        next: Option<TrackRequest>,
        position_ms: i64,
        play: bool,
    ) -> BoxFuture<'_, Result<(), String>> {
        self.inner.load(track, next, position_ms, play)
    }

    fn set_next(&self, next: Option<TrackRequest>) -> BoxFuture<'_, Result<(), String>> {
        self.inner.set_next(next)
    }

    fn play(&self) -> BoxFuture<'_, Result<(), String>> {
        self.inner.play()
    }

    fn pause(&self) -> BoxFuture<'_, Result<(), String>> {
        self.inner.pause()
    }

    fn stop(&self) -> BoxFuture<'_, Result<(), String>> {
        self.inner.stop()
    }

    fn seek(&self, position_ms: i64) -> BoxFuture<'_, Result<(), String>> {
        self.inner.seek(position_ms)
    }

    fn set_volume(&self, level: u8) -> BoxFuture<'_, Result<(), String>> {
        self.inner.set_volume(level)
    }

    fn set_mute(&self, muted: bool) -> BoxFuture<'_, Result<(), String>> {
        self.inner.set_mute(muted)
    }

    fn poll(&self) -> BoxFuture<'_, ()> {
        self.inner.poll()
    }

    fn group_members(&self) -> Vec<String> {
        self.members.clone()
    }
}

/// Every bridge the server knows, and the synced groups made of them.
pub struct BridgeHub {
    server_id: String,
    server_name: String,
    entries: Mutex<HashMap<String, Entry>>,
    groups: Mutex<HashMap<String, Arc<GroupRenderer>>>,
    changed: tokio::sync::broadcast::Sender<()>,
    store: Option<PathBuf>,
    browsing: Mutex<Option<std::thread::JoinHandle<()>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// `host`, `host:port` or `[v6]:port`, with the default port when none.
pub fn normalize_address(address: &str) -> Result<String, String> {
    let address = address.trim();
    if address.is_empty() || address.len() > 255 || address.contains('/') {
        return Err("invalid_bridge_address".into());
    }
    let with_port = if address.parse::<SocketAddr>().is_ok() {
        address.to_string()
    } else if address.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{address}]:{}", parson_bridge::protocol::DEFAULT_PORT)
    } else if address
        .rsplit_once(':')
        .is_some_and(|(_, port)| port.parse::<u16>().is_ok())
    {
        address.to_string()
    } else {
        format!("{address}:{}", parson_bridge::protocol::DEFAULT_PORT)
    };
    Ok(with_port)
}

impl BridgeHub {
    pub fn new(server_id: String, server_name: String, store: Option<PathBuf>) -> Arc<Self> {
        let hub = Arc::new(Self {
            server_id,
            server_name,
            entries: Mutex::new(HashMap::new()),
            groups: Mutex::new(HashMap::new()),
            changed: tokio::sync::broadcast::channel(64).0,
            store,
            browsing: Mutex::new(None),
        });
        if let Some(store) = &hub.store
            && let Ok(text) = std::fs::read_to_string(store)
            && let Ok(addresses) = serde_json::from_str::<Vec<String>>(&text)
        {
            for address in addresses {
                hub.insert(address, BridgeSource::Manual);
            }
        }
        hub
    }

    /// From the environment: on unless `PARSON_BRIDGES=off`; a comma list
    /// in `PARSON_BRIDGES` adds bridges by address.
    pub fn from_env() -> Option<Arc<Self>> {
        let setting = std::env::var("PARSON_BRIDGES").unwrap_or_default();
        if matches!(
            setting.trim().to_ascii_lowercase().as_str(),
            "off" | "0" | "false" | "no"
        ) {
            return None;
        }
        let server_id = crate::settings::instance_id().unwrap_or_else(|_| "parson".to_string());
        let hub = Self::new(
            server_id,
            crate::settings::library_name(),
            Some(crate::settings::data_path(&["Outputs", "bridges.json"])),
        );
        for address in setting.split(',').filter(|value| !value.trim().is_empty()) {
            if let Ok(address) = normalize_address(address) {
                hub.insert(address, BridgeSource::Manual);
            }
        }
        hub.browse();
        Some(hub)
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.changed.subscribe()
    }

    fn notify(&self) {
        let _ = self.changed.send(());
    }

    fn insert(&self, address: String, source: BridgeSource) -> bool {
        let mut entries = lock(&self.entries);
        if let Some(entry) = entries.get_mut(&address) {
            if source == BridgeSource::Manual {
                entry.source = BridgeSource::Manual;
            }
            return false;
        }
        entries.insert(
            address,
            Entry {
                source,
                link: None,
                devices: None,
                last_attempt: None,
                last_error: None,
            },
        );
        true
    }

    fn save(&self) {
        let Some(store) = &self.store else {
            return;
        };
        let manual: Vec<String> = {
            let entries = lock(&self.entries);
            let mut manual: Vec<String> = entries
                .iter()
                .filter(|(_, entry)| entry.source == BridgeSource::Manual)
                .map(|(address, _)| address.clone())
                .collect();
            manual.sort();
            manual
        };
        if let Some(parent) = store.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string(&manual) {
            let _ = std::fs::write(store, text);
        }
    }

    /// Adds a bridge by address and connects to it now.
    pub fn add(self: &Arc<Self>, address: &str) -> Result<BridgeStatus, String> {
        let address = normalize_address(address)?;
        self.insert(address.clone(), BridgeSource::Manual);
        self.save();
        self.connect(&address);
        self.status()
            .into_iter()
            .find(|status| status.address == address)
            .ok_or_else(|| "bridge_not_found".to_string())
    }

    /// Forgets a bridge added by address (discovered ones come back).
    pub fn remove(&self, address: &str) -> bool {
        let address = normalize_address(address).unwrap_or_else(|_| address.to_string());
        let removed = lock(&self.entries).remove(&address);
        let Some(entry) = removed else {
            return false;
        };
        if let Some(link) = &entry.link {
            link.disconnect();
        }
        if let Some(devices) = &entry.devices {
            devices.shutdown();
        }
        self.save();
        self.notify();
        true
    }

    /// Watches the local network for bridges.
    fn browse(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let handle = std::thread::Builder::new()
            .name("parson-bridge-browse".into())
            .spawn(move || browse_loop(weak));
        if let Ok(handle) = handle {
            *lock(&self.browsing) = Some(handle);
        }
    }

    /// Connects to bridges that are not connected (at most every few seconds
    /// each). Blocking: run it off the async runtime.
    pub fn maintain(self: &Arc<Self>) {
        let due: Vec<String> = {
            let entries = lock(&self.entries);
            entries
                .iter()
                .filter(|(_, entry)| {
                    !entry.link.as_ref().is_some_and(RemoteBridge::connected)
                        && entry
                            .last_attempt
                            .is_none_or(|attempt| attempt.elapsed() >= RETRY)
                })
                .map(|(address, _)| address.clone())
                .collect()
        };
        for address in due {
            self.connect(&address);
        }
        for manager in self.managers() {
            manager.refresh();
        }
    }

    fn connect(self: &Arc<Self>, address: &str) {
        // Drop a dead link first so its zones go away.
        let stale = {
            let mut entries = lock(&self.entries);
            let Some(entry) = entries.get_mut(address) else {
                return;
            };
            entry.last_attempt = Some(Instant::now());
            if entry.link.as_ref().is_some_and(RemoteBridge::connected) {
                return;
            }
            entry.link = None;
            entry.devices.take()
        };
        if let Some(devices) = stale {
            devices.shutdown();
        }
        let resolved = address
            .to_socket_addrs()
            .ok()
            .and_then(|mut addresses| addresses.next());
        let Some(socket) = resolved else {
            if let Some(entry) = lock(&self.entries).get_mut(address) {
                entry.last_error = Some(format!("could not resolve {address}"));
            }
            return;
        };
        let result =
            RemoteBridge::connect(socket, &self.server_id, &self.server_name, CONNECT_TIMEOUT);
        let link = match result {
            Ok(link) => link,
            Err(error) => {
                if let Some(entry) = lock(&self.entries).get_mut(address) {
                    entry.last_error = Some(error);
                }
                return;
            }
        };
        let info = link.info();
        // The same bridge under two addresses: keep the first link.
        let duplicate = lock(&self.entries).iter().any(|(other, entry)| {
            other != address
                && entry.link.as_ref().is_some_and(|existing| {
                    existing.connected() && existing.info().bridge_id == info.bridge_id
                })
        });
        if duplicate {
            link.disconnect();
            lock(&self.entries).remove(address);
            return;
        }
        let placement = Placement {
            prefix: format!("{BRIDGE_PREFIX}{}:", info.bridge_id),
            kind: ZoneKind::Bridge,
            host: Some(format!("{} · Parson Bridge", info.name)),
            machine: Some(info.name.clone()),
            route: Some(format!("on {} via Parson Bridge", info.name)),
            step: None,
            notes: vec![bridge_note(info.backend.as_deref())],
            playable_only: false,
        };
        let devices = Arc::new(DeviceManager::placed(
            Arc::new(link.clone()),
            info.note.clone(),
            placement,
            self.changed.clone(),
        ));
        let weak_devices: Weak<DeviceManager> = Arc::downgrade(&devices);
        let weak_hub = Arc::downgrade(self);
        link.on_change(move || {
            if let Some(devices) = weak_devices.upgrade() {
                devices.refresh();
            }
            if let Some(hub) = weak_hub.upgrade() {
                hub.notify();
            }
        });
        tracing::info!(name = info.name, %address, "connected to Parson Bridge");
        if let Some(entry) = lock(&self.entries).get_mut(address) {
            entry.link = Some(link);
            entry.devices = Some(devices);
            entry.last_error = None;
        }
        self.notify();
    }

    fn managers(&self) -> Vec<Arc<DeviceManager>> {
        lock(&self.entries)
            .values()
            .filter_map(|entry| entry.devices.clone())
            .collect()
    }

    fn grouped(&self) -> Vec<String> {
        lock(&self.groups)
            .values()
            .flat_map(|group| group.members.clone())
            .collect()
    }

    /// Bridge zones not in a group, then groups.
    pub fn renderers(&self) -> Vec<Arc<dyn Renderer>> {
        let grouped = self.grouped();
        let mut renderers: Vec<Arc<dyn Renderer>> = self
            .managers()
            .iter()
            .flat_map(|manager| manager.renderers())
            .filter(|renderer| !grouped.contains(&renderer.info().id))
            .collect();
        renderers.sort_by_key(|renderer| renderer.info().id);
        let mut groups: Vec<Arc<GroupRenderer>> = lock(&self.groups).values().cloned().collect();
        groups.sort_by(|left, right| left.id.cmp(&right.id));
        renderers.extend(groups.into_iter().map(|group| group as Arc<dyn Renderer>));
        renderers
    }

    /// A bridge zone (grouped members included, for their own volume) or
    /// a group.
    pub fn renderer(&self, zone_id: &str) -> Option<Arc<dyn Renderer>> {
        if zone_id.starts_with(GROUP_PREFIX) {
            return lock(&self.groups)
                .get(zone_id)
                .cloned()
                .map(|group| group as Arc<dyn Renderer>);
        }
        self.managers()
            .iter()
            .find_map(|manager| manager.renderer(zone_id))
    }

    fn member(&self, zone_id: &str) -> Option<GroupMember> {
        let rest = zone_id.strip_prefix(BRIDGE_PREFIX)?;
        let (bridge_id, device_id) = rest.split_once(':')?;
        let entries = lock(&self.entries);
        let link = entries.values().find_map(|entry| {
            entry
                .link
                .as_ref()
                .filter(|link| link.connected() && link.info().bridge_id == bridge_id)
                .cloned()
        })?;
        Some(GroupMember {
            bridge: link,
            device_id: device_id.to_string(),
        })
    }

    /// Groups bridge zones into one synced zone; the first is the clock
    /// master. Members leave any group they were in.
    pub fn create_group(
        &self,
        zone_ids: &[String],
        name: &str,
        master: &str,
    ) -> Result<String, String> {
        let id = format!("{GROUP_PREFIX}{}", uuid::Uuid::new_v4());
        self.build_group(&id, zone_ids, name, master)?;
        Ok(id)
    }

    fn build_group(
        &self,
        id: &str,
        zone_ids: &[String],
        name: &str,
        master: &str,
    ) -> Result<(), String> {
        let members: Vec<GroupMember> = zone_ids
            .iter()
            .map(|zone_id| {
                self.member(zone_id)
                    .ok_or_else(|| format!("{zone_id} is not connected"))
            })
            .collect::<Result<_, _>>()?;
        let sync = SyncGroup::new(id, name, members);
        let device = sync
            .device()
            .ok_or_else(|| "these devices share no sample format".to_string())?;
        let renderer = Arc::new(GroupRenderer {
            inner: DeviceRenderer::standalone(
                id.to_string(),
                device,
                Arc::new(sync),
                self.changed.clone(),
                Placement {
                    prefix: GROUP_PREFIX.to_string(),
                    kind: ZoneKind::Bridge,
                    host: Some("Sample-synced group".to_string()),
                    machine: None,
                    route: Some("(sample-synced group)".to_string()),
                    step: (zone_ids.len() > 1).then(|| {
                        format!(
                            "Sync: {master} is the clock master and gets every sample untouched; the other zones follow it by repeating or skipping a single frame when they drift"
                        )
                    }),
                    notes: vec![BRIDGE_NOTE.to_string()],
                    playable_only: false,
                },
            ),
            id: id.to_string(),
            name: name.to_string(),
            master: master.to_string(),
            members: zone_ids.to_vec(),
        });
        let mut groups = lock(&self.groups);
        for group in groups.values() {
            if group.id != id && group.members.iter().any(|member| zone_ids.contains(member)) {
                group.inner.shutdown();
            }
        }
        groups.retain(|group_id, group| {
            group_id == id || !group.members.iter().any(|member| zone_ids.contains(member))
        });
        if let Some(old) = groups.insert(id.to_string(), renderer) {
            old.inner.shutdown();
        }
        drop(groups);
        self.notify();
        Ok(())
    }

    pub fn group_members(&self, group_id: &str) -> Option<Vec<String>> {
        lock(&self.groups)
            .get(group_id)
            .map(|group| group.members.clone())
    }

    /// Takes one member out (the group goes when fewer than two remain),
    /// or ends the group. Returns the zones that are on their own again.
    pub fn leave_group(&self, group_id: &str, member: Option<&str>) -> Result<Vec<String>, String> {
        let group = lock(&self.groups)
            .get(group_id)
            .cloned()
            .ok_or_else(|| "group_not_found".to_string())?;
        let remaining: Vec<String> = match member {
            Some(member) => {
                if !group.members.iter().any(|candidate| candidate == member) {
                    return Err("member_not_in_group".into());
                }
                group
                    .members
                    .iter()
                    .filter(|candidate| *candidate != member)
                    .cloned()
                    .collect()
            }
            None => Vec::new(),
        };
        if remaining.len() >= 2 {
            let master = if member == group.members.first().map(String::as_str) {
                self.managers()
                    .iter()
                    .find_map(|manager| manager.renderer(&remaining[0]))
                    .map(|renderer| renderer.info().name)
                    .unwrap_or_else(|| "the first zone".to_string())
            } else {
                group.master.clone()
            };
            self.build_group(group_id, &remaining, &group.name, &master)?;
            return Ok(member
                .map(|member| vec![member.to_string()])
                .unwrap_or_default());
        }
        if let Some(group) = lock(&self.groups).remove(group_id) {
            group.inner.shutdown();
        }
        self.notify();
        Ok(group.members.clone())
    }

    pub fn status(&self) -> Vec<BridgeStatus> {
        let entries = lock(&self.entries);
        let mut statuses: Vec<BridgeStatus> = entries
            .iter()
            .map(|(address, entry)| {
                let link = entry.link.as_ref().filter(|link| link.connected());
                let info = link.map(RemoteBridge::info);
                let clock = link.and_then(RemoteBridge::clock_offset_us);
                BridgeStatus {
                    address: address.clone(),
                    source: entry.source,
                    connected: link.is_some(),
                    bridge_id: info.as_ref().map(|info| info.bridge_id.clone()),
                    name: info.as_ref().map(|info| info.name.clone()),
                    host: info.as_ref().map(|info| info.host.clone()),
                    platform: info.as_ref().map(|info| info.platform.clone()),
                    version: info.as_ref().map(|info| info.version.clone()),
                    backend: info.as_ref().and_then(|info| info.backend.clone()),
                    devices: link
                        .map(|link| {
                            use parson_output::Backend as _;
                            link.devices().len()
                        })
                        .unwrap_or(0),
                    clock_offset_us: clock.map(|(offset, _)| offset),
                    clock_uncertainty_us: clock.map(|(_, uncertainty)| uncertainty),
                    note: info.and_then(|info| info.note),
                    last_error: entry
                        .last_error
                        .clone()
                        .or_else(|| link.and_then(|link| link.recent_errors().last().cloned())),
                }
            })
            .collect();
        statuses.sort_by(|left, right| left.address.cmp(&right.address));
        statuses
    }
}

fn browse_loop(hub: Weak<BridgeHub>) {
    let Ok(daemon) = mdns_sd::ServiceDaemon::new() else {
        return;
    };
    let Ok(receiver) = daemon.browse(parson_bridge::protocol::SERVICE_TYPE) else {
        return;
    };
    loop {
        let event = receiver.recv_timeout(Duration::from_secs(5));
        let Some(hub) = hub.upgrade() else {
            let _ = daemon.shutdown();
            return;
        };
        let Ok(mdns_sd::ServiceEvent::ServiceResolved(info)) = event else {
            continue;
        };
        let port = info.get_port();
        let address = info
            .get_addresses_v4()
            .into_iter()
            .find(|address| !address.is_loopback() && !address.is_unspecified());
        if let Some(address) = address {
            let address = SocketAddr::new(std::net::IpAddr::V4(address), port).to_string();
            if hub.insert(address, BridgeSource::Discovered) {
                hub.notify();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn bridge_zones_say_how_they_play_and_what_is_untested() {
        for backend in [Some("WASAPI"), Some("CoreAudio"), Some("ALSA"), None] {
            let note = super::bridge_note(backend);
            assert!(note.contains("not yet on real hardware"), "{note}");
        }
        assert!(super::bridge_note(Some("WASAPI")).contains("WASAPI exclusive mode"));
        assert!(super::bridge_note(Some("CoreAudio")).contains("hog mode"));
    }

    use super::normalize_address;

    #[test]
    fn addresses_get_the_default_port() {
        assert_eq!(normalize_address("pi.local").unwrap(), "pi.local:1994");
        assert_eq!(normalize_address("10.0.0.7:2000").unwrap(), "10.0.0.7:2000");
        assert_eq!(normalize_address(" 10.0.0.7 ").unwrap(), "10.0.0.7:1994");
        assert_eq!(normalize_address("fe80::1").unwrap(), "[fe80::1]:1994");
        assert_eq!(normalize_address("[fe80::1]:7").unwrap(), "[fe80::1]:7");
        assert!(normalize_address("").is_err());
        assert!(normalize_address("http://x/y").is_err());
    }
}
