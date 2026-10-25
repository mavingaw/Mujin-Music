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
        let target = if self.flac {
            format!(
                "converted to {}-bit FLAC at up to {}",
                self.max_bit_depth.min(24),
                format_rate(self.max_sample_rate)
            )
        } else {
            "converted to 320 kbps MP3".to_string()
        };
        format!(
            "{device} plays up to {}/{}-bit. {listed} within that stream untouched; anything else is {target}.",
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

/// `FLAC 96 kHz/24-bit`, `MP3 44.1 kHz`.
pub fn describe_source(source: &SourceFormat) -> String {
    let codec = source
        .codec
        .as_deref()
        .map(|codec| match codec {
            "pcm" => "PCM".to_string(),
            other => other.to_ascii_uppercase(),
        })
        .unwrap_or_else(|| "Audio".to_string());
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
        Self { upnp, devices }
    }

    pub fn renderers(&self) -> Vec<Arc<dyn Renderer>> {
        let mut renderers: Vec<Arc<dyn Renderer>> = Vec::new();
        if let Some(devices) = &self.devices {
            renderers.extend(devices.renderers());
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
        None
    }

    /// Looks for renderers and devices (start-up, then periodically).
    pub async fn discover(&self) {
        if let Some(devices) = &self.devices {
            devices.refresh();
        }
        if let Some(upnp) = &self.upnp
            && let Err(error) = upnp.discover().await
        {
            tracing::debug!(%error, "UPnP discovery failed");
        }
    }

    /// Fires when a renderer's state changes on its own (a track ended).
    pub fn subscribe(&self) -> Option<tokio::sync::broadcast::Receiver<()>> {
        self.devices.as_ref().map(|devices| devices.subscribe())
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
            "Chromecast plays up to 96 kHz/24-bit. FLAC, MP3 and AAC within that stream untouched; anything else is converted to 24-bit FLAC at up to 96 kHz."
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
