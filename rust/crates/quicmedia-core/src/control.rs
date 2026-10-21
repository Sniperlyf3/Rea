//! Reliable control protocol for stream management, carried on a dedicated QUIC
//! bidirectional stream (distinct from any user data channel).
//!
//! Messages are self-delimiting: a 1-byte tag followed by a fixed-size body.
//! A peer announces a stream it will send with [`ControlMsg::Open`], either peer
//! signals its desired active state with [`ControlMsg::SetActive`], and either
//! peer tears a stream down with [`ControlMsg::Close`].
//!
//! The *effective* media flow on a stream is `sender_wants AND receiver_wants`:
//! the producing side transmits only while both ends want it active.

use quinn::{RecvStream, SendStream};

/// First byte written on a freshly opened bidi stream, so the acceptor can tell
/// the internal control stream from the user data channel regardless of order.
pub const PURPOSE_CONTROL: u8 = 0;
pub const PURPOSE_DATA: u8 = 1;

const TAG_OPEN: u8 = 1;
const TAG_SET_ACTIVE: u8 = 2;
const TAG_CLOSE: u8 = 3;

const OPEN_BODY: usize = 18;
const SET_ACTIVE_BODY: usize = 3;
const CLOSE_BODY: usize = 2;

/// Parameters announced when a stream is opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenStream {
    pub stream_id: u16,
    pub payload_type: u8,
    pub sample_rate: u32,
    pub channels: u16,
    pub frame_ms: u32,
    pub bitrate: u32,
    pub active: bool,
}

/// A control-plane message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlMsg {
    Open(OpenStream),
    SetActive { stream_id: u16, active: bool },
    Close { stream_id: u16 },
}

impl ControlMsg {
    /// Serialize to bytes (tag + fixed body).
    pub fn encode(&self) -> Vec<u8> {
        match self {
            ControlMsg::Open(o) => {
                let mut v = Vec::with_capacity(1 + OPEN_BODY);
                v.push(TAG_OPEN);
                v.extend_from_slice(&o.stream_id.to_be_bytes());
                v.push(o.payload_type);
                v.extend_from_slice(&o.sample_rate.to_be_bytes());
                v.extend_from_slice(&o.channels.to_be_bytes());
                v.extend_from_slice(&o.frame_ms.to_be_bytes());
                v.extend_from_slice(&o.bitrate.to_be_bytes());
                v.push(o.active as u8);
                v
            }
            ControlMsg::SetActive { stream_id, active } => {
                let mut v = Vec::with_capacity(1 + SET_ACTIVE_BODY);
                v.push(TAG_SET_ACTIVE);
                v.extend_from_slice(&stream_id.to_be_bytes());
                v.push(*active as u8);
                v
            }
            ControlMsg::Close { stream_id } => {
                let mut v = Vec::with_capacity(1 + CLOSE_BODY);
                v.push(TAG_CLOSE);
                v.extend_from_slice(&stream_id.to_be_bytes());
                v
            }
        }
    }

    /// Parse one message from the front of `buf`, returning it and bytes consumed.
    pub fn decode(buf: &[u8]) -> Option<(ControlMsg, usize)> {
        let (&tag, rest) = buf.split_first()?;
        match tag {
            TAG_OPEN if rest.len() >= OPEN_BODY => {
                let o = OpenStream {
                    stream_id: u16::from_be_bytes([rest[0], rest[1]]),
                    payload_type: rest[2],
                    sample_rate: u32::from_be_bytes([rest[3], rest[4], rest[5], rest[6]]),
                    channels: u16::from_be_bytes([rest[7], rest[8]]),
                    frame_ms: u32::from_be_bytes([rest[9], rest[10], rest[11], rest[12]]),
                    bitrate: u32::from_be_bytes([rest[13], rest[14], rest[15], rest[16]]),
                    active: rest[17] != 0,
                };
                Some((ControlMsg::Open(o), 1 + OPEN_BODY))
            }
            TAG_SET_ACTIVE if rest.len() >= SET_ACTIVE_BODY => Some((
                ControlMsg::SetActive {
                    stream_id: u16::from_be_bytes([rest[0], rest[1]]),
                    active: rest[2] != 0,
                },
                1 + SET_ACTIVE_BODY,
            )),
            TAG_CLOSE if rest.len() >= CLOSE_BODY => Some((
                ControlMsg::Close {
                    stream_id: u16::from_be_bytes([rest[0], rest[1]]),
                },
                1 + CLOSE_BODY,
            )),
            _ => None,
        }
    }
}

/// Write one control message to the stream.
pub async fn write_message(send: &mut SendStream, msg: &ControlMsg) -> std::io::Result<()> {
    let bytes = msg.encode();
    send.write_all(&bytes)
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))
}

/// Read one control message. Returns `Ok(None)` at end of stream.
pub async fn read_message(recv: &mut RecvStream) -> std::io::Result<Option<ControlMsg>> {
    let mut tag = [0u8; 1];
    if recv.read_exact(&mut tag).await.is_err() {
        return Ok(None);
    }
    let body_len = match tag[0] {
        TAG_OPEN => OPEN_BODY,
        TAG_SET_ACTIVE => SET_ACTIVE_BODY,
        TAG_CLOSE => CLOSE_BODY,
        _ => return Ok(None), // unknown tag — stop reading this stream
    };
    let mut body = vec![0u8; 1 + body_len];
    body[0] = tag[0];
    if recv.read_exact(&mut body[1..]).await.is_err() {
        return Ok(None);
    }
    Ok(ControlMsg::decode(&body).map(|(m, _)| m))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(msg: ControlMsg) {
        let bytes = msg.encode();
        let (decoded, used) = ControlMsg::decode(&bytes).unwrap();
        assert_eq!(decoded, msg);
        assert_eq!(used, bytes.len());
    }

    #[test]
    fn control_roundtrip() {
        roundtrip(ControlMsg::Open(OpenStream {
            stream_id: 0xABCD,
            payload_type: 96,
            sample_rate: 48_000,
            channels: 2,
            frame_ms: 20,
            bitrate: 24_000,
            active: true,
        }));
        roundtrip(ControlMsg::SetActive {
            stream_id: 7,
            active: false,
        });
        roundtrip(ControlMsg::Close { stream_id: 9 });
    }

    #[test]
    fn decode_rejects_truncated() {
        assert!(ControlMsg::decode(&[TAG_OPEN, 0, 1]).is_none());
        assert!(ControlMsg::decode(&[]).is_none());
    }
}
