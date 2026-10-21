//! Compact RTP-like framing carried inside unreliable QUIC DATAGRAMs.
//!
//! QUIC already provides connection identity, encryption, path validation and
//! congestion control, so the per-packet header only carries what the media
//! pipeline itself consumes — a stream identifier (several logical audio streams
//! can share one connection), a payload type, a marker bit, a sequence number
//! for loss/reorder detection, and a media timestamp for playout. This mirrors
//! the intent of the IETF "RTP over QUIC" (RoQ) work while staying minimal.
//!
//! Wire layout (10 bytes, big-endian), followed by the opaque media payload:
//!
//! ```text
//!  0               1               2               3
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |V V|M| RSV   | payload_type  |           stream_id           |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |           sequence            |            timestamp  (hi)    |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |            timestamp (lo)      |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! ```
//!
//! * `V` (2 bits): version, currently 2.
//! * `M` (1 bit): marker (e.g. talk-spurt boundary).
//! * `RSV` (5 bits): reserved, sent as 0.
//! * `payload_type` (8 bits): codec identifier.
//! * `stream_id` (16 bits): which logical stream on this connection.
//! * `sequence` (16 bits): per-stream packet counter, wraps at 2^16.
//! * `timestamp` (32 bits): media clock in samples, wraps at 2^32.

use crate::{Error, Result};

/// Size of the media header in bytes.
pub const HEADER_LEN: usize = 10;

/// Protocol version encoded in the top 2 bits of byte 0.
pub const VERSION: u8 = 2;

/// Payload type for an Opus audio stream.
pub const PT_OPUS: u8 = 96;
/// Payload type for uncompressed 16-bit PCM passthrough (tests/LAN).
pub const PT_PCM: u8 = 97;

/// A logical stream identifier within a connection.
pub type StreamId = u16;

/// Parsed media header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaHeader {
    pub marker: bool,
    pub payload_type: u8,
    pub stream_id: StreamId,
    pub sequence: u16,
    pub timestamp: u32,
}

impl MediaHeader {
    /// Serialize the header into the first [`HEADER_LEN`] bytes of `out`.
    ///
    /// # Panics
    /// Panics if `out.len() < HEADER_LEN`.
    pub fn write(&self, out: &mut [u8]) {
        assert!(out.len() >= HEADER_LEN, "buffer too small for media header");
        let marker = if self.marker { 1u8 } else { 0u8 };
        out[0] = (VERSION << 6) | (marker << 5);
        out[1] = self.payload_type;
        out[2..4].copy_from_slice(&self.stream_id.to_be_bytes());
        out[4..6].copy_from_slice(&self.sequence.to_be_bytes());
        out[6..10].copy_from_slice(&self.timestamp.to_be_bytes());
    }

    /// Serialize into a fresh `Vec` with `payload` appended.
    pub fn to_datagram(&self, payload: &[u8]) -> Vec<u8> {
        let mut buf = vec![0u8; HEADER_LEN + payload.len()];
        self.write(&mut buf);
        buf[HEADER_LEN..].copy_from_slice(payload);
        buf
    }

    /// Parse a header from the front of `buf`, returning it plus the payload slice.
    pub fn parse(buf: &[u8]) -> Result<(MediaHeader, &[u8])> {
        if buf.len() < HEADER_LEN {
            return Err(Error::InvalidArg(
                "datagram shorter than media header".into(),
            ));
        }
        let version = buf[0] >> 6;
        if version != VERSION {
            return Err(Error::InvalidArg(format!(
                "unsupported framing version {version}"
            )));
        }
        let marker = (buf[0] & 0b0010_0000) != 0;
        let payload_type = buf[1];
        let stream_id = u16::from_be_bytes([buf[2], buf[3]]);
        let sequence = u16::from_be_bytes([buf[4], buf[5]]);
        let timestamp = u32::from_be_bytes([buf[6], buf[7], buf[8], buf[9]]);
        Ok((
            MediaHeader {
                marker,
                payload_type,
                stream_id,
                sequence,
                timestamp,
            },
            &buf[HEADER_LEN..],
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let h = MediaHeader {
            marker: true,
            payload_type: PT_OPUS,
            stream_id: 0x1234,
            sequence: 0xBEEF,
            timestamp: 0xDEAD_C0DE,
        };
        let dgram = h.to_datagram(&[1, 2, 3]);
        assert_eq!(dgram.len(), HEADER_LEN + 3);
        let (parsed, payload) = MediaHeader::parse(&dgram).unwrap();
        assert_eq!(parsed, h);
        assert_eq!(payload, &[1, 2, 3]);
    }

    #[test]
    fn rejects_short() {
        assert!(MediaHeader::parse(&[0u8; 6]).is_err());
    }

    #[test]
    fn rejects_bad_version() {
        let mut buf = [0u8; HEADER_LEN];
        buf[0] = 1 << 6;
        assert!(MediaHeader::parse(&buf).is_err());
    }
}
