//! Reaching the server from outside the home.
//!
//! Three ways, best first for most people:
//! - a VPN such as Tailscale or WireGuard: nothing is exposed to the
//!   internet; Parson detects the VPN address (and Tailscale's MagicDNS
//!   name) and hands it to the phone app;
//! - "my own address": an HTTPS reverse proxy, a dynamic-DNS name or any
//!   other URL the owner set up;
//! - "router": Parson asks the home router (UPnP IGD) to forward a port and
//!   keeps the forward alive.
//!
//! Every path can be checked: the server connects to each address and
//! confirms it reaches *this* server (not just something answering). The
//! phone app runs the same check from wherever it is.

pub mod igd;

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

const CONFIG_FILE: [&str; 2] = ["Network", "remote-access.json"];
const LEASE_SECONDS: u32 = 60 * 60;
const CHECK_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_URL_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RemoteAccessMode {
    #[default]
    Off,
    /// Forward a port on the home router with UPnP.
    Router,
    /// An address the owner set up (reverse proxy, dynamic DNS, VPN name).
    Manual,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct RemoteAccessConfig {
    pub mode: RemoteAccessMode,
    pub manual_url: Option<String>,
    /// The port opened on the router; defaults to the server's own port.
    pub external_port: Option<u16>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RouterStatus {
    #[default]
    Off,
    Working,
    Forwarded,
    /// The forward exists but the router's own public address is shared
    /// (carrier-grade NAT) or private (another router in front), so it
    /// cannot be reached from the internet.
    Unreachable,
    Failed,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct RouterState {
    pub status: RouterStatus,
    pub router_name: Option<String>,
    pub external_ip: Option<String>,
    pub external_port: Option<u16>,
    pub internal_address: Option<String>,
    /// 0 means the router only allows permanent forwards.
    pub lease_seconds: Option<u32>,
    pub updated_at: Option<String>,
    pub message: Option<String>,
}

/// An address the phone app can use away from home.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAddress {
    pub origin: String,
    /// `vpn`, `manual` or `router`.
    pub kind: &'static str,
    pub label: String,
    /// The outcome of the last check of this address, when there was one.
    pub verified: Option<bool>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckOutcome {
    Reachable,
    Unreachable,
    /// Something answered, but it is a different Parson server.
    OtherServer,
    /// Something answered, but it is not Parson (a proxy misconfiguration,
    /// a router login page).
    NotParson,
}

#[derive(Clone, Debug, Serialize)]
pub struct PathCheck {
    pub origin: String,
    pub kind: &'static str,
    pub label: String,
    pub outcome: CheckOutcome,
    pub latency_ms: Option<u64>,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CheckReport {
    pub checked_at: String,
    pub results: Vec<PathCheck>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RemoteAccessStatus {
    pub config: RemoteAccessConfig,
    pub router: RouterState,
    pub addresses: Vec<RemoteAddress>,
    pub tailscale_name: Option<String>,
    pub last_check: Option<CheckReport>,
    /// False when the server only listens on this computer.
    pub network_access: bool,
}

struct ActiveMapping {
    gateway: igd::Gateway,
    external_port: u16,
}

pub struct Service {
    config: Mutex<RemoteAccessConfig>,
    router: Mutex<RouterState>,
    mapping: tokio::sync::Mutex<Option<ActiveMapping>>,
    last_check: Mutex<Option<CheckReport>>,
    tailscale: Mutex<Option<(String, Instant)>>,
    port: AtomicU16,
    wake: tokio::sync::Notify,
    ssdp_targets: Vec<SocketAddr>,
    persist: bool,
}

static SERVICE: OnceLock<Arc<Service>> = OnceLock::new();

fn config_path() -> std::path::PathBuf {
    crate::settings::data_path(&CONFIG_FILE)
}

fn load_config() -> RemoteAccessConfig {
    std::fs::read(config_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Accepts `http(s)://host[:port]` and returns its origin.
pub fn normalize_manual_url(value: &str) -> Result<String, String> {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.is_empty() || trimmed.len() > MAX_URL_BYTES {
        return Err("Enter the address, for example https://music.example.com.".into());
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let url = reqwest::Url::parse(&with_scheme)
        .map_err(|_| format!("{trimmed} isn’t a valid address."))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Use an http:// or https:// address.".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Leave the user name and password out of the address.".into());
    }
    if url.path() != "/" || url.query().is_some() {
        return Err("Use the server’s address only, without a path.".into());
    }
    let host = url.host_str().unwrap_or_default();
    if host.is_empty() || host == "localhost" || host.starts_with("127.") || host == "[::1]" {
        return Err("That address only works on this computer.".into());
    }
    Ok(url.origin().ascii_serialization())
}

impl Service {
    fn new(
        config: RemoteAccessConfig,
        port: u16,
        ssdp_targets: Vec<SocketAddr>,
        persist: bool,
    ) -> Self {
        Self {
            config: Mutex::new(config),
            router: Mutex::new(RouterState::default()),
            mapping: tokio::sync::Mutex::new(None),
            last_check: Mutex::new(None),
            tailscale: Mutex::new(None),
            port: AtomicU16::new(port),
            wake: tokio::sync::Notify::new(),
            ssdp_targets,
            persist,
        }
    }

    fn config(&self) -> RemoteAccessConfig {
        self.config.lock().expect("remote access config").clone()
    }

    fn set_router(&self, state: RouterState) {
        *self.router.lock().expect("remote access router") = state;
    }

    fn router(&self) -> RouterState {
        self.router.lock().expect("remote access router").clone()
    }

    fn port(&self) -> u16 {
        self.port.load(Ordering::Relaxed)
    }

    /// Creates or renews the router forward. Returns how long to wait before
    /// renewing it.
    async fn ensure_mapping(&self, config: &RemoteAccessConfig) -> Duration {
        let internal_port = self.port();
        let external_port = config.external_port.unwrap_or(internal_port);
        // Show "working" only for a first attempt; a retry keeps showing the
        // last outcome until it has a new one.
        if self.router().status == RouterStatus::Off {
            self.set_router(RouterState {
                status: RouterStatus::Working,
                external_port: Some(external_port),
                message: Some("Asking your router to forward a port…".into()),
                ..RouterState::default()
            });
        }
        let mut mapping = self.mapping.lock().await;
        // A different external port than last time: remove the old forward.
        if let Some(active) = mapping.as_ref()
            && active.external_port != external_port
        {
            let _ = igd::delete_port_mapping(&active.gateway, active.external_port).await;
            *mapping = None;
        }
        let gateway = match mapping.as_ref() {
            Some(active) => active.gateway.clone(),
            None => match igd::discover(&self.ssdp_targets, Duration::from_secs(3)).await {
                Ok(gateway) => gateway,
                Err(error) => {
                    self.set_router(RouterState {
                        status: RouterStatus::Failed,
                        updated_at: Some(now()),
                        message: Some(error.to_string()),
                        external_port: Some(external_port),
                        ..RouterState::default()
                    });
                    return Duration::from_secs(5 * 60);
                }
            },
        };
        let lease = match igd::add_port_mapping(
            &gateway,
            external_port,
            internal_port,
            LEASE_SECONDS,
        )
        .await
        {
            Ok(lease) => lease,
            Err(error) => {
                *mapping = None;
                let message = match &error {
                    igd::IgdError::Refused { code: 718, .. } => format!(
                        "Port {external_port} is already forwarded to another device on your router. Choose a different outside port."
                    ),
                    _ => error.to_string(),
                };
                self.set_router(RouterState {
                    status: RouterStatus::Failed,
                    router_name: Some(gateway.friendly_name.clone()),
                    external_port: Some(external_port),
                    internal_address: Some(format!("{}:{internal_port}", gateway.local_ip)),
                    updated_at: Some(now()),
                    message: Some(message),
                    ..RouterState::default()
                });
                return Duration::from_secs(5 * 60);
            }
        };
        let external_ip = igd::external_ip(&gateway).await.ok();
        let shared = external_ip.is_some_and(igd::is_shared_address);
        self.set_router(RouterState {
            status: if shared {
                RouterStatus::Unreachable
            } else {
                RouterStatus::Forwarded
            },
            router_name: Some(gateway.friendly_name.clone()),
            external_ip: external_ip.map(|address| address.to_string()),
            external_port: Some(external_port),
            internal_address: Some(format!("{}:{internal_port}", gateway.local_ip)),
            lease_seconds: Some(lease),
            updated_at: Some(now()),
            message: if shared {
                Some(format!(
                    "Your router’s own internet address ({}) is shared or private: your provider uses carrier-grade NAT, or another router sits in front of this one. A port forward can’t be reached from the internet here. Use Tailscale instead.",
                    external_ip.map(|address| address.to_string()).unwrap_or_default()
                ))
            } else if external_ip.is_none() {
                Some("The port is forwarded, but the router did not say what its public address is.".into())
            } else {
                None
            },
        });
        *mapping = Some(ActiveMapping {
            gateway,
            external_port,
        });
        if lease == 0 {
            Duration::from_secs(30 * 60)
        } else {
            Duration::from_secs(u64::from(lease / 2).clamp(60, 30 * 60))
        }
    }

    async fn remove_mapping(&self) {
        let mut mapping = self.mapping.lock().await;
        if let Some(active) = mapping.take()
            && let Err(error) =
                igd::delete_port_mapping(&active.gateway, active.external_port).await
        {
            tracing::info!(%error, "could not remove the router port forward");
        }
        if self.router().status != RouterStatus::Off {
            self.set_router(RouterState::default());
        }
    }

    async fn run(self: Arc<Self>) {
        loop {
            let config = self.config();
            let wait = if config.mode == RemoteAccessMode::Router {
                self.ensure_mapping(&config).await
            } else {
                self.remove_mapping().await;
                Duration::from_secs(60 * 60)
            };
            tokio::select! {
                () = tokio::time::sleep(wait) => {}
                () = self.wake.notified() => {}
            }
        }
    }

    fn tailscale_name(&self) -> Option<String> {
        self.tailscale
            .lock()
            .expect("tailscale name")
            .as_ref()
            .map(|(name, _)| name.clone())
    }

    async fn refresh_tailscale_name(&self) {
        let fresh = self
            .tailscale
            .lock()
            .expect("tailscale name")
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() < Duration::from_secs(10 * 60));
        if fresh {
            return;
        }
        if let Some(name) = tailscale_dns_name().await {
            *self.tailscale.lock().expect("tailscale name") = Some((name, Instant::now()));
        }
    }

    /// Every address a phone could use away from home, best first.
    fn addresses(&self, host_addresses: &[crate::discovery::HostAddress]) -> Vec<RemoteAddress> {
        let config = self.config();
        let port = self.port();
        let verified = |origin: &str| {
            self.last_check
                .lock()
                .expect("remote access check")
                .as_ref()
                .and_then(|report| {
                    report
                        .results
                        .iter()
                        .find(|result| result.origin == origin)
                        .map(|result| result.outcome == CheckOutcome::Reachable)
                })
        };
        let mut addresses = Vec::new();
        if config.mode == RemoteAccessMode::Manual
            && let Some(origin) = config.manual_url.as_deref()
        {
            addresses.push(RemoteAddress {
                origin: origin.to_string(),
                kind: "manual",
                label: "Your address".into(),
                verified: verified(origin),
            });
        }
        if let Some(name) = self.tailscale_name() {
            let origin = format!("http://{name}:{port}");
            addresses.push(RemoteAddress {
                verified: verified(&origin),
                origin,
                kind: "vpn",
                label: "Tailscale".into(),
            });
        }
        for address in host_addresses
            .iter()
            .filter(|address| address.kind == "vpn")
        {
            addresses.push(RemoteAddress {
                origin: address.origin.clone(),
                kind: "vpn",
                label: if address.address.starts_with("100.") {
                    "Tailscale".into()
                } else {
                    "VPN".into()
                },
                verified: verified(&address.origin),
            });
        }
        let router = self.router();
        if config.mode == RemoteAccessMode::Router
            && router.status == RouterStatus::Forwarded
            && let (Some(ip), Some(external_port)) =
                (router.external_ip.as_deref(), router.external_port)
        {
            let origin = format!("http://{ip}:{external_port}");
            addresses.push(RemoteAddress {
                verified: verified(&origin),
                origin,
                kind: "router",
                label: "Port forward".into(),
            });
        }
        let mut seen = std::collections::HashSet::new();
        addresses.retain(|address| seen.insert(address.origin.clone()));
        addresses
    }

    async fn check(&self, host_addresses: &[crate::discovery::HostAddress]) -> CheckReport {
        let instance_id = crate::settings::instance_id().unwrap_or_default();
        let addresses = self.addresses(host_addresses);
        // Connect directly, as a phone would: an HTTP proxy configured for
        // this server must not make an address look reachable.
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(CHECK_TIMEOUT)
            .redirect(reqwest::redirect::Policy::limited(2))
            .build();
        let mut results = Vec::with_capacity(addresses.len());
        let checks = addresses.iter().map(|address| {
            let client = client.as_ref().ok().cloned();
            let instance_id = instance_id.clone();
            async move {
                let started = Instant::now();
                let (outcome, detail) = match client {
                    Some(client) => check_address(&client, &address.origin, &instance_id).await,
                    None => (CheckOutcome::Unreachable, "Could not start the check.".into()),
                };
                let detail = if address.kind == "router" && outcome == CheckOutcome::Unreachable {
                    format!(
                        "{detail} Checked from the server through the router: some routers can’t loop a connection back to their own public address, so open the Parson app on mobile data to be sure."
                    )
                } else {
                    detail
                };
                PathCheck {
                    origin: address.origin.clone(),
                    kind: address.kind,
                    label: address.label.clone(),
                    latency_ms: (outcome == CheckOutcome::Reachable)
                        .then(|| started.elapsed().as_millis() as u64),
                    outcome,
                    detail,
                }
            }
        });
        results.extend(futures::future::join_all(checks).await);
        let report = CheckReport {
            checked_at: now(),
            results,
        };
        *self.last_check.lock().expect("remote access check") = Some(report.clone());
        report
    }
}

/// Connects to `origin` and confirms it is this Parson server.
async fn check_address(
    client: &reqwest::Client,
    origin: &str,
    instance_id: &str,
) -> (CheckOutcome, String) {
    let response = match client
        .get(format!("{origin}/.well-known/parson"))
        .header("Accept", "application/json")
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) if error.is_timeout() => {
            return (
                CheckOutcome::Unreachable,
                "No answer within 5 seconds.".into(),
            );
        }
        Err(error) if error.is_connect() => {
            return (
                CheckOutcome::Unreachable,
                "The connection was refused or could not be made.".into(),
            );
        }
        Err(error) => return (CheckOutcome::Unreachable, error.to_string()),
    };
    if !response.status().is_success() {
        return (
            CheckOutcome::NotParson,
            format!(
                "Something answered with HTTP {}, but not Parson.",
                response.status()
            ),
        );
    }
    let Ok(manifest) = response.json::<serde_json::Value>().await else {
        return (
            CheckOutcome::NotParson,
            "Something answered, but not Parson.".into(),
        );
    };
    if manifest["protocol"] != "parson" {
        return (
            CheckOutcome::NotParson,
            "Something answered, but not Parson.".into(),
        );
    }
    if manifest["instanceId"].as_str() != Some(instance_id) {
        return (
            CheckOutcome::OtherServer,
            format!(
                "This address reaches a different Parson server (“{}”).",
                manifest["name"].as_str().unwrap_or("another library")
            ),
        );
    }
    (CheckOutcome::Reachable, "Reaches this server.".into())
}

/// Tailscale's MagicDNS name for this machine, when Tailscale is installed
/// and connected (`tailscale status --json`, `Self.DNSName`).
async fn tailscale_dns_name() -> Option<String> {
    let output = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::process::Command::new("tailscale")
            .args(["status", "--json"])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_tailscale_status(&output.stdout)
}

fn parse_tailscale_status(bytes: &[u8]) -> Option<String> {
    let status: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    if status["BackendState"]
        .as_str()
        .is_some_and(|state| state != "Running")
    {
        return None;
    }
    let name = status["Self"]["DNSName"]
        .as_str()?
        .trim()
        .trim_end_matches('.');
    let valid = !name.is_empty()
        && name.len() <= 253
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.'));
    valid.then(|| name.to_ascii_lowercase())
}

fn default_ssdp_targets() -> Vec<SocketAddr> {
    vec![SocketAddr::V4(SocketAddrV4::new(
        Ipv4Addr::new(239, 255, 255, 250),
        1900,
    ))]
}

/// Starts remote access for a server listening on `port`.
pub fn start(port: u16) {
    if let Some(service) = SERVICE.get() {
        service.port.store(port, Ordering::Relaxed);
        service.wake.notify_one();
        return;
    }
    let service = Arc::new(Service::new(
        load_config(),
        port,
        default_ssdp_targets(),
        true,
    ));
    if SERVICE.set(service.clone()).is_ok() {
        tokio::spawn(service.clone().run());
        tokio::spawn(async move { service.refresh_tailscale_name().await });
    }
}

fn service() -> Option<&'static Arc<Service>> {
    SERVICE.get()
}

fn network_access() -> bool {
    crate::settings::bind_address()
        .ok()
        .and_then(|address| address.parse::<std::net::IpAddr>().ok())
        .is_some_and(|address| !address.is_loopback())
}

fn host_addresses(port: u16) -> Vec<crate::discovery::HostAddress> {
    crate::discovery::host_info_without_remote(port)
        .map(|info| info.addresses)
        .unwrap_or_default()
}

/// Addresses a phone can use away from home (for `/host`).
pub fn remote_addresses(host_addresses: &[crate::discovery::HostAddress]) -> Vec<RemoteAddress> {
    service()
        .map(|service| service.addresses(host_addresses))
        .unwrap_or_default()
}

pub async fn status() -> Option<RemoteAccessStatus> {
    let service = service()?;
    service.refresh_tailscale_name().await;
    let addresses = host_addresses(service.port());
    Some(RemoteAccessStatus {
        config: service.config(),
        router: service.router(),
        addresses: service.addresses(&addresses),
        tailscale_name: service.tailscale_name(),
        last_check: service
            .last_check
            .lock()
            .expect("remote access check")
            .clone(),
        network_access: network_access(),
    })
}

/// Validates and applies a new configuration, then (re)starts the router
/// forward or removes it.
pub async fn update(mut config: RemoteAccessConfig) -> Result<RemoteAccessStatus, String> {
    let service = service().ok_or("Remote access is not running on this server.")?;
    config.manual_url = match config.manual_url.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(value) => Some(normalize_manual_url(value)?),
    };
    if config.mode == RemoteAccessMode::Manual && config.manual_url.is_none() {
        return Err("Enter the address to use away from home.".into());
    }
    if config.mode == RemoteAccessMode::Router && !network_access() {
        return Err(
            "This server only accepts connections from this computer, so a router can’t forward to it. Set PARSON_BIND_ADDRESS=0.0.0.0 first."
                .into(),
        );
    }
    if let Some(port) = config.external_port
        && port < 1024
    {
        return Err("Choose an outside port of 1024 or higher.".into());
    }
    if service.persist {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let bytes = serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?;
        std::fs::write(&path, bytes).map_err(|error| error.to_string())?;
    }
    *service.config.lock().expect("remote access config") = config.clone();
    *service.last_check.lock().expect("remote access check") = None;
    if config.mode == RemoteAccessMode::Router {
        // Set up the forward now so the answer can show the outcome.
        service.ensure_mapping(&config).await;
    } else {
        service.remove_mapping().await;
    }
    service.wake.notify_one();
    status()
        .await
        .ok_or_else(|| "Remote access is not running.".into())
}

pub async fn check() -> Option<CheckReport> {
    let service = service()?;
    service.refresh_tailscale_name().await;
    let addresses = host_addresses(service.port());
    Some(service.check(&addresses).await)
}

#[cfg(test)]
mod tests {
    use super::igd::fake::{FakeRouter, FakeRouterSpec};
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn fake_parson(instance_id: &'static str, name: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut buffer = [0_u8; 2048];
                let _ = stream.read(&mut buffer).await;
                let body = serde_json::json!({
                    "protocol": "parson", "instanceId": instance_id, "name": name
                })
                .to_string();
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            }
        });
        format!("http://{address}")
    }

    #[test]
    fn manual_addresses_are_normalized_to_origins() {
        assert_eq!(
            normalize_manual_url("music.example.com").unwrap(),
            "https://music.example.com"
        );
        assert_eq!(
            normalize_manual_url("http://home.duckdns.org:1993/").unwrap(),
            "http://home.duckdns.org:1993"
        );
        assert!(normalize_manual_url("ftp://example.com").is_err());
        assert!(normalize_manual_url("https://user:pw@example.com").is_err());
        assert!(normalize_manual_url("https://example.com/parson").is_err());
        assert!(normalize_manual_url("http://localhost:1993").is_err());
        assert!(normalize_manual_url("").is_err());
    }

    #[test]
    fn tailscale_status_yields_the_magicdns_name_only_when_connected() {
        let running =
            br#"{"BackendState":"Running","Self":{"DNSName":"Music-NAS.tail1234.ts.net."}}"#;
        assert_eq!(
            parse_tailscale_status(running).as_deref(),
            Some("music-nas.tail1234.ts.net")
        );
        let stopped = br#"{"BackendState":"Stopped","Self":{"DNSName":"nas.ts.net."}}"#;
        assert_eq!(parse_tailscale_status(stopped), None);
        let hostile = br#"{"BackendState":"Running","Self":{"DNSName":"evil.com/x?"}}"#;
        assert_eq!(parse_tailscale_status(hostile), None);
    }

    #[tokio::test]
    async fn router_mode_forwards_the_port_and_serves_the_public_address() {
        let router = FakeRouter::start(FakeRouterSpec {
            external_ip: "203.0.113.9",
            permanent_leases_only: false,
            refuse_port: None,
        })
        .await;
        let config = RemoteAccessConfig {
            mode: RemoteAccessMode::Router,
            manual_url: None,
            external_port: Some(41993),
        };
        let service = Service::new(config.clone(), 1993, vec![router.ssdp], false);
        let renew = service.ensure_mapping(&config).await;
        assert_eq!(renew, Duration::from_secs(1800));
        let state = service.router();
        assert_eq!(state.status, RouterStatus::Forwarded);
        assert_eq!(state.external_ip.as_deref(), Some("203.0.113.9"));
        assert_eq!(state.router_name.as_deref(), Some("Fake Home Router"));
        assert_eq!(
            router.state.lock().unwrap().mappings.get(&41993),
            Some(&("127.0.0.1".to_string(), 1993, 3600))
        );
        let addresses = service.addresses(&[]);
        assert_eq!(
            addresses,
            vec![RemoteAddress {
                origin: "http://203.0.113.9:41993".into(),
                kind: "router",
                label: "Port forward".into(),
                verified: None,
            }]
        );

        // Changing the outside port replaces the forward.
        let moved = RemoteAccessConfig {
            external_port: Some(42000),
            ..config
        };
        service.ensure_mapping(&moved).await;
        let mappings = router.state.lock().unwrap().mappings.clone();
        assert!(mappings.contains_key(&42000) && !mappings.contains_key(&41993));

        // Turning it off removes the forward from the router.
        service.remove_mapping().await;
        assert!(router.state.lock().unwrap().mappings.is_empty());
        assert_eq!(service.router().status, RouterStatus::Off);
    }

    #[tokio::test]
    async fn carrier_grade_nat_is_reported_honestly_and_not_offered_to_phones() {
        let router = FakeRouter::start(FakeRouterSpec {
            external_ip: "100.88.1.2",
            permanent_leases_only: false,
            refuse_port: None,
        })
        .await;
        let config = RemoteAccessConfig {
            mode: RemoteAccessMode::Router,
            ..RemoteAccessConfig::default()
        };
        let service = Service::new(config.clone(), 1993, vec![router.ssdp], false);
        service.ensure_mapping(&config).await;
        let state = service.router();
        assert_eq!(state.status, RouterStatus::Unreachable);
        assert!(state.message.unwrap().contains("Tailscale"));
        assert!(service.addresses(&[]).is_empty());
    }

    #[tokio::test]
    async fn a_router_conflict_explains_how_to_fix_it() {
        let router = FakeRouter::start(FakeRouterSpec {
            external_ip: "203.0.113.10",
            permanent_leases_only: false,
            refuse_port: Some(1993),
        })
        .await;
        let config = RemoteAccessConfig {
            mode: RemoteAccessMode::Router,
            ..RemoteAccessConfig::default()
        };
        let service = Service::new(config.clone(), 1993, vec![router.ssdp], false);
        service.ensure_mapping(&config).await;
        let state = service.router();
        assert_eq!(state.status, RouterStatus::Failed);
        assert!(state.message.unwrap().contains("different outside port"));
    }

    #[tokio::test]
    async fn the_check_confirms_each_address_reaches_this_server() {
        let this_server = fake_parson("00000000-0000-4000-8000-000000000001", "Home").await;
        let another = fake_parson("99999999-9999-4999-8999-999999999999", "Studio").await;
        let closed = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            drop(listener);
            format!("http://{address}")
        };
        let service = Service::new(
            RemoteAccessConfig {
                mode: RemoteAccessMode::Manual,
                manual_url: Some(this_server.clone()),
                external_port: None,
            },
            1993,
            Vec::new(),
            false,
        );
        let vpn = |origin: &str| crate::discovery::HostAddress {
            address: "100.101.102.103".into(),
            origin: origin.to_string(),
            interface: "tailscale0".into(),
            kind: "vpn",
        };
        let report = service.check(&[vpn(&another), vpn(&closed)]).await;
        let outcome = |origin: &str| {
            report
                .results
                .iter()
                .find(|result| result.origin == origin)
                .map(|result| result.outcome)
        };
        assert_eq!(outcome(&this_server), Some(CheckOutcome::Reachable));
        assert_eq!(outcome(&another), Some(CheckOutcome::OtherServer));
        assert_eq!(outcome(&closed), Some(CheckOutcome::Unreachable));
        let other = report
            .results
            .iter()
            .find(|result| result.origin == another)
            .unwrap();
        assert!(other.detail.contains("Studio"));
        // The phone app sees which addresses passed.
        let addresses = service.addresses(&[vpn(&another)]);
        assert_eq!(addresses[0].verified, Some(true));
        assert_eq!(addresses[1].verified, Some(false));
    }
}
