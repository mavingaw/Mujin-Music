//! Small XML helpers shared by the UPnP modules.

/// Escapes text for use in XML character data and attribute values.
pub fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 8);
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            // XML 1.0 forbids most control characters, even escaped.
            '\t' | '\n' | '\r' => escaped.push(character),
            character if (character as u32) < 0x20 => {}
            character => escaped.push(character),
        }
    }
    escaped
}

/// Text of the first descendant element called `name` (namespace ignored).
pub fn child_text(node: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    node.descendants()
        .find(|child| child.is_element() && child.tag_name().name() == name)
        .map(|child| child.text().unwrap_or_default().trim().to_string())
}

/// Formats milliseconds as UPnP `H:MM:SS`.
pub fn format_duration(milliseconds: i64) -> String {
    let seconds = milliseconds.max(0) / 1_000;
    format!(
        "{}:{:02}:{:02}",
        seconds / 3_600,
        (seconds / 60) % 60,
        seconds % 60
    )
}

/// Parses UPnP `H:MM:SS[.fff]` durations; `NOT_IMPLEMENTED` and junk are `None`.
pub fn parse_duration(value: &str) -> Option<i64> {
    let value = value.trim();
    let (clock, fraction) = match value.split_once('.') {
        Some((clock, fraction)) => (clock, Some(fraction)),
        None => (value, None),
    };
    let mut parts = clock.split(':');
    let hours: i64 = parts.next()?.parse().ok()?;
    let minutes: i64 = parts.next()?.parse().ok()?;
    let seconds: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || minutes >= 60 || seconds >= 60 || hours < 0 {
        return None;
    }
    let mut milliseconds = ((hours * 60 + minutes) * 60 + seconds) * 1_000;
    if let Some(fraction) = fraction {
        let digits: String = fraction.chars().take(3).collect();
        if !digits.is_empty() && digits.chars().all(|digit| digit.is_ascii_digit()) {
            let scale = 10_i64.pow(3 - digits.len() as u32);
            milliseconds += digits.parse::<i64>().ok()? * scale;
        }
    }
    Some(milliseconds)
}

#[cfg(test)]
mod tests {
    use super::{escape, format_duration, parse_duration};

    #[test]
    fn escapes_markup_and_drops_forbidden_controls() {
        assert_eq!(
            escape("Tom & Jerry <\"live\"> 'mix'\u{1}"),
            "Tom &amp; Jerry &lt;&quot;live&quot;&gt; &apos;mix&apos;"
        );
    }

    #[test]
    fn durations_round_trip_in_upnp_format() {
        assert_eq!(format_duration(3_723_400), "1:02:03");
        assert_eq!(format_duration(-5), "0:00:00");
        assert_eq!(parse_duration("1:02:03"), Some(3_723_000));
        assert_eq!(parse_duration("0:03:25.5"), Some(205_500));
        assert_eq!(parse_duration("NOT_IMPLEMENTED"), None);
        assert_eq!(parse_duration("0:61:00"), None);
    }
}
