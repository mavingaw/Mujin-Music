//! UPnP AV and OpenHome renderers.
//!
//! Discovery is SSDP (`M-SEARCH` for MediaRenderer and OpenHome Product
//! devices), or a unicast search / description URL for renderers added by
//! address. Each renderer's description names its services; its
//! ConnectionManager's sink protocol info says which streams it takes,
//! which decides the formats Parson sends it.
//!
//! Playback: the server runs its whole chain (decode, DSP, dither) and
//! serves the result as an endless WAV (or `audio/L16`/`L24`) over HTTP on
//! its own port; it then tells the renderer, over SOAP, to play that URL:
//! `AVTransport.SetAVTransportURI` + `Play`, or for an OpenHome renderer
//! without AVTransport, `Playlist.Insert` + `SeekId`. The renderer pulls
//! the stream, and TCP back pressure paces the chain in real time.
//!
//! An OpenHome renderer also answers `Info.Details` with the codec, rate,
//! depth and losslessness it is decoding, which Parson shows as the
//! renderer's own confirmation of the path.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::stream::{PcmStream, wav_header};
use super::{
    EndpointKind, ManualEndpoint, NetworkCapsSource, NetworkInfo, NetworkProtocol, NetworkRegistry,
    TrackInfo, UPNP_VERIFICATION, http, local_ip_for, random_hex, to_big_endian, xml,
};
use crate::procfs::{HardwareReporter, HardwareStatus, HwParams};
use crate::{
    Backend, DeviceCaps, DeviceInfo, Driver, STANDARD_RATES, SampleFormat, Sink, StreamFormat,
};

/// The SSDP multicast group.
pub const SSDP_MULTICAST: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(239, 255, 255, 250)), 1900);

const SEARCH_TARGETS: [&str; 2] = [
    "urn:schemas-upnp-org:device:MediaRenderer:1",
    "urn:av-openhome-org:service:Product:1",
];

const DESCRIPTION_TIMEOUT: Duration = Duration::from_secs(3);
const SOAP_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_DESCRIPTION: usize = 512 * 1024;

/// Highest rate assumed for a renderer that takes WAV without listing rates.
pub const ASSUMED_MAX_RATE: u32 = 192_000;

/* ------------------------------------------------------------------ */
/* SSDP                                                                */
/* ------------------------------------------------------------------ */

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SsdpResponse {
    pub location: String,
    pub st: Option<String>,
    pub usn: Option<String>,
    pub server: Option<String>,
}

/// Parses a unicast `M-SEARCH` response or a `NOTIFY`.
pub fn parse_ssdp(message: &str) -> Option<SsdpResponse> {
    let mut lines = message.split("\r\n");
    let first = lines.next()?;
    if !(first.starts_with("HTTP/1.1 200") || first.starts_with("NOTIFY")) {
        return None;
    }
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_uppercase(), value.trim().to_string());
        }
    }
    Some(SsdpResponse {
        location: headers
            .remove("LOCATION")
            .filter(|value| value.starts_with("http://"))?,
        st: headers.remove("ST").or_else(|| headers.remove("NT")),
        usn: headers.remove("USN"),
        server: headers.remove("SERVER"),
    })
}

/// Sends `M-SEARCH` for renderers to `target` (the multicast group, or one
/// host for a renderer added by address) and collects the answers for
/// `wait`.
pub fn ssdp_search(target: SocketAddr, wait: Duration) -> Vec<SsdpResponse> {
    let Ok(socket) = UdpSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)) else {
        return Vec::new();
    };
    let _ = socket.set_multicast_ttl_v4(2);
    let mx = wait.as_secs().clamp(1, 5);
    for st in SEARCH_TARGETS {
        let request = format!(
            "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: {mx}\r\nST: {st}\r\nUSER-AGENT: Parson/1.0 UPnP/1.1\r\n\r\n"
        );
        let _ = socket.send_to(request.as_bytes(), target);
    }
    let deadline = Instant::now() + wait;
    let mut found: BTreeMap<String, SsdpResponse> = BTreeMap::new();
    let mut buffer = [0u8; 4096];
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        if remaining.is_zero() || socket.set_read_timeout(Some(remaining)).is_err() {
            break;
        }
        let Ok((length, _)) = socket.recv_from(&mut buffer) else {
            break;
        };
        if let Some(response) = parse_ssdp(&String::from_utf8_lossy(&buffer[..length])) {
            found.entry(response.location.clone()).or_insert(response);
        }
    }
    found.into_values().collect()
}

/* ------------------------------------------------------------------ */
/* Device description                                                  */
/* ------------------------------------------------------------------ */

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Service {
    pub service_type: String,
    pub control_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Icon {
    pub mime: String,
    pub width: u32,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Description {
    pub location: String,
    pub udn: String,
    pub friendly_name: String,
    pub manufacturer: Option<String>,
    pub model_name: Option<String>,
    pub model_number: Option<String>,
    pub model_description: Option<String>,
    pub device_type: Option<String>,
    pub icons: Vec<Icon>,
    pub services: Vec<Service>,
}

impl Description {
    fn service(&self, kind: &str) -> Option<&Service> {
        self.services
            .iter()
            .find(|service| service.service_type.contains(kind))
    }

    pub fn av_transport(&self) -> Option<&Service> {
        self.service(":service:AVTransport:")
    }

    pub fn connection_manager(&self) -> Option<&Service> {
        self.service(":service:ConnectionManager:")
    }

    pub fn oh_playlist(&self) -> Option<&Service> {
        self.service("av-openhome-org:service:Playlist:")
    }

    pub fn oh_info(&self) -> Option<&Service> {
        self.service("av-openhome-org:service:Info:")
    }

    pub fn oh_product(&self) -> Option<&Service> {
        self.service("av-openhome-org:service:Product:")
    }

    pub fn protocol(&self) -> NetworkProtocol {
        if self.oh_product().is_some() || self.oh_playlist().is_some() {
            NetworkProtocol::OpenHome
        } else {
            NetworkProtocol::UpnpAv
        }
    }

    /// A renderer Parson can play to.
    pub fn is_renderer(&self) -> bool {
        self.av_transport().is_some() || self.oh_playlist().is_some()
    }

    /// The best picture: the widest PNG, else the widest icon.
    pub fn best_icon(&self) -> Option<&Icon> {
        let png = self
            .icons
            .iter()
            .filter(|icon| icon.mime.contains("png"))
            .max_by_key(|icon| icon.width);
        png.or_else(|| self.icons.iter().max_by_key(|icon| icon.width))
    }
}

/// Parses a device description fetched from `location`.
pub fn parse_description(text: &str, location: &str) -> Result<Description, String> {
    let device = xml::elements(text, "device")
        .first()
        .copied()
        .ok_or("the description has no device")?;
    let base = xml::text(text, "URLBase").unwrap_or_else(|| location.to_string());
    let friendly_name = xml::text(device, "friendlyName").ok_or("the device has no name")?;
    let icons = xml::elements(device, "icon")
        .into_iter()
        .filter_map(|icon| {
            Some(Icon {
                mime: xml::text(icon, "mimetype").unwrap_or_default(),
                width: xml::text(icon, "width")
                    .and_then(|width| width.parse().ok())
                    .unwrap_or(0),
                url: http::resolve(&base, &xml::text(icon, "url")?),
            })
        })
        .collect();
    let services = xml::elements(text, "service")
        .into_iter()
        .filter_map(|service| {
            Some(Service {
                service_type: xml::text(service, "serviceType")?,
                control_url: http::resolve(&base, &xml::text(service, "controlURL")?),
            })
        })
        .collect();
    Ok(Description {
        location: location.to_string(),
        udn: xml::text(device, "UDN").unwrap_or_else(|| location.to_string()),
        friendly_name,
        manufacturer: xml::text(device, "manufacturer"),
        model_name: xml::text(device, "modelName"),
        model_number: xml::text(device, "modelNumber"),
        model_description: xml::text(device, "modelDescription"),
        device_type: xml::text(device, "deviceType"),
        icons,
        services,
    })
}

pub fn fetch_description(location: &str) -> Result<Description, String> {
    let response = http::get(location, DESCRIPTION_TIMEOUT, MAX_DESCRIPTION)
        .map_err(|error| format!("could not read {location}: {error}"))?;
    if response.status != 200 {
        return Err(format!("{location} answered HTTP {}", response.status));
    }
    parse_description(&response.text(), location)
}

/* ------------------------------------------------------------------ */
/* Protocol info                                                       */
/* ------------------------------------------------------------------ */

/// The PCM streams a renderer's sink protocol info accepts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamSupport {
    /// `audio/wav` and its aliases.
    pub wav: bool,
    /// `audio/L16`: `(rate, channels)` pairs; rate 0 means any.
    pub l16: Vec<(u32, u8)>,
    pub l24: Vec<(u32, u8)>,
    /// Everything else it lists, as short labels (`FLAC`, `MP3`).
    pub others: Vec<String>,
}

impl StreamSupport {
    pub fn takes_pcm(&self) -> bool {
        self.wav || !self.l16.is_empty() || !self.l24.is_empty()
    }

    /// Labels for the UI: `WAV`, `L16 44.1/48kHz`, `FLAC`.
    pub fn labels(&self) -> Vec<String> {
        let mut labels = Vec::new();
        if self.wav {
            labels.push("WAV".to_string());
        }
        for (name, entries) in [("L16", &self.l16), ("L24", &self.l24)] {
            if entries.is_empty() {
                continue;
            }
            let mut rates: Vec<u32> = entries.iter().map(|(rate, _)| *rate).collect();
            rates.sort_unstable();
            rates.dedup();
            if rates.contains(&0) {
                labels.push(name.to_string());
            } else {
                labels.push(format!(
                    "{name} {}kHz",
                    rates
                        .iter()
                        .map(|rate| crate::network::upnp::khz(*rate))
                        .collect::<Vec<_>>()
                        .join("/")
                ));
            }
        }
        labels.extend(self.others.iter().cloned());
        labels
    }
}

pub(crate) fn khz(rate: u32) -> String {
    if rate.is_multiple_of(1000) {
        format!("{}", rate / 1000)
    } else {
        format!("{:.1}", f64::from(rate) / 1000.0)
    }
}

/// Parses a ConnectionManager `Sink` list.
pub fn parse_protocol_info(sink: &str) -> StreamSupport {
    let mut support = StreamSupport::default();
    for entry in sink.split(',') {
        let fields: Vec<&str> = entry.trim().splitn(4, ':').collect();
        if fields.len() < 3 || !fields[0].eq_ignore_ascii_case("http-get") {
            continue;
        }
        let mut parts = fields[2].split(';');
        let mime = parts.next().unwrap_or("").trim().to_ascii_lowercase();
        let mut rate = 0;
        let mut channels = 2;
        for parameter in parts {
            if let Some((key, value)) = parameter.split_once('=') {
                match key.trim().to_ascii_lowercase().as_str() {
                    "rate" => rate = value.trim().parse().unwrap_or(0),
                    "channels" => channels = value.trim().parse().unwrap_or(2),
                    _ => {}
                }
            }
        }
        match mime.as_str() {
            "audio/wav" | "audio/wave" | "audio/x-wav" | "audio/vnd.wave" => support.wav = true,
            "audio/l16" => support.l16.push((rate, channels)),
            "audio/l24" => support.l24.push((rate, channels)),
            other => {
                let label = match other {
                    "audio/flac" | "audio/x-flac" => "FLAC",
                    "audio/mpeg" | "audio/mp3" => "MP3",
                    "audio/mp4" | "audio/aac" | "audio/x-m4a" | "audio/m4a" => "AAC/ALAC",
                    "audio/x-aiff" | "audio/aiff" => "AIFF",
                    "audio/ogg" | "audio/x-ogg" => "Ogg",
                    "audio/x-ms-wma" => "WMA",
                    "audio/x-dsd" | "audio/dsd" | "audio/x-dsf" | "audio/x-dff" => "DSD",
                    _ => continue,
                };
                if !support.others.iter().any(|known| known == label) {
                    support.others.push(label.to_string());
                }
            }
        }
    }
    support
}

/// Published limits for renderers that list WAV but cannot play every
/// rate it allows, matched on the manufacturer.
fn known_model_limits(description: &Description) -> Option<(u32, u8, &'static str)> {
    let manufacturer = description.manufacturer.as_deref()?.to_ascii_lowercase();
    if manufacturer.contains("sonos") {
        return Some((48_000, 16, "Sonos players take WAV up to 48kHz/16bit"));
    }
    None
}

/// The formats Parson sends, and why.
pub fn caps_for(
    description: &Description,
    support: &StreamSupport,
) -> (Option<DeviceCaps>, NetworkCapsSource, Option<String>) {
    if !support.takes_pcm() {
        let listed = if support.others.is_empty() {
            "no audio streams".to_string()
        } else {
            support.others.join(", ")
        };
        return (
            None,
            NetworkCapsSource::Declared,
            Some(format!(
                "This renderer lists {listed} but no uncompressed PCM stream (WAV or LPCM), which is what Parson sends after its DSP chain."
            )),
        );
    }
    let declared_rates = |entries: &[(u32, u8)]| -> Vec<u32> {
        let mut rates: Vec<u32> = entries
            .iter()
            .map(|(rate, _)| *rate)
            .filter(|rate| *rate > 0)
            .collect();
        rates.sort_unstable();
        rates.dedup();
        rates
    };
    let max_channels = |entries: &[(u32, u8)]| entries.iter().map(|(_, channels)| *channels).max();
    let (rates, formats, source, channels) = if support.wav {
        match known_model_limits(description) {
            Some((max_rate, bits, _)) => (
                STANDARD_RATES
                    .iter()
                    .copied()
                    .filter(|rate| *rate <= max_rate)
                    .collect(),
                if bits >= 24 {
                    vec![SampleFormat::S16, SampleFormat::S24Packed]
                } else {
                    vec![SampleFormat::S16]
                },
                NetworkCapsSource::KnownModel,
                2,
            ),
            None => (
                STANDARD_RATES
                    .iter()
                    .copied()
                    .filter(|rate| *rate <= ASSUMED_MAX_RATE)
                    .collect(),
                vec![SampleFormat::S16, SampleFormat::S24Packed],
                NetworkCapsSource::Assumed,
                2,
            ),
        }
    } else {
        let l24 = declared_rates(&support.l24);
        let l16 = declared_rates(&support.l16);
        let any_rate = support
            .l16
            .iter()
            .chain(&support.l24)
            .any(|(rate, _)| *rate == 0);
        let mut rates: Vec<u32> = l16.iter().chain(&l24).copied().collect();
        if any_rate {
            rates.extend(
                STANDARD_RATES
                    .iter()
                    .copied()
                    .filter(|rate| *rate <= ASSUMED_MAX_RATE),
            );
        }
        rates.sort_unstable();
        rates.dedup();
        let mut formats = Vec::new();
        if !support.l16.is_empty() {
            formats.push(SampleFormat::S16);
        }
        if !support.l24.is_empty() {
            formats.push(SampleFormat::S24Packed);
        }
        let channels = max_channels(&support.l16)
            .into_iter()
            .chain(max_channels(&support.l24))
            .max()
            .unwrap_or(2)
            .clamp(1, 8);
        (
            rates,
            formats,
            if any_rate {
                NetworkCapsSource::Assumed
            } else {
                NetworkCapsSource::Declared
            },
            channels,
        )
    };
    (
        Some(DeviceCaps {
            sample_rates: rates,
            sample_formats: formats,
            min_channels: 2.min(channels),
            max_channels: channels,
            dsd_formats: Vec::new(),
            dsd_rates: Vec::new(),
        }),
        source,
        None,
    )
}

/// How the PCM is wrapped for one stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Container {
    Wav,
    L16,
    L24,
}

impl Container {
    pub fn mime(self, format: &StreamFormat) -> String {
        match self {
            Self::Wav => "audio/wav".into(),
            Self::L16 => format!(
                "audio/L16;rate={};channels={}",
                format.sample_rate, format.channels
            ),
            Self::L24 => format!(
                "audio/L24;rate={};channels={}",
                format.sample_rate, format.channels
            ),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Wav => "WAV (LPCM) over HTTP",
            Self::L16 => "LPCM (audio/L16) over HTTP",
            Self::L24 => "LPCM (audio/L24) over HTTP",
        }
    }
}

/// The container for a stream format, or why the renderer cannot take it.
pub fn choose_container(
    support: &StreamSupport,
    format: &StreamFormat,
) -> Result<Container, String> {
    let fits = |entries: &[(u32, u8)]| {
        entries.iter().any(|(rate, channels)| {
            (*rate == 0 || *rate == format.sample_rate) && *channels >= format.channels
        })
    };
    match format.sample_format {
        SampleFormat::S16 | SampleFormat::S24Packed if support.wav => Ok(Container::Wav),
        SampleFormat::S16 if fits(&support.l16) => Ok(Container::L16),
        SampleFormat::S24Packed if fits(&support.l24) => Ok(Container::L24),
        other => Err(format!(
            "the renderer does not take {} at {} Hz in {} channels",
            other.label(),
            format.sample_rate,
            format.channels
        )),
    }
}

/* ------------------------------------------------------------------ */
/* SOAP                                                                */
/* ------------------------------------------------------------------ */

/// Calls `action` and returns the response body, or the UPnP error.
pub fn soap_call(
    service: &Service,
    action: &str,
    args: &[(&str, String)],
) -> Result<String, String> {
    let mut body = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{}\">",
        service.service_type
    );
    for (name, value) in args {
        body.push_str(&format!("<{name}>{}</{name}>", xml::escape(value)));
    }
    body.push_str(&format!("</u:{action}></s:Body></s:Envelope>"));
    let soap_action = format!("\"{}#{action}\"", service.service_type);
    let response = http::request(
        "POST",
        &service.control_url,
        &[
            ("Content-Type", "text/xml; charset=\"utf-8\""),
            ("SOAPACTION", &soap_action),
        ],
        body.as_bytes(),
        SOAP_TIMEOUT,
        MAX_DESCRIPTION,
    )
    .map_err(|error| format!("{action}: {error}"))?;
    let text = response.text();
    if response.status == 200 {
        return Ok(text);
    }
    let code = xml::text(&text, "errorCode");
    let description = xml::text(&text, "errorDescription");
    Err(match (code, description) {
        (Some(code), Some(description)) => {
            format!("{action} failed: UPnP error {code} ({description})")
        }
        (Some(code), None) => format!("{action} failed: UPnP error {code}"),
        _ => format!("{action} failed: HTTP {}", response.status),
    })
}

/// DIDL-Lite for the stream, so the renderer can show the track.
pub fn didl(track: Option<&TrackInfo>, url: &str, mime: &str, format: &StreamFormat) -> String {
    let title = track.map_or("Parson", |track| track.title.as_str());
    let mut item = format!(
        "<DIDL-Lite xmlns=\"urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:upnp=\"urn:schemas-upnp-org:metadata-1-0/upnp/\"><item id=\"parson-stream\" parentID=\"0\" restricted=\"1\"><dc:title>{}</dc:title>",
        xml::escape(title)
    );
    if let Some(artist) = track.and_then(|track| track.artist.as_deref()) {
        item.push_str(&format!(
            "<dc:creator>{0}</dc:creator><upnp:artist>{0}</upnp:artist>",
            xml::escape(artist)
        ));
    }
    if let Some(album) = track.and_then(|track| track.album.as_deref()) {
        item.push_str(&format!("<upnp:album>{}</upnp:album>", xml::escape(album)));
    }
    item.push_str("<upnp:class>object.item.audioItem.musicTrack</upnp:class>");
    let duration = track
        .and_then(|track| track.duration_ms)
        .map(|ms| {
            format!(
                " duration=\"{}:{:02}:{:02}.{:03}\"",
                ms / 3_600_000,
                ms / 60_000 % 60,
                ms / 1000 % 60,
                ms % 1000
            )
        })
        .unwrap_or_default();
    item.push_str(&format!(
        "<res protocolInfo=\"http-get:*:{}:DLNA.ORG_OP=00;DLNA.ORG_CI=0;DLNA.ORG_FLAGS=01700000000000000000000000000000\" sampleFrequency=\"{}\" bitsPerSample=\"{}\" nrAudioChannels=\"{}\"{duration}>{}</res></item></DIDL-Lite>",
        xml::escape(mime),
        format.sample_rate,
        format.sample_format.bits(),
        format.channels,
        xml::escape(url)
    ));
    item
}

/// What an OpenHome renderer says it is decoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodingDetails {
    pub codec: String,
    pub sample_rate: u32,
    pub bit_depth: u8,
    pub lossless: bool,
}

pub fn info_details(service: &Service) -> Result<DecodingDetails, String> {
    let body = soap_call(service, "Details", &[])?;
    let number = |name: &str| xml::text(&body, name).and_then(|value| value.parse::<u32>().ok());
    Ok(DecodingDetails {
        codec: xml::text(&body, "CodecName").unwrap_or_else(|| "PCM".into()),
        sample_rate: number("SampleRate").ok_or("Info.Details has no SampleRate")?,
        bit_depth: number("BitDepth").unwrap_or(0).min(64) as u8,
        lossless: xml::text(&body, "Lossless")
            .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true")),
    })
}

/* ------------------------------------------------------------------ */
/* Backend                                                             */
/* ------------------------------------------------------------------ */

/// A probed renderer.
#[derive(Clone, Debug)]
pub struct Renderer {
    pub description: Description,
    pub support: StreamSupport,
    pub address: SocketAddr,
}

pub fn device_id(description: &Description) -> String {
    format!(
        "upnp:{}",
        description
            .udn
            .trim()
            .trim_start_matches("uuid:")
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            .take(96)
            .collect::<String>()
    )
}

fn kind_for(description: &Description) -> EndpointKind {
    let text = format!(
        "{} {} {}",
        description.model_name.as_deref().unwrap_or(""),
        description.model_description.as_deref().unwrap_or(""),
        description.friendly_name
    )
    .to_ascii_lowercase();
    if text.contains("tv") || text.contains("television") {
        EndpointKind::Television
    } else if text.contains("receiver") || text.contains("avr") || text.contains("amplifier") {
        EndpointKind::Receiver
    } else if text.contains("speaker") || text.contains("sonos") || text.contains("play:") {
        EndpointKind::Speaker
    } else {
        EndpointKind::Streamer
    }
}

/// Describes one renderer as a device.
pub fn device_info(renderer: &Renderer, manual: bool, icon_url: Option<String>) -> DeviceInfo {
    let description = &renderer.description;
    let (caps, caps_source, error) = caps_for(description, &renderer.support);
    let protocol = description.protocol();
    let model = match (&description.model_name, &description.model_number) {
        (Some(name), Some(number)) if !name.contains(number.as_str()) => {
            Some(format!("{name} {number}"))
        }
        (Some(name), _) => Some(name.clone()),
        (None, number) => number.clone(),
    };
    let stream_format = if renderer.support.wav {
        Container::Wav.label()
    } else if !renderer.support.l24.is_empty() {
        "LPCM (audio/L16 or L24) over HTTP"
    } else {
        Container::L16.label()
    };
    let mut detail = vec![protocol.label().to_string()];
    if let Some(manufacturer) = &description.manufacturer {
        detail.push(manufacturer.clone());
    }
    detail.push(renderer.address.ip().to_string());
    DeviceInfo {
        id: device_id(description),
        name: description.friendly_name.clone(),
        detail: detail.join(" · "),
        driver: Driver::Upnp,
        exclusive_capable: true,
        caps,
        probe_error: error,
        is_default: false,
        hardware: None,
        address: Some(renderer.address.to_string()),
        network: Some(NetworkInfo {
            protocol,
            host: renderer.address.ip().to_string(),
            port: renderer.address.port(),
            location: Some(description.location.clone()),
            manufacturer: description.manufacturer.clone(),
            model,
            model_description: description.model_description.clone(),
            icon_url: icon_url.or_else(|| description.best_icon().map(|icon| icon.url.clone())),
            kind: kind_for(description),
            caps_source,
            stream_format: stream_format.to_string(),
            accepts: renderer.support.labels(),
            reports_format: description.oh_info().is_some(),
            manual,
            verification: UPNP_VERIFICATION.to_string(),
        }),
    }
}

/// Fetches a renderer's description and protocol info.
pub fn probe(location: &str) -> Result<Renderer, String> {
    let description = fetch_description(location)?;
    if !description.is_renderer() {
        return Err(format!(
            "{} is not a media renderer",
            description.friendly_name
        ));
    }
    let support = match description.connection_manager() {
        Some(manager) => {
            let body = soap_call(manager, "GetProtocolInfo", &[])?;
            parse_protocol_info(&xml::text(&body, "Sink").unwrap_or_default())
        }
        // OpenHome renderers without a ConnectionManager take WAV (the
        // OpenHome pipeline's own PCM container).
        None => StreamSupport {
            wav: true,
            ..StreamSupport::default()
        },
    };
    let url = http::parse_url(location)?;
    let address = url
        .socket_addr()
        .map_err(|error| format!("{location}: {error}"))?;
    Ok(Renderer {
        description,
        support,
        address,
    })
}

#[derive(Default)]
struct RendererCache {
    renderers: HashMap<String, Arc<Renderer>>,
    tracks: HashMap<String, TrackInfo>,
    details: HashMap<String, (Instant, Option<DecodingDetails>)>,
}

/// Plays to UPnP AV and OpenHome renderers.
pub struct UpnpBackend {
    registry: Arc<NetworkRegistry>,
    /// Where `M-SEARCH` goes; `None` turns discovery off (only renderers
    /// added by address are listed).
    search: Option<SocketAddr>,
    search_wait: Duration,
    cache: Mutex<RendererCache>,
}

impl UpnpBackend {
    pub fn new(registry: Arc<NetworkRegistry>, search: Option<SocketAddr>) -> Self {
        Self {
            registry,
            search,
            search_wait: Duration::from_millis(1500),
            cache: Mutex::new(RendererCache::default()),
        }
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, RendererCache> {
        self.cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn renderer(&self, device_id: &str) -> io::Result<Arc<Renderer>> {
        self.cache()
            .renderers
            .get(device_id)
            .cloned()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "that renderer is no longer on the network",
                )
            })
    }
}

/// A renderer added by address that could not be reached.
fn unreachable_device(endpoint: &ManualEndpoint, error: String) -> DeviceInfo {
    let (host, port) = endpoint.host_port(1900).unwrap_or_default();
    DeviceInfo {
        id: format!("upnp-manual:{}", endpoint.address.trim()),
        name: endpoint
            .name
            .clone()
            .unwrap_or_else(|| format!("Renderer at {}", endpoint.address.trim())),
        detail: format!("UPnP AV · {}", endpoint.address.trim()),
        driver: Driver::Upnp,
        exclusive_capable: true,
        caps: None,
        probe_error: Some(error),
        is_default: false,
        hardware: None,
        address: Some(endpoint.address.trim().to_string()),
        network: Some(NetworkInfo {
            protocol: NetworkProtocol::UpnpAv,
            host,
            port,
            location: None,
            manufacturer: None,
            model: None,
            model_description: None,
            icon_url: None,
            kind: EndpointKind::Streamer,
            caps_source: NetworkCapsSource::Assumed,
            stream_format: Container::Wav.label().into(),
            accepts: Vec::new(),
            reports_format: false,
            manual: true,
            verification: UPNP_VERIFICATION.to_string(),
        }),
    }
}

impl Backend for UpnpBackend {
    fn driver(&self) -> Driver {
        Driver::Upnp
    }

    fn devices(&self) -> Vec<DeviceInfo> {
        // (location, manual endpoint that led to it)
        let mut locations: Vec<(String, Option<ManualEndpoint>)> = Vec::new();
        let mut failed = Vec::new();
        let manual = self
            .registry
            .manual()
            .into_iter()
            .filter(|endpoint| endpoint.protocol != NetworkProtocol::AirPlay)
            .collect::<Vec<_>>();
        std::thread::scope(|scope| {
            let multicast = self
                .search
                .map(|target| scope.spawn(move || ssdp_search(target, self.search_wait)));
            let unicast: Vec<_> = manual
                .iter()
                .map(|endpoint| {
                    let endpoint = endpoint.clone();
                    let wait = self.search_wait;
                    scope.spawn(move || {
                        if endpoint.address.contains("://") {
                            return (
                                endpoint.clone(),
                                Ok(vec![endpoint.address.trim().to_string()]),
                            );
                        }
                        let Some((host, port)) = endpoint.host_port(1900) else {
                            return (endpoint, Err("not a valid address".to_string()));
                        };
                        let target =
                            std::net::ToSocketAddrs::to_socket_addrs(&(host.as_str(), port))
                                .ok()
                                .and_then(|mut addresses| addresses.next());
                        let Some(target) = target else {
                            return (endpoint, Err(format!("{host} does not resolve")));
                        };
                        let found: Vec<String> = ssdp_search(target, wait)
                            .into_iter()
                            .map(|response| response.location)
                            .collect();
                        if found.is_empty() {
                            (
                                endpoint,
                                Err(format!(
                                    "no UPnP renderer answered a search at {host}:{port}"
                                )),
                            )
                        } else {
                            (endpoint, Ok(found))
                        }
                    })
                })
                .collect();
            if let Some(multicast) = multicast {
                for response in multicast.join().unwrap_or_default() {
                    locations.push((response.location, None));
                }
            }
            for handle in unicast {
                if let Ok((endpoint, result)) = handle.join() {
                    match result {
                        Ok(found) => locations.extend(
                            found
                                .into_iter()
                                .map(|location| (location, Some(endpoint.clone()))),
                        ),
                        Err(error) => failed.push((endpoint, error)),
                    }
                }
            }
        });
        locations.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.is_some().cmp(&a.1.is_some())));
        locations.dedup_by(|a, b| a.0 == b.0);
        let probed: Vec<(Result<Renderer, String>, Option<ManualEndpoint>)> =
            std::thread::scope(|scope| {
                let handles: Vec<_> = locations
                    .iter()
                    .map(|(location, endpoint)| {
                        let location = location.clone();
                        let endpoint = endpoint.clone();
                        scope.spawn(move || (probe(&location), endpoint))
                    })
                    .collect();
                handles
                    .into_iter()
                    .filter_map(|handle| handle.join().ok())
                    .collect()
            });
        let mut devices = Vec::new();
        let mut renderers = HashMap::new();
        for (result, endpoint) in probed {
            match result {
                Ok(renderer) => {
                    let device = device_info(&renderer, endpoint.is_some(), None);
                    if renderers.contains_key(&device.id) {
                        continue;
                    }
                    renderers.insert(device.id.clone(), Arc::new(renderer));
                    devices.push(device);
                }
                Err(error) => {
                    if let Some(endpoint) = endpoint {
                        failed.push((endpoint, error));
                    }
                }
            }
        }
        for (endpoint, error) in failed {
            devices.push(unreachable_device(
                &endpoint,
                format!("Parson could not reach this renderer: {error}."),
            ));
        }
        self.cache().renderers = renderers;
        devices.sort_by_key(|device| device.name.to_lowercase());
        devices
    }

    fn prepare(&self, device_id: &str, track: &TrackInfo) {
        self.cache()
            .tracks
            .insert(device_id.to_string(), track.clone());
    }

    fn open(
        &self,
        device_id: &str,
        format: StreamFormat,
        _exclusive: bool,
    ) -> io::Result<Box<dyn Sink>> {
        let renderer = self.renderer(device_id)?;
        let container = choose_container(&renderer.support, &format)
            .map_err(|error| io::Error::new(io::ErrorKind::Unsupported, error))?;
        // Reach the renderer before claiming the device is open, so a
        // renderer that went away fails here rather than mid-stream.
        let reachable = match (
            renderer.description.av_transport(),
            renderer.description.oh_playlist(),
        ) {
            (Some(transport), _) => {
                soap_call(transport, "GetTransportInfo", &[("InstanceID", "0".into())]).map(|_| ())
            }
            (None, Some(playlist)) => soap_call(playlist, "TransportState", &[]).map(|_| ()),
            (None, None) => Err("the renderer has no transport service".into()),
        };
        reachable.map_err(|error| {
            io::Error::new(
                io::ErrorKind::ConnectionRefused,
                format!(
                    "{} did not answer: {error}",
                    renderer.description.friendly_name
                ),
            )
        })?;
        let local_ip = local_ip_for(renderer.address)?;
        let track = self.cache().tracks.get(device_id).cloned();
        let mut sink = UpnpSink {
            renderer,
            format,
            container,
            local_ip,
            track,
            session: None,
            scratch: Vec::new(),
        };
        sink.start()?;
        Ok(Box::new(sink))
    }

    fn hardware_status(&self, device_id: &str, stream: &StreamFormat) -> Option<HardwareStatus> {
        let renderer = self.renderer(device_id).ok()?;
        let info = renderer.description.oh_info()?.clone();
        let cached = {
            let cache = self.cache();
            cache
                .details
                .get(device_id)
                .filter(|(at, _)| at.elapsed() < Duration::from_secs(2))
                .map(|(_, details)| details.clone())
        };
        let details = match cached {
            Some(details) => details,
            None => {
                let details = info_details(&info).ok();
                self.cache()
                    .details
                    .insert(device_id.to_string(), (Instant::now(), details.clone()));
                details
            }
        }?;
        if details.sample_rate == 0 {
            return None;
        }
        Some(renderer_status(&details, &renderer, stream))
    }
}

/// The renderer's report as a hardware status.
pub fn renderer_status(
    details: &DecodingDetails,
    renderer: &Renderer,
    stream: &StreamFormat,
) -> HardwareStatus {
    let bits = stream.sample_format.bits();
    HardwareStatus {
        source: format!("OpenHome Info.Details from {}", renderer.address.ip()),
        reporter: HardwareReporter::Renderer,
        chain: None,
        params: HwParams {
            access: None,
            format: format!(
                "{} {}bit{}",
                details.codec,
                details.bit_depth,
                if details.lossless { ", lossless" } else { "" }
            ),
            channels: stream.channels,
            rate: details.sample_rate,
            period_size: None,
            buffer_size: None,
        },
        momentary_hz: None,
        matches_stream: details.sample_rate == stream.sample_rate
            && details.bit_depth == bits
            && details.lossless,
        kernel_emulator: false,
    }
}

struct UpnpSession {
    stream: Arc<PcmStream>,
    written_frames: u64,
}

/// A stream to one renderer. Created already playing; a flush (pause or
/// seek) stops the renderer, and the next write starts a fresh stream.
pub struct UpnpSink {
    renderer: Arc<Renderer>,
    format: StreamFormat,
    container: Container,
    local_ip: IpAddr,
    track: Option<TrackInfo>,
    session: Option<UpnpSession>,
    scratch: Vec<u8>,
}

impl UpnpSink {
    fn frame_bytes(&self) -> usize {
        self.format.frame_bytes()
    }

    fn start(&mut self) -> io::Result<()> {
        let byte_rate = u64::from(self.format.sample_rate) * self.frame_bytes() as u64;
        let extension = if self.container == Container::Wav {
            "wav"
        } else {
            "pcm"
        };
        let path = format!("/parson/{}.{extension}", random_hex(16));
        let mime = self.container.mime(&self.format);
        let header = if self.container == Container::Wav {
            wav_header(
                self.format.sample_rate,
                u16::from(self.format.sample_format.bits()),
                u16::from(self.format.channels),
            )
        } else {
            Vec::new()
        };
        let stream = Arc::new(PcmStream::start(
            self.local_ip,
            &path,
            &mime,
            header,
            self.frame_bytes(),
            // About two seconds ahead of the renderer.
            (byte_rate * 2) as usize,
            byte_rate,
        )?);
        let metadata = didl(self.track.as_ref(), stream.url(), &mime, &self.format);
        let url = stream.url().to_string();
        let renderer = Arc::clone(&self.renderer);
        let control_stream = Arc::clone(&stream);
        // Tell the renderer to play in the background: many fetch the URL
        // while handling the call, and the stream must already be filling.
        std::thread::Builder::new()
            .name("parson-upnp-control".into())
            .spawn(move || {
                if let Err(error) = start_renderer(&renderer, &url, &metadata) {
                    control_stream
                        .abort(format!("{}: {error}", renderer.description.friendly_name));
                }
            })?;
        self.session = Some(UpnpSession {
            stream,
            written_frames: 0,
        });
        Ok(())
    }

    fn stop_renderer(&self) {
        let _ = stop_renderer(&self.renderer);
    }
}

/// `SetAVTransportURI` + `Play`, or the OpenHome playlist equivalent.
pub fn start_renderer(renderer: &Renderer, url: &str, metadata: &str) -> Result<(), String> {
    if let Some(transport) = renderer.description.av_transport() {
        let instance = || ("InstanceID", "0".to_string());
        let _ = soap_call(transport, "Stop", &[instance()]);
        soap_call(
            transport,
            "SetAVTransportURI",
            &[
                instance(),
                ("CurrentURI", url.to_string()),
                ("CurrentURIMetaData", metadata.to_string()),
            ],
        )?;
        soap_call(transport, "Play", &[instance(), ("Speed", "1".into())])?;
        return Ok(());
    }
    let playlist = renderer
        .description
        .oh_playlist()
        .ok_or("the renderer has no transport service")?;
    let _ = soap_call(playlist, "Stop", &[]);
    soap_call(playlist, "DeleteAll", &[])?;
    let inserted = soap_call(
        playlist,
        "Insert",
        &[
            ("AfterId", "0".into()),
            ("Uri", url.to_string()),
            ("Metadata", metadata.to_string()),
        ],
    )?;
    let id = xml::text(&inserted, "NewId").ok_or("Playlist.Insert returned no id")?;
    soap_call(playlist, "SeekId", &[("Value", id)])?;
    soap_call(playlist, "Play", &[])?;
    Ok(())
}

pub fn stop_renderer(renderer: &Renderer) -> Result<(), String> {
    match (
        renderer.description.av_transport(),
        renderer.description.oh_playlist(),
    ) {
        (Some(transport), _) => {
            soap_call(transport, "Stop", &[("InstanceID", "0".into())]).map(|_| ())
        }
        (None, Some(playlist)) => soap_call(playlist, "Stop", &[]).map(|_| ()),
        (None, None) => Ok(()),
    }
}

impl Sink for UpnpSink {
    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        if self.session.is_none() {
            self.start()?;
        }
        let frame = self.frame_bytes();
        let payload: &[u8] = match self.container {
            Container::Wav => data,
            Container::L16 | Container::L24 => {
                to_big_endian(data, self.format.sample_format.bytes(), &mut self.scratch);
                &self.scratch
            }
        };
        let session = self.session.as_mut().expect("started");
        session.stream.push(payload)?;
        session.written_frames += (data.len() / frame) as u64;
        Ok(())
    }

    fn delay_frames(&mut self) -> u64 {
        let frame = self.frame_bytes() as u64;
        self.session.as_ref().map_or(0, |session| {
            session
                .written_frames
                .saturating_sub(session.stream.played_bytes() / frame)
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(session) = self.session.take() {
            self.stop_renderer();
            session.stream.cancel();
        }
        Ok(())
    }

    fn drain(&mut self) -> io::Result<()> {
        if let Some(session) = self.session.take() {
            let seconds = session.written_frames / u64::from(self.format.sample_rate.max(1));
            session.stream.finish(Duration::from_secs(seconds + 15));
            session.stream.cancel();
        }
        Ok(())
    }
}

impl Drop for UpnpSink {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            self.stop_renderer();
            session.stream.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESCRIPTION: &str = r#"<?xml version="1.0"?>
<root xmlns="urn:schemas-upnp-org:device-1-0"><specVersion><major>1</major></specVersion>
<device><deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType>
<friendlyName>Living Room Streamer</friendlyName><manufacturer>Linn</manufacturer>
<modelName>Klimax DSM</modelName><modelNumber>3</modelNumber><UDN>uuid:4c494e4e-0026-0f21-cafe-01303307013f</UDN>
<iconList><icon><mimetype>image/jpeg</mimetype><width>48</width><height>48</height><url>/icon48.jpg</url></icon>
<icon><mimetype>image/png</mimetype><width>240</width><height>240</height><url>icons/240.png</url></icon></iconList>
<serviceList>
<service><serviceType>urn:schemas-upnp-org:service:ConnectionManager:1</serviceType><controlURL>/cm/control</controlURL></service>
<service><serviceType>urn:schemas-upnp-org:service:AVTransport:1</serviceType><controlURL>/avt/control</controlURL></service>
<service><serviceType>urn:av-openhome-org:service:Product:1</serviceType><controlURL>/oh/product</controlURL></service>
<service><serviceType>urn:av-openhome-org:service:Info:1</serviceType><controlURL>/oh/info</controlURL></service>
</serviceList></device></root>"#;

    #[test]
    fn parses_a_description_and_finds_services() {
        let description =
            parse_description(DESCRIPTION, "http://10.0.0.40:55178/desc/device.xml").unwrap();
        assert_eq!(description.friendly_name, "Living Room Streamer");
        assert_eq!(description.manufacturer.as_deref(), Some("Linn"));
        assert_eq!(
            description.av_transport().unwrap().control_url,
            "http://10.0.0.40:55178/avt/control"
        );
        assert_eq!(description.protocol(), NetworkProtocol::OpenHome);
        assert!(description.oh_info().is_some());
        assert_eq!(
            description.best_icon().unwrap().url,
            "http://10.0.0.40:55178/desc/icons/240.png"
        );
        assert_eq!(
            device_id(&description),
            "upnp:4c494e4e-0026-0f21-cafe-01303307013f"
        );
    }

    #[test]
    fn protocol_info_decides_formats_and_their_source() {
        let support = parse_protocol_info(
            "http-get:*:audio/L16;rate=44100;channels=2:DLNA.ORG_PN=LPCM,http-get:*:audio/L16;rate=48000;channels=2:*,http-get:*:audio/mpeg:*,http-get:*:audio/x-flac:*,rtsp-rtp-udp:*:audio/L16:*",
        );
        assert!(!support.wav);
        assert_eq!(support.l16, vec![(44_100, 2), (48_000, 2)]);
        assert_eq!(support.labels(), vec!["L16 44.1/48kHz", "MP3", "FLAC"]);
        let description = parse_description(DESCRIPTION, "http://10.0.0.40/d.xml").unwrap();
        let (caps, source, error) = caps_for(&description, &support);
        let caps = caps.unwrap();
        assert!(error.is_none());
        assert_eq!(source, NetworkCapsSource::Declared);
        assert_eq!(caps.sample_rates, vec![44_100, 48_000]);
        assert_eq!(caps.sample_formats, vec![SampleFormat::S16]);

        let wav = parse_protocol_info("http-get:*:audio/wav:*,http-get:*:audio/flac:*");
        let (caps, source, _) = caps_for(&description, &wav);
        assert_eq!(source, NetworkCapsSource::Assumed);
        let caps = caps.unwrap();
        assert_eq!(caps.sample_rates.last(), Some(&192_000));
        assert_eq!(caps.bit_depths(), vec![16, 24]);

        let mp3_only = parse_protocol_info("http-get:*:audio/mpeg:*");
        let (caps, _, error) = caps_for(&description, &mp3_only);
        assert!(caps.is_none());
        assert!(error.unwrap().contains("MP3"));

        let sonos = Description {
            manufacturer: Some("Sonos, Inc.".into()),
            ..description
        };
        let (caps, source, _) = caps_for(&sonos, &wav);
        assert_eq!(source, NetworkCapsSource::KnownModel);
        assert_eq!(caps.unwrap().sample_rates, vec![44_100, 48_000]);
    }

    #[test]
    fn containers_follow_what_the_renderer_takes() {
        let l16 = parse_protocol_info("http-get:*:audio/L16;rate=44100;channels=2:*");
        let format = StreamFormat {
            sample_rate: 44_100,
            sample_format: SampleFormat::S16,
            channels: 2,
        };
        assert_eq!(choose_container(&l16, &format), Ok(Container::L16));
        assert_eq!(
            Container::L16.mime(&format),
            "audio/L16;rate=44100;channels=2"
        );
        assert!(
            choose_container(
                &l16,
                &StreamFormat {
                    sample_rate: 96_000,
                    ..format
                }
            )
            .is_err()
        );
        let wav = parse_protocol_info("http-get:*:audio/x-wav:*");
        assert_eq!(
            choose_container(
                &wav,
                &StreamFormat {
                    sample_format: SampleFormat::S24Packed,
                    ..format
                }
            ),
            Ok(Container::Wav)
        );
    }

    #[test]
    fn ssdp_responses_parse() {
        let response = parse_ssdp("HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=1800\r\nLocation: http://10.0.0.40:55178/d.xml\r\nST: urn:schemas-upnp-org:device:MediaRenderer:1\r\nUSN: uuid:x::urn\r\nSERVER: Linux UPnP/1.0 Linn/4\r\n\r\n").unwrap();
        assert_eq!(response.location, "http://10.0.0.40:55178/d.xml");
        assert_eq!(response.server.as_deref(), Some("Linux UPnP/1.0 Linn/4"));
        assert!(parse_ssdp("M-SEARCH * HTTP/1.1\r\n\r\n").is_none());
    }

    #[test]
    fn didl_escapes_and_describes_the_stream() {
        let format = StreamFormat {
            sample_rate: 96_000,
            sample_format: SampleFormat::S24Packed,
            channels: 2,
        };
        let track = TrackInfo {
            title: "Tom & Jerry".into(),
            artist: Some("<A>".into()),
            album: None,
            duration_ms: Some(185_250),
        };
        let didl = didl(
            Some(&track),
            "http://10.0.0.2:4000/parson/x.wav",
            "audio/wav",
            &format,
        );
        assert!(didl.contains("<dc:title>Tom &amp; Jerry</dc:title>"));
        assert!(didl.contains("&lt;A&gt;"));
        assert!(didl.contains("sampleFrequency=\"96000\" bitsPerSample=\"24\""));
        assert!(didl.contains("duration=\"0:03:05.250\""));
        assert_eq!(
            xml::attribute(&didl, "res", "nrAudioChannels").as_deref(),
            Some("2")
        );
    }
}
