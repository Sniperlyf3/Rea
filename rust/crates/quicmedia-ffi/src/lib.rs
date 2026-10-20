//! C ABI for `quicmedia-core`.
//!
//! All functions are `extern "C"`, return an `int32_t` status (0 = success,
//! non-zero = [`quicmedia_core::Error::code`]) unless otherwise noted, and never
//! unwind across the boundary. Media never crosses this ABI — only lifecycle,
//! configuration, small data-channel messages, and stats.
//!
//! See `rust/include/quicmedia.h` for the C declarations.

use std::ffi::{c_char, CStr, CString};
use std::net::SocketAddr;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::Arc;

use quicmedia_core::{
    security, AudioBackendKind, AudioParams, CodecKind, Config, ConnectionState, Events,
    PeerVerification, Role, SecurityConfig, Session,
};

/// Opaque session handle.
pub struct QmSession {
    inner: Session,
}

// ---- C-visible structs ----

#[repr(C)]
pub struct QmConfig {
    pub role: i32, // 0 = server, 1 = client
    pub bind_addr: *const c_char,
    pub peer_addr: *const c_char, // nullable
    pub server_name: *const c_char,
    pub sample_rate: u32,
    pub channels: u16,
    pub frame_ms: u32,
    pub bitrate: u32,
    pub codec: i32,               // 0 = opus, 1 = passthrough
    pub audio_backend: i32,       // 0 = auto, 1 = cpal, 2 = null
    pub enable_audio: i32,        // bool
    pub enable_data_channel: i32, // bool
    pub verification: i32,        // 0 = insecure, 1 = pinned_cert, 2 = pinned_sha256
    pub cert_der: *const u8,
    pub cert_der_len: usize,
    pub key_der: *const u8,
    pub key_der_len: usize,
    pub pin: *const u8, // pinned cert DER or 32-byte sha256, per `verification`
    pub pin_len: usize,
}

#[repr(C)]
pub struct QmCallbacks {
    pub user: *mut std::ffi::c_void,
    pub on_state: Option<extern "C" fn(user: *mut std::ffi::c_void, state: i32)>,
    pub on_data: Option<extern "C" fn(user: *mut std::ffi::c_void, data: *const u8, len: usize)>,
    pub on_error:
        Option<extern "C" fn(user: *mut std::ffi::c_void, code: i32, message: *const c_char)>,
}

#[repr(C)]
#[derive(Default)]
pub struct QmStats {
    pub state: u8,
    pub datagrams_sent: u64,
    pub datagrams_recv: u64,
    pub bytes_sent: u64,
    pub bytes_recv: u64,
    pub send_errors: u64,
    pub rtt_us: u64,
    pub jitter_depth: u32,
    pub jitter_played: u64,
    pub jitter_concealed: u64,
    pub jitter_late_drops: u64,
}

// ---- Callback bridge ----

struct CbBridge {
    cbs: QmCallbacks,
}

// The embedder is responsible for the thread-safety of its callback target;
// callbacks may be invoked from the session's runtime threads.
unsafe impl Send for CbBridge {}
unsafe impl Sync for CbBridge {}

impl Events for CbBridge {
    fn on_state(&self, state: ConnectionState) {
        if let Some(cb) = self.cbs.on_state {
            cb(self.cbs.user, state as i32);
        }
    }
    fn on_data(&self, data: &[u8]) {
        if let Some(cb) = self.cbs.on_data {
            cb(self.cbs.user, data.as_ptr(), data.len());
        }
    }
    fn on_error(&self, code: i32, message: &str) {
        if let Some(cb) = self.cbs.on_error {
            let c = CString::new(message).unwrap_or_default();
            cb(self.cbs.user, code, c.as_ptr());
        }
    }
}

// ---- Helpers ----

const ERR_NULL_ARG: i32 = 100;
// 101 (invalid utf-8) is described by qm_error_message; opt_str currently maps
// bad utf-8 to a null/absent argument rather than surfacing it separately.
const ERR_BAD_ADDR: i32 = 102;
const ERR_PANIC: i32 = 103;

unsafe fn opt_str<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        None
    } else {
        CStr::from_ptr(p).to_str().ok()
    }
}

unsafe fn build_config(cfg: &QmConfig) -> Result<Config, i32> {
    let bind = opt_str(cfg.bind_addr).ok_or(ERR_NULL_ARG)?;
    let bind_addr: SocketAddr = bind.parse().map_err(|_| ERR_BAD_ADDR)?;
    let peer_addr = match opt_str(cfg.peer_addr) {
        Some(s) => Some(s.parse::<SocketAddr>().map_err(|_| ERR_BAD_ADDR)?),
        None => None,
    };
    let server_name = opt_str(cfg.server_name).unwrap_or("localhost").to_string();

    let role = match cfg.role {
        0 => Role::Server,
        1 => Role::Client,
        _ => return Err(ERR_NULL_ARG),
    };
    let codec = match cfg.codec {
        0 => CodecKind::Opus,
        1 => CodecKind::Passthrough,
        _ => CodecKind::Opus,
    };
    let audio_backend = match cfg.audio_backend {
        1 => AudioBackendKind::Cpal,
        2 => AudioBackendKind::Null,
        _ => AudioBackendKind::Auto,
    };

    let cert_der = slice_or_empty(cfg.cert_der, cfg.cert_der_len);
    let key_der = slice_or_empty(cfg.key_der, cfg.key_der_len);
    let pin = slice_or_empty(cfg.pin, cfg.pin_len);

    let verification = match cfg.verification {
        1 => PeerVerification::PinnedCert(pin.clone()),
        2 => {
            if pin.len() != 32 {
                return Err(ERR_BAD_ADDR);
            }
            let mut h = [0u8; 32];
            h.copy_from_slice(&pin);
            PeerVerification::PinnedSha256(h)
        }
        _ => PeerVerification::Insecure,
    };

    Ok(Config {
        role,
        bind_addr,
        peer_addr,
        server_name,
        audio: AudioParams {
            sample_rate: cfg.sample_rate,
            channels: cfg.channels,
            frame_ms: cfg.frame_ms,
            bitrate: cfg.bitrate,
            codec,
        },
        security: SecurityConfig {
            cert_der,
            key_der,
            verification,
            alpn: vec![quicmedia_core::DEFAULT_ALPN.to_vec()],
        },
        audio_backend,
        enable_audio: cfg.enable_audio != 0,
        enable_data_channel: cfg.enable_data_channel != 0,
    })
}

unsafe fn slice_or_empty(ptr: *const u8, len: usize) -> Vec<u8> {
    if ptr.is_null() || len == 0 {
        Vec::new()
    } else {
        std::slice::from_raw_parts(ptr, len).to_vec()
    }
}

// ---- Exported functions ----

/// Initialize process-wide crypto defaults. Safe to call multiple times.
#[no_mangle]
pub extern "C" fn qm_init() -> i32 {
    quicmedia_core::init_crypto();
    0
}

/// Generate a self-signed certificate. `sans_csv` is a comma-separated list of
/// subject alternative names (may be NULL → "localhost"). On success, `*cert_out`
/// / `*key_out` point to heap buffers the caller must release with
/// [`qm_buffer_free`].
///
/// # Safety
/// All out-pointers must be valid and non-null.
#[no_mangle]
pub unsafe extern "C" fn qm_generate_self_signed(
    sans_csv: *const c_char,
    cert_out: *mut *mut u8,
    cert_len: *mut usize,
    key_out: *mut *mut u8,
    key_len: *mut usize,
) -> i32 {
    if cert_out.is_null() || cert_len.is_null() || key_out.is_null() || key_len.is_null() {
        return ERR_NULL_ARG;
    }
    let sans: Vec<String> = match opt_str(sans_csv) {
        Some(s) => s
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect(),
        None => Vec::new(),
    };
    match security::generate_self_signed(&sans) {
        Ok((cert, key)) => {
            write_buffer(cert, cert_out, cert_len);
            write_buffer(key, key_out, key_len);
            0
        }
        Err(e) => e.code(),
    }
}

/// Compute the SHA-256 of a certificate DER into the 32-byte `out`.
///
/// # Safety
/// `cert`/`out` must be valid for the given lengths.
#[no_mangle]
pub unsafe extern "C" fn qm_cert_sha256(cert: *const u8, cert_len: usize, out: *mut u8) -> i32 {
    if cert.is_null() || out.is_null() {
        return ERR_NULL_ARG;
    }
    let der = std::slice::from_raw_parts(cert, cert_len);
    let hash = security::cert_sha256(der);
    ptr::copy_nonoverlapping(hash.as_ptr(), out, 32);
    0
}

/// Start a session. On success `*out` holds the session handle; release it with
/// [`qm_session_stop`].
///
/// # Safety
/// `cfg`, `cbs`, and `out` must be valid non-null pointers.
#[no_mangle]
pub unsafe extern "C" fn qm_session_start(
    cfg: *const QmConfig,
    cbs: *const QmCallbacks,
    out: *mut *mut QmSession,
) -> i32 {
    if cfg.is_null() || cbs.is_null() || out.is_null() {
        return ERR_NULL_ARG;
    }
    *out = ptr::null_mut();

    let result = catch_unwind(AssertUnwindSafe(|| {
        let config = match build_config(&*cfg) {
            Ok(c) => c,
            Err(code) => return code,
        };
        let callbacks = ptr::read(cbs);
        let bridge: Arc<dyn Events> = Arc::new(CbBridge { cbs: callbacks });
        match Session::start(config, bridge) {
            Ok(session) => {
                let boxed = Box::new(QmSession { inner: session });
                *out = Box::into_raw(boxed);
                0
            }
            Err(e) => e.code(),
        }
    }));

    result.unwrap_or(ERR_PANIC)
}

/// Queue a message on the reliable data channel.
///
/// # Safety
/// `session` must be a live handle and `data` valid for `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn qm_session_send_data(
    session: *mut QmSession,
    data: *const u8,
    len: usize,
) -> i32 {
    if session.is_null() || (data.is_null() && len != 0) {
        return ERR_NULL_ARG;
    }
    let s = &*session;
    let bytes = if len == 0 {
        Vec::new()
    } else {
        std::slice::from_raw_parts(data, len).to_vec()
    };
    match s.inner.send_data(bytes) {
        Ok(()) => 0,
        Err(e) => e.code(),
    }
}

/// Fill `out` with the current statistics snapshot.
///
/// # Safety
/// `session` and `out` must be valid non-null pointers.
#[no_mangle]
pub unsafe extern "C" fn qm_session_get_stats(session: *mut QmSession, out: *mut QmStats) -> i32 {
    if session.is_null() || out.is_null() {
        return ERR_NULL_ARG;
    }
    let s = &*session;
    let st = s.inner.stats();
    *out = QmStats {
        state: st.state,
        datagrams_sent: st.datagrams_sent,
        datagrams_recv: st.datagrams_recv,
        bytes_sent: st.bytes_sent,
        bytes_recv: st.bytes_recv,
        send_errors: st.send_errors,
        rtt_us: st.rtt_us,
        jitter_depth: st.jitter_depth,
        jitter_played: st.jitter_played,
        jitter_concealed: st.jitter_concealed,
        jitter_late_drops: st.jitter_late_drops,
    };
    0
}

/// Return the current connection state (see [`ConnectionState`]), or -1 on NULL.
///
/// # Safety
/// `session` must be a live handle or NULL.
#[no_mangle]
pub unsafe extern "C" fn qm_session_state(session: *mut QmSession) -> i32 {
    if session.is_null() {
        return -1;
    }
    (*session).inner.state() as i32
}

/// Stop and free a session handle. Safe to call with NULL.
///
/// # Safety
/// `session` must have come from [`qm_session_start`] and not be used afterwards.
#[no_mangle]
pub unsafe extern "C" fn qm_session_stop(session: *mut QmSession) {
    if !session.is_null() {
        drop(Box::from_raw(session));
    }
}

/// Free a buffer returned by this library.
///
/// # Safety
/// `ptr`/`len` must be exactly as returned by a `qm_*` function.
#[no_mangle]
pub unsafe extern "C" fn qm_buffer_free(ptr: *mut u8, len: usize) {
    if !ptr.is_null() && len != 0 {
        drop(Vec::from_raw_parts(ptr, len, len));
    }
}

/// Return a static, human-readable description for an error code.
#[no_mangle]
pub extern "C" fn qm_error_message(code: i32) -> *const c_char {
    let s: &CStr = match code {
        0 => c"success",
        1 => c"invalid configuration",
        2 => c"invalid argument",
        3 => c"feature not compiled into this build",
        4 => c"i/o error",
        5 => c"tls/crypto error",
        6 => c"certificate pin mismatch",
        7 => c"quic connect error",
        8 => c"quic connection error",
        9 => c"transport closed",
        10 => c"not connected",
        11 => c"codec error",
        12 => c"audio backend error",
        100 => c"null argument",
        101 => c"invalid utf-8 string",
        102 => c"invalid socket address or pin length",
        103 => c"internal panic",
        _ => c"unknown error",
    };
    s.as_ptr()
}

unsafe fn write_buffer(data: Vec<u8>, out_ptr: *mut *mut u8, out_len: *mut usize) {
    let mut boxed = data.into_boxed_slice();
    *out_len = boxed.len();
    *out_ptr = boxed.as_mut_ptr();
    std::mem::forget(boxed);
}
