//! DSD-over-PCM (DoP) framing of DSD files for browsers and HTTP clients.
//!
//! Browsers cannot decode DSD, so the server either converts it to PCM with
//! ffmpeg or, for outputs that pass PCM through untouched to a DoP-capable
//! DAC, wraps the original 1-bit stream in DoP v1.1 frames. DoP carries every
//! DSD bit unchanged: each 24-bit PCM word holds a marker byte (alternately
//! `0x05` and `0xFA`) and 16 DSD bits, so a DSD64 stream travels as 24-bit
//! PCM at 176.4 kHz.
//!
//! The DSF and DSDIFF headers are read by `parson_output::dsd`, the one DSD
//! reader the library indexer, this framing and the outputs share.

use std::io::{Read, Seek, SeekFrom};

pub use parson_output::dsd::{DsdContainer, DsdFile, DsdInfo};

fn io(error: std::io::Error) -> String {
    error.to_string()
}

/// Reads a DSF or DSDIFF file's header, DSDIFF title and artist included.
pub fn read<R: Read + Seek>(reader: &mut R) -> Result<DsdFile, String> {
    parson_output::dsd::read_file(reader).map_err(io)
}

/// The stream can be framed as DoP: raw (not DST) DSD64 or faster.
pub fn supports_dop(info: &DsdInfo) -> bool {
    !info.dst_compressed && info.channels > 0 && info.rate >= 2_822_400
}

/// Size of the DoP WAV header written by [`dop_wav_header`].
pub const DOP_HEADER_LEN: u64 = 68;

/// Total size of the DoP WAV file for a stream.
pub fn dop_wav_length(stream: &DsdInfo) -> u64 {
    DOP_HEADER_LEN + stream.dop_frames() * u64::from(stream.channels) * 3
}

/// A `WAVE_FORMAT_EXTENSIBLE` header for 24-bit DoP PCM.
pub fn dop_wav_header(stream: &DsdInfo) -> Vec<u8> {
    let channels = u32::from(stream.channels);
    let rate = stream.dop_rate();
    let block_align = channels * 3;
    let data_len = (stream.dop_frames() * u64::from(block_align)).min(u64::from(u32::MAX - 60));
    let mut header = Vec::with_capacity(DOP_HEADER_LEN as usize);
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&((data_len as u32) + 60).to_le_bytes());
    header.extend_from_slice(b"WAVEfmt ");
    header.extend_from_slice(&40_u32.to_le_bytes());
    header.extend_from_slice(&0xfffe_u16.to_le_bytes());
    header.extend_from_slice(&(channels as u16).to_le_bytes());
    header.extend_from_slice(&rate.to_le_bytes());
    header.extend_from_slice(&(rate * block_align).to_le_bytes());
    header.extend_from_slice(&(block_align as u16).to_le_bytes());
    header.extend_from_slice(&24_u16.to_le_bytes());
    header.extend_from_slice(&22_u16.to_le_bytes());
    header.extend_from_slice(&24_u16.to_le_bytes());
    let mask: u32 = match channels {
        1 => 0x4,
        2 => 0x3,
        6 => 0x3f,
        8 => 0x63f,
        _ => 0,
    };
    header.extend_from_slice(&mask.to_le_bytes());
    // KSDATAFORMAT_SUBTYPE_PCM
    header.extend_from_slice(&[
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b,
        0x71,
    ]);
    header.extend_from_slice(b"data");
    header.extend_from_slice(&(data_len as u32).to_le_bytes());
    debug_assert_eq!(header.len() as u64, DOP_HEADER_LEN);
    header
}

/// DoP marker for a frame: v1.1 alternates `0x05` and `0xFA`.
pub const fn dop_marker(frame: u64) -> u8 {
    if frame.is_multiple_of(2) { 0x05 } else { 0xfa }
}

/// Reads DSD bytes `[first, first + count)` of one channel, MSB-first.
fn read_channel_bytes<R: Read + Seek>(
    reader: &mut R,
    stream: &DsdInfo,
    channel: u8,
    first: u64,
    count: usize,
) -> Result<Vec<u8>, String> {
    let mut output = vec![0x69_u8; count]; // DSD silence pattern
    let channels = u64::from(stream.channels);
    let bytes_per_channel = stream.bytes_per_channel();
    match stream.container {
        DsdContainer::Dsf => {
            let block = u64::from(stream.block_size);
            let mut index = 0;
            while index < count {
                let byte = first + index as u64;
                if byte >= bytes_per_channel {
                    break;
                }
                let block_index = byte / block;
                let within = byte % block;
                let run = ((block - within) as usize)
                    .min(count - index)
                    .min((bytes_per_channel - byte) as usize);
                let offset = stream.data_offset
                    + block_index * block * channels
                    + u64::from(channel) * block
                    + within;
                reader.seek(SeekFrom::Start(offset)).map_err(io)?;
                reader
                    .read_exact(&mut output[index..index + run])
                    .map_err(io)?;
                index += run;
            }
            if stream.lsb_first {
                for byte in &mut output {
                    *byte = byte.reverse_bits();
                }
            }
        }
        DsdContainer::Dff => {
            let available = bytes_per_channel.saturating_sub(first).min(count as u64) as usize;
            if available > 0 {
                let mut interleaved = vec![0; available * channels as usize];
                reader
                    .seek(SeekFrom::Start(stream.data_offset + first * channels))
                    .map_err(io)?;
                reader.read_exact(&mut interleaved).map_err(io)?;
                for (index, value) in output.iter_mut().take(available).enumerate() {
                    *value = interleaved[index * channels as usize + usize::from(channel)];
                }
            }
        }
    }
    Ok(output)
}

/// Writes DoP frames `[first_frame, first_frame + frames)` as interleaved
/// little-endian 24-bit PCM.
pub fn dop_frames<R: Read + Seek>(
    reader: &mut R,
    stream: &DsdInfo,
    first_frame: u64,
    frames: usize,
) -> Result<Vec<u8>, String> {
    if !supports_dop(stream) {
        return Err("This DSD stream cannot be framed as DoP.".into());
    }
    let frames = frames.min(stream.dop_frames().saturating_sub(first_frame) as usize);
    let channels = usize::from(stream.channels);
    let mut per_channel = Vec::with_capacity(channels);
    for channel in 0..stream.channels {
        per_channel.push(read_channel_bytes(
            reader,
            stream,
            channel,
            first_frame * 2,
            frames * 2,
        )?);
    }
    let mut output = Vec::with_capacity(frames * channels * 3);
    for frame in 0..frames {
        let marker = dop_marker(first_frame + frame as u64);
        for bytes in &per_channel {
            // Little-endian 24-bit word: low = newer DSD byte, mid = older, high = marker.
            output.push(bytes[frame * 2 + 1]);
            output.push(bytes[frame * 2]);
            output.push(marker);
        }
    }
    Ok(output)
}

/// Bytes `[start, end]` of the virtual DoP WAV file.
pub fn dop_wav_range<R: Read + Seek>(
    reader: &mut R,
    stream: &DsdInfo,
    start: u64,
    end: u64,
) -> Result<Vec<u8>, String> {
    let total = dop_wav_length(stream);
    if start > end || start >= total {
        return Ok(Vec::new());
    }
    let end = end.min(total - 1);
    let mut output = Vec::with_capacity((end - start + 1) as usize);
    if start < DOP_HEADER_LEN {
        let header = dop_wav_header(stream);
        output.extend_from_slice(&header[start as usize..=(end.min(DOP_HEADER_LEN - 1) as usize)]);
    }
    if end >= DOP_HEADER_LEN {
        let frame_bytes = u64::from(stream.channels) * 3;
        let data_start = start.max(DOP_HEADER_LEN) - DOP_HEADER_LEN;
        let data_end = end - DOP_HEADER_LEN;
        let first_frame = data_start / frame_bytes;
        let last_frame = data_end / frame_bytes;
        let frames = dop_frames(
            reader,
            stream,
            first_frame,
            (last_frame - first_frame + 1) as usize,
        )?;
        let skip = (data_start - first_frame * frame_bytes) as usize;
        let take = (data_end - data_start + 1) as usize;
        output.extend_from_slice(&frames[skip..skip + take]);
    }
    Ok(output)
}

/// Test helpers that write minimal valid DSD files (ffmpeg cannot mux DSD).
#[cfg(test)]
pub(crate) mod fixtures {
    /// A deterministic pseudo-DSD byte for channel `channel`, byte `index`.
    pub fn pattern(channel: u16, index: u64) -> u8 {
        (index
            .wrapping_mul(31)
            .wrapping_add(u64::from(channel) * 7 + 1)
            & 0xff) as u8
    }

    /// Writes a DSF file holding `pattern` bytes (stored LSB-first) and an optional ID3v2 tag.
    pub fn dsf(sample_rate: u32, channels: u16, samples_per_channel: u64, id3: &[u8]) -> Vec<u8> {
        let block = 4096_u64;
        let bytes_per_channel = samples_per_channel.div_ceil(8);
        let blocks = bytes_per_channel.div_ceil(block);
        let data_len = blocks * block * u64::from(channels);
        let data_chunk = 12 + data_len;
        let metadata_offset = if id3.is_empty() {
            0
        } else {
            28 + 52 + data_chunk
        };
        let file_size = 28 + 52 + data_chunk + id3.len() as u64;
        let mut out = Vec::with_capacity(file_size as usize);
        out.extend_from_slice(b"DSD ");
        out.extend_from_slice(&28_u64.to_le_bytes());
        out.extend_from_slice(&file_size.to_le_bytes());
        out.extend_from_slice(&metadata_offset.to_le_bytes());
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&52_u64.to_le_bytes());
        out.extend_from_slice(&1_u32.to_le_bytes()); // version
        out.extend_from_slice(&0_u32.to_le_bytes()); // raw DSD
        out.extend_from_slice(&(if channels == 1 { 1_u32 } else { 2 }).to_le_bytes());
        out.extend_from_slice(&u32::from(channels).to_le_bytes());
        out.extend_from_slice(&sample_rate.to_le_bytes());
        out.extend_from_slice(&1_u32.to_le_bytes()); // LSB first
        out.extend_from_slice(&samples_per_channel.to_le_bytes());
        out.extend_from_slice(&4096_u32.to_le_bytes());
        out.extend_from_slice(&0_u32.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_chunk.to_le_bytes());
        for block_index in 0..blocks {
            for channel in 0..channels {
                for within in 0..block {
                    let index = block_index * block + within;
                    let value = if index < bytes_per_channel {
                        pattern(channel, index).reverse_bits()
                    } else {
                        0
                    };
                    out.push(value);
                }
            }
        }
        out.extend_from_slice(id3);
        out
    }

    fn chunk(out: &mut Vec<u8>, id: &[u8; 4], body: &[u8]) {
        out.extend_from_slice(id);
        out.extend_from_slice(&(body.len() as u64).to_be_bytes());
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
    }

    /// Writes an uncompressed DSDIFF file with `pattern` bytes and a DIIN title/artist.
    pub fn dff(
        sample_rate: u32,
        channels: u16,
        samples_per_channel: u64,
        title: &str,
        artist: &str,
    ) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(b"DSD ");
        chunk(&mut body, b"FVER", &0x0105_0000_u32.to_be_bytes());
        let mut prop = b"SND ".to_vec();
        chunk(&mut prop, b"FS  ", &sample_rate.to_be_bytes());
        let mut chnl = channels.to_be_bytes().to_vec();
        for channel in 0..channels {
            chnl.extend_from_slice(if channel == 0 { b"SLFT" } else { b"SRGT" });
        }
        chunk(&mut prop, b"CHNL", &chnl);
        let mut cmpr = b"DSD ".to_vec();
        cmpr.push(14);
        cmpr.extend_from_slice(b"not compressed");
        chunk(&mut prop, b"CMPR", &cmpr);
        chunk(&mut body, b"PROP", &prop);
        let bytes_per_channel = samples_per_channel.div_ceil(8);
        let mut data = Vec::with_capacity((bytes_per_channel * u64::from(channels)) as usize);
        for index in 0..bytes_per_channel {
            for channel in 0..channels {
                data.push(pattern(channel, index));
            }
        }
        chunk(&mut body, b"DSD ", &data);
        let mut diin = Vec::new();
        for (id, text) in [(b"DITI", title), (b"DIAR", artist)] {
            let mut value = (text.len() as u32).to_be_bytes().to_vec();
            value.extend_from_slice(text.as_bytes());
            chunk(&mut diin, id, &value);
        }
        chunk(&mut body, b"DIIN", &diin);
        let mut out = Vec::new();
        chunk(&mut out, b"FRM8", &body);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{dff, dsf, pattern};
    use super::*;
    use std::io::Cursor;

    #[test]
    fn dsf_headers_describe_rate_channels_and_length() {
        let bytes = dsf(2_822_400, 2, 2_822_400 / 10, b"ID3\x04\0\0\0\0\0\0");
        let stream = read(&mut Cursor::new(&bytes)).unwrap().info;
        assert_eq!(stream.container, DsdContainer::Dsf);
        assert_eq!(stream.rate, 2_822_400);
        assert_eq!(stream.channels, 2);
        assert!((stream.duration_seconds() - 0.1).abs() < 1e-9);
        assert_eq!(stream.dop_rate(), 176_400);
        assert!(stream.lsb_first);
        assert_eq!(stream.id3.map(|(_, length)| length), Some(10));
        assert_eq!(stream.bitrate_kbps(), 5644);
    }

    #[test]
    fn dff_headers_read_properties_and_diin_text() {
        let bytes = dff(5_644_800, 2, 5_644_800 / 20, "Kind of Blue", "Miles Davis");
        let file = read(&mut Cursor::new(&bytes)).unwrap();
        let stream = file.info;
        assert_eq!(stream.container, DsdContainer::Dff);
        assert_eq!(stream.rate, 5_644_800);
        assert_eq!(stream.channels, 2);
        assert_eq!(stream.samples, 5_644_800 / 20);
        assert_eq!(file.title.as_deref(), Some("Kind of Blue"));
        assert_eq!(file.artist.as_deref(), Some("Miles Davis"));
        assert_eq!(stream.dop_rate(), 352_800);
        assert!(!stream.dst_compressed);
    }

    fn assert_dop_carries_every_bit(bytes: &[u8], stream: &DsdInfo) {
        let mut reader = Cursor::new(bytes);
        let frames = dop_frames(&mut reader, stream, 0, 5000).unwrap();
        let channels = usize::from(stream.channels);
        assert_eq!(frames.len(), 5000 * channels * 3);
        for frame in 0..5000_usize {
            for channel in 0..channels {
                let word = &frames[(frame * channels + channel) * 3..][..3];
                assert_eq!(word[2], if frame % 2 == 0 { 0x05 } else { 0xfa });
                assert_eq!(word[1], pattern(channel as u16, frame as u64 * 2));
                assert_eq!(word[0], pattern(channel as u16, frame as u64 * 2 + 1));
            }
        }
    }

    #[test]
    fn dop_frames_carry_dsf_and_dff_bits_unchanged_msb_first() {
        let samples = 2_822_400 / 10;
        let bytes = dsf(2_822_400, 2, samples, &[]);
        let stream = read(&mut Cursor::new(&bytes)).unwrap().info;
        assert_dop_carries_every_bit(&bytes, &stream);

        let bytes = dff(2_822_400, 2, samples, "t", "a");
        let stream = read(&mut Cursor::new(&bytes)).unwrap().info;
        assert_dop_carries_every_bit(&bytes, &stream);
    }

    #[test]
    fn dop_wav_ranges_match_the_whole_file() {
        let bytes = dsf(2_822_400, 2, 40_000, &[]);
        let stream = read(&mut Cursor::new(&bytes)).unwrap().info;
        let total = dop_wav_length(&stream);
        assert_eq!(total, DOP_HEADER_LEN + 2500 * 2 * 3);
        let whole = dop_wav_range(&mut Cursor::new(&bytes), &stream, 0, total - 1).unwrap();
        assert_eq!(whole.len() as u64, total);
        assert_eq!(&whole[..4], b"RIFF");
        assert_eq!(
            u32::from_le_bytes(whole[24..28].try_into().unwrap()),
            176_400
        );
        for (start, end) in [(0, 10), (60, 80), (100, 4097), (total - 7, total + 50)] {
            let part = dop_wav_range(&mut Cursor::new(&bytes), &stream, start, end).unwrap();
            let end = end.min(total - 1);
            assert_eq!(part, whole[start as usize..=end as usize], "{start}-{end}");
        }
    }

    #[test]
    fn dst_streams_are_not_framed() {
        let bytes = dsf(2_822_400, 2, 8000, &[]);
        let mut stream = read(&mut Cursor::new(&bytes)).unwrap().info;
        stream.dst_compressed = true;
        assert!(dop_frames(&mut Cursor::new(&bytes), &stream, 0, 10).is_err());
    }
}
