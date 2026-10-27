//! Album versions: remasters, deluxe editions and other releases of one
//! album, grouped so the library can show one preferred version.

use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

use regex::Regex;
use serde::Serialize;

use super::duplicates::group_key;
use super::quality::Quality;
use crate::library::identity::{normalize_album_identity, normalize_artist_identity};

#[derive(Clone, Debug)]
pub struct AlbumFacts {
    pub album_id: String,
    pub title: String,
    pub artist_id: String,
    pub artist_name: String,
    pub year: String,
    pub release_group_id: Option<String>,
    pub track_count: usize,
    pub duration: f64,
    /// Quality most of the album's tracks have.
    pub quality: Quality,
    /// Whether the tracks differ in quality.
    pub mixed: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct AlbumVersion {
    pub album_id: String,
    /// e.g. "2011 Remaster", "Deluxe Edition", or "Original".
    pub edition: String,
    pub preferred: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct VersionGroup {
    pub key: String,
    pub title: String,
    pub artist_name: String,
    /// Best version first.
    pub versions: Vec<AlbumVersion>,
    pub preferred: String,
    /// Whether the preferred version was chosen by the user.
    pub chosen: bool,
    pub hide_others: bool,
    /// Grouped by hand rather than by title or release group.
    pub manual: bool,
}

/// A stored preference for one group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionChoice {
    pub preferred_album_id: String,
    pub hide_others: bool,
}

static EDITION_SUFFIX: OnceLock<Regex> = OnceLock::new();

const EDITION_WORDS: &str = r"remaster(?:ed)?|deluxe|expanded|anniversary|edition|special|bonus|reissue|collector'?s|legacy|hi[-\s]?res|\d{2}[-\s]?bit|\d{2}\s*[-/]\s*\d{2,3}|\d{2,3}\s*k(?:hz)?|mfsl|sacd|mobile\s+fidelity|audiophile|super\s+audio|definitive|complete|version|mono|stereo|mix";

/// Splits an album title into its base title and the edition it names,
/// e.g. "Abbey Road (Super Deluxe Edition)" into ("Abbey Road", "Super
/// Deluxe Edition").
pub fn split_edition(title: &str) -> (String, String) {
    let pattern = EDITION_SUFFIX.get_or_init(|| {
        Regex::new(&format!(
            r"(?ix)\s*(?:
                [\(\[\{{]\s*(?P<bracketed>[^\)\]\}}]*?\b(?:{EDITION_WORDS})\b[^\)\]\}}]*?)\s*[\)\]\}}]
              | \s[-–:]\s(?P<dashed>[^-–:]*?\b(?:{EDITION_WORDS})\b[^-–:]*)
            )\s*$"
        ))
        .expect("edition pattern")
    });
    let mut base = title.trim().to_string();
    let mut editions = Vec::new();
    while let Some(captures) = pattern.captures(&base) {
        let edition = captures
            .name("bracketed")
            .or_else(|| captures.name("dashed"))
            .map(|value| value.as_str().trim().to_string())
            .unwrap_or_default();
        let start = captures.get(0).map_or(base.len(), |whole| whole.start());
        let next = base[..start].trim().to_string();
        if next.is_empty() {
            break;
        }
        editions.insert(0, edition);
        base = next;
    }
    (base, editions.join(" · "))
}

/// Groups versions. `manual` maps an album to its hand-made group, or to
/// `None` when the user took it out of every group.
pub fn group_versions(
    albums: &[AlbumFacts],
    manual: &HashMap<String, Option<String>>,
    choices: &HashMap<String, VersionChoice>,
) -> Vec<VersionGroup> {
    let mut groups: BTreeMap<String, (bool, Vec<usize>)> = BTreeMap::new();
    let mut automatic: Vec<usize> = Vec::new();
    for (index, album) in albums.iter().enumerate() {
        match manual.get(&album.album_id) {
            Some(Some(key)) => groups
                .entry(key.clone())
                .or_insert((true, Vec::new()))
                .1
                .push(index),
            Some(None) => {}
            None => automatic.push(index),
        }
    }

    // Automatic: same artist and base title, or same release group.
    let mut parent: Vec<usize> = (0..albums.len()).collect();
    fn find(parent: &mut [usize], index: usize) -> usize {
        let mut root = index;
        while parent[root] != root {
            root = parent[root];
        }
        parent[index] = root;
        root
    }
    let mut first_by_title: HashMap<(String, String), usize> = HashMap::new();
    let mut first_by_release_group: HashMap<String, usize> = HashMap::new();
    let mut base_titles: HashMap<usize, String> = HashMap::new();
    for &index in &automatic {
        let album = &albums[index];
        let (base, _) = split_edition(&album.title);
        let base = normalize_album_identity(&base);
        base_titles.insert(index, base.clone());
        let artist = normalize_artist_identity(&album.artist_name);
        let mut links = Vec::new();
        if !base.is_empty() {
            match first_by_title.get(&(artist.clone(), base.clone())) {
                Some(first) => links.push(*first),
                None => {
                    first_by_title.insert((artist, base), index);
                }
            }
        }
        if let Some(release_group) = album
            .release_group_id
            .as_deref()
            .filter(|id| !id.is_empty())
        {
            match first_by_release_group.get(release_group) {
                Some(first) => links.push(*first),
                None => {
                    first_by_release_group.insert(release_group.to_string(), index);
                }
            }
        }
        for link in links {
            let (left, right) = (find(&mut parent, index), find(&mut parent, link));
            if left != right {
                parent[left.max(right)] = left.min(right);
            }
        }
    }
    let mut automatic_sets: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for &index in &automatic {
        let root = find(&mut parent, index);
        automatic_sets.entry(root).or_default().push(index);
    }
    for members in automatic_sets.into_values() {
        if members.len() < 2 {
            continue;
        }
        let artist = normalize_artist_identity(&albums[members[0]].artist_name);
        let base = members
            .iter()
            .filter_map(|index| base_titles.get(index))
            .min()
            .cloned()
            .unwrap_or_default();
        let key = format!("auto-{}", group_key(&[artist.as_str(), base.as_str()]));
        groups.insert(key, (false, members));
    }

    let mut result = Vec::new();
    for (key, (manual_group, mut members)) in groups {
        if members.len() < 2 {
            continue;
        }
        members.sort_by(|a, b| {
            let (a, b) = (&albums[*a], &albums[*b]);
            b.quality
                .score
                .cmp(&a.quality.score)
                .then(a.mixed.cmp(&b.mixed))
                .then(b.track_count.cmp(&a.track_count))
                .then(a.year.cmp(&b.year))
                .then(a.album_id.cmp(&b.album_id))
        });
        let choice = choices.get(&key).filter(|choice| {
            members
                .iter()
                .any(|m| albums[*m].album_id == choice.preferred_album_id)
        });
        let preferred = choice
            .map(|choice| choice.preferred_album_id.clone())
            .unwrap_or_else(|| albums[members[0]].album_id.clone());
        let years: Vec<&str> = members.iter().map(|m| albums[*m].year.as_str()).collect();
        let versions = members
            .iter()
            .map(|m| {
                let album = &albums[*m];
                let (_, edition) = split_edition(&album.title);
                let edition = if !edition.is_empty() {
                    edition
                } else if !album.year.is_empty()
                    && years.iter().filter(|year| **year == album.year).count() == 1
                {
                    format!("{} release", album.year)
                } else {
                    "Original".to_string()
                };
                AlbumVersion {
                    album_id: album.album_id.clone(),
                    edition,
                    preferred: album.album_id == preferred,
                }
            })
            .collect();
        let preferred_album = members
            .iter()
            .map(|m| &albums[*m])
            .find(|album| album.album_id == preferred)
            .expect("the preferred version is a member");
        result.push(VersionGroup {
            key,
            title: split_edition(&preferred_album.title).0,
            artist_name: preferred_album.artist_name.clone(),
            versions,
            preferred,
            chosen: choice.is_some(),
            hide_others: choice.is_some_and(|choice| choice.hide_others),
            manual: manual_group,
        });
    }
    result.sort_by(|a, b| {
        a.artist_name
            .to_lowercase()
            .cmp(&b.artist_name.to_lowercase())
            .then(a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::AudioProperties;
    use crate::library::tools::quality::grade;

    fn album(id: &str, title: &str, year: &str, codec: &str, rate: u32) -> AlbumFacts {
        let lossless = codec != "mp3";
        AlbumFacts {
            album_id: id.into(),
            title: title.into(),
            artist_id: "artist".into(),
            artist_name: "The Example".into(),
            year: year.into(),
            release_group_id: None,
            track_count: 10,
            duration: 2400.0,
            quality: grade(
                Some(&AudioProperties {
                    codec: Some(codec.into()),
                    sample_rate: Some(rate),
                    bit_depth: lossless.then_some(if rate > 48_000 { 24 } else { 16 }),
                    bitrate_kbps: Some(320),
                    lossless: Some(lossless),
                    ..Default::default()
                }),
                codec,
            ),
            mixed: false,
        }
    }

    #[test]
    fn editions_are_split_from_base_titles() {
        assert_eq!(
            split_edition("Abbey Road (Super Deluxe Edition)"),
            ("Abbey Road".into(), "Super Deluxe Edition".into())
        );
        assert_eq!(
            split_edition("Kind of Blue [2009 Remaster] [24-96]"),
            ("Kind of Blue".into(), "2009 Remaster · 24-96".into())
        );
        assert_eq!(
            split_edition("Rumours - 2004 Remaster"),
            ("Rumours".into(), "2004 Remaster".into())
        );
        // Titles that are not editions stay whole.
        assert_eq!(split_edition("Live at Leeds").0, "Live at Leeds");
        assert_eq!(split_edition("Help!").1, "");
    }

    #[test]
    fn remasters_group_with_the_highest_quality_version_preferred() {
        let albums = vec![
            album("cd", "Fixture Album", "1999", "flac", 44_100),
            album(
                "hires",
                "Fixture Album (2019 Remaster)",
                "2019",
                "flac",
                96_000,
            ),
            album(
                "mp3",
                "Fixture Album [Deluxe Edition]",
                "2009",
                "mp3",
                44_100,
            ),
            album("other", "Another Album", "2001", "flac", 44_100),
        ];
        let groups = group_versions(&albums, &HashMap::new(), &HashMap::new());
        assert_eq!(groups.len(), 1);
        let group = &groups[0];
        assert_eq!(group.preferred, "hires");
        assert_eq!(group.title, "Fixture Album");
        assert_eq!(
            group
                .versions
                .iter()
                .map(|v| v.album_id.as_str())
                .collect::<Vec<_>>(),
            vec!["hires", "cd", "mp3"]
        );
        assert_eq!(group.versions[1].edition, "1999 release");
        assert!(!group.chosen);
    }

    #[test]
    fn choices_and_manual_groups_override_detection() {
        let albums = vec![
            album("cd", "Fixture Album", "1999", "flac", 44_100),
            album(
                "hires",
                "Fixture Album (2019 Remaster)",
                "2019",
                "flac",
                96_000,
            ),
            album("japan", "Fikusucha Arubamu", "2001", "flac", 44_100),
        ];
        let automatic = group_versions(&albums, &HashMap::new(), &HashMap::new());
        let key = automatic[0].key.clone();
        let choices = HashMap::from([(
            key.clone(),
            VersionChoice {
                preferred_album_id: "cd".into(),
                hide_others: true,
            },
        )]);
        let chosen = group_versions(&albums, &HashMap::new(), &choices);
        assert_eq!(chosen[0].preferred, "cd");
        assert!(chosen[0].hide_others && chosen[0].chosen);

        // The Japanese title joins by hand; the remaster leaves the group.
        let manual = HashMap::from([
            ("cd".to_string(), Some("manual-1".to_string())),
            ("japan".to_string(), Some("manual-1".to_string())),
            ("hires".to_string(), None),
        ]);
        let grouped = group_versions(&albums, &manual, &HashMap::new());
        assert_eq!(grouped.len(), 1);
        assert!(grouped[0].manual);
        assert_eq!(grouped[0].versions.len(), 2);
    }
}
