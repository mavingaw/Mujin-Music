use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::Serialize;

pub const SERVICE_TYPE: &str = "_parson._tcp.local.";

static BOOT: OnceLock<(String, u64)> = OnceLock::new();
static ADVERTISING: AtomicBool = AtomicBool::new(false);

fn boot() -> &'static (String, u64) {
    BOOT.get_or_init(|| {
        let started_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default();
        (uuid::Uuid::new_v4().simple().to_string(), started_at)
    })
}

/// A random identifier for this server process. Remotes compare it between
/// probes to notice that the server restarted, even if it came back quickly.
pub fn boot_id() -> &'static str {
    &boot().0
}

/// Unix seconds when this server process started.
pub fn started_at() -> u64 {
    boot().1
}

/// Whether the `_parson._tcp` advertisement is currently registered.
pub fn is_advertising() -> bool {
    ADVERTISING.load(Ordering::Relaxed)
}

/// How a remote device on the network can reach this server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostAddress {
    pub address: String,
    pub origin: String,
    pub interface: String,
    /// `lan` (home network), `vpn` (Tailscale/WireGuard style overlay),
    /// `link-local` (direct cable, no router) or `public`.
    pub kind: &'static str,
}

/// Everything a Parson app needs to show "connect your other devices here".
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInfo {
    pub instance_id: String,
    pub name: String,
    pub server_version: &'static str,
    pub port: u16,
    pub boot_id: &'static str,
    pub started_at: u64,
    /// False when the server only listens on loopback, so no other device can
    /// connect whatever address it tries.
    pub network_access: bool,
    pub advertising: bool,
    pub public_url: Option<String>,
    pub addresses: Vec<HostAddress>,
    /// Addresses for away from home (VPN, the owner's own address, a router
    /// port forward), which the phone app saves and falls back to.
    pub remote: Vec<crate::remote_access::RemoteAddress>,
}

pub fn host_info(port: u16) -> Result<HostInfo, String> {
    let mut info = host_info_without_remote(port)?;
    info.remote = crate::remote_access::remote_addresses(&info.addresses);
    Ok(info)
}

/// [`host_info`] without the remote-access addresses.
pub fn host_info_without_remote(port: u16) -> Result<HostInfo, String> {
    let instance_id = crate::settings::instance_id().map_err(|error| error.to_string())?;
    let bind_address = crate::settings::bind_address()?
        .parse::<IpAddr>()
        .map_err(|error| error.to_string())?;
    let network_access = !bind_address.is_loopback();
    let interfaces = if network_access {
        if_addrs::get_if_addrs()
            .map(|interfaces| {
                interfaces
                    .into_iter()
                    .filter(|interface| {
                        !matches!(
                            interface.oper_status,
                            if_addrs::IfOperStatus::Down | if_addrs::IfOperStatus::NotPresent
                        )
                    })
                    .map(|interface| (interface.name.clone(), interface.ip()))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let public_url = std::env::var("PARSON_PUBLIC_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| {
            (value.starts_with("http://") || value.starts_with("https://"))
                && !value.contains("://127.")
                && !value.contains("://localhost")
                && !value.contains("://[::1]")
        });
    Ok(HostInfo {
        instance_id,
        name: crate::settings::library_name(),
        server_version: env!("CARGO_PKG_VERSION"),
        port,
        boot_id: boot_id(),
        started_at: started_at(),
        network_access,
        advertising: is_advertising(),
        public_url,
        addresses: rank_host_addresses(interfaces, port),
        remote: Vec::new(),
    })
}

fn virtual_interface(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "docker", "br-", "veth", "virbr", "vmnet", "vboxnet", "cni", "flannel", "podman", "lxc",
        "lxd",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
        || name.contains("vethernet")
        || name.contains("virtualbox")
        || name.contains("vmware")
        || name.contains("hyper-v")
        || name.contains("wsl")
}

fn address_kind(address: IpAddr) -> Option<&'static str> {
    match address {
        IpAddr::V4(address) => {
            let [first, second, ..] = address.octets();
            if address.is_loopback() || address.is_unspecified() || address.is_multicast() {
                None
            } else if address.is_private() {
                Some("lan")
            } else if address.is_link_local() {
                Some("link-local")
            } else if first == 100 && (64..128).contains(&second) {
                // Carrier-grade NAT space, used by Tailscale and similar overlays.
                Some("vpn")
            } else {
                Some("public")
            }
        }
        IpAddr::V6(address) => {
            let first = address.segments()[0];
            if address.is_loopback() || address.is_unspecified() || address.is_multicast() {
                None
            } else if first & 0xffc0 == 0xfe80 {
                // Link-local IPv6 needs a zone id, which browsers do not accept.
                None
            } else if first & 0xfe00 == 0xfc00 {
                Some("lan")
            } else {
                Some("public")
            }
        }
    }
}

/// Orders the machine's addresses so the one a phone on the same Wi-Fi is most
/// likely to reach comes first: physical LAN IPv4, then VPN overlays, then the
/// rest. Container and hypervisor bridges are left out because other devices
/// can never reach them.
fn rank_host_addresses(interfaces: Vec<(String, IpAddr)>, port: u16) -> Vec<HostAddress> {
    let mut ranked = interfaces
        .into_iter()
        .filter(|(name, _)| !virtual_interface(name))
        .filter_map(|(name, address)| {
            let kind = address_kind(address)?;
            let rank = match (kind, address.is_ipv4()) {
                ("lan", true) => 0,
                ("vpn", _) => 1,
                ("lan", false) => 2,
                ("link-local", _) => 3,
                _ => 4,
            };
            let host = match address {
                IpAddr::V4(address) => address.to_string(),
                IpAddr::V6(address) => format!("[{address}]"),
            };
            Some((
                rank,
                HostAddress {
                    address: address.to_string(),
                    origin: format!("http://{host}:{port}"),
                    interface: name,
                    kind,
                },
            ))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.interface.cmp(&right.1.interface))
            .then_with(|| left.1.address.cmp(&right.1.address))
    });
    let mut seen = std::collections::HashSet::new();
    ranked
        .into_iter()
        .map(|(_, address)| address)
        .filter(|address| seen.insert(address.origin.clone()))
        .take(8)
        .collect()
}

/// Keeps Parson's DNS-SD advertisement alive for as long as this value exists.
pub struct DiscoveryAdvertisement {
    daemon: ServiceDaemon,
    fullname: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredServer {
    pub instance_id: String,
    pub name: String,
    pub origin: String,
    pub port: u16,
    pub is_current: bool,
}

impl Drop for DiscoveryAdvertisement {
    fn drop(&mut self) {
        ADVERTISING.store(false, Ordering::Relaxed);
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

pub fn advertise(port: u16) -> Result<DiscoveryAdvertisement, String> {
    let bind_address = crate::settings::bind_address()?
        .parse::<std::net::IpAddr>()
        .map_err(|error| error.to_string())?;
    if bind_address.is_loopback() {
        return Err("the server is configured for this device only".to_string());
    }
    let instance_id = crate::settings::instance_id().map_err(|error| error.to_string())?;
    let name = crate::settings::library_name();
    let host = format!("parson-{}.local.", &instance_id[..8]);
    let mut properties = HashMap::from([
        ("id".to_string(), instance_id),
        ("name".to_string(), name.clone()),
        ("version".to_string(), "1".to_string()),
        ("product".to_string(), "parson-music".to_string()),
        ("api".to_string(), "1".to_string()),
        ("tls".to_string(), "0".to_string()),
        ("pairing".to_string(), "required".to_string()),
        ("boot".to_string(), boot_id()[..12].to_string()),
    ]);
    if let Some(adopted_at) = crate::settings::identity_adopted_at() {
        // A takeover of a restored library: see `identityAdoptedAt`.
        properties.insert("adopted".to_string(), adopted_at.to_string());
    }
    let service = ServiceInfo::new(SERVICE_TYPE, &name, &host, (), port, properties)
        .map_err(|error| error.to_string())?
        .enable_addr_auto();
    let fullname = service.get_fullname().to_string();
    let daemon = ServiceDaemon::new().map_err(|error| error.to_string())?;
    daemon
        .register(service)
        .map_err(|error| error.to_string())?;
    ADVERTISING.store(true, Ordering::Relaxed);
    tracing::info!(service = SERVICE_TYPE, %port, %name, "advertising Mujin Music on the local network");
    Ok(DiscoveryAdvertisement { daemon, fullname })
}

/// Searches the local network for Parson libraries for a bounded interval.
pub async fn discover_nearby(timeout: Duration) -> Result<Vec<DiscoveredServer>, String> {
    let daemon = ServiceDaemon::new().map_err(|error| error.to_string())?;
    let receiver = daemon
        .browse(SERVICE_TYPE)
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + timeout;
    let current_instance_id = crate::settings::instance_id().ok();
    let mut servers = HashMap::<String, DiscoveredServer>::new();

    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        let Ok(event) = tokio::time::timeout(remaining, receiver.recv_async()).await else {
            break;
        };
        let Ok(ServiceEvent::ServiceResolved(info)) = event else {
            continue;
        };
        if info.get_property_val_str("product") != Some("parson-music") {
            continue;
        }
        let Some(instance_id) = info
            .get_property_val_str("id")
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let name = info
            .get_property_val_str("name")
            .filter(|value| !value.is_empty())
            .unwrap_or("Mujin Music Library");
        let Some(address) = info
            .get_addresses_v4()
            .into_iter()
            .find(|address| !address.is_loopback() && !address.is_unspecified())
        else {
            continue;
        };
        let port = info.get_port();
        let mut candidate = discovered_server(instance_id, name, address, port);
        candidate.is_current = current_instance_id.as_deref() == Some(instance_id);
        insert_discovered_server(&mut servers, candidate);
    }

    let _ = daemon.stop_browse(SERVICE_TYPE);
    let _ = daemon.shutdown();
    let mut servers = servers.into_values().collect::<Vec<_>>();
    servers.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(servers)
}

fn discovered_server(
    instance_id: &str,
    name: &str,
    address: std::net::Ipv4Addr,
    port: u16,
) -> DiscoveredServer {
    DiscoveredServer {
        instance_id: instance_id.to_string(),
        name: name.to_string(),
        origin: format!("http://{address}:{port}"),
        port,
        is_current: false,
    }
}

fn insert_discovered_server(
    servers: &mut HashMap<String, DiscoveredServer>,
    candidate: DiscoveredServer,
) {
    match servers.entry(candidate.instance_id.clone()) {
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(candidate);
        }
        std::collections::hash_map::Entry::Occupied(mut entry)
            if candidate.port == crate::settings::DEFAULT_PORT
                && entry.get().port != crate::settings::DEFAULT_PORT =>
        {
            entry.insert(candidate);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    fn interface(name: &str, address: &str) -> (String, IpAddr) {
        (name.to_string(), address.parse().unwrap())
    }

    #[test]
    fn host_addresses_put_the_home_network_first_and_skip_unreachable_bridges() {
        let addresses = super::rank_host_addresses(
            vec![
                interface("lo", "127.0.0.1"),
                interface("tailscale0", "100.101.102.103"),
                interface("docker0", "172.17.0.1"),
                interface("vEthernet (WSL)", "172.29.16.1"),
                interface("wlan0", "192.168.1.42"),
                interface("wlan0", "fe80::1"),
                interface("eth0", "fd00::42"),
                interface("eth1", "169.254.10.2"),
            ],
            1993,
        );
        let origins = addresses
            .iter()
            .map(|address| (address.origin.as_str(), address.kind))
            .collect::<Vec<_>>();
        assert_eq!(
            origins,
            vec![
                ("http://192.168.1.42:1993", "lan"),
                ("http://100.101.102.103:1993", "vpn"),
                ("http://[fd00::42]:1993", "lan"),
                ("http://169.254.10.2:1993", "link-local"),
            ]
        );
        assert_eq!(addresses[0].interface, "wlan0");
    }

    #[test]
    fn host_addresses_are_deduplicated_and_bounded() {
        let mut interfaces = vec![interface("en0", "10.0.0.5"), interface("en0", "10.0.0.5")];
        for index in 0..20 {
            interfaces.push(interface(&format!("en{index}"), &format!("10.0.1.{index}")));
        }
        let addresses = super::rank_host_addresses(interfaces, 8080);
        assert_eq!(addresses.len(), 8);
        assert_eq!(
            addresses
                .iter()
                .filter(|address| address.origin == "http://10.0.0.5:8080")
                .count(),
            1
        );
    }

    #[test]
    fn boot_id_is_stable_within_a_process() {
        assert_eq!(super::boot_id(), super::boot_id());
        assert_eq!(super::boot_id().len(), 32);
        assert!(super::started_at() > 0);
    }
    #[test]
    fn discovered_servers_use_a_direct_browser_origin() {
        let server = super::discovered_server(
            "library-id",
            "Living room",
            "192.168.1.25".parse().unwrap(),
            1993,
        );
        assert_eq!(server.instance_id, "library-id");
        assert_eq!(server.name, "Living room");
        assert_eq!(server.origin, "http://192.168.1.25:1993");
        assert_eq!(server.port, 1993);
        assert!(!server.is_current);
    }

    #[test]
    fn official_port_wins_when_one_library_has_multiple_advertisements() {
        let mut servers = std::collections::HashMap::new();
        super::insert_discovered_server(
            &mut servers,
            super::discovered_server(
                "same-library",
                "Library",
                "192.168.1.25".parse().unwrap(),
                3001,
            ),
        );
        super::insert_discovered_server(
            &mut servers,
            super::discovered_server(
                "same-library",
                "Library",
                "192.168.1.25".parse().unwrap(),
                1993,
            ),
        );
        assert_eq!(servers["same-library"].port, 1993);
    }
}
