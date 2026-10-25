//! Wikidata, Wikipedia and Wikimedia Commons requests and parsing.
//!
//! Text and photos come from the endpoints Wikimedia's caches serve most
//! readily: the article page itself (`/wiki/Title`, which carries the lead,
//! the reception section, the Wikidata item and the page image), the REST
//! summary as a fallback when that page is busy, `Special:EntityData` for
//! Wikidata and the raw description page of a Commons file for its license.
//! The Action API (`w/api.php`) answers anonymous clients with 429 much
//! sooner, so it is only a last resort.
//!
//! Wikipedia text is CC BY-SA 4.0 and must be shown with the article title,
//! a link to it and the license. Commons images are used only when their
//! license allows reuse with attribution (CC BY, CC BY-SA, CC0 or public
//! domain).

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use serde_json::Value;

pub const WIKIPEDIA_LICENSE: &str = "CC BY-SA 4.0";
pub const WIKIPEDIA_LICENSE_URL: &str = "https://creativecommons.org/licenses/by-sa/4.0/";
/// Upper bound for one stored text, cut at a paragraph boundary.
pub const MAX_TEXT_CHARS: usize = 6_000;

fn encode(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

/// Characters left as they are in a page title path segment, so the URL is
/// the one Wikimedia's caches already hold (`Kind_of_Blue`, `AC%2FDC`).
const TITLE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'-')
    .remove(b'.')
    .remove(b'(')
    .remove(b')')
    .remove(b',')
    .remove(b'!')
    .remove(b'\'');

/// A page title as a URL path segment: spaces become underscores.
pub fn title_segment(title: &str) -> String {
    utf8_percent_encode(&title.trim().replace(' ', "_"), TITLE_SET).to_string()
}

/// A language code safe to put in a Wikipedia host name.
pub fn valid_language(language: &str) -> bool {
    (2..=12).contains(&language.len())
        && language
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
}

pub fn wikidata_entity_url(id: &str) -> String {
    format!("https://www.wikidata.org/wiki/Special:EntityData/{id}.json")
}

pub fn wikipedia_extract_url(language: &str, title: &str) -> String {
    format!(
        "https://{language}.wikipedia.org/w/api.php?action=query&format=json&formatversion=2&prop=extracts%7Cinfo&inprop=url&explaintext=1&exsectionformat=wiki&redirects=1&titles={}",
        encode(title)
    )
}

/// The REST summary of an article: title, URL, lead paragraph, the
/// Wikidata item (`pageprops.wikibase_item`), a short description and the
/// page image. Redirects are followed.
pub fn wikipedia_summary_url(language: &str, title: &str) -> String {
    format!(
        "https://{language}.wikipedia.org/api/rest_v1/page/summary/{}",
        title_segment(title)
    )
}

/// The article page itself, as a browser asks for it: the full lead, the
/// reception section, the Wikidata item and the page image in one cached
/// response.
pub fn wikipedia_page_url(language: &str, title: &str) -> String {
    format!(
        "https://{language}.wikipedia.org/wiki/{}",
        title_segment(title)
    )
}

/// The Commons description page's wikitext, which names the file's author
/// and license templates.
pub fn commons_wikitext_url(file_name: &str) -> String {
    format!(
        "https://commons.wikimedia.org/wiki/File:{}?action=raw",
        title_segment(file_name)
    )
}

/// A 500 pixel rendition of a Commons file (a standard thumbnail size, so
/// it is served from cache).
pub fn commons_thumbnail_url(file_name: &str) -> String {
    format!(
        "https://commons.wikimedia.org/wiki/Special:FilePath/{}?width=500",
        title_segment(file_name)
    )
}

pub fn commons_page_url(file_name: &str) -> String {
    format!(
        "https://commons.wikimedia.org/wiki/File:{}",
        title_segment(file_name)
    )
}

/// `https://en.wikipedia.org/wiki/Kind_of_Blue` -> `("en", "Kind of Blue")`.
/// Mobile hosts are accepted; anything that is not a Wikipedia article URL
/// is not.
pub fn parse_wikipedia_url(url: &str) -> Option<(String, String)> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let (host, path) = rest.split_once('/')?;
    let language = host
        .strip_suffix(".wikipedia.org")?
        .trim_end_matches(".m")
        .to_string();
    if !valid_language(&language) {
        return None;
    }
    let title = path.strip_prefix("wiki/")?;
    let title = title.split(['#', '?']).next()?;
    let title = percent_decode_str(title)
        .decode_utf8()
        .ok()?
        .replace('_', " ");
    let title = title.trim();
    (!title.is_empty() && title.len() <= 255 && !title.contains(':'))
        .then(|| (language, title.to_string()))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WikipediaSummary {
    pub title: String,
    pub url: String,
    /// The lead paragraph as plain text.
    pub extract: String,
    pub wikibase_item: Option<String>,
    /// Wikidata's short description, e.g. "1959 studio album by Miles Davis".
    pub description: Option<String>,
    pub disambiguation: bool,
    /// The page image when it is a Commons file (never a local, non-free
    /// upload such as an album cover).
    pub commons_image: Option<String>,
}

fn commons_file_from_upload_url(url: &str) -> Option<String> {
    let path = url
        .strip_prefix("https://upload.wikimedia.org/wikipedia/commons/")
        .or_else(|| url.strip_prefix("https://thumb.wikimedia.org/wikipedia/commons/"))?
        .split(['?', '#'])
        .next()?;
    // Originals are `x/xy/Name.jpg`; thumbnails `thumb/x/xy/Name.jpg/330px-Name.jpg`.
    let segments = path.split('/').collect::<Vec<_>>();
    let name = match segments.as_slice() {
        ["thumb", _, _, name, ..] => *name,
        [_, _, name] => *name,
        _ => return None,
    };
    let name = percent_decode_str(name)
        .decode_utf8()
        .ok()?
        .replace('_', " ");
    (!name.is_empty() && name.len() <= 240 && !name.contains(['/', '\\', '|', '#', '?']))
        .then_some(name)
}

fn is_item_id(id: &str) -> bool {
    id.len() > 1 && id.starts_with('Q') && id[1..].bytes().all(|byte| byte.is_ascii_digit())
}

pub fn parse_wikipedia_summary(body: &Value) -> Option<WikipediaSummary> {
    let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_string);
    let title = text(body.pointer("/titles/normalized")).or_else(|| text(body.get("title")))?;
    let url = text(body.pointer("/content_urls/desktop/page"))?;
    if !url.starts_with("https://") {
        return None;
    }
    let kind = body.get("type").and_then(Value::as_str).unwrap_or_default();
    let image = text(body.pointer("/originalimage/source"))
        .or_else(|| text(body.pointer("/thumbnail/source")));
    Some(WikipediaSummary {
        title,
        url,
        extract: text(body.get("extract"))
            .unwrap_or_default()
            .trim()
            .to_string(),
        wikibase_item: text(body.get("wikibase_item")).filter(|id| is_item_id(id)),
        description: text(body.get("description")),
        disambiguation: kind == "disambiguation",
        commons_image: image.as_deref().and_then(commons_file_from_upload_url),
    })
}

fn tag_boundary(byte: Option<u8>) -> bool {
    matches!(byte, Some(b' ' | b'>' | b'\n' | b'\t' | b'/'))
}

/// The end of the HTML element starting at `start` (`<tag ...>`), after its
/// matching close tag, honouring nesting of the same tag.
fn element_end(html: &str, start: usize, tag: &str) -> usize {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut depth = 0usize;
    let mut index = start;
    loop {
        let next_open = html[index..].find(&open).map(|offset| index + offset);
        let next_close = html[index..].find(&close).map(|offset| index + offset);
        match (next_open, next_close) {
            (Some(opening), Some(closing)) if opening < closing => {
                // `<table` must not count `<tbody`.
                if tag_boundary(html.as_bytes().get(opening + open.len()).copied()) {
                    depth += 1;
                }
                index = opening + open.len();
            }
            (_, Some(closing)) => {
                depth = depth.saturating_sub(1);
                index = closing + close.len();
                if depth == 0 {
                    return index;
                }
            }
            _ => return html.len(),
        }
    }
}

/// Removes every `tag` element, with its content.
fn remove_elements(html: &str, tag: &str) -> String {
    let open = format!("<{tag}");
    let mut output = String::with_capacity(html.len());
    let mut index = 0;
    while let Some(offset) = html[index..].find(&open) {
        let start = index + offset;
        if !tag_boundary(html.as_bytes().get(start + open.len()).copied()) {
            output.push_str(&html[index..start + open.len()]);
            index = start + open.len();
            continue;
        }
        output.push_str(&html[index..start]);
        index = element_end(html, start, tag);
    }
    output.push_str(&html[index..]);
    output
}

/// Decodes the HTML entities Wikipedia emits, numeric ones included.
pub fn decode_entities(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        output.push_str(&rest[..start]);
        let candidate = &rest[start..];
        let window = candidate
            .char_indices()
            .take(12)
            .last()
            .map_or(0, |(at, character)| at + character.len_utf8());
        let decoded = candidate[..window].find(';').and_then(|end| {
            let entity = &candidate[1..end];
            let character = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                "ndash" => Some('–'),
                "mdash" => Some('—'),
                _ => entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                    .and_then(char::from_u32),
            }?;
            Some((character, end))
        });
        match decoded {
            Some((character, end)) => {
                output.push(if character == '\u{a0}' {
                    ' '
                } else {
                    character
                });
                rest = &candidate[end + 1..];
            }
            None => {
                output.push('&');
                rest = &candidate[1..];
            }
        }
    }
    output.push_str(rest);
    output
}

/// Removes tags. A `>` inside a quoted attribute value (Parsoid's
/// `data-mw` JSON holds wikitext) does not end the tag.
fn strip_tags(value: &str) -> String {
    let mut text = String::with_capacity(value.len());
    let mut in_tag = false;
    let mut quote = None;
    for character in value.chars() {
        match (in_tag, quote, character) {
            (false, _, '<') => in_tag = true,
            (false, _, _) => text.push(character),
            (true, None, '"' | '\'') => quote = Some(character),
            (true, Some(open), _) if open == character => quote = None,
            (true, None, '>') => in_tag = false,
            _ => {}
        }
    }
    text
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The plain-text paragraphs (`<p>`) of an HTML fragment, without
/// references, tables, infoboxes, styles, figures or quotations.
pub fn html_paragraphs(fragment: &str) -> Vec<String> {
    let mut html = fragment.to_string();
    for tag in ["style", "table", "figure", "sup", "math", "blockquote"] {
        html = remove_elements(&html, tag);
    }
    let mut paragraphs = Vec::new();
    let mut index = 0;
    while let Some(offset) = html[index..].find("<p") {
        let start = index + offset;
        if !matches!(html.as_bytes().get(start + 2).copied(), Some(b' ' | b'>')) {
            index = start + 2;
            continue;
        }
        let end = element_end(&html, start, "p");
        let text = collapse_whitespace(&decode_entities(&strip_tags(&html[start..end])));
        if !text.is_empty() {
            paragraphs.push(text);
        }
        index = end;
    }
    paragraphs
}

/// The heading (level and plain title) a section starts with.
fn section_heading(section: &str) -> Option<(usize, String)> {
    let start = heading_starts(section).into_iter().next()?;
    let level = (section.as_bytes()[start + 2] - b'0') as usize;
    let close = format!("</h{level}>");
    let end = section[start..].find(&close)? + start;
    let title = collapse_whitespace(&decode_entities(&strip_tags(&section[start..end])));
    (!title.is_empty()).then_some((level, title))
}

/// Byte offsets of `<h2>` … `<h6>` opening tags.
fn heading_starts(html: &str) -> Vec<usize> {
    let bytes = html.as_bytes();
    html.match_indices("<h")
        .map(|(at, _)| at)
        .filter(|at| {
            matches!(bytes.get(at + 2), Some(b'2'..=b'6'))
                && matches!(bytes.get(at + 3), Some(b' ' | b'>'))
        })
        .collect()
}

/// The article body: the parser output of a desktop page, without the
/// page chrome around it. Mobile HTML is all body.
fn content_region(html: &str) -> &str {
    let start = html
        .find("mw-parser-output\"")
        .and_then(|at| html[at..].find('>').map(|end| at + end + 1))
        .unwrap_or(0);
    let end = ["class=\"printfooter\"", "id=\"catlinks\""]
        .iter()
        .filter_map(|marker| html[start..].find(marker).map(|at| start + at))
        .min()
        .unwrap_or(html.len());
    &html[start..end]
}

/// The lead and the reception section of an article's HTML (the desktop
/// page or mobile HTML). A reception section includes the deeper
/// sections that follow it.
pub fn parse_article_html(html: &str) -> (String, Option<String>) {
    let content = content_region(html);
    let starts = heading_starts(content);
    let mut cuts = vec![0];
    cuts.extend(starts.iter().copied());
    let sections = cuts
        .iter()
        .enumerate()
        .map(|(position, start)| {
            let end = cuts.get(position + 1).copied().unwrap_or(content.len());
            &content[*start..end]
        })
        .collect::<Vec<_>>();
    let joined = |parts: &[&str]| {
        let paragraphs = parts
            .iter()
            .flat_map(|part| html_paragraphs(part))
            .collect::<Vec<_>>();
        tidy_text(&paragraphs.iter().map(String::as_str).collect::<Vec<_>>())
    };
    let lead = joined(&sections[..1]);
    let mut reception = None;
    for index in 1..sections.len() {
        let Some((level, title)) = section_heading(sections[index]) else {
            continue;
        };
        if !is_reception_heading(&title) {
            continue;
        }
        let end = sections[index + 1..]
            .iter()
            .position(|section| section_heading(section).is_some_and(|(next, _)| next <= level))
            .map_or(sections.len(), |offset| index + 1 + offset);
        let text = joined(&sections[index..end]);
        if !text.is_empty() {
            reception = Some(text);
            break;
        }
    }
    (lead, reception)
}

/// An article page read in one request: what the summary would say (title,
/// URL, Wikidata item, short description, page image, disambiguation) plus
/// the full lead and the reception section.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArticlePage {
    pub summary: WikipediaSummary,
    pub lead: String,
    pub reception: Option<String>,
}

fn after<'a>(html: &'a str, marker: &str) -> Option<&'a str> {
    html.find(marker).map(|at| &html[at + marker.len()..])
}

/// The value of `attribute="…"` in the first tag that starts with `tag`.
fn tag_attribute(html: &str, tag: &str, attribute: &str) -> Option<String> {
    let rest = after(html, tag)?;
    let end = rest.find('>')?;
    let value = after(&rest[..end], &format!("{attribute}=\""))?;
    let value = &value[..value.find('"')?];
    Some(decode_entities(value))
}

/// Reads a desktop Wikipedia page (`/wiki/Title`), the endpoint browsers
/// use and the one Wikimedia's caches serve most readily.
pub fn parse_article_page(html: &str) -> Option<ArticlePage> {
    let url = tag_attribute(html, "<link rel=\"canonical\"", "href")?;
    let (_, title) = parse_wikipedia_url(&url)?;
    if !url.starts_with("https://") {
        return None;
    }
    let wikibase_item = after(html, "\"wgWikibaseItemId\":\"")
        .and_then(|rest| rest.split('"').next())
        .filter(|id| is_item_id(id))
        .map(str::to_string);
    let categories = after(html, "\"wgCategories\":[")
        .and_then(|rest| rest.split(']').next())
        .unwrap_or_default();
    let disambiguation = categories.contains("\"Disambiguation pages\"")
        || categories.contains("disambiguation pages\"")
        || html.contains("id=\"disambigbox\"");
    let description = after(html, "<div class=\"shortdescription")
        .and_then(|rest| rest.find("</div>").map(|end| &rest[..end]))
        .and_then(|block| block.rsplit('>').next())
        .map(|text| collapse_whitespace(&decode_entities(text)))
        .filter(|text| !text.is_empty());
    let commons_image = tag_attribute(html, "<meta property=\"og:image\"", "content")
        .as_deref()
        .and_then(commons_file_from_upload_url);
    let (lead, reception) = parse_article_html(html);
    if lead.is_empty() && !disambiguation {
        return None;
    }
    let extract = lead.split("\n\n").next().unwrap_or_default().to_string();
    Some(ArticlePage {
        summary: WikipediaSummary {
            title,
            url,
            extract,
            wikibase_item,
            description,
            disambiguation,
            commons_image,
        },
        lead,
        reception,
    })
}

/// A Commons license template (`cc-by-sa-4.0`, `cc-by-3.0-de`,
/// `PD-old-70`, `CC-zero`) as a short name and deed URL.
fn license_from_template(name: &str) -> Option<(String, Option<String>)> {
    let name = name.trim().to_lowercase().replace(['_', ' '], "-");
    if name == "cc-zero" || name == "cc0" || name.starts_with("cc0-") {
        return Some((
            "CC0".into(),
            Some("https://creativecommons.org/publicdomain/zero/1.0/".into()),
        ));
    }
    if name.starts_with("pd-") || name == "pd" || name == "public-domain" {
        return Some(("Public domain".into(), None));
    }
    for (prefix, short, path) in [
        ("cc-by-sa-", "CC BY-SA", "by-sa"),
        ("cc-by-", "CC BY", "by"),
    ] {
        if let Some(rest) = name.strip_prefix(prefix) {
            let version = rest.split([',', '-']).next().filter(|version| {
                !version.is_empty()
                    && version
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || byte == b'.')
            })?;
            return Some((
                format!("{short} {version}"),
                Some(format!(
                    "https://creativecommons.org/licenses/{path}/{version}/"
                )),
            ));
        }
    }
    None
}

/// Removes wiki markup from a field value: links keep their label,
/// templates and HTML go.
fn plain_wikitext(value: &str) -> String {
    let mut text = String::new();
    let mut rest = value;
    while let Some(character) = rest.chars().next() {
        if let Some(inner) = rest.strip_prefix("[[") {
            let end = inner.find("]]").unwrap_or(inner.len());
            let link = &inner[..end];
            text.push_str(link.rsplit('|').next().unwrap_or(link));
            rest = inner.get(end + 2..).unwrap_or_default();
        } else if let Some(inner) = rest.strip_prefix("{{") {
            let end = inner.find("}}").unwrap_or(inner.len());
            rest = inner.get(end + 2..).unwrap_or_default();
        } else if let Some(inner) = rest.strip_prefix('[') {
            let end = inner.find(']').unwrap_or(inner.len());
            let link = &inner[..end];
            text.push_str(link.split_once(' ').map_or("", |(_, label)| label));
            rest = inner.get(end + 1..).unwrap_or_default();
        } else {
            text.push(character);
            rest = &rest[character.len_utf8()..];
        }
    }
    let text = collapse_whitespace(&decode_entities(&strip_tags(&text)).replace("''", ""));
    // A list item's marker ("* Name") is not part of the name.
    text.trim_start_matches(['*', '#', ':', ' ']).to_string()
}

/// Author and license from a Commons description page's wikitext. `None`
/// when no reusable license template is found or the file is not a JPEG,
/// PNG or WebP image.
pub fn parse_commons_wikitext(file_name: &str, wikitext: &str) -> Option<CommonsImage> {
    let lower = file_name.to_lowercase();
    let mime_type = if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else {
        return None;
    };
    let mut license = None;
    let mut rest = wikitext;
    while let Some(start) = rest.find("{{") {
        let inner = &rest[start + 2..];
        let end = inner.find(['}', '{']).unwrap_or(inner.len());
        let template = &inner[..end];
        let mut parts = template.split('|');
        let name = parts.next().unwrap_or_default();
        let candidates: Vec<&str> = if name.trim().eq_ignore_ascii_case("self") {
            parts.filter(|part| !part.contains('=')).collect()
        } else {
            vec![name]
        };
        if let Some(found) = candidates.into_iter().find_map(license_from_template) {
            license = Some(found);
            break;
        }
        rest = &inner[end..];
    }
    let (license, license_url) = license?;
    if !reusable_license(&license) {
        return None;
    }
    let author = wikitext
        .lines()
        .find_map(|line| {
            let line = line.trim().trim_start_matches('|').trim_start();
            let (key, value) = line.split_once('=')?;
            matches!(
                key.trim().to_lowercase().as_str(),
                "author" | "artist" | "photographer"
            )
            .then(|| plain_wikitext(value.trim()))
        })
        .filter(|author| !author.is_empty() && author.chars().count() <= 200)
        .unwrap_or_else(|| "Wikimedia Commons contributor".to_string());
    Some(CommonsImage {
        download_url: commons_thumbnail_url(file_name),
        page_url: commons_page_url(file_name),
        mime_type: mime_type.to_string(),
        author,
        license,
        license_url,
    })
}

pub fn commons_image_info_url(file_name: &str) -> String {
    format!(
        "https://commons.wikimedia.org/w/api.php?action=query&format=json&formatversion=2&prop=imageinfo&iiprop=url%7Cmime%7Cextmetadata&iiurlwidth=1000&titles={}",
        encode(&format!("File:{file_name}"))
    )
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WikidataEntity {
    pub id: String,
    /// Article titles by language, e.g. `("en", "Kind of Blue")`.
    pub sitelinks: Vec<(String, String)>,
    /// Commons file name of the item's image (P18).
    pub image: Option<String>,
    pub born: Option<String>,
    pub died: Option<String>,
    pub formed: Option<String>,
    pub dissolved: Option<String>,
}

impl WikidataEntity {
    /// The article in `language`, or English when that language has none.
    pub fn article(&self, language: &str) -> Option<(String, String)> {
        [language, "en"].iter().find_map(|wanted| {
            self.sitelinks
                .iter()
                .find(|(language, _)| language == wanted)
                .cloned()
        })
    }
}

fn claim_values<'a>(entity: &'a Value, property: &str) -> impl Iterator<Item = &'a Value> {
    entity
        .get("claims")
        .and_then(|claims| claims.get(property))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|claim| claim.get("rank").and_then(Value::as_str) != Some("deprecated"))
        .filter_map(|claim| claim.get("mainsnak")?.get("datavalue")?.get("value"))
}

/// Wikidata times look like `+1926-05-26T00:00:00Z` with a precision of 9
/// (year), 10 (month) or 11 (day).
fn wikidata_date(entity: &Value, property: &str) -> Option<String> {
    let value = claim_values(entity, property).next()?;
    let time = value.get("time")?.as_str()?;
    let precision = value.get("precision").and_then(Value::as_i64).unwrap_or(11);
    let negative = time.starts_with('-');
    let date = time.trim_start_matches(['+', '-']).split('T').next()?;
    let mut parts = date.split('-');
    let year = parts.next()?.trim_start_matches('0');
    let month = parts.next().unwrap_or("00");
    let day = parts.next().unwrap_or("00");
    if year.is_empty() || precision < 9 {
        return None;
    }
    let year = if negative {
        format!("-{year}")
    } else {
        year.to_string()
    };
    Some(match precision {
        9 => year,
        10 if month != "00" => format!("{year}-{month}"),
        _ if month != "00" && day != "00" => format!("{year}-{month}-{day}"),
        _ => year,
    })
}

pub fn parse_wikidata_entity(body: &Value) -> Option<WikidataEntity> {
    let (id, entity) = body.get("entities")?.as_object()?.iter().next()?;
    let sitelinks = entity
        .get("sitelinks")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(site, link)| {
            let language = site.strip_suffix("wiki")?;
            // Project wikis share the `wiki` suffix but are not Wikipedias.
            const PROJECTS: [&str; 9] = [
                "commons",
                "species",
                "meta",
                "mediawiki",
                "wikidata",
                "sources",
                "outreach",
                "wikimania",
                "incubator",
            ];
            if !valid_language(language) || PROJECTS.contains(&language) {
                return None;
            }
            let title = link.get("title")?.as_str()?;
            Some((language.to_string(), title.to_string()))
        })
        .collect();
    Some(WikidataEntity {
        id: id.clone(),
        sitelinks,
        image: claim_values(entity, "P18")
            .find_map(Value::as_str)
            .map(str::to_string),
        born: wikidata_date(entity, "P569"),
        died: wikidata_date(entity, "P570"),
        formed: wikidata_date(entity, "P571"),
        dissolved: wikidata_date(entity, "P576"),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikipediaArticle {
    pub title: String,
    pub url: String,
    /// The article's lead section.
    pub lead: String,
    /// A "Critical reception" (or similar) section, when the article has one.
    pub reception: Option<String>,
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim();
    if !trimmed.starts_with("==") || !trimmed.ends_with("==") || trimmed.len() < 5 {
        return None;
    }
    let level = trimmed
        .chars()
        .take_while(|character| *character == '=')
        .count();
    let title = trimmed.trim_matches('=').trim();
    (!title.is_empty()).then_some((level, title))
}

fn is_reception_heading(title: &str) -> bool {
    let title = title.to_lowercase();
    title.contains("reception") || title.contains("critical response") || title == "reviews"
}

/// Joins paragraphs, drops sub-headings and empty lines, and cuts at a
/// paragraph boundary near `MAX_TEXT_CHARS`.
pub fn tidy_text(lines: &[&str]) -> String {
    let mut paragraphs: Vec<&str> = Vec::new();
    let mut length = 0;
    for line in lines {
        let line = line.trim();
        if line.is_empty() || heading(line).is_some() {
            continue;
        }
        let added = line.chars().count();
        if length > 0 && length + added > MAX_TEXT_CHARS {
            break;
        }
        length += added;
        paragraphs.push(line);
    }
    let mut text = paragraphs.join("\n\n");
    if text.chars().count() > MAX_TEXT_CHARS {
        text = text.chars().take(MAX_TEXT_CHARS).collect::<String>();
        if let Some(end) = text.rfind(". ") {
            text.truncate(end + 1);
        }
        text.push_str(" …");
    }
    text
}

pub fn parse_wikipedia_extract(body: &Value) -> Option<WikipediaArticle> {
    let page = body.get("query")?.get("pages")?.as_array()?.first()?;
    if page.get("missing").is_some() || page.get("invalid").is_some() {
        return None;
    }
    let title = page.get("title")?.as_str()?.to_string();
    let url = page.get("fullurl")?.as_str()?.to_string();
    if !url.starts_with("https://") {
        return None;
    }
    let extract = page.get("extract")?.as_str()?;
    let lines = extract.lines().collect::<Vec<_>>();
    let first_heading = lines
        .iter()
        .position(|line| heading(line).is_some())
        .unwrap_or(lines.len());
    let lead = tidy_text(&lines[..first_heading]);
    if lead.is_empty() {
        return None;
    }
    let mut reception = None;
    let mut index = first_heading;
    while index < lines.len() {
        if let Some((level, title)) = heading(lines[index])
            && is_reception_heading(title)
        {
            let end = lines[index + 1..]
                .iter()
                .position(|line| heading(line).is_some_and(|(next, _)| next <= level))
                .map_or(lines.len(), |offset| index + 1 + offset);
            let text = tidy_text(&lines[index + 1..end]);
            if !text.is_empty() {
                reception = Some(text);
                break;
            }
        }
        index += 1;
    }
    Some(WikipediaArticle {
        title,
        url,
        lead,
        reception,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommonsImage {
    pub download_url: String,
    pub page_url: String,
    pub mime_type: String,
    pub author: String,
    pub license: String,
    pub license_url: Option<String>,
}

/// Whether a Commons license short name allows display with attribution.
pub fn reusable_license(license: &str) -> bool {
    let normalized = license.to_lowercase().replace(['-', '_'], " ");
    let words = normalized.split_whitespace().collect::<Vec<_>>();
    if words
        .iter()
        .any(|word| matches!(*word, "nc" | "nd" | "fair" | "non"))
    {
        return false;
    }
    normalized.starts_with("cc by")
        || normalized.starts_with("cc0")
        || normalized.starts_with("public domain")
        || normalized.starts_with("pd")
}

/// Removes markup from Commons' HTML attribution fields.
pub fn strip_html(value: &str) -> String {
    let mut text = String::with_capacity(value.len());
    let mut in_tag = false;
    for character in value.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => text.push(character),
            _ => {}
        }
    }
    let text = text
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ");
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn parse_commons_image(body: &Value) -> Option<CommonsImage> {
    let page = body.get("query")?.get("pages")?.as_array()?.first()?;
    let info = page.get("imageinfo")?.as_array()?.first()?;
    let metadata = info.get("extmetadata")?;
    let field = |name: &str| {
        metadata
            .get(name)
            .and_then(|field| field.get("value"))
            .and_then(Value::as_str)
            .map(strip_html)
            .filter(|value| !value.is_empty())
    };
    let license = field("LicenseShortName")?;
    if !reusable_license(&license) {
        return None;
    }
    let mime_type = info.get("mime")?.as_str()?.to_string();
    if !matches!(
        mime_type.as_str(),
        "image/jpeg" | "image/png" | "image/webp"
    ) {
        return None;
    }
    let download_url = info
        .get("thumburl")
        .or_else(|| info.get("url"))?
        .as_str()?
        .to_string();
    let page_url = info.get("descriptionurl")?.as_str()?.to_string();
    if !download_url.starts_with("https://upload.wikimedia.org/")
        || !page_url.starts_with("https://")
    {
        return None;
    }
    Some(CommonsImage {
        download_url,
        page_url,
        mime_type,
        author: field("Artist").unwrap_or_else(|| "Wikimedia Commons contributor".to_string()),
        license,
        license_url: field("LicenseUrl").filter(|url| url.starts_with("http")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wikidata_entities_give_sitelinks_image_and_dates() {
        let entity = parse_wikidata_entity(&json!({"entities": {"Q93341": {
            "sitelinks": {
                "enwiki": {"title": "Miles Davis"},
                "dewiki": {"title": "Miles Davis (Musiker)"},
                "commonswiki": {"title": "Category:Miles Davis"}
            },
            "claims": {
                "P18": [{"rank": "normal", "mainsnak": {"datavalue": {"value": "Miles Davis 1984.jpg"}}}],
                "P569": [{"rank": "normal", "mainsnak": {"datavalue": {"value": {"time": "+1926-05-26T00:00:00Z", "precision": 11}}}}],
                "P570": [{"rank": "deprecated", "mainsnak": {"datavalue": {"value": {"time": "+1990-01-01T00:00:00Z", "precision": 11}}}},
                         {"rank": "normal", "mainsnak": {"datavalue": {"value": {"time": "+1991-09-28T00:00:00Z", "precision": 11}}}}],
                "P571": [{"mainsnak": {"datavalue": {"value": {"time": "+1960-00-00T00:00:00Z", "precision": 9}}}}]
            }
        }}}))
        .unwrap();
        assert_eq!(entity.id, "Q93341");
        assert_eq!(
            entity.article("de"),
            Some(("de".into(), "Miles Davis (Musiker)".into()))
        );
        assert_eq!(
            entity.article("fr"),
            Some(("en".into(), "Miles Davis".into()))
        );
        assert_eq!(entity.image.as_deref(), Some("Miles Davis 1984.jpg"));
        assert_eq!(entity.born.as_deref(), Some("1926-05-26"));
        assert_eq!(entity.died.as_deref(), Some("1991-09-28"));
        assert_eq!(entity.formed.as_deref(), Some("1960"));
        assert!(
            !entity
                .sitelinks
                .iter()
                .any(|(language, _)| language == "commons")
        );
    }

    #[test]
    fn wikipedia_extracts_split_lead_and_reception() {
        let extract = "Kind of Blue is a studio album by Miles Davis.\nIt was recorded in 1959.\n\n\n== Background ==\nDavis had...\n\n== Critical reception ==\nThe album was praised.\n\n=== Accolades ===\nIt is often ranked first.\n\n== Track listing ==\nSo What";
        let article = parse_wikipedia_extract(&json!({"query": {"pages": [{
            "title": "Kind of Blue", "fullurl": "https://en.wikipedia.org/wiki/Kind_of_Blue",
            "extract": extract
        }]}}))
        .unwrap();
        assert_eq!(
            article.lead,
            "Kind of Blue is a studio album by Miles Davis.\n\nIt was recorded in 1959."
        );
        assert_eq!(
            article.reception.as_deref(),
            Some("The album was praised.\n\nIt is often ranked first.")
        );
        assert!(
            parse_wikipedia_extract(
                &json!({"query": {"pages": [{"title": "Nope", "missing": true}]}})
            )
            .is_none()
        );
    }

    #[test]
    fn long_text_is_cut_at_a_paragraph() {
        let paragraph = "word ".repeat(1_000);
        let lines = [paragraph.as_str(), paragraph.as_str(), paragraph.as_str()];
        let text = tidy_text(&lines);
        assert!(text.chars().count() <= MAX_TEXT_CHARS);
        assert_eq!(text.matches("\n\n").count(), 0);
    }

    #[test]
    fn only_reusable_commons_licenses_are_accepted() {
        for license in ["CC BY-SA 4.0", "CC BY 2.0", "CC0", "Public domain", "PD-US"] {
            assert!(reusable_license(license), "{license}");
        }
        for license in [
            "CC BY-NC 2.0",
            "CC BY-ND 4.0",
            "Fair use",
            "All rights reserved",
        ] {
            assert!(!reusable_license(license), "{license}");
        }
    }

    #[test]
    fn commons_image_info_requires_license_and_attribution() {
        let body = json!({"query": {"pages": [{"imageinfo": [{
            "thumburl": "https://upload.wikimedia.org/thumb/a/ab/Miles.jpg/1000px-Miles.jpg",
            "url": "https://upload.wikimedia.org/a/ab/Miles.jpg",
            "descriptionurl": "https://commons.wikimedia.org/wiki/File:Miles.jpg",
            "mime": "image/jpeg",
            "extmetadata": {
                "LicenseShortName": {"value": "CC BY-SA 2.0"},
                "LicenseUrl": {"value": "https://creativecommons.org/licenses/by-sa/2.0"},
                "Artist": {"value": "<a href=\"//commons.wikimedia.org/wiki/User:Tom\">Tom &amp; Co</a>"}
            }
        }]}]}});
        let image = parse_commons_image(&body).unwrap();
        assert_eq!(image.author, "Tom & Co");
        assert!(image.download_url.contains("1000px"));
        let mut restricted = body.clone();
        restricted["query"]["pages"][0]["imageinfo"][0]["extmetadata"]["LicenseShortName"]["value"] =
            json!("CC BY-NC 2.0");
        assert!(parse_commons_image(&restricted).is_none());
    }

    #[test]
    fn urls_are_encoded_and_languages_validated() {
        assert_eq!(
            wikipedia_extract_url("en", "AC/DC"),
            "https://en.wikipedia.org/w/api.php?action=query&format=json&formatversion=2&prop=extracts%7Cinfo&inprop=url&explaintext=1&exsectionformat=wiki&redirects=1&titles=AC%2FDC"
        );
        assert!(commons_image_info_url("Miles Davis.jpg").ends_with("File%3AMiles%20Davis%2Ejpg"));
        assert!(valid_language("en"));
        assert!(valid_language("zh-yue"));
        assert!(!valid_language("en.evil.com"));
        assert!(!valid_language("EN"));
    }

    #[test]
    fn rest_urls_use_cached_title_paths() {
        assert_eq!(
            wikipedia_summary_url("en", "Kind of Blue"),
            "https://en.wikipedia.org/api/rest_v1/page/summary/Kind_of_Blue"
        );
        assert_eq!(
            wikipedia_page_url("en", "AC/DC"),
            "https://en.wikipedia.org/wiki/AC%2FDC"
        );
        assert_eq!(
            wikipedia_summary_url("de", "Symphonie Nr. 9 (Beethoven)"),
            "https://de.wikipedia.org/api/rest_v1/page/summary/Symphonie_Nr._9_(Beethoven)"
        );
        assert_eq!(
            commons_wikitext_url("Björk 2007.jpg"),
            "https://commons.wikimedia.org/wiki/File:Bj%C3%B6rk_2007.jpg?action=raw"
        );
        assert!(commons_thumbnail_url("A b.jpg").ends_with("Special:FilePath/A_b.jpg?width=500"));
    }

    #[test]
    fn wikipedia_links_from_musicbrainz_are_parsed() {
        assert_eq!(
            parse_wikipedia_url("https://en.wikipedia.org/wiki/Symphony_No._9_(Beethoven)"),
            Some(("en".into(), "Symphony No. 9 (Beethoven)".into()))
        );
        assert_eq!(
            parse_wikipedia_url("https://de.m.wikipedia.org/wiki/Bj%C3%B6rk#Leben"),
            Some(("de".into(), "Björk".into()))
        );
        assert_eq!(
            parse_wikipedia_url("https://en.wikipedia.org/wiki/File:X.jpg"),
            None
        );
        assert_eq!(parse_wikipedia_url("https://evil.example/wiki/X"), None);
        assert_eq!(
            parse_wikipedia_url("https://en.wikipedia.org/w/index.php?title=X"),
            None
        );
    }

    #[test]
    fn summaries_give_the_item_and_only_a_commons_page_image() {
        let body = json!({
            "type": "standard", "title": "Nirvana (band)",
            "titles": {"normalized": "Nirvana (band)"},
            "wikibase_item": "Q11649",
            "description": "American rock band (1987–1994)",
            "extract": "Nirvana was an American rock band.",
            "originalimage": {"source": "https://upload.wikimedia.org/wikipedia/commons/1/19/Nirvana_around_1992.jpg?utm_source=x"},
            "content_urls": {"desktop": {"page": "https://en.wikipedia.org/wiki/Nirvana_(band)"}}
        });
        let summary = parse_wikipedia_summary(&body).unwrap();
        assert_eq!(summary.wikibase_item.as_deref(), Some("Q11649"));
        assert_eq!(
            summary.commons_image.as_deref(),
            Some("Nirvana around 1992.jpg")
        );
        assert!(!summary.disambiguation);
        // A local (non-free) upload such as an album cover is never a photo.
        let mut cover = body.clone();
        cover["originalimage"]["source"] =
            json!("https://upload.wikimedia.org/wikipedia/en/1/10/Kind_of_Blue.jpg");
        assert_eq!(parse_wikipedia_summary(&cover).unwrap().commons_image, None);
        let mut thumb = body.clone();
        thumb["originalimage"] = Value::Null;
        thumb["thumbnail"] = json!({"source": "https://thumb.wikimedia.org/wikipedia/commons/thumb/1/19/Nirvana_around_1992.jpg/330px-Nirvana_around_1992.jpg"});
        assert_eq!(
            parse_wikipedia_summary(&thumb)
                .unwrap()
                .commons_image
                .as_deref(),
            Some("Nirvana around 1992.jpg")
        );
        let mut page = body.clone();
        page["type"] = json!("disambiguation");
        assert!(parse_wikipedia_summary(&page).unwrap().disambiguation);
        assert!(
            parse_wikipedia_summary(
                &json!({"type": "https://mediawiki.org/wiki/HyperSwitch/errors/not_found"})
            )
            .is_none()
        );
    }

    #[test]
    fn article_html_gives_the_lead_and_the_reception_section() {
        let html = r#"<section data-mw-section-id="0"><div role="note" class="hatnote">For other uses, see X.</div>
<table class="infobox"><tbody><tr><td><table><tr><td><p>nested</p></td></tr></table><p>Infobox text</p></td></tr></tbody></table>
<p><i><b>Nevermind</b></i> is the second album by <a href="./Nirvana_(band)">Nirvana</a>.<sup class="mw-ref"><a>[4]</a></sup></p>
<p class="mw-empty-elt"></p><p>It sold &#8220;30&nbsp;million&#8221; copies &amp; more.</p><style>.x{}</style></section>
<section data-mw-section-id="1"><h2 id="Recording">Recording</h2><p>Recorded in 1991.</p></section>
<section data-mw-section-id="2"><h2 id="Critical_reception">Critical reception</h2><table class="wikitable"><tr><td><p>Rolling Stone 5/5</p></td></tr></table><p>Critics praised it.</p><blockquote><p>A quote.</p></blockquote></section>
<section data-mw-section-id="3"><h3 id="Accolades">Accolades</h3><p>Often ranked first.</p></section>
<section data-mw-section-id="4"><h2 id="Legacy">Legacy</h2><p>Later.</p></section>"#;
        let (lead, reception) = parse_article_html(html);
        assert_eq!(
            lead,
            "Nevermind is the second album by Nirvana.\n\nIt sold \u{201c}30 million\u{201d} copies & more."
        );
        assert_eq!(
            reception.as_deref(),
            Some("Critics praised it.\n\nOften ranked first.")
        );
        let (lead, reception) = parse_article_html("<p>Just text.</p>");
        assert_eq!((lead.as_str(), reception), ("Just text.", None));
    }

    #[test]
    fn commons_wikitext_gives_license_and_author() {
        let image = parse_commons_wikitext(
            "Miles Davis 1984.jpg",
            "=={{int:filedesc}}==\n{{Information\n|Description={{en|1=Miles}}\n|Author=[https://flickr.com/x Tom Palumbo] from [[New York]]\n}}\n{{self|GFDL|cc-by-sa-4.0,3.0,2.5}}",
        )
        .unwrap();
        assert_eq!(image.license, "CC BY-SA 4.0");
        assert_eq!(
            image.license_url.as_deref(),
            Some("https://creativecommons.org/licenses/by-sa/4.0/")
        );
        assert_eq!(image.author, "Tom Palumbo from New York");
        assert_eq!(image.mime_type, "image/jpeg");
        assert_eq!(
            image.page_url,
            "https://commons.wikimedia.org/wiki/File:Miles_Davis_1984.jpg"
        );
        let public =
            parse_commons_wikitext("A.png", "{{Information|author=Unknown}}\n{{PD-old-70}}")
                .unwrap();
        assert_eq!(
            (public.license.as_str(), public.license_url),
            ("Public domain", None)
        );
        let regional = parse_commons_wikitext("B.jpg", "{{Cc-by-3.0-de}}").unwrap();
        assert_eq!(regional.license, "CC BY 3.0");
        assert_eq!(regional.author, "Wikimedia Commons contributor");
        let listed =
            parse_commons_wikitext("F.jpg", "|Author=\n* Joop van Bilsen / Anefo\n{{cc0}}");
        assert_eq!(listed.unwrap().author, "Wikimedia Commons contributor");
        let bullet = parse_commons_wikitext("G.jpg", "|Author=* Joop van Bilsen / Anefo\n{{cc0}}");
        assert_eq!(bullet.unwrap().author, "Joop van Bilsen / Anefo");
        // Non-commercial or unlicensed files, and non-images, are refused.
        assert!(parse_commons_wikitext("C.jpg", "{{cc-by-nc-2.0}}").is_none());
        assert!(parse_commons_wikitext("D.jpg", "{{Information|author=X}}").is_none());
        assert!(parse_commons_wikitext("E.svg", "{{cc0}}").is_none());
    }

    #[test]
    fn entities_are_decoded() {
        assert_eq!(
            decode_entities("a &amp; b &#91;1&#93; &#x2013; &bogus; &"),
            "a & b [1] – &bogus; &"
        );
    }

    #[test]
    fn a_desktop_page_gives_everything_the_summary_would() {
        let html = r#"<!DOCTYPE html><html><head><script>RLCONF={"wgTitle":"Nevermind","wgCategories":["Good articles","Articles with short description"],"wgWikibaseItemId":"Q17444"};</script>
<link rel="canonical" href="https://en.wikipedia.org/wiki/Nevermind">
<meta property="og:image" content="https://upload.wikimedia.org/wikipedia/en/b/b7/NirvanaNevermindalbumcover.jpg?utm_source=en.wikipedia.org&amp;utm_content=thumbnail_unscaled">
</head><body><h2>Contents</h2><div id="mw-content-text"><div class="mw-content-ltr mw-parser-output" lang="en" dir="ltr"><div class="shortdescription nomobile noexcerpt" style="display:none" data-mw='{"parts":[{"wt":"May 2&lt;ref>{{Cite web}}&lt;/ref>"}]}'>1991 studio album</div>
<p><i><b>Nevermind</b></i> is the second album by <span typeof="mw:Transclusion" data-mw='{"wt":"a > b"}'>Nirvana</span>.</p>
<div class="mw-heading mw-heading2"><h2 id="Critical_reception">Critical reception</h2></div><p>Critics praised it.</p>
<div class="mw-heading mw-heading2"><h2 id="References">References</h2></div><ol class="references"><li>Ref</li></ol>
</div></div><div class="printfooter">Retrieved from</div><div id="catlinks"><p>Categories: Nirvana albums</p></div></body></html>"#;
        let page = parse_article_page(html).unwrap();
        assert_eq!(page.summary.title, "Nevermind");
        assert_eq!(page.summary.url, "https://en.wikipedia.org/wiki/Nevermind");
        assert_eq!(page.summary.wikibase_item.as_deref(), Some("Q17444"));
        assert_eq!(
            page.summary.description.as_deref(),
            Some("1991 studio album")
        );
        assert_eq!(
            page.summary.commons_image, None,
            "the cover is a local upload"
        );
        assert!(!page.summary.disambiguation);
        assert_eq!(page.lead, "Nevermind is the second album by Nirvana.");
        assert_eq!(page.summary.extract, page.lead);
        assert_eq!(page.reception.as_deref(), Some("Critics praised it."));

        let disambiguation = html.replace("\"Good articles\"", "\"Disambiguation pages\"");
        assert!(
            parse_article_page(&disambiguation)
                .unwrap()
                .summary
                .disambiguation
        );
        let photo = html.replace(
            "wikipedia/en/b/b7/NirvanaNevermindalbumcover.jpg",
            "wikipedia/commons/thumb/1/19/Nirvana_around_1992.jpg/500px-Nirvana_around_1992.jpg",
        );
        assert_eq!(
            parse_article_page(&photo)
                .unwrap()
                .summary
                .commons_image
                .as_deref(),
            Some("Nirvana around 1992.jpg")
        );
        assert!(
            parse_article_page("<html><body><p>No canonical link.</p></body></html>").is_none()
        );
    }
}
