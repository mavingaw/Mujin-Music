//! AirPlay receivers as zones (`airplay:<id>`).
//!
//! The server plays each track itself with the same engine as its audio
//! devices (decode, leveling, DSP, volume), at AirPlay's fixed 44.1 kHz/
//! 16-bit, and streams it to the receiver over RAOP as lossless ALAC. CD
//! quality files arrive bit for bit; hi-res files are converted and the
//! signal path says so. Receivers are found by DNS-SD (`_raop._tcp`) or
//! listed in `PARSON_AIRPLAY_RECEIVERS` (`host:port`, comma separated).

pub mod alac;
#[cfg(test)]
pub mod fake;
pub mod raop;

use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;

use super::device::{
    DeviceBackend, DeviceInfo, DeviceManager, HardwareVolume, OutputStream, Placement,
    SampleFormat, StreamFormat,
};
use crate::zones::ZoneKind;

pub const AIRPLAY_PREFIX: &str = "airplay:";
pub const SERVICE_TYPE: &str = "_raop._tcp.local.";
pub const AIRPLAY_NOTE: &str = "AirPlay (RAOP, unencrypted) as AirPort Express, most AV receivers and shairport-sync speak it. Tested against a protocol-level fake receiver and ffmpeg's ALAC decoder, not yet against Apple hardware. AirPlay 2-only speakers (HomePod, Apple TV) need HomeKit pairing that Parson does not do yet.";

/// A receiver found on the network.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Receiver {
    pub id: String,
    pub name: String,
    pub model: Option<String>,
    pub address: String,
    pub supported: bool,
    /// Why Parson cannot play to it.
    pub reason: Option<String>,
}

/// Whether a receiver takes unencrypted AirPlay 1 from its TXT record:
/// `et` lists encryption types (0 = none), `pw` means a password.
pub fn support_from_txt(et: Option<&str>, password: Option<&str>) -> Result<(), String> {
    if password.is_some_and(|value| value.eq_ignore_ascii_case("true")) {
        return Err(
            "This receiver asks for a password; Parson does not send AirPlay passwords yet.".into(),
        );
    }
    match et {
        Some(types) if !types.split(',').any(|value| value.trim() == "0") => Err(
            "This receiver only takes encrypted AirPlay 2 (HomeKit pairing), which Parson does not do yet."
                .into(),
        ),
        _ => Ok(()),
    }
}

/// The receiver name in a RAOP instance name (`AABBCCDDEEFF@Kitchen`).
pub fn receiver_name(instance: &str) -> (String, String) {
    let instance = instance.split("._raop").next().unwrap_or(instance);
    match instance.split_once('@') {
        Some((id, name)) => (id.to_ascii_lowercase(), name.to_string()),
        None => (instance.to_ascii_lowercase(), instance.to_string()),
    }
}

struct Active {
    session: Arc<Mutex<raop::Session>>,
}

#[derive(Default)]
pub struct AirplayBackend {
    receivers: Mutex<HashMap<String, Receiver>>,
    /// The receiver each open session plays on.
    active: Mutex<HashMap<String, std::sync::Weak<Mutex<raop::Session>>>>,
    volume: Mutex<HashMap<String, f32>>,
    muted: Mutex<HashMap<String, bool>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl AirplayBackend {
    /// Adds or updates a receiver; true when something changed.
    pub fn upsert(&self, receiver: Receiver) -> bool {
        let mut receivers = lock(&self.receivers);
        if receivers.get(&receiver.id) == Some(&receiver) {
            return false;
        }
        receivers.insert(receiver.id.clone(), receiver);
        true
    }

    pub fn receivers(&self) -> Vec<Receiver> {
        let mut receivers: Vec<Receiver> = lock(&self.receivers).values().cloned().collect();
        receivers.sort_by(|left, right| left.name.cmp(&right.name));
        receivers
    }

    fn session(&self, id: &str) -> Option<Arc<Mutex<raop::Session>>> {
        lock(&self.active)
            .get(id)
            .and_then(std::sync::Weak::upgrade)
    }

    fn apply_volume(&self, id: &str) -> Result<(), String> {
        let Some(session) = self.session(id) else {
            return Ok(());
        };
        let muted = lock(&self.muted).get(id).copied().unwrap_or(false);
        let db = if muted {
            -144.0
        } else {
            lock(&self.volume).get(id).copied().unwrap_or(-15.0)
        };
        lock(&session).set_volume(db)
    }
}

impl DeviceBackend for AirplayBackend {
    fn name(&self) -> &'static str {
        "AirPlay"
    }

    fn devices(&self) -> Vec<DeviceInfo> {
        self.receivers()
            .into_iter()
            .filter(|receiver| receiver.supported)
            .map(|receiver| DeviceInfo {
                id: receiver.id.clone(),
                name: receiver.name.clone(),
                description: match &receiver.model {
                    Some(model) => format!("AirPlay · {model}"),
                    None => "AirPlay receiver".to_string(),
                },
                sample_rates: vec![raop::SAMPLE_RATE],
                formats: vec![SampleFormat::S16],
                max_channels: 2,
                hardware_volume: Some(HardwareVolume {
                    min_db: -30.0,
                    max_db: 0.0,
                    step_db: 0.5,
                }),
                busy: false,
            })
            .collect()
    }

    fn open(
        &self,
        device_id: &str,
        format: StreamFormat,
        _exclusive: bool,
    ) -> Result<Box<dyn OutputStream>, String> {
        if format.sample_rate != raop::SAMPLE_RATE
            || format.format != SampleFormat::S16
            || format.channels != 2
        {
            return Err("AirPlay takes 44.1 kHz/16-bit stereo".into());
        }
        let address = lock(&self.receivers)
            .get(device_id)
            .map(|receiver| receiver.address.clone())
            .ok_or("the AirPlay receiver went away")?;
        let socket = address
            .to_socket_addrs()
            .ok()
            .and_then(|mut addresses| addresses.next())
            .ok_or_else(|| format!("could not resolve {address}"))?;
        let session = Arc::new(Mutex::new(raop::Session::open(socket)?));
        lock(&self.active).insert(device_id.to_string(), Arc::downgrade(&session));
        if let Err(error) = self.apply_volume(device_id) {
            tracing::debug!(%error, "AirPlay receiver ignored the volume");
        }
        Ok(Box::new(Stream {
            active: Active { session },
            format,
        }))
    }

    fn set_hardware_volume(&self, device_id: &str, db: f32) -> Result<(), String> {
        lock(&self.volume).insert(device_id.to_string(), db);
        self.apply_volume(device_id)
    }

    fn set_hardware_mute(&self, device_id: &str, muted: bool) -> Result<(), String> {
        lock(&self.muted).insert(device_id.to_string(), muted);
        self.apply_volume(device_id)
    }
}

struct Stream {
    active: Active,
    format: StreamFormat,
}

impl OutputStream for Stream {
    fn format(&self) -> StreamFormat {
        self.format
    }

    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        lock(&self.active.session).write(data)
    }

    fn delay_frames(&mut self) -> u64 {
        lock(&self.active.session).delay_frames()
    }

    fn pause(&mut self, _paused: bool) -> Result<(), String> {
        // RAOP has no pause: flush, and start again on resume.
        Err("AirPlay flushes on pause".into())
    }

    fn drop_buffer(&mut self) {
        if let Err(error) = lock(&self.active.session).flush() {
            tracing::debug!(%error, "AirPlay flush failed");
        }
    }

    fn drain(&mut self) {
        lock(&self.active.session).drain();
    }
}

/// AirPlay receivers and their zones.
pub struct AirplayController {
    pub backend: Arc<AirplayBackend>,
    pub devices: Arc<DeviceManager>,
    changed: tokio::sync::broadcast::Sender<()>,
}

impl AirplayController {
    pub fn new() -> Arc<Self> {
        let backend = Arc::new(AirplayBackend::default());
        let changed = tokio::sync::broadcast::channel(64).0;
        let devices = Arc::new(DeviceManager::placed(
            backend.clone(),
            None,
            Placement {
                prefix: AIRPLAY_PREFIX.to_string(),
                kind: ZoneKind::Airplay,
                host: None,
                route: Some("over AirPlay (ALAC)".to_string()),
                step: None,
                notes: vec![AIRPLAY_NOTE.to_string()],
            },
            changed.clone(),
        ));
        Arc::new(Self {
            backend,
            devices,
            changed,
        })
    }

    /// On unless `PARSON_AIRPLAY=off`; receivers from DNS-SD and
    /// `PARSON_AIRPLAY_RECEIVERS`.
    pub fn from_env() -> Option<Arc<Self>> {
        let setting = std::env::var("PARSON_AIRPLAY").unwrap_or_default();
        if matches!(
            setting.trim().to_ascii_lowercase().as_str(),
            "off" | "0" | "false" | "no"
        ) {
            return None;
        }
        let controller = Self::new();
        if let Ok(list) = std::env::var("PARSON_AIRPLAY_RECEIVERS") {
            for (index, address) in list
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .enumerate()
            {
                controller.add_receiver(Receiver {
                    id: format!("manual-{index}"),
                    name: format!("AirPlay {address}"),
                    model: None,
                    address: address.to_string(),
                    supported: true,
                    reason: None,
                });
            }
        }
        controller.browse();
        Some(controller)
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.changed.subscribe()
    }

    pub fn add_receiver(&self, receiver: Receiver) {
        if self.backend.upsert(receiver) {
            self.devices.refresh();
            let _ = self.changed.send(());
        }
    }

    fn browse(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let _ = std::thread::Builder::new()
            .name("parson-airplay-browse".into())
            .spawn(move || {
                let Ok(daemon) = mdns_sd::ServiceDaemon::new() else {
                    return;
                };
                let Ok(events) = daemon.browse(SERVICE_TYPE) else {
                    return;
                };
                loop {
                    let event = events.recv_timeout(Duration::from_secs(5));
                    let Some(controller) = weak.upgrade() else {
                        let _ = daemon.shutdown();
                        return;
                    };
                    let Ok(mdns_sd::ServiceEvent::ServiceResolved(info)) = event else {
                        continue;
                    };
                    let Some(address) = info
                        .get_addresses_v4()
                        .into_iter()
                        .find(|address| !address.is_loopback() && !address.is_unspecified())
                    else {
                        continue;
                    };
                    let (id, name) = receiver_name(info.get_fullname());
                    let support = support_from_txt(
                        info.get_property_val_str("et"),
                        info.get_property_val_str("pw"),
                    );
                    controller.add_receiver(Receiver {
                        id,
                        name,
                        model: info.get_property_val_str("am").map(str::to_string),
                        address: SocketAddr::new(std::net::IpAddr::V4(address), info.get_port())
                            .to_string(),
                        supported: support.is_ok(),
                        reason: support.err(),
                    });
                }
            });
    }

    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({
            "enabled": true,
            "receivers": self.backend.receivers(),
            "note": AIRPLAY_NOTE,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{receiver_name, support_from_txt};

    #[test]
    fn txt_records_tell_which_receivers_take_airplay_1() {
        assert!(support_from_txt(Some("0,3,5"), None).is_ok());
        assert!(support_from_txt(None, Some("false")).is_ok());
        let homepod = support_from_txt(Some("4"), None).unwrap_err();
        assert!(homepod.contains("AirPlay 2"));
        assert!(support_from_txt(Some("0,1"), Some("true")).is_err());
        assert_eq!(
            receiver_name("A0B1C2D3E4F5@Living Room._raop._tcp.local."),
            ("a0b1c2d3e4f5".to_string(), "Living Room".to_string())
        );
    }
}
