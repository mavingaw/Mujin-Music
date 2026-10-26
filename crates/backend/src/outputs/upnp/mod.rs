//! UPnP/DLNA media renderers (AV receivers, streamers, active speakers,
//! smart TVs): SSDP discovery of `MediaRenderer:1`, AVTransport playback
//! of signed media URLs with gapless `SetNextAVTransportURI`, volume and
//! mute over RenderingControl on the renderer's own scale, and format
//! limits from ConnectionManager `GetProtocolInfo`. State is polled.

pub mod description;
#[cfg(test)]
pub mod fake;
pub mod protocol_info;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;

use self::description::{AV_TRANSPORT, RENDERING_CONTROL, RendererDescription};
use self::protocol_info::SinkCapabilities;
use super::renderer::{
    PlayState, Renderer, RendererInfo, RendererSettings, RendererState, TrackRequest,
    from_device_scale, to_device_scale,
};
use crate::sonos::soap::{SoapClient, SoapError};
use crate::sonos::xml::{format_duration, parse_duration};
use crate::zones::{DeviceVolumeRange, VolumeMode};

const MAX_RENDERERS: usize = 32;
/// A renderer missing from discovery this long, and unreachable, is dropped.
const FORGET_AFTER: Duration = Duration::from_secs(10 * 60);
/// Path of signed media URLs for server-driven zones.
pub const RENDER_MEDIA_PATH: &str = "/api/v1/zones/render/";

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
    /// (comma separated description URLs).
    pub fn from_env() -> Option<Self> {
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
            ssdp_targets: vec![crate::sonos::ssdp::SSDP_MULTICAST],
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
}

/// One UPnP/DLNA renderer.
pub struct UpnpRenderer {
    id: String,
    location: String,
    description: RendererDescription,
    soap: SoapClient,
    caps: SinkCapabilities,
    volume_range: (f32, f32, f32),
    volume_db: Option<(f32, f32)>,
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
        let mut all = vec![("InstanceID", "0")];
        all.extend_from_slice(arguments);
        self.soap
            .call_at(
                &self.description.av_transport.control_url,
                AV_TRANSPORT,
                action,
                &all,
            )
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

    async fn seek_to(&self, position_ms: i64) -> Result<(), String> {
        let target = format_duration(position_ms);
        self.transport("Seek", &[("Unit", "REL_TIME"), ("Target", &target)])
            .await
            .map(|_| ())
    }

    async fn send_next(&self, next: Option<&TrackRequest>) -> Result<(), String> {
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
                "This renderer does not report its highest sample rate; Parson assumes {}/24-bit. Set a lower maximum in Device setup if hi-res tracks stutter.",
                super::format_rate(self.caps.limits.max_sample_rate)
            ));
        }
        if let Some((minimum, maximum)) = self.volume_db {
            notes.push(format!(
                "Volume is the renderer's own amplifier ({minimum:.1} dB to {maximum:.1} dB), so the audio Parson sends stays untouched."
            ));
        }
        let gapless = self.lock().gapless;
        if !gapless {
            notes.push(
                "This renderer cannot queue the next track, so there may be a short gap between tracks."
                    .to_string(),
            );
        }
        RendererInfo {
            id: self.id.clone(),
            kind: crate::zones::ZoneKind::Upnp,
            name: self.description.friendly_name.clone(),
            device: self.description.model(),
            limits: self.caps.limits,
            volume_range: self
                .description
                .rendering_control
                .as_ref()
                .map(|_| self.range()),
            volume_modes: if self.description.rendering_control.is_some() {
                vec![VolumeMode::Hardware, VolumeMode::Fixed]
            } else {
                vec![VolumeMode::Fixed]
            },
            gapless,
            exclusive_capable: false,
            network: true,
            notes,
        }
    }

    fn state(&self) -> RendererState {
        self.lock().state.clone()
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
            {
                let mut inner = self.lock();
                inner.current_uri = Some(track.uri.clone());
                inner.next_uri = None;
                inner.next_serial = None;
                inner.seen_playing = false;
                inner.last_position_ms = position_ms;
                inner.pending_seek = (position_ms > 0).then_some(position_ms);
                inner.state.song_id = Some(track.song_id.clone());
                inner.state.serial = Some(track.serial);
                inner.state.finished_serial = None;
                inner.state.foreign = None;
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
            self.transport("Play", &[("Speed", "1")]).await?;
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
            self.transport("Pause", &[]).await?;
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
            self.transport("Stop", &[]).await?;
            let mut inner = self.lock();
            inner.state.state = PlayState::Stopped;
            inner.state.position_ms = 0;
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
            self.rendering("SetVolume", &[("DesiredVolume", &value.to_string())])
                .await?;
            let mut inner = self.lock();
            inner.state.volume = level;
            inner.state.device_volume = Some(value as f32);
            Ok(())
        })
    }

    fn set_mute(&self, muted: bool) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.rendering("SetMute", &[("DesiredMute", if muted { "1" } else { "0" })])
                .await?;
            self.lock().state.muted = muted;
            Ok(())
        })
    }

    fn poll(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let transport = self.transport("GetTransportInfo", &[]).await;
            let position = self.transport("GetPositionInfo", &[]).await;
            let (transport, position) = match (transport, position) {
                (Ok(transport), Ok(position)) => (transport, position),
                (Err(error), _) | (_, Err(error)) => {
                    let mut inner = self.lock();
                    inner.failures += 1;
                    if inner.failures >= 3 {
                        inner.state.reachable = false;
                    }
                    tracing::debug!(%error, renderer = self.id, "renderer poll failed");
                    return;
                }
            };
            let volume = if self.description.rendering_control.is_some() {
                let volume = self.rendering("GetVolume", &[]).await.ok();
                let mute = self.rendering("GetMute", &[]).await.ok();
                Some((volume, mute))
            } else {
                None
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
            let retry_seek = {
                let mut inner = self.lock();
                inner.failures = 0;
                inner.state.reachable = true;
                let previous = inner.state.state;
                inner.state.state = state;
                if !uri.is_empty() {
                    match song_from_uri(&uri) {
                        Some(song_id) => {
                            inner.state.song_id = Some(song_id);
                            inner.state.foreign = None;
                            if inner.next_uri.as_deref() == Some(uri.as_str()) {
                                // The renderer moved on to the queued next track.
                                inner.current_uri = Some(uri.clone());
                                inner.next_uri = None;
                                inner.state.serial = inner.next_serial.take();
                                inner.seen_playing = false;
                                inner.last_position_ms = 0;
                            }
                        }
                        None => {
                            inner.state.song_id = None;
                            inner.state.foreign = Some(
                                position
                                    .get("TrackMetaData")
                                    .and_then(|metadata| crate::sonos::didl::title(metadata))
                                    .unwrap_or_else(|| "Playing from another app".to_string()),
                            );
                        }
                    }
                }
                if let Some(position_ms) = position
                    .get("RelTime")
                    .and_then(|value| parse_duration(value))
                {
                    inner.state.position_ms = position_ms;
                }
                if let Some(duration_ms) = position
                    .get("TrackDuration")
                    .and_then(|value| parse_duration(value))
                    .filter(|duration| *duration > 0)
                {
                    inner.state.duration_ms = duration_ms;
                }
                inner.state.polled_at_ms = crate::zones::service::now_ms();
                if let Some((volume, mute)) = &volume {
                    if let Some(value) = volume
                        .as_ref()
                        .and_then(|volume| volume.get("CurrentVolume"))
                        .and_then(|value| value.trim().parse::<f32>().ok())
                    {
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
                    if let Some(muted) = mute.as_ref().and_then(|mute| mute.get("CurrentMute")) {
                        inner.state.muted = matches!(muted.trim(), "1" | "true" | "True");
                    }
                }
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
            };
            if let Some(position_ms) = retry_seek {
                let _ = self.seek_to(position_ms).await;
            }
        })
    }
}

/// Every renderer on the network.
pub struct UpnpController {
    config: UpnpConfig,
    soap: SoapClient,
    renderers: Mutex<HashMap<String, Arc<UpnpRenderer>>>,
    last_error: Mutex<Option<String>>,
}

impl UpnpController {
    pub fn new(config: UpnpConfig) -> Result<Self, String> {
        Ok(Self {
            config,
            soap: SoapClient::new()?,
            renderers: Mutex::new(HashMap::new()),
            last_error: Mutex::new(None),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<UpnpRenderer>>> {
        self.renderers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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

    /// SSDP search plus the configured locations; returns how many
    /// renderers are known afterwards.
    pub async fn discover(&self) -> Result<usize, String> {
        let mut locations: Vec<String> = self.config.static_locations.clone();
        match crate::sonos::ssdp::search_with(
            &self.config.ssdp_targets,
            crate::sonos::ssdp::MEDIA_RENDERER_ST,
            self.config.search_timeout,
            |datagram| {
                crate::sonos::ssdp::parse_response_for(
                    datagram,
                    crate::sonos::ssdp::MEDIA_RENDERER_ST,
                )
            },
        )
        .await
        {
            Ok(responses) => {
                for response in responses {
                    // Sonos answers for MediaRenderer too; it has its own zones.
                    if response
                        .usn
                        .as_deref()
                        .is_some_and(|usn| usn.contains("RINCON_"))
                    {
                        continue;
                    }
                    if !locations.contains(&response.location) {
                        locations.push(response.location);
                    }
                }
            }
            Err(error) => {
                *self
                    .last_error
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(error.to_string());
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
            match self.describe(&location).await {
                Ok(Some(renderer)) => {
                    let mut renderers = self.lock();
                    if renderers.len() < MAX_RENDERERS || renderers.contains_key(&renderer.id) {
                        renderers.insert(renderer.id.clone(), Arc::new(renderer));
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::debug!(%error, location, "could not read UPnP renderer");
                    *self
                        .last_error
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(error);
                }
            }
        }
        self.lock().retain(|_, renderer| {
            let inner = renderer.lock();
            inner.state.reachable || inner.seen_at.elapsed() < FORGET_AFTER
        });
        Ok(self.lock().len())
    }

    /// Reads a renderer's description, service descriptions and protocol
    /// list. `Ok(None)` for devices Parson drives another way (Sonos).
    async fn describe(&self, location: &str) -> Result<Option<UpnpRenderer>, String> {
        let document = self.soap.get_text(location).await.map_err(device_error)?;
        let description = description::parse(&document, location)?;
        if description.is_sonos() {
            return Ok(None);
        }
        let av_transport = match self.soap.get_text(&description.av_transport.scpd_url).await {
            Ok(document) => description::parse_scpd(&document).unwrap_or_default(),
            Err(_) => description::ServiceInfo::default(),
        };
        let rendering = match &description.rendering_control {
            Some(service) => match self.soap.get_text(&service.scpd_url).await {
                Ok(document) => description::parse_scpd(&document).unwrap_or_default(),
                Err(_) => description::ServiceInfo::default(),
            },
            None => description::ServiceInfo::default(),
        };
        let caps = match &description.connection_manager {
            Some(service) => match self
                .soap
                .call_at(
                    &service.control_url,
                    description::CONNECTION_MANAGER,
                    "GetProtocolInfo",
                    &[],
                )
                .await
            {
                Ok(outputs) => protocol_info::parse_sink(
                    outputs.get("Sink").map(String::as_str).unwrap_or_default(),
                ),
                Err(_) => protocol_info::baseline(),
            },
            None => protocol_info::baseline(),
        };
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
        let gapless = av_transport.has_action("SetNextAVTransportURI");
        Ok(Some(UpnpRenderer {
            id: format!("upnp:{}", description.udn),
            location: location.to_string(),
            soap: self.soap.clone(),
            caps,
            volume_range: rendering.volume_range.unwrap_or((0.0, 100.0, 1.0)),
            volume_db,
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
