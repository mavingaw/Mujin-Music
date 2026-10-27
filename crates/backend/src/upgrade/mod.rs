//! Better versions: replacing lossy copies with real lossless ones from the
//! FETCH download manager on the home network, catching fake lossless files, and AI
//! enhancement of lossy files on an enhancer worker with a GPU.
//!
//! Everything here is self-hosted: Mujin Music, FETCH and the enhancer all
//! run on your own machines, and the music files are only ever read.
//! New copies are written beside the originals, never over them.

pub mod candidates;
pub mod enhancer;
#[cfg(test)]
pub(crate) mod fake;
pub mod fetch;
pub mod files;
pub mod matching;
pub mod query;
pub mod realtime;
pub mod service;
pub mod settings;
pub mod store;

pub use service::UpgradeService;

/// Characters a path segment or query value keeps as they are (RFC 3986
/// "unreserved"); everything else is percent-encoded.
const SEGMENT: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Encodes one URL path segment (a FETCH or enhancer job id).
pub(crate) fn fetch_segment(value: &str) -> String {
    percent_encoding::utf8_percent_encode(value, SEGMENT).to_string()
}
