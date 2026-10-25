//! SSDP (UPnP discovery) for Sonos ZonePlayers.
//!
//! Sonos players answer `M-SEARCH` requests for
//! `urn:schemas-upnp-org:device:ZonePlayer:1` on the UPnP multicast group
//! `239.255.255.250:1900`. Each answer carries a `LOCATION` header with the
//! player's device description (`http://<ip>:1400/xml/device_description.xml`).

use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use tokio::net::UdpSocket;

/// UPnP multicast group and port.
pub const SSDP_MULTICAST: SocketAddr =
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(239, 255, 255, 250), 1900));
/// Search target that only Sonos players answer.
pub const ZONE_PLAYER_ST: &str = "urn:schemas-upnp-org:device:ZonePlayer:1";
const MAX_DATAGRAM_BYTES: usize = 8 * 1024;
const MAX_RESPONSES: usize = 256;

/// One SSDP search answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SsdpResponse {
    /// URL of the UPnP device description.
    pub location: String,
    /// Unique service name, e.g. `uuid:RINCON_xxx::urn:...`.
    pub usn: Option<String>,
    /// The search target the device answered for.
    pub st: Option<String>,
    /// `SERVER` header, e.g. `Linux UPnP/1.0 Sonos/79.1-56030 (ZPS9)`.
    pub server: Option<String>,
    /// Sonos household id (`X-RINCON-HOUSEHOLD`).
    pub household: Option<String>,
}

impl SsdpResponse {
    /// The `RINCON_...` player id embedded in the USN, when present.
    pub fn player_uuid(&self) -> Option<String> {
        let usn = self.usn.as_deref()?;
        let rest = usn.strip_prefix("uuid:")?;
        let id = rest.split("::").next()?.trim();
        (!id.is_empty()).then(|| id.to_string())
    }
}

/// Builds an `M-SEARCH` request for `search_target`.
pub fn m_search_request(search_target: &str, mx_seconds: u8) -> String {
    format!(
        "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: {}\r\nST: {search_target}\r\nUSER-AGENT: Parson/1.0 UPnP/1.1\r\n\r\n",
        mx_seconds.clamp(1, 5)
    )
}

/// Search target of UPnP/DLNA media renderers.
pub const MEDIA_RENDERER_ST: &str = "urn:schemas-upnp-org:device:MediaRenderer:1";

/// Parses an SSDP search answer, returning `None` for anything that is not a
/// `200 OK` from a Sonos ZonePlayer with an HTTP location.
pub fn parse_response(datagram: &[u8]) -> Option<SsdpResponse> {
    let response = parse_answer(datagram)?;
    let is_zone_player = response
        .st
        .as_deref()
        .is_some_and(|st| st.eq_ignore_ascii_case(ZONE_PLAYER_ST))
        || response
            .usn
            .as_deref()
            .is_some_and(|usn| usn.contains("RINCON_"));
    is_zone_player.then_some(response)
}

/// Parses an SSDP search answer for `search_target` (any `200 OK` whose ST
/// or USN names it and whose location is HTTP).
pub fn parse_response_for(datagram: &[u8], search_target: &str) -> Option<SsdpResponse> {
    let response = parse_answer(datagram)?;
    let matches = response
        .st
        .as_deref()
        .is_some_and(|st| st.eq_ignore_ascii_case(search_target))
        || response
            .usn
            .as_deref()
            .is_some_and(|usn| usn.ends_with(search_target));
    matches.then_some(response)
}

fn parse_answer(datagram: &[u8]) -> Option<SsdpResponse> {
    let text = std::str::from_utf8(datagram).ok()?;
    let mut lines = text.split("\r\n").flat_map(|line| line.split('\n'));
    let status = lines.next()?.trim();
    let mut parts = status.split_whitespace();
    if !parts.next()?.eq_ignore_ascii_case("HTTP/1.1") || parts.next()? != "200" {
        return None;
    }
    let mut response = SsdpResponse {
        location: String::new(),
        usn: None,
        st: None,
        server: None,
        household: None,
    };
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().to_string();
        match name.trim().to_ascii_lowercase().as_str() {
            "location" => response.location = value,
            "usn" => response.usn = Some(value),
            "st" => response.st = Some(value),
            "server" => response.server = Some(value),
            "x-rincon-household" => response.household = Some(value),
            _ => {}
        }
    }
    response.location.starts_with("http://").then_some(response)
}

/// Sends `M-SEARCH` to every target and gathers answers until `timeout`.
///
/// `targets` is normally just [`SSDP_MULTICAST`]; tests and networks without
/// multicast can point it at unicast addresses instead.
pub async fn search(
    targets: &[SocketAddr],
    timeout: Duration,
) -> std::io::Result<Vec<SsdpResponse>> {
    search_with(targets, ZONE_PLAYER_ST, timeout, parse_response).await
}

/// Sends `M-SEARCH` for `search_target` and keeps answers `parse` accepts.
pub async fn search_with(
    targets: &[SocketAddr],
    search_target: &str,
    timeout: Duration,
    parse: impl Fn(&[u8]) -> Option<SsdpResponse>,
) -> std::io::Result<Vec<SsdpResponse>> {
    let all_loopback = targets.iter().all(|target| target.ip().is_loopback());
    let bind: SocketAddr = if all_loopback {
        (Ipv4Addr::LOCALHOST, 0).into()
    } else {
        (Ipv4Addr::UNSPECIFIED, 0).into()
    };
    let socket = UdpSocket::bind(bind).await?;
    let _ = socket.set_multicast_ttl_v4(2);
    let request = m_search_request(search_target, 1);
    // UDP is lossy: send twice, as UPnP recommends.
    for _ in 0..2 {
        for target in targets {
            if let Err(error) = socket.send_to(request.as_bytes(), target).await {
                tracing::debug!(%error, %target, "SSDP search send failed");
            }
        }
    }
    let mut seen = HashSet::new();
    let mut responses = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;
    let mut buffer = vec![0_u8; MAX_DATAGRAM_BYTES];
    while responses.len() < MAX_RESPONSES {
        let received = tokio::time::timeout_at(deadline, socket.recv_from(&mut buffer)).await;
        let Ok(Ok((length, _peer))) = received else {
            break;
        };
        if let Some(response) = parse(&buffer[..length])
            && seen.insert(response.location.clone())
        {
            responses.push(response);
        }
    }
    Ok(responses)
}

#[cfg(test)]
mod tests {
    use super::{ZONE_PLAYER_ST, m_search_request, parse_response};

    #[test]
    fn search_request_targets_zone_players_with_bounded_mx() {
        let request = m_search_request(ZONE_PLAYER_ST, 9);
        assert!(request.starts_with("M-SEARCH * HTTP/1.1\r\n"));
        assert!(request.contains("MAN: \"ssdp:discover\"\r\n"));
        assert!(request.contains("MX: 5\r\n"));
        assert!(request.contains("ST: urn:schemas-upnp-org:device:ZonePlayer:1\r\n"));
        assert!(request.ends_with("\r\n\r\n"));
    }

    #[test]
    fn parses_real_sonos_answers_and_ignores_other_devices() {
        let answer = b"HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age = 1800\r\nEXT:\r\nLOCATION: http://192.168.1.20:1400/xml/device_description.xml\r\nSERVER: Linux UPnP/1.0 Sonos/79.1-56030 (ZPS9)\r\nST: urn:schemas-upnp-org:device:ZonePlayer:1\r\nUSN: uuid:RINCON_000E58A1B2C301400::urn:schemas-upnp-org:device:ZonePlayer:1\r\nX-RINCON-HOUSEHOLD: Sonos_abc\r\n\r\n";
        let parsed = parse_response(answer).expect("sonos answer");
        assert_eq!(
            parsed.location,
            "http://192.168.1.20:1400/xml/device_description.xml"
        );
        assert_eq!(
            parsed.player_uuid().as_deref(),
            Some("RINCON_000E58A1B2C301400")
        );
        assert_eq!(parsed.household.as_deref(), Some("Sonos_abc"));

        let router = b"HTTP/1.1 200 OK\r\nLOCATION: http://192.168.1.1:5000/rootDesc.xml\r\nST: upnp:rootdevice\r\nUSN: uuid:1234::upnp:rootdevice\r\n\r\n";
        assert!(parse_response(router).is_none());
        let notify = b"NOTIFY * HTTP/1.1\r\nLOCATION: http://192.168.1.20:1400/x.xml\r\nST: urn:schemas-upnp-org:device:ZonePlayer:1\r\n\r\n";
        assert!(parse_response(notify).is_none());
        let not_http = b"HTTP/1.1 200 OK\r\nLOCATION: file:///etc/passwd\r\nST: urn:schemas-upnp-org:device:ZonePlayer:1\r\n\r\n";
        assert!(parse_response(not_http).is_none());
        assert!(parse_response(&[0xff, 0xfe]).is_none());
    }
}
