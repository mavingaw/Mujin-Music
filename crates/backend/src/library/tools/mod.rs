//! Library tools: Roon-style maintenance done without touching the music
//! files. Skipped files with exact reasons and fixes, duplicate detection
//! (tags, lengths, ISRCs and acoustic fingerprints), album versions with a
//! preferred version, the Albumizer, rescans, re-identification and audio
//! analysis with progress, library statistics and missing-file clean-up.

pub mod albumizer;
pub mod analysis;
pub mod duplicates;
pub mod jobs;
pub mod layers;
pub mod quality;
pub mod skipped;
pub mod stats;
pub mod store;
pub mod versions;
