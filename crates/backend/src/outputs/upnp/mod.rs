//! UPnP/DLNA media renderers (AV receivers, streamers, active speakers,
//! smart TVs) and OpenHome renderers (Linn and the streamers that speak its
//! protocol): SSDP discovery of `MediaRenderer:1` and OpenHome `Product`,
//! or a player added by address; playback of signed media URLs, gapless
//! with `SetNextAVTransportURI` or the OpenHome playlist; volume and mute
//! on the renderer's own scale (RenderingControl or OpenHome Volume); format
//! limits from `GetProtocolInfo`; what an OpenHome renderer says it decodes
//! (`Info.Details`). State is polled.

pub mod description;
#[cfg(test)]
pub mod fake;
pub mod protocol_info;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;

use parson_output::network::{
    EndpointKind, ManualEndpoint, NetworkCapsSource, NetworkInfo, NetworkProtocol, NetworkRegistry,
};

use self::description::{
    AV_TRANSPORT, Control, RENDERING_CONTROL, RendererDescription, ServiceUrls,
};
use self::protocol_info::SinkCapabilities;
use super::renderer::{
    PlayState, Renderer, RendererDecoding, RendererInfo, RendererSettings, RendererState,
    TrackRequest, from_device_scale, to_device_scale,
};
use crate::sonos::soap::{SoapClient, SoapError};
use crate::sonos::xml::{format_duration, parse_duration};
use crate::zones::{DeviceVolumeRange, VolumeMode};

const MAX_RENDERERS: usize = 32;
/// A renderer missing from discovery this long, and unreachable, is dropped.
const FORGET_AFTER: Duration = Duration::from_secs(10 * 60);
/// Path of signed media URLs for server-driven zones.
pub const RENDER_MEDIA_PATH: &str = "/api/v1/zones/render/";

/// What Parson has (and has not) tested UPnP and OpenHome against.
pub const UPNP_VERIFICATION: &str = "Built to the UPnP AV and OpenHome specifications and tested end to end against Mujin Music's protocol-level fake renderer (SSDP, device description, SOAP control, gapless queueing and the media fetch, with what it streams checked). Not yet tested with a real network streamer.";

/// How the controller finds renderers.
#[derive(Clone, Debug)]
pub struct UpnpConfig {
    pub ssdp_targets: Vec<SocketAddr>,
    /// Description URLs to probe directly (networks that drop multicast).
    pub static_locations: Vec<String>,
    pub search_timeout: Duration,
}

impl UpnpConfig {
    /// `PARSON_UPNP` (`off` disables renderers) and `PARSON_UPNP_RENDERERS`
    /// (comma separated description URLs); SSDP multicast unless
    /// `discovery` is off.
    pub fn from_env(discovery: bool) -> Option<Self> {
        let enabled = std::env::var("PARSON_UPNP")
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
        Some(Self {
            ssdp_targets: if discovery {
                vec![crate::sonos::ssdp::SSDP_MULTICAST]
            } else {
                Vec::new()
            },
            static_locations: std::env::var("PARSON_UPNP_RENDERERS")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|location| location.starts_with("http://"))
                .map(str::to_string)
                .collect(),
            search_timeout: Duration::from_millis(1_500),
        })
    }
}

/// Parson song id in a signed render URL.
pub fn song_from_uri(uri: &str) -> Option<String> {
    let start = uri.find(RENDER_MEDIA_PATH)? + RENDER_MEDIA_PATH.len();
    let rest = &uri[start..];
    let encoded = rest.split('/').next()?;
    let decoded = percent_encoding::percent_decode_str(encoded)
        .decode_utf8()
        .ok()?
        .to_string();
    (!decoded.is_empty()).then_some(decoded)
}

struct Inner {
    state: RendererState,
    current_uri: Option<String>,
    next_uri: Option<String>,
    next_serial: Option<u64>,
    /// The current track was seen playing, and where it last was.
    seen_playing: bool,
    last_position_ms: i64,
    pending_seek: Option<i64>,
    volume_mode: VolumeMode,
    failures: u32,
    seen_at: Instant,
    gapless: bool,
    /// OpenHome playlist ids of the current and the queued next track.
    playlist_current: Option<u32>,
    playlist_next: Option<u32>,
}

/// One UPnP/DLNA or OpenHome renderer.
pub struct UpnpRenderer {
    id: String,
    location: String,
    description: RendererDescription,
    control: Control,
    soap: SoapClient,
    caps: SinkCapabilities,
    volume_range: (f32, f32, f32),
    volume_db: Option<(f32, f32)>,
    /// Added by address rather than found by SSDP.
    manual: bool,
    inner: Mutex<Inner>,
}

fn device_error(error: SoapError) -> String {
    error.to_string()
}

impl UpnpRenderer {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn range(&self) -> DeviceVolumeRange {
        DeviceVolumeRange {
            min: self.volume_range.0,
            max: self.volume_range.1,
            step: self.volume_range.2,
            unit: "steps",
            hardware: true,
            current: self.lock().state.device_volume,
        }
    }

    async fn transport(
        &self,
        action: &str,
        arguments: &[(&str, &str)],
    ) -> Result<HashMap<String, String>, String> {
        let Some(service) = &self.description.av_transport else {
            return Err("renderer has no AVTransport".to_string());
        };
        let mut all = vec![("InstanceID", "0")];
        all.extend_from_slice(arguments);
        self.soap
            .call_at(&service.control_url, AV_TRANSPORT, action, &all)
            .await
            .map_err(device_error)
    }

    async fn rendering(
        &self,
        action: &str,
        arguments: &[(&str, &str)],
    ) -> Result<HashMap<String, String>, String> {
        let Some(service) = &self.description.rendering_control else {
            return Err("renderer has no volume control".to_string());
        };
        let mut all = vec![("InstanceID", "0"), ("Channel", "Master")];
        all.extend_from_slice(arguments);
        self.soap
            .call_at(&service.control_url, RENDERING_CONTROL, action, &all)
            .await
            .map_err(device_error)
    }

    /// An OpenHome action on `service` (its declared type names the call).
    async fn open_home(
        &self,
        service: Option<&ServiceUrls>,
        action: &str,
        arguments: &[(&str, &str)],
    ) -> Result<HashMap<String, String>, String> {
        let Some(service) = service else {
            return Err(format!("renderer has no OpenHome service for {action}"));
        };
        self.soap
            .call_at(
                &service.control_url,
                &service.service_type,
                action,
                arguments,
            )
            .await
            .map_err(device_error)
    }

    async fn playlist(
        &self,
        action: &str,
        arguments: &[(&str, &str)],
    ) -> Result<HashMap<String, String>, String> {
        self.open_home(self.description.oh_playlist.as_ref(), action, arguments)
            .await
    }

    fn has_volume(&self) -> bool {
        match self.control {
            Control::UpnpAv => self.description.rendering_control.is_some(),
            Control::OpenHome => self.description.oh_volume.is_some(),
        }
    }

    async fn seek_to(&self, position_ms: i64) -> Result<(), String> {
        match self.control {
            Control::UpnpAv => {
                let target = format_duration(position_ms);
                self.transport("Seek", &[("Unit", "REL_TIME"), ("Target", &target)])
                    .await
                    .map(|_| ())
            }
            Control::OpenHome => {
                let seconds = (position_ms.max(0) / 1_000).to_string();
                self.playlist("SeekSecondAbsolute", &[("Value", &seconds)])
                    .await
                    .map(|_| ())
            }
        }
    }

    async fn send_next(&self, next: Option<&TrackRequest>) -> Result<(), String> {
        if self.control == Control::OpenHome {
            return self.send_next_open_home(next).await;
        }
        if !self.lock().gapless {
            return Ok(());
        }
        let (uri, metadata) = next
            .map(|track| (track.uri.as_str(), track.metadata.as_str()))
            .unwrap_or(("", ""));
        match self
            .transport(
                "SetNextAVTransportURI",
                &[("NextURI", uri), ("NextURIMetaData", metadata)],
            )
            .await
        {
            Ok(_) => {
                let mut inner = self.lock();
                inner.next_uri = next.map(|track| track.uri.clone());
                inner.next_serial = next.map(|track| track.serial);
                Ok(())
            }
            Err(error) => {
                // Advertised but refused: fall back to starting each track.
                tracing::debug!(%error, renderer = self.id, "renderer refused SetNextAVTransportURI");
                let mut inner = self.lock();
                inner.gapless = false;
                inner.next_uri = None;
                inner.next_serial = None;
                Ok(())
            }
        }
    }

    /// The next track is the playlist entry after the current one.
    async fn send_next_open_home(&self, next: Option<&TrackRequest>) -> Result<(), String> {
        let (current, previous) = {
            let inner = self.lock();
            (inner.playlist_current, inner.playlist_next)
        };
        if let Some(previous) = previous {
            let _ = self
                .playlist("DeleteId", &[("Value", &previous.to_string())])
                .await;
        }
        let inserted = match (next, current) {
            (Some(track), Some(current)) => Some(
                self.playlist(
                    "Insert",
                    &[
                        ("AfterId", &current.to_string()),
                        ("Uri", &track.uri),
                        ("Metadata", &track.metadata),
                    ],
                )
                .await
                .and_then(|outputs| new_id(&outputs))?,
            ),
            _ => None,
        };
        let mut inner = self.lock();
        inner.playlist_next = inserted;
        inner.next_uri = inserted.and(next.map(|track| track.uri.clone()));
        inner.next_serial = inserted.and(next.map(|track| track.serial));
        Ok(())
    }

    /// The renderer's identity, as the zone's endpoint shows it.
    fn endpoint(&self) -> NetworkInfo {
        let address = self.origin_host();
        let manufacturer = Some(self.description.manufacturer.clone()).filter(|m| !m.is_empty());
        let text = format!(
            "{} {}",
            self.description.model_name,
            self.description.model_description.as_deref().unwrap_or("")
        )
        .to_ascii_lowercase();
        let kind = if text.contains("tv") || text.contains("television") {
            EndpointKind::Television
        } else if text.contains("receiver") || text.contains("avr") || text.contains("amplifier") {
            EndpointKind::Receiver
        } else if text.contains("speaker") {
            EndpointKind::Speaker
        } else {
            EndpointKind::Streamer
        };
        NetworkInfo {
            protocol: match self.control {
                Control::UpnpAv => NetworkProtocol::UpnpAv,
                Control::OpenHome => NetworkProtocol::OpenHome,
            },
            host: address
                .map(|address| address.ip().to_string())
                .unwrap_or_default(),
            port: address.map(|address| address.port()).unwrap_or(0),
            location: Some(self.location.clone()),
            manufacturer,
            model: Some(self.description.model_name.clone()),
            model_description: self.description.model_description.clone(),
            icon_url: self.description.best_icon().map(|icon| icon.url.clone()),
            kind,
            caps_source: if self.caps.rate_reported {
                NetworkCapsSource::Declared
            } else {
                NetworkCapsSource::Assumed
            },
            stream_format: "the file, or FLAC Mujin Music makes from it, over HTTP".to_string(),
            accepts: self
                .caps
                .limits
                .formats()
                .into_iter()
                .map(str::to_string)
                .collect(),
            reports_format: self.description.oh_info.is_some(),
            manual: self.manual,
            verification: UPNP_VERIFICATION.to_string(),
        }
    }

    fn origin_host(&self) -> Option<SocketAddr> {
        let authority = self
            .location
            .strip_prefix("http://")?
            .split('/')
            .next()?
            .to_string();
        authority.to_socket_addrs().ok()?.next()
    }
}

impl Renderer for UpnpRenderer {
    fn info(&self) -> RendererInfo {
        let mut notes = Vec::new();
        if !self.caps.rate_reported && self.caps.limits.flac {
            notes.push(format!(
                "This renderer does not report its highest sample rate; Mujin Music assumes {}/24-bit. Set a lower maximum in Device setup if hi-res tracks stutter.",
                super::format_rate(self.caps.limits.max_sample_rate)
            ));
        }
        if let Some((minimum, maximum)) = self.volume_db {
            notes.push(format!(
                "Volume is the renderer's own amplifier ({minimum:.1} dB to {maximum:.1} dB), so the audio Mujin Music sends stays untouched."
            ));
        }
        let gapless = self.lock().gapless;
        if !gapless {
            notes.push(
                "This renderer cannot queue the next track, so there may be a short gap between tracks."
                    .to_string(),
            );
        }
        if self.control == Control::OpenHome {
            notes.push(
                "Played through the renderer's OpenHome playlist: Mujin Music queues the next track in it for gapless playback."
                    .to_string(),
            );
        }
        let volume = self.has_volume();
        RendererInfo {
            id: self.id.clone(),
            kind: crate::zones::ZoneKind::Upnp,
            name: self.description.friendly_name.clone(),
            device: self.description.model(),
            limits: self.caps.limits,
            volume_range: volume.then(|| self.range()),
            volume_modes: if volume {
                vec![VolumeMode::Hardware, VolumeMode::Fixed]
            } else {
                vec![VolumeMode::Fixed]
            },
            gapless,
            exclusive_capable: false,
            network: true,
            endpoint: Some(self.endpoint()),
            notes,
        }
    }

    fn state(&self) -> RendererState {
        self.lock().state.clone()
    }

    fn endpoints(
        &self,
        _output: &parson_output::OutputSettings,
    ) -> Vec<crate::zones::ZoneEndpoint> {
        let network = self.endpoint();
        let mut endpoint = crate::zones::ZoneEndpoint::named(
            &self.id,
            &self.description.friendly_name,
            &self.description.model(),
        );
        endpoint.driver = Some(network.protocol.label().to_string());
        endpoint.address = Some(format!("{}:{}", network.host, network.port));
        endpoint.has_icon = network.icon_url.is_some();
        endpoint.network = Some(network);
        endpoint.dsp_support = Some(crate::zones::served_dsp_support());
        vec![endpoint]
    }

    fn configure(&self, settings: RendererSettings) {
        self.lock().volume_mode = settings.volume_mode;
    }

    fn media_origin(&self, port: u16) -> Option<String> {
        let local = crate::sonos::driver::local_address_towards(self.origin_host()?)?;
        Some(match local {
            IpAddr::V4(address) => format!("http://{address}:{port}"),
            IpAddr::V6(address) => format!("http://[{address}]:{port}"),
        })
    }

    fn load(
        &self,
        track: TrackRequest,
        next: Option<TrackRequest>,
        position_ms: i64,
        play: bool,
    ) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let playlist_id = match self.control {
                Control::UpnpAv => {
                    // Some renderers refuse a new URI while playing.
                    let _ = self.transport("Stop", &[]).await;
                    self.transport(
                        "SetAVTransportURI",
                        &[
                            ("CurrentURI", &track.uri),
                            ("CurrentURIMetaData", &track.metadata),
                        ],
                    )
                    .await?;
                    None
                }
                Control::OpenHome => {
                    let _ = self.playlist("Stop", &[]).await;
                    self.playlist("DeleteAll", &[]).await?;
                    let id = self
                        .playlist(
                            "Insert",
                            &[
                                ("AfterId", "0"),
                                ("Uri", &track.uri),
                                ("Metadata", &track.metadata),
                            ],
                        )
                        .await
                        .and_then(|outputs| new_id(&outputs))?;
                    Some(id)
                }
            };
            {
                let mut inner = self.lock();
                inner.current_uri = Some(track.uri.clone());
                inner.next_uri = None;
                inner.next_serial = None;
                inner.playlist_current = playlist_id;
                inner.playlist_next = None;
                inner.seen_playing = false;
                inner.last_position_ms = position_ms;
                inner.pending_seek = (position_ms > 0).then_some(position_ms);
                inner.state.song_id = Some(track.song_id.clone());
                inner.state.serial = Some(track.serial);
                inner.state.finished_serial = None;
                inner.state.foreign = None;
                inner.state.decoding = None;
                inner.state.position_ms = position_ms;
                inner.state.duration_ms = track.duration_ms;
                inner.state.polled_at_ms = crate::zones::service::now_ms();
                inner.state.state = if play {
                    PlayState::Transitioning
                } else {
                    PlayState::Stopped
                };
            }
            self.send_next(next.as_ref()).await?;
            if let Some(id) = playlist_id {
                // Selects the track; Play (or Stop) decides what happens next.
                self.playlist("SeekId", &[("Value", &id.to_string())])
                    .await?;
                if !play {
                    let _ = self.playlist("Stop", &[]).await;
                }
            }
            if play {
                self.play().await?;
            }
            Ok(())
        })
    }

    fn set_next(&self, next: Option<TrackRequest>) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move { self.send_next(next.as_ref()).await })
    }

    fn play(&self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            match self.control {
                Control::UpnpAv => self.transport("Play", &[("Speed", "1")]).await?,
                Control::OpenHome => self.playlist("Play", &[]).await?,
            };
            let pending = self.lock().pending_seek.take();
            if let Some(position_ms) = pending
                && let Err(error) = self.seek_to(position_ms).await
            {
                // Some renderers only seek once the stream is open.
                tracing::debug!(%error, "renderer refused the first seek; retrying once playing");
                self.lock().pending_seek = Some(position_ms);
            }
            self.lock().state.state = PlayState::Transitioning;
            Ok(())
        })
    }

    fn pause(&self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            match self.control {
                Control::UpnpAv => self.transport("Pause", &[]).await?,
                Control::OpenHome => self.playlist("Pause", &[]).await?,
            };
            let mut inner = self.lock();
            inner.state.position_ms = crate::zones::extrapolate_position(
                inner.state.position_ms,
                inner.state.polled_at_ms,
                crate::zones::service::now_ms(),
                inner.state.state == PlayState::Playing,
                inner.state.duration_ms,
            );
            inner.state.polled_at_ms = crate::zones::service::now_ms();
            inner.state.state = PlayState::Paused;
            Ok(())
        })
    }

    fn stop(&self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            match self.control {
                Control::UpnpAv => self.transport("Stop", &[]).await?,
                Control::OpenHome => self.playlist("Stop", &[]).await?,
            };
            let mut inner = self.lock();
            inner.state.state = PlayState::Stopped;
            inner.state.position_ms = 0;
            inner.state.decoding = None;
            Ok(())
        })
    }

    fn seek(&self, position_ms: i64) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.seek_to(position_ms).await?;
            let mut inner = self.lock();
            inner.state.position_ms = position_ms;
            inner.state.polled_at_ms = crate::zones::service::now_ms();
            Ok(())
        })
    }

    fn set_volume(&self, level: u8) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            if self.lock().volume_mode == VolumeMode::Fixed {
                return Err("fixed_volume".to_string());
            }
            let value = to_device_scale(level, &self.range()).round() as i64;
            match self.control {
                Control::UpnpAv => {
                    self.rendering("SetVolume", &[("DesiredVolume", &value.to_string())])
                        .await?
                }
                Control::OpenHome => {
                    self.open_home(
                        self.description.oh_volume.as_ref(),
                        "SetVolume",
                        &[("Value", &value.to_string())],
                    )
                    .await?
                }
            };
            let mut inner = self.lock();
            inner.state.volume = level;
            inner.state.device_volume = Some(value as f32);
            Ok(())
        })
    }

    fn set_mute(&self, muted: bool) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            match self.control {
                Control::UpnpAv => {
                    self.rendering("SetMute", &[("DesiredMute", if muted { "1" } else { "0" })])
                        .await?
                }
                Control::OpenHome => {
                    self.open_home(
                        self.description.oh_volume.as_ref(),
                        "SetMute",
                        &[("Value", if muted { "true" } else { "false" })],
                    )
                    .await?
                }
            };
            self.lock().state.muted = muted;
            Ok(())
        })
    }

    fn poll(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let reading = match self.control {
                Control::UpnpAv => self.read_av().await,
                Control::OpenHome => self.read_open_home().await,
            };
            let reading = match reading {
                Ok(reading) => reading,
                Err(error) => {
                    let mut inner = self.lock();
                    inner.failures += 1;
                    if inner.failures >= 3 {
                        inner.state.reachable = false;
                    }
                    tracing::debug!(%error, renderer = self.id, "renderer poll failed");
                    return;
                }
            };
            if let Some(position_ms) = self.apply(reading) {
                let _ = self.seek_to(position_ms).await;
            }
        })
    }
}

/// One poll of a renderer, whichever protocol it speaks.
struct Reading {
    state: PlayState,
    /// The track it is on: our song (by URL or playlist id), something
    /// else (with a title), or nothing.
    track: Track,
    position_ms: Option<i64>,
    duration_ms: Option<i64>,
    volume: Option<f32>,
    muted: Option<bool>,
    decoding: Option<RendererDecoding>,
}

enum Track {
    /// No track loaded.
    None,
    /// One of the URLs Parson gave it.
    Ours { uri: String, song_id: String },
    /// Another app's track.
    Foreign(String),
}

fn new_id(outputs: &HashMap<String, String>) -> Result<u32, String> {
    outputs
        .get("NewId")
        .and_then(|value| value.trim().parse().ok())
        .ok_or_else(|| "Playlist.Insert returned no id".to_string())
}

fn number<T: std::str::FromStr>(outputs: &HashMap<String, String>, name: &str) -> Option<T> {
    outputs
        .get(name)
        .and_then(|value| value.trim().parse().ok())
}

impl UpnpRenderer {
    async fn read_av(&self) -> Result<Reading, String> {
        let transport = self.transport("GetTransportInfo", &[]).await?;
        let position = self.transport("GetPositionInfo", &[]).await?;
        let (volume, muted) = if self.description.rendering_control.is_some() {
            let volume = self.rendering("GetVolume", &[]).await.ok();
            let mute = self.rendering("GetMute", &[]).await.ok();
            (
                volume.and_then(|volume| number::<f32>(&volume, "CurrentVolume")),
                mute.and_then(|mute| {
                    mute.get("CurrentMute")
                        .map(|value| matches!(value.trim(), "1" | "true" | "True"))
                }),
            )
        } else {
            (None, None)
        };
        let state = match transport
            .get("CurrentTransportState")
            .map(String::as_str)
            .unwrap_or_default()
        {
            "PLAYING" => PlayState::Playing,
            "PAUSED_PLAYBACK" | "PAUSED_RECORDING" => PlayState::Paused,
            "TRANSITIONING" => PlayState::Transitioning,
            _ => PlayState::Stopped,
        };
        let uri = position.get("TrackURI").cloned().unwrap_or_default();
        let track = if uri.is_empty() {
            Track::None
        } else {
            match song_from_uri(&uri) {
                Some(song_id) => Track::Ours { uri, song_id },
                None => Track::Foreign(
                    position
                        .get("TrackMetaData")
                        .and_then(|metadata| crate::sonos::didl::title(metadata))
                        .unwrap_or_else(|| "Playing from another app".to_string()),
                ),
            }
        };
        Ok(Reading {
            state,
            track,
            position_ms: position
                .get("RelTime")
                .and_then(|value| parse_duration(value)),
            duration_ms: position
                .get("TrackDuration")
                .and_then(|value| parse_duration(value)),
            volume,
            muted,
            decoding: None,
        })
    }

    async fn read_open_home(&self) -> Result<Reading, String> {
        let transport = self.playlist("TransportState", &[]).await?;
        let id: u32 = number(&self.playlist("Id", &[]).await?, "Value").unwrap_or(0);
        let state = match transport
            .get("Value")
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref()
        {
            Some("playing") => PlayState::Playing,
            Some("paused") => PlayState::Paused,
            Some("buffering") | Some("waiting") => PlayState::Transitioning,
            _ => PlayState::Stopped,
        };
        let (current_id, next_id, current_uri, next_uri) = {
            let inner = self.lock();
            (
                inner.playlist_current,
                inner.playlist_next,
                inner.current_uri.clone(),
                inner.next_uri.clone(),
            )
        };
        let track = if id == 0 {
            Track::None
        } else {
            let uri = if Some(id) == current_id {
                current_uri
            } else if Some(id) == next_id {
                next_uri
            } else {
                None
            };
            match uri.and_then(|uri| song_from_uri(&uri).map(|song_id| (uri, song_id))) {
                Some((uri, song_id)) => Track::Ours { uri, song_id },
                None => {
                    let title = match self.description.oh_info.as_ref() {
                        Some(info) => self
                            .open_home(Some(info), "Track", &[])
                            .await
                            .ok()
                            .and_then(|track| {
                                track
                                    .get("Metadata")
                                    .and_then(|metadata| crate::sonos::didl::title(metadata))
                            }),
                        None => None,
                    };
                    Track::Foreign(title.unwrap_or_else(|| "Playing from another app".to_string()))
                }
            }
        };
        let time = match self.description.oh_time.as_ref() {
            Some(service) => self.open_home(Some(service), "Time", &[]).await.ok(),
            None => None,
        };
        let (volume, muted) = match self.description.oh_volume.as_ref() {
            Some(service) => (
                self.open_home(Some(service), "Volume", &[])
                    .await
                    .ok()
                    .and_then(|volume| number::<f32>(&volume, "Value")),
                self.open_home(Some(service), "Mute", &[])
                    .await
                    .ok()
                    .and_then(|mute| {
                        mute.get("Value")
                            .map(|value| matches!(value.trim(), "1" | "true" | "True"))
                    }),
            ),
            None => (None, None),
        };
        let decoding = match self.description.oh_info.as_ref() {
            Some(service) if state == PlayState::Playing => self
                .open_home(Some(service), "Details", &[])
                .await
                .ok()
                .and_then(|details| {
                    Some(RendererDecoding {
                        codec: details
                            .get("CodecName")
                            .cloned()
                            .unwrap_or_else(|| "PCM".to_string()),
                        sample_rate: number(&details, "SampleRate")?,
                        bit_depth: number::<u32>(&details, "BitDepth").unwrap_or(0).min(64) as u8,
                        lossless: details
                            .get("Lossless")
                            .is_some_and(|value| matches!(value.trim(), "1" | "true" | "True")),
                    })
                }),
            _ => None,
        };
        Ok(Reading {
            state,
            track,
            position_ms: time
                .as_ref()
                .and_then(|time| number::<i64>(time, "Seconds"))
                .map(|seconds| seconds * 1_000),
            duration_ms: time
                .as_ref()
                .and_then(|time| number::<i64>(time, "Duration"))
                .map(|seconds| seconds * 1_000),
            volume,
            muted,
            decoding,
        })
    }

    /// Folds a poll into the state; returns a seek to retry now that the
    /// renderer plays.
    fn apply(&self, reading: Reading) -> Option<i64> {
        let mut inner = self.lock();
        inner.failures = 0;
        inner.state.reachable = true;
        let previous = inner.state.state;
        let state = reading.state;
        inner.state.state = state;
        match reading.track {
            Track::None => {}
            Track::Ours { uri, song_id } => {
                inner.state.song_id = Some(song_id);
                inner.state.foreign = None;
                if inner.next_uri.as_deref() == Some(uri.as_str()) {
                    // The renderer moved on to the queued next track.
                    inner.current_uri = Some(uri);
                    inner.next_uri = None;
                    inner.playlist_current = inner.playlist_next.take();
                    inner.state.serial = inner.next_serial.take();
                    inner.seen_playing = false;
                    inner.last_position_ms = 0;
                }
            }
            Track::Foreign(title) => {
                inner.state.song_id = None;
                inner.state.foreign = Some(title);
            }
        }
        if let Some(position_ms) = reading.position_ms {
            inner.state.position_ms = position_ms;
        }
        if let Some(duration_ms) = reading.duration_ms.filter(|duration| *duration > 0) {
            inner.state.duration_ms = duration_ms;
        }
        inner.state.polled_at_ms = crate::zones::service::now_ms();
        if let Some(value) = reading.volume {
            let range = DeviceVolumeRange {
                min: self.volume_range.0,
                max: self.volume_range.1,
                step: self.volume_range.2,
                unit: "steps",
                hardware: true,
                current: None,
            };
            inner.state.device_volume = Some(value);
            inner.state.volume = from_device_scale(value, &range);
        }
        if let Some(muted) = reading.muted {
            inner.state.muted = muted;
        }
        inner.state.decoding = if state == PlayState::Playing && inner.state.foreign.is_none() {
            reading.decoding
        } else {
            None
        };
        // A track that stops by itself near its end has finished.
        if state == PlayState::Playing && inner.state.foreign.is_none() {
            inner.seen_playing = true;
            inner.last_position_ms = inner.state.position_ms;
        } else if state == PlayState::Stopped
            && previous != PlayState::Stopped
            && inner.seen_playing
            && inner.state.foreign.is_none()
        {
            let duration = inner.state.duration_ms;
            let near_end = duration <= 0 || inner.last_position_ms >= duration - 5_000;
            if near_end {
                inner.state.finished_serial = inner.state.serial;
            }
            inner.seen_playing = false;
        }
        if state == PlayState::Playing {
            inner.pending_seek.take()
        } else {
            None
        }
    }
}

/// SSDP search target of OpenHome renderers.
pub const OPEN_HOME_ST: &str = "urn:av-openhome-org:service:Product:1";
/// Port an SSDP search goes to when a player is added by host alone.
const SSDP_PORT: u16 = 1900;

/// Every renderer on the network.
pub struct UpnpController {
    config: UpnpConfig,
    soap: SoapClient,
    /// Players added by address (shared with AirPlay).
    registry: Option<Arc<NetworkRegistry>>,
    renderers: Mutex<HashMap<String, Arc<UpnpRenderer>>>,
    last_error: Mutex<Option<String>>,
}

impl UpnpController {
    pub fn new(config: UpnpConfig, registry: Option<Arc<NetworkRegistry>>) -> Result<Self, String> {
        Ok(Self {
            config,
            soap: SoapClient::new()?,
            registry,
            renderers: Mutex::new(HashMap::new()),
            last_error: Mutex::new(None),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<UpnpRenderer>>> {
        self.renderers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn record_error(&self, error: String) {
        *self
            .last_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(error);
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn renderers(&self) -> Vec<Arc<dyn Renderer>> {
        let mut renderers: Vec<Arc<UpnpRenderer>> = self.lock().values().cloned().collect();
        renderers.sort_by(|left, right| left.id.cmp(&right.id));
        renderers
            .into_iter()
            .map(|renderer| renderer as Arc<dyn Renderer>)
            .collect()
    }

    pub fn renderer(&self, zone_id: &str) -> Option<Arc<dyn Renderer>> {
        self.lock()
            .get(zone_id)
            .cloned()
            .map(|renderer| renderer as Arc<dyn Renderer>)
    }

    /// The renderer's own picture (its best UPnP icon), fetched from it:
    /// `(content type, bytes)`, PNG, JPEG, GIF or WebP up to 1 MB.
    pub async fn icon(&self, zone_id: &str) -> Option<(String, Vec<u8>)> {
        let url = self
            .lock()
            .get(zone_id)?
            .description
            .best_icon()?
            .url
            .clone();
        let response = self
            .soap
            .http()
            .get(&url)
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if !matches!(
            content_type.as_str(),
            "image/png" | "image/jpeg" | "image/gif" | "image/webp"
        ) {
            return None;
        }
        if response
            .content_length()
            .is_some_and(|length| length > 1 << 20)
        {
            return None;
        }
        let bytes = response.bytes().await.ok()?;
        (bytes.len() <= 1 << 20).then(|| (content_type, bytes.to_vec()))
    }

    /// Players added by address: description URLs to read, and hosts to
    /// ask with a unicast SSDP search.
    fn manual(&self) -> (Vec<String>, Vec<SocketAddr>) {
        let mut locations = Vec::new();
        let mut hosts = Vec::new();
        let endpoints: Vec<ManualEndpoint> = self
            .registry
            .as_ref()
            .map(|registry| registry.manual())
            .unwrap_or_default();
        for endpoint in endpoints
            .into_iter()
            .filter(|endpoint| endpoint.protocol != NetworkProtocol::AirPlay)
        {
            let address = endpoint.address.trim();
            if address.starts_with("http://") {
                locations.push(address.to_string());
            } else if let Some((host, port)) = endpoint.host_port(SSDP_PORT)
                && let Some(target) = (host.as_str(), port)
                    .to_socket_addrs()
                    .ok()
                    .and_then(|mut addresses| addresses.next())
            {
                hosts.push(target);
            }
        }
        (locations, hosts)
    }

    /// SSDP search (MediaRenderer and OpenHome), the configured locations
    /// and the players added by address; returns how many renderers are
    /// known afterwards.
    pub async fn discover(&self) -> Result<usize, String> {
        let (manual_locations, manual_hosts) = self.manual();
        let mut locations: Vec<String> = self.config.static_locations.clone();
        locations.extend(manual_locations.iter().cloned());
        let mut targets = self.config.ssdp_targets.clone();
        targets.extend(manual_hosts.iter().copied());
        let mut manual: Vec<String> = manual_locations;
        if !targets.is_empty() {
            for search_target in [crate::sonos::ssdp::MEDIA_RENDERER_ST, OPEN_HOME_ST] {
                match crate::sonos::ssdp::search_with(
                    &targets,
                    search_target,
                    self.config.search_timeout,
                    |datagram| crate::sonos::ssdp::parse_response_for(datagram, search_target),
                )
                .await
                {
                    Ok(responses) => {
                        for response in responses {
                            // Sonos answers for MediaRenderer too; it has its
                            // own zones.
                            if response
                                .usn
                                .as_deref()
                                .is_some_and(|usn| usn.contains("RINCON_"))
                            {
                                continue;
                            }
                            let from_manual_host = manual_hosts.iter().any(|host| {
                                response.location.contains(&format!("//{}", host.ip()))
                            });
                            if from_manual_host && !manual.contains(&response.location) {
                                manual.push(response.location.clone());
                            }
                            if !locations.contains(&response.location) {
                                locations.push(response.location);
                            }
                        }
                    }
                    Err(error) => self.record_error(error.to_string()),
                }
            }
        }
        for location in locations {
            let known = self
                .lock()
                .values()
                .find(|renderer| renderer.location == location)
                .cloned();
            if let Some(renderer) = known {
                renderer.lock().seen_at = Instant::now();
                continue;
            }
            match self.describe(&location, manual.contains(&location)).await {
                Ok(Some(renderer)) => {
                    let mut renderers = self.lock();
                    if renderers.len() < MAX_RENDERERS || renderers.contains_key(&renderer.id) {
                        renderers.insert(renderer.id.clone(), Arc::new(renderer));
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::debug!(%error, location, "could not read UPnP renderer");
                    self.record_error(error);
                }
            }
        }
        self.lock().retain(|_, renderer| {
            let inner = renderer.lock();
            inner.state.reachable || inner.seen_at.elapsed() < FORGET_AFTER
        });
        Ok(self.lock().len())
    }

    async fn scpd(&self, service: Option<&description::ServiceUrls>) -> description::ServiceInfo {
        match service.filter(|service| !service.scpd_url.is_empty()) {
            Some(service) => match self.soap.get_text(&service.scpd_url).await {
                Ok(document) => description::parse_scpd(&document).unwrap_or_default(),
                Err(_) => description::ServiceInfo::default(),
            },
            None => description::ServiceInfo::default(),
        }
    }

    /// Reads a renderer's description, service descriptions and protocol
    /// list. `Ok(None)` for devices Parson drives another way (Sonos).
    async fn describe(&self, location: &str, manual: bool) -> Result<Option<UpnpRenderer>, String> {
        let document = self.soap.get_text(location).await.map_err(device_error)?;
        let description = description::parse(&document, location)?;
        if description.is_sonos() {
            return Ok(None);
        }
        let control = description.control();
        let av_transport = self.scpd(description.av_transport.as_ref()).await;
        let rendering = self.scpd(description.rendering_control.as_ref()).await;
        let sink = match (&description.connection_manager, &description.oh_playlist) {
            (Some(service), _) => self
                .soap
                .call_at(
                    &service.control_url,
                    description::CONNECTION_MANAGER,
                    "GetProtocolInfo",
                    &[],
                )
                .await
                .ok()
                .and_then(|outputs| outputs.get("Sink").cloned()),
            // An OpenHome playlist lists what it plays itself.
            (None, Some(playlist)) => self
                .soap
                .call_at(
                    &playlist.control_url,
                    &playlist.service_type,
                    "ProtocolInfo",
                    &[],
                )
                .await
                .ok()
                .and_then(|outputs| outputs.get("Value").cloned()),
            (None, None) => None,
        };
        let caps = match sink {
            Some(sink) => protocol_info::parse_sink(&sink),
            None => protocol_info::baseline(),
        };
        let (volume_range, volume_db) = match control {
            Control::UpnpAv => {
                let volume_db = if rendering.has_action("GetVolumeDBRange") {
                    if let Some(service) = &description.rendering_control {
                        self.soap
                            .call_at(
                                &service.control_url,
                                RENDERING_CONTROL,
                                "GetVolumeDBRange",
                                &[("InstanceID", "0"), ("Channel", "Master")],
                            )
                            .await
                            .ok()
                            .and_then(|outputs| {
                                // Units of 1/256 dB.
                                let value = |name: &str| -> Option<f32> {
                                    Some(outputs.get(name)?.trim().parse::<f32>().ok()? / 256.0)
                                };
                                Some((value("MinValue")?, value("MaxValue")?))
                            })
                    } else {
                        None
                    }
                } else {
                    None
                };
                (
                    rendering.volume_range.unwrap_or((0.0, 100.0, 1.0)),
                    volume_db,
                )
            }
            Control::OpenHome => {
                // Volume runs 0..VolumeMax in steps of VolumeMilliDbPerStep.
                let characteristics = match &description.oh_volume {
                    Some(service) => self
                        .soap
                        .call_at(
                            &service.control_url,
                            &service.service_type,
                            "Characteristics",
                            &[],
                        )
                        .await
                        .ok(),
                    None => None,
                };
                let max = characteristics
                    .as_ref()
                    .and_then(|outputs| number::<f32>(outputs, "VolumeMax"))
                    .filter(|max| *max > 0.0)
                    .unwrap_or(100.0);
                let volume_db = characteristics.as_ref().and_then(|outputs| {
                    let step = number::<f32>(outputs, "VolumeMilliDbPerStep")? / 1_000.0;
                    let unity = number::<f32>(outputs, "VolumeUnity").unwrap_or(max);
                    (step > 0.0).then(|| (-unity * step, (max - unity) * step))
                });
                ((0.0, max, 1.0), volume_db)
            }
        };
        let gapless = match control {
            Control::UpnpAv => av_transport.has_action("SetNextAVTransportURI"),
            Control::OpenHome => true,
        };
        Ok(Some(UpnpRenderer {
            id: format!("upnp:{}", description.udn),
            location: location.to_string(),
            control,
            soap: self.soap.clone(),
            caps,
            volume_range,
            volume_db,
            manual,
            inner: Mutex::new(Inner {
                state: RendererState::default(),
                current_uri: None,
                next_uri: None,
                next_serial: None,
                seen_playing: false,
                last_position_ms: 0,
                pending_seek: None,
                volume_mode: VolumeMode::Hardware,
                failures: 0,
                seen_at: Instant::now(),
                gapless,
                playlist_current: None,
                playlist_next: None,
            }),
            description,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::song_from_uri;

    #[test]
    fn reads_the_song_back_from_signed_render_urls() {
        assert_eq!(
            song_from_uri(
                "http://10.0.0.2:1993/api/v1/zones/render/song%201%2Fa/stream?zone=upnp%3Ax&expires=1&signature=s"
            )
            .as_deref(),
            Some("song 1/a")
        );
        assert_eq!(song_from_uri("http://radio.example/stream.mp3"), None);
    }
}
