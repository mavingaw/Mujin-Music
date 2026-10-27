//! Compositions: musical works grouped from file tags, with the facts the
//! Focus panel filters on.
//!
//! A work is identified by its WORK tag (or, without one, a MusicBrainz work
//! id, a shared `Work: Movement` title prefix, or the track title) together
//! with its composer, so the same title by two composers stays two works.
//! Each work gets a form (Symphony, Sonata, Concerto, String Quartet,
//! Song, ...), an instrumentation category (Orchestra, Solo Keyboard,
//! Keyboard Trio, ...), a composition year and a period (Baroque,
//! Classical, Romantic, ...). Facts come, in order of trust, from file tags
//! (`COMPOSITIONDATE`), what MusicBrainz says about the work (the `work`
//! rows metadata enrichment fills), a small table of well known works, and
//! finally the work title itself ("Violin Concerto in D major, Op. 61").
//!
//! Works are one model: the library's works are the `work` rows marked
//! `in_library`, under their MusicBrainz id when the tags or MusicBrainz
//! album matching give one (`wk_…` otherwise), linked to tracks by
//! `track_work` rows from the `library` source. Tags come first: a track
//! takes MusicBrainz's link only when its tags name no work id.
//!
//! Composers get life dates, nationality and a period from their
//! MusicBrainz artist profile or a built-in table of canonical composers;
//! a composer with neither takes the period most of their dated works
//! share.
//!
//! The library rows (`work.in_library` and its facts, `library` links,
//! `work_composer`, `composer_entity`) are rebuilt from the catalog
//! whenever its fingerprint changes, so they never drift from the indexed
//! tracks.

use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{Integer, Nullable, Text};
use diesel::sqlite::SqliteConnection;
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

use crate::library::identity::normalize_artist_identity;

/// Musical periods, oldest first.
pub const PERIODS: [&str; 7] = [
    "Medieval",
    "Renaissance",
    "Baroque",
    "Classical",
    "Romantic",
    "Modern",
    "Contemporary",
];

/// The period a year of composition falls in.
pub fn period_for_year(year: i32) -> Option<&'static str> {
    if !(500..=2100).contains(&year) {
        return None;
    }
    Some(match year {
        ..1400 => "Medieval",
        1400..1600 => "Renaissance",
        1600..1750 => "Baroque",
        1750..1820 => "Classical",
        1820..1910 => "Romantic",
        1910..1975 => "Modern",
        _ => "Contemporary",
    })
}

/// A composer's period: the period of the middle of their working life.
pub fn composer_period(birth: Option<i32>, death: Option<i32>) -> Option<&'static str> {
    let birth = birth?;
    let start = birth + 20;
    let end = death
        .map_or(birth + 70, |death| death.min(birth + 80))
        .max(start);
    period_for_year((start + end) / 2)
}

/// Lowercase words with accents folded, e.g. "Sinfonía Nº 9" -> "sinfonia n 9".
fn folded_words(value: &str) -> String {
    let mut folded = String::with_capacity(value.len() + 2);
    folded.push(' ');
    let mut last_space = true;
    for character in value
        .nfkd()
        .filter(|character| !is_combining_mark(*character))
    {
        for lower in character.to_lowercase() {
            let lower = match lower {
                'ß' => 's',
                'ø' => 'o',
                'æ' => 'a',
                'œ' => 'o',
                other => other,
            };
            if lower.is_alphanumeric() {
                folded.push(lower);
                last_space = false;
            } else if !last_space {
                folded.push(' ');
                last_space = true;
            }
        }
    }
    if !last_space {
        folded.push(' ');
    }
    folded
}

fn has(text: &str, phrase: &str) -> bool {
    text.contains(&format!(" {phrase} "))
}

fn has_any(text: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|phrase| has(text, phrase))
}

/// Forms in match order: more specific phrases come first.
const FORMS: &[(&[&str], &str)] = &[
    (
        &[
            "symphonic poem",
            "tone poem",
            "poeme symphonique",
            "sinfonische dichtung",
        ],
        "Symphonic Poem",
    ),
    (
        &["sinfonia concertante", "symphonie concertante"],
        "Sinfonia Concertante",
    ),
    (&["concerto grosso", "concerti grossi"], "Concerto Grosso"),
    (&["trio sonata", "triosonate"], "Trio Sonata"),
    (
        &[
            "songs without words",
            "song without words",
            "lieder ohne worte",
        ],
        "Song Without Words",
    ),
    (
        &[
            "prelude and fugue",
            "preludes and fugues",
            "praeludium und fuge",
            "prelude fugue",
            "well tempered clavier",
            "wohltemperierte klavier",
        ],
        "Prelude and Fugue",
    ),
    (
        &[
            "concerto",
            "concertos",
            "concerti",
            "konzert",
            "klavierkonzert",
            "violinkonzert",
            "concertino",
        ],
        "Concerto",
    ),
    (
        &[
            "symphony",
            "symphonies",
            "symphonie",
            "sinfonie",
            "sinfonia",
            "sinfonietta",
        ],
        "Symphony",
    ),
    (
        &[
            "string quartet",
            "string quartets",
            "streichquartett",
            "quatuor a cordes",
            "quartetto per archi",
        ],
        "String Quartet",
    ),
    (&["requiem", "messa da requiem"], "Requiem"),
    (&["mass", "missa", "messe", "messa"], "Mass"),
    (
        &[
            "passion",
            "matthaus passion",
            "johannes passion",
            "st matthew passion",
        ],
        "Passion",
    ),
    (&["oratorio", "oratorium"], "Oratorio"),
    (&["cantata", "kantate", "cantatas"], "Cantata"),
    (
        &["stabat mater", "magnificat", "te deum", "vespers", "vespro"],
        "Sacred Choral Work",
    ),
    (&["motet", "motets", "motette"], "Motet"),
    (&["madrigal", "madrigals", "madrigali"], "Madrigal"),
    (
        &[
            "opera",
            "oper",
            "opera seria",
            "opera buffa",
            "singspiel",
            "operetta",
            "operette",
            "dramma giocoso",
        ],
        "Opera",
    ),
    (&["ballet", "ballett", "balletto"], "Ballet"),
    (&["incidental music", "schauspielmusik"], "Incidental Music"),
    (
        &[
            "song cycle",
            "liederkreis",
            "liederzyklus",
            "winterreise",
            "dichterliebe",
            "die schone mullerin",
        ],
        "Song Cycle",
    ),
    (&["aria", "arias"], "Aria"),
    (
        &[
            "song", "songs", "lied", "lieder", "chanson", "chansons", "melodie", "melodies",
        ],
        "Song",
    ),
    (&["overture", "ouverture", "sinfonia avanti"], "Overture"),
    (&["sonatina", "sonatine"], "Sonatina"),
    (
        &["sonata", "sonatas", "sonate", "sonaten", "klaviersonate"],
        "Sonata",
    ),
    (
        &["quintet", "quintett", "quintetto", "quintette"],
        "Quintet",
    ),
    (&["quartet", "quartett", "quartetto", "quatuor"], "Quartet"),
    (&["sextet", "sextett", "sestetto"], "Sextet"),
    (&["septet", "septett", "settimino"], "Septet"),
    (&["octet", "oktett", "ottetto"], "Octet"),
    (&["trio", "trios", "terzetto"], "Trio"),
    (
        &["variations", "variationen", "variazioni", "variation"],
        "Variations",
    ),
    (&["suite", "suites"], "Suite"),
    (&["partita", "partitas", "partite"], "Partita"),
    (
        &["serenade", "serenata", "nachtmusik", "notturno serenade"],
        "Serenade",
    ),
    (
        &["divertimento", "divertimenti", "cassation"],
        "Divertimento",
    ),
    (
        &["fugue", "fuge", "fuga", "art of fugue", "kunst der fuge"],
        "Fugue",
    ),
    (
        &["prelude", "preludes", "preludium", "praeludium"],
        "Prelude",
    ),
    (&["toccata"], "Toccata"),
    (
        &["fantasia", "fantasie", "fantaisie", "fantasy"],
        "Fantasia",
    ),
    (&["etude", "etudes", "study", "studies"], "Étude"),
    (&["nocturne", "nocturnes", "notturno"], "Nocturne"),
    (&["waltz", "waltzes", "walzer", "valse", "valses"], "Waltz"),
    (&["mazurka", "mazurkas"], "Mazurka"),
    (&["polonaise", "polonaises"], "Polonaise"),
    (&["ballade", "ballades"], "Ballade"),
    (&["impromptu", "impromptus"], "Impromptu"),
    (&["scherzo", "scherzi"], "Scherzo"),
    (
        &["rhapsody", "rhapsodie", "rapsodia", "rhapsodies"],
        "Rhapsody",
    ),
    (&["intermezzo", "intermezzi"], "Intermezzo"),
    (&["bagatelle", "bagatelles"], "Bagatelle"),
    (&["invention", "inventions"], "Invention"),
    (&["chaconne", "ciaccona", "passacaglia"], "Chaconne"),
    (&["caprice", "capriccio", "caprices", "capricci"], "Caprice"),
    (&["romance", "romanze", "romanza"], "Romance"),
    (&["barcarolle", "barcarola", "berceuse"], "Character Piece"),
    (&["march", "marsch", "marche", "marcia"], "March"),
    (&["minuet", "menuett", "menuet", "minuetto"], "Minuet"),
];

/// Forms that are solo keyboard pieces unless the title says otherwise.
const KEYBOARD_FORMS: [&str; 17] = [
    "Prelude and Fugue",
    "Song Without Words",
    "Fugue",
    "Toccata",
    "Étude",
    "Nocturne",
    "Mazurka",
    "Polonaise",
    "Ballade",
    "Impromptu",
    "Intermezzo",
    "Bagatelle",
    "Invention",
    "Sonatina",
    "Character Piece",
    "Prelude",
    "Waltz",
];

const ORCHESTRAL_FORMS: [&str; 6] = [
    "Symphony",
    "Symphonic Poem",
    "Overture",
    "Concerto Grosso",
    "Ballet",
    "Incidental Music",
];

const CHORAL_ORCHESTRAL_FORMS: [&str; 6] = [
    "Mass",
    "Requiem",
    "Passion",
    "Oratorio",
    "Cantata",
    "Sacred Choral Work",
];

const CHAMBER_FORMS: [&str; 7] = [
    "Trio",
    "Quartet",
    "Quintet",
    "Sextet",
    "Septet",
    "Octet",
    "Trio Sonata",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Family {
    Keyboard,
    Strings,
    Winds,
    Voice,
    Choir,
    Orchestra,
}

/// Instrument words and the name a scoring shows them with.
const INSTRUMENTS: &[(&[&str], &str, Family)] = &[
    (
        &["piano", "pianoforte", "klavier", "fortepiano", "clavier"],
        "Piano",
        Family::Keyboard,
    ),
    (
        &["harpsichord", "cembalo", "clavecin", "clavicembalo"],
        "Harpsichord",
        Family::Keyboard,
    ),
    (
        &["organ", "orgel", "orgue", "organo"],
        "Organ",
        Family::Keyboard,
    ),
    (&["keyboard", "keyboards"], "Keyboard", Family::Keyboard),
    (
        &["violin", "violine", "violon", "violino"],
        "Violin",
        Family::Strings,
    ),
    (
        &["viola", "bratsche", "alto viola"],
        "Viola",
        Family::Strings,
    ),
    (
        &["cello", "violoncello", "violoncelle"],
        "Cello",
        Family::Strings,
    ),
    (
        &["double bass", "contrabass", "kontrabass"],
        "Double Bass",
        Family::Strings,
    ),
    (
        &["viola da gamba", "gamba"],
        "Viola da Gamba",
        Family::Strings,
    ),
    (
        &["guitar", "gitarre", "guitare", "chitarra"],
        "Guitar",
        Family::Strings,
    ),
    (&["lute", "laute", "luth", "liuto"], "Lute", Family::Strings),
    (&["harp", "harfe", "harpe", "arpa"], "Harp", Family::Strings),
    (
        &["flute", "flote", "flauto", "recorder"],
        "Flute",
        Family::Winds,
    ),
    (&["oboe", "hautbois"], "Oboe", Family::Winds),
    (
        &["clarinet", "klarinette", "clarinette", "clarinetto"],
        "Clarinet",
        Family::Winds,
    ),
    (
        &["bassoon", "fagott", "basson", "fagotto"],
        "Bassoon",
        Family::Winds,
    ),
    (&["horn", "corno", "cor"], "Horn", Family::Winds),
    (
        &["trumpet", "trompete", "trompette", "tromba"],
        "Trumpet",
        Family::Winds,
    ),
    (&["trombone", "posaune"], "Trombone", Family::Winds),
    (&["saxophone"], "Saxophone", Family::Winds),
    (
        &[
            "voice",
            "soprano",
            "mezzo soprano",
            "alto",
            "tenor",
            "baritone",
            "singstimme",
            "gesang",
        ],
        "Voice",
        Family::Voice,
    ),
    (
        &["chorus", "choir", "chor", "choeur", "coro", "choral"],
        "Choir",
        Family::Choir,
    ),
    (
        &[
            "orchestra",
            "orchester",
            "orchestre",
            "orchestra d archi",
            "orchestral",
            "string orchestra",
            "strings",
        ],
        "Orchestra",
        Family::Orchestra,
    ),
];

/// What a work title says about the work.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkClass {
    pub form: Option<&'static str>,
    /// Facet category: Orchestra, Solo Keyboard, Keyboard Trio, ...
    pub instrumentation: Option<&'static str>,
    /// The specific scoring when the title names it ("Violin & Orchestra").
    pub scoring: Option<String>,
    /// A year in the title, e.g. "Overture (1812)".
    pub composed_year: Option<i32>,
}

/// The form a MusicBrainz work type names.
pub fn form_from_musicbrainz_type(work_type: &str) -> Option<&'static str> {
    let folded = folded_words(work_type);
    let form = match folded.trim() {
        "song cycle" => "Song Cycle",
        "symphonic poem" => "Symphonic Poem",
        "etude" => "Étude",
        "play" | "poem" | "soundtrack" | "musical" | "audio drama" | "beijing opera" => {
            return None;
        }
        "zarzuela" | "operetta" => "Opera",
        "incidental music" => "Incidental Music",
        _ => return classify_form(&folded),
    };
    Some(form)
}

fn classify_form(text: &str) -> Option<&'static str> {
    FORMS
        .iter()
        .find(|(phrases, _)| has_any(text, phrases))
        .map(|(_, form)| *form)
}

/// The instruments a piece of text names, in title order without repeats.
fn instruments_in(text: &str) -> Vec<(&'static str, Family)> {
    let mut found = Vec::<(usize, &'static str, Family)>::new();
    for (words, name, family) in INSTRUMENTS {
        let position = words
            .iter()
            .filter_map(|word| text.find(&format!(" {word} ")))
            .min();
        if let Some(position) = position
            && !found.iter().any(|(_, known, _)| known == name)
        {
            found.push((position, name, *family));
        }
    }
    // "viola da gamba" also contains "viola".
    if found.iter().any(|(_, name, _)| *name == "Viola da Gamba") {
        found.retain(|(_, name, _)| *name != "Viola");
    }
    // "string orchestra" is an orchestra, not a soloist.
    found.sort_by_key(|(position, _, _)| *position);
    found
        .into_iter()
        .map(|(_, name, family)| (name, family))
        .collect()
}

/// The part of a title after "for" / "für" / "pour" / "per".
fn scoring_clause(text: &str) -> Option<&str> {
    [" for ", " fur ", " pour ", " per "]
        .iter()
        .filter_map(|marker| {
            text.find(marker)
                .map(|index| &text[index + marker.len() - 1..])
        })
        .min_by_key(|clause| std::cmp::Reverse(clause.len()))
}

fn join_scoring(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} & {last}", rest.join(", ")),
    }
}

fn year_in_title(title: &str) -> Option<i32> {
    static YEAR: OnceLock<regex::Regex> = OnceLock::new();
    let year = YEAR.get_or_init(|| {
        regex::Regex::new(
            r"(?i)(?:\(\s*(?:c\.|ca\.|circa)?\s*|\bcomposed\s+(?:in\s+)?)(1[0-9]{3}|20[0-9]{2})\b",
        )
        .expect("year pattern")
    });
    year.captures(title)
        .and_then(|captures| captures.get(1))
        .and_then(|year| year.as_str().parse::<i32>().ok())
        .filter(|year| (900..=2100).contains(year))
}

/// Classifies a work from its title.
pub fn classify_title(title: &str) -> WorkClass {
    let text = folded_words(title);
    let form = classify_form(&text);
    let (instrumentation, scoring) = classify_instrumentation(&text, form);
    WorkClass {
        form,
        instrumentation,
        scoring,
        composed_year: year_in_title(title),
    }
}

fn classify_instrumentation(
    text: &str,
    form: Option<&'static str>,
) -> (Option<&'static str>, Option<String>) {
    let clause = scoring_clause(text);
    let named = clause
        .map(instruments_in)
        .filter(|found| !found.is_empty())
        .unwrap_or_else(|| instruments_in(text));
    let solo = has_any(text, &["solo", "unaccompanied", "senza basso"]);
    let duo_keyboard = has_any(
        text,
        &[
            "two pianos",
            "2 pianos",
            "four hands",
            "piano duet",
            "4 hands",
            "vierhandig",
        ],
    );
    let families = named.iter().map(|(_, family)| *family).collect::<Vec<_>>();
    let orchestra = families.contains(&Family::Orchestra);
    let choir = families.contains(&Family::Choir);
    let voice = families.contains(&Family::Voice);
    let soloists = named
        .iter()
        .filter(|(_, family)| !matches!(family, Family::Orchestra | Family::Choir))
        .map(|(name, _)| *name)
        .collect::<Vec<_>>();
    let keyboards = named
        .iter()
        .filter(|(_, family)| *family == Family::Keyboard)
        .count();
    let others = soloists.len() - keyboards;
    let form_is = |list: &[&str]| form.is_some_and(|form| list.contains(&form));
    let with_orchestra = |soloists: &[&str]| {
        let mut names = soloists.to_vec();
        names.push("Orchestra");
        Some(join_scoring(&names))
    };

    if form == Some("Concerto Grosso") {
        return (Some("Orchestra"), None);
    }
    if form == Some("Concerto") || form == Some("Sinfonia Concertante") {
        if has(text, "for orchestra") {
            return (Some("Orchestra"), None);
        }
        let scoring = (!soloists.is_empty())
            .then(|| with_orchestra(&soloists))
            .flatten();
        return (Some("Solo Instrument & Orchestra"), scoring);
    }
    if choir || form_is(&CHORAL_ORCHESTRAL_FORMS) {
        let category = if orchestra || form_is(&CHORAL_ORCHESTRAL_FORMS) && !has(text, "a cappella")
        {
            "Choir & Orchestra"
        } else {
            "Choir"
        };
        return (Some(category), None);
    }
    if matches!(form, Some("Motet" | "Madrigal")) {
        return (Some("Choir"), None);
    }
    if form == Some("Opera") {
        return (Some("Voices & Orchestra"), None);
    }
    if orchestra && !soloists.is_empty() {
        let category = if soloists == ["Voice"] {
            "Voice & Orchestra"
        } else {
            "Solo Instrument & Orchestra"
        };
        return (Some(category), with_orchestra(&soloists));
    }
    if orchestra || form_is(&ORCHESTRAL_FORMS) {
        return (Some("Orchestra"), None);
    }
    if form == Some("String Quartet") {
        return (Some("String Quartet"), None);
    }
    if voice || matches!(form, Some("Song" | "Song Cycle")) {
        if form == Some("Aria") && keyboards == 0 {
            return (Some("Voice & Orchestra"), None);
        }
        let scoring = (keyboards > 0).then(|| join_scoring(&soloists));
        return (Some("Voice & Keyboard"), scoring);
    }
    if form == Some("Aria") {
        return (Some("Voice & Orchestra"), None);
    }
    if form_is(&CHAMBER_FORMS) || matches!(form, Some("Serenade" | "Divertimento")) {
        let category = match form {
            Some("Trio") if keyboards > 0 => "Keyboard Trio",
            Some("Trio") if has(text, "string trio") => "String Trio",
            Some("Quartet") if keyboards > 0 => "Keyboard Quartet",
            Some("Quintet") if keyboards > 0 => "Keyboard Quintet",
            Some("Quintet") if has(text, "string quintet") => "String Quintet",
            Some("Quintet") if has(text, "wind quintet") => "Wind Ensemble",
            _ => "Chamber Ensemble",
        };
        return (Some(category), None);
    }
    if duo_keyboard {
        return (Some("Keyboard Duo"), Some("Two Pianos".into()));
    }
    match (keyboards, others) {
        (0, 0) if form_is(&KEYBOARD_FORMS) => (Some("Solo Keyboard"), None),
        (0, 0) => (None, None),
        (_, 0) => (Some("Solo Keyboard"), Some(join_scoring(&soloists))),
        (0, 1) if solo || matches!(form, Some("Partita" | "Suite" | "Caprice" | "Chaconne")) => {
            (Some("Solo Instrument"), Some(soloists[0].to_string()))
        }
        (0, 1)
            if matches!(
                form,
                Some("Sonata" | "Sonatina" | "Romance" | "Fantasia" | "Variations")
            ) =>
        {
            (
                Some("Instrument & Keyboard"),
                Some(join_scoring(&[soloists[0], "Keyboard"])),
            )
        }
        (0, 1) => (Some("Solo Instrument"), Some(soloists[0].to_string())),
        (_, 1) => (Some("Instrument & Keyboard"), Some(join_scoring(&soloists))),
        _ => (Some("Chamber Ensemble"), Some(join_scoring(&soloists))),
    }
}

/// Facts about a well known work whose title alone does not say them.
struct KnownWork {
    composer: &'static str,
    title: &'static [&'static str],
    form: &'static str,
    instrumentation: &'static str,
    scoring: Option<&'static str>,
    year: i32,
}

const KNOWN_WORKS: &[KnownWork] = &[
    KnownWork {
        composer: "bach",
        title: &[
            "goldberg variations",
            "goldberg variationen",
            "aria mit verschiedenen veranderungen",
        ],
        form: "Variations",
        instrumentation: "Solo Keyboard",
        scoring: Some("Harpsichord"),
        year: 1741,
    },
    KnownWork {
        composer: "bach",
        title: &["well tempered clavier", "wohltemperierte klavier"],
        form: "Prelude and Fugue",
        instrumentation: "Solo Keyboard",
        scoring: None,
        year: 1722,
    },
    KnownWork {
        composer: "bach",
        title: &[
            "brandenburg concerto",
            "brandenburgisches konzert",
            "brandenburg concertos",
        ],
        form: "Concerto Grosso",
        instrumentation: "Orchestra",
        scoring: None,
        year: 1721,
    },
    KnownWork {
        composer: "bach",
        title: &["art of fugue", "kunst der fuge"],
        form: "Fugue",
        instrumentation: "Solo Keyboard",
        scoring: None,
        year: 1750,
    },
    KnownWork {
        composer: "bach",
        title: &["mass in b minor", "messe in h moll", "hohe messe"],
        form: "Mass",
        instrumentation: "Choir & Orchestra",
        scoring: None,
        year: 1749,
    },
    KnownWork {
        composer: "bach",
        title: &["st matthew passion", "matthaus passion", "matthauspassion"],
        form: "Passion",
        instrumentation: "Choir & Orchestra",
        scoring: None,
        year: 1727,
    },
    KnownWork {
        composer: "vivaldi",
        title: &["four seasons", "quattro stagioni"],
        form: "Concerto",
        instrumentation: "Solo Instrument & Orchestra",
        scoring: Some("Violin & Orchestra"),
        year: 1720,
    },
    KnownWork {
        composer: "handel",
        title: &["messiah"],
        form: "Oratorio",
        instrumentation: "Choir & Orchestra",
        scoring: None,
        year: 1741,
    },
    KnownWork {
        composer: "handel",
        title: &["water music", "wassermusik"],
        form: "Suite",
        instrumentation: "Orchestra",
        scoring: None,
        year: 1717,
    },
    KnownWork {
        composer: "mozart",
        title: &["eine kleine nachtmusik"],
        form: "Serenade",
        instrumentation: "Chamber Ensemble",
        scoring: Some("Strings"),
        year: 1787,
    },
    KnownWork {
        composer: "mozart",
        title: &["die zauberflote", "magic flute"],
        form: "Opera",
        instrumentation: "Voices & Orchestra",
        scoring: None,
        year: 1791,
    },
    KnownWork {
        composer: "mozart",
        title: &["don giovanni"],
        form: "Opera",
        instrumentation: "Voices & Orchestra",
        scoring: None,
        year: 1787,
    },
    KnownWork {
        composer: "mozart",
        title: &["le nozze di figaro", "marriage of figaro"],
        form: "Opera",
        instrumentation: "Voices & Orchestra",
        scoring: None,
        year: 1786,
    },
    KnownWork {
        composer: "mozart",
        title: &["requiem in d minor"],
        form: "Requiem",
        instrumentation: "Choir & Orchestra",
        scoring: None,
        year: 1791,
    },
    KnownWork {
        composer: "beethoven",
        title: &["symphony no 3", "eroica"],
        form: "Symphony",
        instrumentation: "Orchestra",
        scoring: None,
        year: 1804,
    },
    KnownWork {
        composer: "beethoven",
        title: &["symphony no 5 in c minor", "symphony no 5 op 67"],
        form: "Symphony",
        instrumentation: "Orchestra",
        scoring: None,
        year: 1808,
    },
    KnownWork {
        composer: "beethoven",
        title: &["symphony no 6", "pastoral symphony"],
        form: "Symphony",
        instrumentation: "Orchestra",
        scoring: None,
        year: 1808,
    },
    KnownWork {
        composer: "beethoven",
        title: &["symphony no 7"],
        form: "Symphony",
        instrumentation: "Orchestra",
        scoring: None,
        year: 1812,
    },
    KnownWork {
        composer: "beethoven",
        title: &["symphony no 9", "choral symphony"],
        form: "Symphony",
        instrumentation: "Orchestra",
        scoring: None,
        year: 1824,
    },
    KnownWork {
        composer: "beethoven",
        title: &[
            "moonlight sonata",
            "sonata quasi una fantasia op 27 no 2",
            "piano sonata no 14",
        ],
        form: "Sonata",
        instrumentation: "Solo Keyboard",
        scoring: Some("Piano"),
        year: 1801,
    },
    KnownWork {
        composer: "beethoven",
        title: &["fur elise"],
        form: "Bagatelle",
        instrumentation: "Solo Keyboard",
        scoring: Some("Piano"),
        year: 1810,
    },
    KnownWork {
        composer: "beethoven",
        title: &["violin concerto in d major op 61"],
        form: "Concerto",
        instrumentation: "Solo Instrument & Orchestra",
        scoring: Some("Violin & Orchestra"),
        year: 1806,
    },
    KnownWork {
        composer: "schubert",
        title: &["winterreise"],
        form: "Song Cycle",
        instrumentation: "Voice & Keyboard",
        scoring: Some("Voice & Piano"),
        year: 1827,
    },
    KnownWork {
        composer: "schubert",
        title: &["trout quintet", "forellenquintett"],
        form: "Quintet",
        instrumentation: "Keyboard Quintet",
        scoring: None,
        year: 1819,
    },
    KnownWork {
        composer: "schubert",
        title: &["unfinished"],
        form: "Symphony",
        instrumentation: "Orchestra",
        scoring: None,
        year: 1822,
    },
    KnownWork {
        composer: "schumann",
        title: &["kinderszenen", "scenes from childhood"],
        form: "Suite",
        instrumentation: "Solo Keyboard",
        scoring: Some("Piano"),
        year: 1838,
    },
    KnownWork {
        composer: "schumann",
        title: &["dichterliebe"],
        form: "Song Cycle",
        instrumentation: "Voice & Keyboard",
        scoring: Some("Voice & Piano"),
        year: 1840,
    },
    KnownWork {
        composer: "mussorgsky",
        title: &["pictures at an exhibition", "tableaux d une exposition"],
        form: "Suite",
        instrumentation: "Solo Keyboard",
        scoring: Some("Piano"),
        year: 1874,
    },
    KnownWork {
        composer: "tchaikovsky",
        title: &["1812 overture", "overture 1812", "overture solennelle"],
        form: "Overture",
        instrumentation: "Orchestra",
        scoring: None,
        year: 1880,
    },
    KnownWork {
        composer: "tchaikovsky",
        title: &["nutcracker", "shchelkunchik"],
        form: "Ballet",
        instrumentation: "Orchestra",
        scoring: None,
        year: 1892,
    },
    KnownWork {
        composer: "tchaikovsky",
        title: &["swan lake", "lebedinoye ozero"],
        form: "Ballet",
        instrumentation: "Orchestra",
        scoring: None,
        year: 1876,
    },
    KnownWork {
        composer: "bizet",
        title: &["carmen"],
        form: "Opera",
        instrumentation: "Voices & Orchestra",
        scoring: None,
        year: 1875,
    },
    KnownWork {
        composer: "saint saens",
        title: &["carnival of the animals", "carnaval des animaux"],
        form: "Suite",
        instrumentation: "Chamber Ensemble",
        scoring: None,
        year: 1886,
    },
    KnownWork {
        composer: "holst",
        title: &["the planets", "planets"],
        form: "Suite",
        instrumentation: "Orchestra",
        scoring: None,
        year: 1916,
    },
    KnownWork {
        composer: "stravinsky",
        title: &["rite of spring", "sacre du printemps"],
        form: "Ballet",
        instrumentation: "Orchestra",
        scoring: None,
        year: 1913,
    },
    KnownWork {
        composer: "stravinsky",
        title: &["firebird", "l oiseau de feu"],
        form: "Ballet",
        instrumentation: "Orchestra",
        scoring: None,
        year: 1910,
    },
    KnownWork {
        composer: "ravel",
        title: &["bolero"],
        form: "Ballet",
        instrumentation: "Orchestra",
        scoring: None,
        year: 1928,
    },
    KnownWork {
        composer: "debussy",
        title: &["clair de lune", "suite bergamasque"],
        form: "Suite",
        instrumentation: "Solo Keyboard",
        scoring: Some("Piano"),
        year: 1905,
    },
    KnownWork {
        composer: "debussy",
        title: &["la mer"],
        form: "Symphonic Poem",
        instrumentation: "Orchestra",
        scoring: None,
        year: 1905,
    },
    KnownWork {
        composer: "debussy",
        title: &["prelude a l apres midi d un faune", "afternoon of a faun"],
        form: "Symphonic Poem",
        instrumentation: "Orchestra",
        scoring: None,
        year: 1894,
    },
    KnownWork {
        composer: "orff",
        title: &["carmina burana"],
        form: "Cantata",
        instrumentation: "Choir & Orchestra",
        scoring: None,
        year: 1936,
    },
    KnownWork {
        composer: "gershwin",
        title: &["rhapsody in blue"],
        form: "Rhapsody",
        instrumentation: "Solo Instrument & Orchestra",
        scoring: Some("Piano & Orchestra"),
        year: 1924,
    },
    KnownWork {
        composer: "barber",
        title: &["adagio for strings"],
        form: "Adagio",
        instrumentation: "Orchestra",
        scoring: Some("String Orchestra"),
        year: 1936,
    },
    KnownWork {
        composer: "pachelbel",
        title: &["canon in d", "canon and gigue"],
        form: "Canon",
        instrumentation: "Chamber Ensemble",
        scoring: None,
        year: 1690,
    },
    KnownWork {
        composer: "grieg",
        title: &["peer gynt"],
        form: "Incidental Music",
        instrumentation: "Orchestra",
        scoring: None,
        year: 1875,
    },
    KnownWork {
        composer: "dvorak",
        title: &["new world", "symphony no 9 in e minor"],
        form: "Symphony",
        instrumentation: "Orchestra",
        scoring: None,
        year: 1893,
    },
    KnownWork {
        composer: "mahler",
        title: &["das lied von der erde"],
        form: "Song Cycle",
        instrumentation: "Voice & Orchestra",
        scoring: Some("Voices & Orchestra"),
        year: 1909,
    },
    KnownWork {
        composer: "mozart",
        title: &["piano concerto no 21", "k 467"],
        form: "Concerto",
        instrumentation: "Solo Instrument & Orchestra",
        scoring: Some("Piano & Orchestra"),
        year: 1785,
    },
    KnownWork {
        composer: "mozart",
        title: &["symphony no 41", "jupiter"],
        form: "Symphony",
        instrumentation: "Orchestra",
        scoring: None,
        year: 1788,
    },
    KnownWork {
        composer: "mozart",
        title: &["clarinet concerto", "k 622"],
        form: "Concerto",
        instrumentation: "Solo Instrument & Orchestra",
        scoring: Some("Clarinet & Orchestra"),
        year: 1791,
    },
    KnownWork {
        composer: "bach",
        title: &["cello suite no 1", "bwv 1007"],
        form: "Suite",
        instrumentation: "Solo Instrument",
        scoring: Some("Cello"),
        year: 1720,
    },
    KnownWork {
        composer: "bach",
        title: &["partita no 2 in d minor", "bwv 1004"],
        form: "Partita",
        instrumentation: "Solo Instrument",
        scoring: Some("Violin"),
        year: 1720,
    },
    KnownWork {
        composer: "haydn",
        title: &["op 76 no 3", "emperor quartet", "kaiserquartett"],
        form: "String Quartet",
        instrumentation: "String Quartet",
        scoring: None,
        year: 1797,
    },
    KnownWork {
        composer: "chopin",
        title: &["op 9 no 2"],
        form: "Nocturne",
        instrumentation: "Solo Keyboard",
        scoring: Some("Piano"),
        year: 1832,
    },
    KnownWork {
        composer: "chopin",
        title: &["ballade no 1", "op 23"],
        form: "Ballade",
        instrumentation: "Solo Keyboard",
        scoring: Some("Piano"),
        year: 1835,
    },
    KnownWork {
        composer: "chopin",
        title: &["op 17 no 4"],
        form: "Mazurka",
        instrumentation: "Solo Keyboard",
        scoring: Some("Piano"),
        year: 1833,
    },
    KnownWork {
        composer: "schubert",
        title: &["d 929", "piano trio no 2"],
        form: "Trio",
        instrumentation: "Keyboard Trio",
        scoring: Some("Piano, Violin & Cello"),
        year: 1827,
    },
    KnownWork {
        composer: "brahms",
        title: &["ein deutsches requiem", "german requiem"],
        form: "Requiem",
        instrumentation: "Choir & Orchestra",
        scoring: None,
        year: 1868,
    },
    KnownWork {
        composer: "vivaldi",
        title: &["gloria in d major", "rv 589"],
        form: "Sacred Choral Work",
        instrumentation: "Choir & Orchestra",
        scoring: None,
        year: 1715,
    },
    KnownWork {
        composer: "part",
        title: &["spiegel im spiegel"],
        form: "Character Piece",
        instrumentation: "Instrument & Keyboard",
        scoring: Some("Violin & Piano"),
        year: 1978,
    },
];

fn known_work(title: &str, composers: &[String]) -> Option<&'static KnownWork> {
    let text = folded_words(title);
    KNOWN_WORKS.iter().find(|work| {
        composers.iter().any(|composer| {
            let composer = folded_words(composer);
            has(&composer, work.composer)
        }) && work.title.iter().any(|phrase| has(&text, phrase))
    })
}

/// Life dates and nationality of a composer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComposerFacts {
    pub birth: i32,
    pub death: Option<i32>,
    pub nationality: &'static str,
}

/// Name, other spellings, birth, death, nationality.
type ComposerEntry = (
    &'static str,
    &'static [&'static str],
    i32,
    Option<i32>,
    &'static str,
);

/// Canonical composers: name, other spellings, birth, death, nationality.
#[rustfmt::skip]
const COMPOSERS: &[ComposerEntry] = &[
    ("Hildegard von Bingen", &["hildegard"], 1098, Some(1179), "German"),
    ("Guillaume de Machaut", &["machaut"], 1300, Some(1377), "French"),
    ("Guillaume Du Fay", &["dufay", "guillaume dufay"], 1397, Some(1474), "Franco-Flemish"),
    ("Josquin des Prez", &["josquin", "josquin desprez"], 1450, Some(1521), "Franco-Flemish"),
    ("Thomas Tallis", &[], 1505, Some(1585), "British"),
    ("Giovanni Pierluigi da Palestrina", &["palestrina"], 1525, Some(1594), "Italian"),
    ("Orlande de Lassus", &["orlando di lasso", "lassus"], 1532, Some(1594), "Franco-Flemish"),
    ("William Byrd", &[], 1540, Some(1623), "British"),
    ("Tomás Luis de Victoria", &["victoria"], 1548, Some(1611), "Spanish"),
    ("John Dowland", &[], 1563, Some(1626), "British"),
    ("Carlo Gesualdo", &[], 1566, Some(1613), "Italian"),
    ("Claudio Monteverdi", &[], 1567, Some(1643), "Italian"),
    ("Girolamo Frescobaldi", &[], 1583, Some(1643), "Italian"),
    ("Heinrich Schütz", &[], 1585, Some(1672), "German"),
    ("Jean-Baptiste Lully", &["lully"], 1632, Some(1687), "French"),
    ("Dieterich Buxtehude", &[], 1637, Some(1707), "Danish"),
    ("Arcangelo Corelli", &[], 1653, Some(1713), "Italian"),
    ("Johann Pachelbel", &[], 1653, Some(1706), "German"),
    ("Henry Purcell", &[], 1659, Some(1695), "British"),
    ("Alessandro Scarlatti", &[], 1660, Some(1725), "Italian"),
    ("François Couperin", &["couperin"], 1668, Some(1733), "French"),
    ("Tomaso Albinoni", &[], 1671, Some(1751), "Italian"),
    ("Antonio Vivaldi", &[], 1678, Some(1741), "Italian"),
    ("Georg Philipp Telemann", &[], 1681, Some(1767), "German"),
    ("Jean-Philippe Rameau", &[], 1683, Some(1764), "French"),
    ("Johann Sebastian Bach", &["bach", "j s bach", "js bach", "j sebastian bach"], 1685, Some(1750), "German"),
    ("George Frideric Handel", &["handel", "georg friedrich handel", "g f handel", "georg friedrich haendel"], 1685, Some(1759), "German"),
    ("Domenico Scarlatti", &[], 1685, Some(1757), "Italian"),
    ("Giovanni Battista Pergolesi", &["pergolesi"], 1710, Some(1736), "Italian"),
    ("Carl Philipp Emanuel Bach", &["c p e bach", "cpe bach"], 1714, Some(1788), "German"),
    ("Christoph Willibald Gluck", &["gluck"], 1714, Some(1787), "German"),
    ("Joseph Haydn", &["haydn", "franz joseph haydn", "j haydn"], 1732, Some(1809), "Austrian"),
    ("Johann Christian Bach", &["j c bach"], 1735, Some(1782), "German"),
    ("Luigi Boccherini", &[], 1743, Some(1805), "Italian"),
    ("Antonio Salieri", &[], 1750, Some(1825), "Italian"),
    ("Muzio Clementi", &[], 1752, Some(1832), "Italian"),
    ("Wolfgang Amadeus Mozart", &["mozart", "w a mozart", "wa mozart"], 1756, Some(1791), "Austrian"),
    ("Ludwig van Beethoven", &["beethoven", "l van beethoven"], 1770, Some(1827), "German"),
    ("Johann Nepomuk Hummel", &["hummel"], 1778, Some(1837), "Austrian"),
    ("Niccolò Paganini", &["paganini"], 1782, Some(1840), "Italian"),
    ("Carl Maria von Weber", &["weber"], 1786, Some(1826), "German"),
    ("Gioachino Rossini", &["rossini", "gioacchino rossini"], 1792, Some(1868), "Italian"),
    ("Franz Schubert", &["schubert", "franz peter schubert"], 1797, Some(1828), "Austrian"),
    ("Gaetano Donizetti", &["donizetti"], 1797, Some(1848), "Italian"),
    ("Vincenzo Bellini", &["bellini"], 1801, Some(1835), "Italian"),
    ("Hector Berlioz", &["berlioz"], 1803, Some(1869), "French"),
    ("Mikhail Glinka", &["glinka"], 1804, Some(1857), "Russian"),
    ("Fanny Mendelssohn", &["fanny hensel"], 1805, Some(1847), "German"),
    ("Felix Mendelssohn", &["mendelssohn", "felix mendelssohn bartholdy"], 1809, Some(1847), "German"),
    ("Frédéric Chopin", &["chopin", "fryderyk chopin"], 1810, Some(1849), "Polish"),
    ("Robert Schumann", &["schumann"], 1810, Some(1856), "German"),
    ("Franz Liszt", &["liszt", "ferenc liszt"], 1811, Some(1886), "Hungarian"),
    ("Richard Wagner", &["wagner"], 1813, Some(1883), "German"),
    ("Giuseppe Verdi", &["verdi"], 1813, Some(1901), "Italian"),
    ("Charles Gounod", &["gounod"], 1818, Some(1893), "French"),
    ("Clara Schumann", &["clara wieck"], 1819, Some(1896), "German"),
    ("Jacques Offenbach", &["offenbach"], 1819, Some(1880), "French"),
    ("César Franck", &["franck"], 1822, Some(1890), "French"),
    ("Bedřich Smetana", &["smetana"], 1824, Some(1884), "Czech"),
    ("Anton Bruckner", &["bruckner"], 1824, Some(1896), "Austrian"),
    ("Johann Strauss II", &["johann strauss", "johann strauss jr", "johann strauss ii"], 1825, Some(1899), "Austrian"),
    ("Johannes Brahms", &["brahms"], 1833, Some(1897), "German"),
    ("Alexander Borodin", &["borodin"], 1833, Some(1887), "Russian"),
    ("Camille Saint-Saëns", &["saint saens"], 1835, Some(1921), "French"),
    ("Léo Delibes", &["delibes"], 1836, Some(1891), "French"),
    ("Mily Balakirev", &["balakirev"], 1837, Some(1910), "Russian"),
    ("Georges Bizet", &["bizet"], 1838, Some(1875), "French"),
    ("Modest Mussorgsky", &["mussorgsky", "modest moussorgsky", "moussorgsky"], 1839, Some(1881), "Russian"),
    ("Pyotr Ilyich Tchaikovsky", &["tchaikovsky", "peter tchaikovsky", "pyotr tchaikovsky", "peter ilyich tchaikovsky", "piotr ilyich tchaikovsky", "tschaikowsky"], 1840, Some(1893), "Russian"),
    ("Antonín Dvořák", &["dvorak", "antonin dvorak"], 1841, Some(1904), "Czech"),
    ("Jules Massenet", &["massenet"], 1842, Some(1912), "French"),
    ("Arthur Sullivan", &[], 1842, Some(1900), "British"),
    ("Edvard Grieg", &["grieg"], 1843, Some(1907), "Norwegian"),
    ("Nikolai Rimsky-Korsakov", &["rimsky korsakov"], 1844, Some(1908), "Russian"),
    ("Gabriel Fauré", &["faure"], 1845, Some(1924), "French"),
    ("Leoš Janáček", &["janacek"], 1854, Some(1928), "Czech"),
    ("Edward Elgar", &["elgar"], 1857, Some(1934), "British"),
    ("Giacomo Puccini", &["puccini"], 1858, Some(1924), "Italian"),
    ("Gustav Mahler", &["mahler"], 1860, Some(1911), "Austrian"),
    ("Isaac Albéniz", &["albeniz"], 1860, Some(1909), "Spanish"),
    ("Hugo Wolf", &[], 1860, Some(1903), "Austrian"),
    ("Claude Debussy", &["debussy"], 1862, Some(1918), "French"),
    ("Frederick Delius", &["delius"], 1862, Some(1934), "British"),
    ("Pietro Mascagni", &["mascagni"], 1863, Some(1945), "Italian"),
    ("Richard Strauss", &[], 1864, Some(1949), "German"),
    ("Carl Nielsen", &["nielsen"], 1865, Some(1931), "Danish"),
    ("Jean Sibelius", &["sibelius"], 1865, Some(1957), "Finnish"),
    ("Alexander Glazunov", &["glazunov"], 1865, Some(1936), "Russian"),
    ("Erik Satie", &["satie"], 1866, Some(1925), "French"),
    ("Enrique Granados", &["granados"], 1867, Some(1916), "Spanish"),
    ("Alexander Scriabin", &["scriabin", "skryabin"], 1872, Some(1915), "Russian"),
    ("Ralph Vaughan Williams", &["vaughan williams"], 1872, Some(1958), "British"),
    ("Sergei Rachmaninoff", &["rachmaninoff", "rachmaninov", "sergei rachmaninov", "sergey rachmaninov"], 1873, Some(1943), "Russian"),
    ("Max Reger", &["reger"], 1873, Some(1916), "German"),
    ("Gustav Holst", &["holst"], 1874, Some(1934), "British"),
    ("Arnold Schoenberg", &["schoenberg", "arnold schonberg", "schonberg"], 1874, Some(1951), "Austrian"),
    ("Charles Ives", &["ives"], 1874, Some(1954), "American"),
    ("Maurice Ravel", &["ravel"], 1875, Some(1937), "French"),
    ("Manuel de Falla", &["falla"], 1876, Some(1946), "Spanish"),
    ("Ottorino Respighi", &["respighi"], 1879, Some(1936), "Italian"),
    ("Béla Bartók", &["bartok"], 1881, Some(1945), "Hungarian"),
    ("Igor Stravinsky", &["stravinsky"], 1882, Some(1971), "Russian"),
    ("Zoltán Kodály", &["kodaly"], 1882, Some(1967), "Hungarian"),
    ("Karol Szymanowski", &["szymanowski"], 1882, Some(1937), "Polish"),
    ("Anton Webern", &["webern", "anton von webern"], 1883, Some(1945), "Austrian"),
    ("Alban Berg", &[], 1885, Some(1935), "Austrian"),
    ("Heitor Villa-Lobos", &["villa lobos"], 1887, Some(1959), "Brazilian"),
    ("Sergei Prokofiev", &["prokofiev", "sergey prokofiev"], 1891, Some(1953), "Russian"),
    ("Darius Milhaud", &["milhaud"], 1892, Some(1974), "French"),
    ("Arthur Honegger", &["honegger"], 1892, Some(1955), "Swiss"),
    ("Paul Hindemith", &["hindemith"], 1895, Some(1963), "German"),
    ("Carl Orff", &["orff"], 1895, Some(1982), "German"),
    ("George Gershwin", &["gershwin"], 1898, Some(1937), "American"),
    ("Francis Poulenc", &["poulenc"], 1899, Some(1963), "French"),
    ("Aaron Copland", &["copland"], 1900, Some(1990), "American"),
    ("Kurt Weill", &["weill"], 1900, Some(1950), "German"),
    ("Joaquín Rodrigo", &["rodrigo"], 1901, Some(1999), "Spanish"),
    ("Aram Khachaturian", &["khachaturian"], 1903, Some(1978), "Armenian"),
    ("Dmitri Shostakovich", &["shostakovich", "dmitry shostakovich"], 1906, Some(1975), "Russian"),
    ("Olivier Messiaen", &["messiaen"], 1908, Some(1992), "French"),
    ("Samuel Barber", &["barber"], 1910, Some(1981), "American"),
    ("John Cage", &[], 1912, Some(1992), "American"),
    ("Benjamin Britten", &["britten"], 1913, Some(1976), "British"),
    ("Witold Lutosławski", &["lutoslawski"], 1913, Some(1994), "Polish"),
    ("Leonard Bernstein", &["bernstein"], 1918, Some(1990), "American"),
    ("Astor Piazzolla", &["piazzolla"], 1921, Some(1992), "Argentine"),
    ("György Ligeti", &["ligeti"], 1923, Some(2006), "Hungarian"),
    ("Pierre Boulez", &["boulez"], 1925, Some(2016), "French"),
    ("Karlheinz Stockhausen", &["stockhausen"], 1928, Some(2007), "German"),
    ("Ennio Morricone", &["morricone"], 1928, Some(2020), "Italian"),
    ("Einojuhani Rautavaara", &["rautavaara"], 1928, Some(2016), "Finnish"),
    ("Tōru Takemitsu", &["takemitsu", "toru takemitsu"], 1930, Some(1996), "Japanese"),
    ("John Williams", &[], 1932, None, "American"),
    ("Krzysztof Penderecki", &["penderecki"], 1933, Some(2020), "Polish"),
    ("Henryk Górecki", &["gorecki", "henryk mikolaj gorecki"], 1933, Some(2010), "Polish"),
    ("Alfred Schnittke", &["schnittke"], 1934, Some(1998), "Russian"),
    ("Arvo Pärt", &["part", "arvo part"], 1935, None, "Estonian"),
    ("Steve Reich", &[], 1936, None, "American"),
    ("Philip Glass", &[], 1937, None, "American"),
    ("John Adams", &[], 1947, None, "American"),
    ("Kaija Saariaho", &["saariaho"], 1952, Some(2023), "Finnish"),
    ("Ludovico Einaudi", &["einaudi"], 1955, None, "Italian"),
    ("Hans Zimmer", &[], 1957, None, "German"),
    ("Max Richter", &[], 1966, None, "British"),
    ("Jóhann Jóhannsson", &["johann johannsson"], 1969, Some(2018), "Icelandic"),
    ("Thomas Adès", &["ades", "thomas ades"], 1971, None, "British"),
];

fn composer_index() -> &'static HashMap<String, ComposerFacts> {
    static INDEX: OnceLock<HashMap<String, ComposerFacts>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut index = HashMap::new();
        for (name, aliases, birth, death, nationality) in COMPOSERS {
            let facts = ComposerFacts {
                birth: *birth,
                death: *death,
                nationality,
            };
            index.insert(normalize_artist_identity(name), facts);
            for alias in *aliases {
                index.insert(normalize_artist_identity(alias), facts);
            }
        }
        index
    })
}

/// Built-in facts about a canonical composer, matched on the name as tagged
/// ("Ludwig van Beethoven", "Beethoven", "Beethoven, Ludwig van").
pub fn known_composer(name: &str) -> Option<ComposerFacts> {
    let index = composer_index();
    let direct = normalize_artist_identity(name);
    if let Some(facts) = index.get(&direct) {
        return Some(*facts);
    }
    let (last, first) = name.split_once(',')?;
    index
        .get(&normalize_artist_identity(&format!(
            "{} {}",
            first.trim(),
            last.trim()
        )))
        .copied()
}

/// Adjective for an ISO 3166 country code, as MusicBrainz reports them.
pub fn nationality_for_country(code: &str) -> Option<&'static str> {
    Some(match code.to_ascii_uppercase().as_str() {
        "DE" => "German",
        "AT" => "Austrian",
        "FR" => "French",
        "IT" => "Italian",
        "GB" | "UK" | "XE" => "British",
        "IE" => "Irish",
        "RU" | "SU" => "Russian",
        "UA" => "Ukrainian",
        "PL" => "Polish",
        "CZ" | "XC" => "Czech",
        "SK" => "Slovak",
        "HU" => "Hungarian",
        "RO" => "Romanian",
        "BG" => "Bulgarian",
        "GR" => "Greek",
        "ES" => "Spanish",
        "PT" => "Portuguese",
        "NL" => "Dutch",
        "BE" => "Belgian",
        "CH" => "Swiss",
        "DK" => "Danish",
        "NO" => "Norwegian",
        "SE" => "Swedish",
        "FI" => "Finnish",
        "IS" => "Icelandic",
        "EE" => "Estonian",
        "LV" => "Latvian",
        "LT" => "Lithuanian",
        "AM" => "Armenian",
        "GE" => "Georgian",
        "TR" => "Turkish",
        "IL" => "Israeli",
        "US" => "American",
        "CA" => "Canadian",
        "MX" => "Mexican",
        "BR" => "Brazilian",
        "AR" => "Argentine",
        "CU" => "Cuban",
        "AU" => "Australian",
        "NZ" => "New Zealander",
        "JP" => "Japanese",
        "CN" => "Chinese",
        "KR" => "Korean",
        "IN" => "Indian",
        _ => return None,
    })
}

/// Stable id of a work from its grouping key.
pub fn work_id(key: &str) -> String {
    // FNV-1a, 64 bit: short, stable across builds and platforms.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("wk_{hash:016x}")
}

/// One indexed track as the work builder sees it.
#[derive(Clone, Debug, Default)]
pub struct WorkTrack {
    pub track_id: String,
    pub title: String,
    pub album_id: Option<String>,
    pub work: Option<String>,
    pub work_mbid: Option<String>,
    pub movement: Option<String>,
    pub movement_number: Option<i32>,
    pub composed_year: Option<i32>,
    /// Composer `(artist id, name)` pairs in credit order.
    pub composers: Vec<(String, String)>,
    /// Whether the track or its album carries a classical genre.
    pub classical: bool,
}

/// Whether a title reads like art music: a catalogue number ("Op. 61",
/// "BWV 1007", "K. 525"), a key ("in D minor", "in Es-Dur") or a numbered
/// work ("No. 5"). Titles without one are classified only when tagged or
/// filed under a classical genre, so pop songs called "Symphony" stay
/// unclassified.
pub fn looks_classical(title: &str) -> bool {
    static MARKER: OnceLock<regex::Regex> = OnceLock::new();
    MARKER
        .get_or_init(|| {
            regex::Regex::new(
                r"(?i)\b(?:op|opus|bwv|kv?|d|hob|rv|hwv|woo|sz|bb|wab|trv|fs|jw|b|l|s|no|nr|n°|nº)\.?\s*[0-9IVXL]+\b|\bin\s+[a-h](?:[- ]?(?:flat|sharp|is|es|s))?\s*[- ]?(?:major|minor|dur|moll)\b",
            )
            .expect("classical marker pattern")
        })
        .is_match(title)
}

/// What MusicBrainz says about a work.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OnlineWork {
    pub title: Option<String>,
    pub work_type: Option<String>,
    pub composed_year: Option<i32>,
    pub parent_mbid: Option<String>,
}

/// A built work with its facts and tracks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuiltWork {
    pub id: String,
    pub title: String,
    pub tagged: bool,
    pub musicbrainz_work_id: Option<String>,
    pub form: Option<String>,
    pub instrumentation: Option<String>,
    pub scoring: Option<String>,
    pub composed_year: Option<i32>,
    pub period: Option<String>,
    pub composers: Vec<String>,
    /// `(track id, movement, movement number)`.
    pub tracks: Vec<(String, Option<String>, Option<i32>)>,
}

/// Case- and punctuation-insensitive key for grouping titles.
pub fn title_key(title: &str) -> String {
    folded_words(title).trim().to_string()
}

/// Splits `Work: Movement` titles, e.g. "Symphony No. 5: I. Allegro".
pub fn split_work_title(title: &str) -> Option<(&str, &str)> {
    let (work, movement) = title.split_once(": ")?;
    let (work, movement) = (work.trim(), movement.trim());
    (work.chars().count() >= 3 && !movement.is_empty()).then_some((work, movement))
}

/// The movement label once the work title is known.
fn movement_label(title: &str, work: &str, tagged: Option<&str>) -> Option<String> {
    if let Some(tagged) = tagged.map(str::trim).filter(|value| !value.is_empty()) {
        return Some(tagged.to_string());
    }
    let (prefix, rest) = split_work_title(title)?;
    (title_key(prefix) == title_key(work)).then(|| rest.to_string())
}

/// Groups tracks into works and works out each work's facts.
///
/// `online` maps MusicBrainz work ids to what MusicBrainz says about them;
/// `composer_period` gives a composer's period for undated works.
pub fn build_works(
    tracks: &[WorkTrack],
    online: &HashMap<String, OnlineWork>,
    composer_period: &dyn Fn(&str) -> Option<String>,
) -> Vec<BuiltWork> {
    // Untagged `Work: Movement` prefixes shared by two tracks of one album.
    let mut prefixes = HashMap::<(Option<&str>, String), usize>::new();
    for track in tracks.iter().filter(|track| track.work.is_none()) {
        if let Some((prefix, _)) = split_work_title(&track.title) {
            *prefixes
                .entry((track.album_id.as_deref(), title_key(prefix)))
                .or_default() += 1;
        }
    }
    let mut works = BTreeMap::<String, BuiltWork>::new();
    for track in tracks {
        // The composer that keys the work: the first credited, by id order so
        // a co-credit listed in another order does not split the work.
        let composer = track
            .composers
            .iter()
            .map(|(id, _)| id.as_str())
            .min()
            .unwrap_or("");
        let online_work = track.work_mbid.as_deref().and_then(|mbid| online.get(mbid));
        // A tagged movement-level work id whose parent MusicBrainz knows
        // groups under the parent.
        let parent = online_work
            .and_then(|work| work.parent_mbid.as_deref())
            .and_then(|parent| online.get(parent).map(|work| (parent, work)));
        let (title, tagged, key, mbid) = if let Some(work) = track.work.as_deref() {
            let mbid = parent
                .map(|(id, _)| id.to_string())
                .or_else(|| track.work_mbid.clone());
            (
                work.to_string(),
                true,
                format!("w:{composer}|{}", title_key(work)),
                mbid,
            )
        } else if let Some((parent_id, parent_work)) =
            parent.filter(|(_, work)| work.title.is_some())
        {
            let title = parent_work.title.clone().unwrap_or_default();
            (
                title,
                true,
                format!("mb:{parent_id}"),
                Some(parent_id.to_string()),
            )
        } else if let Some(mbid) = track
            .work_mbid
            .as_deref()
            .filter(|mbid| online.get(*mbid).is_some_and(|work| work.title.is_some()))
        {
            let title = online[mbid].title.clone().unwrap_or_default();
            (title, true, format!("mb:{mbid}"), Some(mbid.to_string()))
        } else {
            let prefix = split_work_title(&track.title)
                .map(|(prefix, _)| prefix)
                .filter(|prefix| {
                    prefixes
                        .get(&(track.album_id.as_deref(), title_key(prefix)))
                        .is_some_and(|count| *count >= 2)
                });
            let title = prefix.unwrap_or(&track.title).to_string();
            let key = title_key(&title);
            let key = if key.is_empty() {
                format!("t:{}", track.track_id)
            } else {
                format!("w:{composer}|{key}")
            };
            (title, false, key, track.work_mbid.clone())
        };
        let id = work_id(&key);
        let movement = movement_label(&track.title, &title, track.movement.as_deref())
            .filter(|movement| title_key(movement) != title_key(&title));
        let work = works.entry(id.clone()).or_insert_with(|| BuiltWork {
            id,
            title: title.clone(),
            tagged,
            musicbrainz_work_id: None,
            form: None,
            instrumentation: None,
            scoring: None,
            composed_year: None,
            period: None,
            composers: Vec::new(),
            tracks: Vec::new(),
        });
        work.tagged |= tagged;
        if work.musicbrainz_work_id.is_none() {
            work.musicbrainz_work_id = mbid;
        }
        if work.composed_year.is_none() {
            work.composed_year = track
                .composed_year
                .filter(|year| (900..=2100).contains(year));
        }
        for (composer, _) in &track.composers {
            if !work.composers.contains(composer) {
                work.composers.push(composer.clone());
            }
        }
        work.tracks.push((
            track.track_id.clone(),
            movement,
            track.movement_number.filter(|number| *number > 0),
        ));
    }
    let classical_tracks = tracks
        .iter()
        .filter(|track| track.classical)
        .map(|track| track.track_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let names = tracks
        .iter()
        .flat_map(|track| track.composers.iter())
        .map(|(id, name)| (id.as_str(), name.clone()))
        .collect::<HashMap<_, _>>();
    let mut built = works.into_values().collect::<Vec<_>>();
    for work in &mut built {
        let online = work
            .musicbrainz_work_id
            .as_deref()
            .and_then(|mbid| online.get(mbid));
        let composer_names = work
            .composers
            .iter()
            .filter_map(|id| names.get(id.as_str()).cloned())
            .collect::<Vec<_>>();
        let known = known_work(&work.title, &composer_names);
        let classical = work.tagged
            || online.is_some()
            || looks_classical(&work.title)
            || work
                .tracks
                .iter()
                .any(|(id, _, _)| classical_tracks.contains(id.as_str()));
        let class = if classical {
            classify_title(&work.title)
        } else {
            WorkClass::default()
        };
        let online_form = online
            .and_then(|online| online.work_type.as_deref())
            .and_then(form_from_musicbrainz_type);
        let form = known.map(|known| known.form).or(online_form).or(class.form);
        // The title's instrument words still refine a MusicBrainz form.
        let (instrumentation, scoring) = match known {
            Some(known) => (
                Some(known.instrumentation),
                known.scoring.map(str::to_string),
            ),
            None if form != class.form && classical => {
                classify_instrumentation(&folded_words(&work.title), form)
            }
            None => (class.instrumentation, class.scoring.clone()),
        };
        work.form = form.map(str::to_string);
        work.instrumentation = instrumentation.map(str::to_string);
        work.scoring = scoring;
        work.composed_year = work
            .composed_year
            .or_else(|| online.and_then(|online| online.composed_year))
            .or(known.map(|known| known.year))
            .or(class.composed_year);
        work.period = work
            .composed_year
            .and_then(period_for_year)
            .map(str::to_string)
            .or_else(|| work.composers.iter().find_map(|id| composer_period(id)));
    }
    built
}

#[derive(QueryableByName)]
struct WorkTrackRow {
    #[diesel(sql_type = Text)]
    track_id: String,
    #[diesel(sql_type = Text)]
    title: String,
    #[diesel(sql_type = Nullable<Text>)]
    album_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    work: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    work_mbid: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    movement: Option<String>,
    #[diesel(sql_type = Nullable<Integer>)]
    movement_number: Option<i32>,
    #[diesel(sql_type = Nullable<Integer>)]
    composed_year: Option<i32>,
    #[diesel(sql_type = Nullable<Text>)]
    composer_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    composer_name: Option<String>,
    #[diesel(sql_type = Integer)]
    classical: i32,
}

#[derive(QueryableByName)]
struct OnlineWorkRow {
    #[diesel(sql_type = Text)]
    mbid: String,
    #[diesel(sql_type = Text)]
    title: String,
    #[diesel(sql_type = Nullable<Text>)]
    work_type: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    composed: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    parent_mbid: Option<String>,
}

#[derive(QueryableByName)]
struct ComposerRow {
    #[diesel(sql_type = Text)]
    artist_id: String,
    #[diesel(sql_type = Text)]
    name: String,
    /// From the artist's MusicBrainz profile.
    #[diesel(sql_type = Nullable<Text>)]
    begin: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    end: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    area: Option<String>,
    /// Facts an earlier MusicBrainz lookup stored for the composer.
    #[diesel(sql_type = Nullable<Integer>)]
    kept_birth: Option<i32>,
    #[diesel(sql_type = Nullable<Integer>)]
    kept_death: Option<i32>,
    #[diesel(sql_type = Nullable<Text>)]
    kept_country: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    kept_nationality: Option<String>,
}

/// The first plausible four-digit year in a date or range, e.g. 1822 in
/// `1822–1824` or `1822-05-07`.
fn first_year(text: &str) -> Option<i32> {
    let bytes = text.as_bytes();
    (0..bytes.len().saturating_sub(3))
        .filter(|start| {
            bytes[*start..*start + 4].iter().all(u8::is_ascii_digit)
                && (*start == 0 || !bytes[*start - 1].is_ascii_digit())
                && bytes
                    .get(*start + 4)
                    .is_none_or(|next| !next.is_ascii_digit())
        })
        .find_map(|start| text[start..start + 4].parse::<i32>().ok())
        .filter(|year| (500..=2100).contains(year))
}

/// Adjective for a MusicBrainz area name (an artist profile's `area`).
pub fn nationality_for_area(area: &str) -> Option<&'static str> {
    let code = match folded_words(area).trim() {
        "germany" | "prussia" | "east germany" | "west germany" => "DE",
        "austria" | "austria hungary" => "AT",
        "france" => "FR",
        "italy" => "IT",
        "united kingdom" | "england" | "scotland" | "wales" | "great britain" => "GB",
        "ireland" => "IE",
        "russia" | "soviet union" | "russian empire" => "RU",
        "ukraine" => "UA",
        "poland" => "PL",
        "czech republic" | "czechia" | "czechoslovakia" | "bohemia" => "CZ",
        "slovakia" => "SK",
        "hungary" => "HU",
        "romania" => "RO",
        "bulgaria" => "BG",
        "greece" => "GR",
        "spain" => "ES",
        "portugal" => "PT",
        "netherlands" => "NL",
        "belgium" => "BE",
        "switzerland" => "CH",
        "denmark" => "DK",
        "norway" => "NO",
        "sweden" => "SE",
        "finland" => "FI",
        "iceland" => "IS",
        "estonia" => "EE",
        "latvia" => "LV",
        "lithuania" => "LT",
        "armenia" => "AM",
        "georgia" => "GE",
        "turkey" => "TR",
        "israel" => "IL",
        "united states" | "usa" => "US",
        "canada" => "CA",
        "mexico" => "MX",
        "brazil" => "BR",
        "argentina" => "AR",
        "cuba" => "CU",
        "australia" => "AU",
        "new zealand" => "NZ",
        "japan" => "JP",
        "china" => "CN",
        "south korea" | "korea" => "KR",
        "india" => "IN",
        _ => return None,
    };
    nationality_for_country(code)
}

#[derive(QueryableByName)]
struct FingerprintRow {
    #[diesel(sql_type = Text)]
    fingerprint: String,
}

/// Genres whose tracks are art music.
const CLASSICAL_GENRES: &str = "ge.normalized_name LIKE '%classical%' \
    OR ge.normalized_name LIKE '%baroque%' OR ge.normalized_name LIKE '%opera%' \
    OR ge.normalized_name LIKE '%choral%' OR ge.normalized_name LIKE '%orchestral%' \
    OR ge.normalized_name LIKE '%chamber%' OR ge.normalized_name LIKE '%symphon%' \
    OR ge.normalized_name IN ('early music', 'romantic', 'renaissance', 'medieval', 'lieder', 'art song')";

/// What a rebuild reads: the catalog, MusicBrainz's facts about works and
/// its album links, and composers' artist profiles. A rebuild writes none
/// of these (it never touches `updated_at` or MusicBrainz rows' facts).
const FINGERPRINT_SQL: &str = "SELECT \
    (SELECT COUNT(*) || ':' || COALESCE(MAX(updated_at), '') FROM track_entity) || '|' || \
    (SELECT COUNT(*) || ':' || COALESCE(MAX(parsed_at), '') FROM raw_file_metadata) || '|' || \
    (SELECT COUNT(*) FROM track_artist WHERE role = 'composer') || '|' || \
    (SELECT COUNT(*) || ':' || COALESCE(MAX(updated_at), '') FROM work \
       WHERE musicbrainz_facts = 1) || '|' || \
    (SELECT COUNT(*) || ':' || COALESCE(MAX(rowid), 0) FROM track_work \
       WHERE source = 'musicbrainz') || '|' || \
    (SELECT COUNT(*) || ':' || COALESCE(MAX(updated_at), '') FROM editorial_profile \
       WHERE entity_type = 'artist') || '|' || ? \
    AS fingerprint";

/// Bumped whenever classification rules or tables change, so existing
/// libraries rebuild their works with the new rules.
const WORK_BUILDER_VERSION: &str = "3";

/// Rebuilds the work tables when the catalog changed since the last build.
/// Returns whether a rebuild ran.
///
/// On a database file the check runs at most every few seconds; in-memory
/// databases (tests) always check.
pub fn ensure_works(connection: &mut SqliteConnection) -> QueryResult<bool> {
    static CHECKED: OnceLock<std::sync::Mutex<HashMap<String, std::time::Instant>>> =
        OnceLock::new();
    const CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);
    let path = diesel::sql_query(
        "SELECT file AS fingerprint FROM pragma_database_list WHERE name = 'main'",
    )
    .get_result::<FingerprintRow>(connection)
    .optional()?
    .map(|row| row.fingerprint)
    .unwrap_or_default();
    if !path.is_empty() {
        let mut checked = CHECKED
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if checked
            .get(&path)
            .is_some_and(|at| at.elapsed() < CHECK_INTERVAL)
        {
            return Ok(false);
        }
        checked.insert(path, std::time::Instant::now());
    }
    let stale = |connection: &mut SqliteConnection| -> QueryResult<Option<String>> {
        let current = diesel::sql_query(FINGERPRINT_SQL)
            .bind::<Text, _>(WORK_BUILDER_VERSION)
            .get_result::<FingerprintRow>(connection)?
            .fingerprint;
        let stored =
            diesel::sql_query("SELECT fingerprint FROM work_catalog_state WHERE singleton = 1")
                .get_result::<FingerprintRow>(connection)
                .optional()?
                .map(|row| row.fingerprint);
        Ok((stored.as_deref() != Some(current.as_str())).then_some(current))
    };
    if stale(connection)?.is_none() {
        return Ok(false);
    }
    connection.immediate_transaction(|connection| {
        // Another request may have rebuilt while this one waited.
        let Some(current) = stale(connection)? else {
            return Ok(false);
        };
        rebuild_works(connection)?;
        diesel::sql_query(
            "INSERT INTO work_catalog_state(singleton, fingerprint) VALUES (1, ?) \
             ON CONFLICT(singleton) DO UPDATE SET fingerprint = excluded.fingerprint, \
             built_at = CURRENT_TIMESTAMP",
        )
        .bind::<Text, _>(&current)
        .execute(connection)?;
        Ok(true)
    })
}

/// Rebuilds the library's works (`work` rows marked `in_library`, their
/// `library` track links, `work_composer` and `composer_entity`) from the
/// catalog, MusicBrainz's facts and composers' profiles.
pub fn rebuild_works(connection: &mut SqliteConnection) -> QueryResult<()> {
    // A track's work id: its tags first, then MusicBrainz's album match.
    let rows = diesel::sql_query(format!(
        "SELECT t.id AS track_id, t.title AS title, t.album_id AS album_id, \
           NULLIF(trim(pm.work), '') AS work, \
           COALESCE(pm.work_mbid, (SELECT l.work_mbid FROM track_work l \
             WHERE l.track_id = t.id AND l.source = 'musicbrainz' \
             ORDER BY l.rowid LIMIT 1)) AS work_mbid, \
           NULLIF(trim(pm.movement), '') AS movement, \
           CAST(pm.movement_number AS INTEGER) AS movement_number, \
           CAST(pm.composed_year AS INTEGER) AS composed_year, \
           ta.artist_id AS composer_id, ar.name AS composer_name, \
           EXISTS (SELECT 1 FROM genre_entity ge WHERE ({CLASSICAL_GENRES}) AND ge.id IN ( \
             SELECT genre_id FROM track_genre WHERE track_id = t.id \
             UNION SELECT genre_id FROM album_genre WHERE album_id = t.album_id)) AS classical \
         FROM track_entity t \
         LEFT JOIN track_file pf ON pf.track_id = t.id AND pf.is_primary = 1 \
         LEFT JOIN raw_file_metadata pm ON pm.file_id = pf.file_id \
         LEFT JOIN track_artist ta ON ta.track_id = t.id AND ta.role = 'composer' \
         LEFT JOIN artist_entity ar ON ar.id = ta.artist_id \
         WHERE ta.artist_id IS NOT NULL OR NULLIF(trim(pm.work), '') IS NOT NULL \
           OR pm.work_mbid IS NOT NULL \
           OR EXISTS (SELECT 1 FROM track_work l WHERE l.track_id = t.id \
                      AND l.source = 'musicbrainz') \
         ORDER BY t.album_id, t.disc_number, t.track_number, t.id, ta.position, ar.name"
    ))
    .load::<WorkTrackRow>(connection)?;
    let mut tracks = Vec::<WorkTrack>::new();
    for row in rows {
        if tracks
            .last()
            .is_none_or(|track| track.track_id != row.track_id)
        {
            tracks.push(WorkTrack {
                track_id: row.track_id.clone(),
                title: row.title.clone(),
                album_id: row.album_id.clone(),
                work: row.work.clone(),
                work_mbid: row.work_mbid.clone(),
                movement: row.movement.clone(),
                movement_number: row.movement_number,
                composed_year: row.composed_year,
                composers: Vec::new(),
                classical: row.classical != 0,
            });
        }
        if let (Some(id), Some(name), Some(track)) =
            (row.composer_id, row.composer_name, tracks.last_mut())
            && !track.composers.iter().any(|(known, _)| *known == id)
        {
            track.composers.push((id, name));
        }
    }

    // What MusicBrainz says about works (album matching, composer
    // catalogues and work lookups).
    let online = diesel::sql_query(
        "SELECT mbid, title, form AS work_type, composed, parent_mbid FROM work \
         WHERE musicbrainz_facts = 1 AND title <> ''",
    )
    .load::<OnlineWorkRow>(connection)?
    .into_iter()
    .map(|row| {
        (
            row.mbid,
            OnlineWork {
                title: Some(row.title),
                work_type: row.work_type,
                composed_year: row.composed.as_deref().and_then(first_year),
                parent_mbid: row.parent_mbid,
            },
        )
    })
    .collect::<HashMap<_, _>>();

    // Composer facts: the artist's MusicBrainz profile, then what an
    // earlier MusicBrainz lookup stored, then the built-in table.
    let composers = diesel::sql_query(
        "SELECT ar.id AS artist_id, ar.name AS name, \
           json_extract(p.profile_json, '$.begin') AS begin, \
           json_extract(p.profile_json, '$.end') AS end, \
           json_extract(p.profile_json, '$.area') AS area, \
           ce.birth_year AS kept_birth, ce.death_year AS kept_death, \
           ce.country AS kept_country, ce.nationality AS kept_nationality \
         FROM artist_entity ar \
         LEFT JOIN editorial_profile p ON p.entity_type = 'artist' AND p.entity_id = ar.id \
           AND json_valid(p.profile_json) \
         LEFT JOIN composer_entity ce ON ce.artist_id = ar.id AND ce.source = 'musicbrainz' \
         WHERE ar.id IN (SELECT artist_id FROM track_artist WHERE role = 'composer') \
         GROUP BY ar.id",
    )
    .load::<ComposerRow>(connection)?;
    struct Profile {
        birth: Option<i32>,
        death: Option<i32>,
        nationality: Option<String>,
        country: Option<String>,
        period: Option<String>,
        source: &'static str,
    }
    let mut profiles = HashMap::<String, Profile>::new();
    for row in composers {
        let known = known_composer(&row.name);
        let profile_birth = row.begin.as_deref().and_then(first_year);
        let known_nationality = || known.map(|known| known.nationality.to_string());
        let (birth, death, nationality, country, source) = if profile_birth.is_some() {
            (
                profile_birth,
                row.end.as_deref().and_then(first_year),
                row.area
                    .as_deref()
                    .and_then(nationality_for_area)
                    .map(str::to_string)
                    .or_else(known_nationality),
                None,
                "musicbrainz",
            )
        } else if row.kept_birth.is_some() {
            (
                row.kept_birth,
                row.kept_death,
                row.kept_country
                    .as_deref()
                    .and_then(nationality_for_country)
                    .map(str::to_string)
                    .or(row.kept_nationality)
                    .or_else(known_nationality),
                row.kept_country,
                "musicbrainz",
            )
        } else if let Some(known) = known {
            (
                Some(known.birth),
                known.death,
                Some(known.nationality.to_string()),
                None,
                "builtin",
            )
        } else {
            (None, None, None, None, "derived")
        };
        profiles.insert(
            row.artist_id,
            Profile {
                birth,
                death,
                nationality,
                country,
                period: composer_period(birth, death).map(str::to_string),
                source,
            },
        );
    }
    let period_of = |id: &str| profiles.get(id).and_then(|profile| profile.period.clone());
    let works = build_works(&tracks, &online, &period_of);

    // A composer without life dates takes the period most of their dated
    // works share.
    let mut derived = HashMap::<String, BTreeMap<String, usize>>::new();
    for work in &works {
        if let Some(period) = work.composed_year.and_then(period_for_year) {
            for composer in &work.composers {
                *derived
                    .entry(composer.clone())
                    .or_default()
                    .entry(period.to_string())
                    .or_default() += 1;
            }
        }
    }
    for (composer, periods) in derived {
        if let Some(profile) = profiles.get_mut(&composer)
            && profile.period.is_none()
        {
            profile.period = periods
                .into_iter()
                .max_by_key(|(_, count)| *count)
                .map(|(period, _)| period);
        }
    }

    // A work keeps its MusicBrainz id as its id; works grouped under one
    // id (a tagged title and an untagged one of the same MusicBrainz work)
    // become one.
    let mut merged = BTreeMap::<String, BuiltWork>::new();
    for work in works {
        let id = work
            .musicbrainz_work_id
            .clone()
            .unwrap_or_else(|| work.id.clone());
        match merged.get_mut(&id) {
            Some(known) => {
                known.tagged |= work.tagged;
                for composer in work.composers {
                    if !known.composers.contains(&composer) {
                        known.composers.push(composer);
                    }
                }
                known.tracks.extend(work.tracks);
                known.form = known.form.take().or(work.form);
                known.instrumentation = known.instrumentation.take().or(work.instrumentation);
                known.scoring = known.scoring.take().or(work.scoring);
                known.composed_year = known.composed_year.or(work.composed_year);
                known.period = known.period.take().or(work.period);
            }
            None => {
                merged.insert(id.clone(), BuiltWork { id, ..work });
            }
        }
    }
    let albums = tracks
        .iter()
        .map(|track| (track.track_id.as_str(), track.album_id.as_deref()))
        .collect::<HashMap<_, _>>();
    let names = tracks
        .iter()
        .flat_map(|track| track.composers.iter())
        .map(|(id, name)| (id.as_str(), name.as_str()))
        .collect::<HashMap<_, _>>();

    // Works only the library knew go; MusicBrainz's stay, out of the
    // library until a track performs them again.
    connection.batch_execute(
        "DELETE FROM track_work WHERE source = 'library'; \
         DELETE FROM work_composer; \
         DELETE FROM composer_entity; \
         CREATE TEMP TABLE IF NOT EXISTS work_previous (id TEXT PRIMARY KEY, created_at DATETIME); \
         DELETE FROM work_previous; \
         INSERT INTO work_previous SELECT mbid, created_at FROM work WHERE in_library = 1; \
         DELETE FROM work WHERE musicbrainz_facts = 0; \
         UPDATE work SET in_library = 0, tagged = 0, instrumentation = NULL, scoring = NULL, \
           composed_year = NULL, period = NULL WHERE in_library = 1;",
    )?;
    for work in merged.values() {
        let composer = work
            .composers
            .first()
            .and_then(|id| names.get(id.as_str()).copied());
        diesel::sql_query(
            "INSERT INTO work(mbid, title, form, instrumentation, scoring, composed_year, \
               period, tagged, in_library, composer_name, composer_normalized, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?, \
               COALESCE((SELECT created_at FROM work_previous WHERE id = ?), CURRENT_TIMESTAMP)) \
             ON CONFLICT(mbid) DO UPDATE SET \
               form = COALESCE(excluded.form, work.form), \
               instrumentation = excluded.instrumentation, scoring = excluded.scoring, \
               composed_year = excluded.composed_year, period = excluded.period, \
               tagged = excluded.tagged, in_library = 1, \
               composer_name = COALESCE(work.composer_name, excluded.composer_name), \
               composer_normalized = COALESCE(work.composer_normalized, excluded.composer_normalized), \
               created_at = COALESCE(work.created_at, excluded.created_at)",
        )
        .bind::<Text, _>(&work.id)
        .bind::<Text, _>(&work.title)
        .bind::<Nullable<Text>, _>(work.form.as_deref())
        .bind::<Nullable<Text>, _>(work.instrumentation.as_deref())
        .bind::<Nullable<Text>, _>(work.scoring.as_deref())
        .bind::<Nullable<Integer>, _>(work.composed_year)
        .bind::<Nullable<Text>, _>(work.period.as_deref())
        .bind::<Integer, _>(i32::from(work.tagged))
        .bind::<Nullable<Text>, _>(composer)
        .bind::<Nullable<Text>, _>(composer.map(normalize_artist_identity))
        .bind::<Text, _>(&work.id)
        .execute(connection)?;
        for (position, composer) in work.composers.iter().enumerate() {
            diesel::sql_query(
                "INSERT INTO work_composer(work_id, artist_id, position) VALUES (?, ?, ?) \
                 ON CONFLICT DO NOTHING",
            )
            .bind::<Text, _>(&work.id)
            .bind::<Text, _>(composer)
            .bind::<Integer, _>(i32::try_from(position).unwrap_or(i32::MAX))
            .execute(connection)?;
        }
        for (track, movement, number) in &work.tracks {
            diesel::sql_query(
                "INSERT INTO track_work(track_id, album_id, work_mbid, movement, \
                   movement_number, source) VALUES (?, ?, ?, ?, ?, 'library') \
                 ON CONFLICT DO NOTHING",
            )
            .bind::<Text, _>(track)
            .bind::<Text, _>(albums.get(track.as_str()).copied().flatten().unwrap_or(""))
            .bind::<Text, _>(&work.id)
            .bind::<Nullable<Text>, _>(movement.as_deref())
            .bind::<Nullable<Integer>, _>(*number)
            .execute(connection)?;
        }
    }
    for (artist_id, profile) in profiles {
        diesel::sql_query(
            "INSERT INTO composer_entity(artist_id, birth_year, death_year, nationality, \
               country, period, source) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind::<Text, _>(&artist_id)
        .bind::<Nullable<Integer>, _>(profile.birth)
        .bind::<Nullable<Integer>, _>(profile.death)
        .bind::<Nullable<Text>, _>(profile.nationality.as_deref())
        .bind::<Nullable<Text>, _>(profile.country.as_deref())
        .bind::<Nullable<Text>, _>(profile.period.as_deref())
        .bind::<Text, _>(profile.source)
        .execute(connection)?;
    }
    // Works the tags name by MusicBrainz id but MusicBrainz has not been
    // asked about yet are looked up by metadata enrichment.
    connection.batch_execute(
        "INSERT INTO metadata_task (provider, entity_type, entity_id) \
         SELECT 'musicbrainz', 'work', mbid FROM work \
         WHERE in_library = 1 AND musicbrainz_facts = 0 AND length(mbid) = 36 \
         ON CONFLICT(provider, entity_type, entity_id) DO NOTHING;",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(title: &str) -> (Option<&'static str>, Option<&'static str>, Option<String>) {
        let class = classify_title(title);
        (class.form, class.instrumentation, class.scoring)
    }

    #[test]
    fn titles_classify_into_form_and_instrumentation() {
        assert_eq!(
            class("Symphony No. 7 in A major, Op. 92"),
            (Some("Symphony"), Some("Orchestra"), None)
        );
        assert_eq!(
            class("Violin Concerto in D major, Op. 61"),
            (
                Some("Concerto"),
                Some("Solo Instrument & Orchestra"),
                Some("Violin & Orchestra".into())
            )
        );
        assert_eq!(
            class("Concerto for Orchestra, Sz. 116"),
            (Some("Concerto"), Some("Orchestra"), None)
        );
        assert_eq!(
            class("Piano Sonata No. 23 in F minor \"Appassionata\""),
            (Some("Sonata"), Some("Solo Keyboard"), Some("Piano".into()))
        );
        assert_eq!(
            class("Sonata for Violin and Piano No. 9 \"Kreutzer\""),
            (
                Some("Sonata"),
                Some("Instrument & Keyboard"),
                Some("Violin & Piano".into())
            )
        );
        assert_eq!(
            class("Cello Sonata No. 3 in A major"),
            (
                Some("Sonata"),
                Some("Instrument & Keyboard"),
                Some("Cello & Keyboard".into())
            )
        );
        assert_eq!(
            class("Cello Suite No. 1 in G major, BWV 1007"),
            (Some("Suite"), Some("Solo Instrument"), Some("Cello".into()))
        );
        assert_eq!(
            class("Partita for Solo Violin No. 2, BWV 1004"),
            (
                Some("Partita"),
                Some("Solo Instrument"),
                Some("Violin".into())
            )
        );
        assert_eq!(
            class("Piano Trio No. 7 \"Archduke\""),
            (Some("Trio"), Some("Keyboard Trio"), None)
        );
        assert_eq!(
            class("String Quartet No. 14 in C-sharp minor, Op. 131"),
            (Some("String Quartet"), Some("String Quartet"), None)
        );
        assert_eq!(
            class("Streichquartett Nr. 2"),
            (Some("String Quartet"), Some("String Quartet"), None)
        );
        assert_eq!(
            class("Nocturne in E-flat major, Op. 9 No. 2"),
            (Some("Nocturne"), Some("Solo Keyboard"), None)
        );
        assert_eq!(
            class("Messa da Requiem"),
            (Some("Requiem"), Some("Choir & Orchestra"), None)
        );
        assert_eq!(
            class("Mass in B minor, BWV 232"),
            (Some("Mass"), Some("Choir & Orchestra"), None)
        );
        assert_eq!(
            class("An die Musik, D. 547 (song)"),
            (Some("Song"), Some("Voice & Keyboard"), None)
        );
        assert_eq!(
            class("Symphonie fantastique, Op. 14"),
            (Some("Symphony"), Some("Orchestra"), None)
        );
        assert_eq!(
            class("Clarinet Quintet in A major, K. 581"),
            (Some("Quintet"), Some("Chamber Ensemble"), None)
        );
        assert_eq!(
            class("Piano Quintet in F minor"),
            (Some("Quintet"), Some("Keyboard Quintet"), None)
        );
        assert_eq!(class("Yesterday"), (None, None, None));
    }

    #[test]
    fn only_art_music_titles_are_classified_without_tags() {
        assert!(looks_classical("Symphony No. 5 in C minor"));
        assert!(looks_classical("Prelude in C, BWV 846"));
        assert!(looks_classical("Konzert in Es-Dur"));
        assert!(!looks_classical("Symphony"));
        assert!(!looks_classical("Waltz #2 (XO)"));
        let mut pop = track("p1", "Symphony", "pop", None);
        pop.composers = vec![("writer".into(), "Pop Writer".into())];
        let mut classical = pop.clone();
        classical.track_id = "c1".into();
        classical.title = "Sinfonia".into();
        classical.album_id = Some("box".into());
        classical.classical = true;
        let works = build_works(&[pop, classical], &HashMap::new(), &|_| None);
        let symphony = works
            .iter()
            .find(|work| work.title == "Symphony")
            .expect("pop");
        assert_eq!(symphony.form, None);
        let sinfonia = works
            .iter()
            .find(|work| work.title == "Sinfonia")
            .expect("art");
        assert_eq!(sinfonia.form.as_deref(), Some("Symphony"));
    }

    #[test]
    fn composition_years_come_from_the_title() {
        assert_eq!(classify_title("Overture (1812)").composed_year, Some(1812));
        assert_eq!(
            classify_title("Fantasia (c. 1720), BWV 906").composed_year,
            Some(1720)
        );
        assert_eq!(classify_title("Prelude, BWV 1006").composed_year, None);
        assert_eq!(classify_title("Op. 1812").composed_year, None);
    }

    #[test]
    fn periods_follow_composition_years_and_composer_lives() {
        assert_eq!(period_for_year(1721), Some("Baroque"));
        assert_eq!(period_for_year(1812), Some("Classical"));
        assert_eq!(period_for_year(1876), Some("Romantic"));
        assert_eq!(period_for_year(1913), Some("Modern"));
        assert_eq!(period_for_year(1990), Some("Contemporary"));
        assert_eq!(composer_period(Some(1685), Some(1750)), Some("Baroque"));
        assert_eq!(composer_period(Some(1756), Some(1791)), Some("Classical"));
        assert_eq!(composer_period(Some(1810), Some(1849)), Some("Romantic"));
        assert_eq!(composer_period(Some(1882), Some(1971)), Some("Modern"));
        assert_eq!(composer_period(Some(1935), None), Some("Contemporary"));
        assert_eq!(composer_period(None, Some(1900)), None);
    }

    #[test]
    fn canonical_composers_match_common_spellings() {
        let beethoven = known_composer("Ludwig van Beethoven").expect("beethoven");
        assert_eq!(beethoven.birth, 1770);
        assert_eq!(beethoven.nationality, "German");
        assert_eq!(known_composer("Beethoven"), Some(beethoven));
        assert_eq!(known_composer("Beethoven, Ludwig van"), Some(beethoven));
        assert_eq!(
            known_composer("Antonín Dvořák").map(|facts| facts.nationality),
            Some("Czech")
        );
        assert_eq!(
            known_composer("Dvorak").map(|facts| facts.birth),
            Some(1841)
        );
        assert_eq!(
            known_composer("J.S. Bach").map(|facts| facts.birth),
            Some(1685)
        );
        assert_eq!(known_composer("Nora Kelly"), None);
        assert_eq!(nationality_for_country("at"), Some("Austrian"));
        assert_eq!(nationality_for_country("ZZ"), None);
    }

    fn track(id: &str, title: &str, album: &str, work: Option<&str>) -> WorkTrack {
        WorkTrack {
            track_id: id.into(),
            title: title.into(),
            album_id: Some(album.into()),
            work: work.map(str::to_string),
            composers: vec![("beethoven".into(), "Ludwig van Beethoven".into())],
            ..WorkTrack::default()
        }
    }

    #[test]
    fn works_group_by_work_tag_and_title_prefix_and_carry_facts() {
        let mut tracks = vec![
            track(
                "t1",
                "I. Poco sostenuto",
                "a1",
                Some("Symphony No. 7 in A major, Op. 92"),
            ),
            track(
                "t2",
                "II. Allegretto",
                "a1",
                Some("Symphony No. 7 in A major, Op. 92"),
            ),
            // Another recording of the same work, tagged with other punctuation.
            track(
                "t3",
                "Allegretto",
                "a2",
                Some("Symphony No. 7 in A Major Op 92"),
            ),
            track("t4", "Für Elise: Poco moto", "a2", None),
            track("t5", "Für Elise: Reprise", "a2", None),
            track("t6", "Romance", "a3", None),
        ];
        tracks[2].composed_year = Some(1812);
        tracks[5].composers = vec![("nora".into(), "Nora Kelly".into())];
        let periods = |id: &str| (id == "beethoven").then(|| "Classical".to_string());
        let works = build_works(&tracks, &HashMap::new(), &periods);
        assert_eq!(works.len(), 3);
        let symphony = works
            .iter()
            .find(|work| work.title.starts_with("Symphony"))
            .expect("symphony");
        assert_eq!(symphony.tracks.len(), 3);
        assert_eq!(symphony.form.as_deref(), Some("Symphony"));
        assert_eq!(symphony.instrumentation.as_deref(), Some("Orchestra"));
        assert_eq!(symphony.composed_year, Some(1812));
        assert_eq!(symphony.period.as_deref(), Some("Classical"));
        assert!(symphony.tagged);
        let elise = works
            .iter()
            .find(|work| work.title == "Für Elise")
            .expect("für elise");
        assert_eq!(elise.form.as_deref(), Some("Bagatelle"));
        assert_eq!(elise.composed_year, Some(1810));
        assert_eq!(elise.tracks[0].1.as_deref(), Some("Poco moto"));
        let romance = works
            .iter()
            .find(|work| work.title == "Romance")
            .expect("romance");
        // An untagged pop title is not classified.
        assert_eq!(romance.form, None);
        assert_eq!(romance.period, None);
        assert_eq!(work_id("w:a|b"), work_id("w:a|b"));
        assert_ne!(work_id("w:a|b"), work_id("w:a|c"));
    }

    #[test]
    fn musicbrainz_facts_and_parent_works_win_over_the_title() {
        let mut movement = track("t1", "Allegro", "a1", None);
        movement.work_mbid = Some("child".into());
        let mut other = track("t2", "Adagio", "a1", None);
        other.work_mbid = Some("child-2".into());
        let online = HashMap::from([
            (
                "child".to_string(),
                OnlineWork {
                    title: Some("Serenade: I. Allegro".into()),
                    parent_mbid: Some("parent".into()),
                    ..OnlineWork::default()
                },
            ),
            (
                "child-2".to_string(),
                OnlineWork {
                    title: Some("Serenade: II. Adagio".into()),
                    parent_mbid: Some("parent".into()),
                    ..OnlineWork::default()
                },
            ),
            (
                "parent".to_string(),
                OnlineWork {
                    title: Some("Gran Partita".into()),
                    work_type: Some("Serenade".into()),
                    composed_year: Some(1781),
                    parent_mbid: None,
                },
            ),
        ]);
        let works = build_works(&[movement, other], &online, &|_| None);
        assert_eq!(works.len(), 1);
        assert_eq!(works[0].title, "Gran Partita");
        assert_eq!(works[0].form.as_deref(), Some("Serenade"));
        assert_eq!(works[0].composed_year, Some(1781));
        assert_eq!(works[0].period.as_deref(), Some("Classical"));
        assert_eq!(works[0].musicbrainz_work_id.as_deref(), Some("parent"));
    }
}
