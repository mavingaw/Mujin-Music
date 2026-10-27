//! What a live stream really carries, read from its bytes.
//!
//! Directories and `icy-br` headers say what a station *claims*; the
//! quality light should show what *arrives*. Parson reads the first audio
//! bytes: MP3 and ADTS frame headers, FLAC `STREAMINFO`, MP4 sample entries
//! (via [`crate::providers::probe`]), and Ogg identification headers for
//! Opus, Vorbis and Ogg FLAC. A live stream can start mid-frame, so MP3 and
//! ADTS are only trusted when two consecutive frame headers agree.

use serde::Serialize;

use crate::providers::probe;

/// How the stream's bytes are packaged.
#[derive(Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Container {
    Mp3,
    Adts,
    Flac,
    Ogg,
    /// MPEG transport stream (HLS segments).
    Ts,
    /// Fragmented MP4 (HLS or DASH segments).
    Fmp4,
    Wav,
    Unknown,
}

impl Container {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Adts => "adts",
            Self::Flac => "flac",
            Self::Ogg => "ogg",
            Self::Ts => "mpegts",
            Self::Fmp4 => "mp4",
            Self::Wav => "wav",
            Self::Unknown => "unknown",
        }
    }

    /// Whether browsers and renderers play the bytes as they come.
    pub fn is_playable_stream(self) -> bool {
        matches!(self, Self::Mp3 | Self::Adts | Self::Flac | Self::Ogg)
    }
}

/// A format read from the stream's bytes.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Detected {
    pub container: Container,
    /// `mp3`, `aac`, `flac`, `opus`, `vorbis`, `alac`, `pcm`.
    pub codec: Option<String>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u8>,
    pub channels: Option<u8>,
    /// From the frame header (MP3 only; AAC frames do not state it).
    pub bitrate_kbps: Option<u32>,
}

impl Detected {
    fn container(container: Container) -> Self {
        Self {
            container,
            codec: None,
            sample_rate: None,
            bit_depth: None,
            channels: None,
            bitrate_kbps: None,
        }
    }
}

/// Bytes to read before giving up on detection.
pub const DETECT_BYTES: usize = 64 * 1024;

fn ogg(bytes: &[u8]) -> Option<Detected> {
    // First page: "OggS", version, flags, granule(8), serial(4), seq(4),
    // crc(4), segment count, segment table, then the first packet.
    if bytes.len() < 28 || &bytes[..4] != b"OggS" {
        return None;
    }
    let segments = usize::from(bytes[26]);
    let packet = bytes.get(27 + segments..)?;
    let mut detected = Detected::container(Container::Ogg);
    if packet.starts_with(b"OpusHead") {
        detected.codec = Some("opus".into());
        detected.channels = packet.get(9).copied();
        // Opus always decodes at 48 kHz; the header's rate is the input's.
        detected.sample_rate = Some(48_000);
    } else if packet.len() >= 16 && packet[0] == 1 && &packet[1..7] == b"vorbis" {
        detected.codec = Some("vorbis".into());
        detected.channels = packet.get(11).copied();
        detected.sample_rate = packet
            .get(12..16)
            .map(|rate| u32::from_le_bytes([rate[0], rate[1], rate[2], rate[3]]));
        detected.bitrate_kbps = packet
            .get(20..24)
            .map(|rate| i32::from_le_bytes([rate[0], rate[1], rate[2], rate[3]]))
            .filter(|&rate| rate > 0)
            .map(|rate| (rate / 1000) as u32);
    } else if packet.starts_with(b"\x7fFLAC") {
        detected.codec = Some("flac".into());
        // `\x7fFLAC` + version(2) + header count(2) + "fLaC" + STREAMINFO.
        if let Some(found) = packet.get(9..).and_then(probe::probe) {
            detected.sample_rate = found.format.sample_rate;
            detected.bit_depth = found.format.bit_depth;
            detected.channels = found.format.channels;
        }
    }
    Some(detected)
}

fn from_probe(found: probe::ProbedFormat) -> Detected {
    let container = match found.container {
        "mp3" => Container::Mp3,
        "adts" => Container::Adts,
        "flac" => Container::Flac,
        "mp4" => Container::Fmp4,
        "wav" => Container::Wav,
        _ => Container::Unknown,
    };
    let codec = match found.format.codec.as_str() {
        "" => None,
        codec => Some(codec.to_ascii_lowercase()),
    };
    Detected {
        container,
        codec,
        sample_rate: found.format.sample_rate,
        bit_depth: found.format.bit_depth,
        channels: found.format.channels,
        bitrate_kbps: found.format.bitrate_kbps,
    }
}

fn frame_length(header: &[u8]) -> Option<usize> {
    if header.len() < 7 || header[0] != 0xff || header[1] & 0xe0 != 0xe0 {
        return None;
    }
    if header[1] & 0xf6 == 0xf0 {
        // ADTS: 13-bit frame length.
        let length = (usize::from(header[3] & 0x03) << 11)
            | (usize::from(header[4]) << 3)
            | usize::from(header[5] >> 5);
        return (length >= 7).then_some(length);
    }
    let version = (header[1] >> 3) & 0x03;
    let layer = (header[1] >> 1) & 0x03;
    if layer != 0x01 || version == 0x01 {
        return None;
    }
    const MPEG1: [u32; 15] = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
    ];
    const MPEG2: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
    let bitrate =
        *if version == 0x03 { &MPEG1 } else { &MPEG2 }.get(usize::from(header[2] >> 4))?;
    let rates = match version {
        0x03 => [44_100, 48_000, 32_000],
        0x02 => [22_050, 24_000, 16_000],
        _ => [11_025, 12_000, 8_000],
    };
    let rate = *rates.get(usize::from((header[2] >> 2) & 0x03))?;
    if bitrate == 0 {
        return None;
    }
    let padding = usize::from((header[2] >> 1) & 0x01);
    let samples = if version == 0x03 { 144 } else { 72 };
    Some((samples * bitrate as usize * 1000 / rate as usize) + padding)
}

/// Finds the first MPEG/ADTS frame whose successor also parses, so a
/// stream joined mid-frame is still read correctly.
fn resync(bytes: &[u8]) -> Option<Detected> {
    let limit = bytes.len().min(DETECT_BYTES);
    let mut at = 0;
    while at + 8 < limit {
        if let Some(length) = frame_length(&bytes[at..])
            && let Some(next) = bytes.get(at + length..)
            && frame_length(next).is_some()
            && let Some(found) = probe::probe(&bytes[at..])
        {
            return Some(from_probe(found));
        }
        at += 1;
    }
    None
}

fn section(data: &[u8]) -> Option<&[u8]> {
    let pointer = usize::from(*data.first()?);
    let section = data.get(1 + pointer..)?;
    let length = (usize::from(*section.get(1)? & 0x0f) << 8) | usize::from(*section.get(2)?);
    // Without the trailing CRC.
    section.get(..(3 + length).min(section.len()).saturating_sub(4))
}

/// The audio inside an MPEG transport stream (HLS `.ts` segments): the
/// stream type from the program map, and for AAC and MP3 the rate and
/// channels from the first frame of the first audio packet.
fn transport_stream(bytes: &[u8]) -> Option<Detected> {
    let mut pmt_pid = None;
    let mut audio: Option<(u16, u8)> = None;
    let mut payload: Vec<u8> = Vec::new();
    for packet in bytes.as_chunks::<188>().0 {
        if packet[0] != 0x47 {
            break;
        }
        let start = packet[1] & 0x40 != 0;
        let pid = (u16::from(packet[1] & 0x1f) << 8) | u16::from(packet[2]);
        let offset = match (packet[3] >> 4) & 0x03 {
            0x01 => 4,
            0x03 => 5 + usize::from(packet[4]),
            _ => continue,
        };
        let Some(data) = packet.get(offset..) else {
            continue;
        };
        if pid == 0 && start && pmt_pid.is_none() {
            let table = section(data)?;
            let mut at = 8;
            while at + 4 <= table.len() {
                let program = (u16::from(table[at]) << 8) | u16::from(table[at + 1]);
                if program != 0 {
                    pmt_pid =
                        Some((u16::from(table[at + 2] & 0x1f) << 8) | u16::from(table[at + 3]));
                    break;
                }
                at += 4;
            }
        } else if Some(pid) == pmt_pid && start && audio.is_none() {
            let table = section(data)?;
            let info = (usize::from(*table.get(10)? & 0x0f) << 8) | usize::from(*table.get(11)?);
            let mut at = 12 + info;
            while at + 5 <= table.len() {
                let kind = table[at];
                let elementary = (u16::from(table[at + 1] & 0x1f) << 8) | u16::from(table[at + 2]);
                let extra = (usize::from(table[at + 3] & 0x0f) << 8) | usize::from(table[at + 4]);
                if matches!(kind, 0x03 | 0x04 | 0x0f | 0x11 | 0x81 | 0x87) {
                    audio = Some((elementary, kind));
                    break;
                }
                at += 5 + extra;
            }
        } else if let Some((audio_pid, _)) = audio
            && pid == audio_pid
        {
            if start {
                if !payload.is_empty() {
                    break;
                }
                if data.len() > 9 && data[..3] == [0, 0, 1] {
                    let header = 9 + usize::from(data[8]);
                    payload.extend_from_slice(data.get(header..).unwrap_or_default());
                }
            } else if !payload.is_empty() {
                payload.extend_from_slice(data);
            }
            if payload.len() >= 8 * 1024 {
                break;
            }
        }
    }
    let (_, kind) = audio?;
    let mut detected = Detected::container(Container::Ts);
    detected.codec = Some(
        match kind {
            0x03 | 0x04 => "mp3",
            0x0f | 0x11 => "aac",
            0x81 => "ac3",
            _ => "eac3",
        }
        .to_string(),
    );
    if matches!(kind, 0x03 | 0x04 | 0x0f)
        && let Some(frame) = resync(&payload)
    {
        detected.sample_rate = frame.sample_rate;
        detected.channels = frame.channels;
        detected.bitrate_kbps = frame.bitrate_kbps;
    }
    Some(detected)
}

/// Reads the format from a stream's first bytes; `None` until enough
/// bytes arrived (or when the format is unknown).
pub fn detect(bytes: &[u8]) -> Option<Detected> {
    if bytes.is_empty() {
        return None;
    }
    if bytes[0] == 0x47 && bytes.get(188) == Some(&0x47) {
        return Some(transport_stream(bytes).unwrap_or(Detected::container(Container::Ts)));
    }
    if let Some(found) = ogg(bytes) {
        return Some(found);
    }
    let start = skip_id3(bytes)?;
    let body = &bytes[start..];
    if body.starts_with(b"fLaC")
        || (body.len() >= 12 && &body[..4] == b"RIFF")
        || (body.len() >= 8 && matches!(&body[4..8], b"ftyp" | b"moov" | b"styp" | b"sidx"))
    {
        return probe::probe(body).map(from_probe).or_else(|| {
            (body.len() >= 8 && matches!(&body[4..8], b"ftyp" | b"moov" | b"styp"))
                .then(|| Detected::container(Container::Fmp4))
        });
    }
    resync(body)
}

/// Bytes of a leading ID3v2 tag (HLS packed audio starts every segment
/// with one); `None` while the tag is incomplete.
pub fn skip_id3(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 10 || &bytes[..3] != b"ID3" {
        return Some(0);
    }
    let size = bytes[6..10]
        .iter()
        .fold(0_usize, |size, byte| (size << 7) | usize::from(byte & 0x7f));
    let footer = if bytes[5] & 0x10 != 0 { 10 } else { 0 };
    let end = 10 + size + footer;
    (bytes.len() >= end).then_some(end)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{Container, detect, skip_id3};

    /// MPEG-1 Layer III frames at `kbps`/44.1 kHz, joined back to back.
    pub(crate) fn mp3_frames(kbps: u32, frames: usize) -> Vec<u8> {
        let index = [
            0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
        ]
        .iter()
        .position(|&rate| rate == kbps)
        .unwrap() as u8;
        let length = 144 * kbps as usize * 1000 / 44_100;
        let mut out = Vec::new();
        for _ in 0..frames {
            let mut frame = vec![0_u8; length];
            frame[..4].copy_from_slice(&[0xff, 0xfb, index << 4, 0x44]);
            out.extend(frame);
        }
        out
    }

    /// ADTS AAC-LC frames at 48 kHz stereo.
    pub(crate) fn adts_frames(frames: usize) -> Vec<u8> {
        let length = 200_usize;
        let mut out = Vec::new();
        for _ in 0..frames {
            let mut frame = vec![0_u8; length];
            frame[..7].copy_from_slice(&[
                0xff,
                0xf1,
                0x4c, // LC, 48 kHz
                0x80 | ((length >> 11) as u8 & 0x03),
                (length >> 3) as u8,
                ((length & 0x07) as u8) << 5 | 0x1f,
                0xfc,
            ]);
            out.extend(frame);
        }
        out
    }

    #[test]
    fn mp3_is_found_even_when_joined_mid_frame() {
        let frames = mp3_frames(320, 4);
        let found = detect(&frames).unwrap();
        assert_eq!(found.container, Container::Mp3);
        assert_eq!(found.codec.as_deref(), Some("mp3"));
        assert_eq!(found.bitrate_kbps, Some(320));
        assert_eq!(found.sample_rate, Some(44_100));
        let joined_late = &frames[333..];
        let found = detect(joined_late).unwrap();
        assert_eq!(found.bitrate_kbps, Some(320));
        assert!(detect(&[0x00, 0x11, 0x22]).is_none());
    }

    #[test]
    fn adts_ogg_and_ts_are_recognised() {
        let found = detect(&adts_frames(3)).unwrap();
        assert_eq!(found.container, Container::Adts);
        assert_eq!(found.codec.as_deref(), Some("aac"));
        assert_eq!(found.sample_rate, Some(48_000));

        let mut opus = b"OggS\0\x02".to_vec();
        opus.extend([0_u8; 20]);
        opus.push(1); // one segment
        opus.push(19);
        opus.extend(b"OpusHead\x01\x02\x38\x01\x80\xbb\0\0\0\0\0");
        let found = detect(&opus).unwrap();
        assert_eq!(found.container, Container::Ogg);
        assert_eq!(found.codec.as_deref(), Some("opus"));
        assert_eq!(found.channels, Some(2));
        assert_eq!(found.sample_rate, Some(48_000));

        let mut vorbis = b"OggS\0\x02".to_vec();
        vorbis.extend([0_u8; 20]);
        vorbis.push(1);
        vorbis.push(30);
        vorbis.push(1);
        vorbis.extend(b"vorbis");
        vorbis.extend([0, 0, 0, 0, 2]);
        vorbis.extend(44_100_u32.to_le_bytes());
        vorbis.extend(0_i32.to_le_bytes());
        vorbis.extend(160_000_i32.to_le_bytes());
        vorbis.extend([0; 4]);
        let found = detect(&vorbis).unwrap();
        assert_eq!(found.codec.as_deref(), Some("vorbis"));
        assert_eq!(found.sample_rate, Some(44_100));
        assert_eq!(found.bitrate_kbps, Some(160));

        let mut ts = vec![0_u8; 376];
        ts[0] = 0x47;
        ts[188] = 0x47;
        let unknown = detect(&ts).unwrap();
        assert_eq!(unknown.container, Container::Ts);
        assert_eq!(unknown.codec, None);
    }

    /// A transport stream: PAT, PMT naming one audio stream of `kind`
    /// on PID 0x100, and a PES packet carrying `payload`.
    pub(crate) fn transport_stream_with(kind: u8, payload: &[u8]) -> Vec<u8> {
        let packet = |pid: u16, start: bool, body: &[u8]| {
            let mut packet = vec![
                0x47,
                (pid >> 8) as u8 | if start { 0x40 } else { 0 },
                pid as u8,
                0x10,
            ];
            packet.extend_from_slice(body);
            packet.resize(188, 0xff);
            packet
        };
        // PAT: program 1 on PMT PID 0x1000.
        let pat = [
            0x00, 0x00, 0xb0, 0x0d, 0x00, 0x01, 0xc1, 0x00, 0x00, 0x00, 0x01, 0xf0, 0x00, 0, 0, 0,
            0,
        ];
        // PMT: PCR PID 0x100, no program info, one stream.
        let pmt = [
            0x00, 0x02, 0xb0, 0x12, 0x00, 0x01, 0xc1, 0x00, 0x00, 0xe1, 0x00, 0xf0, 0x00, kind,
            0xe1, 0x00, 0xf0, 0x00, 0, 0, 0, 0,
        ];
        let mut pes = vec![0x00, 0x00, 0x01, 0xc0, 0x00, 0x00, 0x80, 0x00, 0x00];
        pes.extend_from_slice(payload);
        let mut out = packet(0, true, &pat);
        out.extend(packet(0x1000, true, &pmt));
        let mut chunks = pes.chunks(184);
        if let Some(first) = chunks.next() {
            out.extend(packet(0x100, true, first));
        }
        for chunk in chunks {
            out.extend(packet(0x100, false, chunk));
        }
        out.extend(packet(0x100, true, &[0, 0, 1, 0xc0]));
        out
    }

    #[test]
    fn audio_inside_transport_streams_is_identified() {
        let aac = detect(&transport_stream_with(0x0f, &adts_frames(4))).unwrap();
        assert_eq!(aac.container, Container::Ts);
        assert_eq!(aac.codec.as_deref(), Some("aac"));
        assert_eq!(aac.sample_rate, Some(48_000));
        assert_eq!(aac.channels, Some(2));
        let mp3 = detect(&transport_stream_with(0x03, &mp3_frames(128, 4))).unwrap();
        assert_eq!(mp3.codec.as_deref(), Some("mp3"));
        assert_eq!(mp3.bitrate_kbps, Some(128));
        let ac3 = detect(&transport_stream_with(0x81, &[0x0b, 0x77, 0, 0])).unwrap();
        assert_eq!(ac3.codec.as_deref(), Some("ac3"));
        assert_eq!(ac3.sample_rate, None);
    }

    #[test]
    fn flac_and_id3_prefixed_audio_are_read() {
        let mut flac = b"fLaC".to_vec();
        flac.push(0x80); // last metadata block, STREAMINFO
        flac.extend([0, 0, 34]);
        flac.extend(crate::providers::probe::tests::streaminfo(96_000, 2, 24));
        let found = detect(&flac).unwrap();
        assert_eq!(found.container, Container::Flac);
        assert_eq!(found.sample_rate, Some(96_000));
        assert_eq!(found.bit_depth, Some(24));

        let mut tagged = b"ID3\x04\0\0\0\0\0\x05".to_vec();
        tagged.extend([0_u8; 5]);
        assert_eq!(skip_id3(&tagged), Some(15));
        tagged.extend(adts_frames(2));
        assert_eq!(detect(&tagged).unwrap().container, Container::Adts);
        assert_eq!(skip_id3(b"ID3\x04\0\0\0\0\0\x7f"), None);
    }
}
