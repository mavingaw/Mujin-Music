//! ICY (SHOUTcast/Icecast) in-stream metadata.
//!
//! When a request carries `Icy-MetaData: 1`, the server answers with an
//! `icy-metaint: N` header and inserts a metadata block after every `N`
//! audio bytes: one length byte (times 16), then that many bytes of
//! `StreamTitle='Artist - Title';StreamUrl='...';` padded with NULs.
//! [`IcyDemuxer`] removes the blocks so players get clean audio, and hands
//! back each title as it changes.

/// Largest `icy-metaint` Parson accepts (real servers use 8-64 KiB).
pub const MAX_METAINT: usize = 1 << 20;

/// One metadata block, decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IcyMetadata {
    /// `StreamTitle` exactly as sent (decoded to text).
    pub stream_title: String,
    pub stream_url: Option<String>,
}

/// A title split into artist and track, when it has the usual
/// `Artist - Title` shape.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct TrackTitle {
    pub raw: String,
    pub artist: Option<String>,
    pub title: String,
}

/// Decodes metadata text: UTF-8 when valid, otherwise Latin-1 (what most
/// SHOUTcast v1 servers send).
pub fn decode_text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => bytes.iter().map(|&byte| byte as char).collect(),
    }
}

fn field(text: &str, name: &str) -> Option<String> {
    let start = text.find(&format!("{name}='"))? + name.len() + 2;
    let rest = &text[start..];
    // Titles may contain apostrophes ("Guns N' Roses"), so the value ends
    // at the first `';` (or the last `'` when the block is truncated).
    let end = rest.find("';").or_else(|| rest.rfind('\''))?;
    Some(rest[..end].to_string())
}

/// Parses one metadata block's text.
pub fn parse_block(bytes: &[u8]) -> Option<IcyMetadata> {
    let end = bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(bytes.len());
    let text = decode_text(&bytes[..end]);
    let stream_title = field(&text, "StreamTitle")?;
    Some(IcyMetadata {
        stream_title: stream_title.trim().to_string(),
        stream_url: field(&text, "StreamUrl").filter(|url| !url.trim().is_empty()),
    })
}

/// Splits `Artist - Title`. Empty and placeholder titles return `None`.
pub fn split_title(raw: &str, station_name: Option<&str>) -> Option<TrackTitle> {
    let raw = raw.trim();
    // "Europe1" for "Europe 1": compare letters and digits only.
    let simple = |value: &str| -> String {
        value
            .chars()
            .filter(|character| character.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    let placeholder = raw.is_empty()
        || raw == "-"
        || raw.eq_ignore_ascii_case("unknown")
        || raw.eq_ignore_ascii_case("advertisement")
        || station_name.is_some_and(|name| simple(raw) == simple(name));
    if placeholder {
        return None;
    }
    if let Some((artist, title)) = raw.split_once(" - ") {
        let (artist, title) = (artist.trim(), title.trim());
        if !artist.is_empty() && !title.is_empty() {
            return Some(TrackTitle {
                raw: raw.to_string(),
                artist: Some(artist.to_string()),
                title: title.to_string(),
            });
        }
    }
    Some(TrackTitle {
        raw: raw.to_string(),
        artist: None,
        title: raw.to_string(),
    })
}

enum State {
    /// Audio bytes left before the next length byte.
    Audio(usize),
    /// Metadata bytes left to collect.
    Metadata(usize),
}

/// Strips ICY metadata blocks from a stream, chunk by chunk.
pub struct IcyDemuxer {
    metaint: usize,
    state: State,
    block: Vec<u8>,
    last_title: Option<String>,
}

impl IcyDemuxer {
    pub fn new(metaint: usize) -> Option<Self> {
        (1..=MAX_METAINT).contains(&metaint).then(|| Self {
            metaint,
            state: State::Audio(metaint),
            block: Vec::new(),
            last_title: None,
        })
    }

    /// Appends the audio in `chunk` to `audio` and returns metadata blocks
    /// whose title changed.
    pub fn feed(&mut self, mut chunk: &[u8], audio: &mut Vec<u8>) -> Vec<IcyMetadata> {
        let mut changes = Vec::new();
        while !chunk.is_empty() {
            match self.state {
                State::Audio(left) => {
                    let take = left.min(chunk.len());
                    audio.extend_from_slice(&chunk[..take]);
                    chunk = &chunk[take..];
                    self.state = if take == left {
                        State::Metadata(usize::MAX)
                    } else {
                        State::Audio(left - take)
                    };
                }
                State::Metadata(usize::MAX) => {
                    let length = usize::from(chunk[0]) * 16;
                    chunk = &chunk[1..];
                    self.block.clear();
                    self.state = if length == 0 {
                        State::Audio(self.metaint)
                    } else {
                        State::Metadata(length)
                    };
                }
                State::Metadata(left) => {
                    let take = left.min(chunk.len());
                    self.block.extend_from_slice(&chunk[..take]);
                    chunk = &chunk[take..];
                    if take == left {
                        if let Some(metadata) = parse_block(&self.block)
                            && self.last_title.as_deref() != Some(metadata.stream_title.as_str())
                        {
                            self.last_title = Some(metadata.stream_title.clone());
                            changes.push(metadata);
                        }
                        self.block.clear();
                        self.state = State::Audio(self.metaint);
                    } else {
                        self.state = State::Metadata(left - take);
                    }
                }
            }
        }
        changes
    }
}

/// Format hints from `icy-*` response headers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IcyHeaders {
    pub name: Option<String>,
    pub genre: Option<String>,
    pub bitrate_kbps: Option<u32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
}

impl IcyHeaders {
    pub fn read(header: impl Fn(&str) -> Option<String>) -> Self {
        let mut headers = Self {
            name: header("icy-name").filter(|value| !value.trim().is_empty()),
            genre: header("icy-genre").filter(|value| !value.trim().is_empty()),
            bitrate_kbps: header("icy-br")
                .and_then(|value| value.split(',').next()?.trim().parse().ok())
                .filter(|&rate: &u32| (8..=10_000).contains(&rate)),
            sample_rate: header("icy-sr").and_then(|value| value.trim().parse().ok()),
            channels: None,
        };
        // `ice-samplerate=44100;ice-bitrate=128;ice-channels=2`
        if let Some(info) = header("icy-audio-info").or_else(|| header("ice-audio-info")) {
            for pair in info.split(';') {
                let Some((key, value)) = pair.split_once('=') else {
                    continue;
                };
                let value = value.trim();
                match key.trim().to_ascii_lowercase().as_str() {
                    "ice-samplerate" | "samplerate" => {
                        headers.sample_rate = headers.sample_rate.or(value.parse().ok())
                    }
                    "ice-bitrate" | "bitrate" => {
                        headers.bitrate_kbps = headers.bitrate_kbps.or(value.parse().ok())
                    }
                    "ice-channels" | "channels" => headers.channels = value.parse().ok(),
                    _ => {}
                }
            }
        }
        headers
    }
}

#[cfg(test)]
mod tests {
    use super::{IcyDemuxer, IcyHeaders, decode_text, parse_block, split_title};

    fn block(text: &str) -> Vec<u8> {
        let mut bytes = text.as_bytes().to_vec();
        let padded = bytes.len().div_ceil(16) * 16;
        bytes.resize(padded, 0);
        let mut out = vec![(padded / 16) as u8];
        out.extend(bytes);
        out
    }

    /// A stream of `metaint`-byte audio runs separated by metadata blocks.
    fn stream(metaint: usize, blocks: &[&str]) -> (Vec<u8>, Vec<u8>) {
        let mut wire = Vec::new();
        let mut audio = Vec::new();
        for (index, text) in blocks.iter().enumerate() {
            let run: Vec<u8> = (0..metaint).map(|byte| (byte + index) as u8).collect();
            wire.extend(&run);
            audio.extend(&run);
            if text.is_empty() {
                wire.push(0);
            } else {
                wire.extend(block(text));
            }
        }
        (wire, audio)
    }

    #[test]
    fn metadata_is_removed_at_any_chunk_boundary() {
        let (wire, expected_audio) = stream(
            37,
            &[
                "StreamTitle='Miles Davis - So What';StreamUrl='';",
                "",
                "StreamTitle='Miles Davis - So What';",
                "StreamTitle='Guns N' Roses - Patience';StreamUrl='https://x.example/a';",
            ],
        );
        for chunk_size in [1, 2, 7, 16, 37, 38, 100, wire.len()] {
            let mut demuxer = IcyDemuxer::new(37).unwrap();
            let mut audio = Vec::new();
            let mut titles = Vec::new();
            for chunk in wire.chunks(chunk_size) {
                titles.extend(demuxer.feed(chunk, &mut audio));
            }
            assert_eq!(audio, expected_audio, "chunk size {chunk_size}");
            let titles: Vec<_> = titles.into_iter().map(|meta| meta.stream_title).collect();
            assert_eq!(
                titles,
                ["Miles Davis - So What", "Guns N' Roses - Patience"],
                "repeats are dropped (chunk size {chunk_size})"
            );
        }
    }

    #[test]
    fn blocks_decode_utf8_or_latin1_and_split_titles() {
        let meta = parse_block(b"StreamTitle='Bj\xf6rk - J\xf3ga';\0\0\0").unwrap();
        assert_eq!(meta.stream_title, "Björk - Jóga");
        let meta = parse_block("StreamTitle='Sigur Rós - Hoppípolla';".as_bytes()).unwrap();
        let title = split_title(&meta.stream_title, None).unwrap();
        assert_eq!(title.artist.as_deref(), Some("Sigur Rós"));
        assert_eq!(title.title, "Hoppípolla");
        assert_eq!(meta.stream_url, None);
        assert_eq!(decode_text(b"plain"), "plain");
        assert!(parse_block(b"garbage").is_none());

        assert!(split_title("", None).is_none());
        assert!(split_title(" - ", None).is_none());
        assert!(split_title("Radio Paradise", Some("radio paradise")).is_none());
        assert!(split_title("Europe1", Some("Europe 1")).is_none());
        let plain = split_title("The morning news", Some("NPR")).unwrap();
        assert_eq!(plain.artist, None);
        assert_eq!(plain.title, "The morning news");
    }

    #[test]
    fn invalid_intervals_are_refused() {
        assert!(IcyDemuxer::new(0).is_none());
        assert!(IcyDemuxer::new(super::MAX_METAINT + 1).is_none());
    }

    #[test]
    fn headers_give_bitrate_rate_and_channels() {
        let headers = IcyHeaders::read(|name| match name {
            "icy-name" => Some("Jazz24".into()),
            "icy-br" => Some("128, 128".into()),
            "icy-audio-info" => Some("ice-samplerate=48000;ice-bitrate=128;ice-channels=2".into()),
            _ => None,
        });
        assert_eq!(headers.name.as_deref(), Some("Jazz24"));
        assert_eq!(headers.bitrate_kbps, Some(128));
        assert_eq!(headers.sample_rate, Some(48_000));
        assert_eq!(headers.channels, Some(2));
        assert_eq!(IcyHeaders::read(|_| None), IcyHeaders::default());
    }
}
