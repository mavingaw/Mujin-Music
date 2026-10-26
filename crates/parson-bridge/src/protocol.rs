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

use serde::{Deserialize, Serialize};

use crate::audio::{DeviceInfo, StreamFormat};

pub const PROTOCOL_VERSION: u32 = 1;
pub const DEFAULT_PORT: u16 = 1994;
/// DNS-SD service type bridges advertise.
pub const SERVICE_TYPE: &str = "_parson-bridge._tcp.local.";

const KIND_JSON: u8 = 1;
const KIND_AUDIO: u8 = 2;
/// Largest frame accepted: 4 MiB (about 3.6 s of 768 kHz/32-bit stereo).
pub const MAX_FRAME: usize = 4 * 1024 * 1024;

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
        format: StreamFormat,
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
        devices: Vec<DeviceInfo>,
    },
    Pong {
        id: u64,
        server_us: i64,
        bridge_us: i64,
    },
    Devices {
        devices: Vec<DeviceInfo>,
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
    use crate::audio::SampleFormat;

    #[test]
    fn frames_round_trip_and_bad_input_is_refused() {
        let mut wire = Vec::new();
        let open = ServerMessage::Open {
            stream: 7,
            device: "hw-2-0".into(),
            format: StreamFormat {
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
}
