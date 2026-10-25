//! DSD sources and DSD output.
//!
//! A DSD file (DSF or DSDIFF) holds one-bit samples at 2.8224 MHz (DSD64)
//! or a multiple of it. Parson can send them three ways, following the
//! device's "DSD Playback Strategy":
//!
//! - **Native DSD**: the bits go to the driver in a DSD container
//!   (ALSA `DSD_U32_BE` and friends). Nothing is converted.
//! - **DoP v1.0** (DSD over PCM): 16 DSD bits per channel ride in each
//!   24-bit PCM sample, under a marker byte that alternates 0x05 and 0xFA.
//!   The DAC recognises the markers and plays DSD; the bits are untouched,
//!   but a DAC that does not decode DoP plays the carrier as noise, so it
//!   is opt-in.
//! - **Convert to PCM**: the decoder turns DSD into PCM, which can then be
//!   processed. This is Roon's "Conversions for Compatibility".
//!
//! [`DsdReader`] reads a file's bits in time order (first sample in the
//! most significant bit, channels interleaved per byte) and [`DsdPacker`]
//! packs them into the device's frames.

use std::io::{self, Read, Seek, SeekFrom};

use serde::Serialize;

use crate::{
    DSD_FORMATS, DeviceCaps, DsdStrategy, OutputPlan, OutputSettings, PlanError, SampleFormat,
    SourceFormat, dsd_rate_label, plan_output,
};

/// DSD silence: an idle pattern with equal ones and zeros.
pub const DSD_SILENCE: u8 = 0x69;
/// DoP v1.0 marker bytes, alternating frame by frame.
pub const DOP_MARKERS: [u8; 2] = [0x05, 0xFA];

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DsdContainer {
    /// Sony DSD Stream File (`.dsf`): little endian, per-channel blocks,
    /// usually least significant bit first.
    Dsf,
    /// Philips DSDIFF (`.dff`): big endian, channels interleaved per byte,
    /// most significant bit first.
    Dff,
}

/// Where a DSD file's bits are and what they are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DsdInfo {
    pub container: DsdContainer,
    /// One-bit samples per second per channel, e.g. 2 822 400 for DSD64.
    pub rate: u32,
    pub channels: u8,
    /// One-bit samples per channel.
    pub samples: u64,
    /// File offset of the first audio byte.
    pub data_offset: u64,
    /// Audio bytes in the file (all channels, padding included).
    pub data_bytes: u64,
    /// DSF: bytes per channel in each block (4096); DSDIFF: 1.
    pub block_size: u32,
    /// DSF stores the first sample of each byte in its lowest bit.
    pub lsb_first: bool,
    /// File offset of an ID3v2 tag, when there is one.
    pub metadata_offset: Option<u64>,
}

impl DsdInfo {
    /// Bytes of audio per channel (eight samples each).
    pub fn bytes_per_channel(&self) -> u64 {
        self.samples.div_ceil(8)
    }

    pub fn duration_seconds(&self) -> f64 {
        if self.rate == 0 {
            0.0
        } else {
            self.samples as f64 / f64::from(self.rate)
        }
    }

    /// The byte (per channel) at which `position_ms` starts.
    pub fn byte_at_ms(&self, position_ms: u64) -> u64 {
        (u64::from(self.rate / 8) * position_ms / 1000).min(self.bytes_per_channel())
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

fn u32_le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().expect("four bytes"))
}

fn u64_le(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes[..8].try_into().expect("eight bytes"))
}

/// Reads a DSF or DSDIFF header.
pub fn read_info<R: Read + Seek>(reader: &mut R) -> io::Result<DsdInfo> {
    reader.seek(SeekFrom::Start(0))?;
    let mut magic = [0_u8; 4];
    reader.read_exact(&mut magic)?;
    match &magic {
        b"DSD " => read_dsf(reader),
        b"FRM8" => read_dff(reader),
        _ => Err(invalid("not a DSF or DSDIFF file")),
    }
}

fn read_dsf<R: Read + Seek>(reader: &mut R) -> io::Result<DsdInfo> {
    let mut header = [0_u8; 24];
    reader.read_exact(&mut header)?;
    let header_size = u64_le(&header[0..8]);
    let metadata = u64_le(&header[16..24]);
    reader.seek(SeekFrom::Start(header_size.max(28)))?;
    let mut fmt = [0_u8; 12];
    reader.read_exact(&mut fmt)?;
    if &fmt[..4] != b"fmt " {
        return Err(invalid("DSF fmt chunk not found"));
    }
    let fmt_size = u64_le(&fmt[4..12]);
    if fmt_size < 52 {
        return Err(invalid("DSF fmt chunk is too short"));
    }
    let mut body = [0_u8; 40];
    reader.read_exact(&mut body)?;
    let format_id = u32_le(&body[4..8]);
    let channels = u32_le(&body[12..16]);
    let rate = u32_le(&body[16..20]);
    let bits = u32_le(&body[20..24]);
    let samples = u64_le(&body[24..32]);
    let block_size = u32_le(&body[32..36]);
    if format_id != 0 {
        return Err(invalid("DSF file is not raw DSD"));
    }
    if !(1..=8).contains(&channels) || rate == 0 || block_size == 0 || !matches!(bits, 1 | 8) {
        return Err(invalid("DSF fmt chunk has impossible values"));
    }
    let data_start = header_size.max(28) + fmt_size;
    reader.seek(SeekFrom::Start(data_start))?;
    let mut data = [0_u8; 12];
    reader.read_exact(&mut data)?;
    if &data[..4] != b"data" {
        return Err(invalid("DSF data chunk not found"));
    }
    let data_bytes = u64_le(&data[4..12]).saturating_sub(12);
    Ok(DsdInfo {
        container: DsdContainer::Dsf,
        rate,
        channels: channels as u8,
        samples,
        data_offset: data_start + 12,
        data_bytes,
        block_size,
        lsb_first: bits == 1,
        metadata_offset: (metadata > 0).then_some(metadata),
    })
}

fn read_chunk<R: Read>(reader: &mut R) -> io::Result<([u8; 4], u64)> {
    let mut header = [0_u8; 12];
    reader.read_exact(&mut header)?;
    let id = header[..4].try_into().expect("four bytes");
    let size = u64::from_be_bytes(header[4..12].try_into().expect("eight bytes"));
    Ok((id, size))
}

fn read_dff<R: Read + Seek>(reader: &mut R) -> io::Result<DsdInfo> {
    let mut header = [0_u8; 12];
    reader.read_exact(&mut header)?;
    if &header[8..12] != b"DSD " {
        return Err(invalid("not a DSDIFF DSD file"));
    }
    let form_end = 12 + u64::from_be_bytes(header[..8].try_into().expect("eight bytes"));
    let mut rate = 0;
    let mut channels = 0_u16;
    let mut data = None;
    let mut metadata = None;
    let mut position = 16_u64;
    while position + 12 <= form_end {
        reader.seek(SeekFrom::Start(position))?;
        let Ok((id, size)) = read_chunk(reader) else {
            break;
        };
        let body = position + 12;
        match &id {
            b"PROP" => {
                let mut kind = [0_u8; 4];
                reader.read_exact(&mut kind)?;
                if &kind != b"SND " {
                    return Err(invalid("DSDIFF PROP chunk is not SND"));
                }
                let mut sub = body + 4;
                while sub + 12 <= body + size {
                    reader.seek(SeekFrom::Start(sub))?;
                    let (sub_id, sub_size) = read_chunk(reader)?;
                    match &sub_id {
                        b"FS  " => {
                            let mut value = [0_u8; 4];
                            reader.read_exact(&mut value)?;
                            rate = u32::from_be_bytes(value);
                        }
                        b"CHNL" => {
                            let mut value = [0_u8; 2];
                            reader.read_exact(&mut value)?;
                            channels = u16::from_be_bytes(value);
                        }
                        b"CMPR" => {
                            let mut value = [0_u8; 4];
                            reader.read_exact(&mut value)?;
                            if &value != b"DSD " {
                                return Err(invalid(
                                    "compressed DSDIFF (DST) is not supported for DSD output",
                                ));
                            }
                        }
                        _ => {}
                    }
                    sub += 12 + sub_size + (sub_size & 1);
                }
            }
            b"DSD " => data = Some((body, size)),
            b"DST " => {
                return Err(invalid(
                    "compressed DSDIFF (DST) is not supported for DSD output",
                ));
            }
            b"ID3 " => metadata = Some(body),
            _ => {}
        }
        position = body + size + (size & 1);
    }
    let (data_offset, data_bytes) = data.ok_or_else(|| invalid("DSDIFF DSD chunk not found"))?;
    if rate == 0 || !(1..=8).contains(&channels) {
        return Err(invalid("DSDIFF properties are missing or impossible"));
    }
    Ok(DsdInfo {
        container: DsdContainer::Dff,
        rate,
        channels: channels as u8,
        samples: data_bytes / u64::from(channels) * 8,
        data_offset,
        data_bytes,
        block_size: 1,
        lsb_first: false,
        metadata_offset: metadata,
    })
}

/// A DSD file's bits in time order: one byte per channel at a time,
/// channels interleaved, first sample in the most significant bit.
pub struct DsdReader<R> {
    inner: R,
    info: DsdInfo,
    /// Next byte (per channel) to deliver.
    position: u64,
    buffer: Vec<u8>,
    offset: usize,
    block: Vec<u8>,
}

impl<R: Read + Seek> DsdReader<R> {
    /// Starts at `start_byte` bytes (per channel) into the audio.
    pub fn new(inner: R, info: DsdInfo, start_byte: u64) -> Self {
        Self {
            inner,
            info,
            position: start_byte.min(info.bytes_per_channel()),
            buffer: Vec::new(),
            offset: 0,
            block: Vec::new(),
        }
    }

    pub fn info(&self) -> DsdInfo {
        self.info
    }

    fn fill(&mut self) -> io::Result<()> {
        self.buffer.clear();
        self.offset = 0;
        let end = self.info.bytes_per_channel();
        if self.position >= end {
            return Ok(());
        }
        let channels = usize::from(self.info.channels);
        match self.info.container {
            DsdContainer::Dsf => {
                let block = u64::from(self.info.block_size);
                let group = self.position / block;
                let skip = (self.position % block) as usize;
                let take = (block.min(end - group * block) as usize).saturating_sub(skip);
                self.block.resize(block as usize * channels, 0);
                self.inner.seek(SeekFrom::Start(
                    self.info.data_offset + group * block * channels as u64,
                ))?;
                read_up_to(&mut self.inner, &mut self.block)?;
                self.buffer.reserve(take * channels);
                for index in skip..skip + take {
                    for channel in 0..channels {
                        let byte = self.block[channel * block as usize + index];
                        self.buffer.push(if self.info.lsb_first {
                            byte.reverse_bits()
                        } else {
                            byte
                        });
                    }
                }
                self.position += take as u64;
            }
            DsdContainer::Dff => {
                let take = (end - self.position).min(16_384) as usize;
                self.buffer.resize(take * channels, 0);
                self.inner.seek(SeekFrom::Start(
                    self.info.data_offset + self.position * channels as u64,
                ))?;
                let read = read_up_to(&mut self.inner, &mut self.buffer)?;
                self.buffer.truncate(read / channels * channels);
                self.position += (read / channels) as u64;
                if read == 0 {
                    self.position = end;
                }
            }
        }
        Ok(())
    }
}

fn read_up_to<R: Read>(reader: &mut R, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

impl<R: Read + Seek> Read for DsdReader<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.offset >= self.buffer.len() {
            self.fill()?;
        }
        let available = &self.buffer[self.offset..];
        let count = available.len().min(out.len());
        out[..count].copy_from_slice(&available[..count]);
        self.offset += count;
        Ok(count)
    }
}

/// How DSD bytes are laid out for the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DsdPacking {
    /// A native DSD container (one of [`DSD_FORMATS`]).
    Native(SampleFormat),
    /// DoP in a 24-bit capable PCM container.
    Dop(SampleFormat),
}

impl DsdPacking {
    /// DSD bytes per channel in one device frame.
    pub fn bytes_per_frame(self) -> usize {
        match self {
            Self::Native(format) => format.bytes(),
            Self::Dop(_) => 2,
        }
    }

    fn container(self) -> SampleFormat {
        match self {
            Self::Native(format) | Self::Dop(format) => format,
        }
    }
}

/// Packs interleaved DSD bytes (`channels` per time step) into device
/// frames. `input` holds whole frames; `marker` carries the DoP marker
/// phase from one call to the next.
pub fn pack_dsd(
    input: &[u8],
    channels: usize,
    packing: DsdPacking,
    marker: &mut usize,
    output: &mut Vec<u8>,
) {
    let per = packing.bytes_per_frame();
    let frame_in = per * channels;
    output.clear();
    output.reserve(input.len() / frame_in.max(1) * channels * packing.container().bytes());
    for frame in input.chunks_exact(frame_in) {
        let byte = |time: usize, channel: usize| frame[time * channels + channel];
        match packing {
            DsdPacking::Native(SampleFormat::DsdU16Le | SampleFormat::DsdU32Le) => {
                for channel in 0..channels {
                    for time in (0..per).rev() {
                        output.push(byte(time, channel));
                    }
                }
            }
            DsdPacking::Native(_) => {
                for channel in 0..channels {
                    for time in 0..per {
                        output.push(byte(time, channel));
                    }
                }
            }
            DsdPacking::Dop(container) => {
                let mark = DOP_MARKERS[*marker & 1];
                for channel in 0..channels {
                    let (first, second) = (byte(0, channel), byte(1, channel));
                    match container {
                        SampleFormat::S24In32 => {
                            let sign = if mark & 0x80 != 0 { 0xFF } else { 0 };
                            output.extend_from_slice(&[second, first, mark, sign]);
                        }
                        SampleFormat::S32 => output.extend_from_slice(&[0, second, first, mark]),
                        _ => output.extend_from_slice(&[second, first, mark]),
                    }
                }
                *marker += 1;
            }
        }
    }
}

/// Recovers the DSD bytes from DoP frames, or `None` when the markers are
/// missing or do not alternate (the stream is not DoP, or it was altered
/// on the way).
pub fn unpack_dop(frames: &[u8], channels: usize, container: SampleFormat) -> Option<Vec<u8>> {
    let width = container.bytes();
    let (marker_at, first_at, second_at) = match container {
        SampleFormat::S32 => (3, 2, 1),
        SampleFormat::S24Packed | SampleFormat::S24In32 => (2, 1, 0),
        _ => return None,
    };
    let mut out = Vec::with_capacity(frames.len() / width * 2);
    let mut previous: Option<u8> = None;
    for frame in frames.chunks_exact(width * channels) {
        let mark = frame[marker_at];
        if !DOP_MARKERS.contains(&mark) || previous == Some(mark) {
            return None;
        }
        previous = Some(mark);
        let mut seconds = Vec::with_capacity(channels);
        for channel in 0..channels {
            let sample = &frame[channel * width..(channel + 1) * width];
            if sample[marker_at] != mark {
                return None;
            }
            out.push(sample[first_at]);
            seconds.push(sample[second_at]);
        }
        out.extend_from_slice(&seconds);
    }
    Some(out)
}

/// DSD bytes packed for the device as a byte stream the player copies
/// unchanged ([`crate::player::InputFormat::Raw`]).
pub struct DsdPcm<R> {
    inner: R,
    channels: usize,
    packing: DsdPacking,
    marker: usize,
    raw: Vec<u8>,
    packed: Vec<u8>,
    offset: usize,
    done: bool,
}

impl<R: Read> DsdPcm<R> {
    pub fn new(inner: R, channels: u8, packing: DsdPacking) -> Self {
        Self {
            inner,
            channels: usize::from(channels.max(1)),
            packing,
            marker: 0,
            raw: Vec::new(),
            packed: Vec::new(),
            offset: 0,
            done: false,
        }
    }
}

impl<R: Read> Read for DsdPcm<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.offset >= self.packed.len() {
            if self.done {
                return Ok(0);
            }
            let frame_in = self.packing.bytes_per_frame() * self.channels;
            self.raw.resize(frame_in * 2_048, 0);
            let read = read_up_to(&mut self.inner, &mut self.raw)?;
            if read < self.raw.len() {
                self.done = true;
            }
            self.raw.truncate(read);
            // The last partial frame is completed with DSD silence.
            let partial = read % frame_in;
            if partial != 0 {
                self.raw.resize(read + frame_in - partial, DSD_SILENCE);
            }
            pack_dsd(
                &self.raw,
                self.channels,
                self.packing,
                &mut self.marker,
                &mut self.packed,
            );
            self.offset = 0;
            if self.packed.is_empty() {
                return Ok(0);
            }
        }
        let available = &self.packed[self.offset..];
        let count = available.len().min(out.len());
        out[..count].copy_from_slice(&available[..count]);
        self.offset += count;
        Ok(count)
    }
}

/// How a DSD track reaches the device.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DsdMode {
    Native,
    Dop,
    /// Converted to PCM by the decoder.
    Pcm,
}

/// The DSD part of an [`OutputPlan`].
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct DsdPlan {
    pub mode: DsdMode,
    /// The source's DSD rate, e.g. 2 822 400.
    pub dsd_rate: u32,
    /// e.g. `DSD64`.
    pub dsd_label: String,
    pub strategy: DsdStrategy,
    /// The native container (e.g. `DSD_U32_BE`) or DoP carrier container.
    pub container: Option<&'static str>,
    /// The device frame rate carrying the bits (88.2 kHz for DSD64 in
    /// `DSD_U32_BE`, 176.4 kHz for DSD64 over DoP).
    pub frame_rate: Option<u32>,
    /// The rate the decoder converts DSD to before any further conversion
    /// (the DSD rate over 8), for [`DsdMode::Pcm`].
    pub pcm_rate: Option<u32>,
    /// Why DSD was converted when the strategy asked for DSD.
    pub fallback_reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DsdSource {
    pub rate: u32,
    pub channels: u8,
}

/// The rate ffmpeg's DSD decoders produce PCM at.
pub fn pcm_rate(dsd_rate: u32) -> u32 {
    dsd_rate / 8
}

/// The native container the device takes `dsd_rate` in, if any.
pub fn native_format(caps: &DeviceCaps, dsd_rate: u32) -> Option<SampleFormat> {
    if !caps.dsd_rates.contains(&dsd_rate) {
        return None;
    }
    DSD_FORMATS
        .iter()
        .copied()
        .find(|format| caps.dsd_formats.contains(format))
}

/// The PCM carrier rate and container a DoP stream of `dsd_rate` needs,
/// if the device accepts it as PCM.
pub fn dop_carrier(caps: &DeviceCaps, dsd_rate: u32) -> Option<(u32, SampleFormat)> {
    let rate = dsd_rate / 16;
    if !caps.sample_rates.contains(&rate) {
        return None;
    }
    [
        SampleFormat::S24Packed,
        SampleFormat::S24In32,
        SampleFormat::S32,
    ]
    .into_iter()
    .find(|format| caps.sample_formats.contains(format))
    .map(|format| (rate, format))
}

/// Decides how a DSD track goes to a device. Native and DoP plans are
/// bit-perfect; otherwise the track is converted to PCM and planned like
/// any PCM source (never bit-perfect). `force_pcm` carries a reason the
/// caller needs PCM, such as DSP the listener turned on.
pub fn plan_dsd(
    source: DsdSource,
    caps: Option<&DeviceCaps>,
    settings: &OutputSettings,
    force_pcm: Option<&str>,
) -> Result<OutputPlan, PlanError> {
    let caps = caps.ok_or(PlanError::NotProbed)?;
    let label = dsd_rate_label(source.rate);
    let channels_fit =
        (caps.min_channels.max(1)..=caps.max_channels.max(1)).contains(&source.channels.max(1));
    let mut reason = None;
    let mut chosen: Option<(DsdMode, SampleFormat, u32)> = None;
    if let Some(why) = force_pcm {
        reason = Some(why.to_string());
    } else if !settings.exclusive {
        reason = Some("DSD needs Exclusive Mode, which is off for this device".into());
    } else if settings.max_dsd_rate.is_some_and(|max| source.rate > max) {
        reason = Some(format!(
            "{label} is above the max DSD rate set for this device"
        ));
    } else if !channels_fit {
        reason = Some(format!(
            "the device does not take {} channels",
            source.channels
        ));
    } else {
        match settings.dsd_strategy {
            DsdStrategy::Pcm => {}
            DsdStrategy::Auto | DsdStrategy::Native => {
                if let Some(format) = native_format(caps, source.rate) {
                    chosen = Some((DsdMode::Native, format, format.dsd_frame_rate(source.rate)));
                } else {
                    let dop = dop_carrier(caps, source.rate).is_some();
                    reason = Some(if dop && settings.dsd_strategy == DsdStrategy::Auto {
                        format!(
                            "the driver does not report native {label}; choose DoP if the DAC decodes DSD over PCM"
                        )
                    } else {
                        format!("the driver does not report native {label}")
                    });
                }
            }
            DsdStrategy::Dop => {
                if let Some((rate, format)) = dop_carrier(caps, source.rate) {
                    chosen = Some((DsdMode::Dop, format, rate));
                } else {
                    reason = Some(format!(
                        "{label} over DoP needs a {:.1} kHz 24-bit PCM rate the device does not accept",
                        f64::from(source.rate / 16) / 1000.0
                    ));
                }
            }
        }
    }
    if let Some((mode, format, frame_rate)) = chosen {
        return Ok(OutputPlan {
            sample_rate: frame_rate,
            sample_format: format,
            bit_depth: if mode == DsdMode::Dop { 24 } else { 1 },
            channels: source.channels,
            resample: None,
            depth: crate::DepthChange::None,
            channel_change: None,
            exclusive: true,
            bit_perfect: true,
            dsd: Some(DsdPlan {
                mode,
                dsd_rate: source.rate,
                dsd_label: label,
                strategy: settings.dsd_strategy,
                container: Some(format.label()),
                frame_rate: Some(frame_rate),
                pcm_rate: None,
                fallback_reason: None,
            }),
        });
    }
    let pcm = SourceFormat {
        sample_rate: pcm_rate(source.rate),
        bit_depth: None,
        channels: source.channels,
        lossless: true,
    };
    let mut plan = plan_output(pcm, Some(caps), settings)?;
    plan.bit_perfect = false;
    plan.dsd = Some(DsdPlan {
        mode: DsdMode::Pcm,
        dsd_rate: source.rate,
        dsd_label: label,
        strategy: settings.dsd_strategy,
        container: None,
        frame_rate: None,
        pcm_rate: Some(pcm.sample_rate),
        fallback_reason: reason,
    });
    Ok(plan)
}

/// The PCM source a DSD track becomes when converted.
pub fn pcm_source(source: DsdSource) -> SourceFormat {
    SourceFormat {
        sample_rate: pcm_rate(source.rate),
        bit_depth: None,
        channels: source.channels,
        lossless: true,
    }
}

/// Writes a minimal DSF file for tests and demonstrations: `bytes` holds
/// interleaved MSB-first DSD bytes as [`DsdReader`] returns them.
pub fn write_dsf(rate: u32, channels: u8, bytes: &[u8]) -> Vec<u8> {
    let channels_n = usize::from(channels);
    let per_channel = bytes.len() / channels_n;
    let block = 4_096;
    let groups = per_channel.div_ceil(block);
    let data_len = groups * block * channels_n;
    let mut out = Vec::with_capacity(92 + data_len);
    let total = (28 + 52 + 12 + data_len) as u64;
    out.extend_from_slice(b"DSD ");
    out.extend_from_slice(&28_u64.to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&0_u64.to_le_bytes());
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&52_u64.to_le_bytes());
    out.extend_from_slice(&1_u32.to_le_bytes());
    out.extend_from_slice(&0_u32.to_le_bytes());
    out.extend_from_slice(&(if channels == 1 { 1_u32 } else { 2 }).to_le_bytes());
    out.extend_from_slice(&u32::from(channels).to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&1_u32.to_le_bytes());
    out.extend_from_slice(&(per_channel as u64 * 8).to_le_bytes());
    out.extend_from_slice(&(block as u32).to_le_bytes());
    out.extend_from_slice(&0_u32.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&((12 + data_len) as u64).to_le_bytes());
    for group in 0..groups {
        for channel in 0..channels_n {
            for index in 0..block {
                let time = group * block + index;
                out.push(if time < per_channel {
                    bytes[time * channels_n + channel].reverse_bits()
                } else {
                    0
                });
            }
        }
    }
    out
}

/// Writes a minimal DSDIFF file, as [`write_dsf`] does.
pub fn write_dff(rate: u32, channels: u8, bytes: &[u8]) -> Vec<u8> {
    let mut prop = Vec::new();
    prop.extend_from_slice(b"SND ");
    prop.extend_from_slice(b"FS  ");
    prop.extend_from_slice(&4_u64.to_be_bytes());
    prop.extend_from_slice(&rate.to_be_bytes());
    prop.extend_from_slice(b"CHNL");
    let chnl_len = 2 + 4 * u64::from(channels);
    prop.extend_from_slice(&chnl_len.to_be_bytes());
    prop.extend_from_slice(&u16::from(channels).to_be_bytes());
    for index in 0..channels {
        prop.extend_from_slice(match index {
            0 => b"SLFT",
            1 => b"SRGT",
            _ => b"C   ",
        });
    }
    prop.extend_from_slice(b"CMPR");
    prop.extend_from_slice(&20_u64.to_be_bytes());
    prop.extend_from_slice(b"DSD ");
    prop.push(14);
    prop.extend_from_slice(b"not compressed");
    prop.push(0);
    let mut body = Vec::new();
    body.extend_from_slice(b"DSD ");
    body.extend_from_slice(b"FVER");
    body.extend_from_slice(&4_u64.to_be_bytes());
    body.extend_from_slice(&0x0105_0000_u32.to_be_bytes());
    body.extend_from_slice(b"PROP");
    body.extend_from_slice(&(prop.len() as u64).to_be_bytes());
    body.extend_from_slice(&prop);
    body.extend_from_slice(b"DSD ");
    body.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    body.extend_from_slice(bytes);
    if bytes.len() % 2 == 1 {
        body.push(0);
    }
    let mut out = Vec::with_capacity(12 + body.len());
    out.extend_from_slice(b"FRM8");
    out.extend_from_slice(&(body.len() as u64).to_be_bytes());
    out.extend_from_slice(&body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Interleaved stereo DSD bytes with a different pattern per channel.
    fn pattern(bytes_per_channel: usize) -> Vec<u8> {
        (0..bytes_per_channel)
            .flat_map(|n| [(n * 37 % 251) as u8, (n * 91 % 241 + 3) as u8])
            .collect()
    }

    fn read_all<R: Read + Seek>(file: R, start: u64) -> (DsdInfo, Vec<u8>) {
        let mut file = file;
        let info = read_info(&mut file).unwrap();
        let mut out = Vec::new();
        DsdReader::new(file, info, start)
            .read_to_end(&mut out)
            .unwrap();
        (info, out)
    }

    #[test]
    fn dsf_bits_come_back_in_time_order() {
        // Three blocks and a bit, so the last group is partial.
        let bytes = pattern(4_096 * 3 + 100);
        let file = write_dsf(2_822_400, 2, &bytes);
        let (info, out) = read_all(Cursor::new(file), 0);
        assert_eq!(info.container, DsdContainer::Dsf);
        assert_eq!(info.rate, 2_822_400);
        assert_eq!(info.channels, 2);
        assert_eq!(info.bytes_per_channel(), 4_096 * 3 + 100);
        assert!(info.lsb_first);
        assert_eq!(out, bytes);
    }

    #[test]
    fn dff_bits_come_back_in_time_order() {
        let bytes = pattern(20_001);
        let file = write_dff(5_644_800, 2, &bytes);
        let (info, out) = read_all(Cursor::new(file), 0);
        assert_eq!(info.container, DsdContainer::Dff);
        assert_eq!(info.rate, 5_644_800);
        assert_eq!(dsd_rate_label(info.rate), "DSD128");
        assert_eq!(out, bytes);
    }

    #[test]
    fn reading_can_start_mid_file() {
        let bytes = pattern(10_000);
        for file in [
            write_dsf(2_822_400, 2, &bytes),
            write_dff(2_822_400, 2, &bytes),
        ] {
            let (_, out) = read_all(Cursor::new(file), 5_000);
            assert_eq!(out, bytes[10_000..]);
        }
        let info = read_info(&mut Cursor::new(write_dsf(2_822_400, 2, &bytes))).unwrap();
        assert_eq!(info.byte_at_ms(1_000), 10_000);
        assert_eq!(info.byte_at_ms(10), 3_528);
    }

    #[test]
    fn dst_and_garbage_are_refused() {
        assert!(read_info(&mut Cursor::new(b"RIFF....WAVE".to_vec())).is_err());
        let mut dst = write_dff(2_822_400, 2, &pattern(16));
        let at = dst.windows(4).position(|w| w == b"CMPR").unwrap() + 12;
        dst[at..at + 4].copy_from_slice(b"DST ");
        assert!(read_info(&mut Cursor::new(dst)).is_err());
    }

    #[test]
    fn native_containers_keep_time_order_per_channel() {
        // Two channels, four time steps: L0 R0 L1 R1 L2 R2 L3 R3.
        let input = [0x10, 0x20, 0x11, 0x21, 0x12, 0x22, 0x13, 0x23];
        let mut out = Vec::new();
        let mut marker = 0;
        pack_dsd(
            &input,
            2,
            DsdPacking::Native(SampleFormat::DsdU32Be),
            &mut marker,
            &mut out,
        );
        assert_eq!(out, [0x10, 0x11, 0x12, 0x13, 0x20, 0x21, 0x22, 0x23]);
        pack_dsd(
            &input,
            2,
            DsdPacking::Native(SampleFormat::DsdU32Le),
            &mut marker,
            &mut out,
        );
        assert_eq!(out, [0x13, 0x12, 0x11, 0x10, 0x23, 0x22, 0x21, 0x20]);
        pack_dsd(
            &input,
            2,
            DsdPacking::Native(SampleFormat::DsdU8),
            &mut marker,
            &mut out,
        );
        assert_eq!(out, input);
        pack_dsd(
            &input,
            2,
            DsdPacking::Native(SampleFormat::DsdU16Le),
            &mut marker,
            &mut out,
        );
        assert_eq!(out, [0x11, 0x10, 0x21, 0x20, 0x13, 0x12, 0x23, 0x22]);
    }

    #[test]
    fn dop_frames_carry_alternating_markers_and_round_trip() {
        let bytes = pattern(1_000);
        for container in [
            SampleFormat::S24Packed,
            SampleFormat::S24In32,
            SampleFormat::S32,
        ] {
            let mut out = Vec::new();
            let mut marker = 0;
            pack_dsd(&bytes, 2, DsdPacking::Dop(container), &mut marker, &mut out);
            assert_eq!(out.len(), 500 * 2 * container.bytes());
            let top = if container == SampleFormat::S32 { 3 } else { 2 };
            assert_eq!(out[top], 0x05);
            assert_eq!(out[container.bytes() * 2 + top], 0xFA);
            assert_eq!(unpack_dop(&out, 2, container).unwrap(), bytes);
        }
        // As a 24-bit PCM sample, DoP's first frame of L0=0xAB, L1=0xCD
        // is 0x05ABCD.
        let mut out = Vec::new();
        pack_dsd(
            &[0xAB, 0, 0xCD, 0],
            2,
            DsdPacking::Dop(SampleFormat::S24Packed),
            &mut 0,
            &mut out,
        );
        assert_eq!(&out[..3], &[0xCD, 0xAB, 0x05]);
        // A stream that lost its markers (resampled or attenuated) is not
        // DoP any more.
        let mut broken = out.clone();
        broken[2] = 0x04;
        assert_eq!(unpack_dop(&broken, 2, SampleFormat::S24Packed), None);
    }

    #[test]
    fn the_packer_streams_a_file_and_pads_with_silence() {
        let bytes = pattern(4_097);
        let file = write_dsf(2_822_400, 2, &bytes);
        let mut cursor = Cursor::new(file);
        let info = read_info(&mut cursor).unwrap();
        let mut out = Vec::new();
        DsdPcm::new(
            DsdReader::new(cursor, info, 0),
            2,
            DsdPacking::Native(SampleFormat::DsdU32Be),
        )
        .read_to_end(&mut out)
        .unwrap();
        // 4097 bytes per channel round up to 1025 four-byte words.
        assert_eq!(out.len(), 1_025 * 4 * 2);
        let last = &out[out.len() - 8..];
        assert_eq!(last[1..4], [DSD_SILENCE; 3]);
        assert_eq!(last[5..8], [DSD_SILENCE; 3]);
    }

    fn dac(dsd: bool) -> DeviceCaps {
        DeviceCaps {
            sample_rates: vec![44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800],
            sample_formats: vec![SampleFormat::S16, SampleFormat::S24Packed],
            min_channels: 2,
            max_channels: 2,
            dsd_formats: if dsd {
                vec![SampleFormat::DsdU32Be]
            } else {
                Vec::new()
            },
            dsd_rates: if dsd {
                vec![2_822_400, 5_644_800]
            } else {
                Vec::new()
            },
        }
    }

    const DSD64: DsdSource = DsdSource {
        rate: 2_822_400,
        channels: 2,
    };

    #[test]
    fn native_dsd_is_bit_perfect_at_the_container_rate() {
        let plan = plan_dsd(DSD64, Some(&dac(true)), &OutputSettings::default(), None).unwrap();
        let dsd = plan.dsd.as_ref().unwrap();
        assert_eq!(dsd.mode, DsdMode::Native);
        assert_eq!(dsd.container, Some("DSD_U32_BE"));
        assert_eq!(plan.sample_rate, 88_200);
        assert_eq!(plan.sample_format, SampleFormat::DsdU32Be);
        assert!(plan.bit_perfect);
    }

    #[test]
    fn auto_never_guesses_dop_but_dop_can_be_chosen() {
        let plan = plan_dsd(DSD64, Some(&dac(false)), &OutputSettings::default(), None).unwrap();
        let dsd = plan.dsd.as_ref().unwrap();
        assert_eq!(dsd.mode, DsdMode::Pcm);
        assert!(dsd.fallback_reason.as_deref().unwrap().contains("DoP"));
        // Roon's DSD64 to 88.2 kHz for a DAC without DSD.
        let capped = OutputSettings {
            max_sample_rate: Some(96_000),
            ..OutputSettings::default()
        };
        let plan = plan_dsd(DSD64, Some(&dac(false)), &capped, None).unwrap();
        assert_eq!(plan.resample, Some((352_800, 88_200)));
        assert_eq!(plan.bit_depth, 24);
        assert!(!plan.bit_perfect);
        let dop = OutputSettings {
            dsd_strategy: DsdStrategy::Dop,
            ..OutputSettings::default()
        };
        let plan = plan_dsd(DSD64, Some(&dac(false)), &dop, None).unwrap();
        assert_eq!(plan.dsd.as_ref().unwrap().mode, DsdMode::Dop);
        assert_eq!(plan.sample_rate, 176_400);
        assert_eq!(plan.sample_format, SampleFormat::S24Packed);
        assert!(plan.bit_perfect);
        // DSD256 over DoP needs 705.6 kHz, which this DAC lacks.
        let plan = plan_dsd(
            DsdSource {
                rate: 11_289_600,
                channels: 2,
            },
            Some(&dac(false)),
            &dop,
            None,
        )
        .unwrap();
        assert_eq!(plan.dsd.as_ref().unwrap().mode, DsdMode::Pcm);
        assert!(!plan.bit_perfect);
    }

    #[test]
    fn settings_and_dsp_force_pcm() {
        let caps = dac(true);
        let shared = OutputSettings {
            exclusive: false,
            ..OutputSettings::default()
        };
        let plan = plan_dsd(DSD64, Some(&caps), &shared, None).unwrap();
        assert_eq!(plan.dsd.as_ref().unwrap().mode, DsdMode::Pcm);
        let capped = OutputSettings {
            max_dsd_rate: Some(2_822_400),
            ..OutputSettings::default()
        };
        let dsd128 = DsdSource {
            rate: 5_644_800,
            channels: 2,
        };
        let plan = plan_dsd(dsd128, Some(&caps), &capped, None).unwrap();
        assert!(
            plan.dsd
                .as_ref()
                .unwrap()
                .fallback_reason
                .as_deref()
                .unwrap()
                .contains("max DSD rate")
        );
        let plan = plan_dsd(
            DSD64,
            Some(&caps),
            &OutputSettings::default(),
            Some("the DSP engine needs PCM"),
        )
        .unwrap();
        assert_eq!(plan.dsd.as_ref().unwrap().mode, DsdMode::Pcm);
        let pcm = OutputSettings {
            dsd_strategy: DsdStrategy::Pcm,
            ..OutputSettings::default()
        };
        let plan = plan_dsd(DSD64, Some(&caps), &pcm, None).unwrap();
        assert_eq!(plan.dsd.as_ref().unwrap().fallback_reason, None);
        assert_eq!(plan.resample, None);
        assert!(!plan.bit_perfect);
    }

    #[test]
    fn format_support_lists_native_and_dop_rates() {
        let support = crate::format_support(&dac(true), &OutputSettings::default());
        let native: Vec<u32> = support
            .dsd_rates
            .iter()
            .filter(|entry| entry.supported)
            .map(|entry| entry.value)
            .collect();
        assert_eq!(native, [2_822_400, 5_644_800]);
        // 176.4k and 352.8k carriers: DSD64 and DSD128 over DoP.
        assert_eq!(support.dop_rates, [2_822_400, 5_644_800]);
    }
}
