//! Reads the format a stream actually delivers from its first bytes.
//!
//! The quality a service reports in its playback answer is what it intends
//! to send; this module reads what arrived: FLAC `STREAMINFO` (plain or
//! inside fragmented MP4 `dfLa`), MP4 sample entries (`mp4a`, `alac`,
//! `ec-3`), WAV `fmt `, MP3 and ADTS frame headers. For FLAC it also parses
//! the first audio frame header, checks its CRC-8 and confirms it agrees
//! with `STREAMINFO`, so a mislabelled container cannot pass as hi-res.

use serde::Serialize;

use super::AudioFormat;

/// How many leading bytes the media proxy keeps for probing. Large enough
/// to pass embedded cover art in FLAC metadata and a DASH init segment.
pub const PROBE_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProbedFormat {
    pub format: AudioFormat,
    /// `flac`, `mp4`, `wav`, `mp3` or `adts`.
    pub container: &'static str,
    /// Whether the first audio frame was found and agrees with the header.
    pub frame_verified: bool,
    /// The stream's length as its own header states it (FLAC
    /// `STREAMINFO` total samples, MP4 `mvhd`/`mehd`, WAV `data` size).
    /// `None` when the header does not say, as with MP3 and ADTS.
    pub seconds: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StreamInfo {
    sample_rate: u32,
    channels: u8,
    bit_depth: u8,
    /// 0 when the encoder did not know the length.
    total_samples: u64,
}

impl StreamInfo {
    fn seconds(self) -> Option<f64> {
        (self.total_samples > 0).then(|| self.total_samples as f64 / f64::from(self.sample_rate))
    }
}

fn u16_be(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_be(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u16_le(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_le(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// Probes the first bytes of a stream. `None` means more bytes are needed
/// or the format is unknown.
pub fn probe(bytes: &[u8]) -> Option<ProbedFormat> {
    let start = skip_id3(bytes)?;
    let body = &bytes[start..];
    if body.starts_with(b"fLaC") {
        return probe_flac(body);
    }
    if body.len() >= 12 && &body[..4] == b"RIFF" && &body[8..12] == b"WAVE" {
        return probe_wav(body);
    }
    if body.len() >= 8 && matches!(&body[4..8], b"ftyp" | b"moov" | b"styp" | b"sidx") {
        return probe_mp4(body);
    }
    probe_mpeg_frame(body)
}

/// Offset past a leading ID3v2 tag, or `None` while the tag is incomplete.
fn skip_id3(bytes: &[u8]) -> Option<usize> {
    if !bytes.starts_with(b"ID3") {
        return Some(0);
    }
    let header = bytes.get(..10)?;
    let size = header[6..10]
        .iter()
        .fold(0_usize, |size, byte| (size << 7) | usize::from(byte & 0x7f));
    let footer = if header[5] & 0x10 != 0 { 10 } else { 0 };
    let end = 10 + size + footer;
    (bytes.len() > end).then_some(end)
}

fn parse_streaminfo(block: &[u8]) -> Option<StreamInfo> {
    let block = block.get(..18)?;
    let sample_rate =
        (u32::from(block[10]) << 12) | (u32::from(block[11]) << 4) | (u32::from(block[12]) >> 4);
    let channels = ((block[12] >> 1) & 0x07) + 1;
    let bit_depth = (((block[12] & 0x01) << 4) | (block[13] >> 4)) + 1;
    let total_samples = (u64::from(block[13] & 0x0f) << 32) | u64::from(u32_be(block, 14)?);
    (sample_rate > 0).then_some(StreamInfo {
        sample_rate,
        channels,
        bit_depth,
        total_samples,
    })
}

fn flac_format(info: StreamInfo) -> AudioFormat {
    let mut format = AudioFormat::lossless("flac", Some(info.bit_depth), Some(info.sample_rate));
    format.channels = Some(info.channels);
    format
}

/// Walks FLAC metadata blocks; returns `STREAMINFO` and the offset of the
/// first audio frame when the walk reached it.
fn flac_metadata(body: &[u8]) -> Option<(StreamInfo, Option<usize>)> {
    let mut at = 4;
    let mut info = None;
    loop {
        let Some(header) = body.get(at..at + 4) else {
            return info.map(|info| (info, None));
        };
        let last = header[0] & 0x80 != 0;
        let kind = header[0] & 0x7f;
        let length =
            (usize::from(header[1]) << 16) | (usize::from(header[2]) << 8) | usize::from(header[3]);
        if kind == 0 {
            info = Some(parse_streaminfo(body.get(at + 4..at + 4 + length)?)?);
        }
        at += 4 + length;
        if last {
            return info.map(|info| (info, Some(at)));
        }
    }
}

fn probe_flac(body: &[u8]) -> Option<ProbedFormat> {
    let (info, frame_at) = flac_metadata(body)?;
    let frame_verified = frame_at
        .and_then(|at| body.get(at..))
        .is_some_and(|frame| flac_frame_matches(frame, info));
    Some(ProbedFormat {
        format: flac_format(info),
        container: "flac",
        frame_verified,
        seconds: info.seconds(),
    })
}

/// CRC-8 with polynomial 0x07, as FLAC frame headers use.
fn crc8(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0_u8, |mut crc, byte| {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
        crc
    })
}

/// Parses a FLAC frame header and checks it against `STREAMINFO`.
fn flac_frame_matches(frame: &[u8], info: StreamInfo) -> bool {
    let Some(header) = frame.get(..4) else {
        return false;
    };
    if header[0] != 0xff || header[1] & 0xfe != 0xf8 {
        return false;
    }
    let block_code = header[2] >> 4;
    let rate_code = header[2] & 0x0f;
    let channel_code = header[3] >> 4;
    let size_code = (header[3] >> 1) & 0x07;
    if header[3] & 0x01 != 0 || rate_code == 0x0f || channel_code > 10 || size_code == 3 {
        return false;
    }
    // The UTF-8-style coded frame or sample number.
    let Some(&lead) = frame.get(4) else {
        return false;
    };
    let extra = match lead {
        0x00..=0x7f => 0,
        0xc0..=0xdf => 1,
        0xe0..=0xef => 2,
        0xf0..=0xf7 => 3,
        0xf8..=0xfb => 4,
        0xfc..=0xfd => 5,
        0xfe => 6,
        _ => return false,
    };
    let mut at = 5 + extra;
    at += match block_code {
        6 => 1,
        7 => 2,
        _ => 0,
    };
    let rate = match rate_code {
        0 => Some(info.sample_rate),
        1 => Some(88_200),
        2 => Some(176_400),
        3 => Some(192_000),
        4 => Some(8_000),
        5 => Some(16_000),
        6 => Some(22_050),
        7 => Some(24_000),
        8 => Some(32_000),
        9 => Some(44_100),
        10 => Some(48_000),
        11 => Some(96_000),
        12 => {
            at += 1;
            frame.get(at - 1).map(|value| u32::from(*value) * 1000)
        }
        13 => {
            at += 2;
            u16_be(frame, at - 2).map(u32::from)
        }
        14 => {
            at += 2;
            u16_be(frame, at - 2).map(|value| u32::from(value) * 10)
        }
        _ => None,
    };
    let depth = match size_code {
        0 => Some(info.bit_depth),
        1 => Some(8),
        2 => Some(12),
        4 => Some(16),
        5 => Some(20),
        6 => Some(24),
        7 => Some(32),
        _ => None,
    };
    let channels = if channel_code <= 7 {
        channel_code + 1
    } else {
        2
    };
    let Some(&crc) = frame.get(at) else {
        return false;
    };
    crc8(&frame[..at]) == crc
        && rate == Some(info.sample_rate)
        && depth == Some(info.bit_depth)
        && channels == info.channels
}

fn probe_wav(body: &[u8]) -> Option<ProbedFormat> {
    let mut at = 12;
    while let Some(id) = body.get(at..at + 4) {
        let length = u32_le(body, at + 4)? as usize;
        if id == b"fmt " {
            let tag = u16_le(body, at + 8)?;
            let channels = u16_le(body, at + 10)?;
            let sample_rate = u32_le(body, at + 12)?;
            let bits = u16_le(body, at + 22)?;
            let codec = match tag {
                1 | 0xfffe => "pcm",
                3 => "pcm float",
                _ => return None,
            };
            let mut format =
                AudioFormat::lossless(codec, u8::try_from(bits).ok(), Some(sample_rate));
            format.channels = u8::try_from(channels).ok();
            return Some(ProbedFormat {
                format,
                container: "wav",
                frame_verified: false,
                seconds: wav_seconds(body, at + 8 + length, sample_rate, channels, bits),
            });
        }
        at += 8 + length + (length & 1);
    }
    None
}

/// The length of a WAV file's `data` chunk in seconds, looking from the
/// chunk after `fmt `.
fn wav_seconds(body: &[u8], from: usize, rate: u32, channels: u16, bits: u16) -> Option<f64> {
    let bytes_per_second = u64::from(rate) * u64::from(channels) * u64::from(bits).div_ceil(8);
    let mut at = from + (from & 1);
    while let Some(id) = body.get(at..at + 4) {
        let length = u32_le(body, at + 4)?;
        if id == b"data" {
            return (bytes_per_second > 0 && length > 0)
                .then(|| f64::from(length) / bytes_per_second as f64);
        }
        at += 8 + length as usize + (length as usize & 1);
    }
    None
}

/// The movie length from `mvhd`, or for fragmented MP4 (DASH) from
/// `mvex/mehd`, in seconds.
fn mp4_seconds(moov: &[u8]) -> Option<f64> {
    let mvhd = child(moov, b"mvhd")?;
    let (timescale, duration) = if *mvhd.first()? == 1 {
        (
            u32_be(mvhd, 20)?,
            u64::from_be_bytes(mvhd.get(24..32)?.try_into().ok()?),
        )
    } else {
        (u32_be(mvhd, 12)?, u64::from(u32_be(mvhd, 16)?))
    };
    let unknown = |value: u64| value == 0 || value == u64::from(u32::MAX) || value == u64::MAX;
    let duration = if unknown(duration) {
        let mehd = path(moov, &[b"mvex", b"mehd"])?;
        if *mehd.first()? == 1 {
            u64::from_be_bytes(mehd.get(4..12)?.try_into().ok()?)
        } else {
            u64::from(u32_be(mehd, 4)?)
        }
    } else {
        duration
    };
    (timescale > 0 && !unknown(duration)).then(|| duration as f64 / f64::from(timescale))
}

/// An MP4 box: type, payload range.
fn boxes(bytes: &[u8]) -> impl Iterator<Item = (&[u8], &[u8])> {
    let mut at = 0;
    std::iter::from_fn(move || {
        let size = u32_be(bytes, at)? as usize;
        let kind = bytes.get(at + 4..at + 8)?;
        let (header, size) = match size {
            1 => (
                16,
                usize::try_from(u64::from_be_bytes(
                    bytes.get(at + 8..at + 16)?.try_into().ok()?,
                ))
                .ok()?,
            ),
            0 => (8, bytes.len() - at),
            size => (8, size),
        };
        if size < header {
            return None;
        }
        // A box cut off by the probe window still yields what arrived.
        let end = (at + size).min(bytes.len());
        let payload = bytes.get(at + header..end)?;
        at += size;
        Some((kind, payload))
    })
}

fn child<'a>(bytes: &'a [u8], kind: &[u8]) -> Option<&'a [u8]> {
    boxes(bytes)
        .find(|(name, _)| *name == kind)
        .map(|(_, payload)| payload)
}

fn path<'a>(bytes: &'a [u8], kinds: &[&[u8]]) -> Option<&'a [u8]> {
    kinds
        .iter()
        .try_fold(bytes, |bytes, kind| child(bytes, kind))
}

fn probe_mp4(body: &[u8]) -> Option<ProbedFormat> {
    let moov = child(body, b"moov")?;
    let stsd = boxes(moov)
        .filter(|(kind, _)| *kind == b"trak")
        .find_map(|(_, trak)| {
            let handler = path(trak, &[b"mdia", b"hdlr"])?;
            (handler.get(8..12)? == b"soun")
                .then(|| path(trak, &[b"mdia", b"minf", b"stbl", b"stsd"]))
                .flatten()
        })?;
    // Full box header (4) and entry count (4), then the first sample entry.
    let (entry, payload) = boxes(stsd.get(8..)?).next()?;
    // Sample entry (8) and audio sample entry fields (20) come first.
    let channels = u16_be(payload, 16)?;
    let sample_size = u16_be(payload, 18)?;
    let rate_fixed = u32_be(payload, 24)? >> 16;
    let children = payload.get(28..)?;
    let mut format = match entry {
        b"fLaC" => {
            let dfla = child(children, b"dfLa")?;
            let blocks = dfla.get(4..)?;
            let length = (usize::from(*blocks.get(1)?) << 16)
                | (usize::from(*blocks.get(2)?) << 8)
                | usize::from(*blocks.get(3)?);
            if blocks[0] & 0x7f != 0 {
                return None;
            }
            let info = parse_streaminfo(blocks.get(4..4 + length)?)?;
            let frame_verified =
                first_mdat_sample(body).is_some_and(|sample| flac_frame_matches(sample, info));
            return Some(ProbedFormat {
                format: flac_format(info),
                container: "mp4",
                frame_verified,
                seconds: mp4_seconds(moov).or_else(|| info.seconds()),
            });
        }
        b"alac" => {
            let config = child(children, b"alac")?.get(4..)?;
            let mut format =
                AudioFormat::lossless("alac", config.get(5).copied(), u32_be(config, 20));
            format.channels = config.get(9).copied();
            format
        }
        b"mp4a" => {
            let (rate, bitrate) = child(children, b"esds")
                .and_then(|esds| aac_config(esds.get(4..)?))
                .unwrap_or((rate_fixed, None));
            let mut format = AudioFormat::lossy("aac", bitrate.unwrap_or(0));
            if bitrate.is_none() {
                format.bitrate_kbps = None;
                format.label = "AAC".into();
            }
            format.sample_rate = Some(rate);
            format
        }
        b"ec-3" | b"ac-3" => {
            let mut format = AudioFormat::lossy(if entry == b"ec-3" { "eac3" } else { "ac3" }, 0);
            format.bitrate_kbps = None;
            format.label = if entry == b"ec-3" { "E-AC-3" } else { "AC-3" }.into();
            format.sample_rate = Some(rate_fixed);
            format
        }
        _ => return None,
    };
    if format.channels.is_none() {
        format.channels = u8::try_from(channels).ok();
    }
    if format.lossless && format.bit_depth.is_none() {
        format.bit_depth = u8::try_from(sample_size).ok();
    }
    Some(ProbedFormat {
        format,
        container: "mp4",
        frame_verified: false,
        seconds: mp4_seconds(moov),
    })
}

/// The start of the first `mdat` payload after the movie header.
fn first_mdat_sample(body: &[u8]) -> Option<&[u8]> {
    boxes(body)
        .find(|(kind, _)| *kind == b"mdat")
        .map(|(_, payload)| payload)
}

/// Reads an ES descriptor: the AudioSpecificConfig sample rate and the
/// average bitrate.
fn aac_config(descriptor: &[u8]) -> Option<(u32, Option<u32>)> {
    fn read(bytes: &[u8], at: usize) -> Option<(u8, usize, usize)> {
        let tag = *bytes.get(at)?;
        let mut length = 0_usize;
        let mut offset = at + 1;
        for _ in 0..4 {
            let byte = *bytes.get(offset)?;
            offset += 1;
            length = (length << 7) | usize::from(byte & 0x7f);
            if byte & 0x80 == 0 {
                break;
            }
        }
        Some((tag, offset, length))
    }
    let (tag, mut at, _) = read(descriptor, 0)?;
    if tag != 0x03 {
        return None;
    }
    let flags = *descriptor.get(at + 2)?;
    at += 3;
    if flags & 0x80 != 0 {
        at += 2;
    }
    if flags & 0x40 != 0 {
        at += 1 + usize::from(*descriptor.get(at)?);
    }
    if flags & 0x20 != 0 {
        at += 2;
    }
    let (tag, at, _) = read(descriptor, at)?;
    if tag != 0x04 {
        return None;
    }
    let average = u32_be(descriptor, at + 9)?;
    let (tag, config_at, _) = read(descriptor, at + 13)?;
    if tag != 0x05 {
        return None;
    }
    let config = u16_be(descriptor, config_at)?;
    let index = ((config >> 7) & 0x0f) as usize;
    const RATES: [u32; 13] = [
        96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025,
        8_000, 7_350,
    ];
    let rate = *RATES.get(index)?;
    Some((rate, (average > 0).then(|| (average + 500) / 1000)))
}

fn probe_mpeg_frame(body: &[u8]) -> Option<ProbedFormat> {
    let header = body.get(..4)?;
    if header[0] != 0xff || header[1] & 0xe0 != 0xe0 {
        return None;
    }
    // ADTS AAC: layer bits are 00 and the sync is 12 bits.
    if header[1] & 0xf6 == 0xf0 {
        let index = usize::from((header[2] >> 2) & 0x0f);
        const RATES: [u32; 13] = [
            96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025,
            8_000, 7_350,
        ];
        let mut format = AudioFormat::lossy("aac", 0);
        format.bitrate_kbps = None;
        format.label = "AAC".into();
        format.sample_rate = Some(*RATES.get(index)?);
        format.channels = Some(((header[2] & 0x01) << 2) | (header[3] >> 6));
        return Some(ProbedFormat {
            format,
            container: "adts",
            frame_verified: true,
            seconds: None,
        });
    }
    let version = (header[1] >> 3) & 0x03;
    let layer = (header[1] >> 1) & 0x03;
    if layer != 0x01 || version == 0x01 {
        return None;
    }
    let bitrate_index = usize::from(header[2] >> 4);
    let rate_index = usize::from((header[2] >> 2) & 0x03);
    const MPEG1_L3: [u32; 15] = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
    ];
    const MPEG2_L3: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
    let (bitrates, rates): (&[u32], [u32; 3]) = match version {
        0x03 => (&MPEG1_L3, [44_100, 48_000, 32_000]),
        0x02 => (&MPEG2_L3, [22_050, 24_000, 16_000]),
        _ => (&MPEG2_L3, [11_025, 12_000, 8_000]),
    };
    let bitrate = *bitrates.get(bitrate_index)?;
    let rate = *rates.get(rate_index)?;
    if bitrate == 0 {
        return None;
    }
    let mut format = AudioFormat::lossy("mp3", bitrate);
    format.sample_rate = Some(rate);
    format.channels = Some(if header[3] >> 6 == 0x03 { 1 } else { 2 });
    Some(ProbedFormat {
        format,
        container: "mp3",
        frame_verified: true,
        seconds: None,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{crc8, probe};

    /// A FLAC `STREAMINFO` block body for the given format.
    pub(crate) fn streaminfo(rate: u32, channels: u8, depth: u8) -> [u8; 34] {
        let mut block = [0_u8; 34];
        block[0..2].copy_from_slice(&4096_u16.to_be_bytes());
        block[2..4].copy_from_slice(&4096_u16.to_be_bytes());
        block[10] = (rate >> 12) as u8;
        block[11] = (rate >> 4) as u8;
        block[12] = ((rate & 0x0f) as u8) << 4 | ((channels - 1) << 1) | ((depth - 1) >> 4);
        block[13] = ((depth - 1) & 0x0f) << 4;
        block
    }

    /// A FLAC frame header with codes for the rate and depth, and its CRC.
    pub(crate) fn frame_header(rate_code: u8, channel_code: u8, size_code: u8) -> Vec<u8> {
        let mut header = vec![
            0xff,
            0xf8,
            0xc0 | rate_code,
            (channel_code << 4) | (size_code << 1),
            0x00,
        ];
        header.push(crc8(&header));
        header
    }

    pub(crate) fn flac_file(rate: u32, channels: u8, depth: u8, frame: &[u8]) -> Vec<u8> {
        let mut file = b"fLaC".to_vec();
        // STREAMINFO, then a padding block standing in for cover art.
        file.extend_from_slice(&[0x00, 0x00, 0x00, 34]);
        file.extend_from_slice(&streaminfo(rate, channels, depth));
        file.extend_from_slice(&[0x81, 0x00, 0x00, 64]);
        file.extend_from_slice(&[0_u8; 64]);
        file.extend_from_slice(frame);
        file
    }

    #[test]
    fn flac_streams_report_streaminfo_and_verify_the_first_frame() {
        let file = flac_file(192_000, 2, 24, &frame_header(3, 1, 6));
        let probed = probe(&file).expect("flac");
        assert_eq!(probed.container, "flac");
        assert_eq!(probed.format.label, "24/192 FLAC");
        assert_eq!(probed.format.channels, Some(2));
        assert!(probed.frame_verified);

        // A frame header that says 16-bit inside a "24-bit" stream fails.
        let mislabelled = flac_file(192_000, 2, 24, &frame_header(3, 1, 4));
        assert!(!probe(&mislabelled).expect("flac").frame_verified);

        // A corrupt CRC fails verification but keeps the header format.
        let mut corrupt = flac_file(44_100, 2, 16, &frame_header(9, 1, 4));
        let last = corrupt.len() - 1;
        corrupt[last] ^= 0x55;
        let probed = probe(&corrupt).expect("flac");
        assert_eq!(probed.format.label, "16/44.1 FLAC");
        assert!(!probed.frame_verified);

        // No sample count in STREAMINFO: the length is unknown.
        assert_eq!(probe(&file).expect("flac").seconds, None);
        let mut whole = file.clone();
        // 4:01 at 192 kHz: 46,272,000 samples in bits 108-143.
        let samples: u64 = 241 * 192_000;
        whole[8 + 13] |= (samples >> 32) as u8 & 0x0f;
        whole[8 + 14..8 + 18].copy_from_slice(&(samples as u32).to_be_bytes());
        assert_eq!(probe(&whole).expect("flac").seconds, Some(241.0));
        assert!(probe(&whole).expect("flac").frame_verified);

        // Only the metadata arrived: format known, frame not yet verified.
        let partial = &file[..file.len() - 6];
        assert!(!probe(partial).expect("flac").frame_verified);
        assert_eq!(probe(&file[..20]), None, "STREAMINFO is incomplete");
    }

    #[test]
    fn id3_tags_before_flac_and_mp3_are_skipped() {
        let mut tagged = b"ID3\x04\x00\x00\x00\x00\x00\x0a".to_vec();
        tagged.extend_from_slice(&[0_u8; 10]);
        tagged.extend(flac_file(96_000, 2, 24, &frame_header(11, 1, 6)));
        let probed = probe(&tagged).expect("flac");
        assert_eq!(probed.format.label, "24/96 FLAC");
        assert!(probed.frame_verified);

        let mp3 = [0xff, 0xfb, 0xe0, 0x44];
        let probed = probe(&mp3).expect("mp3");
        assert_eq!(probed.format.label, "MP3 320");
        assert_eq!(probed.format.sample_rate, Some(44_100));
    }

    #[test]
    fn wav_headers_report_pcm_depth_and_rate() {
        let mut wav = b"RIFF\x00\x00\x00\x00WAVEfmt ".to_vec();
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&44_100_u32.to_le_bytes());
        wav.extend_from_slice(&88_200_u32.to_le_bytes());
        wav.extend_from_slice(&2_u16.to_le_bytes());
        wav.extend_from_slice(&16_u16.to_le_bytes());
        let probed = probe(&wav).expect("wav");
        assert_eq!(probed.format.label, "16/44.1 PCM");
        assert_eq!(probed.format.channels, Some(1));
        assert_eq!(probed.seconds, None, "no data chunk yet");
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(88_200_u32 * 30).to_le_bytes());
        assert_eq!(probe(&wav).expect("wav").seconds, Some(30.0));
    }

    fn mp4_box(kind: &[u8], payload: &[u8]) -> Vec<u8> {
        let mut out = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out
    }

    /// A fragmented MP4 init segment plus one fragment, the shape TIDAL's
    /// DASH streams use for FLAC.
    pub(crate) fn fmp4_flac(rate: u32, depth: u8, frame: &[u8]) -> Vec<u8> {
        let mut dfla = vec![0, 0, 0, 0, 0x80, 0, 0, 34];
        dfla.extend_from_slice(&streaminfo(rate, 2, depth));
        let mut entry = vec![0_u8; 6];
        entry.extend_from_slice(&1_u16.to_be_bytes());
        entry.extend_from_slice(&[0_u8; 8]);
        entry.extend_from_slice(&2_u16.to_be_bytes());
        entry.extend_from_slice(&u16::from(depth).to_be_bytes());
        entry.extend_from_slice(&[0_u8; 4]);
        entry.extend_from_slice(&(rate.min(65_535) << 16).to_be_bytes());
        entry.extend(mp4_box(b"dfLa", &dfla));
        let mut stsd = vec![0, 0, 0, 0, 0, 0, 0, 1];
        stsd.extend(mp4_box(b"fLaC", &entry));
        let stbl = mp4_box(b"stbl", &mp4_box(b"stsd", &stsd));
        let minf = mp4_box(b"minf", &stbl);
        let mut hdlr = vec![0_u8; 8];
        hdlr.extend_from_slice(b"soun");
        hdlr.extend_from_slice(&[0_u8; 12]);
        let mut mdia = mp4_box(b"hdlr", &hdlr);
        mdia.extend(minf);
        let trak = mp4_box(b"trak", &mp4_box(b"mdia", &mdia));
        let mut file = mp4_box(b"ftyp", b"iso6\x00\x00\x00\x00iso6dash");
        file.extend(mp4_box(b"moov", &trak));
        file.extend(mp4_box(b"moof", &[0_u8; 16]));
        file.extend(mp4_box(b"mdat", frame));
        file
    }

    #[test]
    fn fragmented_mp4_flac_reads_dfla_and_the_first_sample() {
        let file = fmp4_flac(192_000, 24, &frame_header(3, 1, 6));
        let probed = probe(&file).expect("mp4");
        assert_eq!(probed.container, "mp4");
        assert_eq!(probed.format.label, "24/192 FLAC");
        assert!(probed.frame_verified);

        // The init segment alone gives the format, not the verification.
        let init_end = file.len() - (16 + 8) - (6 + 8);
        let probed = probe(&file[..init_end]).expect("init segment");
        assert_eq!(probed.format.label, "24/192 FLAC");
        assert!(!probed.frame_verified);
    }

    /// Files encoded by ffmpeg, the way real services package them.
    #[test]
    fn encoder_written_files_probe_to_their_real_formats() {
        let flac = include_bytes!("fixtures/media/hires-192.flac");
        let probed = probe(flac).expect("flac");
        assert_eq!(
            (
                probed.container,
                probed.format.label.as_str(),
                probed.frame_verified
            ),
            ("flac", "24/192 FLAC", true)
        );
        assert_eq!(probed.format.channels, Some(2));
        let close = |seconds: Option<f64>, expected: f64| {
            seconds.is_some_and(|seconds| (seconds - expected).abs() < 0.005)
        };
        assert!(close(probed.seconds, 0.03), "{:?}", probed.seconds);

        for (file, label) in [
            (
                &include_bytes!("fixtures/media/hires-192-dash.mp4")[..],
                "24/192 FLAC",
            ),
            (
                &include_bytes!("fixtures/media/hires-96-dash.mp4")[..],
                "24/96 FLAC",
            ),
        ] {
            let probed = probe(file).expect("dash flac");
            assert_eq!(probed.container, "mp4");
            assert_eq!(probed.format.label, label);
            assert!(probed.frame_verified, "{label}: first fragment frame");
        }

        let aac = probe(include_bytes!("fixtures/media/aac-320.mp4")).expect("aac");
        // Fragmented, with no `mehd`: the file does not state its length.
        assert_eq!(aac.seconds, None);
        assert_eq!(aac.format.codec, "aac");
        assert!(!aac.format.lossless);
        assert_eq!(aac.format.sample_rate, Some(44_100));

        let mp3 = probe(include_bytes!("fixtures/media/mp3-320.mp3")).expect("mp3");
        assert_eq!(mp3.format.label, "MP3 320");
    }

    #[test]
    fn unknown_or_truncated_bytes_are_not_guessed() {
        assert_eq!(probe(b""), None);
        assert_eq!(probe(b"<html>error</html>"), None);
        assert_eq!(probe(b"ID3\x04\x00\x00\x00\x00\x10\x00"), None);
        assert_eq!(probe(&[0xff, 0xfb, 0x00, 0x00]), None, "free bitrate");
    }
}
