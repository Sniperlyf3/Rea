//! # quicmedia-core
//!
//! A QUIC-based real-time audio + data transport. The full media path lives in
//! native code: capture → Opus encode → QUIC DATAGRAM → jitter buffer → decode
//! → render. A reliable generic data channel rides the same QUIC connection on
//! a bidirectional stream. The crate is consumed from C# through the thin C ABI
//! in `quicmedia-ffi`; media bytes never cross the managed boundary.
//!
//! ## Layout
//! * [`config`] — session configuration.
//! * [`security`] — TLS 1.3 material and the pluggable peer verifier.
//! * [`transport`] — quinn endpoint setup and connection establishment.
//! * [`framing`] — the compact RTP-like datagram header.
//! * [`jitter`] — the playout / jitter buffer.
//! * [`codec`] — Opus (feature `opus-codec`) and PCM passthrough.
//! * [`audio`] — native capture/render backends (null + miniaudio).
//! * [`datachannel`] — the reliable data channel tasks.
//! * [`session`] — the orchestrator and public [`Session`] handle.

pub mod audio;
pub mod codec;
pub mod config;
pub mod datachannel;
mod error;
pub mod framing;
pub mod jitter;
pub mod security;
pub mod session;
pub mod transport;

pub use config::{
    AudioBackendKind, AudioParams, CodecKind, Config, PeerVerification, Role, SecurityConfig,
};
pub use error::{Error, Result};
pub use session::{ConnectionState, Events, Session, Stats};

/// Default ALPN protocol identifier for this transport.
pub const DEFAULT_ALPN: &[u8] = b"rea-qm/1";

/// Ensure the process-wide default crypto provider is installed.
///
/// `quicmedia-core` builds its `rustls` configs with an explicit provider, so
/// this is not strictly required, but calling it once at startup avoids
/// surprises if other code relies on the process default.
pub fn init_crypto() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}
