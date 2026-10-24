//! Wikidata, Wikipedia and Wikimedia Commons requests and parsing.
//!
//! Wikipedia text is CC BY-SA 4.0 and must be shown with the article title,
//! a link to it and the license. Commons images are used only when their
//! license allows reuse with attribution (CC BY, CC BY-SA, CC0 or public
//! domain).

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::Value;

pub const WIKIPEDIA_LICENSE: &str = "CC BY-SA 4.0";
pub const WIKIPEDIA_LICENSE_URL: &str = "https://creativecommons.org/licenses/by-sa/4.0/";
/// Upper bound for one stored text, cut at a paragraph boundary.
pub const MAX_TEXT_CHARS: usize = 6_000;

fn encode(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
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
}
