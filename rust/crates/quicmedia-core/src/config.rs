//! Session configuration types.

use std::net::SocketAddr;

/// Which side of the connection this session plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Listens for an incoming QUIC connection.
    Server,
    /// Dials a remote peer.
    Client,
}

/// Audio codec selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodecKind {
    /// Opus (requires the `opus-codec` feature).
    Opus,
    /// Uncompressed 16-bit PCM passthrough (always available; for testing or LAN).
    Passthrough,
}

/// Which audio backend to use for capture/render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioBackendKind {
    /// Pick the best available: real hardware (`cpal-backend`) if compiled, else null.
    Auto,
    /// Real hardware devices (requires the `cpal-backend` feature).
    Cpal,
    /// Headless backend: captures silence, discards playout. For tests/servers.
    Null,
}

/// Audio pipeline parameters. Both peers must agree on these out of band.
#[derive(Clone, Copy, Debug)]
pub struct AudioParams {
    /// Sample rate in Hz. Opus supports 8000/12000/16000/24000/48000.
    pub sample_rate: u32,
    /// Channel count (1 = mono, 2 = stereo).
    pub channels: u16,
    /// Frame duration in milliseconds (Opus: 2.5/5/10/20/40/60).
    pub frame_ms: u32,
    /// Target encoder bitrate in bits/sec (Opus only).
    pub bitrate: u32,
    /// Codec to use.
    pub codec: CodecKind,
}

impl Default for AudioParams {
    fn default() -> Self {
        AudioParams {
            sample_rate: 48_000,
            channels: 1,
            frame_ms: 20,
            bitrate: 24_000,
            codec: CodecKind::Opus,
        }
    }
}

impl AudioParams {
    /// Number of samples per channel in one frame.
    pub fn frame_samples(&self) -> usize {
        (self.sample_rate as usize * self.frame_ms as usize) / 1000
    }

    /// Total i16 samples in one frame across all channels.
    pub fn frame_len(&self) -> usize {
        self.frame_samples() * self.channels as usize
    }

    pub(crate) fn validate(&self) -> crate::Result<()> {
        if self.channels == 0 || self.channels > 2 {
            return Err(crate::Error::Config("channels must be 1 or 2".into()));
        }
        if self.sample_rate == 0 {
            return Err(crate::Error::Config("sample_rate must be > 0".into()));
        }
        if self.frame_ms == 0 {
            return Err(crate::Error::Config("frame_ms must be > 0".into()));
        }
        Ok(())
    }
}

/// How the client verifies the server's certificate (and, with mutual TLS, vice versa).
#[derive(Clone, Debug)]
pub enum PeerVerification {
    /// Accept any certificate. INSECURE — development only.
    Insecure,
    /// Pin the exact end-entity certificate (DER bytes).
    PinnedCert(Vec<u8>),
    /// Pin the SHA-256 digest of the end-entity certificate DER. Compact form
    /// of [`PeerVerification::PinnedCert`] that is easy to carry out of band.
    PinnedSha256([u8; 32]),
}

/// TLS material and verification policy.
#[derive(Clone, Debug)]
pub struct SecurityConfig {
    /// Server/own certificate chain (DER). Required for `Role::Server`.
    pub cert_der: Vec<u8>,
    /// Own private key (PKCS#8 DER). Required for `Role::Server`.
    pub key_der: Vec<u8>,
    /// How this peer verifies the other end.
    pub verification: PeerVerification,
    /// ALPN protocol identifiers. Both peers must share at least one.
    pub alpn: Vec<Vec<u8>>,
}

impl SecurityConfig {
    /// Development default: a fresh self-signed cert and an insecure verifier.
    ///
    /// Use [`crate::security::generate_self_signed`] + [`PeerVerification::PinnedSpkiSha256`]
    /// for anything beyond a local prototype.
    pub fn self_signed_insecure(subject_alt_names: &[String]) -> crate::Result<Self> {
        let (cert_der, key_der) = crate::security::generate_self_signed(subject_alt_names)?;
        Ok(SecurityConfig {
            cert_der,
            key_der,
            verification: PeerVerification::Insecure,
            alpn: vec![crate::DEFAULT_ALPN.to_vec()],
        })
    }
}

/// Full session configuration.
#[derive(Clone, Debug)]
pub struct Config {
    pub role: Role,
    /// Local UDP socket to bind. Server listens here; client uses it as source.
    pub bind_addr: SocketAddr,
    /// Remote peer address. Required for `Role::Client`.
    pub peer_addr: Option<SocketAddr>,
    /// Expected server name (SNI / certificate name).
    pub server_name: String,
    pub audio: AudioParams,
    pub security: SecurityConfig,
    pub audio_backend: AudioBackendKind,
    /// Enable the audio data path (capture/encode/send + recv/decode/render).
    pub enable_audio: bool,
    /// Enable the reliable generic data channel (QUIC bidi stream).
    pub enable_data_channel: bool,
}

impl Config {
    pub(crate) fn validate(&self) -> crate::Result<()> {
        self.audio.validate()?;
        if self.role == Role::Client && self.peer_addr.is_none() {
            return Err(crate::Error::Config("client requires peer_addr".into()));
        }
        if self.role == Role::Server
            && (self.security.cert_der.is_empty() || self.security.key_der.is_empty())
        {
            return Err(crate::Error::Config(
                "server requires cert_der and key_der".into(),
            ));
        }
        if self.security.alpn.is_empty() {
            return Err(crate::Error::Config(
                "at least one ALPN id is required".into(),
            ));
        }
        Ok(())
    }
}
