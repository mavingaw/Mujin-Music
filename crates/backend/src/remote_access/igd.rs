//! A small UPnP Internet Gateway Device (IGD) client: finds the home router
//! with SSDP and asks it to forward a TCP port to this server
//! (`AddPortMapping`), reports the router's public address
//! (`GetExternalIPAddress`) and removes the forward again
//! (`DeletePortMapping`). Works with IGD v1 and v2 routers that expose
//! `WANIPConnection` or `WANPPPConnection`.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

use crate::outputs::upnp::description::absolute_url;
use crate::sonos::ssdp::{parse_response_for, search_with};

pub const IGD_V1: &str = "urn:schemas-upnp-org:device:InternetGatewayDevice:1";
pub const IGD_V2: &str = "urn:schemas-upnp-org:device:InternetGatewayDevice:2";
const CONNECTION_SERVICES: [&str; 3] = [
    "urn:schemas-upnp-org:service:WANIPConnection:2",
    "urn:schemas-upnp-org:service:WANIPConnection:1",
    "urn:schemas-upnp-org:service:WANPPPConnection:1",
];
const MAX_DESCRIPTION_BYTES: usize = 256 * 1024;
pub const MAPPING_DESCRIPTION: &str = "Parson remote access";

/// A router that accepted SSDP and exposes a WAN connection service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gateway {
    pub control_url: String,
    pub service_type: String,
    pub friendly_name: String,
    /// This computer's address on the router's network: the forward target.
    pub local_ip: Ipv4Addr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IgdError {
    /// No router answered SSDP: UPnP is off or the network blocks multicast.
    NotFound,
    /// The router refused (a UPnP fault, with its code and description).
    Refused { code: u16, description: String },
    /// Anything else: network errors, bad XML.
    Failed(String),
}

impl std::fmt::Display for IgdError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => formatter.write_str(
                "No router answered. UPnP (often called “automatic port forwarding”) may be switched off on the router, or this network blocks it.",
            ),
            Self::Refused { code, description } => {
                write!(formatter, "The router refused the port forward ({code} {description}).")
            }
            Self::Failed(message) => formatter.write_str(message),
        }
    }
}

fn http_client(timeout: Duration) -> Result<reqwest::Client, IgdError> {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| IgdError::Failed(error.to_string()))
}

/// Parses an IGD description; returns the WAN connection service's control
/// URL and type, and the router's friendly name.
pub fn parse_description(document: &str, location: &str) -> Option<(String, String, String)> {
    let document = roxmltree::Document::parse(document).ok()?;
    let base = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "URLBase")
        .and_then(|node| node.text())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(location)
        .to_string();
    let friendly_name = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "friendlyName")
        .and_then(|node| node.text())
        .map(|value| value.trim().chars().take(96).collect::<String>())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "Router".to_string());
    for wanted in CONNECTION_SERVICES {
        for service in document
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "service")
        {
            let child = |name: &str| {
                service
                    .children()
                    .find(|node| node.is_element() && node.tag_name().name() == name)
                    .and_then(|node| node.text())
                    .map(str::trim)
            };
            if child("serviceType") == Some(wanted)
                && let Some(control) = child("controlURL").and_then(|url| absolute_url(&base, url))
            {
                return Some((control, wanted.to_string(), friendly_name));
            }
        }
    }
    None
}

/// The local address the operating system would use to reach `host`.
fn local_ip_towards(host: &str, port: u16) -> Option<Ipv4Addr> {
    let target = format!("{host}:{port}");
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect(target).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(address) if !address.is_unspecified() => Some(address),
        _ => None,
    }
}

fn host_and_port(url: &str) -> Option<(String, u16)> {
    let rest = url.strip_prefix("http://")?;
    let authority = rest.split('/').next()?;
    match authority.rsplit_once(':') {
        Some((host, port)) => Some((host.to_string(), port.parse().ok()?)),
        None => Some((authority.to_string(), 80)),
    }
}

/// Finds the router with SSDP (`targets` is normally the UPnP multicast group).
pub async fn discover(targets: &[SocketAddr], timeout: Duration) -> Result<Gateway, IgdError> {
    let (v1, v2) = tokio::join!(
        search_with(targets, IGD_V1, timeout, |datagram| {
            parse_response_for(datagram, IGD_V1)
        }),
        search_with(targets, IGD_V2, timeout, |datagram| {
            parse_response_for(datagram, IGD_V2)
        }),
    );
    let mut locations = Vec::new();
    for response in v2.into_iter().chain(v1).flatten() {
        if !locations.contains(&response.location) {
            locations.push(response.location);
        }
    }
    if locations.is_empty() {
        return Err(IgdError::NotFound);
    }
    let http = http_client(Duration::from_secs(4))?;
    let mut last_error = IgdError::NotFound;
    for location in locations {
        let body = match http.get(&location).send().await {
            Ok(response) if response.status().is_success() => match response.bytes().await {
                Ok(bytes) if bytes.len() <= MAX_DESCRIPTION_BYTES => {
                    String::from_utf8_lossy(&bytes).into_owned()
                }
                Ok(_) => {
                    last_error = IgdError::Failed("the router description is too large".into());
                    continue;
                }
                Err(error) => {
                    last_error = IgdError::Failed(error.to_string());
                    continue;
                }
            },
            Ok(response) => {
                last_error = IgdError::Failed(format!(
                    "the router description returned HTTP {}",
                    response.status()
                ));
                continue;
            }
            Err(error) => {
                last_error = IgdError::Failed(error.to_string());
                continue;
            }
        };
        let Some((control_url, service_type, friendly_name)) = parse_description(&body, &location)
        else {
            last_error =
                IgdError::Failed("the router answered but offers no WAN connection service".into());
            continue;
        };
        let Some((host, port)) = host_and_port(&control_url) else {
            continue;
        };
        let Some(local_ip) = local_ip_towards(&host, port) else {
            last_error = IgdError::Failed("this computer has no route to the router".into());
            continue;
        };
        return Ok(Gateway {
            control_url,
            service_type,
            friendly_name,
            local_ip,
        });
    }
    Err(last_error)
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn envelope(service_type: &str, action: &str, arguments: &[(&str, String)]) -> String {
    let arguments = arguments
        .iter()
        .map(|(name, value)| format!("<{name}>{}</{name}>", escape(value)))
        .collect::<String>();
    format!(
        "<?xml version=\"1.0\"?>\r\n<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{service_type}\">{arguments}</u:{action}></s:Body></s:Envelope>"
    )
}

fn element_text(document: &roxmltree::Document<'_>, name: &str) -> Option<String> {
    document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == name)
        .map(|node| node.text().unwrap_or_default().trim().to_string())
}

async fn call(
    gateway: &Gateway,
    action: &str,
    arguments: &[(&str, String)],
) -> Result<String, IgdError> {
    let http = http_client(Duration::from_secs(6))?;
    let response = http
        .post(&gateway.control_url)
        .header("Content-Type", "text/xml; charset=\"utf-8\"")
        .header(
            "SOAPAction",
            format!("\"{}#{action}\"", gateway.service_type),
        )
        .body(envelope(&gateway.service_type, action, arguments))
        .send()
        .await
        .map_err(|error| IgdError::Failed(format!("the router did not answer: {error}")))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| IgdError::Failed(error.to_string()))?;
    if status.is_success() {
        return Ok(body);
    }
    let document = roxmltree::Document::parse(&body)
        .map_err(|_| IgdError::Failed(format!("the router returned HTTP {status}")))?;
    let code = element_text(&document, "errorCode")
        .and_then(|value| value.parse().ok())
        .unwrap_or(status.as_u16());
    let description = element_text(&document, "errorDescription").unwrap_or_default();
    Err(IgdError::Refused { code, description })
}

/// The router's public (WAN) IPv4 address.
pub async fn external_ip(gateway: &Gateway) -> Result<Ipv4Addr, IgdError> {
    let body = call(gateway, "GetExternalIPAddress", &[]).await?;
    let document =
        roxmltree::Document::parse(&body).map_err(|error| IgdError::Failed(error.to_string()))?;
    element_text(&document, "NewExternalIPAddress")
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| IgdError::Failed("the router did not report a public address".into()))
}

/// Forwards `external_port` on the router to this server. Routers that only
/// allow permanent forwards (fault 725) get one with no lease.
pub async fn add_port_mapping(
    gateway: &Gateway,
    external_port: u16,
    internal_port: u16,
    lease_seconds: u32,
) -> Result<u32, IgdError> {
    let arguments = |lease: u32| {
        vec![
            ("NewRemoteHost", String::new()),
            ("NewExternalPort", external_port.to_string()),
            ("NewProtocol", "TCP".to_string()),
            ("NewInternalPort", internal_port.to_string()),
            ("NewInternalClient", gateway.local_ip.to_string()),
            ("NewEnabled", "1".to_string()),
            ("NewPortMappingDescription", MAPPING_DESCRIPTION.to_string()),
            ("NewLeaseDuration", lease.to_string()),
        ]
    };
    match call(gateway, "AddPortMapping", &arguments(lease_seconds)).await {
        Err(IgdError::Refused { code: 725, .. }) if lease_seconds != 0 => {
            call(gateway, "AddPortMapping", &arguments(0)).await?;
            Ok(0)
        }
        Err(error) => Err(error),
        Ok(_) => Ok(lease_seconds),
    }
}

pub async fn delete_port_mapping(gateway: &Gateway, external_port: u16) -> Result<(), IgdError> {
    call(
        gateway,
        "DeletePortMapping",
        &[
            ("NewRemoteHost", String::new()),
            ("NewExternalPort", external_port.to_string()),
            ("NewProtocol", "TCP".to_string()),
        ],
    )
    .await
    .map(|_| ())
}

/// A public address that cannot be reached from the internet: the provider
/// shares one address between customers (carrier-grade NAT, 100.64.0.0/10)
/// or there is a second router in front of this one (a private address).
pub fn is_shared_address(address: Ipv4Addr) -> bool {
    let [first, second, ..] = address.octets();
    address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || address.is_unspecified()
        || (first == 100 && (64..128).contains(&second))
}

#[cfg(test)]
pub(crate) mod fake {
    //! A protocol-level fake router: answers SSDP M-SEARCH for IGD:1 and
    //! serves a device description and WANIPConnection SOAP actions, like a
    //! consumer router with UPnP switched on.

    use std::collections::HashMap;
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, UdpSocket};

    #[derive(Clone, Debug, Default)]
    pub struct FakeRouterState {
        pub mappings: HashMap<u16, (String, u16, u32)>,
        pub calls: Vec<String>,
    }

    pub struct FakeRouter {
        pub ssdp: SocketAddr,
        pub state: Arc<Mutex<FakeRouterState>>,
        tasks: Vec<tokio::task::JoinHandle<()>>,
    }

    impl Drop for FakeRouter {
        fn drop(&mut self) {
            for task in &self.tasks {
                task.abort();
            }
        }
    }

    #[derive(Clone)]
    pub struct FakeRouterSpec {
        pub external_ip: &'static str,
        pub permanent_leases_only: bool,
        pub refuse_port: Option<u16>,
    }

    fn header_end(buffer: &[u8]) -> Option<usize> {
        buffer.windows(4).position(|window| window == b"\r\n\r\n")
    }

    fn argument(body: &str, name: &str) -> String {
        body.split(&format!("<{name}>"))
            .nth(1)
            .and_then(|rest| rest.split(&format!("</{name}>")).next())
            .unwrap_or_default()
            .to_string()
    }

    fn fault(code: u16, description: &str) -> (u16, String) {
        (
            500,
            format!(
                "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><s:Fault><faultcode>s:Client</faultcode><faultstring>UPnPError</faultstring><detail><UPnPError xmlns=\"urn:schemas-upnp-org:control-1-0\"><errorCode>{code}</errorCode><errorDescription>{description}</errorDescription></UPnPError></detail></s:Fault></s:Body></s:Envelope>"
            ),
        )
    }

    fn ok(action: &str, inner: &str) -> (u16, String) {
        (
            200,
            format!(
                "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><u:{action}Response xmlns:u=\"urn:schemas-upnp-org:service:WANIPConnection:1\">{inner}</u:{action}Response></s:Body></s:Envelope>"
            ),
        )
    }

    const DESCRIPTION: &str = r#"<?xml version="1.0"?>
<root xmlns="urn:schemas-upnp-org:device-1-0">
  <specVersion><major>1</major><minor>0</minor></specVersion>
  <device>
    <deviceType>urn:schemas-upnp-org:device:InternetGatewayDevice:1</deviceType>
    <friendlyName>Fake Home Router</friendlyName>
    <deviceList><device>
      <deviceType>urn:schemas-upnp-org:device:WANDevice:1</deviceType>
      <deviceList><device>
        <deviceType>urn:schemas-upnp-org:device:WANConnectionDevice:1</deviceType>
        <serviceList><service>
          <serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType>
          <serviceId>urn:upnp-org:serviceId:WANIPConn1</serviceId>
          <controlURL>/ctl/IPConn</controlURL>
          <eventSubURL>/evt/IPConn</eventSubURL>
          <SCPDURL>/WANIPCn.xml</SCPDURL>
        </service></serviceList>
      </device></deviceList>
    </device></deviceList>
  </device>
</root>"#;

    impl FakeRouter {
        pub async fn start(spec: FakeRouterSpec) -> Self {
            let state = Arc::new(Mutex::new(FakeRouterState::default()));
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let http = listener.local_addr().unwrap();
            let mut tasks = Vec::new();
            {
                let state = state.clone();
                let spec = spec.clone();
                tasks.push(tokio::spawn(async move {
                    while let Ok((mut stream, _)) = listener.accept().await {
                        let state = state.clone();
                        let spec = spec.clone();
                        tokio::spawn(async move {
                            let mut buffer = Vec::new();
                            let mut chunk = [0_u8; 4096];
                            let (head, body) = loop {
                                let Ok(read) = stream.read(&mut chunk).await else {
                                    return;
                                };
                                if read == 0 {
                                    return;
                                }
                                buffer.extend_from_slice(&chunk[..read]);
                                if let Some(end) = header_end(&buffer) {
                                    let head = String::from_utf8_lossy(&buffer[..end]).to_string();
                                    let length = head
                                        .lines()
                                        .find_map(|line| {
                                            let (name, value) = line.split_once(':')?;
                                            name.eq_ignore_ascii_case("content-length")
                                                .then(|| value.trim().parse::<usize>().ok())?
                                        })
                                        .unwrap_or(0);
                                    while buffer.len() < end + 4 + length {
                                        let Ok(read) = stream.read(&mut chunk).await else {
                                            return;
                                        };
                                        if read == 0 {
                                            break;
                                        }
                                        buffer.extend_from_slice(&chunk[..read]);
                                    }
                                    let body = String::from_utf8_lossy(&buffer[end + 4..]).to_string();
                                    break (head, body);
                                }
                            };
                            let (status, response) = if head.starts_with("GET /rootDesc.xml") {
                                (200, DESCRIPTION.to_string())
                            } else if head.starts_with("POST /ctl/IPConn") {
                                let action = head
                                    .lines()
                                    .find_map(|line| {
                                        let (name, value) = line.split_once(':')?;
                                        name.eq_ignore_ascii_case("soapaction").then(|| {
                                            value.trim().trim_matches('"').rsplit('#').next().unwrap_or_default().to_string()
                                        })
                                    })
                                    .unwrap_or_default();
                                let mut state = state.lock().unwrap();
                                state.calls.push(action.clone());
                                match action.as_str() {
                                    "GetExternalIPAddress" => ok(
                                        &action,
                                        &format!("<NewExternalIPAddress>{}</NewExternalIPAddress>", spec.external_ip),
                                    ),
                                    "AddPortMapping" => {
                                        let external: u16 = argument(&body, "NewExternalPort").parse().unwrap_or(0);
                                        let lease: u32 = argument(&body, "NewLeaseDuration").parse().unwrap_or(0);
                                        if spec.refuse_port == Some(external) {
                                            fault(718, "ConflictInMappingEntry")
                                        } else if spec.permanent_leases_only && lease != 0 {
                                            fault(725, "OnlyPermanentLeasesSupported")
                                        } else {
                                            state.mappings.insert(
                                                external,
                                                (
                                                    argument(&body, "NewInternalClient"),
                                                    argument(&body, "NewInternalPort").parse().unwrap_or(0),
                                                    lease,
                                                ),
                                            );
                                            ok(&action, "")
                                        }
                                    }
                                    "DeletePortMapping" => {
                                        let external: u16 = argument(&body, "NewExternalPort").parse().unwrap_or(0);
                                        if state.mappings.remove(&external).is_some() {
                                            ok(&action, "")
                                        } else {
                                            fault(714, "NoSuchEntryInArray")
                                        }
                                    }
                                    _ => fault(401, "Invalid Action"),
                                }
                            } else {
                                (404, String::new())
                            };
                            let reply = format!(
                                "HTTP/1.1 {status} X\r\nContent-Type: text/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                                response.len()
                            );
                            let _ = stream.write_all(reply.as_bytes()).await;
                        });
                    }
                }));
            }
            let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let ssdp = udp.local_addr().unwrap();
            tasks.push(tokio::spawn(async move {
                let mut buffer = vec![0_u8; 4096];
                while let Ok((length, peer)) = udp.recv_from(&mut buffer).await {
                    let request = String::from_utf8_lossy(&buffer[..length]).to_string();
                    if !request.starts_with("M-SEARCH") || !request.contains(super::IGD_V1) {
                        continue;
                    }
                    let answer = format!(
                        "HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=120\r\nEXT:\r\nLOCATION: http://{http}/rootDesc.xml\r\nSERVER: FakeOS/1.0 UPnP/1.1 FakeRouter/1.0\r\nST: {}\r\nUSN: uuid:fake-router::{}\r\n\r\n",
                        super::IGD_V1,
                        super::IGD_V1
                    );
                    let _ = udp.send_to(answer.as_bytes(), peer).await;
                }
            }));
            Self { ssdp, state, tasks }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{FakeRouter, FakeRouterSpec};
    use super::*;

    #[test]
    fn descriptions_resolve_the_wan_service_against_url_base() {
        let document = r#"<root><URLBase>http://192.168.1.1:5000/</URLBase><device>
            <friendlyName>  Box  </friendlyName><serviceList>
            <service><serviceType>urn:schemas-upnp-org:service:Layer3Forwarding:1</serviceType><controlURL>/l3f</controlURL></service>
            <service><serviceType>urn:schemas-upnp-org:service:WANPPPConnection:1</serviceType><controlURL>ctl/ppp</controlURL></service>
            </serviceList></device></root>"#;
        let (control, service, name) =
            parse_description(document, "http://10.0.0.1/desc.xml").expect("wan service");
        assert_eq!(control, "http://192.168.1.1:5000/ctl/ppp");
        assert_eq!(service, "urn:schemas-upnp-org:service:WANPPPConnection:1");
        assert_eq!(name, "Box");
        assert!(parse_description("<root/>", "http://10.0.0.1/").is_none());
    }

    #[test]
    fn soap_arguments_are_escaped() {
        let body = envelope("urn:x", "AddPortMapping", &[("A", "<&>".to_string())]);
        assert!(body.contains("<A>&lt;&amp;&gt;</A>"));
        assert!(body.contains("<u:AddPortMapping xmlns:u=\"urn:x\">"));
    }

    #[test]
    fn shared_addresses_cannot_be_reached_from_outside() {
        assert!(is_shared_address(Ipv4Addr::new(100, 72, 1, 9)));
        assert!(is_shared_address(Ipv4Addr::new(192, 168, 0, 2)));
        assert!(is_shared_address(Ipv4Addr::new(10, 1, 1, 1)));
        assert!(!is_shared_address(Ipv4Addr::new(100, 128, 0, 1)));
        assert!(!is_shared_address(Ipv4Addr::new(203, 0, 113, 7)));
    }

    #[tokio::test]
    async fn forwards_a_port_through_a_upnp_router_and_removes_it() {
        let router = FakeRouter::start(FakeRouterSpec {
            external_ip: "203.0.113.7",
            permanent_leases_only: false,
            refuse_port: None,
        })
        .await;
        let gateway = discover(&[router.ssdp], Duration::from_millis(600))
            .await
            .expect("router found");
        assert_eq!(gateway.friendly_name, "Fake Home Router");
        assert_eq!(gateway.local_ip, Ipv4Addr::LOCALHOST);
        assert_eq!(
            external_ip(&gateway).await.unwrap(),
            Ipv4Addr::new(203, 0, 113, 7)
        );
        assert_eq!(
            add_port_mapping(&gateway, 41993, 1993, 3600).await.unwrap(),
            3600
        );
        assert_eq!(
            router.state.lock().unwrap().mappings.get(&41993),
            Some(&("127.0.0.1".to_string(), 1993, 3600))
        );
        delete_port_mapping(&gateway, 41993).await.unwrap();
        assert!(router.state.lock().unwrap().mappings.is_empty());
        assert_eq!(
            delete_port_mapping(&gateway, 41993).await,
            Err(IgdError::Refused {
                code: 714,
                description: "NoSuchEntryInArray".into()
            })
        );
    }

    #[tokio::test]
    async fn falls_back_to_a_permanent_forward_and_reports_conflicts() {
        let router = FakeRouter::start(FakeRouterSpec {
            external_ip: "203.0.113.8",
            permanent_leases_only: true,
            refuse_port: Some(1993),
        })
        .await;
        let gateway = discover(&[router.ssdp], Duration::from_millis(600))
            .await
            .unwrap();
        assert_eq!(add_port_mapping(&gateway, 8443, 1993, 3600).await, Ok(0));
        assert_eq!(
            router
                .state
                .lock()
                .unwrap()
                .mappings
                .get(&8443)
                .map(|m| m.2),
            Some(0)
        );
        let conflict = add_port_mapping(&gateway, 1993, 1993, 3600)
            .await
            .unwrap_err();
        assert!(matches!(conflict, IgdError::Refused { code: 718, .. }));
        assert!(conflict.to_string().contains("ConflictInMappingEntry"));
    }

    #[tokio::test]
    async fn reports_when_no_router_answers() {
        let silent = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let target = silent.local_addr().unwrap();
        assert_eq!(
            discover(&[target], Duration::from_millis(200)).await,
            Err(IgdError::NotFound)
        );
    }
}
