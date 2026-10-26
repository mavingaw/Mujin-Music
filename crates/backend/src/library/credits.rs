//! Credit, label and ISRC extraction from file tags.
//!
//! Tag formats spell the same credit many ways (Vorbis `COMPOSER`, ID3 `TCOM`,
//! MP4 `©wrt`, ID3 `TIPL` involvement pairs, iTunes freeform atoms). This
//! module maps all of them onto a small set of canonical roles so that the
//! indexer can store them in `track_artist.role` next to the existing
//! `primary` and `featured` artist rows.

use serde::{Deserialize, Serialize};

use crate::domain::Loudness;
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
    /// The musical work (composition) a recording performs.
    Work,
    /// A movement's name within its work.
    Movement,
    /// A movement's position within its work (`3` or `3/4`).
    MovementNumber,
    /// The MusicBrainz release type (`album`, `single`, `ep`, ...).
    ReleaseType,
    /// Picard's multi-value list of the individual performing artists.
    Artists,
    /// A conductor credit. Taggers that misuse ID3 `TPE3` for soloists
    /// write `Name (instrument)`, which is a performer credit instead.
    Conductor,
    /// The MusicBrainz work id of the recorded work.
    WorkId,
    /// The year a work was composed (`1812`, `1808-1812`, `1812-05-01`).
    ComposedYear,
    /// A ReplayGain value.
    Loudness(LoudnessField),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoudnessField {
    TrackGain,
    TrackPeak,
    AlbumGain,
    AlbumPeak,
    /// Opus `R128_*_GAIN`: Q7.8 fixed point relative to -23 LUFS.
    R128Track,
    R128Album,
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
        "CONDUCTOR" | "CONDUCTEDBY" => TagField::Conductor,
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
        "WORK" | "WORKTITLE" => TagField::Work,
        "MOVEMENTNAME" => TagField::Movement,
        "MOVEMENT" | "MOVEMENTNUMBER" => TagField::MovementNumber,
        "RELEASETYPE" | "MUSICBRAINZALBUMTYPE" | "MUSICBRAINZRELEASETYPE" => TagField::ReleaseType,
        "ARTISTS" => TagField::Artists,
        "MUSICBRAINZWORKID" | "MUSICBRAINZWORK" => TagField::WorkId,
        "COMPOSITIONDATE" | "COMPOSITIONYEAR" | "COMPOSED" | "DATECOMPOSED" | "COMPOSEDDATE"
        | "WORKDATE" | "WORKYEAR" => TagField::ComposedYear,
        "REPLAYGAINTRACKGAIN" => TagField::Loudness(LoudnessField::TrackGain),
        "REPLAYGAINTRACKPEAK" => TagField::Loudness(LoudnessField::TrackPeak),
        "REPLAYGAINALBUMGAIN" => TagField::Loudness(LoudnessField::AlbumGain),
        "REPLAYGAINALBUMPEAK" => TagField::Loudness(LoudnessField::AlbumPeak),
        "R128TRACKGAIN" => TagField::Loudness(LoudnessField::R128Track),
        "R128ALBUMGAIN" => TagField::Loudness(LoudnessField::R128Album),
        _ => return None,
    })
}

/// Maps an ID3v2 text frame (v2.2 or v2.3/2.4 id) to the field it carries.
fn id3_frame_field(id: &str) -> Option<TagField> {
    Some(match id {
        "TCOM" | "TCM" => TagField::Credit("composer"),
        "TEXT" | "TXT" => TagField::Credit("lyricist"),
        "TPE3" | "TP3" => TagField::Conductor,
        "TPE4" | "TP4" => TagField::Credit("remixer"),
        "TPUB" | "TPB" => TagField::Label(0),
        "TSRC" | "TRC" => TagField::Isrc,
        "MVNM" => TagField::Movement,
        "MVIN" => TagField::MovementNumber,
        _ => return None,
    })
}

/// Maps a `TIPL`/`IPLS` involvement (e.g. `producer`, `mix`) to a role.
fn involvement_role(involvement: &str) -> Option<&'static str> {
    match named_field(involvement) {
        Some(TagField::Credit(role)) => Some(role),
        Some(TagField::Conductor) => Some("conductor"),
        Some(TagField::Performer) => Some("performer"),
        Some(TagField::Artists) => Some("performer"),
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

/// Instruments a conductor credit may name without being a soloist.
fn is_conducting(instrument: &str) -> bool {
    matches!(
        instrument,
        "conductor" | "conducting" | "conducted by" | "dirigent" | "direction" | "director"
    )
}

/// Reads a MusicBrainz id (a lowercase UUID).
pub fn normalize_musicbrainz_id(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    let bytes = value.as_bytes();
    let valid = bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        });
    valid.then_some(value)
}

/// Reads the first plausible composition year from a date or range.
pub fn parse_composed_year(value: &str) -> Option<u16> {
    let bytes = value.as_bytes();
    (0..bytes.len().saturating_sub(3)).find_map(|start| {
        let digits = &bytes[start..start + 4];
        let bounded = (start == 0 || !bytes[start - 1].is_ascii_digit())
            && bytes
                .get(start + 4)
                .is_none_or(|next| !next.is_ascii_digit());
        if !bounded || !digits.iter().all(u8::is_ascii_digit) {
            return None;
        }
        std::str::from_utf8(digits)
            .ok()?
            .parse::<u16>()
            .ok()
            .filter(|year| (900..=2100).contains(year))
    })
}

fn split_values(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(['\0', ';'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// Credits, label, ISRC and ReplayGain collected while reading one file's tags.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TagCredits {
    pub credits: Vec<TagCredit>,
    label: Option<(u8, String)>,
    pub isrc: Option<String>,
    pub work: Option<String>,
    pub movement: Option<String>,
    pub movement_number: Option<u16>,
    /// Canonical MusicBrainz release type; `None` when no tag states it.
    pub release_type: Option<String>,
    /// MusicBrainz work id (lowercase UUID).
    pub work_id: Option<String>,
    pub composed_year: Option<u16>,
    pub loudness: Loudness,
    track_gain_from_tag: bool,
    album_gain_from_tag: bool,
}

/// Parses a ReplayGain gain such as `-6.40 dB` or `+1.2dB`.
pub fn parse_replay_gain_db(value: &str) -> Option<f32> {
    let value = value.trim();
    let number = value
        .strip_suffix("dB")
        .or_else(|| value.strip_suffix("db"))
        .or_else(|| value.strip_suffix("DB"))
        .unwrap_or(value)
        .trim();
    let gain = number.parse::<f32>().ok()?;
    (gain.is_finite() && (-60.0..=30.0).contains(&gain)).then_some(gain)
}

/// Parses a linear ReplayGain peak such as `0.988`.
pub fn parse_replay_gain_peak(value: &str) -> Option<f32> {
    let peak = value.trim().parse::<f32>().ok()?;
    (peak.is_finite() && peak > 0.0 && peak <= 10.0).then_some(peak)
}

/// Converts an Opus `R128_*_GAIN` (Q7.8, relative to -23 LUFS) to a
/// ReplayGain 2.0 gain (relative to -18 LUFS).
pub fn r128_to_replay_gain_db(value: &str) -> Option<f32> {
    let raw = value.trim().parse::<i32>().ok()?;
    if !(-32768..=32767).contains(&raw) {
        return None;
    }
    let gain = raw as f32 / 256.0 + 5.0;
    (-60.0..=30.0).contains(&gain).then_some(gain)
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
                    self.push("performer", &name, instrument);
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
            TagField::Work => {
                if self.work.is_none() {
                    self.work = split_values(value).next().and_then(clean_text);
                }
            }
            TagField::Movement => {
                if self.movement.is_none() {
                    self.movement = split_values(value).next().and_then(clean_text);
                }
            }
            TagField::MovementNumber => {
                if self.movement_number.is_none() {
                    self.movement_number = parse_movement_number(value);
                }
            }
            TagField::ReleaseType => {
                if self.release_type.is_none() {
                    self.release_type = normalize_release_type(value);
                }
            }
            TagField::Artists => {
                for name in split_values(value) {
                    self.push("performer", name, None);
                }
            }
            TagField::Conductor => {
                for value in split_values(value) {
                    match split_performer(value) {
                        (name, Some(instrument)) if !is_conducting(&instrument) => {
                            self.push("performer", &name, Some(instrument));
                        }
                        (name, _) => self.push("conductor", &name, None),
                    }
                }
            }
            TagField::WorkId => {
                if self.work_id.is_none() {
                    self.work_id = split_values(value).find_map(normalize_musicbrainz_id);
                }
            }
            TagField::ComposedYear => {
                if self.composed_year.is_none() {
                    self.composed_year = parse_composed_year(value);
                }
            }
            TagField::Loudness(field) => self.apply_loudness(field, value),
        }
    }

    /// ReplayGain tags win over Opus R128 gains; the first value of each
    /// field is kept.
    fn apply_loudness(&mut self, field: LoudnessField, value: &str) {
        let loudness = &mut self.loudness;
        match field {
            LoudnessField::TrackGain => {
                if let Some(gain) = parse_replay_gain_db(value) {
                    if !self.track_gain_from_tag {
                        loudness.track_gain_db = Some(gain);
                    }
                    self.track_gain_from_tag = true;
                }
            }
            LoudnessField::AlbumGain => {
                if let Some(gain) = parse_replay_gain_db(value) {
                    if !self.album_gain_from_tag {
                        loudness.album_gain_db = Some(gain);
                    }
                    self.album_gain_from_tag = true;
                }
            }
            LoudnessField::TrackPeak => {
                if loudness.track_peak.is_none() {
                    loudness.track_peak = parse_replay_gain_peak(value);
                }
            }
            LoudnessField::AlbumPeak => {
                if loudness.album_peak.is_none() {
                    loudness.album_peak = parse_replay_gain_peak(value);
                }
            }
            LoudnessField::R128Track => {
                if !self.track_gain_from_tag && loudness.track_gain_db.is_none() {
                    loudness.track_gain_db = r128_to_replay_gain_db(value);
                }
            }
            LoudnessField::R128Album => {
                if !self.album_gain_from_tag && loudness.album_gain_db.is_none() {
                    loudness.album_gain_db = r128_to_replay_gain_db(value);
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

/// Reads a movement number such as `3`, `03` or `3/4`.
pub fn parse_movement_number(value: &str) -> Option<u16> {
    let digits = value
        .trim()
        .split(['/', ' ', '\0'])
        .next()?
        .trim_start_matches('0');
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u16>().ok().filter(|number| *number > 0)
}

/// Primary release types MusicBrainz defines, in the order a combined value
/// is resolved.
pub const PRIMARY_RELEASE_TYPES: [&str; 5] = ["album", "single", "ep", "broadcast", "other"];
/// Secondary release types, used when no primary type is tagged.
pub const SECONDARY_RELEASE_TYPES: [&str; 11] = [
    "compilation",
    "soundtrack",
    "live",
    "remix",
    "spokenword",
    "interview",
    "audiobook",
    "audio drama",
    "demo",
    "dj-mix",
    "mixtape/street",
];

/// Canonical release type from a `RELEASETYPE` / `MusicBrainz Album Type`
/// value such as `album`, `Album; Live` or `ep`. Primary types win over
/// secondary ones; values that are not MusicBrainz types are rejected so
/// that a stray tag never becomes a wrong fact.
pub fn normalize_release_type(value: &str) -> Option<String> {
    let values = value
        .split(['\0', ';', ',', '/'])
        .map(|part| part.trim().to_ascii_lowercase().replace(['_'], "-"))
        .filter(|part| !part.is_empty())
        .map(|part| match part.as_str() {
            "spoken word" | "spoken-word" => "spokenword".to_string(),
            "dj mix" | "djmix" => "dj-mix".to_string(),
            "mixtape" | "street" => "mixtape/street".to_string(),
            "audio-drama" => "audio drama".to_string(),
            _ => part,
        })
        .collect::<Vec<_>>();
    PRIMARY_RELEASE_TYPES
        .iter()
        .chain(SECONDARY_RELEASE_TYPES.iter())
        .find(|candidate| values.iter().any(|value| value == *candidate))
        .map(|value| value.to_string())
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
    fn replay_gain_tags_parse_and_win_over_opus_r128_gains() {
        let mut tags = TagCredits::default();
        assert!(tags.apply_named("R128_TRACK_GAIN", "-1024"));
        assert_eq!(tags.loudness.track_gain_db, Some(1.0));
        assert!(tags.apply_named("replaygain_track_gain", "-6.40 dB"));
        assert!(tags.apply_named("REPLAYGAIN_TRACK_GAIN", "-1.00 dB"));
        assert!(tags.apply_named("REPLAYGAIN_TRACK_PEAK", "0.988312"));
        assert!(tags.apply_named("REPLAYGAIN_ALBUM_GAIN", "+2.5dB"));
        assert!(tags.apply_named("R128_ALBUM_GAIN", "0"));
        assert!(tags.apply_named("REPLAYGAIN_ALBUM_PEAK", "nan"));
        assert_eq!(tags.loudness.track_gain_db, Some(-6.4));
        assert_eq!(tags.loudness.track_peak, Some(0.988312));
        assert_eq!(tags.loudness.album_gain_db, Some(2.5));
        assert_eq!(tags.loudness.album_peak, None);

        let mut opus = TagCredits::default();
        opus.apply_named("R128_ALBUM_GAIN", "512");
        assert_eq!(opus.loudness.album_gain_db, Some(7.0));
        assert_eq!(parse_replay_gain_db("loud"), None);
        assert_eq!(parse_replay_gain_db("-99 dB"), None);
        assert_eq!(parse_replay_gain_peak("0"), None);
        assert_eq!(r128_to_replay_gain_db("99999"), None);
    }

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
    fn performer_conductor_ensemble_and_artists_tags_are_performance_credits() {
        let mut credits = TagCredits::default();
        assert!(credits.apply_named("PERFORMER", "Martha Argerich (piano)"));
        assert!(credits.apply_named("CONDUCTOR", "Claudio Abbado"));
        assert!(credits.apply_named("ORCHESTRA", "Berliner Philharmoniker"));
        assert!(credits.apply_named("ENSEMBLE", "Arditti Quartet"));
        assert!(credits.apply_named("ARTISTS", "Nora Kelly; Abe Rounds"));
        assert!(credits.apply_id3_frame("TPE3", "Claudio Abbado"));
        // Taggers that put soloists in TPE3 write "Name (instrument)".
        assert!(credits.apply_id3_frame("TPE3", "Hilary Hahn (violin)"));
        assert!(credits.apply_id3_frame("TPE3", "Simon Rattle (conductor)"));
        let roles = credits
            .credits
            .iter()
            .map(|credit| (credit.role_key(), credit.name.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            roles,
            [
                ("performer:piano".to_string(), "Martha Argerich"),
                ("conductor".to_string(), "Claudio Abbado"),
                ("ensemble".to_string(), "Berliner Philharmoniker"),
                ("ensemble".to_string(), "Arditti Quartet"),
                ("performer".to_string(), "Nora Kelly"),
                ("performer".to_string(), "Abe Rounds"),
                ("performer:violin".to_string(), "Hilary Hahn"),
                ("conductor".to_string(), "Simon Rattle"),
            ]
        );
    }

    #[test]
    fn work_ids_and_composition_dates_are_read_and_validated() {
        let mut credits = TagCredits::default();
        assert!(credits.apply_named("MUSICBRAINZ_WORKID", "not-a-uuid"));
        assert!(credits.apply_named(
            "MusicBrainz Work Id",
            "1F9DF192-A621-4E4D-9A53-9A0E8B8A4B1E"
        ));
        assert!(credits.apply_named("COMPOSITIONDATE", "1808-12-22"));
        assert_eq!(
            credits.work_id.as_deref(),
            Some("1f9df192-a621-4e4d-9a53-9a0e8b8a4b1e")
        );
        assert_eq!(credits.composed_year, Some(1808));
        assert_eq!(parse_composed_year("c. 1720–1723"), Some(1720));
        assert_eq!(parse_composed_year("Op. 12345"), None);
        assert_eq!(parse_composed_year("BWV 988"), None);
    }

    #[test]
    fn work_and_movement_tags_are_read_from_every_format() {
        let mut vorbis = TagCredits::default();
        assert!(vorbis.apply_named("WORK", "Symphony No. 5 in C minor, Op. 67"));
        assert!(vorbis.apply_named("MOVEMENTNAME", "Allegro con brio"));
        assert!(vorbis.apply_named("MOVEMENT", "1"));
        assert_eq!(
            vorbis.work.as_deref(),
            Some("Symphony No. 5 in C minor, Op. 67")
        );
        assert_eq!(vorbis.movement.as_deref(), Some("Allegro con brio"));
        assert_eq!(vorbis.movement_number, Some(1));

        let mut id3 = TagCredits::default();
        assert!(id3.apply_named("WORK", "Goldberg Variations"));
        assert!(id3.apply_id3_frame("MVNM", "Aria"));
        assert!(id3.apply_id3_frame("MVIN", "01/32"));
        assert_eq!(id3.movement_number, Some(1));
        assert_eq!(parse_movement_number("0"), None);
        assert_eq!(parse_movement_number("IV"), None);
    }

    #[test]
    fn release_types_come_only_from_musicbrainz_values() {
        assert_eq!(normalize_release_type("Album").as_deref(), Some("album"));
        assert_eq!(normalize_release_type("EP").as_deref(), Some("ep"));
        assert_eq!(
            normalize_release_type("album; live").as_deref(),
            Some("album")
        );
        assert_eq!(
            normalize_release_type("Compilation").as_deref(),
            Some("compilation")
        );
        assert_eq!(
            normalize_release_type("Spoken Word").as_deref(),
            Some("spokenword")
        );
        assert_eq!(normalize_release_type("deluxe"), None);
        assert_eq!(normalize_release_type(""), None);

        let mut credits = TagCredits::default();
        assert!(credits.apply_named("MusicBrainz Album Type", "single"));
        assert!(credits.apply_named("RELEASETYPE", "album"));
        assert_eq!(credits.release_type.as_deref(), Some("single"));
        let mut untagged = TagCredits::default();
        untagged.apply_named("COMPOSER", "Someone");
        assert_eq!(untagged.release_type, None);
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
