//! Network players added by address: UPnP/OpenHome renderers and AirPlay
//! receivers on networks where discovery (SSDP, mDNS) does not reach them.
//! Both protocols read the same list; it is kept in the data folder.

use std::sync::Arc;

use parson_output::network::{ManualEndpoint, NetworkProtocol, NetworkRegistry};
use serde::Serialize;

use crate::settings::DataFile;

/// Most players that can be added by address.
pub const MAX_MANUAL_PLAYERS: usize = 32;
/// `0` turns off SSDP and mDNS discovery of network players (players
/// added by address are still played to).
pub const NETWORK_DISCOVERY_ENV: &str = "PARSON_NETWORK_DISCOVERY";

/// What the outputs API says about network players.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct NetworkPlayersView {
    /// SSDP (UPnP) and mDNS (AirPlay) discovery is on.
    pub discovery: bool,
    pub manual: Vec<ManualEndpoint>,
}

/// The list of players added by address.
pub struct NetworkPlayers {
    registry: Arc<NetworkRegistry>,
    /// Where the list is kept; `None` keeps it in memory (tests).
    file: Option<DataFile>,
    discovery: bool,
}

fn parse(bytes: &[u8]) -> Vec<ManualEndpoint> {
    serde_json::from_slice::<Vec<ManualEndpoint>>(bytes)
        .unwrap_or_default()
        .into_iter()
        .filter(|endpoint| endpoint.validate().is_ok())
        .take(MAX_MANUAL_PLAYERS)
        .collect()
}

impl NetworkPlayers {
    /// The saved list (or the old server-output picker's, moved over),
    /// with discovery unless `PARSON_NETWORK_DISCOVERY=0`.
    pub fn from_env() -> Self {
        let mut manual = DataFile::NetworkPlayers
            .read()
            .map(|bytes| parse(&bytes))
            .unwrap_or_default();
        if let Some(bytes) = DataFile::LegacyNetworkOutputs.read() {
            for endpoint in parse(&bytes) {
                if !manual.contains(&endpoint) && manual.len() < MAX_MANUAL_PLAYERS {
                    manual.push(endpoint);
                }
            }
            if serde_json::to_vec_pretty(&manual)
                .ok()
                .is_some_and(|text| DataFile::NetworkPlayers.write(&text).is_ok())
            {
                let _ = DataFile::LegacyNetworkOutputs.retire();
            }
        }
        let discovery = std::env::var(NETWORK_DISCOVERY_ENV).map_or(true, |value| value != "0");
        Self {
            registry: Arc::new(NetworkRegistry::new(manual)),
            file: Some(DataFile::NetworkPlayers),
            discovery,
        }
    }

    /// A list kept in memory only (tests).
    pub fn in_memory(manual: Vec<ManualEndpoint>, discovery: bool) -> Self {
        Self {
            registry: Arc::new(NetworkRegistry::new(manual)),
            file: None,
            discovery,
        }
    }

    pub fn registry(&self) -> Arc<NetworkRegistry> {
        Arc::clone(&self.registry)
    }

    pub fn discovery(&self) -> bool {
        self.discovery
    }

    pub fn view(&self) -> NetworkPlayersView {
        NetworkPlayersView {
            discovery: self.discovery,
            manual: self.registry.manual(),
        }
    }

    /// The UPnP players added by address.
    pub fn upnp(&self) -> Vec<ManualEndpoint> {
        self.registry
            .manual()
            .into_iter()
            .filter(|endpoint| endpoint.protocol != NetworkProtocol::AirPlay)
            .collect()
    }

    fn save(&self, manual: Vec<ManualEndpoint>) -> Result<(), String> {
        if let Some(file) = self.file {
            let text = serde_json::to_vec_pretty(&manual)
                .map_err(|error| format!("Could not save the network players: {error}"))?;
            file.write(&text)
                .map_err(|error| format!("Could not save the network players: {error}"))?;
        }
        self.registry.set_manual(manual);
        Ok(())
    }

    /// Adds (or renames) a player by address.
    pub fn add(&self, endpoint: ManualEndpoint) -> Result<(), String> {
        endpoint.validate()?;
        let mut manual = self.registry.manual();
        let address = endpoint.address.trim().to_string();
        manual
            .retain(|known| known.address.trim() != address || known.protocol != endpoint.protocol);
        if manual.len() >= MAX_MANUAL_PLAYERS {
            return Err(format!(
                "Remove a network player first; at most {MAX_MANUAL_PLAYERS} can be added by address."
            ));
        }
        manual.push(ManualEndpoint {
            address,
            ..endpoint
        });
        self.save(manual)
    }

    pub fn remove(&self, address: &str) -> Result<(), String> {
        let mut manual = self.registry.manual();
        let before = manual.len();
        manual.retain(|known| known.address.trim() != address.trim());
        if manual.len() == before {
            return Err("No network player was added at that address.".into());
        }
        self.save(manual)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn players_are_added_renamed_and_removed_by_address() {
        let players = NetworkPlayers::in_memory(Vec::new(), false);
        let kitchen = ManualEndpoint {
            protocol: NetworkProtocol::AirPlay,
            address: " 10.0.0.9:7000 ".into(),
            name: Some("Kitchen".into()),
        };
        players.add(kitchen.clone()).unwrap();
        players
            .add(ManualEndpoint {
                name: Some("Kitchen speaker".into()),
                ..kitchen.clone()
            })
            .unwrap();
        let manual = players.view().manual;
        assert_eq!(manual.len(), 1, "the same address is renamed, not added");
        assert_eq!(manual[0].address, "10.0.0.9:7000");
        assert_eq!(manual[0].name.as_deref(), Some("Kitchen speaker"));
        players
            .add(ManualEndpoint {
                protocol: NetworkProtocol::UpnpAv,
                address: "http://10.0.0.40:49152/description.xml".into(),
                name: None,
            })
            .unwrap();
        assert_eq!(players.upnp().len(), 1);
        assert!(
            players
                .add(ManualEndpoint {
                    address: "http://x/".into(),
                    ..kitchen
                })
                .is_err(),
            "AirPlay takes host:port"
        );
        players.remove("10.0.0.9:7000").unwrap();
        assert!(players.remove("10.0.0.9:7000").is_err());
        assert_eq!(players.view().manual.len(), 1);
    }
}
