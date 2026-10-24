//! Credit, label and ISRC extraction from file tags.
//!
//! Tag formats spell the same credit many ways (Vorbis `COMPOSER`, ID3 `TCOM`,
//! MP4 `©wrt`, ID3 `TIPL` involvement pairs, iTunes freeform atoms). This
//! module maps all of them onto a small set of canonical roles so that the
//! indexer can store them in `track_artist.role` next to the existing
//! `primary` and `featured` artist rows.

use serde::{Deserialize, Serialize};

use crate::library::identity::normalize_artist_identity;

/// Roles the indexer writes to `track_artist.role` for an artist's own
/// performance credit. Every other role is a contributor credit.
pub const ARTIST_ROLES: [&str; 2] = ["primary", "featured"];

/// Canonical contributor roles, in display order.
pub const CREDIT_ROLES: [&str; 10] = [
    "composer",
    "lyricist",
    "arranger",
    "conductor",
    "performer",
    "ensemble",
    "producer",
    "remixer",
    "mixer",
    "engineer",
];

const MAX_CREDITS_PER_TRACK: usize = 64;
const MAX_CREDIT_TEXT_CHARS: usize = 200;

/// A contributor credit read from a file's tags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagCredit {
    pub role: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instrument: Option<String>,
}

impl TagCredit {
    /// The value stored in `track_artist.role`: the canonical role, or
    /// `performer:<instrument>` for an instrument credit.
    pub fn role_key(&self) -> String {
        role_key(&self.role, self.instrument.as_deref())
    }
}

pub fn role_key(role: &str, instrument: Option<&str>) -> String {
    match instrument {
        Some(instrument) if !instrument.is_empty() => format!("{role}:{instrument}"),
        _ => role.to_string(),
    }
}

/// Splits a stored `track_artist.role` back into role and instrument.
pub fn split_role_key(key: &str) -> (&str, Option<&str>) {
    match key.split_once(':') {
        Some((role, instrument)) if !instrument.is_empty() => (role, Some(instrument)),
        Some((role, _)) => (role, None),
        None => (key, None),
    }
}

/// Position of a role in [`CREDIT_ROLES`], for stable display ordering.
pub fn role_rank(role: &str) -> usize {
    CREDIT_ROLES
        .iter()
        .position(|candidate| *candidate == role)
        .unwrap_or(CREDIT_ROLES.len())
}

/// What a tag field contributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TagField {
    Credit(&'static str),
    /// A performer credit, optionally written as `Name (instrument)`.
    Performer,
    /// A record label; lower values win when several label fields exist.
    Label(u8),
    Isrc,
}

/// Maps a Vorbis comment key, ID3 `TXXX` description or iTunes freeform name
/// to the field it carries. Matching ignores case, spaces and punctuation.
fn named_field(key: &str) -> Option<TagField> {
    let key = key
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_uppercase())
        .collect::<String>();
    Some(match key.as_str() {
        "COMPOSER" | "WRITER" | "SONGWRITER" | "WRITTENBY" | "COMPOSEDBY" => {
            TagField::Credit("composer")
        }
        "LYRICIST" | "LYRICSBY" | "TEXTWRITER" => TagField::Credit("lyricist"),
        "ARRANGER" | "ARRANGEDBY" => TagField::Credit("arranger"),
        "CONDUCTOR" | "CONDUCTEDBY" => TagField::Credit("conductor"),
        "PERFORMER" => TagField::Performer,
        "ENSEMBLE" | "ORCHESTRA" => TagField::Credit("ensemble"),
        "PRODUCER" | "PRODUCEDBY" => TagField::Credit("producer"),
        "REMIXER" | "MIXARTIST" | "REMIXEDBY" => TagField::Credit("remixer"),
        "MIXER" | "MIXENGINEER" | "MIXEDBY" | "DJMIXER" => TagField::Credit("mixer"),
        "ENGINEER" | "RECORDINGENGINEER" | "MASTERINGENGINEER" => TagField::Credit("engineer"),
        "LABEL" | "RECORDLABEL" => TagField::Label(0),
        "ORGANIZATION" | "ORGANISATION" => TagField::Label(1),
        "PUBLISHER" => TagField::Label(2),
        "ISRC" => TagField::Isrc,
        _ => return None,
    })
}

/// Maps an ID3v2 text frame (v2.2 or v2.3/2.4 id) to the field it carries.
fn id3_frame_field(id: &str) -> Option<TagField> {
    Some(match id {
        "TCOM" | "TCM" => TagField::Credit("composer"),
        "TEXT" | "TXT" => TagField::Credit("lyricist"),
        "TPE3" | "TP3" => TagField::Credit("conductor"),
        "TPE4" | "TP4" => TagField::Credit("remixer"),
        "TPUB" | "TPB" => TagField::Label(0),
        "TSRC" | "TRC" => TagField::Isrc,
        _ => return None,
    })
}

/// Maps a `TIPL`/`IPLS` involvement (e.g. `producer`, `mix`) to a role.
fn involvement_role(involvement: &str) -> Option<&'static str> {
    match named_field(involvement) {
        Some(TagField::Credit(role)) => Some(role),
        Some(TagField::Performer) => Some("performer"),
        _ => {
            let involvement = involvement.trim().to_ascii_lowercase();
            match involvement.as_str() {
                "mix" | "dj-mix" | "mixing" => Some("mixer"),
                "composition" => Some("composer"),
                "lyrics" => Some("lyricist"),
                "arrangement" => Some("arranger"),
                "recording" | "mastering" => Some("engineer"),
                _ => None,
            }
        }
    }
}

fn clean_text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.chars().take(MAX_CREDIT_TEXT_CHARS).collect())
}

fn normalize_instrument(value: &str) -> Option<String> {
    clean_text(value).map(|value| value.to_lowercase())
}

/// Splits a Vorbis/Picard style performer value, `Name (instrument)`.
pub fn split_performer(value: &str) -> (String, Option<String>) {
    let value = value.trim();
    if let Some(stripped) = value.strip_suffix(')')
        && let Some(open) = stripped.rfind(" (")
    {
        let name = stripped[..open].trim();
        let instrument = &stripped[open + 2..];
        if !name.is_empty() {
            return (name.to_string(), normalize_instrument(instrument));
        }
    }
    (value.to_string(), None)
}

fn split_values(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(['\0', ';'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// Credits, label and ISRC collected while reading one file's tags.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TagCredits {
    pub credits: Vec<TagCredit>,
    label: Option<(u8, String)>,
    pub isrc: Option<String>,
}

impl TagCredits {
    pub fn label(&self) -> Option<&str> {
        self.label.as_ref().map(|(_, label)| label.as_str())
    }

    fn apply_field(&mut self, field: TagField, value: &str) {
        match field {
            TagField::Credit(role) => {
                for name in split_values(value) {
                    self.push(role, name, None);
                }
            }
            TagField::Performer => {
                for value in split_values(value) {
                    let (name, instrument) = split_performer(value);
                    // Picard writes "Name (conductor)" and "Name (orchestra)"
                    // into PERFORMER; those are roles, not instruments.
                    match instrument.as_deref() {
                        Some("conductor") => self.push("conductor", &name, None),
                        Some("orchestra" | "ensemble") => self.push("ensemble", &name, None),
                        _ => self.push("performer", &name, instrument),
                    }
                }
            }
            TagField::Label(priority) => {
                if let Some(label) = split_values(value).next().and_then(clean_text)
                    && self
                        .label
                        .as_ref()
                        .is_none_or(|(current, _)| priority < *current)
                {
                    self.label = Some((priority, label));
                }
            }
            TagField::Isrc => {
                if self.isrc.is_none() {
                    self.isrc = split_values(value).find_map(normalize_isrc);
                }
            }
        }
    }

    /// Applies a Vorbis comment, ID3 `TXXX` or iTunes freeform field.
    /// Returns whether the key was recognised.
    pub fn apply_named(&mut self, key: &str, value: &str) -> bool {
        match named_field(key) {
            Some(field) => {
                self.apply_field(field, value);
                true
            }
            None => false,
        }
    }

    /// Applies an ID3v2 text frame by id. Returns whether it was recognised.
    pub fn apply_id3_frame(&mut self, id: &str, value: &str) -> bool {
        match id3_frame_field(id) {
            Some(field) => {
                self.apply_field(field, value);
                true
            }
            None => false,
        }
    }

    /// Applies one `TIPL`/`IPLS` involvement pair.
    pub fn apply_involvement(&mut self, involvement: &str, names: &str) {
        if let Some(role) = involvement_role(involvement) {
            for name in split_values(names) {
                self.push(role, name, None);
            }
        }
    }

    /// Applies one `TMCL` musician credit pair (instrument, name).
    pub fn apply_musician(&mut self, instrument: &str, names: &str) {
        let instrument = normalize_instrument(instrument);
        for name in split_values(names) {
            self.push("performer", name, instrument.clone());
        }
    }

    /// Adds a credit, ignoring duplicates of the same person in the same role.
    pub fn push(&mut self, role: &str, name: &str, instrument: Option<String>) {
        let Some(name) = clean_text(name) else {
            return;
        };
        if self.credits.len() >= MAX_CREDITS_PER_TRACK {
            return;
        }
        let identity = normalize_artist_identity(&name);
        if identity.is_empty() {
            return;
        }
        let duplicate = self.credits.iter().any(|credit| {
            credit.role == role
                && credit.instrument == instrument
                && normalize_artist_identity(&credit.name) == identity
        });
        if !duplicate {
            self.credits.push(TagCredit {
                role: role.to_string(),
                name,
                instrument,
            });
        }
    }
}

/// Normalises an ISRC (`CC-XXX-YY-NNNNN` or `CCXXXYYNNNNN`) to its 12
/// character form, rejecting values that do not look like one.
pub fn normalize_isrc(value: &str) -> Option<String> {
    let compact = value
        .chars()
        .filter(|character| !matches!(character, '-' | ' '))
        .map(|character| character.to_ascii_uppercase())
        .collect::<String>();
    let bytes = compact.as_bytes();
    let valid = bytes.len() == 12
        && bytes[..2].iter().all(u8::is_ascii_alphabetic)
        && bytes[2..5].iter().all(u8::is_ascii_alphanumeric)
        && bytes[5..].iter().all(u8::is_ascii_digit);
    valid.then_some(compact)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vorbis_keys_map_to_canonical_roles() {
        let mut credits = TagCredits::default();
        assert!(credits.apply_named("COMPOSER", "Clara Composer; Second Composer"));
        assert!(credits.apply_named("Conductor", "Carl Conductor"));
        assert!(credits.apply_named("PRODUCER", "Pat Producer"));
        assert!(credits.apply_named("MIXARTIST", "Rémy Remixer"));
        assert!(credits.apply_named("PERFORMER", "Piper Pianist (Piano)"));
        assert!(credits.apply_named("PERFORMER", "Solo Singer"));
        assert!(!credits.apply_named("COMMENT", "ignored"));

        let roles = credits
            .credits
            .iter()
            .map(|credit| (credit.role_key(), credit.name.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            roles,
            [
                ("composer".to_string(), "Clara Composer"),
                ("composer".to_string(), "Second Composer"),
                ("conductor".to_string(), "Carl Conductor"),
                ("producer".to_string(), "Pat Producer"),
                ("remixer".to_string(), "Rémy Remixer"),
                ("performer:piano".to_string(), "Piper Pianist"),
                ("performer".to_string(), "Solo Singer"),
            ]
        );
    }

    #[test]
    fn performer_roles_written_as_instruments_become_roles() {
        let mut credits = TagCredits::default();
        credits.apply_named("PERFORMER", "Gil Evans (conductor); LSO (Orchestra)");
        let roles = credits
            .credits
            .iter()
            .map(|credit| (credit.role_key(), credit.name.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            roles,
            [
                ("conductor".to_string(), "Gil Evans"),
                ("ensemble".to_string(), "LSO")
            ]
        );
    }

    #[test]
    fn duplicate_credits_collapse_by_identity() {
        let mut credits = TagCredits::default();
        credits.apply_named("COMPOSER", "Clara Composer");
        credits.apply_id3_frame("TCOM", "clara composer");
        credits.apply_musician("Guitar", "Gil Guitarist");
        credits.apply_musician("guitar", "Gil Guitarist");
        credits.apply_musician("Bass", "Gil Guitarist");
        assert_eq!(credits.credits.len(), 3);
        assert_eq!(credits.credits[1].role_key(), "performer:guitar");
        assert_eq!(credits.credits[2].role_key(), "performer:bass");
    }

    #[test]
    fn involvement_lists_map_known_roles_and_skip_unknown_ones() {
        let mut credits = TagCredits::default();
        credits.apply_involvement("producer", "Pat Producer");
        credits.apply_involvement("mix", "Max Mixer");
        credits.apply_involvement("DJ-mix", "Dee Jay");
        credits.apply_involvement("engineer", "Erin Engineer");
        credits.apply_involvement("catering", "Nobody");
        let roles = credits
            .credits
            .iter()
            .map(|credit| credit.role.as_str())
            .collect::<Vec<_>>();
        assert_eq!(roles, ["producer", "mixer", "mixer", "engineer"]);
    }

    #[test]
    fn label_prefers_label_over_organization_and_publisher() {
        let mut credits = TagCredits::default();
        credits.apply_named("PUBLISHER", "Publishing Co");
        assert_eq!(credits.label(), Some("Publishing Co"));
        credits.apply_named("ORGANIZATION", "Org Records");
        assert_eq!(credits.label(), Some("Org Records"));
        credits.apply_named("LABEL", "Real Label");
        credits.apply_named("PUBLISHER", "Later Publisher");
        assert_eq!(credits.label(), Some("Real Label"));
    }

    #[test]
    fn isrc_is_normalized_and_validated() {
        assert_eq!(
            normalize_isrc("us-rc1-76-07839").as_deref(),
            Some("USRC17607839")
        );
        assert_eq!(normalize_isrc("not an isrc"), None);
        let mut credits = TagCredits::default();
        credits.apply_id3_frame("TSRC", "garbage");
        credits.apply_id3_frame("TSRC", "GBAYE0601498");
        credits.apply_named("ISRC", "USRC17607839");
        assert_eq!(credits.isrc.as_deref(), Some("GBAYE0601498"));
    }

    #[test]
    fn performer_values_split_only_a_trailing_parenthesised_instrument() {
        assert_eq!(
            split_performer("Orchestra (Live)"),
            ("Orchestra".to_string(), Some("live".to_string()))
        );
        assert_eq!(split_performer("(Piano)"), ("(Piano)".to_string(), None));
        assert_eq!(
            split_performer("Plain Name"),
            ("Plain Name".to_string(), None)
        );
    }

    #[test]
    fn role_keys_round_trip() {
        assert_eq!(
            split_role_key("performer:piano"),
            ("performer", Some("piano"))
        );
        assert_eq!(split_role_key("composer"), ("composer", None));
        assert_eq!(role_key("performer", Some("violin")), "performer:violin");
        assert!(role_rank("composer") < role_rank("engineer"));
        assert_eq!(role_rank("unknown"), CREDIT_ROLES.len());
    }
}
