//! Station playlists: what a station URL points at, and the stream URLs
//! inside PLS, M3U, ASX and XSPF files and HLS playlists.
//!
//! Everything here is pure text parsing so it is tested with fixtures.

use reqwest::Url;

/// Most bytes Parson reads from a playlist file.
pub const MAX_PLAYLIST_BYTES: usize = 256 * 1024;

/// What a station URL answered with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Payload {
    Pls,
    M3u,
    /// An HLS playlist (master or media).
    Hls,
    Asx,
    Xspf,
    /// An HTML page (a station's website, not its stream).
    WebPage,
    /// Audio bytes (or something Parson will try to play as audio).
    Audio,
}

fn path_extension(url: &Url) -> String {
    url.path()
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Whether a URL names an HLS playlist by its extension.
pub fn looks_like_hls(url: &Url) -> bool {
    path_extension(url) == "m3u8"
}

/// Decides what a response is from its content type, URL and first bytes.
pub fn sniff(content_type: Option<&str>, url: &Url, head: &[u8]) -> Payload {
    let content_type = content_type
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let text = String::from_utf8_lossy(&head[..head.len().min(4096)]);
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    let lowered = trimmed.to_ascii_lowercase();
    if trimmed.starts_with("#EXTM3U") || trimmed.starts_with("#EXT-X-") {
        return if is_hls_text(trimmed) {
            Payload::Hls
        } else {
            Payload::M3u
        };
    }
    if content_type == "text/html"
        || content_type == "application/xhtml+xml"
        || lowered.starts_with("<!doctype html")
        || lowered.starts_with("<html")
    {
        return Payload::WebPage;
    }
    if lowered.starts_with("[playlist]") {
        return Payload::Pls;
    }
    if lowered.starts_with("<asx") {
        return Payload::Asx;
    }
    if lowered.starts_with("<?xml") && lowered.contains("<playlist") && lowered.contains("xspf") {
        return Payload::Xspf;
    }
    match content_type.as_str() {
        "application/vnd.apple.mpegurl" | "application/x-mpegurl" => return Payload::Hls,
        "audio/x-scpls" | "application/pls+xml" | "audio/scpls" => return Payload::Pls,
        "audio/x-mpegurl" | "audio/mpegurl" => return Payload::M3u,
        "video/x-ms-asf" | "video/x-ms-asx" | "audio/x-ms-wax" => {
            if lowered.starts_with('<') {
                return Payload::Asx;
            }
        }
        "application/xspf+xml" => return Payload::Xspf,
        _ => {}
    }
    let extension = path_extension(url);
    let textual = content_type.starts_with("text/")
        || content_type.is_empty()
        || content_type == "application/octet-stream";
    if textual && looks_textual(head) {
        match extension.as_str() {
            "pls" => return Payload::Pls,
            "m3u" => return Payload::M3u,
            "m3u8" => return Payload::Hls,
            "asx" | "wax" => return Payload::Asx,
            "xspf" => return Payload::Xspf,
            _ => {}
        }
        // A bare list of URLs served as text.
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            return Payload::M3u;
        }
    }
    Payload::Audio
}

fn looks_textual(head: &[u8]) -> bool {
    let sample = &head[..head.len().min(512)];
    !sample.is_empty()
        && sample
            .iter()
            .all(|byte| byte.is_ascii_graphic() || byte.is_ascii_whitespace() || *byte >= 0x80)
        && std::str::from_utf8(sample).is_ok()
}

fn is_hls_text(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim();
        line.starts_with("#EXT-X-TARGETDURATION")
            || line.starts_with("#EXT-X-STREAM-INF")
            || line.starts_with("#EXT-X-MEDIA-SEQUENCE")
            || line.starts_with("#EXT-X-MEDIA:")
            || line.starts_with("#EXT-X-VERSION")
    })
}

/// One entry of a playlist file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub url: Url,
    pub title: Option<String>,
}

fn join(base: &Url, raw: &str) -> Option<Url> {
    let url = base.join(raw.trim()).ok()?;
    matches!(url.scheme(), "http" | "https").then_some(url)
}

/// Parses `[playlist]` files: `File1=...`, `Title1=...`.
pub fn parse_pls(text: &str, base: &Url) -> Vec<Entry> {
    let mut files: Vec<(u32, Url)> = Vec::new();
    let mut titles: Vec<(u32, String)> = Vec::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        if let Some(index) = key.strip_prefix("file").and_then(|n| n.parse::<u32>().ok()) {
            if let Some(url) = join(base, value) {
                files.push((index, url));
            }
        } else if let Some(index) = key
            .strip_prefix("title")
            .and_then(|n| n.parse::<u32>().ok())
        {
            let value = value.trim();
            if !value.is_empty() {
                titles.push((index, value.to_string()));
            }
        }
    }
    files.sort_by_key(|(index, _)| *index);
    files
        .into_iter()
        .map(|(index, url)| Entry {
            url,
            title: titles
                .iter()
                .find(|(title_index, _)| *title_index == index)
                .map(|(_, title)| title.clone()),
        })
        .collect()
}

/// Parses plain and extended M3U.
pub fn parse_m3u(text: &str, base: &Url) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut title = None;
    for line in text.lines() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() {
            continue;
        }
        if let Some(info) = line.strip_prefix("#EXTINF:") {
            title = info
                .split_once(',')
                .map(|(_, title)| title.trim().to_string())
                .filter(|title| !title.is_empty());
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        if let Some(url) = join(base, line) {
            entries.push(Entry {
                url,
                title: title.take(),
            });
        }
    }
    entries
}

fn xml_attribute(tag: &str, name: &str) -> Option<String> {
    let lowered = tag.to_ascii_lowercase();
    let at = lowered.find(&format!("{name}="))? + name.len() + 1;
    let rest = &tag[at..];
    let quote = rest.chars().next()?;
    if quote == '"' || quote == '\'' {
        let value = &rest[1..];
        Some(value[..value.find(quote)?].to_string())
    } else {
        Some(
            rest.split(|character: char| {
                character.is_whitespace() || character == '>' || character == '/'
            })
            .next()?
            .to_string(),
        )
    }
}

fn unescape_xml(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// Parses ASX (`<ref href="...">`) files.
pub fn parse_asx(text: &str, base: &Url) -> Vec<Entry> {
    let lowered = text.to_ascii_lowercase();
    let mut entries = Vec::new();
    let mut from = 0;
    while let Some(start) = lowered[from..].find("<ref") {
        let start = from + start;
        let end = lowered[start..]
            .find('>')
            .map(|end| start + end)
            .unwrap_or(text.len());
        if let Some(href) = xml_attribute(&text[start..end], "href")
            && let Some(url) = join(base, &unescape_xml(&href))
        {
            entries.push(Entry { url, title: None });
        }
        from = end;
    }
    entries
}

/// Parses XSPF (`<location>...</location>`) files.
pub fn parse_xspf(text: &str, base: &Url) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("<location>") {
        let after = &rest[start + "<location>".len()..];
        let Some(end) = after.find("</location>") else {
            break;
        };
        if let Some(url) = join(base, &unescape_xml(after[..end].trim())) {
            entries.push(Entry { url, title: None });
        }
        rest = &after[end..];
    }
    entries
}

/// Parses any playlist file into candidate stream URLs, best first.
pub fn entries(kind: Payload, text: &str, base: &Url) -> Vec<Entry> {
    let mut entries = match kind {
        Payload::Pls => parse_pls(text, base),
        Payload::M3u => parse_m3u(text, base),
        Payload::Asx => parse_asx(text, base),
        Payload::Xspf => parse_xspf(text, base),
        Payload::Hls | Payload::Audio | Payload::WebPage => Vec::new(),
    };
    let mut seen = std::collections::HashSet::new();
    entries.retain(|entry| seen.insert(entry.url.clone()));
    entries.truncate(8);
    entries
}

/// One rendition in an HLS master playlist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    pub url: Url,
    pub bandwidth: Option<u64>,
    pub codecs: Option<String>,
}

impl Variant {
    /// The audio codec named by `CODECS`, in Parson's names.
    pub fn audio_codec(&self) -> Option<&'static str> {
        hls_codec(self.codecs.as_deref()?)
    }
}

/// Maps HLS `CODECS` values to codec names.
pub fn hls_codec(codecs: &str) -> Option<&'static str> {
    codecs.split(',').find_map(|codec| {
        let codec = codec.trim().to_ascii_lowercase();
        match codec.as_str() {
            "flac" | "fla" => Some("flac"),
            "alac" => Some("alac"),
            "opus" => Some("opus"),
            "ac-3" => Some("ac3"),
            "ec-3" => Some("eac3"),
            "mp3" | "mp4a.40.34" | "mp4a.6b" => Some("mp3"),
            _ if codec.starts_with("mp4a.40") => Some("aac"),
            _ => None,
        }
    })
}

/// Splits an HLS attribute list (`KEY=VALUE,KEY="A,B"`).
pub fn attributes(list: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = list.trim();
    while !rest.is_empty() {
        let Some((key, after)) = rest.split_once('=') else {
            break;
        };
        let (value, remaining) = if let Some(quoted) = after.strip_prefix('"') {
            match quoted.find('"') {
                Some(end) => (&quoted[..end], quoted[end + 1..].trim_start_matches(',')),
                None => (quoted, ""),
            }
        } else {
            match after.find(',') {
                Some(end) => (&after[..end], &after[end + 1..]),
                None => (after, ""),
            }
        };
        out.push((key.trim().to_ascii_uppercase(), value.to_string()));
        rest = remaining.trim_start();
    }
    out
}

fn attribute<'a>(list: &'a [(String, String)], name: &str) -> Option<&'a str> {
    list.iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// Whether an HLS playlist is a master playlist.
pub fn is_master(text: &str) -> bool {
    text.lines()
        .any(|line| line.trim().starts_with("#EXT-X-STREAM-INF"))
}

/// Renditions of a master playlist, plus alternative audio renditions
/// (`#EXT-X-MEDIA:TYPE=AUDIO`).
pub fn parse_master(text: &str, base: &Url) -> Vec<Variant> {
    let mut variants = Vec::new();
    let mut pending: Option<Vec<(String, String)>> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(list) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            pending = Some(attributes(list));
            continue;
        }
        if let Some(list) = line.strip_prefix("#EXT-X-MEDIA:") {
            let list = attributes(list);
            if attribute(&list, "TYPE") == Some("AUDIO")
                && let Some(uri) = attribute(&list, "URI")
                && let Some(url) = join(base, uri)
            {
                variants.push(Variant {
                    url,
                    bandwidth: None,
                    codecs: None,
                });
            }
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        if let Some(list) = pending.take()
            && let Some(url) = join(base, line)
        {
            variants.push(Variant {
                url,
                bandwidth: attribute(&list, "AVERAGE-BANDWIDTH")
                    .or_else(|| attribute(&list, "BANDWIDTH"))
                    .and_then(|value| value.parse().ok()),
                codecs: attribute(&list, "CODECS").map(str::to_string),
            });
        }
    }
    variants
}

/// The rendition Parson plays: lossless first, then the highest bandwidth;
/// renditions with video are skipped when audio-only ones exist.
pub fn best_variant(variants: &[Variant]) -> Option<&Variant> {
    let audio_only = |variant: &&Variant| {
        variant
            .codecs
            .as_deref()
            .is_none_or(|codecs| codecs.split(',').all(|codec| hls_codec(codec).is_some()))
    };
    let pool: Vec<&Variant> = if variants.iter().any(|variant| audio_only(&variant)) {
        variants.iter().filter(audio_only).collect()
    } else {
        variants.iter().collect()
    };
    pool.into_iter().max_by_key(|variant| {
        let lossless = matches!(variant.audio_codec(), Some("flac" | "alac"));
        (lossless, variant.bandwidth.unwrap_or(0))
    })
}

/// One segment of an HLS media playlist.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub sequence: u64,
    pub url: Url,
    pub duration: f64,
    /// `#EXTINF` title, which some stations fill with the current track.
    pub title: Option<String>,
}

/// An HLS media playlist.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaPlaylist {
    pub target_duration: f64,
    pub segments: Vec<Segment>,
    /// `#EXT-X-MAP` initialization segment (fragmented MP4).
    pub init: Option<Url>,
    /// `#EXT-X-ENDLIST`: not live.
    pub ended: bool,
    /// Segments are encrypted (`#EXT-X-KEY` with a method other than NONE).
    pub encrypted: bool,
}

/// Reads a track title from an `#EXTINF` description: either `title="..."`
/// attributes or plain `Artist - Title` text.
fn extinf_title(description: &str) -> Option<String> {
    let description = description.trim();
    if description.is_empty() {
        return None;
    }
    if description.contains("title=") {
        let list = attributes(description);
        let title = attribute(&list, "TITLE").map(str::trim).unwrap_or_default();
        let artist = attribute(&list, "ARTIST")
            .map(str::trim)
            .unwrap_or_default();
        return match (artist.is_empty(), title.is_empty()) {
            (false, false) => Some(format!("{artist} - {title}")),
            (true, false) => Some(title.to_string()),
            _ => None,
        };
    }
    // Some packagers write a timestamp or a number here.
    if description
        .chars()
        .all(|character| character.is_ascii_digit() || matches!(character, '.' | ':' | '-' | ' '))
    {
        return None;
    }
    Some(description.to_string())
}

pub fn parse_media(text: &str, base: &Url) -> MediaPlaylist {
    let mut playlist = MediaPlaylist {
        target_duration: 6.0,
        segments: Vec::new(),
        init: None,
        ended: false,
        encrypted: false,
    };
    let mut sequence = 0_u64;
    let mut pending: Option<(f64, Option<String>)> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(value) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            if let Ok(value) = value.trim().parse::<f64>() {
                playlist.target_duration = value.clamp(1.0, 60.0);
            }
        } else if let Some(value) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            sequence = value.trim().parse().unwrap_or(0);
        } else if let Some(list) = line.strip_prefix("#EXT-X-MAP:") {
            let list = attributes(list);
            playlist.init = attribute(&list, "URI").and_then(|uri| join(base, uri));
        } else if let Some(list) = line.strip_prefix("#EXT-X-KEY:") {
            let list = attributes(list);
            playlist.encrypted = attribute(&list, "METHOD").is_some_and(|method| method != "NONE");
        } else if line.starts_with("#EXT-X-ENDLIST") {
            playlist.ended = true;
        } else if let Some(info) = line.strip_prefix("#EXTINF:") {
            let (duration, description) = info.split_once(',').unwrap_or((info, ""));
            pending = Some((
                duration.trim().parse().unwrap_or(0.0),
                extinf_title(description),
            ));
        } else if !line.starts_with('#') {
            if let Some(url) = join(base, line) {
                let (duration, title) = pending.take().unwrap_or((0.0, None));
                playlist.segments.push(Segment {
                    sequence,
                    url,
                    duration,
                    title,
                });
            }
            sequence += 1;
        }
    }
    playlist
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://radio.example/streams/station.pls").unwrap()
    }

    #[test]
    fn pls_files_list_streams_in_order_with_titles() {
        let text = "[playlist]\nNumberOfEntries=2\nFile2=http://backup.example:8000/live\nTitle2=Backup\nFile1=https://main.example/live.aac\nTitle1=Main (AAC 128)\nLength1=-1\nVersion=2\n";
        let entries = parse_pls(text, &base());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].url.as_str(), "https://main.example/live.aac");
        assert_eq!(entries[0].title.as_deref(), Some("Main (AAC 128)"));
        assert_eq!(entries[1].url.as_str(), "http://backup.example:8000/live");
    }

    #[test]
    fn m3u_files_resolve_relative_urls_and_skip_other_schemes() {
        let text = "#EXTM3U\n#EXTINF:-1,Jazz FM\nlive.mp3\nrtsp://ignored.example/x\n\nhttps://other.example/a.ogg\n";
        let entries = parse_m3u(text, &base());
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0].url.as_str(),
            "https://radio.example/streams/live.mp3"
        );
        assert_eq!(entries[0].title.as_deref(), Some("Jazz FM"));
        assert_eq!(entries[1].title, None);
    }

    #[test]
    fn asx_and_xspf_files_are_read() {
        let asx = r#"<ASX version="3.0"><Entry><Ref href="http://a.example/1?x=1&amp;y=2"/><REF HREF='http://a.example/2' /></Entry></ASX>"#;
        let entries = parse_asx(asx, &base());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].url.as_str(), "http://a.example/1?x=1&y=2");
        let xspf = r#"<?xml version="1.0"?><playlist version="1" xmlns="http://xspf.org/ns/0/"><trackList><track><location>https://b.example/live.flac</location></track></trackList></playlist>"#;
        assert_eq!(
            parse_xspf(xspf, &base())[0].url.as_str(),
            "https://b.example/live.flac"
        );
        assert_eq!(sniff(None, &base(), xspf.as_bytes()), Payload::Xspf);
    }

    #[test]
    fn sniffing_uses_content_type_extension_and_bytes() {
        let url = Url::parse("https://radio.example/listen.pls").unwrap();
        assert_eq!(
            sniff(Some("audio/x-scpls"), &url, b"[playlist]"),
            Payload::Pls
        );
        assert_eq!(
            sniff(Some("text/plain"), &url, b"[Playlist]\nFile1=x"),
            Payload::Pls
        );
        assert_eq!(
            sniff(Some("audio/mpeg"), &url, &[0xff, 0xfb, 0x90, 0x64]),
            Payload::Audio
        );
        let hls = Url::parse("https://radio.example/live/master.m3u8").unwrap();
        assert_eq!(
            sniff(
                Some("application/vnd.apple.mpegurl"),
                &hls,
                b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1"
            ),
            Payload::Hls
        );
        // An .m3u that is really HLS is played as HLS.
        let m3u = Url::parse("https://radio.example/a.m3u").unwrap();
        assert_eq!(
            sniff(
                Some("audio/x-mpegurl"),
                &m3u,
                b"#EXTM3U\n#EXT-X-TARGETDURATION:6\n"
            ),
            Payload::Hls
        );
        assert_eq!(
            sniff(Some("audio/x-mpegurl"), &m3u, b"#EXTM3U\nhttp://x/1\n"),
            Payload::M3u
        );
        assert_eq!(
            sniff(
                Some("text/plain"),
                &Url::parse("https://r.example/x").unwrap(),
                b"http://a.example/live\n"
            ),
            Payload::M3u
        );
        // Binary data with a playlist extension is still audio.
        assert_eq!(sniff(None, &url, &[0x00, 0xff, 0x10]), Payload::Audio);
        assert_eq!(
            sniff(
                Some("text/html; charset=utf-8"),
                &url,
                b"<!DOCTYPE html><html>"
            ),
            Payload::WebPage
        );
        assert_eq!(sniff(None, &url, b"<html><body>"), Payload::WebPage);
        assert!(looks_like_hls(&hls));
    }

    #[test]
    fn master_playlists_prefer_lossless_audio_then_bandwidth() {
        let text = "#EXTM3U\n\
            #EXT-X-STREAM-INF:BANDWIDTH=64000,CODECS=\"mp4a.40.5\"\nlow/index.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=320000,AVERAGE-BANDWIDTH=310000,CODECS=\"mp4a.40.2\"\nhigh/index.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=900000,CODECS=\"fLaC\"\nflac/index.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=3000000,CODECS=\"avc1.64001f,mp4a.40.2\"\nvideo/index.m3u8\n";
        let base = Url::parse("https://hls.example/station/master.m3u8").unwrap();
        assert!(is_master(text));
        let variants = parse_master(text, &base);
        assert_eq!(variants.len(), 4);
        assert_eq!(variants[1].bandwidth, Some(310_000));
        let best = best_variant(&variants).unwrap();
        assert_eq!(
            best.url.as_str(),
            "https://hls.example/station/flac/index.m3u8"
        );
        assert_eq!(best.audio_codec(), Some("flac"));
        let lossy = best_variant(&variants[..2]).unwrap();
        assert_eq!(lossy.audio_codec(), Some("aac"));
        assert_eq!(
            lossy.url.as_str(),
            "https://hls.example/station/high/index.m3u8"
        );
    }

    #[test]
    fn media_playlists_carry_sequence_titles_and_init_segments() {
        let text = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:10\n#EXT-X-MEDIA-SEQUENCE:4021\n\
            #EXT-X-MAP:URI=\"init.mp4\"\n\
            #EXTINF:10.0,title=\"So What\",artist=\"Miles Davis\"\nseg4021.m4s\n\
            #EXTINF:9.98,\nseg4022.m4s\n\
            #EXTINF:10,Nina Simone - Feeling Good\nhttps://cdn.example/seg4023.m4s\n";
        let base = Url::parse("https://hls.example/live/audio.m3u8").unwrap();
        let playlist = parse_media(text, &base);
        assert_eq!(playlist.target_duration, 10.0);
        assert_eq!(
            playlist.init.as_ref().unwrap().as_str(),
            "https://hls.example/live/init.mp4"
        );
        assert_eq!(playlist.segments.len(), 3);
        assert_eq!(playlist.segments[0].sequence, 4021);
        assert_eq!(
            playlist.segments[0].title.as_deref(),
            Some("Miles Davis - So What")
        );
        assert_eq!(playlist.segments[1].title, None);
        assert_eq!(playlist.segments[2].sequence, 4023);
        assert_eq!(
            playlist.segments[2].title.as_deref(),
            Some("Nina Simone - Feeling Good")
        );
        assert!(!playlist.ended);
        assert!(!playlist.encrypted);

        let encrypted = parse_media(
            "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"k\"\n#EXTINF:6,\na.ts\n#EXT-X-ENDLIST\n",
            &base,
        );
        assert!(encrypted.encrypted);
        assert!(encrypted.ended);
    }

    #[test]
    fn attribute_lists_keep_quoted_commas() {
        let list = attributes("BANDWIDTH=1,CODECS=\"mp4a.40.2,avc1\",NAME=x");
        assert_eq!(list[1], ("CODECS".into(), "mp4a.40.2,avc1".into()));
        assert_eq!(list[2], ("NAME".into(), "x".into()));
        assert_eq!(hls_codec("mp4a.40.2"), Some("aac"));
        assert_eq!(hls_codec("avc1.4d401f"), None);
    }
}
