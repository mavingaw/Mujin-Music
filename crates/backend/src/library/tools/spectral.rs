//! The fake-lossless check: does a FLAC, ALAC or WAV really carry a
//! lossless master, or was it made from an MP3 or AAC?
//!
//! Lossy encoders throw away everything above a fixed low-pass (about
//! 16 kHz for 128 kbps MP3, 19 to 20 kHz for 256 to 320 kbps), and a
//! lossless file made from such a copy keeps that hole: its spectrum ends
//! in a cliff, with only the noise floor above it. A real CD master rolls
//! off gradually up to about 21 to 22 kHz.
//!
//! The check is deliberately cheap: one ffmpeg decode to 44.1 kHz mono and
//! a few dozen 8192-point FFTs spread over the track. It records the
//! cutoff and a confidence, never a certainty: quiet or very old
//! recordings with no treble at all are reported as inconclusive rather
//! than accused.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use tokio::io::AsyncReadExt;

/// Rate the check decodes at.
pub const SPECTRAL_RATE: u32 = 44_100;
/// FFT size (about 5.4 Hz per bin at 44.1 kHz).
const FFT_SIZE: usize = 8_192;
/// Most windows analysed per file.
const MAX_WINDOWS: usize = 32;
/// Fewest loud-enough windows needed for a verdict.
const MIN_WINDOWS: usize = 4;
/// Width of one analysis band.
const BAND_HZ: f64 = 250.0;
/// Highest frequency looked at (just under Nyquist of 44.1 kHz).
const TOP_HZ: f64 = 21_750.0;
/// Lowest cutoff the check looks for.
const LOWEST_CUTOFF_HZ: f64 = 11_000.0;
/// A cliff at or above this is the normal anti-alias filter of a CD.
pub const GENUINE_CUTOFF_HZ: u32 = 20_500;
/// Windows quieter than this (RMS, dBFS) say nothing about the spectrum.
const QUIET_WINDOW_DBFS: f64 = -55.0;
/// Seconds decoded at most.
const MAX_SECONDS: u32 = 300;
const DECODE_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpectralVerdict {
    /// Content reaches the top of the band, or ends at a CD's filter.
    Genuine,
    /// The spectrum ends in a lossy encoder's cliff.
    Transcode,
    /// Too quiet, too short, or no treble to judge by.
    Inconclusive,
}

impl SpectralVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Genuine => "genuine",
            Self::Transcode => "transcode",
            Self::Inconclusive => "inconclusive",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "genuine" => Some(Self::Genuine),
            "transcode" => Some(Self::Transcode),
            "inconclusive" => Some(Self::Inconclusive),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SpectralResult {
    pub verdict: SpectralVerdict,
    /// Frequency where the spectrum falls off a cliff, if it does.
    pub cutoff_hz: Option<u32>,
    /// 0.0 to 1.0: how sure the verdict is.
    pub confidence: f32,
    /// One plain sentence for people.
    pub detail: String,
}

impl SpectralResult {
    fn inconclusive(detail: impl Into<String>) -> Self {
        Self {
            verdict: SpectralVerdict::Inconclusive,
            cutoff_hz: None,
            confidence: 0.0,
            detail: detail.into(),
        }
    }

    /// A transcode the check is sure enough of to say so.
    pub fn is_confident_transcode(&self) -> bool {
        self.verdict == SpectralVerdict::Transcode && self.confidence >= 0.6
    }
}

/// The lossy bitrate a cutoff usually comes from (LAME's default
/// low-pass filters), to grade a fake-lossless file like the MP3 it was.
pub fn likely_source_kbps(cutoff_hz: u32) -> u32 {
    match cutoff_hz {
        0..=11_999 => 64,
        12_000..=15_499 => 96,
        15_500..=16_999 => 128,
        17_000..=18_499 => 160,
        18_500..=19_249 => 192,
        19_250..=19_749 => 256,
        _ => 320,
    }
}

/// `16 kHz`, `19.5 kHz`.
pub fn format_khz(hz: u32) -> String {
    let khz = f64::from(hz) / 1000.0;
    let text = format!("{khz:.1}");
    format!("{} kHz", text.trim_end_matches(".0"))
}

/* ------------------------------------------------------------------ */
/* FFT                                                                 */
/* ------------------------------------------------------------------ */

/// In-place iterative radix-2 FFT; `re.len()` must be a power of two.
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    debug_assert!(n.is_power_of_two() && im.len() == n);
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut length = 2;
    while length <= n {
        let angle = -2.0 * std::f64::consts::PI / length as f64;
        let (w_im, w_re) = angle.sin_cos();
        for start in (0..n).step_by(length) {
            let (mut cur_re, mut cur_im) = (1.0f64, 0.0f64);
            for k in 0..length / 2 {
                let a = start + k;
                let b = a + length / 2;
                let t_re = re[b] * cur_re - im[b] * cur_im;
                let t_im = re[b] * cur_im + im[b] * cur_re;
                re[b] = re[a] - t_re;
                im[b] = im[a] - t_im;
                re[a] += t_re;
                im[a] += t_im;
                let next_re = cur_re * w_re - cur_im * w_im;
                cur_im = cur_re * w_im + cur_im * w_re;
                cur_re = next_re;
            }
        }
        length <<= 1;
    }
}

/// Power spectrum (one-sided, `FFT_SIZE / 2` bins) of one Hann-windowed
/// frame.
fn power_spectrum(frame: &[f32], window: &[f64]) -> Vec<f64> {
    let mut re: Vec<f64> = frame
        .iter()
        .zip(window)
        .map(|(sample, weight)| f64::from(*sample) * weight)
        .collect();
    let mut im = vec![0.0; re.len()];
    fft(&mut re, &mut im);
    re.iter()
        .zip(&im)
        .take(FFT_SIZE / 2)
        .map(|(r, i)| r * r + i * i)
        .collect()
}

/// Mean level of 250 Hz bands, in dB, from 0 Hz to [`TOP_HZ`].
fn band_levels(power: &[f64], rate: u32) -> Vec<f64> {
    let bin_hz = f64::from(rate) / FFT_SIZE as f64;
    let bands = (TOP_HZ / BAND_HZ) as usize;
    (0..bands)
        .map(|band| {
            let start = ((band as f64 * BAND_HZ) / bin_hz).floor() as usize;
            let end = (((band + 1) as f64 * BAND_HZ) / bin_hz).floor() as usize;
            let end = end.min(power.len()).max(start + 1);
            let mean = power[start..end].iter().sum::<f64>() / (end - start) as f64;
            10.0 * (mean + 1e-20).log10()
        })
        .collect()
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return f64::NEG_INFINITY;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

/// The sharpest cliff in a band-level curve: `(band index where the drop
/// starts, drop in dB, gap to the loudest band above)`.
fn find_cliff(levels: &[f64]) -> Option<(usize, f64, f64)> {
    let first = (LOWEST_CUTOFF_HZ / BAND_HZ) as usize;
    let last = levels.len().saturating_sub(3);
    let mut best: Option<(usize, f64, f64)> = None;
    for boundary in first.max(4)..last {
        let below = mean(&levels[boundary - 4..boundary]);
        let above_end = (boundary + 5).min(levels.len());
        let above = mean(&levels[boundary + 1..above_end]);
        let drop = below - above;
        let loudest_above = levels[boundary + 1..]
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let gap = below - loudest_above;
        if best.is_none_or(|(_, best_drop, _)| drop > best_drop) {
            best = Some((boundary, drop, gap));
        }
    }
    best
}

/// Judges decoded mono samples at `rate`. `source_rate` is the file's own
/// sample rate: a file recorded below 44.1 kHz cannot have the top octave.
pub fn analyze_samples(samples: &[f32], rate: u32, source_rate: Option<u32>) -> SpectralResult {
    if let Some(source_rate) = source_rate
        && source_rate < 44_000
    {
        return SpectralResult::inconclusive(format!(
            "Recorded at {}, which cannot carry the top of the band, so the check cannot tell.",
            format_khz(source_rate)
        ));
    }
    if samples.len() < FFT_SIZE * MIN_WINDOWS {
        return SpectralResult::inconclusive("Too short to check.");
    }
    let window: Vec<f64> = (0..FFT_SIZE)
        .map(|index| {
            0.5 - 0.5 * (2.0 * std::f64::consts::PI * index as f64 / (FFT_SIZE - 1) as f64).cos()
        })
        .collect();
    // Windows spread over the track, skipping the first and last 5%.
    let usable_start = samples.len() / 20;
    let usable_end = samples.len() - samples.len() / 20;
    let span = usable_end.saturating_sub(usable_start + FFT_SIZE);
    let count = MAX_WINDOWS.min(span / FFT_SIZE + 1).max(1);
    let mut total = vec![0.0f64; FFT_SIZE / 2];
    let mut per_window_cliffs = Vec::new();
    let mut used = 0usize;
    for index in 0..count {
        let start = usable_start
            + if count > 1 {
                span * index / (count - 1)
            } else {
                0
            };
        let frame = &samples[start..start + FFT_SIZE];
        let energy = frame
            .iter()
            .map(|sample| f64::from(*sample) * f64::from(*sample))
            .sum::<f64>()
            / FFT_SIZE as f64;
        let rms_db = 10.0 * (energy + 1e-20).log10();
        if rms_db < QUIET_WINDOW_DBFS {
            continue;
        }
        let power = power_spectrum(frame, &window);
        if let Some((boundary, drop, _)) = find_cliff(&band_levels(&power, rate)) {
            per_window_cliffs.push((boundary, drop));
        }
        for (sum, value) in total.iter_mut().zip(&power) {
            *sum += value;
        }
        used += 1;
    }
    if used < MIN_WINDOWS {
        return SpectralResult::inconclusive("Mostly silence, so there is too little to check.");
    }
    for value in &mut total {
        *value /= used as f64;
    }
    let levels = band_levels(&total, rate);
    let mid = mean(&levels[(2_000.0 / BAND_HZ) as usize..(8_000.0 / BAND_HZ) as usize]);
    let loudest = levels.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if loudest - mid > 60.0 {
        // Bass and a melody, but nothing above a few kHz (a test tone, a
        // solo instrument): no treble to judge by.
        return SpectralResult::inconclusive(
            "There is almost no treble in this recording, so the check cannot tell.",
        );
    }
    let Some((boundary, drop, gap)) = find_cliff(&levels) else {
        return SpectralResult::inconclusive("The spectrum could not be read.");
    };
    let cutoff_hz = (boundary as f64 * BAND_HZ).round() as u32;
    let below = mean(&levels[boundary - 4..boundary]);
    // A cliff: a steep fall, and nothing loud anywhere above it.
    let cliff = drop >= 25.0 && gap >= 18.0;
    if cliff && cutoff_hz < GENUINE_CUTOFF_HZ {
        // No treble at all before the "cliff": the recording itself may
        // simply stop there (old tapes, quiet passages).
        if mid - below > 60.0 {
            return SpectralResult::inconclusive(
                "There is almost no treble in this recording, so the check cannot tell.",
            );
        }
        let consistency = if per_window_cliffs.is_empty() {
            0.0
        } else {
            per_window_cliffs
                .iter()
                .filter(|(window_boundary, window_drop)| {
                    window_boundary.abs_diff(boundary) <= 2 && *window_drop >= 15.0
                })
                .count() as f64
                / per_window_cliffs.len() as f64
        };
        let strength = ((drop - 20.0) / 30.0).clamp(0.0, 1.0);
        let mut confidence = 0.55 * strength + 0.45 * consistency;
        if cutoff_hz >= 19_000 {
            // 19 to 20.5 kHz: a 256-320 kbps encode, or an unusually steep
            // filter on a genuine master. Less sure.
            confidence *= 0.75;
        }
        let confidence = confidence.clamp(0.0, 0.99) as f32;
        return SpectralResult {
            verdict: SpectralVerdict::Transcode,
            cutoff_hz: Some(cutoff_hz),
            confidence,
            detail: format!(
                "The spectrum stops at {} like a {} kbps lossy encode, so this lossless file was probably made from a lossy copy.",
                format_khz(cutoff_hz),
                likely_source_kbps(cutoff_hz)
            ),
        };
    }
    // No early cliff. Is there treble at all?
    let top = mean(&levels[(18_000.0 / BAND_HZ) as usize..(20_000.0 / BAND_HZ) as usize]);
    if mid - top > 75.0 {
        return SpectralResult::inconclusive(
            "There is almost no treble in this recording, so the check cannot tell.",
        );
    }
    let confidence = (1.0 - ((mid - top) - 30.0).max(0.0) / 60.0).clamp(0.5, 0.95) as f32;
    SpectralResult {
        verdict: SpectralVerdict::Genuine,
        cutoff_hz: cliff.then_some(cutoff_hz),
        confidence,
        detail: if cliff {
            format!(
                "Content reaches {}, where a CD's own filter ends it: no sign of a lossy source.",
                format_khz(cutoff_hz)
            )
        } else {
            "Content reaches the top of the band: no sign of a lossy source.".to_string()
        },
    }
}

/// Decodes the first audio stream to mono `f32` at [`SPECTRAL_RATE`].
async fn decode(path: &Path) -> Result<Vec<f32>, String> {
    let mut command = tokio::process::Command::new("ffmpeg");
    command
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-map", "0:a:0", "-t"])
        .arg(MAX_SECONDS.to_string())
        .args(["-ac", "1", "-ar"])
        .arg(SPECTRAL_RATE.to_string())
        .args(["-f", "f32le", "-"])
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|error| format!("could not start ffmpeg: {error}"))?;
    let mut stdout = child.stdout.take().ok_or("ffmpeg has no output")?;
    let limit = (SPECTRAL_RATE * MAX_SECONDS * 4) as usize;
    let read = async {
        let mut buffer = Vec::new();
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            let count = stdout.read(&mut chunk).await.map_err(|e| e.to_string())?;
            if count == 0 || buffer.len() >= limit {
                break;
            }
            buffer.extend_from_slice(&chunk[..count.min(limit - buffer.len())]);
        }
        Ok::<_, String>(buffer)
    };
    let bytes = tokio::time::timeout(DECODE_TIMEOUT, read)
        .await
        .map_err(|_| "decoding took too long".to_string())??;
    let _ = child.kill().await;
    if bytes.len() < 4 {
        return Err("ffmpeg could not decode the audio stream".into());
    }
    Ok(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect())
}

/// Checks one file. `source_rate` is its sample rate, when known.
pub async fn check_file(path: &Path, source_rate: Option<u32>) -> Result<SpectralResult, String> {
    let samples = decode(path).await?;
    Ok(analyze_samples(&samples, SPECTRAL_RATE, source_rate))
}

/// Whether a codec or extension is lossless, so worth checking.
pub fn worth_checking(codec: Option<&str>, extension: &str) -> bool {
    match codec {
        Some(codec) => crate::domain::codec_is_lossless(codec) == Some(true) && codec != "dsd",
        None => matches!(
            extension.to_ascii_lowercase().as_str(),
            "flac" | "wav" | "aif" | "aiff" | "alac" | "ape" | "wv" | "tta"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic white noise with a slow loudness swell.
    fn noise(seconds: f64, seed: u64) -> Vec<f32> {
        let mut state = seed;
        let count = (seconds * f64::from(SPECTRAL_RATE)) as usize;
        (0..count)
            .map(|index| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let white = (state >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0;
                let t = index as f64 / f64::from(SPECTRAL_RATE);
                (white * 0.25 * (0.6 + 0.4 * (t * 0.7).sin())) as f32
            })
            .collect()
    }

    /// A brick-wall low-pass, as a lossy encoder applies, done block by
    /// block in the frequency domain.
    fn brickwall(samples: &[f32], cutoff_hz: f64) -> Vec<f32> {
        let n = 16_384;
        let bin_hz = f64::from(SPECTRAL_RATE) / n as f64;
        let mut out = Vec::with_capacity(samples.len());
        for block in samples.chunks(n) {
            let mut re: Vec<f64> = block.iter().map(|s| f64::from(*s)).collect();
            re.resize(n, 0.0);
            let mut im = vec![0.0; n];
            fft(&mut re, &mut im);
            for bin in 0..n {
                let frequency = bin.min(n - bin) as f64 * bin_hz;
                if frequency > cutoff_hz {
                    re[bin] = 0.0;
                    im[bin] = 0.0;
                }
            }
            // Inverse through the forward transform of the conjugate.
            for value in &mut im {
                *value = -*value;
            }
            fft(&mut re, &mut im);
            out.extend(re.iter().take(block.len()).map(|v| (*v / n as f64) as f32));
        }
        out
    }

    #[test]
    fn fft_finds_a_pure_tone() {
        let mut re: Vec<f64> = (0..1024)
            .map(|i| (2.0 * std::f64::consts::PI * 64.0 * i as f64 / 1024.0).sin())
            .collect();
        let mut im = vec![0.0; 1024];
        fft(&mut re, &mut im);
        let peak = (0..512)
            .max_by(|a, b| {
                (re[*a].hypot(im[*a]))
                    .partial_cmp(&re[*b].hypot(im[*b]))
                    .unwrap()
            })
            .unwrap();
        assert_eq!(peak, 64);
    }

    #[test]
    fn full_band_noise_is_genuine() {
        let result = analyze_samples(&noise(12.0, 7), SPECTRAL_RATE, Some(44_100));
        assert_eq!(result.verdict, SpectralVerdict::Genuine, "{result:?}");
        assert!(result.confidence >= 0.5);
    }

    #[test]
    fn a_16khz_brickwall_is_a_confident_transcode() {
        let filtered = brickwall(&noise(12.0, 9), 16_000.0);
        let result = analyze_samples(&filtered, SPECTRAL_RATE, Some(44_100));
        assert_eq!(result.verdict, SpectralVerdict::Transcode, "{result:?}");
        let cutoff = result.cutoff_hz.expect("cutoff");
        assert!((15_500..=16_500).contains(&cutoff), "{cutoff}");
        assert!(result.is_confident_transcode(), "{result:?}");
        assert_eq!(likely_source_kbps(cutoff), 128);
        assert!(result.detail.contains("16 kHz"), "{}", result.detail);
    }

    #[test]
    fn a_cd_filter_near_22khz_is_not_a_transcode() {
        let filtered = brickwall(&noise(12.0, 11), 21_300.0);
        let result = analyze_samples(&filtered, SPECTRAL_RATE, Some(44_100));
        assert_eq!(result.verdict, SpectralVerdict::Genuine, "{result:?}");
    }

    #[test]
    fn silence_short_files_and_low_rates_are_inconclusive() {
        let silence = vec![0.0f32; SPECTRAL_RATE as usize * 10];
        assert_eq!(
            analyze_samples(&silence, SPECTRAL_RATE, None).verdict,
            SpectralVerdict::Inconclusive
        );
        assert_eq!(
            analyze_samples(&noise(0.2, 3), SPECTRAL_RATE, None).verdict,
            SpectralVerdict::Inconclusive
        );
        let low = analyze_samples(&noise(10.0, 3), SPECTRAL_RATE, Some(32_000));
        assert_eq!(low.verdict, SpectralVerdict::Inconclusive);
        assert!(low.detail.contains("32 kHz"));
    }

    #[test]
    fn a_pure_tone_has_no_treble_to_judge_by() {
        let tone: Vec<f32> = (0..SPECTRAL_RATE as usize * 10)
            .map(|i| {
                (0.5 * (2.0 * std::f64::consts::PI * 440.0 * i as f64 / 44_100.0).sin()) as f32
            })
            .collect();
        let result = analyze_samples(&tone, SPECTRAL_RATE, Some(44_100));
        assert_eq!(result.verdict, SpectralVerdict::Inconclusive, "{result:?}");
    }

    #[test]
    fn verdicts_round_trip_and_lossless_files_are_worth_checking() {
        for verdict in [
            SpectralVerdict::Genuine,
            SpectralVerdict::Transcode,
            SpectralVerdict::Inconclusive,
        ] {
            assert_eq!(SpectralVerdict::parse(verdict.as_str()), Some(verdict));
        }
        assert!(worth_checking(Some("flac"), "flac"));
        assert!(worth_checking(Some("pcm"), "wav"));
        assert!(!worth_checking(Some("mp3"), "mp3"));
        assert!(!worth_checking(Some("dsd"), "dsf"));
        assert!(worth_checking(None, "FLAC"));
        assert_eq!(format_khz(19_500), "19.5 kHz");
        assert_eq!(format_khz(16_000), "16 kHz");
    }

    /// Real files made by ffmpeg: a FLAC of full-band noise, and the same
    /// noise squeezed through a 128 kbps MP3 and saved as FLAC again.
    #[tokio::test]
    async fn ffmpeg_made_real_and_transcoded_flacs_are_told_apart() {
        let dir = std::env::temp_dir().join(format!("mujin-spectral-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            let status = std::process::Command::new("ffmpeg")
                .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y"])
                .args(args)
                .status()
                .expect("ffmpeg is installed (the CI installs it)");
            assert!(status.success(), "ffmpeg {args:?}");
        };
        let real = dir.join("real.flac");
        let mp3 = dir.join("lossy.mp3");
        let fake = dir.join("fake.flac");
        let source = "anoisesrc=color=pink:duration=12:sample_rate=44100:amplitude=0.4:seed=5";
        run(&[
            "-f",
            "lavfi",
            "-i",
            source,
            "-ac",
            "2",
            real.to_str().unwrap(),
        ]);
        run(&[
            "-i",
            real.to_str().unwrap(),
            "-c:a",
            "libmp3lame",
            "-b:a",
            "128k",
            mp3.to_str().unwrap(),
        ]);
        run(&["-i", mp3.to_str().unwrap(), fake.to_str().unwrap()]);

        let genuine = check_file(&real, Some(44_100)).await.expect("real decodes");
        let transcode = check_file(&fake, Some(44_100)).await.expect("fake decodes");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(genuine.verdict, SpectralVerdict::Genuine, "{genuine:?}");
        assert_eq!(
            transcode.verdict,
            SpectralVerdict::Transcode,
            "{transcode:?}"
        );
        assert!(transcode.is_confident_transcode(), "{transcode:?}");
        let cutoff = transcode.cutoff_hz.unwrap();
        assert!((15_000..=18_500).contains(&cutoff), "{cutoff}");
    }
}
