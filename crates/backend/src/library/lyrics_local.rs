//! Lyrics carried by the music itself, read before anything goes online:
//!
//! 1. an LRC file beside the track (`Song.lrc` or `Song.flac.lrc`);
//! 2. an ID3v2 `SYLT` frame (synchronised lyrics, millisecond timestamps);
//! 3. lyrics tags: ID3v2 `USLT`, Vorbis `LYRICS` / `SYNCEDLYRICS` /
//!    `UNSYNCEDLYRICS`, MP4 `©lyr` and APE `Lyrics`. Taggers such as
//!    foobar2000, Mp3tag and MusicBrainz Picard often store LRC text in these,
//!    so a tag whose lines carry `[mm:ss.xx]` stamps counts as timed lyrics.
//!
//! Timed lyrics from any of these win over plain text from any of them.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use lofty::config::ParseOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::flac::FlacFile;
use lofty::probe::Probe;
use lofty::tag::ItemKey;
use serde::Serialize;

/// LRC files and lyrics tags larger than this are ignored.
pub const MAX_LYRICS_BYTES: u64 = 512 * 1024;
/// More timed lines than this are dropped (a full opera libretto has fewer).
const MAX_LRC_LINES: usize = 5_000;

/// Where a track's lyrics came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LyricsSource {
    /// An `.lrc` file next to the audio file.
    SidecarLrc,
    /// An ID3v2 `SYLT` (synchronised lyrics) frame.
    EmbeddedSylt,
    /// LRC text inside a lyrics tag (USLT, Vorbis LYRICS, MP4 ©lyr, ...).
    EmbeddedLrc,
    /// Untimed text inside a lyrics tag.
    EmbeddedText,
    /// LRCLIB, looked up online.
    Lrclib,
}

impl LyricsSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SidecarLrc => "sidecar_lrc",
            Self::EmbeddedSylt => "embedded_sylt",
            Self::EmbeddedLrc => "embedded_lrc",
            Self::EmbeddedText => "embedded_text",
            Self::Lrclib => "lrclib",
        }
    }
}

/// One timed line of LRC.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LrcLine {
    pub time_ms: u64,
    pub text: String,
}

/// Lyrics found in or beside a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalLyrics {
    pub source: LyricsSource,
    /// Normalised LRC (`[mm:ss.xx] text`, sorted), when timed.
    pub synced: Option<String>,
    /// Plain text: the untimed lyrics, or the timed lines without stamps.
    pub plain: Option<String>,
}

impl LocalLyrics {
    pub fn is_synced(&self) -> bool {
        self.synced.is_some()
    }

    fn timed(source: LyricsSource, lines: &[LrcLine]) -> Self {
        Self {
            source,
            synced: Some(format_lrc(lines)),
            plain: Some(plain_text(lines)).filter(|text| !text.is_empty()),
        }
    }
}

/// Parses `mm:ss`, `mm:ss.x`, `mm:ss.xx`, `mm:ss.xxx`, `mm:ss:xx` and
/// `h:mm:ss.xx` into milliseconds.
fn parse_stamp(stamp: &str) -> Option<u64> {
    let stamp = stamp.trim();
    if stamp.is_empty() || !stamp.as_bytes()[0].is_ascii_digit() {
        return None;
    }
    let parts = stamp.split(':').collect::<Vec<_>>();
    let (hours, minutes, rest) = match parts.as_slice() {
        [minutes, rest] => ("0", *minutes, *rest),
        // `mm:ss:cc`, the common variant with centiseconds after a colon.
        [minutes, seconds, hundredths]
            if hundredths.len() == 2 && !hundredths.contains('.') && seconds.len() == 2 =>
        {
            let minutes = minutes.parse::<u64>().ok()?;
            let seconds = seconds.parse::<u64>().ok()?;
            let hundredths = hundredths.parse::<u64>().ok()?;
            if seconds >= 60 {
                return None;
            }
            return Some(minutes * 60_000 + seconds * 1_000 + hundredths * 10);
        }
        // `h:mm:ss.xx`.
        [hours, minutes, rest] => (*hours, *minutes, *rest),
        _ => return None,
    };
    let hours = hours.parse::<u64>().ok()?;
    let minutes = minutes.parse::<u64>().ok()?;
    let (seconds, fraction) = rest.split_once('.').unwrap_or((rest, ""));
    if seconds.is_empty() || seconds.len() > 2 || fraction.len() > 3 {
        return None;
    }
    let seconds = seconds.parse::<u64>().ok()?;
    if seconds >= 60 || (parts.len() == 3 && minutes >= 60) {
        return None;
    }
    let fraction_ms = if fraction.is_empty() {
        0
    } else {
        let digits = fraction.parse::<u64>().ok()?;
        match fraction.len() {
            1 => digits * 100,
            2 => digits * 10,
            _ => digits,
        }
    };
    Some(hours * 3_600_000 + minutes * 60_000 + seconds * 1_000 + fraction_ms)
}

/// Removes enhanced-LRC word stamps (`<00:12.34>`) from a line's text.
fn strip_word_stamps(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        let Some(end) = rest[start..].find('>') else {
            break;
        };
        let inner = &rest[start + 1..start + end];
        if parse_stamp(inner).is_some() {
            out.push_str(&rest[..start]);
            rest = &rest[start + end + 1..];
        } else {
            out.push_str(&rest[..=start]);
            rest = &rest[start + 1..];
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Parses LRC text into timed lines, sorted by time.
///
/// Handles several stamps on one line (`[00:12.00][01:30.00]Chorus`),
/// `[offset:±ms]`, metadata tags (`[ar:]`, `[ti:]`, ... are skipped),
/// enhanced-LRC word stamps and blank timed lines (instrumental breaks).
pub fn parse_lrc(text: &str) -> Vec<LrcLine> {
    let mut offset_ms: i64 = 0;
    let mut lines = Vec::new();
    for raw in text.lines() {
        let mut rest = raw.trim_start_matches('\u{feff}').trim();
        let mut stamps = Vec::new();
        while let Some(after) = rest.strip_prefix('[') {
            let Some(close) = after.find(']') else {
                break;
            };
            let tag = &after[..close];
            if let Some(stamp) = parse_stamp(tag) {
                stamps.push(stamp);
            } else if let Some((key, value)) = tag.split_once(':') {
                if key.trim().eq_ignore_ascii_case("offset") {
                    offset_ms = value.trim().trim_start_matches('+').parse().unwrap_or(0);
                }
            } else {
                break;
            }
            rest = after[close + 1..].trim_start();
        }
        if stamps.is_empty() {
            continue;
        }
        let text = strip_word_stamps(rest);
        for stamp in stamps {
            // A positive offset makes lyrics appear sooner.
            let time = (stamp as i64 - offset_ms).max(0) as u64;
            lines.push(LrcLine {
                time_ms: time,
                text: text.clone(),
            });
            if lines.len() >= MAX_LRC_LINES {
                break;
            }
        }
        if lines.len() >= MAX_LRC_LINES {
            break;
        }
    }
    lines.sort_by_key(|line| line.time_ms);
    lines
}

/// Writes timed lines back as normalised LRC: `[mm:ss.xx] text`.
pub fn format_lrc(lines: &[LrcLine]) -> String {
    lines
        .iter()
        .map(|line| {
            let minutes = line.time_ms / 60_000;
            let seconds = (line.time_ms % 60_000) / 1_000;
            let hundredths = (line.time_ms % 1_000) / 10;
            if line.text.is_empty() {
                format!("[{minutes:02}:{seconds:02}.{hundredths:02}]")
            } else {
                format!("[{minutes:02}:{seconds:02}.{hundredths:02}] {}", line.text)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The words of timed lines, one per line, without stamps or blank breaks.
pub fn plain_text(lines: &[LrcLine]) -> String {
    lines
        .iter()
        .map(|line| line.text.as_str())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Reads a tag's lyrics text: timed when most of its words carry stamps.
fn from_tag_text(text: &str, timed_source: LyricsSource) -> Option<LocalLyrics> {
    let text = text.trim_matches(|character: char| character == '\u{feff}' || character == '\0');
    if text.trim().is_empty() || text.len() as u64 > MAX_LYRICS_BYTES {
        return None;
    }
    let timed = parse_lrc(text);
    let worded = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !is_lrc_metadata_line(line))
        .count();
    let timed_words = timed.iter().filter(|line| !line.text.is_empty()).count();
    if timed_words > 0 && timed_words * 2 >= worded.max(1) {
        return Some(LocalLyrics::timed(timed_source, &timed));
    }
    Some(LocalLyrics {
        source: if timed_source == LyricsSource::SidecarLrc {
            LyricsSource::SidecarLrc
        } else {
            LyricsSource::EmbeddedText
        },
        synced: None,
        plain: Some(text.trim().replace("\r\n", "\n")),
    })
}

/// `[ar:Artist]`, `[length: 3:20]` and similar header lines.
fn is_lrc_metadata_line(line: &str) -> bool {
    let Some(inner) = line
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    else {
        return false;
    };
    inner
        .split_once(':')
        .is_some_and(|(key, _)| key.chars().all(|character| character.is_ascii_alphabetic()))
}

/// Decodes an LRC file: UTF-8 (with or without BOM), UTF-16 with a BOM, or
/// Latin-1 as a last resort.
fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    let utf16 = |rest: &[u8], little: bool| {
        let units = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                if little {
                    u16::from_le_bytes(*pair)
                } else {
                    u16::from_be_bytes(*pair)
                }
            })
            .collect::<Vec<_>>();
        String::from_utf16_lossy(&units)
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, true);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, false);
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => bytes.iter().map(|byte| char::from(*byte)).collect(),
    }
}

/// LRC files that belong to `path`: `Song.lrc` and `Song.flac.lrc`, in
/// either case.
pub fn sidecar_candidates(path: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for extension in ["lrc", "LRC"] {
        candidates.push(path.with_extension(extension));
    }
    if let Some(name) = path.file_name() {
        let mut with_suffix = name.to_os_string();
        with_suffix.push(".lrc");
        candidates.push(path.with_file_name(with_suffix));
    }
    candidates.dedup();
    candidates
}

fn read_sidecar(path: &Path) -> Option<LocalLyrics> {
    for candidate in sidecar_candidates(path) {
        let Ok(metadata) = std::fs::metadata(&candidate) else {
            continue;
        };
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_LYRICS_BYTES {
            continue;
        }
        let Ok(bytes) = std::fs::read(&candidate) else {
            continue;
        };
        if let Some(lyrics) = from_tag_text(&decode_text(&bytes), LyricsSource::SidecarLrc) {
            return Some(lyrics);
        }
    }
    None
}

fn starts_with_id3(path: &Path) -> bool {
    let mut header = [0_u8; 3];
    File::open(path)
        .and_then(|mut file| {
            file.seek(SeekFrom::Start(0))?;
            file.read_exact(&mut header)
        })
        .is_ok()
        && &header == b"ID3"
}

/// SYLT and USLT frames of an ID3v2 tag (MP3, and AIFF/WAV with ID3 chunks).
fn read_id3(path: &Path) -> Vec<LocalLyrics> {
    let mut found = Vec::new();
    let is_mp3 = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp3"));
    let tag = if is_mp3 || starts_with_id3(path) {
        id3::Tag::read_from_path(path).ok()
    } else {
        None
    };
    let Some(tag) = tag else {
        return found;
    };
    for sylt in tag.synchronised_lyrics() {
        // MPEG-frame stamps need the stream's frame rate; taggers write
        // milliseconds, so only those are read.
        if sylt.timestamp_format != id3::frame::TimestampFormat::Ms {
            continue;
        }
        let mut lines = sylt
            .content
            .iter()
            .take(MAX_LRC_LINES)
            .map(|(time, text)| LrcLine {
                time_ms: u64::from(*time),
                text: text.trim().to_string(),
            })
            .collect::<Vec<_>>();
        lines.sort_by_key(|line| line.time_ms);
        if lines.iter().any(|line| !line.text.is_empty()) {
            found.push(LocalLyrics::timed(LyricsSource::EmbeddedSylt, &lines));
        }
    }
    for uslt in tag.lyrics() {
        if let Some(lyrics) = from_tag_text(&uslt.text, LyricsSource::EmbeddedLrc) {
            found.push(lyrics);
        }
    }
    found
}

/// Lyrics tags read through lofty: Vorbis comments (including the
/// non-standard SYNCEDLYRICS / UNSYNCEDLYRICS keys for FLAC), MP4 and APE.
fn read_tags(path: &Path) -> Vec<LocalLyrics> {
    let options = ParseOptions::new()
        .read_properties(false)
        .read_cover_art(false);
    let mut texts = Vec::new();
    let is_flac = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("flac"));
    if is_flac {
        if let Ok(mut file) = File::open(path)
            && let Ok(flac) = FlacFile::read_from(&mut file, options)
            && let Some(comments) = flac.vorbis_comments()
        {
            for key in ["SYNCEDLYRICS", "LYRICS", "UNSYNCEDLYRICS"] {
                texts.extend(comments.get_all(key).map(str::to_owned));
            }
        }
    } else if let Ok(probe) = Probe::open(path).map(|probe| probe.options(options))
        && let Ok(probe) = probe.guess_file_type()
        && let Ok(tagged) = probe.read()
    {
        for tag in tagged.tags() {
            for key in [ItemKey::Lyrics, ItemKey::UnsyncLyrics] {
                texts.extend(tag.get_strings(key).map(str::to_owned));
            }
        }
    }
    texts
        .iter()
        .filter_map(|text| from_tag_text(text, LyricsSource::EmbeddedLrc))
        .collect()
}

/// Lyrics in or beside the audio file at `path`: timed lyrics first (an LRC
/// file beside it, then SYLT, then LRC text in a tag), else plain text.
pub fn read_local_lyrics(path: &Path) -> Option<LocalLyrics> {
    let mut candidates = Vec::new();
    candidates.extend(read_sidecar(path));
    candidates.extend(read_id3(path));
    candidates.extend(read_tags(path));
    candidates
        .iter()
        .find(|lyrics| lyrics.is_synced())
        .or_else(|| candidates.first())
        .cloned()
}

/// What decides whether cached availability is still right: the audio
/// file's size and modification time, and each sidecar's modification time.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LyricsFingerprint {
    file: Option<(u64, u128)>,
    sidecars: Vec<Option<u128>>,
}

fn modified_nanos(metadata: &std::fs::Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos())
}

pub fn lyrics_fingerprint(path: &Path) -> LyricsFingerprint {
    LyricsFingerprint {
        file: std::fs::metadata(path)
            .ok()
            .map(|metadata| (metadata.len(), modified_nanos(&metadata))),
        sidecars: sidecar_candidates(path)
            .iter()
            .map(|candidate| {
                std::fs::metadata(candidate)
                    .ok()
                    .map(|metadata| modified_nanos(&metadata))
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lofty::config::WriteOptions;
    use lofty::tag::{Accessor, Tag, TagExt, TagType};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    /// A fresh scratch directory under the system temp dir.
    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "mujin-lyrics-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn fixture(name: &str) -> &'static [u8] {
        match name {
            "lrc" => include_bytes!("../../testdata/audio/flac-48k-24bit-embedded-lrc.flac"),
            "flac" => include_bytes!("../../testdata/audio/flac-44k-16bit-mono.flac"),
            "mp3" => include_bytes!("../../testdata/audio/mp3-44k-cbr192-stereo.mp3"),
            "ogg" => include_bytes!("../../testdata/audio/vorbis-44k-stereo.ogg"),
            _ => unreachable!(),
        }
    }

    fn copy_fixture(directory: &Path, fixture_name: &str, file_name: &str) -> PathBuf {
        let path = directory.join(file_name);
        std::fs::write(&path, fixture(fixture_name)).unwrap();
        path
    }

    #[test]
    fn lrc_stamps_cover_every_common_variant() {
        assert_eq!(parse_stamp("01:02.5"), Some(62_500));
        assert_eq!(parse_stamp("01:02.50"), Some(62_500));
        assert_eq!(parse_stamp("01:02.503"), Some(62_503));
        assert_eq!(parse_stamp("01:02"), Some(62_000));
        assert_eq!(parse_stamp("01:02:50"), Some(62_500));
        assert_eq!(parse_stamp("1:02:03.25"), Some(3_723_250));
        assert_eq!(parse_stamp("ar"), None);
        assert_eq!(parse_stamp("00:61.00"), None);
        assert_eq!(parse_stamp(""), None);
    }

    #[test]
    fn lrc_parsing_handles_repeats_offsets_metadata_and_word_stamps() {
        let lines = parse_lrc(
            "\u{feff}[ar:Fixture Artist]\n[ti:Fixture]\n[offset:+500]\n\
             [00:10.00][00:40.00]Chorus line\n\
             [00:05.00]<00:05.00>First <00:06.20>verse\n\
             [00:20.00]\n\
             not a lyric line\n",
        );
        assert_eq!(
            lines,
            vec![
                LrcLine {
                    time_ms: 4_500,
                    text: "First verse".into()
                },
                LrcLine {
                    time_ms: 9_500,
                    text: "Chorus line".into()
                },
                LrcLine {
                    time_ms: 19_500,
                    text: String::new()
                },
                LrcLine {
                    time_ms: 39_500,
                    text: "Chorus line".into()
                },
            ]
        );
        assert_eq!(
            format_lrc(&lines),
            "[00:04.50] First verse\n[00:09.50] Chorus line\n[00:19.50]\n[00:39.50] Chorus line"
        );
        assert_eq!(plain_text(&lines), "First verse\nChorus line\nChorus line");
    }

    #[test]
    fn plain_text_tags_stay_plain_and_mostly_timed_tags_are_timed() {
        let plain =
            from_tag_text("One line\nTwo [bracketed] line", LyricsSource::EmbeddedLrc).unwrap();
        assert_eq!(plain.source, LyricsSource::EmbeddedText);
        assert!(!plain.is_synced());
        let timed = from_tag_text(
            "[ar:Someone]\n[00:01.00]One\n[00:02.00]Two\nstray credit line",
            LyricsSource::EmbeddedLrc,
        )
        .unwrap();
        assert_eq!(timed.source, LyricsSource::EmbeddedLrc);
        assert_eq!(timed.plain.as_deref(), Some("One\nTwo"));
        assert!(from_tag_text("  \n", LyricsSource::EmbeddedLrc).is_none());
    }

    /// The checked-in test track: a 24-bit / 48 kHz FLAC whose Vorbis LYRICS
    /// comment holds an LRC file (see testdata/audio/generate.sh).
    #[test]
    fn the_test_track_with_an_embedded_lrc_file_reads_as_timed_lyrics() {
        let directory = scratch("embedded");
        let path = copy_fixture(&directory, "lrc", "Embedded LRC.flac");
        let lyrics = read_local_lyrics(&path).expect("embedded lyrics");
        assert_eq!(lyrics.source, LyricsSource::EmbeddedLrc);
        let lines = parse_lrc(lyrics.synced.as_deref().unwrap());
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[0].time_ms, 0);
        assert_eq!(lines[0].text, "Every line arrives on time");
        assert_eq!(lines.last().unwrap().text, "Every line arrives on time");
        assert!(
            lines
                .windows(2)
                .all(|pair| pair[0].time_ms <= pair[1].time_ms)
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_sidecar_lrc_file_wins_over_plain_embedded_text() {
        let directory = scratch("sidecar");
        let path = copy_fixture(&directory, "flac", "Song.flac");
        let mut flac = FlacFile::read_from(
            &mut File::open(&path).unwrap(),
            ParseOptions::new().read_properties(false),
        )
        .unwrap();
        let comments = flac.vorbis_comments_mut().unwrap();
        comments.insert("UNSYNCEDLYRICS".into(), "Plain words only".into());
        flac.save_to_path(&path, WriteOptions::default()).unwrap();
        assert_eq!(
            read_local_lyrics(&path).unwrap(),
            LocalLyrics {
                source: LyricsSource::EmbeddedText,
                synced: None,
                plain: Some("Plain words only".into()),
            }
        );

        let before = lyrics_fingerprint(&path);
        // UTF-16 with a BOM, as some Windows tools write it.
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "[00:01.00]Beside the file\r\n[00:03.00]Still beside it".encode_utf16() {
            bytes.extend(unit.to_le_bytes());
        }
        std::fs::write(directory.join("Song.lrc"), bytes).unwrap();
        assert_ne!(lyrics_fingerprint(&path), before);
        let lyrics = read_local_lyrics(&path).unwrap();
        assert_eq!(lyrics.source, LyricsSource::SidecarLrc);
        assert_eq!(
            lyrics.synced.as_deref(),
            Some("[00:01.00] Beside the file\n[00:03.00] Still beside it")
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn id3_sylt_frames_are_timed_and_uslt_lrc_is_read_too() {
        use id3::frame::{Lyrics, SynchronisedLyrics, SynchronisedLyricsType, TimestampFormat};
        use id3::{TagLike, Version};

        let directory = scratch("id3");
        let sylt_path = copy_fixture(&directory, "mp3", "sylt.mp3");
        let mut tag = id3::Tag::new();
        tag.add_frame(SynchronisedLyrics {
            lang: "eng".into(),
            timestamp_format: TimestampFormat::Ms,
            content_type: SynchronisedLyricsType::Lyrics,
            description: String::new(),
            content: vec![(2_500, "Second".into()), (1_000, "First".into())],
        });
        tag.add_frame(Lyrics {
            lang: "eng".into(),
            description: String::new(),
            text: "Plain fallback".into(),
        });
        tag.write_to_path(&sylt_path, Version::Id3v24).unwrap();
        let lyrics = read_local_lyrics(&sylt_path).unwrap();
        assert_eq!(lyrics.source, LyricsSource::EmbeddedSylt);
        assert_eq!(
            lyrics.synced.as_deref(),
            Some("[00:01.00] First\n[00:02.50] Second")
        );

        let uslt_path = copy_fixture(&directory, "mp3", "uslt.mp3");
        let mut tag = id3::Tag::new();
        tag.add_frame(Lyrics {
            lang: "eng".into(),
            description: String::new(),
            text: "[00:00.50]From USLT\n[00:01.50]As LRC".into(),
        });
        tag.write_to_path(&uslt_path, Version::Id3v23).unwrap();
        let lyrics = read_local_lyrics(&uslt_path).unwrap();
        assert_eq!(lyrics.source, LyricsSource::EmbeddedLrc);
        assert_eq!(lyrics.plain.as_deref(), Some("From USLT\nAs LRC"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn ogg_lyrics_comments_are_read_through_lofty() {
        let directory = scratch("ogg");
        let path = copy_fixture(&directory, "ogg", "song.ogg");
        let mut tag = Tag::new(TagType::VorbisComments);
        tag.set_title("Fixture".into());
        tag.insert_text(ItemKey::Lyrics, "[00:00.10]Ogg words".into());
        tag.save_to_path(&path, WriteOptions::default()).unwrap();
        let lyrics = read_local_lyrics(&path).unwrap();
        assert_eq!(lyrics.source, LyricsSource::EmbeddedLrc);
        assert_eq!(lyrics.synced.as_deref(), Some("[00:00.10] Ogg words"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn files_without_lyrics_report_none() {
        let directory = scratch("none");
        let path = copy_fixture(&directory, "flac", "quiet.flac");
        assert_eq!(read_local_lyrics(&path), None);
        assert_eq!(read_local_lyrics(&directory.join("missing.flac")), None);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
