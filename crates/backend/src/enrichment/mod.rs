//! Opt-in metadata enrichment from MusicBrainz, Wikidata, Wikipedia and
//! Wikimedia Commons.
//!
//! Nothing here contacts the network unless an administrator turns
//! enrichment on in settings (it is off by default). The worker resolves
//! albums and artists to MusicBrainz, follows Wikidata to Wikipedia for
//! biographies, album background and reception text (CC BY-SA, stored with
//! attribution), keeps freely licensed artist photos, and stores
//! MusicBrainz relationship credits next to the credits read from tags.

pub mod fetch;
pub mod musicbrainz;
pub mod store;
pub mod wikimedia;
pub mod worker;
pub mod works;
