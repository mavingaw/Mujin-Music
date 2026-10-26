//! AirPlay receivers as zones (`airplay:<id>`).
//!
//! The server plays each track itself with the same engine as its audio
//! devices (decode, leveling, DSP, volume), at AirPlay's fixed 44.1 kHz/
//! 16-bit, and streams it to the receiver over RAOP as lossless ALAC
//! (`parson_output::network::raop`). CD quality files arrive bit for bit;
//! hi-res files are converted and the signal path says so. Receivers are
//! found by DNS-SD (`_raop._tcp`), added by address (Settings → Audio
//! outputs), or listed in `PARSON_AIRPLAY_RECEIVERS` (`host:port`, comma
//! separated).

use std::sync::Arc;

use parson_output::network::raop::{RaopBackend, Receiver};
use parson_output::network::{ManualEndpoint, NetworkProtocol, NetworkRegistry};
use serde::Serialize;

use super::device::{DeviceManager, Placement};
use crate::zones::ZoneKind;

pub const AIRPLAY_PREFIX: &str = "airplay:";
pub const AIRPLAY_NOTE: &str = "AirPlay (RAOP, unencrypted) as AirPort Express, most AV receivers and shairport-sync speak it. Tested against a protocol-level fake receiver and ffmpeg's ALAC decoder, not yet against Apple hardware. AirPlay 2-only speakers (HomePod, Apple TV) need HomeKit pairing that Parson does not do yet.";

/// A receiver as the outputs status lists it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReceiverStatus {
    pub id: String,
    pub name: String,
    pub model: Option<String>,
    pub address: String,
    pub supported: bool,
    /// Why Parson cannot play to it.
    pub reason: Option<String>,
}

impl ReceiverStatus {
    fn from_receiver(receiver: &Receiver) -> Self {
        let reason = receiver.txt.unsupported_reason();
        Self {
            id: receiver.id.clone(),
            name: receiver.name.clone(),
            model: parson_output::network::raop::identify_model(receiver.txt.am.as_deref()).1,
            address: receiver.address.to_string(),
            supported: reason.is_none(),
            reason,
        }
    }
}

/// AirPlay receivers and their zones.
pub struct AirplayController {
    pub backend: Arc<RaopBackend>,
    pub devices: Arc<DeviceManager>,
    changed: tokio::sync::broadcast::Sender<()>,
}

impl AirplayController {
    /// Receivers from `registry` (added by address) and those reported
    /// through [`AirplayController::add_receiver`] or DNS-SD.
    pub fn new(registry: Arc<NetworkRegistry>) -> Arc<Self> {
        let backend = Arc::new(RaopBackend::new(registry));
        let changed = tokio::sync::broadcast::channel(64).0;
        let devices = Arc::new(DeviceManager::placed(
            backend.clone(),
            None,
            Placement {
                // Receiver ids already start with `airplay:`.
                prefix: String::new(),
                kind: ZoneKind::Airplay,
                host: None,
                machine: None,
                route: Some("over AirPlay (ALAC)".to_string()),
                step: None,
                notes: vec![AIRPLAY_NOTE.to_string()],
                playable_only: true,
            },
            changed.clone(),
        ));
        let weak = Arc::downgrade(&devices);
        backend.on_change(Arc::new(move || {
            if let Some(devices) = weak.upgrade() {
                devices.refresh();
            }
        }));
        Arc::new(Self {
            backend,
            devices,
            changed,
        })
    }

    /// On unless `PARSON_AIRPLAY=off`; receivers from DNS-SD, from
    /// `registry` and from `PARSON_AIRPLAY_RECEIVERS`.
    pub fn from_env(registry: Arc<NetworkRegistry>, discovery: bool) -> Option<Arc<Self>> {
        let setting = std::env::var("PARSON_AIRPLAY").unwrap_or_default();
        if matches!(
            setting.trim().to_ascii_lowercase().as_str(),
            "off" | "0" | "false" | "no"
        ) {
            return None;
        }
        if let Ok(list) = std::env::var("PARSON_AIRPLAY_RECEIVERS") {
            let mut manual = registry.manual();
            for address in list
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let endpoint = ManualEndpoint {
                    protocol: NetworkProtocol::AirPlay,
                    address: address.to_string(),
                    name: None,
                };
                if endpoint.validate().is_ok() && !manual.contains(&endpoint) {
                    manual.push(endpoint);
                }
            }
            registry.set_manual(manual);
        }
        let controller = Self::new(registry);
        if discovery {
            controller.backend.browse();
        }
        controller.devices.refresh();
        Some(controller)
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.changed.subscribe()
    }

    /// Adds or updates a receiver (as DNS-SD reports it).
    pub fn add_receiver(&self, receiver: Receiver) {
        if !self.backend.upsert(receiver) {
            return;
        }
        let _ = self.changed.send(());
    }

    /// Re-reads the receivers added by address.
    pub fn refresh(&self) {
        self.devices.refresh();
    }

    pub fn status(&self) -> serde_json::Value {
        let receivers: Vec<ReceiverStatus> = self
            .backend
            .found()
            .iter()
            .map(ReceiverStatus::from_receiver)
            .collect();
        serde_json::json!({
            "enabled": true,
            "receivers": receivers,
            "note": AIRPLAY_NOTE,
        })
    }
}
