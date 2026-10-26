//! Playlist file parsers: M3U/M3U8, PLS, XSPF, CSV/TSV and iTunes/Music XML.
//!
//! These are the no-terms-risk baseline: whatever a person exported from
//! any service or player can be rebuilt from their own library.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;
use serde::Serialize;

use super::{ImportedPlaylist, ImportedTrack, MAX_IMPORT_TRACKS};
use crate::library::matching::SourceTrack;

/// Largest playlist file accepted, in bytes of UTF-8 text.
pub const MAX_PLAYLIST_FILE_BYTES: usize = 5 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaylistFormat {
    M3u,
    Pls,
    Xspf,
    Csv,
    ItunesXml,
}

pub fn detect_format(file_name: &str, content: &str) -> Option<PlaylistFormat> {
    let head: String = content
        .trim_start_matches('\u{feff}')
        .chars()
        .take(4096)
        .collect();
    let head_trimmed = head.trim_start();
    let extension = file_name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    if head_trimmed.starts_with("#EXTM3U") {
        return Some(PlaylistFormat::M3u);
    }
    if head.contains("<playlist") && head.contains("xspf.org") {
        return Some(PlaylistFormat::Xspf);
    }
    if head.contains("<plist") {
        return Some(PlaylistFormat::ItunesXml);
    }
    if head_trimmed.to_ascii_lowercase().starts_with("[playlist]") {
        return Some(PlaylistFormat::Pls);
    }
    match extension.as_str() {
        "m3u" | "m3u8" => Some(PlaylistFormat::M3u),
        "pls" => Some(PlaylistFormat::Pls),
        "xspf" => Some(PlaylistFormat::Xspf),
        "csv" | "tsv" | "txt" => Some(PlaylistFormat::Csv),
        "xml" if head.contains("<playlist") => Some(PlaylistFormat::Xspf),
        _ => None,
    }
}

fn file_stem(file_name: &str) -> String {
    let base = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    base.rsplit_once('.')
        .map_or(base, |(stem, _)| stem)
        .trim()
        .to_string()
}

/// Parses a playlist file into tracks. `file_name` names the playlist when
/// the file itself does not.
pub fn parse_playlist_file(file_name: &str, content: &str) -> Result<ImportedPlaylist, String> {
    if content.len() > MAX_PLAYLIST_FILE_BYTES {
        return Err("Playlist files can be at most 5 MB.".to_string());
    }
    let content = content.trim_start_matches('\u{feff}');
    let format = detect_format(file_name, content).ok_or_else(|| {
        "Unrecognised playlist file. Use M3U, M3U8, PLS, XSPF, CSV, TSV or an iTunes/Music XML export."
            .to_string()
    })?;
    let mut playlist = match format {
        PlaylistFormat::M3u => parse_m3u(content),
        PlaylistFormat::Pls => parse_pls(content),
        PlaylistFormat::Xspf => parse_xspf(content),
        PlaylistFormat::Csv => parse_csv(content)?,
        PlaylistFormat::ItunesXml => parse_itunes_xml(content)?,
    };
    if playlist.name.trim().is_empty() {
        playlist.name = file_stem(file_name);
    }
    if playlist.name.trim().is_empty() {
        playlist.name = "Imported playlist".to_string();
    }
    playlist.tracks.truncate(MAX_IMPORT_TRACKS);
    if playlist.tracks.is_empty() {
        return Err("The file does not contain any tracks.".to_string());
    }
    Ok(playlist)
}

fn leading_track_number() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"^\s*(?:\d{1,2}[-.]?)?\d{1,3}\s*(?:[-.)_]\s*|\s+)").expect("track number regex")
    })
}

/// Splits "Artist - Title" display strings; anything else is a bare title.
pub fn split_display_title(display: &str) -> (Vec<String>, String) {
    let display = display.trim();
    for separator in [" - ", " – ", " — "] {
        if let Some((artist, title)) = display.split_once(separator) {
            let artist = artist.trim();
            let title = title.trim();
            if !artist.is_empty() && !title.is_empty() {
                return (vec![artist.to_string()], title.to_string());
            }
        }
    }
    (Vec::new(), display.to_string())
}

fn track_from_location(location: &str) -> SourceTrack {
    let decoded = percent_encoding::percent_decode_str(location).decode_utf8_lossy();
    let stem = file_stem(&decoded);
    let stem = leading_track_number().replace(&stem, "").to_string();
    let (artists, title) = split_display_title(&stem);
    SourceTrack {
        title,
        artists,
        location: Some(location.to_string()),
        ..SourceTrack::default()
    }
}

fn imported(track: SourceTrack) -> ImportedTrack {
    ImportedTrack {
        track,
        external_id: None,
        external_url: None,
    }
}

fn parse_m3u(content: &str) -> ImportedPlaylist {
    let mut playlist = ImportedPlaylist::default();
    let mut pending_display: Option<(Option<u64>, String)> = None;
    let mut pending_album: Option<String> = None;
    let mut pending_artist: Option<String> = None;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix("#PLAYLIST:") {
            playlist.name = name.trim().to_string();
        } else if let Some(info) = line.strip_prefix("#EXTINF:") {
            let (head, display) = info.split_once(',').unwrap_or((info, ""));
            let seconds = head
                .split_whitespace()
                .next()
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|value| *value > 0.0)
                .map(|value| (value * 1000.0) as u64);
            pending_display = Some((seconds, display.trim().to_string()));
        } else if let Some(album) = line.strip_prefix("#EXTALB:") {
            pending_album = Some(album.trim().to_string());
        } else if let Some(artist) = line.strip_prefix("#EXTART:") {
            pending_artist = Some(artist.trim().to_string());
        } else if line.starts_with('#') {
            continue;
        } else {
            let mut track = track_from_location(line);
            if let Some((duration_ms, display)) = pending_display.take() {
                track.duration_ms = duration_ms;
                if !display.is_empty() {
                    let (artists, title) = split_display_title(&display);
                    track.title = title;
                    if !artists.is_empty() {
                        track.artists = artists;
                    }
                }
            }
            if let Some(artist) = pending_artist.take() {
                track.artists = vec![artist];
            }
            track.album = pending_album.take();
            playlist.tracks.push(imported(track));
            if playlist.tracks.len() >= MAX_IMPORT_TRACKS {
                break;
            }
        }
    }
    playlist
}

/// `FileN`, `TitleN` and `LengthN` (in milliseconds) of one PLS entry.
type PlsEntry = (Option<String>, Option<String>, Option<u64>);

fn parse_pls(content: &str) -> ImportedPlaylist {
    let mut entries: HashMap<u32, PlsEntry> = HashMap::new();
    for line in content.lines() {
        let Some((key, value)) = line.trim().split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim().to_string();
        for (prefix, slot) in [("file", 0), ("title", 1), ("length", 2)] {
            if let Some(index) = key
                .strip_prefix(prefix)
                .and_then(|n| n.parse::<u32>().ok())
                .filter(|n| *n <= MAX_IMPORT_TRACKS as u32)
            {
                let entry = entries.entry(index).or_default();
                match slot {
                    0 => entry.0 = Some(value.clone()),
                    1 => entry.1 = Some(value.clone()),
                    _ => {
                        entry.2 = value
                            .parse::<i64>()
                            .ok()
                            .filter(|seconds| *seconds > 0)
                            .map(|seconds| seconds as u64 * 1000)
                    }
                }
            }
        }
    }
    let mut indexes: Vec<u32> = entries.keys().copied().collect();
    indexes.sort_unstable();
    let tracks = indexes
        .into_iter()
        .filter_map(|index| {
            let (file, title, duration_ms) = entries.remove(&index)?;
            let mut track = match file {
                Some(file) => track_from_location(&file),
                None => SourceTrack::default(),
            };
            if let Some(title) = title.filter(|title| !title.is_empty()) {
                let (artists, title) = split_display_title(&title);
                track.title = title;
                if !artists.is_empty() {
                    track.artists = artists;
                }
            }
            track.duration_ms = duration_ms;
            (!track.title.is_empty()).then(|| imported(track))
        })
        .collect();
    ImportedPlaylist {
        name: String::new(),
        source_url: None,
        tracks,
    }
}

pub(crate) fn decode_xml_entities(value: &str) -> String {
    if !value.contains('&') {
        return value.to_string();
    }
    let mut output = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find('&') {
        output.push_str(&rest[..start]);
        let after = &rest[start..];
        let Some(end) = after.find(';').filter(|end| *end <= 10) else {
            output.push('&');
            rest = &after[1..];
            continue;
        };
        let entity = &after[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                u32::from_str_radix(&entity[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if entity.starts_with('#') => entity[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(character) => {
                output.push(character);
                rest = &after[end + 1..];
            }
            None => {
                output.push('&');
                rest = &after[1..];
            }
        }
    }
    output.push_str(rest);
    output
}

fn xml_tag_text(block: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut search_from = 0;
    while let Some(relative) = block[search_from..].find(&open) {
        let start = search_from + relative;
        let after_name = &block[start + open.len()..];
        // Reject longer tag names such as <titleSort>.
        if !after_name.starts_with('>') && !after_name.starts_with(char::is_whitespace) {
            search_from = start + open.len();
            continue;
        }
        let content_start = start + open.len() + after_name.find('>')? + 1;
        let content_end = content_start + block[content_start..].find(&close)?;
        let raw = block[content_start..content_end].trim();
        let raw = raw
            .strip_prefix("<![CDATA[")
            .and_then(|value| value.strip_suffix("]]>"))
            .unwrap_or(raw);
        let text = decode_xml_entities(raw).trim().to_string();
        return (!text.is_empty()).then_some(text);
    }
    None
}

fn parse_xspf(content: &str) -> ImportedPlaylist {
    static TRACK: OnceLock<Regex> = OnceLock::new();
    let track_pattern =
        TRACK.get_or_init(|| Regex::new(r"(?s)<track\b[^>]*>(.*?)</track>").expect("xspf regex"));
    let header = content.split("<trackList").next().unwrap_or_default();
    let name = xml_tag_text(header, "title").unwrap_or_default();
    let tracks = track_pattern
        .captures_iter(content)
        .take(MAX_IMPORT_TRACKS)
        .filter_map(|captures| {
            let block = captures.get(1)?.as_str();
            let location = xml_tag_text(block, "location");
            let mut track = match location.as_deref() {
                Some(location) => track_from_location(location),
                None => SourceTrack::default(),
            };
            if let Some(title) = xml_tag_text(block, "title") {
                track.title = title;
            }
            if let Some(creator) = xml_tag_text(block, "creator") {
                track.artists = vec![creator];
            }
            track.album = xml_tag_text(block, "album");
            track.duration_ms = xml_tag_text(block, "duration")
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|ms| *ms > 0);
            let identifier = xml_tag_text(block, "identifier");
            if let Some(isrc) = identifier
                .as_deref()
                .and_then(|value| value.strip_prefix("isrc:"))
            {
                track.isrc = Some(isrc.to_string());
            }
            (!track.title.is_empty() || track.location.is_some()).then(|| imported(track))
        })
        .collect();
    ImportedPlaylist {
        name,
        source_url: None,
        tracks,
    }
}

/// Parses delimited text (RFC 4180 quoting) into rows.
fn parse_delimited(content: &str, delimiter: char) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = content.chars().peekable();
    while let Some(character) = chars.next() {
        if in_quotes {
            if character == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(character);
            }
        } else if character == '"' && field.is_empty() {
            in_quotes = true;
        } else if character == delimiter {
            row.push(std::mem::take(&mut field));
        } else if character == '\n' || character == '\r' {
            if character == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut field));
            if row.iter().any(|value| !value.trim().is_empty()) {
                rows.push(std::mem::take(&mut row));
                if rows.len() > MAX_IMPORT_TRACKS {
                    return rows;
                }
            } else {
                row.clear();
            }
        } else {
            field.push(character);
        }
    }
    row.push(field);
    if row.iter().any(|value| !value.trim().is_empty()) {
        rows.push(row);
    }
    rows
}

fn sniff_delimiter(header: &str) -> char {
    [('\t', 0), (',', 0), (';', 0), ('|', 0)]
        .into_iter()
        .map(|(delimiter, _)| (delimiter, header.matches(delimiter).count()))
        .max_by_key(|(_, count)| *count)
        .filter(|(_, count)| *count > 0)
        .map_or(',', |(delimiter, _)| delimiter)
}

const TITLE_HEADERS: &[&str] = &[
    "track name",
    "title",
    "track title",
    "song name",
    "song",
    "track",
    "name",
];
const ARTIST_HEADERS: &[&str] = &[
    "artist name(s)",
    "artist name",
    "artist",
    "artists",
    "album artist",
    "creator",
];
const ALBUM_HEADERS: &[&str] = &["album name", "album", "album title", "release"];
const ISRC_HEADERS: &[&str] = &["isrc", "track isrc"];
const DURATION_HEADERS: &[&str] = &[
    "duration (ms)",
    "duration_ms",
    "track duration (ms)",
    "duration",
    "length",
    "time",
];
const ID_HEADERS: &[&str] = &[
    "track uri",
    "spotify uri",
    "spotify - id",
    "track id",
    "uri",
];
const LOCATION_HEADERS: &[&str] = &["location", "path", "file"];

fn column(headers: &[String], names: &[&str]) -> Option<usize> {
    names
        .iter()
        .find_map(|name| headers.iter().position(|header| header == name))
}

/// Durations as `ms`, `m:ss`, `h:mm:ss` or seconds.
fn parse_duration_ms(value: &str, header_is_ms: bool) -> Option<u64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if value.contains(':') {
        let mut total = 0_u64;
        for part in value.split(':') {
            total = total * 60 + part.trim().parse::<f64>().ok()? as u64;
        }
        return Some(total * 1000).filter(|ms| *ms > 0);
    }
    let number = value.parse::<f64>().ok().filter(|n| *n > 0.0)?;
    // Without a unit, values above ~3 hours in seconds are milliseconds.
    if header_is_ms || number > 10_000.0 {
        Some(number as u64)
    } else {
        Some((number * 1000.0) as u64)
    }
}

fn parse_csv(content: &str) -> Result<ImportedPlaylist, String> {
    let header_line = content.lines().next().unwrap_or_default();
    let rows = parse_delimited(content, sniff_delimiter(header_line));
    let Some((header, records)) = rows.split_first() else {
        return Err("The file is empty.".to_string());
    };
    let headers: Vec<String> = header
        .iter()
        .map(|value| value.trim().trim_matches('\u{feff}').to_ascii_lowercase())
        .collect();
    let title_column = column(&headers, TITLE_HEADERS)
        .ok_or_else(|| "The CSV needs a column named Title, Track Name or Name.".to_string())?;
    let artist_column = column(&headers, ARTIST_HEADERS);
    let album_column = column(&headers, ALBUM_HEADERS);
    let isrc_column = column(&headers, ISRC_HEADERS);
    let duration_column = column(&headers, DURATION_HEADERS);
    let duration_is_ms = duration_column.is_some_and(|index| headers[index].contains("ms"));
    let id_column = column(&headers, ID_HEADERS);
    let location_column = column(&headers, LOCATION_HEADERS);
    let field = |record: &Vec<String>, index: Option<usize>| -> Option<String> {
        index
            .and_then(|index| record.get(index))
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let tracks = records
        .iter()
        .take(MAX_IMPORT_TRACKS)
        .filter_map(|record| {
            let title = field(record, Some(title_column))?;
            let artists = field(record, artist_column)
                .map(|value| {
                    // Exportify joins artists with commas; matching splits further.
                    value
                        .split(';')
                        .map(|name| name.trim().to_string())
                        .filter(|name| !name.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            let external_id = field(record, id_column);
            let external_url = external_id
                .as_deref()
                .and_then(|id| id.strip_prefix("spotify:track:"))
                .map(|id| format!("https://open.spotify.com/track/{id}"));
            Some(ImportedTrack {
                track: SourceTrack {
                    title,
                    artists,
                    album: field(record, album_column),
                    duration_ms: field(record, duration_column)
                        .and_then(|value| parse_duration_ms(&value, duration_is_ms)),
                    isrc: field(record, isrc_column),
                    location: field(record, location_column),
                },
                external_id,
                external_url,
            })
        })
        .collect();
    Ok(ImportedPlaylist {
        name: String::new(),
        source_url: None,
        tracks,
    })
}

/// A minimal property-list value, enough for iTunes/Music XML exports.
#[derive(Debug, Clone, PartialEq)]
enum PlistValue {
    Dict(Vec<(String, PlistValue)>),
    Array(Vec<PlistValue>),
    Text(String),
    Integer(i64),
    Bool(bool),
}

impl PlistValue {
    fn get(&self, key: &str) -> Option<&PlistValue> {
        match self {
            PlistValue::Dict(entries) => entries
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    fn text(&self, key: &str) -> Option<String> {
        match self.get(key)? {
            PlistValue::Text(value) => Some(value.clone()),
            PlistValue::Integer(value) => Some(value.to_string()),
            _ => None,
        }
    }

    fn integer(&self, key: &str) -> Option<i64> {
        match self.get(key)? {
            PlistValue::Integer(value) => Some(*value),
            PlistValue::Text(value) => value.parse().ok(),
            _ => None,
        }
    }

    fn flag(&self, key: &str) -> bool {
        matches!(self.get(key), Some(PlistValue::Bool(true)))
    }
}

struct PlistParser<'a> {
    input: &'a str,
    position: usize,
    depth: usize,
}

impl<'a> PlistParser<'a> {
    /// Returns the next tag (without brackets) and advances past it.
    fn next_tag(&mut self) -> Option<&'a str> {
        loop {
            let start = self.position + self.input[self.position..].find('<')?;
            let end = start + self.input[start..].find('>')?;
            self.position = end + 1;
            let tag = &self.input[start + 1..end];
            if tag.starts_with('?') || tag.starts_with('!') {
                continue;
            }
            return Some(tag);
        }
    }

    fn text_until(&mut self, close: &str) -> Option<String> {
        let end = self.position + self.input[self.position..].find(close)?;
        let text = decode_xml_entities(&self.input[self.position..end]);
        self.position = end + close.len();
        Some(text)
    }

    fn value_from(&mut self, tag: &str) -> Option<PlistValue> {
        self.depth += 1;
        if self.depth > 64 {
            return None;
        }
        let value = match tag.trim_end_matches('/').trim() {
            "dict" if tag.ends_with('/') => PlistValue::Dict(Vec::new()),
            "array" if tag.ends_with('/') => PlistValue::Array(Vec::new()),
            "dict" => {
                let mut entries = Vec::new();
                loop {
                    let next = self.next_tag()?;
                    if next == "/dict" {
                        break;
                    }
                    if next != "key" {
                        return None;
                    }
                    let key = self.text_until("</key>")?;
                    let value_tag = self.next_tag()?;
                    entries.push((key, self.value_from(value_tag)?));
                }
                PlistValue::Dict(entries)
            }
            "array" => {
                let mut items = Vec::new();
                loop {
                    let next = self.next_tag()?;
                    if next == "/array" {
                        break;
                    }
                    items.push(self.value_from(next)?);
                }
                PlistValue::Array(items)
            }
            "true" => PlistValue::Bool(true),
            "false" => PlistValue::Bool(false),
            "string" if tag.ends_with('/') => PlistValue::Text(String::new()),
            "integer" => PlistValue::Integer(
                self.text_until("</integer>")?
                    .trim()
                    .parse()
                    .unwrap_or_default(),
            ),
            name @ ("string" | "real" | "date" | "data") => {
                PlistValue::Text(self.text_until(&format!("</{name}>"))?)
            }
            _ => return None,
        };
        self.depth -= 1;
        Some(value)
    }

    fn parse(input: &'a str) -> Option<PlistValue> {
        let mut parser = PlistParser {
            input,
            position: 0,
            depth: 0,
        };
        loop {
            let tag = parser.next_tag()?;
            if tag.starts_with("plist") {
                let root = parser.next_tag()?;
                return parser.value_from(root);
            }
        }
    }
}

fn parse_itunes_xml(content: &str) -> Result<ImportedPlaylist, String> {
    let root = PlistParser::parse(content)
        .ok_or_else(|| "The XML file is not a readable iTunes/Music export.".to_string())?;
    let Some(PlistValue::Dict(track_entries)) = root.get("Tracks") else {
        return Err("The XML file has no Tracks section.".to_string());
    };
    let tracks_by_id: HashMap<i64, &PlistValue> = track_entries
        .iter()
        .filter_map(|(key, value)| Some((key.parse().ok()?, value)))
        .collect();
    let to_track = |value: &PlistValue| -> Option<ImportedTrack> {
        let title = value.text("Name")?;
        Some(imported(SourceTrack {
            title,
            artists: value.text("Artist").into_iter().collect(),
            album: value.text("Album"),
            duration_ms: value
                .integer("Total Time")
                .filter(|ms| *ms > 0)
                .map(|ms| ms as u64),
            isrc: None,
            location: value.text("Location"),
        }))
    };
    let chosen = match root.get("Playlists") {
        Some(PlistValue::Array(playlists)) => playlists.iter().find(|playlist| {
            !playlist.flag("Master")
                && !playlist.flag("Folder")
                && playlist.get("Distinguished Kind").is_none()
                && playlist.get("Playlist Items").is_some()
        }),
        _ => None,
    };
    if let Some(playlist) = chosen {
        let tracks = match playlist.get("Playlist Items") {
            Some(PlistValue::Array(items)) => items
                .iter()
                .filter_map(|item| tracks_by_id.get(&item.integer("Track ID")?))
                .filter_map(|track| to_track(track))
                .take(MAX_IMPORT_TRACKS)
                .collect(),
            _ => Vec::new(),
        };
        return Ok(ImportedPlaylist {
            name: playlist.text("Name").unwrap_or_default(),
            source_url: None,
            tracks,
        });
    }
    Ok(ImportedPlaylist {
        name: String::new(),
        source_url: None,
        tracks: track_entries
            .iter()
            .filter_map(|(_, value)| to_track(value))
            .take(MAX_IMPORT_TRACKS)
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(playlist: &ImportedPlaylist) -> Vec<(&str, Vec<&str>)> {
        playlist
            .tracks
            .iter()
            .map(|track| {
                (
                    track.track.title.as_str(),
                    track.track.artists.iter().map(String::as_str).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn extended_m3u_reads_names_durations_and_paths() {
        let playlist = parse_playlist_file(
            "road trip.m3u8",
            "\u{feff}#EXTM3U\n#PLAYLIST:Road Trip\n#EXTINF:185,The Beatles - Here Comes the Sun\n\
             #EXTALB:Abbey Road\n/music/Abbey Road/07 Here Comes the Sun.flac\n\n\
             # a comment\nC:\\Music\\Queen\\01 - Bohemian Rhapsody.mp3\n",
        )
        .unwrap();
        assert_eq!(playlist.name, "Road Trip");
        assert_eq!(
            titles(&playlist),
            vec![
                ("Here Comes the Sun", vec!["The Beatles"]),
                ("Bohemian Rhapsody", vec![]),
            ]
        );
        let first = &playlist.tracks[0].track;
        assert_eq!(first.duration_ms, Some(185_000));
        assert_eq!(first.album.as_deref(), Some("Abbey Road"));
        assert_eq!(
            first.location.as_deref(),
            Some("/music/Abbey Road/07 Here Comes the Sun.flac")
        );
    }

    #[test]
    fn plain_m3u_falls_back_to_file_names() {
        let playlist =
            parse_playlist_file("mix.m3u", "Artist A - Song One.mp3\n02. Song Two.flac\n").unwrap();
        assert_eq!(playlist.name, "mix");
        assert_eq!(
            titles(&playlist),
            vec![("Song One", vec!["Artist A"]), ("Song Two", vec![])]
        );
    }

    #[test]
    fn pls_entries_are_ordered_by_index() {
        let playlist = parse_playlist_file(
            "x.pls",
            "[playlist]\nFile2=/b.mp3\nTitle2=Band - Second\nFile1=/a.mp3\nTitle1=First\nLength1=61\nNumberOfEntries=2\n",
        )
        .unwrap();
        assert_eq!(
            titles(&playlist),
            vec![("First", vec![]), ("Second", vec!["Band"])]
        );
        assert_eq!(playlist.tracks[0].track.duration_ms, Some(61_000));
    }

    #[test]
    fn xspf_reads_creator_album_duration_and_entities() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<playlist version="1" xmlns="http://xspf.org/ns/0/">
  <title>Late &amp; Night</title>
  <trackList>
    <track>
      <location>file:///music/Massive%20Attack/Mezzanine/01%20Angel.flac</location>
      <title>Angel</title><creator>Massive Attack</creator>
      <album>Mezzanine</album><duration>379000</duration>
    </track>
    <track><title><![CDATA[Tom's Diner]]></title><creator>Suzanne Vega</creator></track>
  </trackList>
</playlist>"#;
        let playlist = parse_playlist_file("late.xspf", xml).unwrap();
        assert_eq!(playlist.name, "Late & Night");
        assert_eq!(
            titles(&playlist),
            vec![
                ("Angel", vec!["Massive Attack"]),
                ("Tom's Diner", vec!["Suzanne Vega"])
            ]
        );
        assert_eq!(playlist.tracks[0].track.duration_ms, Some(379_000));
        assert_eq!(playlist.tracks[0].track.album.as_deref(), Some("Mezzanine"));
    }

    #[test]
    fn exportify_csv_keeps_isrc_and_spotify_ids() {
        let csv = "\"Track URI\",\"Track Name\",\"Artist Name(s)\",\"Album Name\",\"Duration (ms)\",\"ISRC\"\n\
                   \"spotify:track:6rqhFgbbKwnb9MLmUQDhG6\",\"Speak to Me - 2011 Remaster\",\"Pink Floyd\",\"The Dark Side of the Moon\",\"67960\",\"GBN9Y1100088\"\n\
                   \"spotify:track:x\",\"Quote \"\"Inside\"\"\",\"A, B\",\"\",\"\",\"\"\n";
        let playlist = parse_playlist_file("Dark Side.csv", csv).unwrap();
        assert_eq!(playlist.name, "Dark Side");
        let first = &playlist.tracks[0];
        assert_eq!(first.track.title, "Speak to Me - 2011 Remaster");
        assert_eq!(first.track.isrc.as_deref(), Some("GBN9Y1100088"));
        assert_eq!(first.track.duration_ms, Some(67_960));
        assert_eq!(
            first.external_url.as_deref(),
            Some("https://open.spotify.com/track/6rqhFgbbKwnb9MLmUQDhG6")
        );
        assert_eq!(playlist.tracks[1].track.title, "Quote \"Inside\"");
        assert_eq!(playlist.tracks[1].track.artists, vec!["A, B"]);
    }

    #[test]
    fn apple_music_text_export_is_tab_separated_with_seconds() {
        let text = "Name\tArtist\tComposer\tAlbum\tTime\tLocation\n\
                    Clair de Lune\tClaude Debussy\t\tSuite\t300\t/Music/Debussy/Clair.m4a\n";
        let playlist = parse_playlist_file("Classical.txt", text).unwrap();
        let track = &playlist.tracks[0].track;
        assert_eq!(track.title, "Clair de Lune");
        assert_eq!(track.duration_ms, Some(300_000));
        assert_eq!(track.location.as_deref(), Some("/Music/Debussy/Clair.m4a"));
    }

    #[test]
    fn durations_accept_clock_and_unitless_values() {
        assert_eq!(parse_duration_ms("3:05", false), Some(185_000));
        assert_eq!(parse_duration_ms("1:00:01", false), Some(3_601_000));
        assert_eq!(parse_duration_ms("185", false), Some(185_000));
        assert_eq!(parse_duration_ms("185000", false), Some(185_000));
        assert_eq!(parse_duration_ms("", false), None);
        assert_eq!(parse_duration_ms("abc", false), None);
    }

    #[test]
    fn itunes_xml_imports_the_exported_playlist_in_order() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Major Version</key><integer>1</integer>
  <key>Tracks</key><dict>
    <key>10</key><dict><key>Track ID</key><integer>10</integer><key>Name</key><string>Hey Jude</string>
      <key>Artist</key><string>The Beatles</string><key>Total Time</key><integer>431000</integer>
      <key>Explicit</key><true/></dict>
    <key>20</key><dict><key>Track ID</key><integer>20</integer><key>Name</key><string>R&amp;B Song</string></dict>
  </dict>
  <key>Playlists</key><array>
    <dict><key>Name</key><string>Library</string><key>Master</key><true/>
      <key>Playlist Items</key><array><dict><key>Track ID</key><integer>10</integer></dict></array></dict>
    <dict><key>Name</key><string>Favourites</string>
      <key>Playlist Items</key><array>
        <dict><key>Track ID</key><integer>20</integer></dict>
        <dict><key>Track ID</key><integer>10</integer></dict>
      </array></dict>
  </array>
</dict></plist>"#;
        let playlist = parse_playlist_file("Favourites.xml", xml).unwrap();
        assert_eq!(playlist.name, "Favourites");
        assert_eq!(
            titles(&playlist),
            vec![("R&B Song", vec![]), ("Hey Jude", vec!["The Beatles"])]
        );
        assert_eq!(playlist.tracks[1].track.duration_ms, Some(431_000));
    }

    #[test]
    fn unknown_empty_and_oversized_files_are_rejected() {
        assert!(parse_playlist_file("song.mp3", "ID3").is_err());
        assert!(parse_playlist_file("empty.m3u", "#EXTM3U\n").is_err());
        assert!(parse_playlist_file("bad.csv", "foo,bar\n1,2\n").is_err());
        let large = "a".repeat(MAX_PLAYLIST_FILE_BYTES + 1);
        assert!(parse_playlist_file("big.m3u", &large).is_err());
        assert!(parse_playlist_file("x.xml", "<plist><dict><key>").is_err());
    }

    #[test]
    fn deeply_nested_plists_do_not_overflow() {
        let nested = format!(
            "<plist>{}{}</plist>",
            "<array>".repeat(10_000),
            "</array>".repeat(10_000)
        );
        assert!(parse_playlist_file("deep.xml", &nested).is_err());
    }
}
