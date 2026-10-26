//! Apple Lossless (ALAC) frames as AirPlay carries them: 352 frames of
//! 44.1 kHz/16-bit stereo per packet.
//!
//! Parson sends ALAC's verbatim ("escape") frames: the samples themselves,
//! bit for bit, in a valid ALAC bitstream. Every ALAC decoder (AirPort
//! Express, AV receivers, shairport-sync, ffmpeg) plays them, and nothing
//! is lost. The tests check them against ffmpeg's ALAC decoder.

/// Frames per AirPlay packet (the `fmtp` frame length).
pub const FRAMES_PER_PACKET: usize = 352;

/// The SDP `fmtp` line for 44.1 kHz/16-bit stereo, 352 frames per packet.
pub const FMTP: &str = "96 352 0 16 40 10 14 2 255 0 0 44100";

struct BitWriter {
    bytes: Vec<u8>,
    bits: u32,
}

impl BitWriter {
    fn new(capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(capacity),
            bits: 0,
        }
    }

    /// Writes the low `count` bits of `value`, most significant first.
    fn write(&mut self, value: u32, count: u32) {
        for index in (0..count).rev() {
            let bit = (value >> index) & 1;
            if self.bits.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if bit == 1 {
                let last = self.bytes.len() - 1;
                self.bytes[last] |= 0x80 >> (self.bits % 8);
            }
            self.bits += 1;
        }
    }
}

/// Encodes interleaved little-endian s16 stereo `pcm` (at most 352 frames)
/// as one verbatim ALAC frame.
pub fn encode(pcm: &[u8]) -> Vec<u8> {
    let frames = (pcm.len() / 4).min(FRAMES_PER_PACKET);
    let mut writer = BitWriter::new(frames * 4 + 16);
    // Channel pair element, instance 0, 12 unused bits.
    writer.write(1, 3);
    writer.write(0, 4);
    writer.write(0, 12);
    // A short (last) packet says how many frames it has.
    let partial = frames != FRAMES_PER_PACKET;
    writer.write(u32::from(partial), 1);
    // No shifted bytes; escape (verbatim) samples.
    writer.write(0, 2);
    writer.write(1, 1);
    if partial {
        writer.write(frames as u32, 32);
    }
    for sample in pcm[..frames * 4].as_chunks::<2>().0 {
        let sample = u16::from_le_bytes(*sample);
        writer.write(u32::from(sample), 16);
    }
    // End element.
    writer.write(7, 3);
    writer.bytes
}

/// Decodes a verbatim ALAC frame back to little-endian s16 stereo. A
/// separate implementation of the bitstream, for the fake receiver.
pub fn decode_verbatim(frame: &[u8]) -> Option<Vec<u8>> {
    let mut position = 0_usize;
    let mut read = |count: usize| -> Option<u32> {
        let mut value = 0_u32;
        for _ in 0..count {
            let byte = *frame.get(position / 8)?;
            let bit = (byte >> (7 - position % 8)) & 1;
            value = (value << 1) | u32::from(bit);
            position += 1;
        }
        Some(value)
    };
    if read(3)? != 1 {
        return None;
    }
    let _instance = read(4)?;
    let _unused = read(12)?;
    let partial = read(1)? == 1;
    let shift = read(2)?;
    let escape = read(1)? == 1;
    if !escape || shift != 0 {
        return None;
    }
    let frames = if partial {
        read(32)? as usize
    } else {
        FRAMES_PER_PACKET
    };
    let mut pcm = Vec::with_capacity(frames * 4);
    for _ in 0..frames * 2 {
        let sample = read(16)? as u16;
        pcm.extend_from_slice(&sample.to_le_bytes());
    }
    if read(3)? != 7 {
        return None;
    }
    Some(pcm)
}

/// A CAF file holding ALAC packets (tests decode it with ffmpeg).
#[cfg(test)]
fn caf(packets: &[Vec<u8>], frames: u64) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"caff");
    out.extend_from_slice(&1_u16.to_be_bytes());
    out.extend_from_slice(&0_u16.to_be_bytes());
    // desc
    out.extend_from_slice(b"desc");
    out.extend_from_slice(&32_i64.to_be_bytes());
    out.extend_from_slice(&44_100_f64.to_be_bytes());
    out.extend_from_slice(b"alac");
    out.extend_from_slice(&1_u32.to_be_bytes());
    out.extend_from_slice(&0_u32.to_be_bytes());
    out.extend_from_slice(&(FRAMES_PER_PACKET as u32).to_be_bytes());
    out.extend_from_slice(&2_u32.to_be_bytes());
    out.extend_from_slice(&0_u32.to_be_bytes());
    // kuki: ALACSpecificConfig
    out.extend_from_slice(b"kuki");
    out.extend_from_slice(&24_i64.to_be_bytes());
    out.extend_from_slice(&(FRAMES_PER_PACKET as u32).to_be_bytes());
    out.extend_from_slice(&[0, 16, 40, 10, 14, 2]);
    out.extend_from_slice(&255_u16.to_be_bytes());
    out.extend_from_slice(&0_u32.to_be_bytes());
    out.extend_from_slice(&0_u32.to_be_bytes());
    out.extend_from_slice(&44_100_u32.to_be_bytes());
    // pakt
    let mut table = Vec::new();
    table.extend_from_slice(&(packets.len() as i64).to_be_bytes());
    table.extend_from_slice(&(frames as i64).to_be_bytes());
    table.extend_from_slice(&0_i32.to_be_bytes());
    let remainder = (packets.len() * FRAMES_PER_PACKET) as i64 - frames as i64;
    table.extend_from_slice(&(remainder as i32).to_be_bytes());
    for packet in packets {
        let mut size = packet.len();
        let mut groups = vec![(size & 0x7f) as u8];
        size >>= 7;
        while size > 0 {
            groups.push(((size & 0x7f) as u8) | 0x80);
            size >>= 7;
        }
        groups.reverse();
        table.extend_from_slice(&groups);
    }
    out.extend_from_slice(b"pakt");
    out.extend_from_slice(&(table.len() as i64).to_be_bytes());
    out.extend_from_slice(&table);
    // data
    let data: Vec<u8> = packets.concat();
    out.extend_from_slice(b"data");
    out.extend_from_slice(&((data.len() + 4) as i64).to_be_bytes());
    out.extend_from_slice(&0_u32.to_be_bytes());
    out.extend_from_slice(&data);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(frames: usize) -> Vec<u8> {
        let mut state = 99_u32;
        (0..frames * 2)
            .flat_map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((state >> 16) as u16).to_le_bytes()
            })
            .collect()
    }

    #[test]
    fn verbatim_frames_round_trip_bit_for_bit() {
        let pcm = noise(FRAMES_PER_PACKET);
        let frame = encode(&pcm);
        // 23 header bits + 352 * 2 * 16 sample bits + 3 end bits.
        assert_eq!(frame.len(), (23 + 352 * 32 + 3_usize).div_ceil(8));
        assert_eq!(frame[0] >> 5, 1, "channel pair element");
        assert_eq!(decode_verbatim(&frame).as_deref(), Some(pcm.as_slice()));
        let short = noise(100);
        assert_eq!(
            decode_verbatim(&encode(&short)).as_deref(),
            Some(short.as_slice())
        );
        assert!(decode_verbatim(&[0xff, 0, 0]).is_none());
    }

    #[test]
    fn ffmpegs_alac_decoder_reads_them_unchanged() {
        let available = std::process::Command::new("ffmpeg")
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !available {
            eprintln!("ffmpeg not installed; skipping");
            return;
        }
        let frames = FRAMES_PER_PACKET * 40 + 123;
        let pcm = noise(frames);
        let packets: Vec<Vec<u8>> = pcm.chunks(FRAMES_PER_PACKET * 4).map(encode).collect();
        let directory = std::env::temp_dir().join(format!("parson-alac-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("verbatim.caf");
        std::fs::write(&path, caf(&packets, frames as u64)).unwrap();
        let output = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&path)
            .args(["-f", "s16le", "-"])
            .output()
            .expect("ffmpeg");
        let _ = std::fs::remove_dir_all(&directory);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout.len(), pcm.len());
        assert!(
            output.stdout == pcm,
            "ffmpeg decodes every sample unchanged"
        );
    }
}
