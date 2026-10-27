//! Sonos output support over local UPnP.
//!
//! - [`ssdp`]: `M-SEARCH` discovery of `ZonePlayer` devices.
//! - [`description`]: device description parsing (room, model, player id).
//! - [`soap`]: AVTransport, RenderingControl, GroupRenderingControl and
//!   ZoneGroupTopology control plus GENA subscriptions.
//! - [`didl`]: DIDL-Lite metadata for queued tracks.
//! - [`topology`]: `GetZoneGroupState` parsing (groups, stereo pairs).
//! - [`driver`]: the controller the zone service drives.

pub mod description;
pub mod didl;
pub mod driver;
#[cfg(test)]
pub mod fake;
pub mod soap;
pub mod ssdp;
pub mod topology;
pub mod xml;

pub use driver::{
    SonosConfig, SonosController, SonosGroup, SonosPlayer, Transport, TransportState,
};
