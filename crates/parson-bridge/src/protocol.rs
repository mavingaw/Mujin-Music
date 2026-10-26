//! The Parson Bridge wire protocol (version 1), over one TCP connection the
//! Parson server opens to the bridge (port 1994 unless configured).
//!
//! Every frame is `u32 length (big endian) | u8 kind | body`, where the
//! length counts the kind byte and the body:
//!
//! - kind 1: a JSON control message ([`ServerMessage`] or [`BridgeMessage`]);
//! - kind 2: audio for an open stream: `u32 stream id (big endian)` then
//!   whole interleaved PCM frames in the stream's format, exactly as the
//!   device gets them. Nothing is compressed, resampled or requantised on
//!   the way, so the DAC receives the server's samples bit for bit.
//!
//! Clocks: the bridge reports its own monotonic clock in microseconds
//! (`bridge_us`); the server estimates the offset from ping round trips and
//! sends every time already converted to the bridge's clock, so a bridge
//! never needs the server's time.

use std::io::{self, Read, Write};

use parson_output::{DeviceCaps, DeviceInfo, Driver, HardwareVolume, SampleFormat, StreamFormat};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
pub const DEFAULT_PORT: u16 = 1994;
/// DNS-SD service type bridges advertise.
pub const SERVICE_TYPE: &str = "_parson-bridge._tcp.local.";

const KIND_JSON: u8 = 1;
const KIND_AUDIO: u8 = 2;
/// Largest frame accepted: 4 MiB (about 3.6 s of 768 kHz/32-bit stereo).
pub const MAX_FRAME: usize = 4 * 1024 * 1024;

/// A stream format as the wire carries it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireFormat {
    pub sample_rate: u32,
    pub format: SampleFormat,
    pub channels: u8,
}

impl From<StreamFormat> for WireFormat {
    fn from(format: StreamFormat) -> Self {
        Self {
            sample_rate: format.sample_rate,
            format: format.sample_format,
            channels: format.channels,
        }
    }
}

impl From<WireFormat> for StreamFormat {
    fn from(format: WireFormat) -> Self {
        Self {
            sample_rate: format.sample_rate,
            sample_format: format.format,
            channels: format.channels,
        }
    }
}

/// One audio output of a bridge, as the wire carries it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WireDevice {
    pub id: String,
    pub name: String,
    pub description: String,
    /// PCM rates the device takes natively, ascending.
    pub sample_rates: Vec<u32>,
    pub formats: Vec<SampleFormat>,
    pub max_channels: u8,
    pub hardware_volume: Option<HardwareVolume>,
    /// Another app holds the device right now.
    pub busy: bool,
    /// Native DSD containers and rates (newer bridges only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dsd_formats: Vec<SampleFormat>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dsd_rates: Vec<u32>,
}

impl WireDevice {
    /// What a bridge sends for one of its devices (devices that could not
    /// be probed are listed with no formats).
    pub fn from_device(device: &DeviceInfo) -> Self {
        let caps = device.caps.clone().unwrap_or_default();
        Self {
            id: device.id.clone(),
            name: device.name.clone(),
            description: device.detail.clone(),
            sample_rates: caps.sample_rates,
            formats: caps.sample_formats,
            max_channels: caps.max_channels.max(1),
            hardware_volume: device.hardware_volume,
            busy: device.busy,
            dsd_formats: caps.dsd_formats,
            dsd_rates: caps.dsd_rates,
        }
    }

    /// The device as the server sees it: a Parson Bridge output.
    pub fn into_device(self) -> DeviceInfo {
        DeviceInfo {
            id: self.id,
            name: self.name,
            detail: self.description,
            driver: Driver::Bridge,
            exclusive_capable: true,
            caps: Some(DeviceCaps {
                sample_rates: self.sample_rates,
                sample_formats: self.formats,
                min_channels: 1,
                max_channels: self.max_channels.max(1),
                dsd_formats: self.dsd_formats,
                dsd_rates: self.dsd_rates,
            }),
            probe_error: self
                .busy
                .then(|| "in use by another app on the bridge".to_string()),
            is_default: false,
            hardware: None,
            address: None,
            network: None,
            hardware_volume: self.hardware_volume,
            busy: self.busy,
        }
    }
}

/// How a stream starts when it plays in a sample-synced group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncStart {
    /// When the first frame must be heard, on the bridge's clock.
    pub start_bridge_us: i64,
    /// The clock master never corrects itself; followers track it.
    pub master: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Hello {
        protocol: u32,
        server_id: String,
        server_name: String,
    },
    Ping {
        id: u64,
        server_us: i64,
    },
    /// Opens `device` at exactly `format` for stream `stream`.
    Open {
        stream: u32,
        device: String,
        format: WireFormat,
        exclusive: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sync: Option<SyncStart>,
    },
    Pause {
        stream: u32,
        paused: bool,
    },
    /// Discards what is buffered (seek, stop).
    Flush {
        stream: u32,
    },
    /// Plays out what is buffered, then reports `Drained`.
    Drain {
        stream: u32,
    },
    Close {
        stream: u32,
    },
    /// The clock master's heard position: source frame `frame` was heard at
    /// `bridge_us` (on the receiving bridge's clock).
    SyncRef {
        stream: u32,
        frame: u64,
        bridge_us: i64,
        /// The master's measured rate in frames per second of the
        /// receiving bridge's clock (its DAC clock seen from here).
        rate: f64,
    },
    Volume {
        device: String,
        db: f32,
    },
    Mute {
        device: String,
        muted: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeMessage {
    Welcome {
        protocol: u32,
        bridge_id: String,
        name: String,
        host: String,
        platform: String,
        version: String,
        /// `ALSA`, `Simulated`, or none.
        backend: Option<String>,
        /// Why there are no devices, when there are none.
        note: Option<String>,
        devices: Vec<WireDevice>,
    },
    Pong {
        id: u64,
        server_us: i64,
        bridge_us: i64,
    },
    Devices {
        devices: Vec<WireDevice>,
    },
    Opened {
        stream: u32,
    },
    Failed {
        stream: u32,
        error: String,
    },
    /// Playback progress of a stream.
    Status {
        stream: u32,
        /// Source frames received from the server.
        received: u64,
        /// Source frames heard so far (written to the device and played
        /// out), on `bridge_us`.
        heard: u64,
        bridge_us: i64,
        /// Frames inserted (positive) or dropped (negative) to follow the
        /// clock master; always 0 for a stream that is not synced.
        corrections: i64,
        underruns: u32,
        /// Playback has begun (a synced stream waits for its start time).
        started: bool,
        /// The device is paused: `heard` holds until playback resumes.
        /// Older bridges leave it out.
        #[serde(default)]
        paused: bool,
    },
    Drained {
        stream: u32,
    },
    /// The bridge will not play for this server (another one is using it,
    /// or the protocol versions differ); the connection closes.
    Refused {
        reason: String,
    },
    /// A volume or mute change the device refused.
    DeviceError {
        device: String,
        error: String,
    },
}

/// A decoded frame.
#[derive(Debug, PartialEq)]
pub enum Frame<M> {
    Message(M),
    Audio { stream: u32, data: Vec<u8> },
}

pub fn write_message<W: Write, M: Serialize>(writer: &mut W, message: &M) -> io::Result<()> {
    let body = serde_json::to_vec(message).map_err(io::Error::other)?;
    let mut frame = Vec::with_capacity(body.len() + 5);
    frame.extend_from_slice(&((body.len() + 1) as u32).to_be_bytes());
    frame.push(KIND_JSON);
    frame.extend_from_slice(&body);
    writer.write_all(&frame)
}

pub fn write_audio<W: Write>(writer: &mut W, stream: u32, data: &[u8]) -> io::Result<()> {
    if data.len() + 5 > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "audio frame too large",
        ));
    }
    let mut frame = Vec::with_capacity(data.len() + 9);
    frame.extend_from_slice(&((data.len() + 5) as u32).to_be_bytes());
    frame.push(KIND_AUDIO);
    frame.extend_from_slice(&stream.to_be_bytes());
    frame.extend_from_slice(data);
    writer.write_all(&frame)
}

pub fn read_frame<R: Read, M: for<'de> Deserialize<'de>>(reader: &mut R) -> io::Result<Frame<M>> {
    let mut header = [0_u8; 4];
    reader.read_exact(&mut header)?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bad frame length",
        ));
    }
    let mut body = vec![0_u8; length];
    reader.read_exact(&mut body)?;
    match body[0] {
        KIND_JSON => serde_json::from_slice(&body[1..])
            .map(Frame::Message)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
        KIND_AUDIO if body.len() >= 5 => {
            let stream = u32::from_be_bytes([body[1], body[2], body[3], body[4]]);
            body.drain(..5);
            Ok(Frame::Audio { stream, data: body })
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unknown frame kind",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_and_bad_input_is_refused() {
        let mut wire = Vec::new();
        let open = ServerMessage::Open {
            stream: 7,
            device: "hw-2-0".into(),
            format: WireFormat {
                sample_rate: 192_000,
                format: SampleFormat::S24Packed,
                channels: 2,
            },
            exclusive: true,
            sync: Some(SyncStart {
                start_bridge_us: 42,
                master: false,
            }),
        };
        write_message(&mut wire, &open).unwrap();
        write_audio(&mut wire, 7, &[1, 2, 3, 4, 5, 6]).unwrap();
        let mut reader = wire.as_slice();
        assert_eq!(
            read_frame::<_, ServerMessage>(&mut reader).unwrap(),
            Frame::Message(open)
        );
        assert_eq!(
            read_frame::<_, ServerMessage>(&mut reader).unwrap(),
            Frame::Audio {
                stream: 7,
                data: vec![1, 2, 3, 4, 5, 6]
            }
        );
        assert!(read_frame::<_, ServerMessage>(&mut reader).is_err(), "end");
        let json = String::from_utf8(wire[5..].to_vec()).unwrap();
        assert!(json.starts_with(r#"{"type":"open","stream":7"#), "{json}");
        let huge = (MAX_FRAME as u32 + 1).to_be_bytes();
        assert!(read_frame::<_, ServerMessage>(&mut huge.as_slice()).is_err());
        let unknown = [0, 0, 0, 1, 9];
        assert!(read_frame::<_, ServerMessage>(&mut unknown.as_slice()).is_err());
    }

    #[test]
    fn devices_keep_their_version_1_shape_on_the_wire() {
        let device = WireDevice {
            id: "hw-2-0".into(),
            name: "E30".into(),
            description: "USB Audio".into(),
            sample_rates: vec![44_100, 96_000],
            formats: vec![SampleFormat::S16, SampleFormat::S24Packed],
            max_channels: 2,
            hardware_volume: None,
            busy: false,
            dsd_formats: Vec::new(),
            dsd_rates: Vec::new(),
        };
        let json = serde_json::to_value(&device).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "id": "hw-2-0", "name": "E30", "description": "USB Audio",
                "sample_rates": [44_100, 96_000], "formats": ["s16", "s24_packed"],
                "max_channels": 2, "hardware_volume": null, "busy": false
            }),
            "what bridges and servers of version 1 exchange"
        );
        let back: WireDevice = serde_json::from_value(json).unwrap();
        let info = back.into_device();
        assert_eq!(info.driver, Driver::Bridge);
        assert_eq!(info.sample_rates(), [44_100, 96_000]);
        assert_eq!(WireDevice::from_device(&info), device);
    }
}
