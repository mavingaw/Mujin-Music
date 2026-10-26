//! Google Cast v2 framing: each message is a 4-byte big-endian length and
//! a protobuf `CastMessage`:
//!
//! ```text
//! message CastMessage {
//!   required ProtocolVersion protocol_version = 1;  // CASTV2_1_0 = 0
//!   required string source_id = 2;
//!   required string destination_id = 3;
//!   required string namespace = 4;
//!   required PayloadType payload_type = 5;           // STRING = 0
//!   optional string payload_utf8 = 6;
//!   optional bytes payload_binary = 7;
//! }
//! ```

pub const NS_CONNECTION: &str = "urn:x-cast:com.google.cast.tp.connection";
pub const NS_HEARTBEAT: &str = "urn:x-cast:com.google.cast.tp.heartbeat";
pub const NS_RECEIVER: &str = "urn:x-cast:com.google.cast.receiver";
pub const NS_MEDIA: &str = "urn:x-cast:com.google.cast.media";
/// The Default Media Receiver.
pub const DEFAULT_MEDIA_RECEIVER: &str = "CC1AD845";
pub const MAX_MESSAGE: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CastMessage {
    pub source: String,
    pub destination: String,
    pub namespace: String,
    pub payload: String,
}

fn varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn string(out: &mut Vec<u8>, field: u8, value: &str) {
    out.push((field << 3) | 2);
    varint(out, value.len() as u64);
    out.extend_from_slice(value.as_bytes());
}

impl CastMessage {
    pub fn new(source: &str, destination: &str, namespace: &str, payload: String) -> Self {
        Self {
            source: source.to_string(),
            destination: destination.to_string(),
            namespace: namespace.to_string(),
            payload,
        }
    }

    /// The length-prefixed frame.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Vec::with_capacity(self.payload.len() + 96);
        body.extend_from_slice(&[0x08, 0x00]);
        string(&mut body, 2, &self.source);
        string(&mut body, 3, &self.destination);
        string(&mut body, 4, &self.namespace);
        body.extend_from_slice(&[0x28, 0x00]);
        string(&mut body, 6, &self.payload);
        let mut frame = Vec::with_capacity(body.len() + 4);
        frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frame.extend_from_slice(&body);
        frame
    }

    /// Decodes one message body (without the length prefix).
    pub fn decode(body: &[u8]) -> Option<Self> {
        let mut message = Self {
            source: String::new(),
            destination: String::new(),
            namespace: String::new(),
            payload: String::new(),
        };
        let mut position = 0;
        let read_varint = |position: &mut usize| -> Option<u64> {
            let mut value = 0_u64;
            for shift in (0..64).step_by(7) {
                let byte = *body.get(*position)?;
                *position += 1;
                value |= u64::from(byte & 0x7f) << shift;
                if byte & 0x80 == 0 {
                    return Some(value);
                }
            }
            None
        };
        while position < body.len() {
            let key = read_varint(&mut position)?;
            let (field, wire) = (key >> 3, key & 7);
            match wire {
                0 => {
                    read_varint(&mut position)?;
                }
                2 => {
                    let length = read_varint(&mut position)? as usize;
                    let end = position.checked_add(length)?;
                    let bytes = body.get(position..end)?;
                    position = end;
                    let text = || String::from_utf8(bytes.to_vec()).ok();
                    match field {
                        2 => message.source = text()?,
                        3 => message.destination = text()?,
                        4 => message.namespace = text()?,
                        6 => message.payload = text()?,
                        _ => {}
                    }
                }
                1 => position += 8,
                5 => position += 4,
                _ => return None,
            }
        }
        Some(message)
    }
}

/// Splits complete frames off the front of `buffer`.
pub fn take_frames(buffer: &mut Vec<u8>) -> Result<Vec<CastMessage>, String> {
    let mut messages = Vec::new();
    loop {
        if buffer.len() < 4 {
            return Ok(messages);
        }
        let length = u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) as usize;
        if length > MAX_MESSAGE {
            return Err("cast message too large".into());
        }
        if buffer.len() < 4 + length {
            return Ok(messages);
        }
        let body: Vec<u8> = buffer.drain(..4 + length).skip(4).collect();
        messages.push(CastMessage::decode(&body).ok_or("malformed cast message")?);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_through_the_protobuf_framing() {
        let message = CastMessage::new(
            "sender-0",
            "receiver-0",
            NS_RECEIVER,
            r#"{"type":"GET_STATUS","requestId":1}"#.to_string(),
        );
        let frame = message.encode();
        // Known prefix: version 0, then field 2 "sender-0".
        assert_eq!(&frame[4..8], &[0x08, 0x00, 0x12, 0x08]);
        let mut buffer = frame.clone();
        buffer.extend_from_slice(&frame[..10]);
        let messages = take_frames(&mut buffer).unwrap();
        assert_eq!(messages, vec![message]);
        assert_eq!(buffer.len(), 10, "a partial frame waits for more bytes");
        let mut huge = vec![0xff, 0xff, 0xff, 0xff];
        assert!(take_frames(&mut huge).is_err());
        // A long payload needs a multi-byte length.
        let long = CastMessage::new("a", "b", NS_MEDIA, "x".repeat(300));
        let mut buffer = long.encode();
        assert_eq!(take_frames(&mut buffer).unwrap(), vec![long]);
    }
}
