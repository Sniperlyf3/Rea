//! Error types for the QUIC media core.

use thiserror::Error;

/// Result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced by the QUIC media core.
///
/// Each variant maps to a stable integer code in the C ABI (see
/// `quicmedia-ffi`). Keep the discriminants in `code()` in sync with the
/// generated header.
#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid configuration: {0}")]
    Config(String),

    #[error("invalid argument: {0}")]
    InvalidArg(String),

    #[error("feature not compiled into this build: {0}")]
    Unsupported(String),

    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("tls/crypto error: {0}")]
    Tls(String),

    #[error("certificate pin mismatch")]
    PinMismatch,

    #[error("quic connect error: {0}")]
    Connect(String),

    #[error("quic connection error: {0}")]
    Connection(String),

    #[error("transport closed")]
    Closed,

    #[error("not connected")]
    NotConnected,

    #[error("codec error: {0}")]
    Codec(String),

    #[error("audio backend error: {0}")]
    Audio(String),
}

impl Error {
    /// Stable integer code for the C ABI. 0 is reserved for success.
    pub fn code(&self) -> i32 {
        match self {
            Error::Config(_) => 1,
            Error::InvalidArg(_) => 2,
            Error::Unsupported(_) => 3,
            Error::Io(_) => 4,
            Error::Tls(_) => 5,
            Error::PinMismatch => 6,
            Error::Connect(_) => 7,
            Error::Connection(_) => 8,
            Error::Closed => 9,
            Error::NotConnected => 10,
            Error::Codec(_) => 11,
            Error::Audio(_) => 12,
        }
    }
}
