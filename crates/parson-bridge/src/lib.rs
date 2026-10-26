//! Parson Bridge: a small network endpoint that makes the audio devices of
//! another machine Parson zones. The Parson server connects to it, decodes
//! each track at its own sample rate and bit depth, and sends the PCM; the
//! bridge plays it on the DAC in exclusive mode, untouched. Bridges grouped
//! together play sample-synced.
//!
//! The bridge plays to its devices through `parson-output`, the same
//! device layer the server uses for its own outputs, so local and remote
//! devices behave the same.

pub mod clock;
pub mod endpoint;
pub mod protocol;
pub mod remote;

#[cfg(test)]
mod tests;
