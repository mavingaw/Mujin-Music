//! The plain-language request FETCH reads, built from an album's tags so
//! it is as precise as possible: album artist, album title, year, the
//! MusicBrainz release id when the files carry one, and "as flac".

/// Tags of the album a better copy is wanted for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AlbumTags {
    pub artist: String,
    pub title: String,
    pub year: String,
    pub release_mbid: Option<String>,
}

/// One line of text: no quotes, control characters or runs of spaces,
/// and never longer than `max` characters.
fn clean(value: &str, max: usize) -> String {
    let mut text = String::with_capacity(value.len());
    for character in value.chars() {
        let character = match character {
            '"' | '\u{201c}' | '\u{201d}' => '\'',
            c if c.is_control() => ' ',
            c => c,
        };
        if character == ' ' && text.ends_with(' ') {
            continue;
        }
        text.push(character);
    }
    text.trim().chars().take(max).collect()
}

/// `the album Endgame by Rise Against (2011, MusicBrainz release
/// 6f1d...) as flac`.
pub fn build_query(tags: &AlbumTags) -> String {
    let title = clean(&tags.title, 200);
    let artist = clean(&tags.artist, 120);
    let year: String = tags
        .year
        .chars()
        .take(4)
        .filter(char::is_ascii_digit)
        .collect();
    let mbid = tags
        .release_mbid
        .as_deref()
        .map(str::trim)
        .filter(|mbid| uuid::Uuid::parse_str(mbid).is_ok());
    let mut text = format!("the album {title}");
    if !artist.is_empty() && !artist.eq_ignore_ascii_case("unknown artist") {
        text.push_str(&format!(" by {artist}"));
    }
    let details: Vec<String> = [
        (year.len() == 4).then_some(year),
        mbid.map(|mbid| format!("MusicBrainz release {mbid}")),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !details.is_empty() {
        text.push_str(&format!(" ({})", details.join(", ")));
    }
    text.push_str(" as flac");
    text
}

/// The words to look the album up by on FETCH's search page.
pub fn search_terms(tags: &AlbumTags) -> String {
    clean(&format!("{} {}", tags.artist, tags.title), 200)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_follow_fetchs_plain_language_and_add_what_the_tags_know() {
        let mut tags = AlbumTags {
            artist: "Rise Against".into(),
            title: "Endgame".into(),
            ..Default::default()
        };
        assert_eq!(
            build_query(&tags),
            "the album Endgame by Rise Against as flac"
        );
        tags.year = "2011-03-15".into();
        tags.release_mbid = Some("6f1d0c3d-3f0e-4c37-9c8a-6c3bd1b1d1a7".into());
        assert_eq!(
            build_query(&tags),
            "the album Endgame by Rise Against (2011, MusicBrainz release 6f1d0c3d-3f0e-4c37-9c8a-6c3bd1b1d1a7) as flac"
        );
        // Junk ids and years are left out; quotes and newlines are cleaned.
        let messy = AlbumTags {
            artist: "The \"Band\"\n".into(),
            title: "  Live   at  Leeds ".into(),
            year: "n/a".into(),
            release_mbid: Some("not-an-id".into()),
        };
        assert_eq!(
            build_query(&messy),
            "the album Live at Leeds by The 'Band' as flac"
        );
        assert_eq!(search_terms(&messy), "The 'Band' Live at Leeds");
        let unknown = AlbumTags {
            artist: "Unknown Artist".into(),
            title: "Demos".into(),
            ..Default::default()
        };
        assert_eq!(build_query(&unknown), "the album Demos as flac");
    }
}
