//! Audio analysis for library tools: a peak/RMS waveform for display and an
//! acoustic fingerprint for duplicate checks.
//!
//! The fingerprint is Chromaprint (the AcoustID algorithm) when ffmpeg was
//! built with it or `fpcalc` is installed. Without either, a loudness
//! envelope taken from the same decode stands in: it cannot find a song
//! in a noisy recording, but it reliably tells two encodes of one master
//! apart from two different recordings with the same title and length.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Serialize;
use tokio::io::AsyncReadExt;

/// Points in a stored waveform.
pub const WAVEFORM_BINS: usize = 240;
/// Decode rate for the waveform and the envelope fingerprint.
const ANALYSIS_RATE: u32 = 8_000;
/// Envelope frame length (250 ms) at the analysis rate.
const ENVELOPE_FRAME: usize = (ANALYSIS_RATE / 4) as usize;
/// Seconds of audio fingerprinted.
const FINGERPRINT_SECONDS: u32 = 120;
const ENVELOPE_FRAMES: usize = (FINGERPRINT_SECONDS * 4) as usize;
const DECODE_TIMEOUT: Duration = Duration::from_secs(180);
/// Upper bound on decoded PCM read into memory (about 70 minutes).
const MAX_PCM_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FingerprintKind {
    Chromaprint,
    Envelope,
}

impl FingerprintKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chromaprint => "chromaprint",
            Self::Envelope => "envelope",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "chromaprint" => Some(Self::Chromaprint),
            "envelope" => Some(Self::Envelope),
            _ => None,
        }
    }

    /// Similarity at or above which two files are the same recording.
    pub fn match_threshold(self) -> f32 {
        match self {
            Self::Chromaprint => 0.70,
            Self::Envelope => 0.92,
        }
    }

    /// Similarity below which two files are certainly different recordings.
    pub fn mismatch_threshold(self) -> f32 {
        match self {
            Self::Chromaprint => 0.30,
            Self::Envelope => 0.60,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Fingerprint {
    /// Raw 32-bit Chromaprint sub-fingerprints (about 8 per second).
    Chromaprint(Vec<u32>),
    /// Loudness per 250 ms frame, 0..=255 (about 0.375 dB steps).
    Envelope(Vec<u8>),
}

impl Fingerprint {
    pub fn kind(&self) -> FingerprintKind {
        match self {
            Self::Chromaprint(_) => FingerprintKind::Chromaprint,
            Self::Envelope(_) => FingerprintKind::Envelope,
        }
    }

    pub fn encode(&self) -> String {
        match self {
            Self::Chromaprint(values) => {
                let bytes: Vec<u8> = values
                    .iter()
                    .flat_map(|value| value.to_le_bytes())
                    .collect();
                STANDARD.encode(bytes)
            }
            Self::Envelope(values) => STANDARD.encode(values),
        }
    }

    pub fn decode(kind: FingerprintKind, text: &str) -> Option<Self> {
        let bytes = STANDARD.decode(text.as_bytes()).ok()?;
        if bytes.is_empty() {
            return None;
        }
        Some(match kind {
            FingerprintKind::Chromaprint => {
                if !bytes.len().is_multiple_of(4) {
                    return None;
                }
                Self::Chromaprint(
                    bytes
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|chunk| u32::from_le_bytes(*chunk))
                        .collect(),
                )
            }
            FingerprintKind::Envelope => Self::Envelope(bytes),
        })
    }

    /// Whether the print carries enough detail to identify audio. Steady
    /// tones, silence and test signals give near-constant prints that would
    /// "match" anything of the same length.
    pub fn is_informative(&self) -> bool {
        match self {
            Self::Chromaprint(values) => {
                if values.len() < 16 {
                    return false;
                }
                let changes = values.windows(2).filter(|pair| pair[0] != pair[1]).count();
                let distinct: std::collections::HashSet<&u32> = values.iter().collect();
                changes * 10 >= values.len() * 3 && distinct.len() * 4 >= values.len()
            }
            Self::Envelope(values) => {
                if values.len() < 16 {
                    return false;
                }
                let mean = values.iter().map(|v| f64::from(*v)).sum::<f64>() / values.len() as f64;
                let variance = values
                    .iter()
                    .map(|v| (f64::from(*v) - mean).powi(2))
                    .sum::<f64>()
                    / values.len() as f64;
                variance.sqrt() >= 4.0
            }
        }
    }

    /// 0.0 (unrelated) to 1.0 (identical); `None` across kinds.
    pub fn similarity(&self, other: &Self) -> Option<f32> {
        match (self, other) {
            (Self::Chromaprint(left), Self::Chromaprint(right)) => {
                Some(chromaprint_similarity(left, right))
            }
            (Self::Envelope(left), Self::Envelope(right)) => Some(envelope_similarity(left, right)),
            _ => None,
        }
    }
}

/// Best bit agreement over small alignment offsets, rescaled so unrelated
/// audio (about half the bits agree) scores near zero.
pub fn chromaprint_similarity(left: &[u32], right: &[u32]) -> f32 {
    const MAX_OFFSET: isize = 4;
    const MAX_COMPARED: usize = 960;
    let mut best = 0.0f32;
    for offset in -MAX_OFFSET..=MAX_OFFSET {
        let (left_start, right_start) = if offset >= 0 {
            (offset as usize, 0)
        } else {
            (0, (-offset) as usize)
        };
        if left_start >= left.len() || right_start >= right.len() {
            continue;
        }
        let overlap = (left.len() - left_start)
            .min(right.len() - right_start)
            .min(MAX_COMPARED);
        if overlap < 16 {
            continue;
        }
        let errors: u32 = left[left_start..left_start + overlap]
            .iter()
            .zip(&right[right_start..right_start + overlap])
            .map(|(a, b)| (a ^ b).count_ones())
            .sum();
        let agreement = 1.0 - errors as f32 / (32.0 * overlap as f32);
        best = best.max(((agreement - 0.5) * 2.0).clamp(0.0, 1.0));
    }
    best
}

/// Best Pearson correlation of two loudness envelopes over small offsets.
pub fn envelope_similarity(left: &[u8], right: &[u8]) -> f32 {
    const MAX_OFFSET: isize = 8;
    let mut best = 0.0f32;
    for offset in -MAX_OFFSET..=MAX_OFFSET {
        let (left_start, right_start) = if offset >= 0 {
            (offset as usize, 0)
        } else {
            (0, (-offset) as usize)
        };
        if left_start >= left.len() || right_start >= right.len() {
            continue;
        }
        let overlap = (left.len() - left_start).min(right.len() - right_start);
        if overlap < 16 {
            continue;
        }
        let a = &left[left_start..left_start + overlap];
        let b = &right[right_start..right_start + overlap];
        let mean =
            |values: &[u8]| values.iter().map(|v| f64::from(*v)).sum::<f64>() / overlap as f64;
        let (mean_a, mean_b) = (mean(a), mean(b));
        let mut covariance = 0.0;
        let mut variance_a = 0.0;
        let mut variance_b = 0.0;
        for (x, y) in a.iter().zip(b) {
            let dx = f64::from(*x) - mean_a;
            let dy = f64::from(*y) - mean_b;
            covariance += dx * dy;
            variance_a += dx * dx;
            variance_b += dy * dy;
        }
        let correlation = if variance_a < 1e-9 || variance_b < 1e-9 {
            // Flat envelopes (silence, test tones) only match each other.
            if (mean_a - mean_b).abs() < 2.0 {
                1.0
            } else {
                0.0
            }
        } else {
            covariance / (variance_a.sqrt() * variance_b.sqrt())
        };
        best = best.max(correlation.clamp(0.0, 1.0) as f32);
    }
    best
}

/// Peak and RMS per bin, each 0..=255 on a perceptual (square-root) scale.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Waveform {
    pub peaks: Vec<u8>,
    pub rms: Vec<u8>,
}

impl Waveform {
    pub fn encode(&self) -> String {
        let mut bytes = self.peaks.clone();
        bytes.extend_from_slice(&self.rms);
        STANDARD.encode(bytes)
    }

    pub fn decode(text: &str) -> Option<Self> {
        let bytes = STANDARD.decode(text.as_bytes()).ok()?;
        if bytes.is_empty() || !bytes.len().is_multiple_of(2) {
            return None;
        }
        let half = bytes.len() / 2;
        Some(Self {
            peaks: bytes[..half].to_vec(),
            rms: bytes[half..].to_vec(),
        })
    }
}

fn perceptual(level: f64) -> u8 {
    (level.clamp(0.0, 1.0).sqrt() * 255.0).round() as u8
}

/// Waveform of mono samples in `bins` columns.
pub fn waveform_from_samples(samples: &[i16], bins: usize) -> Waveform {
    if samples.is_empty() || bins == 0 {
        return Waveform::default();
    }
    let mut peaks = Vec::with_capacity(bins);
    let mut rms = Vec::with_capacity(bins);
    for bin in 0..bins {
        let start = bin * samples.len() / bins;
        let end = ((bin + 1) * samples.len() / bins)
            .max(start + 1)
            .min(samples.len());
        let slice = &samples[start.min(samples.len() - 1)..end];
        let mut peak = 0i32;
        let mut energy = 0f64;
        for sample in slice {
            let value = i32::from(*sample);
            peak = peak.max(value.abs());
            energy += f64::from(value) * f64::from(value);
        }
        let full_scale = f64::from(i16::MAX);
        peaks.push(perceptual(f64::from(peak) / full_scale));
        rms.push(perceptual(
            (energy / slice.len() as f64).sqrt() / full_scale * std::f64::consts::SQRT_2,
        ));
    }
    Waveform { peaks, rms }
}

/// Loudness envelope of the first [`FINGERPRINT_SECONDS`] seconds.
pub fn envelope_from_samples(samples: &[i16]) -> Vec<u8> {
    samples
        .chunks(ENVELOPE_FRAME)
        .take(ENVELOPE_FRAMES)
        .filter(|frame| frame.len() * 2 >= ENVELOPE_FRAME)
        .map(|frame| {
            let energy = frame
                .iter()
                .map(|sample| f64::from(*sample) * f64::from(*sample))
                .sum::<f64>()
                / frame.len() as f64;
            let rms = energy.sqrt() / f64::from(i16::MAX);
            // -96 dBFS .. 0 dBFS onto 0..255.
            let db = if rms > 0.0 { 20.0 * rms.log10() } else { -96.0 };
            (((db + 96.0) / 96.0).clamp(0.0, 1.0) * 255.0).round() as u8
        })
        .collect()
}

/// Everything one analysis pass learns about a file.
#[derive(Clone, Debug)]
pub struct AnalysisResult {
    pub waveform: Waveform,
    pub fingerprint: Option<Fingerprint>,
    pub duration_seconds: f64,
}

async fn run_capture(mut command: tokio::process::Command, limit: usize) -> Option<Vec<u8>> {
    command
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let read = async {
        let mut buffer = Vec::new();
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            let count = stdout.read(&mut chunk).await.ok()?;
            if count == 0 {
                break;
            }
            if buffer.len() + count > limit {
                buffer.extend_from_slice(&chunk[..limit - buffer.len()]);
                break;
            }
            buffer.extend_from_slice(&chunk[..count]);
        }
        Some(buffer)
    };
    let buffer = tokio::time::timeout(DECODE_TIMEOUT, read).await.ok()??;
    let _ = child.kill().await;
    Some(buffer)
}

/// Decodes the first audio stream to mono 16-bit PCM at the analysis rate.
async fn decode_pcm(path: &Path) -> Option<Vec<i16>> {
    let mut command = tokio::process::Command::new("ffmpeg");
    command
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-map", "0:a:0", "-ac", "1", "-ar"])
        .arg(ANALYSIS_RATE.to_string())
        .args(["-f", "s16le", "-"]);
    let bytes = run_capture(command, MAX_PCM_BYTES).await?;
    if bytes.len() < 2 {
        return None;
    }
    Some(
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| i16::from_le_bytes(*pair))
            .collect(),
    )
}

/// Chromaprint through ffmpeg's muxer, then through `fpcalc`.
async fn chromaprint(path: &Path) -> Option<Vec<u32>> {
    let mut command = tokio::process::Command::new("ffmpeg");
    command
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-map", "0:a:0", "-t"])
        .arg(FINGERPRINT_SECONDS.to_string())
        .args(["-f", "chromaprint", "-fp_format", "raw", "-"]);
    if let Some(bytes) = run_capture(command, 1024 * 1024).await
        && bytes.len() >= 64
        && bytes.len().is_multiple_of(4)
    {
        return Some(
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|chunk| u32::from_le_bytes(*chunk))
                .collect(),
        );
    }
    let mut command = tokio::process::Command::new("fpcalc");
    command
        .args(["-raw", "-json", "-length"])
        .arg(FINGERPRINT_SECONDS.to_string())
        .arg(path);
    let bytes = run_capture(command, 2 * 1024 * 1024).await?;
    parse_fpcalc_json(&bytes)
}

/// Reads `{"fingerprint": [..]}` from `fpcalc -raw -json`.
pub fn parse_fpcalc_json(bytes: &[u8]) -> Option<Vec<u32>> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let values: Vec<u32> = value
        .get("fingerprint")?
        .as_array()?
        .iter()
        .filter_map(|item| {
            item.as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .or_else(|| item.as_i64().map(|value| value as i32 as u32))
        })
        .collect();
    (values.len() >= 16).then_some(values)
}

/// Decodes `path` once for the waveform and envelope, and fingerprints it.
pub async fn analyze_file(path: &Path) -> Result<AnalysisResult, String> {
    let samples = decode_pcm(path)
        .await
        .ok_or_else(|| "ffmpeg could not decode the audio stream".to_string())?;
    let duration_seconds = samples.len() as f64 / f64::from(ANALYSIS_RATE);
    let waveform = waveform_from_samples(&samples, WAVEFORM_BINS);
    let fingerprint = match chromaprint(path).await {
        Some(values) => Some(Fingerprint::Chromaprint(values)),
        None => {
            let envelope = envelope_from_samples(&samples);
            (envelope.len() >= 16).then_some(Fingerprint::Envelope(envelope))
        }
    };
    Ok(AnalysisResult {
        waveform,
        fingerprint,
        duration_seconds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(seconds: f64, amplitude: f64, wobble: f64) -> Vec<i16> {
        let count = (seconds * f64::from(ANALYSIS_RATE)) as usize;
        (0..count)
            .map(|index| {
                let t = index as f64 / f64::from(ANALYSIS_RATE);
                // A slow loudness swell makes the envelope distinctive.
                let level = amplitude * (0.55 + 0.45 * (t * wobble).sin());
                (level * (t * 440.0 * std::f64::consts::TAU).sin() * f64::from(i16::MAX)) as i16
            })
            .collect()
    }

    #[test]
    fn waveform_has_the_requested_bins_and_tracks_level() {
        let mut samples = tone(4.0, 0.1, 0.0);
        samples.extend(tone(4.0, 0.9, 0.0));
        let waveform = waveform_from_samples(&samples, 8);
        assert_eq!(waveform.peaks.len(), 8);
        assert_eq!(waveform.rms.len(), 8);
        assert!(waveform.peaks[7] > waveform.peaks[0]);
        assert!(
            waveform
                .rms
                .iter()
                .zip(&waveform.peaks)
                .all(|(r, p)| r <= p)
        );
        let round_trip = Waveform::decode(&waveform.encode()).expect("decodes");
        assert_eq!(round_trip, waveform);
    }

    #[test]
    fn envelopes_match_the_same_master_and_reject_a_different_one() {
        let original = envelope_from_samples(&tone(60.0, 0.8, 0.7));
        // A quieter encode of the same master: same shape, lower level.
        let quieter = envelope_from_samples(&tone(60.0, 0.5, 0.7));
        let different = envelope_from_samples(&tone(60.0, 0.8, 2.3));
        let same = envelope_similarity(&original, &quieter);
        let other = envelope_similarity(&original, &different);
        assert!(
            same >= FingerprintKind::Envelope.match_threshold(),
            "{same}"
        );
        assert!(
            other < FingerprintKind::Envelope.match_threshold(),
            "{other}"
        );
    }

    #[test]
    fn chromaprint_similarity_is_high_for_near_copies_and_low_for_noise() {
        let base: Vec<u32> = (0..400u32)
            .map(|index| index.wrapping_mul(2_654_435_761))
            .collect();
        // A lossy copy flips a couple of bits per word and is shifted by one.
        let mut copy: Vec<u32> = base.iter().map(|value| value ^ 0b101).collect();
        copy.insert(0, 7);
        let noise: Vec<u32> = (0..400u32)
            .map(|index| index.wrapping_mul(40_503).rotate_left(index % 31) ^ 0x5555_5555)
            .collect();
        assert!(chromaprint_similarity(&base, &copy) > 0.85);
        assert!(chromaprint_similarity(&base, &noise) < 0.3);
        let fingerprint = Fingerprint::Chromaprint(base.clone());
        assert_eq!(
            Fingerprint::decode(FingerprintKind::Chromaprint, &fingerprint.encode()),
            Some(fingerprint)
        );
    }

    #[test]
    fn steady_prints_are_not_informative() {
        assert!(!Fingerprint::Chromaprint(vec![7; 200]).is_informative());
        let varied: Vec<u32> = (0..200u32).map(|i| i.wrapping_mul(2_654_435_761)).collect();
        assert!(Fingerprint::Chromaprint(varied).is_informative());
        assert!(!Fingerprint::Envelope(vec![120; 100]).is_informative());
        let swell = envelope_from_samples(&tone(60.0, 0.8, 0.7));
        assert!(Fingerprint::Envelope(swell).is_informative());
    }

    #[test]
    fn fpcalc_json_is_read() {
        let json = br#"{"duration": 12.5, "fingerprint": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, -1]}"#;
        let values = parse_fpcalc_json(json).expect("values");
        assert_eq!(values.len(), 16);
        assert_eq!(values[15], u32::MAX);
        assert!(parse_fpcalc_json(b"{}").is_none());
    }
}
