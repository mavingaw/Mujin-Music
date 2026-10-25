//! Native audio output for Parson.
//!
//! Browsers and phones hand audio to the operating system's mixer in shared
//! mode, which may resample it and mix other sounds in. This crate opens an
//! output device directly instead, the way Roon's own audio transport does:
//!
//! - Linux: an ALSA `hw:` device, which has no mixer or converter in front
//!   of it (libasound is loaded at run time, so no headers are needed to
//!   build and the server still starts on machines without ALSA).
//! - Windows: WASAPI in exclusive mode.
//! - macOS: CoreAudio in hog mode, with the device's physical format set to
//!   the source's sample rate and bit depth.
//!
//! Each device is probed for the PCM formats it accepts bit-perfectly, and
//! [`plan_output`] decides, per track, whether the samples can go out
//! unmodified or which conversion is unavoidable. [`player::Player`] then
//! plays decoded PCM to the device on its own thread.

pub mod dsp;
pub mod player;
pub mod simulated;

#[cfg(target_os = "linux")]
mod alsa;
#[cfg(target_os = "macos")]
mod coreaudio;
#[cfg(windows)]
mod wasapi;

use std::io;

use serde::{Deserialize, Serialize};

/// PCM sample rates probed on every device, as Roon's "Bit-Perfect Format
/// Support" list shows them.
pub const STANDARD_RATES: [u32; 10] = [
    44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000, 705_600, 768_000,
];

/// Integer PCM bit depths probed on every device.
pub const STANDARD_BIT_DEPTHS: [u8; 3] = [16, 24, 32];

/// How the device is driven.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Driver {
    /// ALSA, `hw:` (exclusive) or `plughw:` (conversions allowed).
    Alsa,
    /// Windows Audio Session API.
    Wasapi,
    /// macOS CoreAudio HAL.
    CoreAudio,
    /// A software device that consumes audio in real time without a sound
    /// card, for testing and demonstrations (`PARSON_NATIVE_OUTPUT_SIMULATE`).
    Simulated,
}

impl Driver {
    /// e.g. `WASAPI Exclusive Mode`, `ALSA hw (exclusive)`.
    pub fn mode_label(self, exclusive: bool) -> &'static str {
        match (self, exclusive) {
            (Self::Alsa, true) => "ALSA hw, exclusive",
            (Self::Alsa, false) => "ALSA plughw, conversions allowed",
            (Self::Wasapi, true) => "WASAPI Exclusive Mode",
            (Self::Wasapi, false) => "WASAPI Shared Mode",
            (Self::CoreAudio, true) => "CoreAudio Exclusive Mode (hog mode)",
            (Self::CoreAudio, false) => "CoreAudio, shared",
            (Self::Simulated, true) => "Simulated exclusive output",
            (Self::Simulated, false) => "Simulated shared output",
        }
    }
}

/// A PCM sample container a device accepts.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SampleFormat {
    /// 16-bit little endian.
    S16,
    /// 24-bit little endian packed in 3 bytes.
    S24Packed,
    /// 24-bit little endian in the low bits of 4 bytes.
    S24In32,
    /// 32-bit little endian.
    S32,
}

impl SampleFormat {
    pub fn bytes(self) -> usize {
        match self {
            Self::S16 => 2,
            Self::S24Packed => 3,
            Self::S24In32 | Self::S32 => 4,
        }
    }

    /// Bits of precision the container carries.
    pub fn bits(self) -> u8 {
        match self {
            Self::S16 => 16,
            Self::S24Packed | Self::S24In32 => 24,
            Self::S32 => 32,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::S16 => "16-bit",
            Self::S24Packed => "24-bit packed",
            Self::S24In32 => "24-bit in 32",
            Self::S32 => "32-bit",
        }
    }
}

/// What a device accepts without conversion.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceCaps {
    /// Supported entries of [`STANDARD_RATES`], ascending.
    pub sample_rates: Vec<u32>,
    /// Supported sample containers, best first within each depth.
    pub sample_formats: Vec<SampleFormat>,
    pub min_channels: u8,
    pub max_channels: u8,
}

impl DeviceCaps {
    /// Supported entries of [`STANDARD_BIT_DEPTHS`].
    pub fn bit_depths(&self) -> Vec<u8> {
        let mut depths: Vec<u8> = self.sample_formats.iter().map(|f| f.bits()).collect();
        depths.sort_unstable();
        depths.dedup();
        depths
    }

    /// The container used for `bits`, preferring an exact fit.
    pub fn format_for_bits(&self, bits: u8) -> Option<SampleFormat> {
        let order: &[SampleFormat] = match bits {
            16 => &[SampleFormat::S16],
            24 => &[SampleFormat::S24Packed, SampleFormat::S24In32],
            32 => &[SampleFormat::S32],
            _ => &[],
        };
        order
            .iter()
            .copied()
            .find(|format| self.sample_formats.contains(format))
    }
}

/// One output device and what it supports.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Stable within one machine, e.g. `alsa:USB:0` or a WASAPI endpoint id.
    pub id: String,
    /// e.g. `Pre Box RS2 Digital`.
    pub name: String,
    /// Where it is, e.g. `hw:1,0`, `USB`, or the endpoint description.
    pub detail: String,
    pub driver: Driver,
    /// Whether exclusive access is possible on this platform and device.
    pub exclusive_capable: bool,
    /// `None` when the device could not be probed (see `probe_error`).
    pub caps: Option<DeviceCaps>,
    /// Why probing failed, e.g. the device is in use by another app.
    pub probe_error: Option<String>,
    pub is_default: bool,
}

/// Per-device user settings, mirroring Roon's device setup.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct OutputSettings {
    /// Open the device exclusively (bit-perfect); off lets the system
    /// convert or mix.
    #[serde(default = "default_true")]
    pub exclusive: bool,
    /// Highest PCM rate to send, for devices that misreport support.
    #[serde(default)]
    pub max_sample_rate: Option<u32>,
    /// Highest bit depth to send.
    #[serde(default)]
    pub max_bit_depth: Option<u8>,
}

fn default_true() -> bool {
    true
}

impl Default for OutputSettings {
    fn default() -> Self {
        Self {
            exclusive: true,
            max_sample_rate: None,
            max_bit_depth: None,
        }
    }
}

impl OutputSettings {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(rate) = self.max_sample_rate
            && !STANDARD_RATES.contains(&rate)
        {
            return Err(format!("{rate} Hz is not a standard PCM rate"));
        }
        if let Some(bits) = self.max_bit_depth
            && !STANDARD_BIT_DEPTHS.contains(&bits)
        {
            return Err(format!("{bits} bit is not a supported bit depth"));
        }
        Ok(())
    }
}

/// One row of the "Bit-Perfect Format Support" list.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct FormatSupportEntry<T> {
    pub value: T,
    /// The device accepts it.
    pub supported: bool,
    /// The user's max setting lets it through.
    pub allowed: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct FormatSupport {
    pub sample_rates: Vec<FormatSupportEntry<u32>>,
    pub bit_depths: Vec<FormatSupportEntry<u8>>,
}

/// Every standard rate and depth, marked supported or not, for the UI.
pub fn format_support(caps: &DeviceCaps, settings: &OutputSettings) -> FormatSupport {
    let depths = caps.bit_depths();
    FormatSupport {
        sample_rates: STANDARD_RATES
            .iter()
            .map(|&rate| FormatSupportEntry {
                value: rate,
                supported: caps.sample_rates.contains(&rate),
                allowed: settings.max_sample_rate.is_none_or(|max| rate <= max),
            })
            .collect(),
        bit_depths: STANDARD_BIT_DEPTHS
            .iter()
            .map(|&bits| FormatSupportEntry {
                value: bits,
                supported: depths.contains(&bits),
                allowed: settings.max_bit_depth.is_none_or(|max| bits <= max),
            })
            .collect(),
    }
}

/// The decoded source, as far as the output cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceFormat {
    pub sample_rate: u32,
    /// Integer bit depth of a lossless source; `None` for lossy or float
    /// sources, which have no fixed depth.
    pub bit_depth: Option<u8>,
    pub channels: u8,
    pub lossless: bool,
}

/// What happens to the bit depth on the way to the device.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DepthChange {
    /// Same depth: the samples are copied.
    None,
    /// Zero-padded into a wider word; every sample is preserved exactly.
    Pad { from: u8, to: u8 },
    /// Reduced with triangular dither because the device cannot take the
    /// source's depth.
    Reduce { from: u8, to: u8 },
    /// A lossy or float source quantised to the device's integer format.
    Quantize { to: u8 },
}

/// How one track will be sent to one device.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct OutputPlan {
    pub sample_rate: u32,
    pub sample_format: SampleFormat,
    pub bit_depth: u8,
    pub channels: u8,
    /// `(from, to)` when the device cannot take the source's rate.
    pub resample: Option<(u32, u32)>,
    pub depth: DepthChange,
    /// `(from, to)` when the channel count changes.
    pub channel_change: Option<(u8, u8)>,
    pub exclusive: bool,
    /// The samples reach the device unmodified: exclusive access, no
    /// resampling, no depth reduction and no downmix.
    pub bit_perfect: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// The device was not probed, so nothing is known to work.
    NotProbed,
    /// No rate or format survives the device's support and the settings.
    NoUsableFormat,
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotProbed => f.write_str("the device's formats are unknown"),
            Self::NoUsableFormat => f.write_str(
                "no sample rate or bit depth the device supports is allowed by its settings",
            ),
        }
    }
}

fn rate_family(rate: u32) -> u32 {
    if rate.is_multiple_of(11_025) {
        44_100
    } else {
        48_000
    }
}

/// The rate to send `source` at: the source rate when allowed; otherwise
/// a rate in the same family (44.1k or 48k multiples), so the conversion
/// ratio is an integer where possible, preferring the lowest one at or
/// above the source and then the highest one below it.
pub fn choose_rate(source: u32, allowed: &[u32]) -> Option<u32> {
    if allowed.contains(&source) {
        return Some(source);
    }
    let family = rate_family(source);
    let same: Vec<u32> = allowed
        .iter()
        .copied()
        .filter(|rate| rate_family(*rate) == family)
        .collect();
    let pick = |rates: &[u32]| {
        rates
            .iter()
            .copied()
            .filter(|rate| *rate >= source)
            .min()
            .or_else(|| rates.iter().copied().filter(|rate| *rate < source).max())
    };
    pick(&same).or_else(|| pick(allowed))
}

/// Decides how `source` is sent to a device with `caps` under `settings`.
pub fn plan_output(
    source: SourceFormat,
    caps: Option<&DeviceCaps>,
    settings: &OutputSettings,
) -> Result<OutputPlan, PlanError> {
    let caps = caps.ok_or(PlanError::NotProbed)?;
    let rates: Vec<u32> = caps
        .sample_rates
        .iter()
        .copied()
        .filter(|rate| settings.max_sample_rate.is_none_or(|max| *rate <= max))
        .collect();
    let depths: Vec<u8> = caps
        .bit_depths()
        .into_iter()
        .filter(|bits| settings.max_bit_depth.is_none_or(|max| *bits <= max))
        .collect();
    let rate = choose_rate(source.sample_rate, &rates).ok_or(PlanError::NoUsableFormat)?;
    let wanted_bits = source.bit_depth.unwrap_or(24);
    let bits = if depths.contains(&wanted_bits) {
        wanted_bits
    } else {
        depths
            .iter()
            .copied()
            .filter(|bits| *bits >= wanted_bits)
            .min()
            .or_else(|| depths.iter().copied().max())
            .ok_or(PlanError::NoUsableFormat)?
    };
    let sample_format = caps
        .format_for_bits(bits)
        .ok_or(PlanError::NoUsableFormat)?;
    let min_channels = caps.min_channels.max(1);
    let max_channels = caps.max_channels.max(min_channels);
    let channels = source.channels.clamp(min_channels, max_channels);
    let depth = match source.bit_depth {
        None => DepthChange::Quantize { to: bits },
        Some(from) if from == bits => DepthChange::None,
        Some(from) if from < bits => DepthChange::Pad { from, to: bits },
        Some(from) => DepthChange::Reduce { from, to: bits },
    };
    let resample = (rate != source.sample_rate).then_some((source.sample_rate, rate));
    let channel_change = (channels != source.channels).then_some((source.channels, channels));
    let bit_perfect = settings.exclusive
        && source.lossless
        && resample.is_none()
        && matches!(depth, DepthChange::None | DepthChange::Pad { .. })
        // Duplicating mono into stereo is exact; folding channels is not.
        && channels >= source.channels;
    Ok(OutputPlan {
        sample_rate: rate,
        sample_format,
        bit_depth: bits,
        channels,
        resample,
        depth,
        channel_change,
        exclusive: settings.exclusive,
        bit_perfect,
    })
}

/// The sample layout a stream is opened with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamFormat {
    pub sample_rate: u32,
    pub sample_format: SampleFormat,
    pub channels: u8,
}

impl StreamFormat {
    pub fn frame_bytes(&self) -> usize {
        self.sample_format.bytes() * usize::from(self.channels)
    }
}

/// An open device stream. Writes block until the device has room, so the
/// player runs in real time.
pub trait Sink: Send {
    /// Writes whole interleaved frames in the stream's container format.
    fn write(&mut self, data: &[u8]) -> io::Result<()>;
    /// Frames written but not yet heard.
    fn delay_frames(&mut self) -> u64;
    /// Stops output now, discarding what is buffered.
    fn flush(&mut self) -> io::Result<()>;
    /// Plays out what is buffered, then returns.
    fn drain(&mut self) -> io::Result<()>;
}

/// Lists devices and opens them.
pub trait Backend: Send + Sync {
    fn driver(&self) -> Driver;
    /// Enumerates and probes output devices. This opens each device
    /// briefly, so it is not free; callers cache the result.
    fn devices(&self) -> Vec<DeviceInfo>;
    /// Opens `device_id` for `format`. Called on the playback thread.
    fn open(
        &self,
        device_id: &str,
        format: StreamFormat,
        exclusive: bool,
    ) -> io::Result<Box<dyn Sink>>;
}

/// This platform's native backend, or `None` where there is none (or the
/// system audio library cannot be loaded).
pub fn platform_backend() -> Option<Box<dyn Backend>> {
    #[cfg(target_os = "linux")]
    {
        alsa::AlsaBackend::load().map(|backend| Box::new(backend) as Box<dyn Backend>)
    }
    #[cfg(windows)]
    {
        Some(Box::new(wasapi::WasapiBackend))
    }
    #[cfg(target_os = "macos")]
    {
        Some(Box::new(coreaudio::CoreAudioBackend))
    }
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    {
        None
    }
}

/// Converts interleaved left-justified signed 32-bit samples (what the
/// decoder produces) to `format`. 16- and 24-bit content keeps every bit.
pub fn pack_samples(input: &[u8], format: SampleFormat, output: &mut Vec<u8>) {
    output.clear();
    match format {
        SampleFormat::S32 => output.extend_from_slice(&input[..input.len() / 4 * 4]),
        SampleFormat::S24Packed => {
            output.reserve(input.len() / 4 * 3);
            for sample in input.as_chunks::<4>().0 {
                output.extend_from_slice(&sample[1..4]);
            }
        }
        SampleFormat::S24In32 => {
            output.reserve(input.len());
            for sample in input.as_chunks::<4>().0 {
                let value = i32::from_le_bytes(*sample) >> 8;
                output.extend_from_slice(&value.to_le_bytes());
            }
        }
        SampleFormat::S16 => {
            output.reserve(input.len() / 2);
            for sample in input.as_chunks::<4>().0 {
                output.extend_from_slice(&sample[2..4]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dac(rates: &[u32], formats: &[SampleFormat]) -> DeviceCaps {
        DeviceCaps {
            sample_rates: rates.to_vec(),
            sample_formats: formats.to_vec(),
            min_channels: 2,
            max_channels: 2,
        }
    }

    fn flac(rate: u32, bits: u8) -> SourceFormat {
        SourceFormat {
            sample_rate: rate,
            bit_depth: Some(bits),
            channels: 2,
            lossless: true,
        }
    }

    const FORMATS: [SampleFormat; 3] = [
        SampleFormat::S16,
        SampleFormat::S24Packed,
        SampleFormat::S32,
    ];

    #[test]
    fn a_supported_source_goes_out_bit_perfect() {
        let caps = dac(&[44_100, 48_000, 88_200, 96_000], &FORMATS);
        let plan = plan_output(flac(96_000, 24), Some(&caps), &OutputSettings::default()).unwrap();
        assert_eq!(plan.sample_rate, 96_000);
        assert_eq!(plan.bit_depth, 24);
        assert_eq!(plan.sample_format, SampleFormat::S24Packed);
        assert!(plan.resample.is_none());
        assert_eq!(plan.depth, DepthChange::None);
        assert!(plan.bit_perfect);
    }

    #[test]
    fn unsupported_rates_convert_within_their_family() {
        let caps = dac(&[44_100, 48_000, 88_200, 96_000], &FORMATS);
        let plan = plan_output(flac(192_000, 24), Some(&caps), &OutputSettings::default()).unwrap();
        assert_eq!(plan.resample, Some((192_000, 96_000)));
        assert!(!plan.bit_perfect);
        let plan = plan_output(flac(352_800, 24), Some(&caps), &OutputSettings::default()).unwrap();
        assert_eq!(plan.resample, Some((352_800, 88_200)));
        // A 48k-only device takes 44.1k material at 48k.
        let only_48 = dac(&[48_000], &FORMATS);
        let plan =
            plan_output(flac(44_100, 16), Some(&only_48), &OutputSettings::default()).unwrap();
        assert_eq!(plan.resample, Some((44_100, 48_000)));
    }

    #[test]
    fn choose_rate_prefers_the_lowest_rate_at_or_above_the_source() {
        let allowed = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];
        assert_eq!(choose_rate(22_050, &allowed), Some(44_100));
        assert_eq!(choose_rate(32_000, &allowed), Some(48_000));
        assert_eq!(choose_rate(384_000, &allowed), Some(192_000));
        assert_eq!(choose_rate(44_100, &[]), None);
    }

    #[test]
    fn settings_cap_rate_and_depth() {
        let caps = dac(&STANDARD_RATES, &FORMATS);
        let settings = OutputSettings {
            exclusive: true,
            max_sample_rate: Some(96_000),
            max_bit_depth: Some(24),
        };
        let plan = plan_output(flac(192_000, 24), Some(&caps), &settings).unwrap();
        assert_eq!(plan.resample, Some((192_000, 96_000)));
        let plan = plan_output(flac(44_100, 32), Some(&caps), &settings).unwrap();
        assert_eq!(plan.depth, DepthChange::Reduce { from: 32, to: 24 });
        assert!(!plan.bit_perfect);
        let none = OutputSettings {
            max_sample_rate: Some(44_100),
            ..OutputSettings::default()
        };
        assert_eq!(
            plan_output(flac(44_100, 16), Some(&dac(&[48_000], &FORMATS)), &none),
            Err(PlanError::NoUsableFormat)
        );
    }

    #[test]
    fn padding_is_bit_perfect_but_reduction_and_shared_mode_are_not() {
        let caps = dac(&[44_100], &[SampleFormat::S24In32, SampleFormat::S32]);
        let plan = plan_output(flac(44_100, 16), Some(&caps), &OutputSettings::default()).unwrap();
        assert_eq!(plan.depth, DepthChange::Pad { from: 16, to: 24 });
        assert_eq!(plan.sample_format, SampleFormat::S24In32);
        assert!(plan.bit_perfect);
        let shared = OutputSettings {
            exclusive: false,
            ..OutputSettings::default()
        };
        assert!(
            !plan_output(flac(44_100, 16), Some(&caps), &shared)
                .unwrap()
                .bit_perfect
        );
        let lossy = SourceFormat {
            sample_rate: 44_100,
            bit_depth: None,
            channels: 2,
            lossless: false,
        };
        let plan = plan_output(lossy, Some(&caps), &OutputSettings::default()).unwrap();
        assert_eq!(plan.depth, DepthChange::Quantize { to: 24 });
        assert!(!plan.bit_perfect);
        assert_eq!(
            plan_output(lossy, None, &OutputSettings::default()),
            Err(PlanError::NotProbed)
        );
    }

    #[test]
    fn channels_follow_the_device() {
        let caps = dac(&[44_100], &FORMATS);
        let mono = SourceFormat {
            channels: 1,
            ..flac(44_100, 16)
        };
        let plan = plan_output(mono, Some(&caps), &OutputSettings::default()).unwrap();
        assert_eq!(plan.channel_change, Some((1, 2)));
        assert!(plan.bit_perfect);
        let surround = SourceFormat {
            channels: 6,
            ..flac(44_100, 16)
        };
        let plan = plan_output(surround, Some(&caps), &OutputSettings::default()).unwrap();
        assert_eq!(plan.channel_change, Some((6, 2)));
        assert!(!plan.bit_perfect);
    }

    #[test]
    fn format_support_marks_unsupported_and_capped_entries() {
        let caps = dac(
            &[44_100, 48_000, 96_000],
            &[SampleFormat::S16, SampleFormat::S24Packed],
        );
        let settings = OutputSettings {
            max_sample_rate: Some(48_000),
            ..OutputSettings::default()
        };
        let support = format_support(&caps, &settings);
        let rate = |value| {
            support
                .sample_rates
                .iter()
                .find(|e| e.value == value)
                .unwrap()
        };
        assert!(rate(44_100).supported && rate(44_100).allowed);
        assert!(!rate(88_200).supported);
        assert!(rate(96_000).supported && !rate(96_000).allowed);
        assert_eq!(support.sample_rates.len(), STANDARD_RATES.len());
        let depth = |value| {
            support
                .bit_depths
                .iter()
                .find(|e| e.value == value)
                .unwrap()
        };
        assert!(depth(24).supported && !depth(32).supported);
    }

    #[test]
    fn settings_validation_rejects_odd_values() {
        assert!(OutputSettings::default().validate().is_ok());
        let bad_rate = OutputSettings {
            max_sample_rate: Some(50_000),
            ..OutputSettings::default()
        };
        assert!(bad_rate.validate().is_err());
        let bad_bits = OutputSettings {
            max_bit_depth: Some(20),
            ..OutputSettings::default()
        };
        assert!(bad_bits.validate().is_err());
    }

    #[test]
    fn packing_keeps_every_bit_of_16_and_24_bit_content() {
        // One 24-bit sample 0x123456 left-justified in 32 bits, and -1.
        let sample = (0x0012_3456_i32 << 8).to_le_bytes();
        let negative = (-1_i32 << 8).to_le_bytes();
        let input = [sample, negative].concat();
        let mut out = Vec::new();
        pack_samples(&input, SampleFormat::S24Packed, &mut out);
        assert_eq!(out, [0x56, 0x34, 0x12, 0xff, 0xff, 0xff]);
        pack_samples(&input, SampleFormat::S24In32, &mut out);
        assert_eq!(&out[..4], &0x0012_3456_i32.to_le_bytes());
        assert_eq!(&out[4..], &(-1_i32).to_le_bytes());
        pack_samples(&input, SampleFormat::S32, &mut out);
        assert_eq!(out, input);
        let sixteen = ((0x1234_i32) << 16).to_le_bytes();
        pack_samples(&sixteen, SampleFormat::S16, &mut out);
        assert_eq!(out, [0x34, 0x12]);
    }

    #[test]
    fn mode_labels_name_the_driver_and_mode() {
        assert_eq!(Driver::Wasapi.mode_label(true), "WASAPI Exclusive Mode");
        assert!(Driver::CoreAudio.mode_label(true).contains("hog"));
        assert!(Driver::Alsa.mode_label(true).starts_with("ALSA hw"));
    }
}
