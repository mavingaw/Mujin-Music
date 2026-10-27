//! Roon-style zones: every place music can play, with transfer, grouping,
//! per-zone volume and remote control.

#[cfg(test)]
mod e2e_tests;
pub mod leveling;
pub mod model;
#[cfg(test)]
mod outputs_e2e_tests;
pub mod service;

pub use model::*;
pub use service::{
    ClientRegistration, Viewer, ZoneEvent, ZoneService, ZoneSettings, ZoneSettingsPatch,
};
