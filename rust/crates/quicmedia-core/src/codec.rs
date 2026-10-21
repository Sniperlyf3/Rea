//! Audio codec abstraction.
//!
//! The media path only ever sees encoded payloads on the wire. Encoding and
//! decoding happen entirely in native code so raw or compressed audio never
//! crosses the managed (C#) boundary.
//!
//! Two implementations are provided:
//! * [`OpusCodec`] — real Opus, behind the `opus-codec` feature.
//! * [`PassthroughCodec`] — uncompressed little-endian 16-bit PCM, always
//!   available. Useful for tests and LAN links, and as a graceful fallback when
//!   Opus is not compiled in.

use crate::config::{AudioParams, CodecKind};
use crate::{Error, Result};

/// A frame-oriented audio codec. One call encodes/decodes exactly one frame.
pub trait Codec: Send {
    /// Encode one frame of interleaved PCM into `out` (cleared first).
    fn encode(&mut self, pcm: &[i16], out: &mut Vec<u8>) -> Result<()>;

    /// Decode one frame into `out` (cleared first). `data == None` signals a
    /// lost packet and requests concealment (PLC or silence).
    fn decode(&mut self, data: Option<&[u8]>, out: &mut Vec<i16>) -> Result<()>;
}

/// Map a codec kind to its on-wire payload type.
pub fn payload_type(kind: CodecKind) -> u8 {
    match kind {
        CodecKind::Opus => crate::framing::PT_OPUS,
        CodecKind::Passthrough => crate::framing::PT_PCM,
    }
}

/// Map an on-wire payload type back to a codec kind.
pub fn codec_kind_from_payload_type(pt: u8) -> Option<CodecKind> {
    match pt {
        crate::framing::PT_OPUS => Some(CodecKind::Opus),
        crate::framing::PT_PCM => Some(CodecKind::Passthrough),
        _ => None,
    }
}

/// Build the codec requested by `params`, honouring compiled features.
pub fn make_codec(params: &AudioParams) -> Result<Box<dyn Codec>> {
    match params.codec {
        CodecKind::Passthrough => Ok(Box::new(PassthroughCodec::new(params))),
        CodecKind::Opus => {
            #[cfg(feature = "opus-codec")]
            {
                Ok(Box::new(OpusCodec::new(params)?))
            }
            #[cfg(not(feature = "opus-codec"))]
            {
                Err(Error::Unsupported(
                    "Opus requested but the `opus-codec` feature was not compiled in".into(),
                ))
            }
        }
    }
}

/// Uncompressed 16-bit PCM, little-endian on the wire.
pub struct PassthroughCodec {
    frame_len: usize,
}

impl PassthroughCodec {
    pub fn new(params: &AudioParams) -> Self {
        PassthroughCodec {
            frame_len: params.frame_len(),
        }
    }
}

impl Codec for PassthroughCodec {
    fn encode(&mut self, pcm: &[i16], out: &mut Vec<u8>) -> Result<()> {
        out.clear();
        out.reserve(pcm.len() * 2);
        for &s in pcm {
            out.extend_from_slice(&s.to_le_bytes());
        }
        Ok(())
    }

    fn decode(&mut self, data: Option<&[u8]>, out: &mut Vec<i16>) -> Result<()> {
        out.clear();
        match data {
            None => out.resize(self.frame_len, 0),
            Some(bytes) => {
                if bytes.len() % 2 != 0 {
                    return Err(Error::Codec(
                        "passthrough payload not 16-bit aligned".into(),
                    ));
                }
                out.reserve(bytes.len() / 2);
                for chunk in bytes.chunks_exact(2) {
                    out.push(i16::from_le_bytes([chunk[0], chunk[1]]));
                }
            }
        }
        Ok(())
    }
}

#[cfg(feature = "opus-codec")]
mod opus_impl {
    use super::*;
    use audiopus::{coder::Decoder, coder::Encoder, Application, Bitrate, Channels, SampleRate};

    fn sample_rate(hz: u32) -> Result<SampleRate> {
        Ok(match hz {
            8_000 => SampleRate::Hz8000,
            12_000 => SampleRate::Hz12000,
            16_000 => SampleRate::Hz16000,
            24_000 => SampleRate::Hz24000,
            48_000 => SampleRate::Hz48000,
            other => return Err(Error::Config(format!("Opus does not support {other} Hz"))),
        })
    }

    fn channels(n: u16) -> Result<Channels> {
        Ok(match n {
            1 => Channels::Mono,
            2 => Channels::Stereo,
            other => {
                return Err(Error::Config(format!(
                    "Opus supports 1 or 2 channels, got {other}"
                )))
            }
        })
    }

    /// Opus encoder + decoder pair sized for one configured frame.
    pub struct OpusCodec {
        encoder: Encoder,
        decoder: Decoder,
        frame_len: usize,
        channels: usize,
        enc_buf: Vec<u8>,
    }

    impl OpusCodec {
        pub fn new(params: &AudioParams) -> Result<Self> {
            let sr = sample_rate(params.sample_rate)?;
            let ch = channels(params.channels)?;
            let mut encoder = Encoder::new(sr, ch, Application::Voip)
                .map_err(|e| Error::Codec(format!("opus encoder init: {e}")))?;
            encoder
                .set_bitrate(Bitrate::BitsPerSecond(params.bitrate as i32))
                .map_err(|e| Error::Codec(format!("opus set_bitrate: {e}")))?;
            let decoder = Decoder::new(sr, ch)
                .map_err(|e| Error::Codec(format!("opus decoder init: {e}")))?;
            Ok(OpusCodec {
                encoder,
                decoder,
                frame_len: params.frame_len(),
                channels: params.channels as usize,
                // Opus packets are bounded well under 1500 bytes for voice.
                enc_buf: vec![0u8; 4000],
            })
        }
    }

    impl Codec for OpusCodec {
        fn encode(&mut self, pcm: &[i16], out: &mut Vec<u8>) -> Result<()> {
            let n = self
                .encoder
                .encode(pcm, &mut self.enc_buf)
                .map_err(|e| Error::Codec(format!("opus encode: {e}")))?;
            out.clear();
            out.extend_from_slice(&self.enc_buf[..n]);
            Ok(())
        }

        fn decode(&mut self, data: Option<&[u8]>, out: &mut Vec<i16>) -> Result<()> {
            // Opus needs an output buffer large enough for the frame; it returns
            // the number of samples *per channel* actually decoded.
            out.resize(self.frame_len, 0);
            let per_channel = self
                .decoder
                .decode(data, &mut out[..], false)
                .map_err(|e| Error::Codec(format!("opus decode: {e}")))?;
            out.truncate(per_channel * self.channels);
            Ok(())
        }
    }
}

#[cfg(feature = "opus-codec")]
pub use opus_impl::OpusCodec;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_roundtrip() {
        let params = AudioParams {
            codec: CodecKind::Passthrough,
            ..Default::default()
        };
        let mut c = PassthroughCodec::new(&params);
        let pcm: Vec<i16> = (0..params.frame_len() as i16).collect();
        let mut enc = Vec::new();
        c.encode(&pcm, &mut enc).unwrap();
        let mut dec = Vec::new();
        c.decode(Some(&enc), &mut dec).unwrap();
        assert_eq!(pcm, dec);
    }

    #[test]
    fn passthrough_conceals_with_silence() {
        let params = AudioParams {
            codec: CodecKind::Passthrough,
            ..Default::default()
        };
        let mut c = PassthroughCodec::new(&params);
        let mut dec = Vec::new();
        c.decode(None, &mut dec).unwrap();
        assert_eq!(dec.len(), params.frame_len());
        assert!(dec.iter().all(|&s| s == 0));
    }
}
