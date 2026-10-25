//! A small canonical genre tree with aliases.
//!
//! File tags and MusicBrainz both name genres freely ("Bop", "hard-bop",
//! "Jazz Music", "R&B/Soul"). Each raw name is normalised to one node of
//! this tree, so "Bop" becomes Jazz > Bebop and an album tagged "Hard Bop"
//! also shows up on the Jazz page. Names the tree does not know are kept as
//! they are, as genres without a parent.
//!
//! Every node carries the English Wikipedia article that describes it. The
//! genre page's About text is looked up through Wikidata from that title,
//! with the same opt-in pipeline (and attribution) as biographies.

use std::collections::HashSet;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenreNode {
    pub slug: &'static str,
    pub name: &'static str,
    pub parent: Option<&'static str>,
    /// Other spellings, compared after normalisation.
    pub aliases: &'static [&'static str],
    /// The English Wikipedia article about the genre.
    pub wikipedia: &'static str,
}

const fn node(
    slug: &'static str,
    name: &'static str,
    parent: Option<&'static str>,
    aliases: &'static [&'static str],
    wikipedia: &'static str,
) -> GenreNode {
    GenreNode {
        slug,
        name,
        parent,
        aliases,
        wikipedia,
    }
}

const JAZZ: Option<&str> = Some("jazz");
const CLASSICAL: Option<&str> = Some("classical");
const ROCK: Option<&str> = Some("rock");
const METAL: Option<&str> = Some("metal");
const POP: Option<&str> = Some("pop");
const ELECTRONIC: Option<&str> = Some("electronic");
const HIP_HOP: Option<&str> = Some("hip-hop");
const RNB: Option<&str> = Some("rnb");
const BLUES: Option<&str> = Some("blues");
const COUNTRY: Option<&str> = Some("country");
const FOLK: Option<&str> = Some("folk");
const LATIN: Option<&str> = Some("latin");
const REGGAE: Option<&str> = Some("reggae");
const WORLD: Option<&str> = Some("world");
const SOUNDTRACK: Option<&str> = Some("soundtrack");

/// The tree: top-level genres first, then their sub-genres.
pub const TREE: &[GenreNode] = &[
    node("jazz", "Jazz", None, &["jazz music"], "Jazz"),
    node(
        "classical",
        "Classical",
        None,
        &["classical music", "klassik", "art music"],
        "Classical music",
    ),
    node("rock", "Rock", None, &["rock music"], "Rock music"),
    node(
        "metal",
        "Metal",
        None,
        &["heavy metal", "heavy metal music"],
        "Heavy metal music",
    ),
    node("pop", "Pop", None, &["pop music", "popular"], "Pop music"),
    node(
        "electronic",
        "Electronic",
        None,
        &["electronic music", "electronica"],
        "Electronic music",
    ),
    node(
        "hip-hop",
        "Hip Hop",
        None,
        &["hip hop music", "hiphop", "rap", "rap music", "hip hop rap"],
        "Hip hop music",
    ),
    node(
        "rnb",
        "R&B",
        None,
        &["rhythm and blues", "r and b", "rnb", "r b"],
        "Rhythm and blues",
    ),
    node("blues", "Blues", None, &["blues music"], "Blues"),
    node(
        "country",
        "Country",
        None,
        &["country music", "country and western"],
        "Country music",
    ),
    node("folk", "Folk", None, &["folk music"], "Folk music"),
    node(
        "latin",
        "Latin",
        None,
        &["latin music", "latin american music"],
        "Latin music",
    ),
    node("reggae", "Reggae", None, &["reggae music"], "Reggae"),
    node(
        "world",
        "World",
        None,
        &["world music", "ethnic", "international"],
        "World music",
    ),
    node(
        "soundtrack",
        "Soundtrack",
        None,
        &[
            "soundtracks",
            "ost",
            "original soundtrack",
            "stage and screen",
        ],
        "Soundtrack",
    ),
    node(
        "gospel",
        "Gospel",
        None,
        &["gospel music", "christian", "religious"],
        "Gospel music",
    ),
    node(
        "easy-listening",
        "Easy Listening",
        None,
        &["lounge", "easy"],
        "Easy listening",
    ),
    node(
        "new-age",
        "New Age",
        None,
        &["new age music", "newage"],
        "New-age music",
    ),
    node(
        "experimental",
        "Experimental",
        None,
        &["experimental music", "avant garde", "avantgarde"],
        "Experimental music",
    ),
    node(
        "childrens",
        "Children's Music",
        None,
        &["children", "childrens", "kids", "children s music"],
        "Children's music",
    ),
    // Jazz
    node("bebop", "Bebop", JAZZ, &["bop", "be bop"], "Bebop"),
    node("hard-bop", "Hard Bop", JAZZ, &[], "Hard bop"),
    node("post-bop", "Post-Bop", JAZZ, &[], "Post-bop"),
    node(
        "modal-jazz",
        "Modal Jazz",
        JAZZ,
        &["modal", "modal music"],
        "Modal jazz",
    ),
    node(
        "cool-jazz",
        "Cool Jazz",
        JAZZ,
        &["west coast jazz"],
        "Cool jazz",
    ),
    node("free-jazz", "Free Jazz", JAZZ, &[], "Free jazz"),
    node(
        "avant-garde-jazz",
        "Avant-Garde Jazz",
        JAZZ,
        &[],
        "Avant-garde jazz",
    ),
    node(
        "jazz-fusion",
        "Jazz Fusion",
        JAZZ,
        &["fusion", "jazz rock"],
        "Jazz fusion",
    ),
    node("soul-jazz", "Soul Jazz", JAZZ, &[], "Soul jazz"),
    node(
        "latin-jazz",
        "Latin Jazz",
        JAZZ,
        &["afro cuban jazz"],
        "Latin jazz",
    ),
    node(
        "vocal-jazz",
        "Vocal Jazz",
        JAZZ,
        &["jazz vocal"],
        "Vocal jazz",
    ),
    node("swing", "Swing", JAZZ, &["swing music"], "Swing music"),
    node("big-band", "Big Band", JAZZ, &[], "Big band"),
    node(
        "dixieland",
        "Dixieland",
        JAZZ,
        &["new orleans jazz", "trad jazz", "traditional jazz"],
        "Dixieland",
    ),
    node("smooth-jazz", "Smooth Jazz", JAZZ, &[], "Smooth jazz"),
    node("acid-jazz", "Acid Jazz", JAZZ, &[], "Acid jazz"),
    // Classical
    node(
        "early-music",
        "Early Music",
        CLASSICAL,
        &[
            "medieval",
            "renaissance",
            "medieval music",
            "renaissance music",
        ],
        "Early music",
    ),
    node(
        "baroque",
        "Baroque",
        CLASSICAL,
        &["baroque music", "baroque period"],
        "Baroque music",
    ),
    node(
        "classical-period",
        "Classical Period",
        CLASSICAL,
        &["classicism", "viennese classicism"],
        "Classical period (music)",
    ),
    node(
        "romantic",
        "Romantic",
        CLASSICAL,
        &["romantic music", "romanticism", "romantic period"],
        "Romantic music",
    ),
    node(
        "modern-classical",
        "20th-Century Classical",
        CLASSICAL,
        &[
            "20th century classical",
            "20th century",
            "modern classical",
            "modernism",
        ],
        "20th-century classical music",
    ),
    node(
        "contemporary-classical",
        "Contemporary Classical",
        CLASSICAL,
        &["contemporary classical music", "new music"],
        "Contemporary classical music",
    ),
    node(
        "minimalism",
        "Minimalism",
        CLASSICAL,
        &["minimal music", "minimalist"],
        "Minimal music",
    ),
    node("opera", "Opera", CLASSICAL, &[], "Opera"),
    node(
        "chamber-music",
        "Chamber Music",
        CLASSICAL,
        &["chamber"],
        "Chamber music",
    ),
    node(
        "choral",
        "Choral",
        CLASSICAL,
        &["choral music", "choir"],
        "Choral music",
    ),
    node(
        "orchestral",
        "Orchestral",
        CLASSICAL,
        &["orchestral music", "symphonic", "symphony"],
        "Orchestra",
    ),
    // Rock
    node(
        "rock-and-roll",
        "Rock and Roll",
        ROCK,
        &["rock n roll", "rock n  roll", "rock & roll", "rockabilly"],
        "Rock and roll",
    ),
    node("classic-rock", "Classic Rock", ROCK, &[], "Classic rock"),
    node("hard-rock", "Hard Rock", ROCK, &[], "Hard rock"),
    node(
        "alternative-rock",
        "Alternative Rock",
        ROCK,
        &[
            "alternative",
            "alt rock",
            "alternative music",
            "alternative and punk",
        ],
        "Alternative rock",
    ),
    node("indie-rock", "Indie Rock", ROCK, &["indie"], "Indie rock"),
    node("punk", "Punk", ROCK, &["punk rock"], "Punk rock"),
    node("post-punk", "Post-Punk", ROCK, &[], "Post-punk"),
    node(
        "progressive-rock",
        "Progressive Rock",
        ROCK,
        &["prog", "prog rock"],
        "Progressive rock",
    ),
    node(
        "psychedelic-rock",
        "Psychedelic Rock",
        ROCK,
        &["psychedelic", "psychedelia"],
        "Psychedelic rock",
    ),
    node("art-rock", "Art Rock", ROCK, &[], "Art rock"),
    node("blues-rock", "Blues Rock", ROCK, &[], "Blues rock"),
    node("folk-rock", "Folk Rock", ROCK, &[], "Folk rock"),
    node("soft-rock", "Soft Rock", ROCK, &[], "Soft rock"),
    node("grunge", "Grunge", ROCK, &[], "Grunge"),
    node("shoegaze", "Shoegaze", ROCK, &["shoegazing"], "Shoegaze"),
    node("post-rock", "Post-Rock", ROCK, &[], "Post-rock"),
    // Metal
    node(
        "thrash-metal",
        "Thrash Metal",
        METAL,
        &["thrash"],
        "Thrash metal",
    ),
    node("death-metal", "Death Metal", METAL, &[], "Death metal"),
    node("black-metal", "Black Metal", METAL, &[], "Black metal"),
    node("doom-metal", "Doom Metal", METAL, &["doom"], "Doom metal"),
    node("power-metal", "Power Metal", METAL, &[], "Power metal"),
    node(
        "progressive-metal",
        "Progressive Metal",
        METAL,
        &["prog metal"],
        "Progressive metal",
    ),
    node("metalcore", "Metalcore", METAL, &[], "Metalcore"),
    node("nu-metal", "Nu Metal", METAL, &[], "Nu metal"),
    // Pop
    node(
        "synth-pop",
        "Synth-Pop",
        POP,
        &["synthpop", "synth pop"],
        "Synth-pop",
    ),
    node("dance-pop", "Dance-Pop", POP, &[], "Dance-pop"),
    node("electropop", "Electropop", POP, &[], "Electropop"),
    node("indie-pop", "Indie Pop", POP, &[], "Indie pop"),
    node("dream-pop", "Dream Pop", POP, &[], "Dream pop"),
    node("art-pop", "Art Pop", POP, &[], "Art pop"),
    node(
        "baroque-pop",
        "Baroque Pop",
        POP,
        &["chamber pop"],
        "Baroque pop",
    ),
    node("k-pop", "K-Pop", POP, &["kpop", "korean pop"], "K-pop"),
    node("j-pop", "J-Pop", POP, &["jpop", "japanese pop"], "J-pop"),
    // Electronic
    node(
        "house",
        "House",
        ELECTRONIC,
        &["house music", "deep house"],
        "House music",
    ),
    node("techno", "Techno", ELECTRONIC, &[], "Techno"),
    node(
        "trance",
        "Trance",
        ELECTRONIC,
        &["trance music"],
        "Trance music",
    ),
    node(
        "ambient",
        "Ambient",
        ELECTRONIC,
        &["ambient music", "dark ambient"],
        "Ambient music",
    ),
    node(
        "drum-and-bass",
        "Drum and Bass",
        ELECTRONIC,
        &["dnb", "d and b", "jungle"],
        "Drum and bass",
    ),
    node("dubstep", "Dubstep", ELECTRONIC, &[], "Dubstep"),
    node(
        "idm",
        "IDM",
        ELECTRONIC,
        &["intelligent dance music"],
        "Intelligent dance music",
    ),
    node(
        "downtempo",
        "Downtempo",
        ELECTRONIC,
        &["chillout", "chill out", "chill", "lounge electronic"],
        "Downtempo",
    ),
    node("trip-hop", "Trip Hop", ELECTRONIC, &["triphop"], "Trip hop"),
    node(
        "breakbeat",
        "Breakbeat",
        ELECTRONIC,
        &["big beat"],
        "Breakbeat",
    ),
    node("electro", "Electro", ELECTRONIC, &[], "Electro (music)"),
    node(
        "synthwave",
        "Synthwave",
        ELECTRONIC,
        &["retrowave", "outrun"],
        "Synthwave",
    ),
    node(
        "industrial",
        "Industrial",
        ELECTRONIC,
        &["industrial music", "ebm"],
        "Industrial music",
    ),
    node(
        "edm",
        "Dance",
        ELECTRONIC,
        &[
            "dance",
            "electronic dance music",
            "electronic dance",
            "club",
            "dance music",
        ],
        "Electronic dance music",
    ),
    // Hip hop
    node("trap", "Trap", HIP_HOP, &["trap music"], "Trap music"),
    node("boom-bap", "Boom Bap", HIP_HOP, &[], "Boom bap"),
    node(
        "gangsta-rap",
        "Gangsta Rap",
        HIP_HOP,
        &["gangsta"],
        "Gangsta rap",
    ),
    node(
        "alternative-hip-hop",
        "Alternative Hip Hop",
        HIP_HOP,
        &["alternative rap"],
        "Alternative hip hop",
    ),
    node(
        "conscious-hip-hop",
        "Conscious Hip Hop",
        HIP_HOP,
        &["conscious rap"],
        "Conscious hip hop",
    ),
    node(
        "jazz-rap",
        "Jazz Rap",
        HIP_HOP,
        &["jazz hip hop"],
        "Jazz rap",
    ),
    // R&B
    node(
        "soul",
        "Soul",
        RNB,
        &["soul music", "southern soul"],
        "Soul music",
    ),
    node("funk", "Funk", RNB, &[], "Funk"),
    node("disco", "Disco", RNB, &[], "Disco"),
    node("neo-soul", "Neo Soul", RNB, &["neosoul"], "Neo soul"),
    node(
        "contemporary-rnb",
        "Contemporary R&B",
        RNB,
        &["contemporary r and b", "urban"],
        "Contemporary R&B",
    ),
    // Blues
    node("delta-blues", "Delta Blues", BLUES, &[], "Delta blues"),
    node(
        "chicago-blues",
        "Chicago Blues",
        BLUES,
        &[],
        "Chicago blues",
    ),
    node(
        "electric-blues",
        "Electric Blues",
        BLUES,
        &[],
        "Electric blues",
    ),
    // Country
    node(
        "bluegrass",
        "Bluegrass",
        COUNTRY,
        &["bluegrass music"],
        "Bluegrass music",
    ),
    node("americana", "Americana", COUNTRY, &[], "Americana (music)"),
    node(
        "alt-country",
        "Alt-Country",
        COUNTRY,
        &["alternative country", "altcountry"],
        "Alternative country",
    ),
    node(
        "honky-tonk",
        "Honky-Tonk",
        COUNTRY,
        &["honky tonk"],
        "Honky-tonk",
    ),
    // Folk
    node(
        "singer-songwriter",
        "Singer-Songwriter",
        FOLK,
        &["singer songwriter", "singer", "songwriter"],
        "Singer-songwriter",
    ),
    node(
        "contemporary-folk",
        "Contemporary Folk",
        FOLK,
        &["folk revival", "american folk"],
        "Contemporary folk music",
    ),
    node("indie-folk", "Indie Folk", FOLK, &[], "Indie folk"),
    node(
        "celtic",
        "Celtic",
        FOLK,
        &["celtic music", "irish folk"],
        "Celtic music",
    ),
    // Latin
    node("bossa-nova", "Bossa Nova", LATIN, &[], "Bossa nova"),
    node("samba", "Samba", LATIN, &[], "Samba"),
    node("salsa", "Salsa", LATIN, &["salsa music"], "Salsa music"),
    node(
        "tango",
        "Tango",
        LATIN,
        &["tango music", "nuevo tango"],
        "Tango music",
    ),
    node("reggaeton", "Reggaeton", LATIN, &["reggaetón"], "Reggaeton"),
    // Reggae
    node("ska", "Ska", REGGAE, &[], "Ska"),
    node("rocksteady", "Rocksteady", REGGAE, &[], "Rocksteady"),
    node("dub", "Dub", REGGAE, &["dub music"], "Dub music"),
    node("dancehall", "Dancehall", REGGAE, &[], "Dancehall"),
    // World
    node("afrobeat", "Afrobeat", WORLD, &["afro beat"], "Afrobeat"),
    node("flamenco", "Flamenco", WORLD, &[], "Flamenco"),
    node("fado", "Fado", WORLD, &[], "Fado"),
    // Soundtrack
    node(
        "film-score",
        "Film Score",
        SOUNDTRACK,
        &["score", "film soundtrack", "film music", "movie soundtrack"],
        "Film score",
    ),
    node(
        "video-game-music",
        "Video Game Music",
        SOUNDTRACK,
        &["game", "video game", "game soundtrack", "vgm"],
        "Video game music",
    ),
    node(
        "musical-theatre",
        "Musical Theatre",
        SOUNDTRACK,
        &[
            "musical",
            "musicals",
            "show tunes",
            "broadway",
            "musical theater",
            "cast recording",
        ],
        "Musical theatre",
    ),
];

/// Lower case, `&` as "and", apostrophes dropped, other punctuation as
/// spaces, single spaces; a trailing "music" is ignored ("Jazz Music").
pub fn normalize_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for character in raw.trim().chars().flat_map(char::to_lowercase) {
        match character {
            '&' | '+' => out.push_str(" and "),
            '\'' | '’' | '`' => {}
            'é' | 'è' | 'ê' => out.push('e'),
            'ó' | 'ò' => out.push('o'),
            'á' | 'à' => out.push('a'),
            character if character.is_alphanumeric() => out.push(character),
            _ => out.push(' '),
        }
    }
    let words = out.split_whitespace().collect::<Vec<_>>();
    let words = match words.as_slice() {
        [rest @ .., "music"] if !rest.is_empty() => rest,
        all => all,
    };
    words.join(" ")
}

/// The normalised name without spaces, so "hip hop", "hip-hop" and
/// "hiphop" compare equal.
fn compact(raw: &str) -> String {
    normalize_name(raw).replace(' ', "")
}

pub fn by_slug(slug: &str) -> Option<&'static GenreNode> {
    TREE.iter().find(|node| node.slug == slug)
}

/// The canonical node a raw genre name stands for.
pub fn canonical(raw: &str) -> Option<&'static GenreNode> {
    let wanted = compact(raw);
    if wanted.is_empty() {
        return None;
    }
    TREE.iter().find(|node| {
        compact(node.name) == wanted
            || compact(node.slug) == wanted
            || node.aliases.iter().any(|alias| compact(alias) == wanted)
    })
}

/// A URL-safe id for a genre the tree does not know: "Krautrock" ->
/// "krautrock", "Neue Deutsche Härte" -> "neue-deutsche-härte".
pub fn free_slug(raw: &str) -> String {
    normalize_name(raw).replace(' ', "-")
}

/// Splits one tag value that holds several genres: "Jazz; Hard Bop",
/// "R&B/Soul", "Rock, Pop".
/// A slash only separates genres when the whole part is not itself a
/// known genre ("Singer/Songwriter").
pub fn split_raw(value: &str) -> Vec<String> {
    let mut parts = Vec::new();
    for part in value.split([';', ',', '|', '\u{0}']).map(str::trim) {
        if part.is_empty() {
            continue;
        }
        if canonical(part).is_some() || !part.contains('/') {
            parts.push(part.to_string());
            continue;
        }
        parts.extend(
            part.split('/')
                .map(str::trim)
                .filter(|piece| !piece.is_empty())
                .map(str::to_string),
        );
    }
    parts
}

/// The node and its ancestors, from the top-level genre down.
pub fn lineage(slug: &str) -> Vec<&'static GenreNode> {
    let mut chain = Vec::new();
    let mut current = by_slug(slug);
    while let Some(node) = current {
        if chain.iter().any(|seen: &&GenreNode| seen.slug == node.slug) {
            break;
        }
        chain.push(node);
        current = node.parent.and_then(by_slug);
    }
    chain.reverse();
    chain
}

pub fn children(slug: &str) -> Vec<&'static GenreNode> {
    TREE.iter()
        .filter(|node| node.parent == Some(slug))
        .collect()
}

/// Whether `slug` is `ancestor` or sits below it.
pub fn is_within(slug: &str, ancestor: &str) -> bool {
    slug == ancestor || lineage(slug).iter().any(|node| node.slug == ancestor)
}

/// One genre as attached to an album or artist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GenreRef {
    /// The canonical slug, or a slug made from the name for genres the
    /// tree does not know.
    pub slug: String,
    pub name: String,
    /// Names of the ancestors, from the top-level genre down.
    pub path: Vec<String>,
    /// Whether the genre is one of the canonical tree's.
    pub canonical: bool,
    /// Where it came from: `tags`, `musicbrainz` or `user`.
    pub sources: Vec<String>,
    /// The raw names that were normalised to it.
    pub raw_names: Vec<String>,
}

/// A raw genre name from one source, with its weight (MusicBrainz vote
/// count; 1 for tags and edits).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawGenre {
    pub name: String,
    pub source: String,
    pub weight: i64,
}

/// Normalises raw names into genre refs, merged by slug. A specific genre
/// and its parent both present keep both ("Jazz", "Hard Bop"). Order:
/// heaviest first, then sub-genres before their parents for ties, then by
/// name; at most `limit` refs.
pub fn resolve(raw: &[RawGenre], limit: usize) -> Vec<GenreRef> {
    let mut merged: Vec<(GenreRef, i64, usize)> = Vec::new();
    for genre in raw {
        let name = genre.name.trim();
        if name.is_empty() || name.chars().count() > 64 {
            continue;
        }
        let (slug, display, path, is_canonical, depth) = match canonical(name) {
            Some(node) => {
                let chain = lineage(node.slug);
                let path = chain[..chain.len().saturating_sub(1)]
                    .iter()
                    .map(|ancestor| ancestor.name.to_string())
                    .collect::<Vec<_>>();
                let depth = chain.len();
                (
                    node.slug.to_string(),
                    node.name.to_string(),
                    path,
                    true,
                    depth,
                )
            }
            None => {
                let slug = free_slug(name);
                if slug.is_empty() {
                    continue;
                }
                (slug, name.to_string(), Vec::new(), false, 1)
            }
        };
        match merged.iter_mut().find(|(entry, _, _)| entry.slug == slug) {
            Some((entry, weight, _)) => {
                *weight += genre.weight.max(1);
                if !entry.sources.contains(&genre.source) {
                    entry.sources.push(genre.source.clone());
                }
                if !entry.raw_names.iter().any(|raw| raw == name) {
                    entry.raw_names.push(name.to_string());
                }
            }
            None => merged.push((
                GenreRef {
                    slug,
                    name: display,
                    path,
                    canonical: is_canonical,
                    sources: vec![genre.source.clone()],
                    raw_names: vec![name.to_string()],
                },
                genre.weight.max(1),
                depth,
            )),
        }
    }
    merged.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then(right.2.cmp(&left.2))
            .then_with(|| left.0.name.to_lowercase().cmp(&right.0.name.to_lowercase()))
    });
    merged
        .into_iter()
        .take(limit)
        .map(|(genre, _, _)| genre)
        .collect()
}

/// Every canonical slug among `refs` together with their ancestors, so an
/// album in Hard Bop also counts for Jazz.
pub fn with_ancestors<'a>(slugs: impl IntoIterator<Item = &'a str>) -> HashSet<String> {
    let mut all = HashSet::new();
    for slug in slugs {
        all.insert(slug.to_string());
        for node in lineage(slug) {
            all.insert(node.slug.to_string());
        }
    }
    all
}

/// Which genres an item shows, given the administrator's preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GenreSource {
    /// MusicBrainz genres when the item has any, otherwise file tags.
    #[default]
    Online,
    /// File tags when the item has any, otherwise MusicBrainz genres.
    Tags,
}

/// Chooses the raw genres to show: an edit replaces everything; otherwise
/// the preferred source, falling back to the other one when it has none.
pub fn choose_raw(
    preference: GenreSource,
    edited: Option<&[String]>,
    tags: &[String],
    online: &[(String, i64)],
) -> Vec<RawGenre> {
    if let Some(edited) = edited {
        return edited
            .iter()
            .map(|name| RawGenre {
                name: name.clone(),
                source: "user".into(),
                weight: 1,
            })
            .collect();
    }
    let from_tags = || {
        tags.iter()
            .flat_map(|value| split_raw(value))
            .map(|name| RawGenre {
                name,
                source: "tags".into(),
                weight: 1,
            })
            .collect::<Vec<_>>()
    };
    let from_online = || {
        online
            .iter()
            .map(|(name, votes)| RawGenre {
                name: name.clone(),
                source: "musicbrainz".into(),
                weight: *votes,
            })
            .collect::<Vec<_>>()
    };
    let (first, second) = match preference {
        GenreSource::Online => (from_online(), from_tags()),
        GenreSource::Tags => (from_tags(), from_online()),
    };
    if first.is_empty() { second } else { first }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_node_has_a_known_parent_and_unique_names() {
        let mut slugs = HashSet::new();
        let mut names = HashSet::new();
        for node in TREE {
            assert!(slugs.insert(node.slug), "duplicate slug {}", node.slug);
            assert!(names.insert(compact(node.name)), "duplicate {}", node.name);
            if let Some(parent) = node.parent {
                assert!(by_slug(parent).is_some(), "{} has no parent", node.slug);
            }
            assert!(!node.wikipedia.is_empty());
            assert_eq!(
                canonical(node.name).map(|found| found.slug),
                Some(node.slug)
            );
        }
        // An alias never names a different node.
        for node in TREE {
            for alias in node.aliases {
                assert_eq!(
                    canonical(alias).map(|found| found.slug),
                    Some(node.slug),
                    "alias {alias} of {}",
                    node.slug
                );
            }
        }
    }

    #[test]
    fn aliases_and_spellings_normalise_into_the_tree() {
        let slug = |raw: &str| canonical(raw).map(|node| node.slug);
        assert_eq!(slug("Bop"), Some("bebop"));
        assert_eq!(slug("be-bop"), Some("bebop"));
        assert_eq!(slug("Hard-Bop"), Some("hard-bop"));
        assert_eq!(slug("HARD BOP"), Some("hard-bop"));
        assert_eq!(slug("Jazz Music"), Some("jazz"));
        assert_eq!(slug("Hip-Hop"), Some("hip-hop"));
        assert_eq!(slug("hiphop"), Some("hip-hop"));
        assert_eq!(slug("R&B"), Some("rnb"));
        assert_eq!(slug("Rhythm & Blues"), Some("rnb"));
        assert_eq!(slug("Drum & Bass"), Some("drum-and-bass"));
        assert_eq!(slug("Rock 'n' Roll"), Some("rock-and-roll"));
        assert_eq!(slug("Synthpop"), Some("synth-pop"));
        assert_eq!(slug("Modal Music"), Some("modal-jazz"));
        assert_eq!(slug("Krautrock"), None);
        assert_eq!(free_slug("Neue Deutsche Härte"), "neue-deutsche-härte");
        let names = lineage("bebop")
            .iter()
            .map(|node| node.name)
            .collect::<Vec<_>>();
        assert_eq!(names, ["Jazz", "Bebop"]);
        assert!(is_within("hard-bop", "jazz"));
        assert!(!is_within("jazz", "hard-bop"));
        assert!(children("jazz").iter().any(|node| node.slug == "hard-bop"));
    }

    #[test]
    fn multi_genre_tags_split_and_merge_by_canonical_genre() {
        assert_eq!(split_raw("Jazz; Hard Bop"), ["Jazz", "Hard Bop"]);
        assert_eq!(split_raw("R&B/Soul"), ["R&B", "Soul"]);
        assert_eq!(split_raw("Singer/Songwriter"), ["Singer/Songwriter"]);
        let raw = ["Jazz", "hard bop", "Hard-Bop", "Modal", "Krautrock"]
            .iter()
            .map(|name| RawGenre {
                name: name.to_string(),
                source: "tags".into(),
                weight: 1,
            })
            .collect::<Vec<_>>();
        let refs = resolve(&raw, 8);
        let slugs = refs
            .iter()
            .map(|genre| genre.slug.as_str())
            .collect::<Vec<_>>();
        // Hard Bop was named twice; sub-genres come before Jazz for ties.
        assert_eq!(slugs, ["hard-bop", "modal-jazz", "jazz", "krautrock"]);
        assert_eq!(refs[0].path, ["Jazz"]);
        assert_eq!(refs[0].raw_names, ["hard bop", "Hard-Bop"]);
        assert!(!refs[3].canonical);
        assert_eq!(refs[3].name, "Krautrock");
    }

    #[test]
    fn the_preferred_source_wins_and_falls_back_and_edits_replace_both() {
        let tags = vec!["Jazz".to_string()];
        let online = vec![("hard bop".to_string(), 6), ("jazz".to_string(), 3)];
        let names = |raw: Vec<RawGenre>| {
            raw.into_iter()
                .map(|genre| (genre.name, genre.source))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(choose_raw(GenreSource::Online, None, &tags, &online)),
            [
                ("hard bop".to_string(), "musicbrainz".to_string()),
                ("jazz".to_string(), "musicbrainz".to_string())
            ]
        );
        assert_eq!(
            names(choose_raw(GenreSource::Tags, None, &tags, &online)),
            [("Jazz".to_string(), "tags".to_string())]
        );
        assert_eq!(
            names(choose_raw(GenreSource::Tags, None, &[], &online)).len(),
            2
        );
        assert_eq!(
            names(choose_raw(GenreSource::Online, None, &tags, &[])),
            [("Jazz".to_string(), "tags".to_string())]
        );
        let edited = vec!["Cool Jazz".to_string()];
        assert_eq!(
            names(choose_raw(
                GenreSource::Online,
                Some(&edited),
                &tags,
                &online
            )),
            [("Cool Jazz".to_string(), "user".to_string())]
        );
        assert!(choose_raw(GenreSource::Online, Some(&[]), &tags, &online).is_empty());
    }
}
