//! Compact RTP-like framing carried inside unreliable QUIC DATAGRAMs.
//!
//! We deliberately do not use full RFC 3550 RTP on the wire: QUIC already
//! provides connection identity, encryption, path validation and congestion
//! control, so the per-packet header only needs what the media pipeline itself
//! consumes — a payload type, a marker bit, a sequence number for loss/reorder
//! detection, and a media timestamp for playout. This mirrors the intent of the
//! IETF "RTP over QUIC" (RoQ) work while staying minimal.
//!
//! Wire layout (8 bytes, big-endian), followed by the opaque media payload:
//!
//! ```text
//!  0               1               2               3
//!  0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |V V|M|  RSV  | payload_type  |          sequence             |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |                          timestamp                          |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! ```
//!
//! * `V` (2 bits): version, currently 1.
//! * `M` (1 bit): marker (e.g. talk-spurt boundary).
//! * `RSV` (4 bits): reserved, sent as 0.
//! * `payload_type` (8 bits): codec/stream identifier.
//! * `sequence` (16 bits): per-stream packet counter, wraps at 2^16.
//! * `timestamp` (32 bits): media clock in samples, wraps at 2^32.

use crate::{Error, Result};

/// Size of the media header in bytes.
pub const HEADER_LEN: usize = 8;

/// Protocol version encoded in the top 2 bits of byte 0.
pub const VERSION: u8 = 1;

/// Payload type identifying an Opus (or passthrough PCM) audio stream.
pub const PT_AUDIO: u8 = 96;

/// Parsed media header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaHeader {
    pub marker: bool,
    pub payload_type: u8,
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
        out[2..4].copy_from_slice(&self.sequence.to_be_bytes());
        out[4..8].copy_from_slice(&self.timestamp.to_be_bytes());
    }

    /// Parse a header from the front of `buf`, returning it plus the payload slice.
    pub fn parse(buf: &[u8]) -> Result<(MediaHeader, &[u8])> {
        if buf.len() < HEADER_LEN {
            return Err(Error::InvalidArg("datagram shorter than media header".into()));
        }
        let version = buf[0] >> 6;
        if version != VERSION {
            return Err(Error::InvalidArg(format!("unsupported framing version {version}")));
        }
        let marker = (buf[0] & 0b0010_0000) != 0;
        let payload_type = buf[1];
        let sequence = u16::from_be_bytes([buf[2], buf[3]]);
        let timestamp = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
        Ok((
            MediaHeader { marker, payload_type, sequence, timestamp },
            &buf[HEADER_LEN..],
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let h = MediaHeader { marker: true, payload_type: PT_AUDIO, sequence: 0xBEEF, timestamp: 0xDEAD_C0DE };
        let mut buf = [0u8; HEADER_LEN + 3];
        h.write(&mut buf);
        buf[HEADER_LEN..].copy_from_slice(&[1, 2, 3]);
        let (parsed, payload) = MediaHeader::parse(&buf).unwrap();
        assert_eq!(parsed, h);
        assert_eq!(payload, &[1, 2, 3]);
    }

    #[test]
    fn rejects_short() {
        assert!(MediaHeader::parse(&[0u8; 4]).is_err());
    }

    #[test]
    fn rejects_bad_version() {
        let mut buf = [0u8; HEADER_LEN];
        buf[0] = 3 << 6;
        assert!(MediaHeader::parse(&buf).is_err());
    }
}
