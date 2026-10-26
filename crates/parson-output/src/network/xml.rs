//! Just enough XML for UPnP: device descriptions, SOAP envelopes and
//! DIDL-Lite. Elements are matched by local name (any namespace prefix),
//! which is how renderers in the wild differ most.

/// The raw inner content of every `<name …>…</name>` element, in document
/// order. Nested elements of the same name are not supported (UPnP does
/// not nest them). Self-closing elements yield an empty string.
pub fn elements<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let mut found = Vec::new();
    let mut cursor = 0;
    while let Some(offset) = xml[cursor..].find('<') {
        let start = cursor + offset;
        let after = &xml[start + 1..];
        if after.starts_with('/') || after.starts_with('?') || after.starts_with('!') {
            cursor = start + 1;
            continue;
        }
        let tag_end = match after.find('>') {
            Some(index) => start + 1 + index,
            None => break,
        };
        let tag = &xml[start + 1..tag_end];
        let tag_name = tag
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("");
        let local = tag_name.rsplit(':').next().unwrap_or(tag_name);
        if local != name {
            cursor = tag_end + 1;
            continue;
        }
        if tag.ends_with('/') {
            found.push("");
            cursor = tag_end + 1;
            continue;
        }
        let content_start = tag_end + 1;
        let close = format!("</{tag_name}>");
        let Some(close_offset) = xml[content_start..].find(&close) else {
            break;
        };
        found.push(&xml[content_start..content_start + close_offset]);
        cursor = content_start + close_offset + close.len();
    }
    found
}

/// The unescaped, trimmed text of the first `name` element.
pub fn text(xml: &str, name: &str) -> Option<String> {
    elements(xml, name)
        .first()
        .map(|inner| unescape(inner.trim()))
        .filter(|value| !value.is_empty())
}

/// The value of attribute `name` on the first `element` start tag.
pub fn attribute(xml: &str, element: &str, name: &str) -> Option<String> {
    let mut cursor = 0;
    while let Some(offset) = xml[cursor..].find('<') {
        let start = cursor + offset;
        let end = start + xml[start..].find('>')?;
        let tag = &xml[start + 1..end];
        let tag_name = tag.split_whitespace().next().unwrap_or("");
        if tag_name.rsplit(':').next() == Some(element) {
            let needle = format!("{name}=");
            let index = tag.find(&needle)?;
            let rest = &tag[index + needle.len()..];
            let quote = rest.chars().next()?;
            let rest = &rest[1..];
            let close = rest.find(quote)?;
            return Some(unescape(&rest[..close]));
        }
        cursor = end + 1;
    }
    None
}

pub fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find('&') {
        out.push_str(&rest[..index]);
        rest = &rest[index..];
        let Some(end) = rest.find(';').filter(|end| *end <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(character) => {
                out.push(character);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Control characters are not allowed in XML 1.0.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_elements_by_local_name() {
        let xml = r#"<?xml version="1.0"?><root xmlns="urn:x"><device><friendlyName>Den &amp; Kitchen</friendlyName>
            <serviceList><service><serviceType>urn:a</serviceType></service><service><serviceType>urn:b</serviceType></service></serviceList>
            <s:Body><u:Result>&lt;DIDL&gt;</u:Result><empty/></s:Body></device></root>"#;
        assert_eq!(text(xml, "friendlyName").as_deref(), Some("Den & Kitchen"));
        let services = elements(xml, "service");
        assert_eq!(services.len(), 2);
        assert_eq!(text(services[1], "serviceType").as_deref(), Some("urn:b"));
        assert_eq!(text(xml, "Result").as_deref(), Some("<DIDL>"));
        assert_eq!(elements(xml, "empty"), vec![""]);
        assert!(text(xml, "missing").is_none());
    }

    #[test]
    fn reads_attributes_and_round_trips_escapes() {
        let xml = r#"<res protocolInfo="http-get:*:audio/wav:*" bitsPerSample='24'>u</res>"#;
        assert_eq!(
            attribute(xml, "res", "bitsPerSample").as_deref(),
            Some("24")
        );
        assert_eq!(
            attribute(xml, "res", "protocolInfo").as_deref(),
            Some("http-get:*:audio/wav:*")
        );
        let text = "Tom & Jerry <\"live\"> 'x' &#169; &#x263A;";
        assert_eq!(unescape(&escape(text)), text);
        assert_eq!(unescape("&#169;&#x263A;&bogus;"), "\u{a9}\u{263a}&bogus;");
    }
}
