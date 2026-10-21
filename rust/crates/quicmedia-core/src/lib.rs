//! # quicmedia-core
//!
//! A QUIC-based real-time audio engine. One [`Engine`] owns many QUIC
//! connections, each carrying several logical audio streams (encoded packets in
//! unreliable datagrams) plus a reliable control channel. The engine can act as
//! a capturing client, a decoding receiver, or an SFU that forwards encoded
//! packets between connections natively — all driven by a control-plane API
//! while media stays in native code and never crosses the managed boundary.
//!
//! ## Layout
//! * [`config`] — configuration types.
//! * [`security`] — TLS 1.3 material and the pluggable peer verifier.
//! * [`transport`] — quinn endpoint setup (client dial, server accept).
//! * [`framing`] — the compact RTP-like datagram header (with `stream_id`).
//! * [`control`] — the reliable stream-management protocol.
//! * [`jitter`] — the playout / jitter buffer.
//! * [`codec`] — Opus (feature `opus-codec`) and PCM passthrough.
//! * [`audio`] — native capture/render backends (null + miniaudio).
//! * [`datachannel`] — the reliable data channel tasks.
//! * [`conn`] — one QUIC connection as encoded-packet transport + control.
//! * [`engine`] — the multi-connection engine, native routing, and audio hub.

pub mod audio;
pub mod codec;
pub mod config;
pub mod conn;
pub mod control;
pub mod datachannel;
pub mod engine;
mod error;
pub mod framing;
pub mod jitter;
pub mod security;
pub mod transport;

pub use config::{
    AudioBackendKind, AudioParams, CodecKind, PeerVerification, Role, SecurityConfig,
};
pub use conn::ConnId;
pub use engine::{Engine, EngineEvents, StreamParams};
pub use error::{Error, Result};
pub use framing::StreamId;

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
