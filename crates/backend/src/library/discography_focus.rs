//! Focus: stackable facet filters over a person's discography or a work's
//! recordings ("Beethoven, conducted by Karajan, with the Philharmonia,
//! 24-bit only").
//!
//! Every album (or recording) becomes a [`FocusRecord`]: the values it has
//! in each facet group. A selection keeps a record when, in every group
//! with a chosen value, the record has at least one of the chosen values:
//! values in one group widen (OR), groups narrow (AND). Each group's
//! counts are taken over the records the *other* groups keep, so a chip
//! always says how many albums choosing it would show.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::domain::{AudioProperties, Song};

/// Facet groups in display order, with their labels.
pub const GROUPS: [(&str, &str); 13] = [
    ("role", "Their role"),
    ("performer", "Performer"),
    ("conductor", "Conductor"),
    ("ensemble", "Ensemble"),
    ("soloist", "Soloist"),
    ("instrument", "Instrument"),
    ("composer", "Composer"),
    ("form", "Form"),
    ("period", "Period"),
    ("quality", "Quality"),
    ("bit_depth", "Bit depth"),
    ("sample_rate", "Sample rate"),
    ("source", "Source"),
];

const MAX_VALUES_PER_GROUP: usize = 40;
const MAX_SELECTED_PER_GROUP: usize = 32;
const MAX_KEY_CHARS: usize = 200;

/// A value a record has in a group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetValue {
    pub key: String,
    pub label: String,
    /// The person's page, for people groups.
    pub artist_id: Option<String>,
}

/// The facet values of one album or recording.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusRecord {
    values: BTreeMap<&'static str, Vec<FacetValue>>,
}

fn group_key(group: &str) -> Option<&'static str> {
    GROUPS
        .iter()
        .find(|(key, _)| *key == group)
        .map(|(key, _)| *key)
}

impl FocusRecord {
    /// Adds a value; a key already in the group is kept once.
    pub fn add(&mut self, group: &str, key: &str, label: &str, artist_id: Option<&str>) {
        let Some(group) = group_key(group) else {
            return;
        };
        let key = key.trim();
        if key.is_empty() || label.trim().is_empty() {
            return;
        }
        let values = self.values.entry(group).or_default();
        if values.iter().any(|value| value.key == key) {
            return;
        }
        values.push(FacetValue {
            key: key.to_string(),
            label: label.trim().to_string(),
            artist_id: artist_id.map(str::to_string),
        });
    }

    /// Adds a person by their (canonical) id, or by name when they have no
    /// page.
    pub fn add_person(&mut self, group: &str, artist_id: Option<&str>, name: &str) {
        match artist_id.filter(|id| !id.is_empty()) {
            Some(id) => self.add(group, id, name, Some(id)),
            None => {
                let key = format!(
                    "name:{}",
                    crate::library::identity::normalize_artist_identity(name)
                );
                self.add(group, &key, name, None);
            }
        }
    }

    /// Adds the format and source facets of a set of tracks.
    pub fn add_tracks<'a>(&mut self, songs: impl IntoIterator<Item = &'a Song>) {
        for song in songs {
            self.add_track(song.audio.as_ref(), &song.path);
        }
    }

    /// Adds one track's format and source.
    pub fn add_track(&mut self, audio: Option<&AudioProperties>, path: &str) {
        if let Some(audio) = audio {
            self.add_audio(audio);
        }
        let source = song_source(path);
        self.add("source", &source, &source_label(&source), None);
    }

    pub fn add_audio(&mut self, audio: &AudioProperties) {
        match quality(audio) {
            Some(Quality::HiRes) => {
                self.add("quality", "hires", "Hi-Res", None);
                self.add("quality", "lossless", "Lossless", None);
            }
            Some(Quality::Lossless) => self.add("quality", "lossless", "Lossless", None),
            Some(Quality::Lossy) => self.add("quality", "lossy", "Lossy", None),
            None => {}
        }
        if let Some(bits) = audio.bit_depth.filter(|bits| *bits > 0)
            && audio.lossless != Some(false)
        {
            self.add("bit_depth", &bits.to_string(), &format!("{bits}-bit"), None);
        }
        if let Some(rate) = audio.sample_rate.filter(|rate| *rate > 0) {
            self.add(
                "sample_rate",
                &rate.to_string(),
                &sample_rate_label(rate),
                None,
            );
        }
    }

    pub fn values(&self, group: &str) -> &[FacetValue] {
        self.values
            .get(group)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn has_any(&self, group: &str, keys: &BTreeSet<String>) -> bool {
        self.values(group)
            .iter()
            .any(|value| keys.contains(&value.key))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    /// Lossless at more than CD resolution: 24-bit or above 48 kHz.
    HiRes,
    Lossless,
    Lossy,
}

pub fn quality(audio: &AudioProperties) -> Option<Quality> {
    match audio.lossless? {
        false => Some(Quality::Lossy),
        true if audio.bit_depth.is_some_and(|bits| bits >= 24)
            || audio.sample_rate.is_some_and(|rate| rate > 48_000) =>
        {
            Some(Quality::HiRes)
        }
        true => Some(Quality::Lossless),
    }
}

/// "44.1 kHz", "96 kHz", "22.05 kHz".
pub fn sample_rate_label(rate: u32) -> String {
    if rate.is_multiple_of(1000) {
        return format!("{} kHz", rate / 1000);
    }
    let text = format!("{:.2}", f64::from(rate) / 1000.0);
    format!("{} kHz", text.trim_end_matches('0').trim_end_matches('.'))
}

/// Where a track plays from: `local` for a file, or the scheme of a
/// streaming item's URI (`tidal://...` is `tidal`).
pub fn song_source(path: &str) -> String {
    match path.split_once("://") {
        Some((scheme, _))
            if !scheme.is_empty()
                && scheme.len() <= 24
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-')
                && scheme != "file" =>
        {
            scheme.to_ascii_lowercase()
        }
        _ => "local".to_string(),
    }
}

pub fn source_label(source: &str) -> String {
    match source {
        "local" => "Local files".to_string(),
        "tidal" => "TIDAL".to_string(),
        "qobuz" => "Qobuz".to_string(),
        other => {
            let mut characters = other.chars();
            match characters.next() {
                Some(first) => first.to_uppercase().chain(characters).collect(),
                None => String::new(),
            }
        }
    }
}

/// Whether a billed artist name is an orchestra, choir or chamber group,
/// for albums whose tags name the ensemble only as a track artist.
pub fn looks_like_ensemble(name: &str) -> bool {
    // Word stems that only name groups ("Philharmoniker", "Orchestre").
    const STEMS: [&str; 12] = [
        "orchest",
        "orquest",
        "philharmon",
        "filarmon",
        "symphon",
        "sinfon",
        "ensemble",
        "quartet",
        "quatuor",
        "kapelle",
        "consort",
        "choir",
    ];
    // Whole words that do.
    const WORDS: [&str; 8] = [
        "trio", "quintet", "chor", "chorus", "chœur", "choeur", "singers", "players",
    ];
    let lower = name.to_lowercase();
    lower.contains("academy of")
        || lower
            .split(|c: char| !c.is_alphanumeric() && c != 'œ')
            .filter(|word| !word.is_empty())
            .any(|word| {
                STEMS.iter().any(|stem| word.starts_with(stem))
                    || WORDS.contains(&word)
                    || word.ends_with("orchester")
                    || word.ends_with("kapelle")
            })
}

/// Chosen values per group.
pub type Selection = BTreeMap<&'static str, BTreeSet<String>>;

/// Reads a selection from query pairs (`conductor=<id>&bit_depth=24`).
/// Unknown groups, empty values and oversized input are ignored.
pub fn parse_selection(pairs: &[(String, String)]) -> Selection {
    let mut selection = Selection::new();
    for (group, value) in pairs {
        let Some(group) = group_key(group) else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() || value.chars().count() > MAX_KEY_CHARS {
            continue;
        }
        let values = selection.entry(group).or_default();
        if values.len() < MAX_SELECTED_PER_GROUP {
            values.insert(value.to_string());
        }
    }
    selection.retain(|_, values| !values.is_empty());
    selection
}

/// Whether a record passes every chosen group except `except`.
pub fn matches(record: &FocusRecord, selection: &Selection, except: Option<&str>) -> bool {
    selection
        .iter()
        .filter(|(group, _)| Some(**group) != except)
        .all(|(group, keys)| record.has_any(group, keys))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FocusChip {
    pub key: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist_id: Option<String>,
    /// Records this chip would show with the other groups as chosen.
    pub count: usize,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FocusGroup {
    pub key: &'static str,
    pub label: &'static str,
    pub values: Vec<FocusChip>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FocusSummary {
    /// Records before filtering.
    pub total: usize,
    /// Records the selection keeps.
    pub matched: usize,
    /// Groups worth showing: those with a value that narrows or is chosen.
    pub groups: Vec<FocusGroup>,
    /// Chosen values, including any no record has.
    pub selected: Vec<FocusChip>,
}

fn chip_order(group: &str, left: &FocusChip, right: &FocusChip) -> std::cmp::Ordering {
    let number = |chip: &FocusChip| chip.key.parse::<u64>().unwrap_or(0);
    match group {
        "quality" => {
            let rank = |chip: &FocusChip| match chip.key.as_str() {
                "hires" => 0,
                "lossless" => 1,
                _ => 2,
            };
            rank(left).cmp(&rank(right))
        }
        "bit_depth" | "sample_rate" => number(right).cmp(&number(left)),
        _ => right
            .count
            .cmp(&left.count)
            .then_with(|| left.label.to_lowercase().cmp(&right.label.to_lowercase())),
    }
}

/// Filters records and counts each group's values.
pub fn focus(records: &[FocusRecord], selection: &Selection) -> (Vec<usize>, FocusSummary) {
    let matched = records
        .iter()
        .enumerate()
        .filter(|(_, record)| matches(record, selection, None))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let mut groups = Vec::new();
    let mut selected_chips = Vec::new();
    for (group, label) in GROUPS {
        let chosen = selection.get(group);
        let mut chips: Vec<FocusChip> = Vec::new();
        let mut everywhere = 0usize;
        for record in records
            .iter()
            .filter(|record| matches(record, selection, Some(group)))
        {
            everywhere += 1;
            for value in record.values(group) {
                match chips.iter_mut().find(|chip| chip.key == value.key) {
                    Some(chip) => chip.count += 1,
                    None => chips.push(FocusChip {
                        key: value.key.clone(),
                        label: value.label.clone(),
                        artist_id: value.artist_id.clone(),
                        count: 1,
                        selected: chosen.is_some_and(|keys| keys.contains(&value.key)),
                    }),
                }
            }
        }
        // A chosen value no record has any more still shows, so it can be
        // turned off.
        for key in chosen.into_iter().flatten() {
            if !chips.iter().any(|chip| &chip.key == key) {
                let label = records
                    .iter()
                    .flat_map(|record| record.values(group))
                    .find(|value| &value.key == key)
                    .map(|value| value.label.clone())
                    .unwrap_or_else(|| key.trim_start_matches("name:").to_string());
                chips.push(FocusChip {
                    key: key.clone(),
                    label,
                    artist_id: None,
                    count: 0,
                    selected: true,
                });
            }
        }
        chips.sort_by(|left, right| chip_order(group, left, right));
        selected_chips.extend(chips.iter().filter(|chip| chip.selected).cloned().map(
            |mut chip| {
                chip.key = format!("{group}={}", chip.key);
                chip
            },
        ));
        // A group where every value is on every album cannot narrow.
        let narrows = chips
            .iter()
            .any(|chip| chip.selected || chip.count < everywhere);
        if chips.is_empty() || !narrows {
            continue;
        }
        let mut shown = chips
            .iter()
            .take(MAX_VALUES_PER_GROUP)
            .cloned()
            .collect::<Vec<_>>();
        shown.extend(
            chips
                .iter()
                .skip(MAX_VALUES_PER_GROUP)
                .filter(|chip| chip.selected)
                .cloned(),
        );
        groups.push(FocusGroup {
            key: group,
            label,
            values: shown,
        });
    }
    let summary = FocusSummary {
        total: records.len(),
        matched: matched.len(),
        groups,
        selected: selected_chips,
    };
    (matched, summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio(codec: &str, bits: Option<u8>, rate: u32) -> AudioProperties {
        AudioProperties {
            codec: Some(codec.into()),
            sample_rate: Some(rate),
            bit_depth: bits,
            lossless: crate::domain::codec_is_lossless(codec),
            ..AudioProperties::default()
        }
    }

    fn record(
        conductor: &str,
        ensemble: &str,
        composer: &str,
        format: AudioProperties,
    ) -> FocusRecord {
        let mut record = FocusRecord::default();
        record.add_person("conductor", Some(conductor), conductor);
        record.add_person("ensemble", Some(ensemble), ensemble);
        record.add_person("composer", Some(composer), composer);
        record.add_audio(&format);
        record.add("source", "local", "Local files", None);
        record
    }

    fn library() -> Vec<FocusRecord> {
        vec![
            // 0: Karajan, Philharmonia, Beethoven, 24/96
            record(
                "karajan",
                "philharmonia",
                "beethoven",
                audio("flac", Some(24), 96_000),
            ),
            // 1: Karajan, Philharmonia, Beethoven, 16/44.1
            record(
                "karajan",
                "philharmonia",
                "beethoven",
                audio("flac", Some(16), 44_100),
            ),
            // 2: Karajan, Berlin, Beethoven, 24/192
            record(
                "karajan",
                "berlin",
                "beethoven",
                audio("flac", Some(24), 192_000),
            ),
            // 3: Kleiber, Vienna, Beethoven, 24/96
            record(
                "kleiber",
                "vienna",
                "beethoven",
                audio("alac", Some(24), 96_000),
            ),
            // 4: Karajan, Philharmonia, Brahms, 24/96
            record(
                "karajan",
                "philharmonia",
                "brahms",
                audio("flac", Some(24), 96_000),
            ),
            // 5: Karajan, Philharmonia, Beethoven, MP3
            record(
                "karajan",
                "philharmonia",
                "beethoven",
                audio("mp3", None, 44_100),
            ),
        ]
    }

    fn select(pairs: &[(&str, &str)]) -> Selection {
        parse_selection(
            &pairs
                .iter()
                .map(|(group, value)| (group.to_string(), value.to_string()))
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn groups_stack_and_values_in_a_group_widen() {
        let records = library();
        // Beethoven, conducted by Karajan, with the Philharmonia, 24-bit.
        let (matched, summary) = focus(
            &records,
            &select(&[
                ("composer", "beethoven"),
                ("conductor", "karajan"),
                ("ensemble", "philharmonia"),
                ("bit_depth", "24"),
            ]),
        );
        assert_eq!(matched, vec![0]);
        assert_eq!(summary.matched, 1);
        assert_eq!(summary.total, 6);
        // Two ensembles widen within the group.
        let (matched, _) = focus(
            &records,
            &select(&[
                ("conductor", "karajan"),
                ("ensemble", "philharmonia"),
                ("ensemble", "berlin"),
                ("bit_depth", "24"),
            ]),
        );
        assert_eq!(matched, vec![0, 2, 4]);
    }

    #[test]
    fn hi_res_only_means_lossless_above_cd_resolution() {
        let records = library();
        let (matched, _) = focus(&records, &select(&[("quality", "hires")]));
        assert_eq!(matched, vec![0, 2, 3, 4]);
        // Lossless includes hi-res but not the MP3.
        let (matched, _) = focus(&records, &select(&[("quality", "lossless")]));
        assert_eq!(matched, vec![0, 1, 2, 3, 4]);
        let (matched, _) = focus(&records, &select(&[("sample_rate", "192000")]));
        assert_eq!(matched, vec![2]);
        // 24-bit lossy does not exist, and lossy files have no bit depth.
        assert!(records[5].values("bit_depth").is_empty());
    }

    #[test]
    fn counts_say_what_each_chip_would_show() {
        let records = library();
        let (_, summary) = focus(
            &records,
            &select(&[("conductor", "karajan"), ("bit_depth", "24")]),
        );
        let group = |key: &str| {
            summary
                .groups
                .iter()
                .find(|group| group.key == key)
                .unwrap_or_else(|| panic!("no {key} group"))
        };
        // Conductor counts ignore the conductor choice: Kleiber would add 1.
        let conductors = group("conductor");
        let count = |group: &FocusGroup, key: &str| {
            group
                .values
                .iter()
                .find(|chip| chip.key == key)
                .map(|chip| (chip.count, chip.selected))
        };
        assert_eq!(count(conductors, "karajan"), Some((3, true)));
        assert_eq!(count(conductors, "kleiber"), Some((1, false)));
        // Ensemble counts follow both choices.
        let ensembles = group("ensemble");
        assert_eq!(count(ensembles, "philharmonia"), Some((2, false)));
        assert_eq!(count(ensembles, "berlin"), Some((1, false)));
        assert_eq!(count(ensembles, "vienna"), None);
        // Bit depths sort highest first.
        let bits = group("bit_depth");
        assert_eq!(bits.values[0].key, "24");
        assert_eq!(bits.values[0].label, "24-bit");
        // A group whose one value covers everything is left out.
        assert!(summary.groups.iter().all(|group| group.key != "source"));
        assert_eq!(
            summary
                .selected
                .iter()
                .map(|chip| chip.key.as_str())
                .collect::<Vec<_>>(),
            vec!["conductor=karajan", "bit_depth=24"]
        );
    }

    #[test]
    fn a_chosen_value_nobody_has_stays_visible() {
        let records = library();
        let (matched, summary) = focus(&records, &select(&[("conductor", "bernstein")]));
        assert!(matched.is_empty());
        let conductors = summary
            .groups
            .iter()
            .find(|group| group.key == "conductor")
            .unwrap();
        assert!(
            conductors
                .values
                .iter()
                .any(|chip| chip.key == "bernstein" && chip.selected && chip.count == 0)
        );
    }

    #[test]
    fn selections_ignore_unknown_groups_and_empty_values() {
        let selection = select(&[
            ("conductor", " karajan "),
            ("nonsense", "x"),
            ("ensemble", ""),
            ("bit_depth", "24"),
            ("bit_depth", "24"),
        ]);
        assert_eq!(selection.len(), 2);
        assert_eq!(selection["conductor"].len(), 1);
        assert!(selection["conductor"].contains("karajan"));
    }

    #[test]
    fn formats_and_sources_have_readable_labels() {
        assert_eq!(sample_rate_label(44_100), "44.1 kHz");
        assert_eq!(sample_rate_label(22_050), "22.05 kHz");
        assert_eq!(sample_rate_label(96_000), "96 kHz");
        assert_eq!(sample_rate_label(176_400), "176.4 kHz");
        assert_eq!(song_source("/music/Miles Davis/So What.flac"), "local");
        assert_eq!(song_source("C:\\Music\\a.flac"), "local");
        assert_eq!(song_source("file:///music/a.flac"), "local");
        assert_eq!(song_source("tidal://track/12345"), "tidal");
        assert_eq!(source_label("tidal"), "TIDAL");
        assert_eq!(source_label("local"), "Local files");
        assert_eq!(
            quality(&audio("flac", Some(16), 48_000)),
            Some(Quality::Lossless)
        );
        assert_eq!(
            quality(&audio("pcm", Some(16), 88_200)),
            Some(Quality::HiRes)
        );
        assert_eq!(quality(&audio("opus", None, 48_000)), Some(Quality::Lossy));
    }

    #[test]
    fn ensembles_are_recognised_by_name() {
        for name in [
            "Berliner Philharmoniker",
            "Philharmonia Orchestra",
            "Wiener Philharmoniker",
            "Academy of St Martin in the Fields",
            "Emerson String Quartet",
            "Monteverdi Choir",
            "Staatskapelle Dresden",
            "Orchestre de Paris",
        ] {
            assert!(looks_like_ensemble(name), "{name}");
        }
        for name in ["Herbert von Karajan", "Miles Davis", "Chorley", "Triology"] {
            assert!(!looks_like_ensemble(name), "{name}");
        }
    }

    #[test]
    fn people_without_a_page_are_keyed_by_name() {
        let mut record = FocusRecord::default();
        record.add_person("soloist", None, "Wynton Kelly");
        record.add_person("soloist", None, "Wynton  Kelly");
        assert_eq!(record.values("soloist").len(), 1);
        assert!(record.values("soloist")[0].key.starts_with("name:"));
        record.add("nonsense", "x", "X", None);
        assert!(record.values("nonsense").is_empty());
    }
}
