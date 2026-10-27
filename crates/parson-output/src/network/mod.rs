//! Network outputs: streamers and speakers the server plays to over the
//! LAN, after its own decoder and DSP chain.
//!
//! - [`raop`]: AirPlay receivers, through AirPlay 1 (RAOP): ALAC at
//!   44.1 kHz/16 bit over RTP, the only format that protocol carries.
//! - UPnP AV and OpenHome renderers fetch what the server serves them; the
//!   server's zones drive them (`parson-music`'s `outputs::upnp`), with the
//!   identity types here ([`NetworkInfo`], [`ManualEndpoint`]).
//!
//! Either way the DSP chain runs on the server before the network hop, the
//! format limits are per endpoint (declared by the renderer, fixed by the
//! protocol, or assumed and labelled as such), and every device says what
//! it was tested against.

pub mod http;
pub mod raop;

#[cfg(any(test, feature = "network-fakes"))]
pub mod fakes;
#[cfg(test)]
mod tests_end_to_end;

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// How Parson reaches a network endpoint.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum NetworkProtocol {
    /// UPnP AV MediaRenderer (AVTransport), also called DLNA.
    UpnpAv,
    /// An OpenHome renderer (Linn and many streamers): Product, Playlist
    /// and Info services; it can report what it is decoding.
    OpenHome,
    /// AirPlay 1 (RAOP): ALAC over RTP.
    AirPlay,
}

impl NetworkProtocol {
    pub fn label(self) -> &'static str {
        match self {
            Self::UpnpAv => "UPnP AV",
            Self::OpenHome => "OpenHome",
            Self::AirPlay => "AirPlay",
        }
    }
}

/// Where a network endpoint's format limits come from.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NetworkCapsSource {
    /// The renderer listed the rates it takes (`audio/L16;rate=…`).
    Declared,
    /// The protocol fixes the format (AirPlay 1: 44.1 kHz/16 bit ALAC).
    Protocol,
    /// The maker's published limits for this model (e.g. Sonos).
    KnownModel,
    /// The renderer takes WAV but does not say which rates; Parson assumes
    /// 44.1–192 kHz at 16 or 24 bit. Lower the max sample rate if it
    /// cannot play them.
    Assumed,
}

/// Broad device class, for the picture shown when the device has none.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EndpointKind {
    Streamer,
    Speaker,
    Television,
    Receiver,
    Computer,
}

/// What Parson knows about a network endpoint, beyond its formats.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct NetworkInfo {
    pub protocol: NetworkProtocol,
    /// The address Parson controls it at.
    pub host: String,
    pub port: u16,
    /// UPnP: the device description URL.
    pub location: Option<String>,
    pub manufacturer: Option<String>,
    /// e.g. `Klimax DSM` or `HomePod mini`.
    pub model: Option<String>,
    pub model_description: Option<String>,
    /// The device's own picture (UPnP icon), served through the API.
    pub icon_url: Option<String>,
    pub kind: EndpointKind,
    pub caps_source: NetworkCapsSource,
    /// How the audio travels, e.g. `WAV (LPCM) over HTTP`.
    pub stream_format: String,
    /// What the endpoint says it accepts, e.g. `WAV`, `L16 44.1/48kHz`,
    /// `FLAC`, `MP3`.
    pub accepts: Vec<String>,
    /// The endpoint reports what it is decoding while it plays
    /// (OpenHome `Info.Details`), so the path can be confirmed.
    pub reports_format: bool,
    /// Added by address rather than discovered.
    pub manual: bool,
    /// What this output has, and has not, been tested against.
    pub verification: String,
}

/// Said on every AirPlay device.
pub const AIRPLAY_VERIFICATION: &str = "Built to the AirPlay 1 (RAOP) protocol and tested end to end against Mujin Music's protocol-level fake receiver (RTSP, RTP audio, timing and sync, with the decoded ALAC compared sample for sample). Not yet tested with real AirPlay hardware. Receivers that require encryption, a password or pairing are listed but cannot be played to.";

/// Track details a renderer can show while it plays.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackInfo {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
}

/// An endpoint added by address, for networks where discovery does not
/// reach it (VLANs, multicast filtering, a VPN).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ManualEndpoint {
    pub protocol: NetworkProtocol,
    /// UPnP: a description URL, or `host[:port]` to search at. AirPlay:
    /// `host[:port]`.
    pub address: String,
    /// The name to show for an AirPlay receiver added by address.
    #[serde(default)]
    pub name: Option<String>,
}

impl ManualEndpoint {
    pub fn validate(&self) -> Result<(), String> {
        let address = self.address.trim();
        if address.is_empty() || address.len() > 256 || address.chars().any(char::is_control) {
            return Err("Enter the player's address, e.g. 192.168.1.40 or http://192.168.1.40:49152/description.xml.".into());
        }
        if self.protocol == NetworkProtocol::AirPlay && address.contains("://") {
            return Err(
                "Enter an AirPlay receiver as host or host:port, e.g. 192.168.1.50:7000.".into(),
            );
        }
        if address.contains("://") {
            http::parse_url(address)?;
        }
        if self.name.as_ref().is_some_and(|name| name.len() > 128) {
            return Err("The name is too long.".into());
        }
        Ok(())
    }

    /// `(host, port)` for a `host[:port]` address.
    pub fn host_port(&self, default_port: u16) -> Option<(String, u16)> {
        let address = self.address.trim();
        if address.contains("://") {
            let url = http::parse_url(address).ok()?;
            return Some((url.host, url.port));
        }
        if let Some(stripped) = address.strip_prefix('[') {
            let (host, rest) = stripped.split_once(']')?;
            let port = rest.strip_prefix(':').and_then(|port| port.parse().ok());
            return Some((host.to_string(), port.unwrap_or(default_port)));
        }
        match address.rsplit_once(':') {
            Some((host, port)) if !host.contains(':') => {
                Some((host.to_string(), port.parse().ok()?))
            }
            _ => Some((address.to_string(), default_port)),
        }
    }
}

/// Network endpoints added by address, shared by the network backends and
/// the API that edits them.
#[derive(Default)]
pub struct NetworkRegistry {
    manual: Mutex<Vec<ManualEndpoint>>,
}

impl NetworkRegistry {
    pub fn new(manual: Vec<ManualEndpoint>) -> Self {
        Self {
            manual: Mutex::new(manual),
        }
    }

    pub fn manual(&self) -> Vec<ManualEndpoint> {
        self.manual
            .lock()
            .map(|list| list.clone())
            .unwrap_or_default()
    }

    pub fn set_manual(&self, endpoints: Vec<ManualEndpoint>) {
        if let Ok(mut list) = self.manual.lock() {
            *list = endpoints;
        }
    }
}

/// The address this machine has on the route to `peer`: what a renderer
/// must use to reach the stream Parson serves.
pub fn local_ip_for(peer: SocketAddr) -> std::io::Result<IpAddr> {
    let bind: SocketAddr = if peer.is_ipv4() {
        "0.0.0.0:0".parse().expect("valid address")
    } else {
        "[::]:0".parse().expect("valid address")
    };
    let socket = UdpSocket::bind(bind)?;
    socket.connect(peer)?;
    Ok(socket.local_addr()?.ip())
}

/// Unpredictable hex for stream paths and session ids (the URL is the
/// only thing protecting a stream from other machines on the LAN).
pub fn random_hex(bytes: usize) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut out = String::with_capacity(bytes * 2);
    while out.len() < bytes * 2 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        hasher.write_u128(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0),
        );
        out.push_str(&format!("{:016x}", hasher.finish()));
    }
    out.truncate(bytes * 2);
    out
}

/// Converts little-endian PCM to the big-endian layout `audio/L16` and
/// `audio/L24` require (RFC 2586, RFC 3190).
pub fn to_big_endian(data: &[u8], bytes_per_sample: usize, out: &mut Vec<u8>) {
    out.clear();
    out.reserve(data.len());
    for sample in data.chunks_exact(bytes_per_sample) {
        out.extend(sample.iter().rev());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_endpoints_validate_and_split() {
        let upnp = ManualEndpoint {
            protocol: NetworkProtocol::UpnpAv,
            address: "http://192.168.1.40:49152/description.xml".into(),
            name: None,
        };
        assert!(upnp.validate().is_ok());
        assert_eq!(upnp.host_port(1900), Some(("192.168.1.40".into(), 49152)));
        let airplay = ManualEndpoint {
            protocol: NetworkProtocol::AirPlay,
            address: "10.0.0.9".into(),
            name: Some("Kitchen".into()),
        };
        assert_eq!(airplay.host_port(7000), Some(("10.0.0.9".into(), 7000)));
        let with_port = ManualEndpoint {
            address: "10.0.0.9:5000".into(),
            ..airplay.clone()
        };
        assert_eq!(with_port.host_port(7000), Some(("10.0.0.9".into(), 5000)));
        assert!(
            ManualEndpoint {
                address: "http://x/".into(),
                ..airplay
            }
            .validate()
            .is_err()
        );
        assert!(
            ManualEndpoint {
                address: " ".into(),
                ..upnp
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn random_hex_is_unique_and_sized() {
        let a = random_hex(16);
        let b = random_hex(16);
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn big_endian_swaps_each_sample() {
        let mut out = Vec::new();
        to_big_endian(&[1, 2, 3, 4], 2, &mut out);
        assert_eq!(out, [2, 1, 4, 3]);
        to_big_endian(&[1, 2, 3, 4, 5, 6], 3, &mut out);
        assert_eq!(out, [3, 2, 1, 6, 5, 4]);
    }
}
