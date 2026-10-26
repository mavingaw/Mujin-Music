//! Parson's DSP engine for server outputs.
//!
//! Browsers run the DSP chain in a Web Audio graph, which always ends in
//! the operating system's shared mixer. A server output instead processes
//! the samples here, before they reach the exclusively opened device, so
//! an equalised track still arrives at the DAC exactly as Parson computed
//! it, with no mixer or second resampler after it.
//!
//! Everything runs in 64-bit float, in the order Roon's MUSE uses:
//!
//! 1. decode (ffmpeg, to `f64le` at the source rate),
//! 2. volume leveling and headroom adjustment (one gain),
//! 3. the parametric EQ (RBJ biquads, transposed direct form II),
//! 4. crossfeed (Bauer-style: low-passed, delayed feed to the other ear),
//! 5. convolution (an FIR filter such as room correction, see
//!    [`crate::convolution`]),
//! 6. sample rate conversion (polyphase windowed-sinc, linear phase),
//! 7. TPDF dither to the device's integer format.
//!
//! The settings are the same plain data the SDK's DSP panel stores per
//! output (`DspSettings` in `parson-music-sdk`), so a chain set up in the
//! browser is applied by the server unchanged.

use std::collections::HashMap;
use std::io::{self, Read};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::convolution::{Convolver, FilterSummary, ImpulseResponse};
use crate::player::InputFormat;
use crate::{DepthChange, DeviceCaps, OutputPlan, OutputSettings, SourceFormat};

/* ------------------------------------------------------------------ */
/* Settings (shared with the SDK)                                      */
/* ------------------------------------------------------------------ */

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EqFilterType {
    Peaking,
    Lowshelf,
    Highshelf,
    Lowpass,
    Highpass,
    Notch,
}

impl EqFilterType {
    fn has_gain(self) -> bool {
        matches!(self, Self::Peaking | Self::Lowshelf | Self::Highshelf)
    }

    fn label(self) -> &'static str {
        match self {
            Self::Peaking => "peak",
            Self::Lowshelf => "low shelf",
            Self::Highshelf => "high shelf",
            Self::Lowpass => "low pass",
            Self::Highpass => "high pass",
            Self::Notch => "notch",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EqBand {
    #[serde(default)]
    pub id: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(rename = "type")]
    pub filter_type: EqFilterType,
    /// Centre or corner frequency in Hz.
    pub frequency: f64,
    /// Ignored by pass and notch filters.
    #[serde(default)]
    pub gain_db: f64,
    #[serde(default = "unit_q")]
    pub q: f64,
}

fn yes() -> bool {
    true
}

fn unit_q() -> f64 {
    1.0
}

impl EqBand {
    /// e.g. `80 Hz low shelf +5.0 dB`, as the SDK writes it.
    pub fn describe(&self) -> String {
        let at = format_frequency(self.frequency);
        if self.filter_type.has_gain() {
            format!(
                "{at} {} {}",
                self.filter_type.label(),
                format_db(self.gain_db)
            )
        } else {
            format!("{at} {}", self.filter_type.label())
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CrossfeedStrength {
    Light,
    Medium,
    Strong,
}

impl CrossfeedStrength {
    /// `(cutoff Hz, feed dB, delay ms)`, matching the SDK's table.
    pub fn parameters(self) -> (f64, f64, f64) {
        match self {
            Self::Light => (700.0, -9.5, 0.3),
            Self::Medium => (700.0, -6.0, 0.3),
            Self::Strong => (650.0, -4.5, 0.35),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Medium => "Medium",
            Self::Strong => "Strong",
        }
    }
}

/// `device` keeps the output's rate; `source` follows each track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateMode {
    Device,
    Source,
    Fixed(u32),
}

impl Serialize for RateMode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Device => serializer.serialize_str("device"),
            Self::Source => serializer.serialize_str("source"),
            Self::Fixed(rate) => serializer.serialize_u32(*rate),
        }
    }
}

impl<'de> Deserialize<'de> for RateMode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Text(String),
            Number(f64),
        }
        Ok(match Raw::deserialize(deserializer)? {
            Raw::Text(text) if text == "source" => Self::Source,
            Raw::Number(rate) if rate.is_finite() && DSP_SAMPLE_RATES.contains(&(rate as u32)) => {
                Self::Fixed(rate as u32)
            }
            _ => Self::Device,
        })
    }
}

/// Fixed processing rates the DSP panel offers.
pub const DSP_SAMPLE_RATES: [u32; 6] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct HeadroomSettings {
    pub enabled: bool,
    pub gain_db: f64,
}

impl Default for HeadroomSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            gain_db: -3.0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct EqSettings {
    pub enabled: bool,
    pub preset: String,
    pub bands: Vec<EqBand>,
}

const FIVE_BAND: [f64; 5] = [80.0, 250.0, 1_000.0, 4_000.0, 10_000.0];

impl Default for EqSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            preset: "flat".into(),
            bands: FIVE_BAND
                .iter()
                .enumerate()
                .map(|(index, frequency)| EqBand {
                    id: format!("band-{}", index + 1),
                    enabled: true,
                    filter_type: match index {
                        0 => EqFilterType::Lowshelf,
                        4 => EqFilterType::Highshelf,
                        _ => EqFilterType::Peaking,
                    },
                    frequency: *frequency,
                    gain_db: 0.0,
                    q: if index == 0 || index == 4 { 0.71 } else { 1.0 },
                })
                .collect(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct CrossfeedSettings {
    pub enabled: bool,
    pub strength: CrossfeedStrength,
}

impl Default for CrossfeedSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            strength: CrossfeedStrength::Medium,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct SampleRateSettings {
    pub enabled: bool,
    pub mode: RateMode,
}

impl Default for SampleRateSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: RateMode::Device,
        }
    }
}

/// An FIR filter (room correction and the like), chosen from the filters
/// uploaded to the server.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ConvolutionSettings {
    pub enabled: bool,
    /// The uploaded filter's id.
    pub filter_id: Option<String>,
}

/// One output's DSP chain, in the SDK's `DspSettings` shape.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct DspSettings {
    /// Master switch: off bypasses every section.
    pub enabled: bool,
    pub headroom: HeadroomSettings,
    pub eq: EqSettings,
    pub crossfeed: CrossfeedSettings,
    pub convolution: ConvolutionSettings,
    pub sample_rate: SampleRateSettings,
}

impl Default for DspSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            headroom: HeadroomSettings::default(),
            eq: EqSettings::default(),
            crossfeed: CrossfeedSettings::default(),
            convolution: ConvolutionSettings::default(),
            sample_rate: SampleRateSettings::default(),
        }
    }
}

/// A filter id as the server writes them: lowercase hex and dashes.
pub fn valid_filter_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

fn clamp_finite(value: f64, fallback: f64, min: f64, max: f64) -> f64 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

impl DspSettings {
    /// Clamps every value to the ranges the SDK enforces.
    pub fn normalized(mut self) -> Self {
        self.headroom.gain_db = clamp_finite(self.headroom.gain_db, -3.0, -24.0, 0.0);
        self.eq.bands.truncate(10);
        for (index, band) in self.eq.bands.iter_mut().enumerate() {
            if band.id.is_empty() {
                band.id = format!("band-{}", index + 1);
            }
            band.frequency = clamp_finite(band.frequency, 1_000.0, 20.0, 20_000.0);
            band.gain_db = clamp_finite(band.gain_db, 0.0, -18.0, 18.0);
            band.q = clamp_finite(band.q, 1.0, 0.1, 18.0);
        }
        self.eq.preset.truncate(64);
        if self
            .convolution
            .filter_id
            .as_deref()
            .is_some_and(|id| !valid_filter_id(id))
        {
            self.convolution.filter_id = None;
        }
        self
    }

    /// The convolution filter the chain should run, if any.
    pub fn convolution_filter(&self) -> Option<&str> {
        (self.enabled && self.convolution.enabled)
            .then_some(self.convolution.filter_id.as_deref())
            .flatten()
    }

    pub fn headroom_db(&self) -> Option<f64> {
        (self.enabled && self.headroom.enabled && self.headroom.gain_db <= -0.05)
            .then_some(self.headroom.gain_db)
    }

    /// Bands that change the sound: enabled, and not a 0 dB peak or shelf.
    pub fn active_bands(&self) -> Vec<EqBand> {
        if !self.enabled || !self.eq.enabled {
            return Vec::new();
        }
        self.eq
            .bands
            .iter()
            .filter(|band| {
                band.enabled && (!band.filter_type.has_gain() || band.gain_db.abs() >= 0.05)
            })
            .cloned()
            .collect()
    }

    pub fn crossfeed(&self) -> Option<CrossfeedStrength> {
        (self.enabled && self.crossfeed.enabled).then_some(self.crossfeed.strength)
    }

    /// The processing rate asked for, `None` for the device's own.
    pub fn requested_rate(&self, source_rate: u32) -> Option<u32> {
        if !self.enabled || !self.sample_rate.enabled {
            return None;
        }
        match self.sample_rate.mode {
            RateMode::Device => None,
            RateMode::Source => Some(source_rate),
            RateMode::Fixed(rate) => Some(rate),
        }
    }

    /// Any section that changes the samples.
    pub fn processes(&self) -> bool {
        self.headroom_db().is_some()
            || !self.active_bands().is_empty()
            || self.crossfeed().is_some()
            || self.convolution_filter().is_some()
    }
}

/// `1 kHz`, `80 Hz`, as the SDK writes frequencies.
pub fn format_frequency(frequency: f64) -> String {
    if frequency >= 1_000.0 {
        let khz = frequency / 1_000.0;
        if khz.fract() == 0.0 {
            format!("{khz} kHz")
        } else {
            format!("{khz:.1} kHz")
        }
    } else {
        format!("{} Hz", frequency.round())
    }
}

/// `+1.5 dB`, `−6.0 dB` with a real minus sign.
pub fn format_db(db: f64) -> String {
    let rounded = (db * 10.0).round() / 10.0;
    if rounded == 0.0 {
        return "0.0 dB".into();
    }
    format!(
        "{}{:.1} dB",
        if rounded > 0.0 { "+" } else { "\u{2212}" },
        rounded.abs()
    )
}

/* ------------------------------------------------------------------ */
/* Planning                                                            */
/* ------------------------------------------------------------------ */

/// Which converter changes the rate.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResamplerKind {
    /// Parson's own 64-bit polyphase sinc, inside the chain.
    Parson,
    /// The decoder's resampler, for ratios too large for a polyphase
    /// table (it then runs before the EQ instead of after crossfeed).
    Decoder,
}

/// What the server-side chain will do to one track on one device.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ProcessingPlan {
    /// Volume leveling the client asked for, in dB.
    pub leveling_db: Option<f64>,
    pub headroom_db: Option<f64>,
    pub bands: Vec<EqBand>,
    pub crossfeed: Option<CrossfeedStrength>,
    /// Crossfeed is on but the stream is not stereo, so it is skipped.
    pub crossfeed_skipped: bool,
    pub channels: u8,
    pub input_rate: u32,
    pub output_rate: u32,
    /// The rate change comes from the DSP panel's sample rate choice
    /// rather than the device's limits.
    pub rate_requested: bool,
    /// The DSP panel asked for a rate the device does not accept.
    pub requested_rate_unavailable: Option<u32>,
    pub resampler: Option<ResamplerKind>,
    pub output_bits: u8,
    /// The FIR filter the chain convolves with.
    #[serde(default)]
    pub convolution: Option<ConvolutionPlan>,
    /// Why convolution is on but does not run for this track.
    #[serde(default)]
    pub convolution_skipped: Option<String>,
}

/// A convolution filter as the plan uses it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ConvolutionPlan {
    pub filter_id: String,
    pub name: String,
    pub taps: usize,
    /// The rate the filter was designed at.
    pub filter_rate: u32,
    /// The rate the chain convolves at (the filter is converted to it).
    pub run_rate: u32,
    pub channels: u8,
    pub peak_gain_db: f64,
}

/// An uploaded filter, ready for the chain.
#[derive(Clone, Debug)]
pub struct ConvolutionFilter {
    pub id: String,
    pub name: String,
    pub summary: FilterSummary,
    pub impulse: Arc<ImpulseResponse>,
}

impl ProcessingPlan {
    /// Total linear-domain gain before the EQ, in dB.
    pub fn gain_db(&self) -> f64 {
        self.leveling_db.unwrap_or(0.0) + self.headroom_db.unwrap_or(0.0)
    }
}

/// Largest polyphase table the chain builds (taps × phases).
const MAX_RESAMPLER_COEFFICIENTS: usize = 3_000_000;

fn allowed_rates(caps: &DeviceCaps, settings: &OutputSettings) -> Vec<u32> {
    caps.sample_rates
        .iter()
        .copied()
        .filter(|rate| settings.max_sample_rate.is_none_or(|max| *rate <= max))
        .collect()
}

/// Rates a fixed DSP rate may be set to on this device.
pub fn selectable_rates(caps: &DeviceCaps, settings: &OutputSettings) -> Vec<u32> {
    let allowed = allowed_rates(caps, settings);
    DSP_SAMPLE_RATES
        .iter()
        .copied()
        .filter(|rate| allowed.contains(rate))
        .collect()
}

/// Folds the DSP chain into `plan`. Returns `None`, leaving the plan as
/// it is (bit-perfect when it was), when nothing processes the samples.
///
/// With processing, the device gets its highest allowed bit depth (the
/// chain's output is float, so a wider word keeps more of it), the rate
/// the DSP panel asked for when the device accepts it, and the plan is no
/// longer bit-perfect.
pub fn plan_processing(
    plan: &mut OutputPlan,
    source: &SourceFormat,
    dsp: &DspSettings,
    leveling_db: Option<f64>,
    caps: &DeviceCaps,
    settings: &OutputSettings,
) -> Option<ProcessingPlan> {
    plan_processing_with(plan, source, dsp, leveling_db, caps, settings, None)
}

/// [`plan_processing`] with the convolution filter the settings name
/// (`None` when it is not on the server).
pub fn plan_processing_with(
    plan: &mut OutputPlan,
    source: &SourceFormat,
    dsp: &DspSettings,
    leveling_db: Option<f64>,
    caps: &DeviceCaps,
    settings: &OutputSettings,
    filter: Option<&ConvolutionFilter>,
) -> Option<ProcessingPlan> {
    let leveling_db = leveling_db.filter(|gain| gain.is_finite() && gain.abs() >= 0.005);
    let wanted_filter = dsp.convolution_filter();
    let (convolve, convolution_skipped) = match (wanted_filter, filter) {
        (None, _) => (None, None),
        (Some(_), None) => (
            None,
            Some("The convolution filter chosen for this output is no longer on the server; choose or upload it again.".to_string()),
        ),
        (Some(_), Some(filter)) => {
            let filter_channels = filter.impulse.channels.len();
            if filter_channels != 1 && filter_channels != usize::from(plan.channels) {
                (
                    None,
                    Some(format!(
                        "The convolution filter {} has {filter_channels} channels but this track plays in {}, so it is skipped.",
                        filter.name, plan.channels
                    )),
                )
            } else {
                (Some(filter), None)
            }
        }
    };
    let allowed = allowed_rates(caps, settings);
    let requested = dsp.requested_rate(source.sample_rate);
    let (output_rate, rate_requested, unavailable) = match requested {
        Some(rate) if allowed.contains(&rate) => (rate, rate != source.sample_rate, None),
        Some(rate) => (plan.sample_rate, false, Some(rate)),
        None => (plan.sample_rate, false, None),
    };
    let others =
        dsp.headroom_db().is_some() || !dsp.active_bands().is_empty() || dsp.crossfeed().is_some();
    let processes = leveling_db.is_some() || others || rate_requested || convolve.is_some();
    if !processes {
        return None;
    }
    let bits = caps
        .bit_depths()
        .into_iter()
        .filter(|bits| settings.max_bit_depth.is_none_or(|max| *bits <= max))
        .max()
        .unwrap_or(plan.bit_depth);
    if let Some(format) = caps.format_for_bits(bits) {
        plan.bit_depth = bits;
        plan.sample_format = format;
    }
    plan.sample_rate = output_rate;
    plan.resample =
        (output_rate != source.sample_rate).then_some((source.sample_rate, output_rate));
    plan.depth = DepthChange::Quantize { to: plan.bit_depth };
    plan.bit_perfect = false;
    let crossfeed = dsp.crossfeed();
    let resampler = plan.resample.map(|(from, to)| {
        if Resampler::coefficient_count(from, to) <= MAX_RESAMPLER_COEFFICIENTS {
            ResamplerKind::Parson
        } else {
            ResamplerKind::Decoder
        }
    });
    Some(ProcessingPlan {
        leveling_db,
        headroom_db: dsp.headroom_db(),
        bands: dsp.active_bands(),
        crossfeed: crossfeed.filter(|_| plan.channels == 2),
        crossfeed_skipped: crossfeed.is_some() && plan.channels != 2,
        channels: plan.channels,
        input_rate: source.sample_rate,
        output_rate,
        rate_requested,
        requested_rate_unavailable: unavailable,
        convolution: convolve.map(|filter| ConvolutionPlan {
            filter_id: filter.id.clone(),
            name: filter.name.clone(),
            taps: filter.summary.taps,
            filter_rate: filter.summary.sample_rate,
            run_rate: if resampler == Some(ResamplerKind::Decoder) {
                output_rate
            } else {
                source.sample_rate
            },
            channels: filter.summary.channels,
            peak_gain_db: filter.summary.peak_gain_db,
        }),
        convolution_skipped,
        resampler,
        output_bits: plan.bit_depth,
    })
}

/* ------------------------------------------------------------------ */
/* Filters                                                             */
/* ------------------------------------------------------------------ */

/// Normalised biquad coefficients `[b0, b1, b2, a1, a2]`.
pub fn biquad_coefficients(
    filter_type: EqFilterType,
    frequency: f64,
    gain_db: f64,
    q: f64,
    sample_rate: f64,
) -> [f64; 5] {
    use std::f64::consts::{PI, SQRT_2};
    let nyquist = sample_rate / 2.0;
    let f0 = frequency.min(nyquist * 0.999);
    let w0 = 2.0 * PI * f0 / sample_rate;
    let (sin, cos) = w0.sin_cos();
    let a = 10f64.powf(gain_db / 40.0);
    let q = q.max(1e-4);
    let (b0, b1, b2, a0, a1, a2) = match filter_type {
        EqFilterType::Lowpass => {
            let alpha = sin / (2.0 * q);
            (
                (1.0 - cos) / 2.0,
                1.0 - cos,
                (1.0 - cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            )
        }
        EqFilterType::Highpass => {
            let alpha = sin / (2.0 * q);
            (
                (1.0 + cos) / 2.0,
                -(1.0 + cos),
                (1.0 + cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            )
        }
        EqFilterType::Notch => {
            let alpha = sin / (2.0 * q);
            (1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
        }
        EqFilterType::Lowshelf | EqFilterType::Highshelf => {
            // Slope 1, as Web Audio's shelves (and the SDK's curve) use.
            let alpha = sin / 2.0 * SQRT_2;
            let root = 2.0 * a.sqrt() * alpha;
            if filter_type == EqFilterType::Lowshelf {
                (
                    a * (a + 1.0 - (a - 1.0) * cos + root),
                    2.0 * a * (a - 1.0 - (a + 1.0) * cos),
                    a * (a + 1.0 - (a - 1.0) * cos - root),
                    a + 1.0 + (a - 1.0) * cos + root,
                    -2.0 * (a - 1.0 + (a + 1.0) * cos),
                    a + 1.0 + (a - 1.0) * cos - root,
                )
            } else {
                (
                    a * (a + 1.0 + (a - 1.0) * cos + root),
                    -2.0 * a * (a - 1.0 + (a + 1.0) * cos),
                    a * (a + 1.0 + (a - 1.0) * cos - root),
                    a + 1.0 - (a - 1.0) * cos + root,
                    2.0 * (a - 1.0 - (a + 1.0) * cos),
                    a + 1.0 - (a - 1.0) * cos - root,
                )
            }
        }
        EqFilterType::Peaking => {
            let alpha = sin / (2.0 * q);
            (
                1.0 + alpha * a,
                -2.0 * cos,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos,
                1.0 - alpha / a,
            )
        }
    };
    [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0]
}

/// One biquad in transposed direct form II, 64-bit state.
#[derive(Clone, Debug)]
struct Biquad {
    c: [f64; 5],
    z1: f64,
    z2: f64,
}

impl Biquad {
    fn new(c: [f64; 5]) -> Self {
        Self {
            c,
            z1: 0.0,
            z2: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let [b0, b1, b2, a1, a2] = self.c;
        let y = b0 * x + self.z1;
        self.z1 = b1 * x - a1 * y + self.z2;
        self.z2 = b2 * x - a2 * y;
        y
    }
}

/// Bauer-style crossfeed on a stereo pair: each ear gets the direct
/// signal plus a low-passed, delayed share of the other channel, scaled
/// so a centred source keeps its level.
struct Crossfeed {
    direct: f64,
    cross: f64,
    lowpass: [Biquad; 2],
    /// Ring buffers of the low-passed signal, one per source channel.
    history: [Vec<f64>; 2],
    write: usize,
    delay: f64,
}

impl Crossfeed {
    fn new(strength: CrossfeedStrength, sample_rate: f64) -> Self {
        let (cutoff, feed_db, delay_ms) = strength.parameters();
        let feed = 10f64.powf(feed_db / 20.0);
        let lowpass = biquad_coefficients(
            EqFilterType::Lowpass,
            cutoff,
            0.0,
            std::f64::consts::FRAC_1_SQRT_2,
            sample_rate,
        );
        let delay = delay_ms / 1_000.0 * sample_rate;
        let size = delay.ceil() as usize + 2;
        Self {
            direct: 1.0 / (1.0 + feed),
            cross: feed / (1.0 + feed),
            lowpass: [Biquad::new(lowpass), Biquad::new(lowpass)],
            history: [vec![0.0; size], vec![0.0; size]],
            write: 0,
            delay,
        }
    }

    /// The low-passed channel `channel`, `delay` samples ago.
    fn delayed(&self, channel: usize) -> f64 {
        let size = self.history[channel].len();
        let whole = self.delay.floor() as usize;
        let fraction = self.delay - whole as f64;
        let newer = (self.write + size - whole) % size;
        let older = (newer + size - 1) % size;
        let history = &self.history[channel];
        history[newer] * (1.0 - fraction) + history[older] * fraction
    }

    #[inline]
    fn process(&mut self, left: f64, right: f64) -> (f64, f64) {
        let size = self.history[0].len();
        self.history[0][self.write] = self.lowpass[0].process(left);
        self.history[1][self.write] = self.lowpass[1].process(right);
        let out_left = self.direct * left + self.cross * self.delayed(1);
        let out_right = self.direct * right + self.cross * self.delayed(0);
        self.write = (self.write + 1) % size;
        (out_left, out_right)
    }
}

/* ------------------------------------------------------------------ */
/* Sample rate conversion                                              */
/* ------------------------------------------------------------------ */

/// Stopband attenuation of the converter's Kaiser-windowed sinc.
pub const RESAMPLER_STOPBAND_DB: f64 = 140.0;
/// The transition band, as a share of the lower rate's Nyquist frequency:
/// at 44.1 kHz the passband is flat to about 20.3 kHz.
const RESAMPLER_TRANSITION: f64 = 0.08;

/// e.g. `64-bit polyphase sinc, 140 dB`.
pub fn resampler_label() -> String {
    format!("64-bit polyphase sinc, {RESAMPLER_STOPBAND_DB:.0} dB")
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let half = x / 2.0;
    for k in 1..200 {
        term *= (half / k as f64) * (half / k as f64);
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

type DesignCache = HashMap<(u32, u32), Arc<ResamplerDesign>>;

struct ResamplerDesign {
    up: u64,
    down: u64,
    taps: usize,
    /// Phase-major and reversed within a phase, so a phase's taps line up
    /// with the input window oldest first: `coefficients[phase * taps +
    /// (taps - 1 - k)]` multiplies the sample `k` before the newest.
    coefficients: Vec<f64>,
    /// Centre of the prototype filter, in virtual (up-sampled) samples.
    centre: u64,
}

impl ResamplerDesign {
    fn dimensions(from: u32, to: u32) -> (u64, u64, usize, usize) {
        let g = gcd(u64::from(from), u64::from(to));
        let up = u64::from(to) / g;
        let down = u64::from(from) / g;
        let lower_nyquist = f64::from(from.min(to)) / 2.0;
        let virtual_rate = up as f64 * f64::from(from);
        let transition = lower_nyquist * RESAMPLER_TRANSITION / virtual_rate;
        let length = ((RESAMPLER_STOPBAND_DB - 7.95) / (14.36 * transition)).ceil() as usize + 1;
        let taps = length.div_ceil(up as usize).max(2);
        (up, down, taps, taps * up as usize)
    }

    fn new(from: u32, to: u32) -> Self {
        let (up, down, taps, table) = Self::dimensions(from, to);
        // An odd length puts the centre on a whole virtual sample.
        let length = if table % 2 == 0 { table - 1 } else { table };
        let centre = (length - 1) / 2;
        let lower_nyquist = f64::from(from.min(to)) / 2.0;
        let virtual_rate = up as f64 * f64::from(from);
        let cutoff = lower_nyquist * (1.0 - RESAMPLER_TRANSITION / 2.0) / virtual_rate;
        let beta = 0.1102 * (RESAMPLER_STOPBAND_DB - 8.7);
        let i0_beta = bessel_i0(beta);
        let mut prototype = vec![0.0; table];
        for (n, value) in prototype.iter_mut().enumerate().take(length) {
            let offset = n as f64 - centre as f64;
            let x = 2.0 * cutoff * offset;
            let sinc = if offset == 0.0 {
                1.0
            } else {
                (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
            };
            let ratio = offset / centre.max(1) as f64;
            let window = bessel_i0(beta * (1.0 - ratio * ratio).max(0.0).sqrt()) / i0_beta;
            *value = 2.0 * cutoff * sinc * window * up as f64;
        }
        let mut coefficients = vec![0.0; table];
        for phase in 0..up as usize {
            for k in 0..taps {
                coefficients[phase * taps + (taps - 1 - k)] = prototype[phase + k * up as usize];
            }
        }
        Self {
            up,
            down,
            taps,
            coefficients,
            centre: centre as u64,
        }
    }

    /// Designs are reused across seeks and tracks.
    fn shared(from: u32, to: u32) -> Arc<Self> {
        static CACHE: OnceLock<Mutex<DesignCache>> = OnceLock::new();
        let cache = CACHE.get_or_init(Default::default);
        if let Some(design) = cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&(from, to))
        {
            return Arc::clone(design);
        }
        let design = Arc::new(Self::new(from, to));
        let mut cache = cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if cache.len() >= 16 {
            cache.clear();
        }
        cache.insert((from, to), Arc::clone(&design));
        design
    }
}

/// Dot product with four independent accumulators, which the compiler
/// turns into vector instructions.
#[inline]
fn dot(samples: &[f64], coefficients: &[f64]) -> f64 {
    let (samples_4, samples_rest) = samples.as_chunks::<4>();
    let (coefficients_4, coefficients_rest) = coefficients.as_chunks::<4>();
    let mut lanes = [0.0f64; 4];
    for (sample, coefficient) in samples_4.iter().zip(coefficients_4) {
        for lane in 0..4 {
            lanes[lane] += sample[lane] * coefficient[lane];
        }
    }
    let mut sum = (lanes[0] + lanes[1]) + (lanes[2] + lanes[3]);
    for (sample, coefficient) in samples_rest.iter().zip(coefficients_rest) {
        sum += sample * coefficient;
    }
    sum
}

/// A streaming rational-ratio polyphase resampler, aligned so output
/// sample `j` is the input at time `j / to` (no added latency).
pub struct Resampler {
    design: Arc<ResamplerDesign>,
    channels: usize,
    /// Planar history; index 0 is absolute input sample `start`, offset by
    /// the `taps - 1` zeros the stream starts with.
    buffers: Vec<Vec<f64>>,
    start: u64,
    /// Virtual time of the next output sample.
    time: u64,
    inputs: u64,
    outputs: u64,
}

impl Resampler {
    pub fn new(from: u32, to: u32, channels: usize) -> Self {
        let design = ResamplerDesign::shared(from, to);
        let pad = design.taps - 1;
        Self {
            time: pad as u64 * design.up + design.centre,
            buffers: vec![vec![0.0; pad]; channels],
            design,
            channels,
            start: 0,
            inputs: 0,
            outputs: 0,
        }
    }

    /// Size of the polyphase table the conversion needs.
    pub fn coefficient_count(from: u32, to: u32) -> usize {
        ResamplerDesign::dimensions(from, to).3
    }

    /// Output frames owed for all input so far.
    fn expected(&self) -> u64 {
        (self.inputs * self.design.up).div_ceil(self.design.down)
    }

    fn run(&mut self, output: &mut Vec<f64>, limit: Option<u64>) {
        let design = &*self.design;
        let available = self.start + self.buffers[0].len() as u64;
        loop {
            if limit.is_some_and(|limit| self.outputs >= limit) {
                break;
            }
            let newest = self.time / design.up;
            if newest >= available {
                break;
            }
            let phase = (self.time % design.up) as usize;
            let coefficients = &design.coefficients[phase * design.taps..(phase + 1) * design.taps];
            let newest = (newest - self.start) as usize;
            for buffer in &self.buffers {
                output.push(dot(
                    &buffer[newest + 1 - design.taps..=newest],
                    coefficients,
                ));
            }
            self.time += design.down;
            self.outputs += 1;
        }
        // Drop history no later output needs.
        let needed = (self.time / design.up + 1).saturating_sub(design.taps as u64);
        if needed > self.start {
            let drop = ((needed - self.start) as usize).min(self.buffers[0].len());
            for buffer in &mut self.buffers {
                buffer.drain(..drop);
            }
            self.start += drop as u64;
        }
    }

    /// Converts interleaved frames, appending interleaved output.
    pub fn process(&mut self, input: &[f64], output: &mut Vec<f64>) {
        let frames = input.len() / self.channels;
        for (channel, buffer) in self.buffers.iter_mut().enumerate() {
            buffer.extend(
                input
                    .chunks_exact(self.channels)
                    .map(|frame| frame[channel]),
            );
        }
        self.inputs += frames as u64;
        let limit = self.expected();
        self.run(output, Some(limit));
    }

    /// Emits the tail once the input has ended.
    pub fn finish(&mut self, output: &mut Vec<f64>) {
        let limit = self.expected();
        let tail = self.design.taps + 1;
        for buffer in &mut self.buffers {
            buffer.extend(std::iter::repeat_n(0.0, tail));
        }
        self.run(output, Some(limit));
    }
}

/* ------------------------------------------------------------------ */
/* The chain                                                           */
/* ------------------------------------------------------------------ */

/// Live measurements of a running chain, shared with the API.
#[derive(Default, Debug)]
pub struct DspStats {
    frames: AtomicU64,
    rate: AtomicU64,
    busy_ns: AtomicU64,
    clipped: AtomicU64,
    peak_bits: AtomicU64,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct DspStatus {
    /// Seconds of audio processed per second of processing time (Roon's
    /// "processing speed"), once there is enough to measure.
    pub realtime_factor: Option<f64>,
    /// Highest level after the chain, before dither, in dBFS.
    pub peak_dbfs: Option<f64>,
    /// Samples that went past full scale and were limited.
    pub clipped_samples: u64,
    pub clipping: bool,
}

impl DspStats {
    pub fn snapshot(&self) -> DspStatus {
        let frames = self.frames.load(Ordering::Relaxed);
        let rate = self.rate.load(Ordering::Relaxed);
        let busy = self.busy_ns.load(Ordering::Relaxed);
        let audio_seconds = if rate > 0 {
            frames as f64 / rate as f64
        } else {
            0.0
        };
        let realtime_factor =
            (audio_seconds >= 0.05 && busy > 0).then(|| audio_seconds / (busy as f64 / 1e9));
        let peak = f64::from_bits(self.peak_bits.load(Ordering::Relaxed));
        let clipped = self.clipped.load(Ordering::Relaxed);
        DspStatus {
            realtime_factor,
            peak_dbfs: (frames > 0 && peak > 0.0).then(|| 20.0 * peak.log10()),
            clipped_samples: clipped,
            clipping: clipped > 0,
        }
    }

    pub fn reset(&self) {
        self.frames.store(0, Ordering::Relaxed);
        self.busy_ns.store(0, Ordering::Relaxed);
        self.clipped.store(0, Ordering::Relaxed);
        self.peak_bits.store(0, Ordering::Relaxed);
    }
}

/// Xorshift64* noise for TPDF dither.
struct Noise(u64);

impl Noise {
    #[inline]
    fn uniform(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Triangular noise in (−1, 1) LSB.
    #[inline]
    fn tpdf(&mut self) -> f64 {
        self.uniform() - self.uniform()
    }
}

/// Dithers `samples` into `bytes` in the player's input layout, limiting
/// (and counting) anything past full scale.
fn quantize(
    noise: &mut Noise,
    stats: &DspStats,
    bits: u8,
    output: InputFormat,
    samples: &[f64],
    bytes: &mut Vec<u8>,
) {
    let scale = f64::from(1u32 << (bits - 1));
    let shift = 32 - u32::from(bits);
    let mut peak = f64::from_bits(stats.peak_bits.load(Ordering::Relaxed));
    let mut clipped = 0;
    bytes.reserve(samples.len() * if output == InputFormat::S16 { 2 } else { 4 });
    for sample in samples {
        let magnitude = sample.abs();
        if magnitude > peak {
            peak = magnitude;
        }
        let limited = if magnitude > 1.0 {
            clipped += 1;
            sample.signum()
        } else {
            *sample
        };
        let value = (limited * scale + noise.tpdf())
            .round()
            .clamp(-scale, scale - 1.0) as i64;
        match output {
            InputFormat::S16 => {
                let value = if bits <= 16 {
                    value << (16 - bits)
                } else {
                    value >> (bits - 16)
                };
                bytes.extend_from_slice(&(value as i16).to_le_bytes());
            }
            // The chain never feeds a DSD container; it writes 32-bit words.
            InputFormat::S32 | InputFormat::Raw => {
                bytes.extend_from_slice(&((value << shift) as i32).to_le_bytes());
            }
        }
    }
    stats.peak_bits.store(peak.to_bits(), Ordering::Relaxed);
    if clipped > 0 {
        stats.clipped.fetch_add(clipped, Ordering::Relaxed);
    }
}

/// Processes 64-bit float frames through the chain.
pub struct DspChain {
    channels: usize,
    gain: f64,
    eq: Vec<Vec<Biquad>>,
    crossfeed: Option<Crossfeed>,
    convolver: Option<Convolver>,
    convolved: Vec<f64>,
    resampler: Option<Resampler>,
    bits: u8,
    output: InputFormat,
    noise: Noise,
    stats: Arc<DspStats>,
    scratch: Vec<f64>,
    resampled: Vec<f64>,
    input_rate: u32,
}

impl DspChain {
    /// Adds the convolution stage, with `impulse` converted to the rate
    /// the chain runs at.
    pub fn with_convolution(mut self, impulse: &ImpulseResponse) -> Result<Self, String> {
        let at_rate = impulse.at_rate(self.input_rate);
        self.convolver = Some(Convolver::new(&at_rate, self.channels)?);
        Ok(self)
    }

    /// `input_rate` is the rate frames arrive at (the decoder's, which is
    /// already the output rate when the decoder resamples).
    pub fn new(
        plan: &ProcessingPlan,
        input_rate: u32,
        output: InputFormat,
        stats: Arc<DspStats>,
    ) -> Self {
        let channels = usize::from(plan.channels.max(1));
        let rate = f64::from(input_rate);
        let eq = (0..channels)
            .map(|_| {
                plan.bands
                    .iter()
                    .map(|band| {
                        Biquad::new(biquad_coefficients(
                            band.filter_type,
                            band.frequency,
                            band.gain_db,
                            band.q,
                            rate,
                        ))
                    })
                    .collect()
            })
            .collect();
        stats
            .rate
            .store(u64::from(plan.output_rate), Ordering::Relaxed);
        Self {
            channels,
            gain: 10f64.powf(plan.gain_db() / 20.0),
            eq,
            crossfeed: plan
                .crossfeed
                .filter(|_| channels == 2)
                .map(|strength| Crossfeed::new(strength, rate)),
            convolver: None,
            convolved: Vec::new(),
            input_rate,
            resampler: (input_rate != plan.output_rate)
                .then(|| Resampler::new(input_rate, plan.output_rate, channels)),
            bits: plan.output_bits.clamp(8, 32),
            output,
            noise: Noise(0x9E37_79B9_7F4A_7C15),
            stats,
            scratch: Vec::new(),
            resampled: Vec::new(),
        }
    }

    fn filter(&mut self, input: &[f64]) {
        self.scratch.clear();
        self.scratch.reserve(input.len());
        for frame in input.chunks_exact(self.channels) {
            let start = self.scratch.len();
            for (channel, sample) in frame.iter().enumerate() {
                let mut value = sample * self.gain;
                for filter in &mut self.eq[channel] {
                    value = filter.process(value);
                }
                self.scratch.push(value);
            }
            if let Some(crossfeed) = &mut self.crossfeed {
                let (left, right) = crossfeed.process(self.scratch[start], self.scratch[start + 1]);
                self.scratch[start] = left;
                self.scratch[start + 1] = right;
            }
        }
        if let Some(convolver) = &mut self.convolver {
            self.convolved.clear();
            convolver.process(&self.scratch, &mut self.convolved);
            std::mem::swap(&mut self.scratch, &mut self.convolved);
        }
    }

    fn emit(&mut self, finishing: bool, bytes: &mut Vec<u8>) {
        let mut out = std::mem::take(&mut self.resampled);
        out.clear();
        let resampled = if let Some(resampler) = &mut self.resampler {
            if finishing {
                resampler.finish(&mut out);
            } else {
                resampler.process(&self.scratch, &mut out);
            }
            true
        } else {
            false
        };
        let samples: &[f64] = if resampled { &out } else { &self.scratch };
        quantize(
            &mut self.noise,
            &self.stats,
            self.bits,
            self.output,
            samples,
            bytes,
        );
        self.stats
            .frames
            .fetch_add((samples.len() / self.channels) as u64, Ordering::Relaxed);
        self.resampled = out;
    }

    /// Runs interleaved frames through every stage, appending device
    /// samples to `bytes`.
    pub fn process(&mut self, input: &[f64], bytes: &mut Vec<u8>) {
        let started = Instant::now();
        self.filter(input);
        self.emit(false, bytes);
        self.record(started);
    }

    /// Flushes the converter's tail at the end of the track.
    pub fn finish(&mut self, bytes: &mut Vec<u8>) {
        let started = Instant::now();
        self.scratch.clear();
        if let Some(convolver) = &mut self.convolver {
            convolver.flush(&mut self.scratch);
            if !self.scratch.is_empty() {
                self.emit(false, bytes);
                self.scratch.clear();
            }
        }
        if self.resampler.is_some() {
            self.emit(true, bytes);
        }
        self.record(started);
    }

    fn record(&self, started: Instant) {
        let elapsed = started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        self.stats
            .busy_ns
            .fetch_add(elapsed.max(1), Ordering::Relaxed);
    }
}

/// Wraps a decoder's `f64le` output in the chain, producing the PCM the
/// player expects.
pub struct DspPcm {
    inner: Box<dyn Read + Send>,
    chain: DspChain,
    frame_bytes: usize,
    raw: Vec<u8>,
    filled: usize,
    floats: Vec<f64>,
    pending: Vec<u8>,
    offset: usize,
    done: bool,
}

/// Frames processed per block.
const BLOCK_FRAMES: usize = 4096;

impl DspPcm {
    pub fn new(inner: Box<dyn Read + Send>, chain: DspChain) -> Self {
        let frame_bytes = chain.channels * 8;
        Self {
            inner,
            frame_bytes,
            raw: vec![0; BLOCK_FRAMES * frame_bytes],
            filled: 0,
            floats: Vec::with_capacity(BLOCK_FRAMES * chain.channels),
            chain,
            pending: Vec::new(),
            offset: 0,
            done: false,
        }
    }

    fn refill(&mut self) -> io::Result<()> {
        self.pending.clear();
        self.offset = 0;
        while self.pending.is_empty() && !self.done {
            let read = self.inner.read(&mut self.raw[self.filled..])?;
            if read == 0 {
                self.done = true;
                self.chain.finish(&mut self.pending);
                break;
            }
            self.filled += read;
            let whole = self.filled / self.frame_bytes * self.frame_bytes;
            if whole == 0 {
                continue;
            }
            self.floats.clear();
            self.floats.extend(
                self.raw[..whole]
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .map(|bytes| f64::from_le_bytes(*bytes)),
            );
            self.raw.copy_within(whole..self.filled, 0);
            self.filled -= whole;
            let floats = std::mem::take(&mut self.floats);
            self.chain.process(&floats, &mut self.pending);
            self.floats = floats;
        }
        Ok(())
    }
}

impl Read for DspPcm {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.offset >= self.pending.len() {
            if self.done {
                return Ok(0);
            }
            self.refill()?;
            if self.pending.is_empty() {
                return Ok(0);
            }
        }
        let count = buffer.len().min(self.pending.len() - self.offset);
        buffer[..count].copy_from_slice(&self.pending[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SampleFormat;

    fn plan(bands: Vec<EqBand>, input_rate: u32, output_rate: u32) -> ProcessingPlan {
        ProcessingPlan {
            leveling_db: None,
            headroom_db: None,
            bands,
            crossfeed: None,
            crossfeed_skipped: false,
            channels: 2,
            input_rate,
            output_rate,
            rate_requested: false,
            requested_rate_unavailable: None,
            resampler: None,
            output_bits: 24,
            convolution: None,
            convolution_skipped: None,
        }
    }

    fn sine(frequency: f64, rate: u32, frames: usize, amplitude: f64) -> Vec<f64> {
        (0..frames)
            .flat_map(|n| {
                let value = amplitude
                    * (2.0 * std::f64::consts::PI * frequency * n as f64 / f64::from(rate)).sin();
                [value, value]
            })
            .collect()
    }

    fn decode_s32(bytes: &[u8]) -> Vec<f64> {
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| f64::from(i32::from_le_bytes(*chunk)) / 2f64.powi(31))
            .collect()
    }

    fn rms(samples: &[f64]) -> f64 {
        (samples.iter().map(|value| value * value).sum::<f64>() / samples.len() as f64).sqrt()
    }

    fn left(samples: &[f64]) -> Vec<f64> {
        samples.iter().step_by(2).copied().collect()
    }

    #[test]
    fn settings_round_trip_the_sdk_shape() {
        let json = r#"{"enabled":true,"headroom":{"enabled":true,"gainDb":-4.5},
            "eq":{"enabled":true,"preset":"bass","bands":[{"id":"band-1","enabled":true,"type":"lowshelf","frequency":80,"gainDb":5,"q":0.71}]},
            "crossfeed":{"enabled":true,"strength":"strong"},"sampleRate":{"enabled":true,"mode":96000}}"#;
        let settings: DspSettings = serde_json::from_str(json).unwrap();
        assert_eq!(settings.headroom_db(), Some(-4.5));
        assert_eq!(settings.active_bands().len(), 1);
        assert_eq!(settings.crossfeed(), Some(CrossfeedStrength::Strong));
        assert_eq!(settings.requested_rate(44_100), Some(96_000));
        let back = serde_json::to_value(&settings).unwrap();
        assert_eq!(back["sampleRate"]["mode"], 96_000);
        assert_eq!(back["eq"]["bands"][0]["gainDb"], 5.0);
        assert_eq!(back["eq"]["bands"][0]["type"], "lowshelf");

        let odd: DspSettings = serde_json::from_str(
            r#"{"sampleRate":{"enabled":true,"mode":12345},"headroom":{"gainDb":-99}}"#,
        )
        .unwrap();
        assert_eq!(odd.sample_rate.mode, RateMode::Device);
        assert_eq!(odd.normalized().headroom.gain_db, -24.0);
        assert!(!DspSettings::default().processes());
    }

    #[test]
    fn biquads_match_the_rbj_cookbook() {
        // A peak's gain at its centre frequency is its gain.
        let c = biquad_coefficients(EqFilterType::Peaking, 1_000.0, 6.0, 1.0, 48_000.0);
        let w = 2.0 * std::f64::consts::PI * 1_000.0 / 48_000.0;
        let (re_n, im_n) = (
            c[0] + c[1] * w.cos() + c[2] * (2.0 * w).cos(),
            -(c[1] * w.sin() + c[2] * (2.0 * w).sin()),
        );
        let (re_d, im_d) = (
            1.0 + c[3] * w.cos() + c[4] * (2.0 * w).cos(),
            -(c[3] * w.sin() + c[4] * (2.0 * w).sin()),
        );
        let db = 10.0 * ((re_n * re_n + im_n * im_n) / (re_d * re_d + im_d * im_d)).log10();
        assert!((db - 6.0).abs() < 1e-9, "{db}");
    }

    #[test]
    fn the_eq_boosts_in_float_and_dither_lands_on_the_device_depth() {
        let band = EqBand {
            id: "b".into(),
            enabled: true,
            filter_type: EqFilterType::Peaking,
            frequency: 1_000.0,
            gain_db: 6.0,
            q: 1.0,
        };
        let stats = Arc::new(DspStats::default());
        let mut chain = DspChain::new(
            &plan(vec![band], 48_000, 48_000),
            48_000,
            InputFormat::S32,
            Arc::clone(&stats),
        );
        let mut bytes = Vec::new();
        chain.process(&sine(1_000.0, 48_000, 48_000, 0.25), &mut bytes);
        let out = decode_s32(&bytes);
        let gain =
            rms(&left(&out)[24_000..]) / rms(&left(&sine(1_000.0, 48_000, 48_000, 0.25))[24_000..]);
        assert!(
            (20.0 * gain.log10() - 6.0).abs() < 0.05,
            "{}",
            20.0 * gain.log10()
        );
        // 24-bit words in a left-justified 32-bit container.
        assert!(bytes.as_chunks::<4>().0.iter().all(|sample| sample[0] == 0));
        let status = stats.snapshot();
        assert!(!status.clipping);
        assert!(status.realtime_factor.unwrap() > 1.0);
        assert!((status.peak_dbfs.unwrap() - 20.0 * (0.5f64).log10()).abs() < 0.5);
    }

    #[test]
    fn clipping_is_detected_and_limited() {
        let band = EqBand {
            id: "b".into(),
            enabled: true,
            filter_type: EqFilterType::Peaking,
            frequency: 1_000.0,
            gain_db: 12.0,
            q: 1.0,
        };
        let stats = Arc::new(DspStats::default());
        let mut config = plan(vec![band], 44_100, 44_100);
        config.output_bits = 16;
        let mut chain = DspChain::new(&config, 44_100, InputFormat::S16, Arc::clone(&stats));
        let mut bytes = Vec::new();
        chain.process(&sine(1_000.0, 44_100, 4_410, 0.9), &mut bytes);
        assert_eq!(bytes.len(), 4_410 * 2 * 2);
        let status = stats.snapshot();
        assert!(status.clipping && status.clipped_samples > 100);
        assert!(status.peak_dbfs.unwrap() > 6.0);
        stats.reset();
        assert!(!stats.snapshot().clipping);
    }

    #[test]
    fn headroom_lowers_the_level_before_the_eq() {
        let stats = Arc::new(DspStats::default());
        let mut config = plan(Vec::new(), 44_100, 44_100);
        config.headroom_db = Some(-6.0);
        config.leveling_db = Some(-2.0);
        let mut chain = DspChain::new(&config, 44_100, InputFormat::S32, stats);
        let mut bytes = Vec::new();
        let input = sine(440.0, 44_100, 8_820, 0.5);
        chain.process(&input, &mut bytes);
        let ratio = rms(&decode_s32(&bytes)) / rms(&input);
        assert!((20.0 * ratio.log10() + 8.0).abs() < 0.01);
    }

    #[test]
    fn convolution_runs_in_the_chain_without_adding_or_losing_frames() {
        // A filter that halves the level after a 300-sample delay, at
        // 48 kHz, run on 44.1 kHz audio: the chain converts it.
        let mut taps = vec![0.0; 1_200];
        taps[300] = 0.5;
        let impulse = ImpulseResponse {
            sample_rate: 48_000,
            channels: vec![taps],
        };
        let stats = Arc::new(DspStats::default());
        let config = plan(Vec::new(), 44_100, 44_100);
        let mut chain = DspChain::new(&config, 44_100, InputFormat::S32, Arc::clone(&stats))
            .with_convolution(&impulse)
            .unwrap();
        let input = sine(1_000.0, 44_100, 10_000, 0.8);
        let mut bytes = Vec::new();
        for chunk in input.chunks(2 * 999) {
            chain.process(chunk, &mut bytes);
        }
        chain.finish(&mut bytes);
        let out = decode_s32(&bytes);
        assert_eq!(
            out.len(),
            input.len(),
            "the convolver keeps the frame count"
        );
        let gain_db = 20.0 * (rms(&out[8_000..]) / rms(&input[8_000..])).log10();
        assert!((gain_db + 6.02).abs() < 0.1, "{gain_db}");
    }

    #[test]
    fn convolution_is_planned_from_the_filter_or_explained() {
        let caps = DeviceCaps {
            sample_rates: vec![44_100, 48_000],
            sample_formats: vec![SampleFormat::S16, SampleFormat::S24Packed],
            min_channels: 2,
            max_channels: 2,
            dsd_formats: Vec::new(),
            dsd_rates: Vec::new(),
        };
        let settings = OutputSettings::default();
        let source = SourceFormat {
            sample_rate: 44_100,
            bit_depth: Some(16),
            channels: 2,
            lossless: true,
        };
        let dsp = DspSettings {
            convolution: ConvolutionSettings {
                enabled: true,
                filter_id: Some("0f-room".into()),
            },
            ..DspSettings::default()
        };
        assert!(dsp.processes());
        let mut output = crate::plan_output(source, Some(&caps), &settings).unwrap();
        let skipped = plan_processing_with(
            &mut output.clone(),
            &source,
            &dsp,
            None,
            &caps,
            &settings,
            None,
        );
        assert!(skipped.is_none(), "a missing filter alone does not process");
        let impulse = ImpulseResponse {
            sample_rate: 48_000,
            channels: vec![vec![1.0, 0.0, 0.0]],
        };
        let filter = ConvolutionFilter {
            id: "0f-room".into(),
            name: "Living room".into(),
            summary: impulse.summary("64-bit float".into()),
            impulse: Arc::new(impulse),
        };
        let processing = plan_processing_with(
            &mut output,
            &source,
            &dsp,
            None,
            &caps,
            &settings,
            Some(&filter),
        )
        .unwrap();
        let convolution = processing.convolution.unwrap();
        assert_eq!(convolution.name, "Living room");
        assert_eq!(convolution.filter_rate, 48_000);
        assert_eq!(convolution.run_rate, 44_100);
        assert!(!output.bit_perfect);
        let mut bad = dsp.clone();
        bad.convolution.filter_id = Some("../etc".into());
        assert!(bad.normalized().convolution.filter_id.is_none());
    }

    #[test]
    fn crossfeed_feeds_a_low_passed_share_to_the_other_ear() {
        let stats = Arc::new(DspStats::default());
        let mut config = plan(Vec::new(), 44_100, 44_100);
        config.crossfeed = Some(CrossfeedStrength::Medium);
        let mut chain = DspChain::new(&config, 44_100, InputFormat::S32, stats);
        // A 200 Hz tone in the left channel only.
        let input: Vec<f64> = sine(200.0, 44_100, 22_050, 0.5)
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|frame| [frame[0], 0.0])
            .collect();
        let mut bytes = Vec::new();
        chain.process(&input, &mut bytes);
        let out = decode_s32(&bytes);
        let right: Vec<f64> = out.iter().skip(1).step_by(2).copied().collect();
        let left_out: Vec<f64> = out.iter().step_by(2).copied().collect();
        let feed = 10f64.powf(-6.0 / 20.0);
        let direct_db = 20.0 * (rms(&left_out[4_410..]) / rms(&left(&input)[4_410..])).log10();
        assert!((direct_db - 20.0 * (1.0 / (1.0 + feed)).log10()).abs() < 0.05);
        // Below the 700 Hz corner the fed share is about feed / (1 + feed).
        let cross_db = 20.0 * (rms(&right[4_410..]) / rms(&left(&input)[4_410..])).log10();
        assert!(
            (cross_db - 20.0 * (feed / (1.0 + feed)).log10()).abs() < 0.6,
            "{cross_db}"
        );
    }

    #[test]
    fn the_resampler_keeps_timing_level_and_rejects_images() {
        for (from, to) in [
            (44_100, 48_000),
            (48_000, 44_100),
            (44_100, 88_200),
            (96_000, 48_000),
        ] {
            let frames = (from / 5) as usize;
            let input = sine(1_000.0, from, frames, 0.5);
            let mut resampler = Resampler::new(from, to, 2);
            let mut output = Vec::new();
            for chunk in input.chunks(2 * 1_000) {
                resampler.process(chunk, &mut output);
            }
            resampler.finish(&mut output);
            let expected = (frames as u64 * u64::from(to)).div_ceil(u64::from(from)) as usize;
            assert_eq!(output.len() / 2, expected, "{from} to {to}");
            // Compare with the ideal sine at the new rate, away from the edges.
            let reference = sine(1_000.0, to, expected, 0.5);
            let margin = expected / 4;
            let error: Vec<f64> = output[2 * margin..2 * (expected - margin)]
                .iter()
                .zip(&reference[2 * margin..2 * (expected - margin)])
                .map(|(a, b)| a - b)
                .collect();
            let snr = 20.0 * (rms(&reference) / rms(&error)).log10();
            assert!(snr > 140.0, "{from} to {to}: {snr:.1} dB");
        }
    }

    #[test]
    fn processing_planning_keeps_bit_perfect_paths_and_picks_rates() {
        let caps = DeviceCaps {
            sample_rates: vec![44_100, 48_000, 88_200, 96_000],
            sample_formats: vec![
                SampleFormat::S16,
                SampleFormat::S24Packed,
                SampleFormat::S32,
            ],
            min_channels: 2,
            max_channels: 2,
            ..DeviceCaps::default()
        };
        let source = SourceFormat {
            sample_rate: 44_100,
            bit_depth: Some(16),
            channels: 2,
            lossless: true,
        };
        let settings = OutputSettings::default();
        let base = crate::plan_output(source, Some(&caps), &settings).unwrap();
        let mut untouched = base.clone();
        assert!(
            plan_processing(
                &mut untouched,
                &source,
                &DspSettings::default(),
                None,
                &caps,
                &settings
            )
            .is_none()
        );
        assert_eq!(untouched, base);

        let mut dsp = DspSettings {
            sample_rate: SampleRateSettings {
                enabled: true,
                mode: RateMode::Fixed(96_000),
            },
            ..DspSettings::default()
        };
        let mut plan = base.clone();
        let processing = plan_processing(&mut plan, &source, &dsp, None, &caps, &settings).unwrap();
        assert!(processing.rate_requested);
        assert_eq!(plan.resample, Some((44_100, 96_000)));
        assert_eq!(plan.bit_depth, 32);
        assert!(!plan.bit_perfect);
        assert_eq!(processing.resampler, Some(ResamplerKind::Parson));

        dsp.sample_rate.mode = RateMode::Fixed(192_000);
        let mut plan = base.clone();
        assert!(plan_processing(&mut plan, &source, &dsp, None, &caps, &settings).is_none());
        assert_eq!(
            selectable_rates(&caps, &settings),
            vec![44_100, 48_000, 88_200, 96_000]
        );

        let mut plan = base.clone();
        let capped = OutputSettings {
            max_bit_depth: Some(24),
            ..OutputSettings::default()
        };
        let processing =
            plan_processing(&mut plan, &source, &dsp, Some(-3.0), &caps, &capped).unwrap();
        assert_eq!(processing.requested_rate_unavailable, Some(192_000));
        assert_eq!(plan.sample_rate, 44_100);
        assert_eq!(plan.bit_depth, 24);
        assert_eq!(plan.sample_format, SampleFormat::S24Packed);
    }

    #[test]
    fn dsp_pcm_streams_whole_blocks_from_a_float_decoder() {
        let input = sine(500.0, 44_100, 10_000, 0.3);
        let bytes: Vec<u8> = input.iter().flat_map(|value| value.to_le_bytes()).collect();
        // Odd read sizes split frames across reads.
        struct Trickle(io::Cursor<Vec<u8>>);
        impl Read for Trickle {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                let size = buffer.len().min(1_003);
                self.0.read(&mut buffer[..size])
            }
        }
        let stats = Arc::new(DspStats::default());
        let mut config = plan(Vec::new(), 44_100, 48_000);
        config.headroom_db = Some(-1.0);
        let chain = DspChain::new(&config, 44_100, InputFormat::S32, Arc::clone(&stats));
        let mut pcm = DspPcm::new(Box::new(Trickle(io::Cursor::new(bytes))), chain);
        let mut out = Vec::new();
        pcm.read_to_end(&mut out).unwrap();
        let expected = (10_000u64 * 48_000).div_ceil(44_100) as usize;
        assert_eq!(out.len(), expected * 2 * 4);
        assert!(stats.snapshot().realtime_factor.is_some());
    }
}
