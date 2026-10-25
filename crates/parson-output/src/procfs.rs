//! What the Linux kernel itself reports about sound hardware.
//!
//! libasound tells Parson what a device *accepts*. The kernel's
//! `/proc/asound` tree adds two things it cannot fake:
//!
//! - `cardN/streamM` for USB DACs: the formats, rates and bit depths the
//!   DAC's own USB descriptors declare, its product name and USB speed.
//!   It is readable even while another program holds the device, when
//!   libasound cannot probe it.
//! - `cardN/pcmMp/sub0/hw_params`: the format the hardware is running at
//!   this moment. Read during playback, it proves which rate, container
//!   and channel count reached the driver, independently of Parson's own
//!   plan.
//!
//! The parsers take text so they are tested on any platform; the readers
//! take the root directory (`/proc/asound`) so tests can point them at a
//! fixture tree.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{DSD_FORMATS, DSD_RATES, DeviceCaps, STANDARD_RATES, SampleFormat, StreamFormat};

pub const PROC_ASOUND: &str = "/proc/asound";

/// The ALSA name of a container, as the kernel prints it.
pub fn alsa_format_name(format: SampleFormat) -> &'static str {
    match format {
        SampleFormat::S16 => "S16_LE",
        SampleFormat::S24Packed => "S24_3LE",
        SampleFormat::S24In32 => "S24_LE",
        SampleFormat::S32 => "S32_LE",
        SampleFormat::DsdU8 => "DSD_U8",
        SampleFormat::DsdU16Le => "DSD_U16_LE",
        SampleFormat::DsdU16Be => "DSD_U16_BE",
        SampleFormat::DsdU32Le => "DSD_U32_LE",
        SampleFormat::DsdU32Be => "DSD_U32_BE",
    }
}

pub fn format_from_alsa_name(name: &str) -> Option<SampleFormat> {
    Some(match name {
        "S16_LE" => SampleFormat::S16,
        "S24_3LE" => SampleFormat::S24Packed,
        "S24_LE" => SampleFormat::S24In32,
        "S32_LE" => SampleFormat::S32,
        "DSD_U8" => SampleFormat::DsdU8,
        "DSD_U16_LE" => SampleFormat::DsdU16Le,
        "DSD_U16_BE" => SampleFormat::DsdU16Be,
        "DSD_U32_LE" => SampleFormat::DsdU32Le,
        "DSD_U32_BE" => SampleFormat::DsdU32Be,
        _ => return None,
    })
}

/// One playback alternate setting of a USB audio interface.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct UsbAltset {
    pub interface: Option<u32>,
    pub altset: Option<u32>,
    /// Kernel format names, e.g. `S24_3LE`, `DSD_U32_BE`.
    pub formats: Vec<String>,
    pub channels: Option<u8>,
    /// Discrete rates; a continuous range is expanded to the standard
    /// rates inside it.
    pub rates: Vec<u32>,
    /// Valid bits per sample, e.g. 24 in an `S32_LE` container.
    pub bits: Option<u8>,
}

/// A USB DAC's `streamN` file.
#[derive(Serialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct UsbStream {
    /// e.g. `AudioQuest DragonFly Red v1.0`.
    pub product: Option<String>,
    /// e.g. `usb-0000:00:14.0-2`.
    pub bus_path: Option<String>,
    /// e.g. `high speed`.
    pub speed: Option<String>,
    pub playback: Vec<UsbAltset>,
    /// The playback endpoint is streaming.
    pub running: bool,
    /// `Momentary freq` while streaming: the rate the DAC is clocking.
    pub momentary_hz: Option<u32>,
}

/// Removes a vendor name the descriptor repeats ("AudioQuest AudioQuest
/// DragonFly Red" becomes "AudioQuest DragonFly Red").
fn collapse_repeat(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    for size in (1..=words.len() / 2).rev() {
        if words[..size] == words[size..size * 2] {
            return words[size..].join(" ");
        }
    }
    words.join(" ")
}

fn parse_rates(text: &str) -> Vec<u32> {
    let text = text.trim();
    if let Some((low, rest)) = text.split_once(" - ") {
        let high = rest.split_whitespace().next().unwrap_or_default();
        let (Ok(low), Ok(high)) = (low.trim().parse::<u32>(), high.parse::<u32>()) else {
            return Vec::new();
        };
        return STANDARD_RATES
            .iter()
            .copied()
            .filter(|rate| (low..=high).contains(rate))
            .collect();
    }
    text.split(',')
        .filter_map(|rate| rate.trim().parse().ok())
        .collect()
}

/// Parses `/proc/asound/cardN/streamM`, written by `snd-usb-audio`.
pub fn parse_usb_stream(text: &str) -> Option<UsbStream> {
    let mut lines = text.lines();
    let first = lines.next()?;
    let (card, _) = first.rsplit_once(" : ").unwrap_or((first, ""));
    let mut stream = UsbStream::default();
    if let Some((product, rest)) = card.split_once(" at ") {
        stream.product = Some(collapse_repeat(product.trim())).filter(|name| !name.is_empty());
        let mut parts = rest.split(", ");
        stream.bus_path = parts.next().map(str::to_string);
        stream.speed = parts.next().map(|speed| speed.trim().to_string());
    } else {
        stream.product = Some(collapse_repeat(card.trim())).filter(|name| !name.is_empty());
    }
    let mut in_playback = false;
    let mut interface = None;
    let mut current: Option<UsbAltset> = None;
    for line in lines {
        let trimmed = line.trim();
        if !line.starts_with(' ') && !trimmed.is_empty() {
            if let Some(done) = current.take().filter(|_| in_playback) {
                stream.playback.push(done);
            }
            in_playback = trimmed.starts_with("Playback:");
            continue;
        }
        if !in_playback {
            continue;
        }
        if trimmed.starts_with("Interface =") || trimmed.starts_with("Altset =") {
            // The running endpoint's summary, not a descriptor.
            continue;
        }
        if let Some(status) = trimmed.strip_prefix("Status:") {
            stream.running = status.trim() == "Running";
        } else if let Some(freq) = trimmed.strip_prefix("Momentary freq = ") {
            stream.momentary_hz = freq
                .split_whitespace()
                .next()
                .and_then(|hz| hz.parse().ok());
        } else if let Some(number) = trimmed.strip_prefix("Interface ") {
            if let Some(done) = current.take() {
                stream.playback.push(done);
            }
            interface = number.trim().parse().ok();
        } else if let Some(number) = trimmed.strip_prefix("Altset ") {
            if let Some(done) = current.take() {
                stream.playback.push(done);
            }
            current = Some(UsbAltset {
                interface,
                altset: number.trim().parse().ok(),
                formats: Vec::new(),
                channels: None,
                rates: Vec::new(),
                bits: None,
            });
        } else if let Some(altset) = current.as_mut() {
            if let Some(formats) = trimmed.strip_prefix("Format:") {
                altset.formats = formats.split_whitespace().map(str::to_string).collect();
            } else if let Some(channels) = trimmed.strip_prefix("Channels:") {
                altset.channels = channels.trim().parse().ok();
            } else if let Some(rates) = trimmed.strip_prefix("Rates:") {
                altset.rates = parse_rates(rates);
            } else if let Some(bits) = trimmed.strip_prefix("Bits:") {
                altset.bits = bits.trim().parse().ok();
            }
        }
    }
    if let Some(done) = current.take().filter(|_| in_playback) {
        stream.playback.push(done);
    }
    Some(stream)
}

/// What the DAC's USB descriptors declare, as [`DeviceCaps`].
pub fn caps_from_usb(stream: &UsbStream) -> Option<DeviceCaps> {
    if stream.playback.is_empty() {
        return None;
    }
    let mut caps = DeviceCaps {
        min_channels: u8::MAX,
        ..DeviceCaps::default()
    };
    for altset in &stream.playback {
        let channels = altset.channels.unwrap_or(2).max(1);
        let formats: Vec<SampleFormat> = altset
            .formats
            .iter()
            .filter_map(|name| format_from_alsa_name(name))
            .collect();
        let pcm: Vec<SampleFormat> = formats.iter().copied().filter(|f| !f.is_dsd()).collect();
        if !pcm.is_empty() {
            caps.min_channels = caps.min_channels.min(channels);
            caps.max_channels = caps.max_channels.max(channels);
            for format in pcm {
                if !caps.sample_formats.contains(&format) {
                    caps.sample_formats.push(format);
                }
            }
            for rate in &altset.rates {
                if STANDARD_RATES.contains(rate) && !caps.sample_rates.contains(rate) {
                    caps.sample_rates.push(*rate);
                }
            }
        }
        for format in formats.iter().copied().filter(|f| f.is_dsd()) {
            if !caps.dsd_formats.contains(&format) {
                caps.dsd_formats.push(format);
            }
        }
    }
    caps.dsd_formats
        .sort_by_key(|format| DSD_FORMATS.iter().position(|known| known == format));
    if let Some(preferred) = caps.dsd_formats.first().copied() {
        for altset in &stream.playback {
            if altset
                .formats
                .iter()
                .any(|name| format_from_alsa_name(name) == Some(preferred))
            {
                for rate in &altset.rates {
                    let dsd_rate = rate * 8 * preferred.bytes() as u32;
                    if DSD_RATES.contains(&dsd_rate) && !caps.dsd_rates.contains(&dsd_rate) {
                        caps.dsd_rates.push(dsd_rate);
                    }
                }
            }
        }
    }
    if caps.min_channels == u8::MAX {
        caps.min_channels = 2;
        caps.max_channels = 2;
    }
    caps.sample_rates.sort_unstable();
    caps.sample_formats.sort_unstable();
    caps.dsd_rates.sort_unstable();
    Some(caps)
}

/// A running PCM's `hw_params`.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct HwParams {
    pub access: Option<String>,
    /// Kernel format name, e.g. `S24_3LE`.
    pub format: String,
    pub channels: u8,
    pub rate: u32,
    pub period_size: Option<u32>,
    pub buffer_size: Option<u32>,
}

/// Parses `pcmNp/sub0/hw_params`; `None` while the PCM is closed.
pub fn parse_hw_params(text: &str) -> Option<HwParams> {
    let mut access = None;
    let mut format = None;
    let mut channels = None;
    let mut rate = None;
    let mut period_size = None;
    let mut buffer_size = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "access" => access = Some(value.to_string()),
            "format" => format = Some(value.to_string()),
            "channels" => channels = value.parse().ok(),
            "rate" => rate = value.split_whitespace().next().and_then(|v| v.parse().ok()),
            "period_size" => period_size = value.parse().ok(),
            "buffer_size" => buffer_size = value.parse().ok(),
            _ => {}
        }
    }
    Some(HwParams {
        access,
        format: format?,
        channels: channels?,
        rate: rate?,
        period_size,
        buffer_size,
    })
}

/// A USB device id file (`usbid`), e.g. `21b4:0082`.
pub fn parse_usb_id(text: &str) -> Option<String> {
    let id = text.trim();
    let (vendor, product) = id.split_once(':')?;
    (vendor.len() == 4
        && product.len() == 4
        && id.chars().all(|c| c == ':' || c.is_ascii_hexdigit()))
    .then(|| id.to_ascii_lowercase())
}

/// What the kernel says about a device's hardware.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct HardwareInfo {
    /// `usb`, or `pci`/`platform` for built-in cards.
    pub bus: String,
    pub usb_id: Option<String>,
    /// The product name from the USB descriptors.
    pub product: Option<String>,
    pub speed: Option<String>,
    /// Where the format list came from.
    pub caps_source: CapsSource,
    /// The playback alternate settings the descriptors declare.
    pub altsets: Vec<UsbAltset>,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapsSource {
    /// libasound opened the device and tested each format.
    Driver,
    /// The device was busy; the formats are the USB descriptors'.
    UsbDescriptors,
}

/// Reads `<root>/<card>/usbid` and `stream<device>`, where `card` is
/// `card1` or the card's id (the kernel links `/proc/asound/<id>` to its
/// `cardN` directory).
pub fn read_usb(root: &Path, card: &str, device: i32) -> Option<(Option<String>, UsbStream)> {
    let dir = root.join(card);
    let stream = std::fs::read_to_string(dir.join(format!("stream{device}"))).ok()?;
    let usb_id = std::fs::read_to_string(dir.join("usbid"))
        .ok()
        .and_then(|text| parse_usb_id(&text));
    Some((usb_id, parse_usb_stream(&stream)?))
}

/// The kernel's account of a stream that is playing.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct HardwareStatus {
    /// The file read, e.g. `/proc/asound/card1/pcm0p/sub0/hw_params`.
    pub source: String,
    pub params: HwParams,
    /// The USB DAC's measured clock while streaming, when it reports one.
    pub momentary_hz: Option<u32>,
    /// The hardware runs exactly the container, rate and channels Parson
    /// opened it with (nothing in between converted them).
    pub matches_stream: bool,
}

/// `<root>/<card>/pcm<D>p/sub0/hw_params`.
pub fn hw_params_path(root: &Path, card: &str, device: i32) -> PathBuf {
    root.join(card)
        .join(format!("pcm{device}p"))
        .join("sub0")
        .join("hw_params")
}

/// Reads what the hardware runs at and compares it with `stream`.
pub fn read_status(
    root: &Path,
    card: &str,
    device: i32,
    stream: &StreamFormat,
) -> Option<HardwareStatus> {
    let path = hw_params_path(root, card, device);
    let params = parse_hw_params(&std::fs::read_to_string(&path).ok()?)?;
    let momentary_hz = std::fs::read_to_string(root.join(card).join(format!("stream{device}")))
        .ok()
        .and_then(|text| parse_usb_stream(&text))
        .and_then(|usb| usb.momentary_hz);
    Some(HardwareStatus {
        source: path.display().to_string(),
        matches_stream: params_match(&params, stream),
        params,
        momentary_hz,
    })
}

pub fn params_match(params: &HwParams, stream: &StreamFormat) -> bool {
    params.format == alsa_format_name(stream.sample_format)
        && params.rate == stream.sample_rate
        && params.channels == stream.channels
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The layout `snd-usb-audio`'s proc.c writes for a stopped DAC.
    const DRAGONFLY_STOPPED: &str = "\
AudioQuest AudioQuest DragonFly Red v1.0 at usb-0000:00:14.0-2, high speed : USB Audio

Playback:
  Status: Stop
  Interface 1
    Altset 1
    Format: S24_3LE
    Channels: 2
    Endpoint: 0x01 (1 OUT) (ASYNC)
    Rates: 44100, 48000, 88200, 96000
    Data packet interval: 125 us
    Bits: 24
    Channel map: FL FR
    Sync Endpoint: 0x81 (1 IN)
    Sync EP Interface: 1
    Sync EP Altset: 1
    Implicit Feedback Mode: No
";

    /// A DSD-capable XMOS DAC while streaming, with a capture section the
    /// parser must ignore.
    const XMOS_RUNNING: &str = "\
Topping D10s at usb-0000:00:14.0-1, high speed : USB Audio

Playback:
  Status: Running
    Interface = 1
    Altset = 2
    Packet Size = 72
    Momentary freq = 96000 Hz (0xc.0000)
    Feedback Format = 16.16
  Interface 1
    Altset 1
    Format: S32_LE DSD_U32_BE
    Channels: 2
    Endpoint: 0x01 (1 OUT) (ASYNC)
    Rates: 44100, 48000, 88200, 96000, 176400, 192000, 352800, 384000
    Bits: 32
  Interface 1
    Altset 2
    Format: S24_3LE
    Channels: 2
    Rates: 44100 - 96000 (continuous)
    Bits: 24

Capture:
  Status: Stop
  Interface 2
    Altset 1
    Format: S16_LE
    Channels: 1
    Rates: 8000, 16000
";

    #[test]
    fn a_usb_dac_is_named_from_its_descriptors() {
        let stream = parse_usb_stream(DRAGONFLY_STOPPED).unwrap();
        assert_eq!(
            stream.product.as_deref(),
            Some("AudioQuest DragonFly Red v1.0")
        );
        assert_eq!(stream.bus_path.as_deref(), Some("usb-0000:00:14.0-2"));
        assert_eq!(stream.speed.as_deref(), Some("high speed"));
        assert!(!stream.running);
        assert_eq!(stream.playback.len(), 1);
        let caps = caps_from_usb(&stream).unwrap();
        assert_eq!(caps.sample_rates, [44_100, 48_000, 88_200, 96_000]);
        assert_eq!(caps.sample_formats, [SampleFormat::S24Packed]);
        assert_eq!(caps.bit_depths(), [24]);
        assert!(caps.dsd_formats.is_empty());
    }

    #[test]
    fn dsd_formats_rates_and_the_running_clock_are_read() {
        let stream = parse_usb_stream(XMOS_RUNNING).unwrap();
        assert_eq!(stream.product.as_deref(), Some("Topping D10s"));
        assert!(stream.running);
        assert_eq!(stream.momentary_hz, Some(96_000));
        assert_eq!(stream.playback.len(), 2, "capture is ignored");
        let caps = caps_from_usb(&stream).unwrap();
        assert_eq!(caps.dsd_formats, [SampleFormat::DsdU32Be]);
        // DSD_U32_BE frames at 88.2k..352.8k carry DSD64..DSD256.
        assert_eq!(caps.dsd_rates, [2_822_400, 5_644_800, 11_289_600]);
        assert!(caps.sample_rates.contains(&384_000));
        assert_eq!(
            caps.sample_formats,
            [SampleFormat::S24Packed, SampleFormat::S32]
        );
        assert_eq!(stream.playback[1].rates, [44_100, 48_000, 88_200, 96_000]);
    }

    #[test]
    fn hw_params_report_what_the_hardware_runs() {
        assert_eq!(parse_hw_params("closed\n"), None);
        let params = parse_hw_params(
            "access: RW_INTERLEAVED\nformat: S24_3LE\nsubformat: STD\nchannels: 2\nrate: 96000 (96000/1)\nperiod_size: 4800\nbuffer_size: 19200\n",
        )
        .unwrap();
        assert_eq!(params.format, "S24_3LE");
        assert_eq!(params.rate, 96_000);
        assert_eq!(params.buffer_size, Some(19_200));
        let stream = StreamFormat {
            sample_rate: 96_000,
            sample_format: SampleFormat::S24Packed,
            channels: 2,
        };
        assert!(params_match(&params, &stream));
        assert!(!params_match(
            &params,
            &StreamFormat {
                sample_rate: 44_100,
                ..stream
            }
        ));
    }

    #[test]
    fn a_fixture_tree_is_read_like_proc_asound() {
        let root = std::env::temp_dir().join(format!("parson-procfs-{}", std::process::id()));
        let pcm = root.join("card1").join("pcm0p").join("sub0");
        std::fs::create_dir_all(&pcm).unwrap();
        std::fs::write(root.join("card1").join("usbid"), "21b4:0082\n").unwrap();
        std::fs::write(root.join("card1").join("stream0"), XMOS_RUNNING).unwrap();
        std::fs::write(
            pcm.join("hw_params"),
            "access: RW_INTERLEAVED\nformat: DSD_U32_BE\nsubformat: STD\nchannels: 2\nrate: 88200 (88200/1)\n",
        )
        .unwrap();
        let (usb_id, stream) = read_usb(&root, "card1", 0).unwrap();
        assert_eq!(usb_id.as_deref(), Some("21b4:0082"));
        assert_eq!(stream.product.as_deref(), Some("Topping D10s"));
        let status = read_status(
            &root,
            "card1",
            0,
            &StreamFormat {
                sample_rate: 88_200,
                sample_format: SampleFormat::DsdU32Be,
                channels: 2,
            },
        )
        .unwrap();
        assert!(status.matches_stream);
        assert_eq!(status.momentary_hz, Some(96_000));
        assert!(status.source.ends_with("card1/pcm0p/sub0/hw_params"));
        assert_eq!(read_usb(&root, "card2", 0), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn usb_ids_and_names_are_validated() {
        assert_eq!(parse_usb_id("21B4:0082\n").as_deref(), Some("21b4:0082"));
        assert_eq!(parse_usb_id("nonsense"), None);
        assert_eq!(collapse_repeat("Topping Topping D10s"), "Topping D10s");
        assert_eq!(collapse_repeat("RME ADI-2 DAC"), "RME ADI-2 DAC");
        for format in DSD_FORMATS {
            assert_eq!(
                format_from_alsa_name(alsa_format_name(format)),
                Some(format)
            );
        }
    }
}
