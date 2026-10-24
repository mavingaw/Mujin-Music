//! Roon-style zones: every place music can play, with transfer, grouping,
//! per-zone volume and remote control.

#[cfg(test)]
mod e2e_tests;
pub mod model;
pub mod service;

pub use model::*;
pub use service::{ClientRegistration, ZoneEvent, ZoneService, ZoneSettings};
