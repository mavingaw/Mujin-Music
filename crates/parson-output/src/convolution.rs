//! Convolution: FIR filters such as room correction, run by the server's
//! DSP engine in 64-bit float.
//!
//! A filter is an impulse response in a WAV file (16/24/32-bit integer or
//! 32/64-bit float, mono or one channel per output channel), as room
//! correction tools (REW, Dirac export, DRC-FIR, Acourate) write them.
//! When the filter's rate differs from the audio's, the impulse response
//! is converted with the engine's own linear-phase resampler and scaled by
//! the rate ratio, so the filter's frequency response is kept.
//!
//! The convolver is uniformly partitioned overlap-save: the impulse
//! response is cut into blocks of [`BLOCK`] samples, each transformed
//! once; every input block is transformed, multiplied with the delay line
//! of past spectra and transformed back. Output sample `n` is exactly the
//! linear convolution at `n`: no latency is added to the timeline.

use std::sync::Arc;

use serde::Serialize;

use crate::dsp::Resampler;

/// Samples per partition (the FFT is twice this).
pub const BLOCK: usize = 2048;
/// Longest impulse response accepted, in samples (about 5.5 s at 96 kHz).
pub const MAX_TAPS: usize = 1 << 19;
/// Most channels a filter may have.
pub const MAX_CHANNELS: usize = 8;

/// A decoded impulse response, planar.
#[derive(Clone, Debug, PartialEq)]
pub struct ImpulseResponse {
    pub sample_rate: u32,
    pub channels: Vec<Vec<f64>>,
}

/// What the API shows about a stored filter.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FilterSummary {
    pub sample_rate: u32,
    pub channels: u8,
    pub taps: usize,
    pub duration_ms: f64,
    /// Highest gain of the filter's frequency response, in dB, across its
    /// channels: how much headroom it needs not to clip.
    pub peak_gain_db: f64,
    /// Gain at 1 kHz, for a sense of the filter's overall level.
    pub gain_1khz_db: f64,
    /// Sample format of the file, e.g. `32-bit float`.
    pub format: String,
}

fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// Parses a WAV impulse response.
pub fn parse_wav(bytes: &[u8]) -> Result<(ImpulseResponse, String), String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("The filter must be a WAV file (RIFF/WAVE).".into());
    }
    let mut position = 12;
    let mut format: Option<(u16, u16, u32, u16)> = None;
    let mut data: Option<&[u8]> = None;
    while position + 8 <= bytes.len() {
        let id = &bytes[position..position + 4];
        let size = read_u32(bytes, position + 4).unwrap_or(0) as usize;
        let body_start = position + 8;
        let body_end = body_start.saturating_add(size).min(bytes.len());
        let body = &bytes[body_start..body_end];
        match id {
            b"fmt " => {
                let mut tag = read_u16(body, 0).ok_or("The WAV format chunk is too short.")?;
                let channels = read_u16(body, 2).ok_or("The WAV format chunk is too short.")?;
                let rate = read_u32(body, 4).ok_or("The WAV format chunk is too short.")?;
                let bits = read_u16(body, 14).ok_or("The WAV format chunk is too short.")?;
                if tag == 0xFFFE {
                    // WAVE_FORMAT_EXTENSIBLE: the sub-format GUID starts
                    // with the real tag.
                    tag = read_u16(body, 24).ok_or("The extensible WAV header is too short.")?;
                }
                format = Some((tag, channels, rate, bits));
            }
            b"data" => data = Some(body),
            _ => {}
        }
        position = body_start + size + (size & 1);
    }
    let (tag, channels, rate, bits) = format.ok_or("The WAV file has no format chunk.")?;
    let data = data.ok_or("The WAV file has no audio data.")?;
    let channels = usize::from(channels);
    if channels == 0 || channels > MAX_CHANNELS {
        return Err(format!(
            "A filter can have 1 to {MAX_CHANNELS} channels; this one has {channels}."
        ));
    }
    if !(8_000..=768_000).contains(&rate) {
        return Err(format!("{rate} Hz is not a usable filter sample rate."));
    }
    type Decode = fn(&[u8]) -> f64;
    let (width, label, decode): (usize, String, Decode) = match (tag, bits) {
        (1, 16) => (2, "16-bit integer".into(), |b| {
            f64::from(i16::from_le_bytes([b[0], b[1]])) / 32_768.0
        }),
        (1, 24) => (3, "24-bit integer".into(), |b| {
            f64::from(i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) / 8_388_608.0
        }),
        (1, 32) => (4, "32-bit integer".into(), |b| {
            f64::from(i32::from_le_bytes([b[0], b[1], b[2], b[3]])) / 2_147_483_648.0
        }),
        (3, 32) => (4, "32-bit float".into(), |b| {
            f64::from(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        }),
        (3, 64) => (8, "64-bit float".into(), |b| {
            f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
        }),
        _ => {
            return Err(format!(
                "Mujin Music reads 16/24/32-bit integer or 32/64-bit float WAV filters; this one is format {tag} at {bits} bits."
            ));
        }
    };
    let frame = width * channels;
    let taps = data.len() / frame;
    if taps == 0 {
        return Err("The filter has no samples.".into());
    }
    if taps > MAX_TAPS {
        return Err(format!(
            "The filter has {taps} taps; Mujin Music accepts up to {MAX_TAPS}."
        ));
    }
    let mut planar = vec![Vec::with_capacity(taps); channels];
    for samples in data.chunks_exact(frame) {
        for (channel, sample) in samples.chunks_exact(width).enumerate() {
            let value = decode(sample);
            planar[channel].push(if value.is_finite() { value } else { 0.0 });
        }
    }
    Ok((
        ImpulseResponse {
            sample_rate: rate,
            channels: planar,
        },
        label,
    ))
}

/// Writes an impulse response as a 64-bit float WAV (used for tests and
/// for filters made in code).
pub fn write_wav_f64(sample_rate: u32, channels: &[Vec<f64>]) -> Vec<u8> {
    let count = channels.len() as u16;
    let taps = channels.first().map_or(0, Vec::len);
    let data_size = (taps * channels.len() * 8) as u32;
    let mut out = Vec::with_capacity(44 + data_size as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_size).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * u32::from(count) * 8).to_le_bytes());
    out.extend_from_slice(&(count * 8).to_le_bytes());
    out.extend_from_slice(&64u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_size.to_le_bytes());
    for index in 0..taps {
        for channel in channels {
            out.extend_from_slice(&channel[index].to_le_bytes());
        }
    }
    out
}

impl ImpulseResponse {
    pub fn taps(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }

    /// The same filter at `rate`: resampled with the engine's linear-phase
    /// converter and scaled by the ratio, so its gain at each frequency is
    /// unchanged.
    pub fn at_rate(&self, rate: u32) -> ImpulseResponse {
        if rate == self.sample_rate {
            return self.clone();
        }
        let scale = f64::from(self.sample_rate) / f64::from(rate);
        let channels = self
            .channels
            .iter()
            .map(|channel| {
                let mut resampler = Resampler::new(self.sample_rate, rate, 1);
                let mut out = Vec::new();
                resampler.process(channel, &mut out);
                resampler.finish(&mut out);
                out.iter_mut().for_each(|sample| *sample *= scale);
                out.truncate(MAX_TAPS * 4);
                out
            })
            .collect();
        ImpulseResponse {
            sample_rate: rate,
            channels,
        }
    }

    /// Frequency response magnitudes (dB) of one channel at `points`
    /// frequencies, log-spaced from 20 Hz to 20 kHz (or Nyquist).
    pub fn response_db(&self, channel: usize, points: usize) -> Vec<(f64, f64)> {
        let taps = &self.channels[channel];
        let size = taps.len().next_power_of_two().max(4096) * 2;
        let size = size.min(1 << 21);
        let fft = Fft::new(size);
        let mut re = vec![0.0; size];
        let mut im = vec![0.0; size];
        for (index, tap) in taps.iter().take(size).enumerate() {
            re[index] = *tap;
        }
        fft.forward(&mut re, &mut im);
        let nyquist = f64::from(self.sample_rate) / 2.0;
        let top = nyquist.min(20_000.0);
        (0..points)
            .map(|index| {
                let fraction = index as f64 / (points.max(2) - 1) as f64;
                let frequency = 20.0 * (top / 20.0).powf(fraction);
                let bin =
                    ((frequency / f64::from(self.sample_rate)) * size as f64).round() as usize;
                let bin = bin.min(size / 2);
                let magnitude = (re[bin] * re[bin] + im[bin] * im[bin]).sqrt();
                (frequency, 20.0 * magnitude.max(1e-12).log10())
            })
            .collect()
    }

    pub fn summary(&self, format: String) -> FilterSummary {
        let mut peak = f64::NEG_INFINITY;
        let mut at_1k = f64::NEG_INFINITY;
        for channel in 0..self.channels.len() {
            let response = self.response_db(channel, 256);
            for (_, db) in &response {
                peak = peak.max(*db);
            }
            if let Some((_, db)) = response
                .iter()
                .min_by(|a, b| (a.0 - 1_000.0).abs().total_cmp(&(b.0 - 1_000.0).abs()))
            {
                at_1k = at_1k.max(*db);
            }
        }
        FilterSummary {
            sample_rate: self.sample_rate,
            channels: self.channels.len() as u8,
            taps: self.taps(),
            duration_ms: self.taps() as f64 * 1_000.0 / f64::from(self.sample_rate),
            peak_gain_db: (peak * 10.0).round() / 10.0,
            gain_1khz_db: (at_1k * 10.0).round() / 10.0,
            format,
        }
    }
}

/* ------------------------------------------------------------------ */
/* FFT                                                                 */
/* ------------------------------------------------------------------ */

/// An iterative radix-2 complex FFT of one size, with its tables.
pub struct Fft {
    size: usize,
    reversed: Vec<usize>,
    cos: Vec<f64>,
    sin: Vec<f64>,
}

impl Fft {
    pub fn new(size: usize) -> Self {
        assert!(size.is_power_of_two() && size >= 2);
        let bits = size.trailing_zeros();
        let reversed = (0..size)
            .map(|index| index.reverse_bits() >> (usize::BITS - bits))
            .collect();
        let (cos, sin) = (0..size / 2)
            .map(|index| {
                let angle = -2.0 * std::f64::consts::PI * index as f64 / size as f64;
                (angle.cos(), angle.sin())
            })
            .unzip();
        Self {
            size,
            reversed,
            cos,
            sin,
        }
    }

    fn transform(&self, re: &mut [f64], im: &mut [f64], inverse: bool) {
        let n = self.size;
        for index in 0..n {
            let other = self.reversed[index];
            if other > index {
                re.swap(index, other);
                im.swap(index, other);
            }
        }
        let mut length = 2;
        while length <= n {
            let half = length / 2;
            let stride = n / length;
            for start in (0..n).step_by(length) {
                for k in 0..half {
                    let wr = self.cos[k * stride];
                    let wi = if inverse {
                        -self.sin[k * stride]
                    } else {
                        self.sin[k * stride]
                    };
                    let a = start + k;
                    let b = a + half;
                    let tr = re[b] * wr - im[b] * wi;
                    let ti = re[b] * wi + im[b] * wr;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            length <<= 1;
        }
        if inverse {
            let scale = 1.0 / n as f64;
            re.iter_mut().for_each(|value| *value *= scale);
            im.iter_mut().for_each(|value| *value *= scale);
        }
    }

    pub fn forward(&self, re: &mut [f64], im: &mut [f64]) {
        self.transform(re, im, false);
    }

    pub fn inverse(&self, re: &mut [f64], im: &mut [f64]) {
        self.transform(re, im, true);
    }
}

/* ------------------------------------------------------------------ */
/* Convolver                                                           */
/* ------------------------------------------------------------------ */

struct Spectrum {
    re: Vec<f64>,
    im: Vec<f64>,
}

struct ChannelConvolver {
    /// The impulse response's partitions, transformed.
    partitions: Arc<Vec<Spectrum>>,
    /// Transformed input blocks, newest at `head`.
    history: Vec<Spectrum>,
    head: usize,
    /// The previous input block (the overlap of overlap-save).
    previous: Vec<f64>,
}

/// Convolves interleaved frames with one filter per channel.
pub struct Convolver {
    fft: Arc<Fft>,
    channels: Vec<ChannelConvolver>,
    /// Input frames waiting for a whole block.
    pending: Vec<f64>,
    work_re: Vec<f64>,
    work_im: Vec<f64>,
    accumulate_re: Vec<f64>,
    accumulate_im: Vec<f64>,
}

fn transform_partitions(fft: &Fft, taps: &[f64]) -> Vec<Spectrum> {
    taps.chunks(BLOCK)
        .map(|chunk| {
            let mut re = vec![0.0; BLOCK * 2];
            let mut im = vec![0.0; BLOCK * 2];
            re[..chunk.len()].copy_from_slice(chunk);
            fft.forward(&mut re, &mut im);
            Spectrum { re, im }
        })
        .collect()
}

impl Convolver {
    /// `channels` streams; the filter has one channel (used for all) or
    /// one per stream channel.
    pub fn new(filter: &ImpulseResponse, channels: usize) -> Result<Self, String> {
        let filters = filter.channels.len();
        if filters != 1 && filters != channels {
            return Err(format!(
                "the filter has {filters} channels and this track plays in {channels}"
            ));
        }
        let fft = Arc::new(Fft::new(BLOCK * 2));
        let shared: Vec<Arc<Vec<Spectrum>>> = filter
            .channels
            .iter()
            .map(|taps| Arc::new(transform_partitions(&fft, taps)))
            .collect();
        let channel_convolvers = (0..channels)
            .map(|channel| {
                let partitions = Arc::clone(&shared[if filters == 1 { 0 } else { channel }]);
                let count = partitions.len();
                ChannelConvolver {
                    partitions,
                    history: (0..count)
                        .map(|_| Spectrum {
                            re: vec![0.0; BLOCK * 2],
                            im: vec![0.0; BLOCK * 2],
                        })
                        .collect(),
                    head: 0,
                    previous: vec![0.0; BLOCK],
                }
            })
            .collect();
        Ok(Self {
            fft,
            channels: channel_convolvers,
            pending: Vec::with_capacity(BLOCK * channels),
            work_re: vec![0.0; BLOCK * 2],
            work_im: vec![0.0; BLOCK * 2],
            accumulate_re: vec![0.0; BLOCK * 2],
            accumulate_im: vec![0.0; BLOCK * 2],
        })
    }

    fn run_block(&mut self, block: &[f64], output: &mut Vec<f64>) {
        let count = self.channels.len();
        let frames = block.len() / count;
        let start = output.len();
        output.resize(start + frames * count, 0.0);
        for (index, channel) in self.channels.iter_mut().enumerate() {
            // [previous block | this block], zero-padded when short.
            self.work_re[..BLOCK].copy_from_slice(&channel.previous);
            for (slot, frame) in self.work_re[BLOCK..].iter_mut().enumerate() {
                *frame = if slot < frames {
                    block[slot * count + index]
                } else {
                    0.0
                };
            }
            self.work_im.iter_mut().for_each(|value| *value = 0.0);
            channel.previous.copy_from_slice(&self.work_re[BLOCK..]);
            self.fft.forward(&mut self.work_re, &mut self.work_im);
            let partitions = channel.partitions.len();
            channel.head = (channel.head + partitions - 1) % partitions;
            let slot = &mut channel.history[channel.head];
            slot.re.copy_from_slice(&self.work_re);
            slot.im.copy_from_slice(&self.work_im);
            self.accumulate_re.iter_mut().for_each(|value| *value = 0.0);
            self.accumulate_im.iter_mut().for_each(|value| *value = 0.0);
            for (offset, partition) in channel.partitions.iter().enumerate() {
                let input = &channel.history[(channel.head + offset) % partitions];
                for bin in 0..BLOCK * 2 {
                    let (ar, ai) = (input.re[bin], input.im[bin]);
                    let (br, bi) = (partition.re[bin], partition.im[bin]);
                    self.accumulate_re[bin] += ar * br - ai * bi;
                    self.accumulate_im[bin] += ar * bi + ai * br;
                }
            }
            self.fft
                .inverse(&mut self.accumulate_re, &mut self.accumulate_im);
            for frame in 0..frames {
                output[start + frame * count + index] = self.accumulate_re[BLOCK + frame];
            }
        }
    }

    /// Convolves interleaved `input`, appending whatever output is ready
    /// (whole blocks; the rest waits for more input or [`Self::flush`]).
    pub fn process(&mut self, input: &[f64], output: &mut Vec<f64>) {
        let count = self.channels.len();
        let block_samples = BLOCK * count;
        let mut input = input;
        if !self.pending.is_empty() {
            let need = block_samples - self.pending.len();
            let take = need.min(input.len());
            self.pending.extend_from_slice(&input[..take]);
            input = &input[take..];
            if self.pending.len() < block_samples {
                return;
            }
            let pending = std::mem::take(&mut self.pending);
            self.run_block(&pending, output);
            self.pending = pending;
            self.pending.clear();
        }
        let whole = input.len() / block_samples * block_samples;
        for block in input[..whole].chunks_exact(block_samples) {
            self.run_block(block, output);
        }
        self.pending.extend_from_slice(&input[whole..]);
    }

    /// Emits the frames still waiting, at the end of the track.
    pub fn flush(&mut self, output: &mut Vec<f64>) {
        if self.pending.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.pending);
        self.run_block(&pending, output);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn direct(signal: &[f64], taps: &[f64]) -> Vec<f64> {
        (0..signal.len())
            .map(|n| {
                taps.iter()
                    .enumerate()
                    .filter(|(k, _)| *k <= n)
                    .map(|(k, tap)| tap * signal[n - k])
                    .sum()
            })
            .collect()
    }

    #[test]
    fn fft_round_trips_and_finds_a_tone() {
        let fft = Fft::new(64);
        let mut re: Vec<f64> = (0..64)
            .map(|i| (2.0 * std::f64::consts::PI * 5.0 * i as f64 / 64.0).cos())
            .collect();
        let original = re.clone();
        let mut im = vec![0.0; 64];
        fft.forward(&mut re, &mut im);
        assert!((re[5] - 32.0).abs() < 1e-9);
        assert!(re[6].abs() < 1e-9);
        fft.inverse(&mut re, &mut im);
        for (a, b) in re.iter().zip(&original) {
            assert!((a - b).abs() < 1e-12);
        }
    }

    #[test]
    fn partitioned_convolution_matches_direct_convolution() {
        // A filter longer than two partitions, and input in odd chunks.
        let taps: Vec<f64> = (0..BLOCK * 2 + 300)
            .map(|i| ((i * 37 % 101) as f64 - 50.0) / 500.0 * (-(i as f64) / 1500.0).exp())
            .collect();
        let frames = BLOCK * 3 + 777;
        let left: Vec<f64> = (0..frames)
            .map(|i| ((i * 13 % 97) as f64 - 48.0) / 64.0)
            .collect();
        let right: Vec<f64> = (0..frames).map(|i| ((i as f64) * 0.01).sin()).collect();
        let interleaved: Vec<f64> = left
            .iter()
            .zip(&right)
            .flat_map(|(l, r)| [*l, *r])
            .collect();
        let filter = ImpulseResponse {
            sample_rate: 48_000,
            channels: vec![taps.clone()],
        };
        let mut convolver = Convolver::new(&filter, 2).unwrap();
        let mut output = Vec::new();
        for chunk in interleaved.chunks(2 * 1000 + 2) {
            convolver.process(chunk, &mut output);
        }
        convolver.flush(&mut output);
        assert_eq!(
            output.len(),
            interleaved.len(),
            "no frames are added or lost"
        );
        let expected_left = direct(&left, &taps);
        let expected_right = direct(&right, &taps);
        for frame in 0..frames {
            assert!(
                (output[frame * 2] - expected_left[frame]).abs() < 1e-9,
                "left {frame}"
            );
            assert!(
                (output[frame * 2 + 1] - expected_right[frame]).abs() < 1e-9,
                "right {frame}"
            );
        }
    }

    #[test]
    fn per_channel_filters_and_a_dirac_pass_audio_unchanged() {
        let mut dirac = vec![0.0; 64];
        dirac[0] = 1.0;
        let mut half = vec![0.0; 64];
        half[0] = 0.5;
        let filter = ImpulseResponse {
            sample_rate: 44_100,
            channels: vec![dirac, half],
        };
        let mut convolver = Convolver::new(&filter, 2).unwrap();
        let input: Vec<f64> = (0..BLOCK * 2 + 10)
            .map(|i| (i % 17) as f64 / 17.0)
            .collect();
        let mut output = Vec::new();
        convolver.process(&input, &mut output);
        convolver.flush(&mut output);
        for (index, (out, inp)) in output.iter().zip(&input).enumerate() {
            let want = if index % 2 == 0 { *inp } else { inp * 0.5 };
            assert!((out - want).abs() < 1e-12);
        }
        assert!(Convolver::new(&filter, 6).is_err());
    }

    #[test]
    fn wav_filters_parse_in_every_supported_format() {
        let taps = vec![vec![1.0, 0.5, -0.25, 0.0], vec![0.0, 1.0, 0.0, 0.0]];
        let wav = write_wav_f64(48_000, &taps);
        let (parsed, format) = parse_wav(&wav).unwrap();
        assert_eq!(parsed.sample_rate, 48_000);
        assert_eq!(parsed.channels, taps);
        assert_eq!(format, "64-bit float");
        // 16-bit integer mono.
        let mut pcm16 = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        pcm16.extend_from_slice(&16u32.to_le_bytes());
        pcm16.extend_from_slice(&1u16.to_le_bytes());
        pcm16.extend_from_slice(&1u16.to_le_bytes());
        pcm16.extend_from_slice(&44_100u32.to_le_bytes());
        pcm16.extend_from_slice(&88_200u32.to_le_bytes());
        pcm16.extend_from_slice(&2u16.to_le_bytes());
        pcm16.extend_from_slice(&16u16.to_le_bytes());
        pcm16.extend_from_slice(b"data");
        pcm16.extend_from_slice(&4u32.to_le_bytes());
        pcm16.extend_from_slice(&16_384i16.to_le_bytes());
        pcm16.extend_from_slice(&(-32_768i16).to_le_bytes());
        let (parsed, format) = parse_wav(&pcm16).unwrap();
        assert_eq!(parsed.channels, vec![vec![0.5, -1.0]]);
        assert_eq!(format, "16-bit integer");
        assert!(parse_wav(b"not a wav").is_err());
    }

    #[test]
    fn resampled_filters_keep_their_gain() {
        // A 6 dB attenuator, delayed like a real filter's main peak.
        let mut taps = vec![0.0; 512];
        taps[200] = 0.5;
        let filter = ImpulseResponse {
            sample_rate: 48_000,
            channels: vec![taps],
        };
        let summary = filter.summary("64-bit float".into());
        assert!((summary.peak_gain_db + 6.02).abs() < 0.05, "{summary:?}");
        let converted = filter.at_rate(96_000);
        assert_eq!(converted.sample_rate, 96_000);
        let response = converted.response_db(0, 32);
        for (frequency, db) in response
            .iter()
            .filter(|(frequency, _)| *frequency < 18_000.0)
        {
            assert!((db + 6.02).abs() < 0.1, "{frequency} Hz: {db} dB");
        }
    }
}
