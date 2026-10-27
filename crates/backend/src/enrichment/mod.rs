//! Opt-in metadata enrichment from MusicBrainz, Wikidata, Wikipedia and
//! Wikimedia Commons.
//!
//! Nothing here contacts the network unless an administrator turns
//! enrichment on in settings (it is off by default). The worker resolves
//! albums and artists to MusicBrainz, follows Wikidata to Wikipedia for
//! biographies, album background and reception text (CC BY-SA, stored with
//! attribution), keeps freely licensed artist photos, and stores
//! MusicBrainz relationship credits next to the credits read from tags.

pub mod confidence;
pub mod fetch;
pub mod genres;
pub mod identity;
pub mod musicbrainz;
pub mod preferences;
pub mod store;
pub mod wikimedia;
pub mod worker;
pub mod works;
