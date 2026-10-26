//! Outputs the server plays itself, one track at a time, from a queue the
//! zone service holds:
//!
//! - [`upnp`]: UPnP/DLNA media renderers on the network (SSDP discovery,
//!   AVTransport/RenderingControl/ConnectionManager over SOAP, gapless with
//!   `SetNextAVTransportURI`, signed media URLs).
//! - [`device`]: the audio devices of the machine running the server (the
//!   desktop app's own computer): each USB DAC or output is its own zone,
//!   with exclusive, bit-perfect playback and the device's own volume.
//!
//! Both implement [`renderer::Renderer`], so the zone service drives them
//! the same way.

pub mod airplay;
pub mod bridge;
pub mod cast;
pub mod device;
pub mod renderer;
pub mod upnp;

use std::sync::Arc;

use serde::Serialize;

pub use renderer::{PlayState, Renderer, RendererInfo, RendererState, SourceFormat, TrackRequest};

/// What an output plays as-is, and its format ceiling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct FormatLimits {
    pub max_sample_rate: u32,
    pub max_bit_depth: u8,
    pub flac: bool,
    pub wav: bool,
    pub alac: bool,
    pub mp3: bool,
    pub aac: bool,
}

impl FormatLimits {
    /// Sonos players: up to 48 kHz/24-bit.
    pub const SONOS: Self = Self {
        max_sample_rate: 48_000,
        max_bit_depth: 24,
        flac: true,
        wav: true,
        alac: true,
        mp3: true,
        aac: true,
    };

    /// Google Cast's Default Media Receiver: FLAC up to 96 kHz/24-bit, MP3
    /// and AAC.
    pub const CHROMECAST: Self = Self {
        max_sample_rate: 96_000,
        max_bit_depth: 24,
        flac: true,
        wav: false,
        alac: false,
        mp3: true,
        aac: true,
    };

    /// Applies a user's maximum (Roon's "Max sample rate" in device setup).
    pub fn capped(self, max_sample_rate: Option<u32>, max_bit_depth: Option<u8>) -> Self {
        Self {
            max_sample_rate: max_sample_rate
                .map(|rate| rate.min(self.max_sample_rate))
                .unwrap_or(self.max_sample_rate),
            max_bit_depth: max_bit_depth
                .map(|depth| depth.min(self.max_bit_depth))
                .unwrap_or(self.max_bit_depth),
            ..self
        }
    }

    /// Formats played as-is, for display.
    pub fn formats(&self) -> Vec<&'static str> {
        let mut formats = Vec::new();
        for (supported, name) in [
            (self.flac, "FLAC"),
            (self.alac, "ALAC"),
            (self.wav, "WAV"),
            (self.mp3, "MP3"),
            (self.aac, "AAC"),
        ] {
            if supported {
                formats.push(name);
            }
        }
        formats
    }

    /// "Plays up to X kHz/Y-bit, converts the rest" in words.
    pub fn note(&self, device: &str) -> String {
        let formats = self.formats();
        let listed = match formats.as_slice() {
            [] => "nothing".to_string(),
            [only] => (*only).to_string(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        };
        let depth = self.max_bit_depth.clamp(16, 24);
        let rest = if self.flac {
            format!(
                "Anything else is sent as FLAC: higher sample rates become the nearest rate it takes in the same family (44.1 or 48 kHz multiples), lossless files keep their own bit depth up to {depth}-bit, and lossy files (Ogg Vorbis, Opus) are decoded to {depth}-bit."
            )
        } else {
            "Anything else is converted to 320 kbps MP3.".to_string()
        };
        format!(
            "{device} plays up to {}/{}-bit. {listed} within that reach it untouched. {rest}",
            format_rate(self.max_sample_rate),
            self.max_bit_depth
        )
    }
}

/// `96 kHz`, `44.1 kHz`.
pub fn format_rate(rate: u32) -> String {
    if rate.is_multiple_of(1_000) {
        format!("{} kHz", rate / 1_000)
    } else {
        let tenths = (rate + 50) / 100;
        format!("{}.{} kHz", tenths / 10, tenths % 10)
    }
}

/// Codec names as people know them: `FLAC`, `Ogg Vorbis`, `WAV`.
pub fn codec_name(source: &SourceFormat) -> String {
    match source.codec.as_deref() {
        Some("vorbis") => "Ogg Vorbis".to_string(),
        Some("opus") => "Opus".to_string(),
        Some("pcm" | "wav") => "WAV".to_string(),
        Some("wavpack" | "wv") => "WavPack".to_string(),
        Some(other) => other.to_ascii_uppercase(),
        None => "Audio".to_string(),
    }
}

/// `FLAC 96 kHz/24-bit`, `MP3 44.1 kHz`.
pub fn describe_source(source: &SourceFormat) -> String {
    let codec = codec_name(source);
    match (source.sample_rate, source.bit_depth) {
        (Some(rate), Some(depth)) if source.lossless() => {
            format!("{codec} {}/{depth}-bit", format_rate(rate))
        }
        (Some(rate), _) => format!("{codec} {}", format_rate(rate)),
        _ => codec,
    }
}

/// Every server-driven output, for the zone service.
#[derive(Default)]
pub struct Outputs {
    pub upnp: Option<Arc<upnp::UpnpController>>,
    pub devices: Option<Arc<device::DeviceManager>>,
    /// Parson Bridges: audio devices of other machines.
    pub bridges: Option<Arc<bridge::BridgeHub>>,
    /// AirPlay receivers.
    pub airplay: Option<Arc<airplay::AirplayController>>,
    /// Chromecast devices and Google Home groups, driven by the server.
    pub cast: Option<Arc<cast::CastController>>,
}

impl Outputs {
    /// Builds the outputs from the environment: UPnP unless `PARSON_UPNP=off`,
    /// local audio devices when `PARSON_LOCAL_AUDIO` asks for them (the
    /// desktop app sets it).
    pub fn from_env() -> Self {
        let upnp =
            upnp::UpnpConfig::from_env().and_then(|config| {
                match upnp::UpnpController::new(config) {
                    Ok(controller) => Some(Arc::new(controller)),
                    Err(error) => {
                        tracing::warn!(%error, "UPnP renderer support is unavailable");
                        None
                    }
                }
            });
        let devices = device::DeviceManager::from_env().map(Arc::new);
        let bridges = bridge::BridgeHub::from_env();
        let airplay = airplay::AirplayController::from_env();
        let cast = cast::CastController::from_env();
        Self {
            upnp,
            devices,
            bridges,
            airplay,
            cast,
        }
    }

    pub fn renderers(&self) -> Vec<Arc<dyn Renderer>> {
        let mut renderers: Vec<Arc<dyn Renderer>> = Vec::new();
        if let Some(devices) = &self.devices {
            renderers.extend(devices.renderers());
        }
        if let Some(bridges) = &self.bridges {
            renderers.extend(bridges.renderers());
        }
        if let Some(airplay) = &self.airplay {
            renderers.extend(airplay.devices.renderers());
        }
        if let Some(cast) = &self.cast {
            renderers.extend(cast.renderers());
        }
        if let Some(upnp) = &self.upnp {
            renderers.extend(upnp.renderers());
        }
        renderers
    }

    pub fn renderer(&self, zone_id: &str) -> Option<Arc<dyn Renderer>> {
        if zone_id.starts_with("device:") {
            return self.devices.as_ref()?.renderer(zone_id);
        }
        if zone_id.starts_with("upnp:") {
            return self.upnp.as_ref()?.renderer(zone_id);
        }
        if zone_id.starts_with(bridge::BRIDGE_PREFIX) || zone_id.starts_with(bridge::GROUP_PREFIX) {
            return self.bridges.as_ref()?.renderer(zone_id);
        }
        if zone_id.starts_with(airplay::AIRPLAY_PREFIX) {
            return self.airplay.as_ref()?.devices.renderer(zone_id);
        }
        if zone_id.starts_with(cast::CAST_PREFIX) {
            return self.cast.as_ref()?.renderer(zone_id);
        }
        None
    }

    /// Looks for renderers and devices (start-up, then periodically).
    pub async fn discover(&self) {
        if let Some(devices) = &self.devices {
            devices.refresh();
        }
        self.maintain_bridges().await;
        if let Some(upnp) = &self.upnp
            && let Err(error) = upnp.discover().await
        {
            tracing::debug!(%error, "UPnP discovery failed");
        }
    }

    /// Connects to bridges that are not connected yet.
    pub async fn maintain_bridges(&self) {
        if let Some(bridges) = self.bridges.clone() {
            let _ = tokio::task::spawn_blocking(move || bridges.maintain()).await;
        }
    }

    /// Fires when a renderer's state changes on its own (a track ended).
    /// Merges the change signals of every output kind (call from within a
    /// Tokio runtime).
    pub fn subscribe(&self) -> Option<tokio::sync::broadcast::Receiver<()>> {
        let sources: Vec<tokio::sync::broadcast::Receiver<()>> = [
            self.devices.as_ref().map(|devices| devices.subscribe()),
            self.bridges.as_ref().map(|bridges| bridges.subscribe()),
            self.airplay.as_ref().map(|airplay| airplay.subscribe()),
            self.cast.as_ref().map(|cast| cast.subscribe()),
        ]
        .into_iter()
        .flatten()
        .collect();
        if sources.is_empty() {
            return None;
        }
        let (sender, receiver) = tokio::sync::broadcast::channel(64);
        for mut source in sources {
            let sender = sender.clone();
            tokio::spawn(async move {
                loop {
                    match source.recv().await {
                        Ok(()) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            if sender.send(()).is_err() {
                                return;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                    }
                }
            });
        }
        Some(receiver)
    }

    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({
            "upnp": self.upnp.as_ref().map(|upnp| serde_json::json!({
                "enabled": true,
                "renderers": upnp.renderers().len(),
                "last_error": upnp.last_error(),
            })).unwrap_or_else(|| serde_json::json!({"enabled": false, "renderers": 0, "last_error": null})),
            "devices": self.devices.as_ref().map(|devices| serde_json::json!({
                "enabled": true,
                "backend": devices.backend_name(),
                "devices": devices.renderers().len(),
                "note": devices.note(),
            })).unwrap_or_else(|| serde_json::json!({
                "enabled": false,
                "backend": null,
                "devices": 0,
                "note": "Audio devices of this computer show as zones in the Parson desktop app.",
            })),
            "airplay": self.airplay.as_ref().map(|airplay| airplay.status())
                .unwrap_or_else(|| serde_json::json!({"enabled": false, "receivers": [], "note": null})),
            "chromecast": self.cast.as_ref().map(|cast| cast.status())
                .unwrap_or_else(|| serde_json::json!({"enabled": false, "devices": [], "note": null})),
            "bridges": self.bridges.as_ref().map(|bridges| serde_json::json!({
                "enabled": true,
                "bridges": bridges.status(),
            })).unwrap_or_else(|| serde_json::json!({"enabled": false, "bridges": []})),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{FormatLimits, SourceFormat, describe_source, format_rate};

    #[test]
    fn notes_say_what_plays_untouched_and_what_converts() {
        assert_eq!(format_rate(44_100), "44.1 kHz");
        assert_eq!(format_rate(352_800), "352.8 kHz");
        assert_eq!(format_rate(96_000), "96 kHz");
        assert_eq!(
            FormatLimits::CHROMECAST.note("Chromecast"),
            "Chromecast plays up to 96 kHz/24-bit. FLAC, MP3 and AAC within that reach it untouched. Anything else is sent as FLAC: higher sample rates become the nearest rate it takes in the same family (44.1 or 48 kHz multiples), lossless files keep their own bit depth up to 24-bit, and lossy files (Ogg Vorbis, Opus) are decoded to 24-bit."
        );
        let capped = FormatLimits::SONOS.capped(Some(44_100), Some(16));
        assert_eq!((capped.max_sample_rate, capped.max_bit_depth), (44_100, 16));
        let mp3_only = FormatLimits {
            flac: false,
            wav: false,
            alac: false,
            aac: false,
            ..FormatLimits::SONOS
        };
        assert!(
            mp3_only
                .note("Radio")
                .ends_with("converted to 320 kbps MP3.")
        );
        assert_eq!(
            describe_source(&SourceFormat {
                codec: Some("flac".into()),
                sample_rate: Some(96_000),
                bit_depth: Some(24),
                channels: Some(2),
            }),
            "FLAC 96 kHz/24-bit"
        );
        assert_eq!(
            describe_source(&SourceFormat {
                codec: Some("mp3".into()),
                sample_rate: Some(44_100),
                bit_depth: None,
                channels: Some(2),
            }),
            "MP3 44.1 kHz"
        );
    }
}
