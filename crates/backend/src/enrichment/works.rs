//! Works (compositions) from MusicBrainz recording-to-work relationships.
//!
//! A recording is a performance of a work; a movement is a work that is part
//! of a larger work. Parson keeps both so a track can lead to the piece and
//! the piece to every recording of it in the library. Form, key and
//! catalogue number come from MusicBrainz work types and attributes when
//! present, otherwise from the work title ("Symphony no. 9 in D minor,
//! op. 125").

use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use super::musicbrainz::is_mbid;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkComposer {
    pub name: String,
    pub mbid: Option<String>,
}

/// A work as described by MusicBrainz, already normalised for display.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkInfo {
    pub mbid: String,
    pub title: String,
    /// MusicBrainz work type or a form read from the title, e.g. `Symphony`.
    pub form: Option<String>,
    pub key: Option<String>,
    /// Catalogue number, e.g. `Op. 125` or `BWV 1007`.
    pub catalogue: Option<String>,
    /// Year or range the work was composed in, e.g. `1822–1824`.
    pub composed: Option<String>,
    pub composers: Vec<WorkComposer>,
    pub language: Option<String>,
    /// The larger work this one is part of, e.g. the symphony of a movement.
    pub parent: Option<Box<WorkInfo>>,
    /// Position inside the parent (movement number).
    pub position: Option<i64>,
}

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn regex(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid work regex"))
}

/// MusicBrainz writes "no." and "op." in lower case; display them the way
/// album covers do.
pub fn display_title(title: &str) -> String {
    static WORDS: OnceLock<Regex> = OnceLock::new();
    regex(&WORDS, r"\b(no|op|opp|nos|posth|arr|hob|wo[oO])\.(\s|$|\d)")
        .replace_all(title.trim(), |captures: &regex::Captures| {
            let word = &captures[1];
            let mut characters = word.chars();
            let first = characters.next().map(|c| c.to_ascii_uppercase());
            let rest = characters.as_str();
            let word = match word {
                "woO" | "woo" => "WoO".to_string(),
                _ => format!("{}{rest}", first.unwrap_or_default()),
            };
            format!("{word}.{}", &captures[2])
        })
        .into_owned()
}

/// Catalogue number from a title: Op. 125, BWV 1007, K. 550, D. 960, ...
pub fn catalogue_from_title(title: &str) -> Option<String> {
    static CATALOGUE: OnceLock<Regex> = OnceLock::new();
    let pattern = regex(
        &CATALOGUE,
        r"(?i)(?:^|[\s,(])(op\.?|opus|bwv|kv|k\.|d\.|hob\.?|hwv|rv|woo|s\.|sz\.?|l\.|twv|wab|fs|bb|jw)\s*((?:[ivxlc]+:)?\d+[a-z]?(?:[/:]\d+[a-z]?)?(?:,?\s*no\.?\s*\d+)?)",
    );
    let captures = pattern.captures(title)?;
    let prefix = captures[1].trim_end_matches('.').to_lowercase();
    let prefix = match prefix.as_str() {
        "op" | "opus" => "Op.",
        "bwv" => "BWV",
        "kv" | "k" => "K.",
        "d" => "D.",
        "hob" => "Hob.",
        "hwv" => "HWV",
        "rv" => "RV",
        "woo" => "WoO",
        "s" => "S.",
        "sz" => "Sz.",
        "l" => "L.",
        "twv" => "TWV",
        "wab" => "WAB",
        "fs" => "FS",
        "bb" => "BB",
        "jw" => "JW",
        _ => return None,
    };
    let number = display_title(captures[2].trim());
    Some(format!("{prefix} {number}"))
}

/// Key from a title: "in D minor", "in E-flat major", "in F♯ minor".
pub fn key_from_title(title: &str) -> Option<String> {
    static KEY: OnceLock<Regex> = OnceLock::new();
    let pattern = regex(
        &KEY,
        r"\bin ([A-Ga-g])(?:[\s\-‐‑]?(flat|sharp|♭|♯|b|#))?[\s\-‐‑]+(major|minor|dur|moll)\b",
    );
    let captures = pattern.captures(title)?;
    let note = captures[1].to_uppercase();
    let accidental = match captures.get(2).map(|value| value.as_str()) {
        Some("flat" | "♭" | "b") => "-flat",
        Some("sharp" | "♯" | "#") => "-sharp",
        _ => "",
    };
    let mode = match &captures[3] {
        "minor" | "moll" => "minor",
        _ => "major",
    };
    Some(format!("{note}{accidental} {mode}"))
}

/// Normalises a MusicBrainz "Key" attribute ("E-flat major", "d minor").
pub fn normalize_key(value: &str) -> Option<String> {
    key_from_title(&format!("in {value}")).or_else(|| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

const TITLE_FORMS: &[(&str, &str)] = &[
    ("symphonic poem", "Symphonic poem"),
    ("symphony", "Symphony"),
    ("sinfonia", "Symphony"),
    ("piano concerto", "Concerto"),
    ("violin concerto", "Concerto"),
    ("concerto", "Concerto"),
    ("string quartet", "String quartet"),
    ("quartet", "Quartet"),
    ("quintet", "Quintet"),
    ("trio", "Trio"),
    ("sonata", "Sonata"),
    ("partita", "Partita"),
    ("suite", "Suite"),
    ("overture", "Overture"),
    ("requiem", "Requiem"),
    ("mass", "Mass"),
    ("missa", "Mass"),
    ("cantata", "Cantata"),
    ("oratorio", "Oratorio"),
    ("opera", "Opera"),
    ("étude", "Étude"),
    ("etude", "Étude"),
    ("prelude", "Prelude"),
    ("fugue", "Fugue"),
    ("nocturne", "Nocturne"),
    ("variations", "Variations"),
    ("ballade", "Ballade"),
    ("mazurka", "Mazurka"),
    ("waltz", "Waltz"),
    ("rhapsody", "Rhapsody"),
    ("serenade", "Serenade"),
    ("divertimento", "Divertimento"),
    ("motet", "Motet"),
    ("madrigal", "Madrigal"),
    ("lied", "Song"),
    ("bagatelle", "Bagatelle"),
    ("rondo", "Rondo"),
    ("minuet", "Minuet"),
    ("impromptu", "Impromptu"),
];

/// Form from a title when MusicBrainz gives no work type.
pub fn form_from_title(title: &str) -> Option<String> {
    let lower = title.to_lowercase();
    TITLE_FORMS
        .iter()
        .find(|(word, _)| {
            lower.match_indices(word).any(|(index, _)| {
                let before = lower[..index].chars().next_back();
                let after = lower[index + word.len()..].chars().next();
                !before.is_some_and(char::is_alphanumeric)
                    && !after.is_some_and(|c| c.is_alphanumeric() && c != 's')
            })
        })
        .map(|(_, form)| (*form).to_string())
}

/// MusicBrainz work types, folded to the forms Parson groups by.
pub fn normalize_form(work_type: &str) -> String {
    match work_type.to_lowercase().as_str() {
        "song-cycle" => "Song cycle".to_string(),
        "symphonic poem" => "Symphonic poem".to_string(),
        "incidental music" => "Incidental music".to_string(),
        "etude" => "Étude".to_string(),
        _ => {
            let mut characters = work_type.trim().chars();
            match characters.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), characters.as_str()),
                None => String::new(),
            }
        }
    }
}

/// The first four-digit year in a date or range.
pub fn first_year(value: &str) -> Option<i32> {
    static YEAR: OnceLock<Regex> = OnceLock::new();
    regex(&YEAR, r"\b(\d{4})\b")
        .captures(value)
        .and_then(|captures| captures[1].parse().ok())
}

/// The style period a year falls in. Boundaries follow common usage; the
/// Classical period is taken to run to 1830 so late Beethoven and Schubert
/// stay with it.
pub fn period_for_year(year: i32) -> &'static str {
    match year {
        ..=1399 => "Medieval",
        1400..=1599 => "Renaissance",
        1600..=1749 => "Baroque",
        1750..=1830 => "Classical",
        1831..=1909 => "Romantic",
        1910..=1974 => "Modern",
        _ => "Contemporary",
    }
}

fn composed_range(begin: Option<&str>, end: Option<&str>) -> Option<String> {
    let begin = begin.and_then(first_year);
    let end = end.and_then(first_year);
    match (begin, end) {
        (Some(begin), Some(end)) if begin != end => Some(format!("{begin}–{end}")),
        (Some(year), _) | (None, Some(year)) => Some(year.to_string()),
        _ => None,
    }
}

fn attribute(work: &Value, wanted: impl Fn(&str) -> bool) -> Option<String> {
    array(work, "attributes").iter().find_map(|attribute| {
        let kind = text(attribute, "type")?;
        wanted(&kind.to_lowercase())
            .then(|| text(attribute, "value"))
            .flatten()
    })
}

/// One work object as embedded in a release lookup (with work-level
/// relationships). The parent ("parts", backward) is read one level up.
pub fn parse_work(work: &Value, depth: usize) -> Option<WorkInfo> {
    let mbid = text(work, "id").filter(|id| is_mbid(id))?;
    let raw_title = text(work, "title")?;
    let title = display_title(&raw_title);
    let mut composers = Vec::new();
    let mut begin = None;
    let mut end = None;
    let mut parent = None;
    for relation in array(work, "relations") {
        let kind = relation
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match (relation.get("target-type").and_then(Value::as_str), kind) {
            (Some("artist"), "composer" | "writer") => {
                let Some(artist) = relation.get("artist") else {
                    continue;
                };
                let Some(name) = text(relation, "target-credit").or_else(|| text(artist, "name"))
                else {
                    continue;
                };
                let composer = WorkComposer {
                    name,
                    mbid: text(artist, "id").filter(|id| is_mbid(id)),
                };
                if !composers.contains(&composer) {
                    composers.push(composer);
                }
                begin = begin.or_else(|| text(relation, "begin"));
                end = end.or_else(|| text(relation, "end"));
            }
            (Some("work"), "parts")
                if depth < 3
                    && relation.get("direction").and_then(Value::as_str) == Some("backward") =>
            {
                if let Some(found) = relation
                    .get("work")
                    .and_then(|parent| parse_work(parent, depth + 1))
                {
                    let position = relation
                        .get("ordering-key")
                        .and_then(Value::as_i64)
                        .or_else(|| {
                            relation
                                .get("ordering-key")
                                .and_then(Value::as_str)
                                .and_then(|value| value.parse().ok())
                        });
                    parent = Some((found, position));
                }
            }
            _ => {}
        }
    }
    let form = text(work, "type")
        .map(|kind| normalize_form(&kind))
        .or_else(|| form_from_title(&raw_title));
    let key = attribute(work, |kind| kind == "key")
        .and_then(|value| normalize_key(&value))
        .or_else(|| key_from_title(&raw_title));
    let catalogue = catalogue_from_title(&raw_title).or_else(|| {
        attribute(work, |kind| {
            kind.contains("catalog") || kind.contains("opus")
        })
        .filter(|value| value.chars().any(char::is_alphabetic))
    });
    let (parent, position) = match parent {
        Some((mut parent, position)) => {
            // A parent embedded in a relation carries no composer of its
            // own; the movement's composer wrote the whole work.
            if parent.composers.is_empty() {
                parent.composers = composers.clone();
            }
            if parent.composed.is_none() {
                parent.composed = composed_range(begin.as_deref(), end.as_deref());
            }
            (Some(Box::new(parent)), position)
        }
        None => (None, None),
    };
    Some(WorkInfo {
        mbid,
        title,
        form,
        key,
        catalogue,
        composed: composed_range(begin.as_deref(), end.as_deref()),
        composers,
        language: text(work, "language").filter(|language| language != "zxx"),
        parent,
        position,
    })
}

/// A work as listed in a composer's catalogue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogueWork {
    pub info: WorkInfo,
    /// The composer as this work names them (the catalogue's composer).
    pub composer: WorkComposer,
    /// The work's Wikidata item, from its MusicBrainz URL relationship.
    pub wikidata_id: Option<String>,
}

/// One work from a "browse works by artist" page, when `composer` wrote it.
/// Works the composer only wrote words for, arranged or had dedicated to
/// them are not their compositions, and an arrangement of another work is
/// left out (its original is listed).
pub fn catalogue_work(work: &Value, composer: &str) -> Option<CatalogueWork> {
    let relations = array(work, "relations");
    let arrangement = relations.iter().any(|relation| {
        relation.get("type").and_then(Value::as_str) == Some("arrangement")
            && relation.get("direction").and_then(Value::as_str) == Some("backward")
            && relation.get("target-type").and_then(Value::as_str) == Some("work")
    });
    if arrangement {
        return None;
    }
    let info = parse_work(work, 0)?;
    let named = info
        .composers
        .iter()
        .find(|known| known.mbid.as_deref() == Some(composer))?
        .clone();
    let wikidata_id = relations.iter().find_map(|relation| {
        if relation.get("type").and_then(Value::as_str) != Some("wikidata") {
            return None;
        }
        let resource = relation.get("url").and_then(|url| text(url, "resource"))?;
        let id = resource.rsplit('/').next()?;
        super::musicbrainz::is_wikidata_id(id).then(|| id.to_string())
    });
    Some(CatalogueWork {
        info,
        composer: named,
        wikidata_id,
    })
}

/// The works a recording performs.
pub fn recording_works(recording: &Value) -> Vec<WorkInfo> {
    let mut works: Vec<WorkInfo> = Vec::new();
    for relation in array(recording, "relations") {
        if relation.get("target-type").and_then(Value::as_str) != Some("work")
            || relation.get("type").and_then(Value::as_str) != Some("performance")
        {
            continue;
        }
        if let Some(work) = relation.get("work").and_then(|work| parse_work(work, 0))
            && !works.iter().any(|known| known.mbid == work.mbid)
        {
            works.push(work);
        }
    }
    works
}

/// Movement titles repeat the work: "Symphony No. 9 …: I. Allegro". The
/// part after the work title reads better under a work heading.
pub fn part_title(work_title: &str, part: &str) -> String {
    let trimmed = part
        .strip_prefix(work_title)
        .map(|rest| rest.trim_start_matches([':', ' ', '-', '–', ',']))
        .filter(|rest| !rest.is_empty());
    trimmed.unwrap_or(part).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const MOVEMENT: &str = "11111111-1111-4111-8111-111111111111";
    const SYMPHONY: &str = "22222222-2222-4222-8222-222222222222";
    const BEETHOVEN: &str = "1f9df192-a621-4f54-8850-2c5373b7eac9";

    #[test]
    fn titles_give_catalogue_numbers_keys_and_forms() {
        let title = "Symphony no. 9 in D minor, op. 125 \"Choral\"";
        assert_eq!(catalogue_from_title(title).as_deref(), Some("Op. 125"));
        assert_eq!(key_from_title(title).as_deref(), Some("D minor"));
        assert_eq!(form_from_title(title).as_deref(), Some("Symphony"));
        assert_eq!(
            display_title(title),
            "Symphony No. 9 in D minor, Op. 125 \"Choral\""
        );
        assert_eq!(
            catalogue_from_title("Cello Suite No. 1 in G major, BWV 1007").as_deref(),
            Some("BWV 1007")
        );
        assert_eq!(
            catalogue_from_title("Piano Sonata in B-flat major, D. 960").as_deref(),
            Some("D. 960")
        );
        assert_eq!(
            catalogue_from_title("String Quartet, op. 59 no. 1").as_deref(),
            Some("Op. 59 No. 1")
        );
        assert_eq!(
            key_from_title("Concerto in E-flat major").as_deref(),
            Some("E-flat major")
        );
        assert_eq!(
            key_from_title("Prelude in C# minor").as_deref(),
            Some("C-sharp minor")
        );
        // MusicBrainz often writes a Unicode hyphen.
        assert_eq!(
            key_from_title("Sonata No. 14 in C\u{2010}sharp minor").as_deref(),
            Some("C-sharp minor")
        );
        assert_eq!(catalogue_from_title("So What"), None);
        assert_eq!(key_from_title("So What"), None);
        assert_eq!(form_from_title("Blue in Green"), None);
        // "Mass" inside another word is not a form.
        assert_eq!(form_from_title("Massive Attack"), None);
        assert_eq!(
            form_from_title("Two Piano Sonatas").as_deref(),
            Some("Sonata")
        );
    }

    #[test]
    fn periods_follow_composition_years() {
        assert_eq!(period_for_year(1720), "Baroque");
        assert_eq!(period_for_year(1824), "Classical");
        assert_eq!(period_for_year(1888), "Romantic");
        assert_eq!(period_for_year(1959), "Modern");
        assert_eq!(period_for_year(2001), "Contemporary");
        assert_eq!(first_year("1822-05"), Some(1822));
        assert_eq!(
            composed_range(Some("1822"), Some("1824")).as_deref(),
            Some("1822–1824")
        );
        assert_eq!(composed_range(Some("1959"), None).as_deref(), Some("1959"));
    }

    #[test]
    fn a_movement_leads_to_its_symphony_with_composer_key_and_catalogue() {
        let recording = json!({"relations": [{
            "type": "performance", "target-type": "work",
            "work": {
                "id": MOVEMENT,
                "title": "Symphony no. 9 in D minor, op. 125: IV. Presto – Allegro assai",
                "type": null, "language": "deu",
                "relations": [
                    {"type": "composer", "target-type": "artist", "begin": "1822", "end": "1824",
                     "artist": {"id": BEETHOVEN, "name": "Ludwig van Beethoven"}},
                    {"type": "parts", "target-type": "work", "direction": "backward", "ordering-key": 4,
                     "work": {"id": SYMPHONY, "title": "Symphony no. 9 in D minor, op. 125", "type": "Symphony",
                              "attributes": [{"type": "Key", "value": "D minor"}]}}
                ]
            }
        }, {"type": "instrument", "target-type": "artist", "artist": {"id": BEETHOVEN, "name": "x"}}]});
        let works = recording_works(&recording);
        assert_eq!(works.len(), 1);
        let movement = &works[0];
        assert_eq!(movement.mbid, MOVEMENT);
        assert_eq!(movement.position, Some(4));
        assert_eq!(movement.composed.as_deref(), Some("1822–1824"));
        assert_eq!(movement.language.as_deref(), Some("deu"));
        let symphony = movement.parent.as_deref().unwrap();
        assert_eq!(symphony.mbid, SYMPHONY);
        assert_eq!(symphony.title, "Symphony No. 9 in D minor, Op. 125");
        assert_eq!(symphony.form.as_deref(), Some("Symphony"));
        assert_eq!(symphony.key.as_deref(), Some("D minor"));
        assert_eq!(symphony.catalogue.as_deref(), Some("Op. 125"));
        assert_eq!(symphony.composed.as_deref(), Some("1822–1824"));
        assert_eq!(
            symphony.composers,
            [WorkComposer {
                name: "Ludwig van Beethoven".into(),
                mbid: Some(BEETHOVEN.into())
            }]
        );
        assert_eq!(
            part_title(&symphony.title, &movement.title),
            "IV. Presto – Allegro assai"
        );
    }

    #[test]
    fn a_catalogue_keeps_the_composers_own_works_with_their_wikidata_items() {
        const ODE: &str = "33333333-3333-4333-8333-333333333333";
        let composer = json!({"type": "composer", "target-type": "artist", "direction": "backward",
            "begin": "1804", "end": "1808",
            "artist": {"id": BEETHOVEN, "name": "Ludwig van Beethoven"}});
        let symphony = json!({"id": SYMPHONY, "title": "Symphony no. 5 in C minor, op. 67",
            "type": "Symphony", "relations": [composer.clone(),
                {"type": "wikidata", "target-type": "url", "direction": "backward",
                 "url": {"resource": "https://www.wikidata.org/wiki/Q193778"}}]});
        let found = catalogue_work(&symphony, BEETHOVEN).unwrap();
        assert_eq!(found.info.title, "Symphony No. 5 in C minor, Op. 67");
        assert_eq!(found.info.composed.as_deref(), Some("1804–1808"));
        assert_eq!(found.wikidata_id.as_deref(), Some("Q193778"));
        assert_eq!(found.composer.mbid.as_deref(), Some(BEETHOVEN));

        // A movement keeps its piece.
        let movement = json!({"id": MOVEMENT, "title": "Symphony no. 5 in C minor, op. 67: I. Allegro con brio",
            "relations": [composer.clone(), {"type": "parts", "target-type": "work",
                "direction": "backward", "ordering-key": 1,
                "work": {"id": SYMPHONY, "title": "Symphony no. 5 in C minor, op. 67"}}]});
        let found = catalogue_work(&movement, BEETHOVEN).unwrap();
        assert_eq!(
            found
                .info
                .parent
                .as_deref()
                .map(|parent| parent.mbid.as_str()),
            Some(SYMPHONY)
        );
        assert_eq!(found.info.position, Some(1));

        // Someone else's arrangement and words written to Beethoven's
        // music are not his compositions.
        let arrangement = json!({"id": ODE, "title": "Ode to Joy (arr. for piano)",
            "relations": [composer.clone(), {"type": "arrangement", "target-type": "work",
                "direction": "backward", "work": {"id": SYMPHONY, "title": "x"}}]});
        assert!(catalogue_work(&arrangement, BEETHOVEN).is_none());
        let lyrics = json!({"id": ODE, "title": "An die Freude",
            "relations": [{"type": "lyricist", "target-type": "artist", "direction": "backward",
                "artist": {"id": BEETHOVEN, "name": "Ludwig van Beethoven"}}]});
        assert!(catalogue_work(&lyrics, BEETHOVEN).is_none());
        // A namesake's work is not in this composer's catalogue.
        assert!(catalogue_work(&symphony, MOVEMENT).is_none());
        assert_eq!(
            form_from_title("Bagatelle in A minor").as_deref(),
            Some("Bagatelle")
        );
    }

    #[test]
    fn works_without_ids_are_ignored_and_songs_keep_their_type() {
        let recording = json!({"relations": [
            {"type": "performance", "target-type": "work", "work": {"title": "No id"}},
            {"type": "performance", "target-type": "work",
             "work": {"id": MOVEMENT, "title": "So What", "type": "Song", "language": "zxx"}}
        ]});
        let works = recording_works(&recording);
        assert_eq!(works.len(), 1);
        assert_eq!(works[0].form.as_deref(), Some("Song"));
        assert_eq!(works[0].language, None);
        assert_eq!(works[0].parent, None);
    }
}
