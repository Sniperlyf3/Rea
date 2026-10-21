//! C ABI for the `quicmedia-core` engine. Consumed by the .NET (P/Invoke) and
//! C++ bindings. All functions are `extern "C"`, return an `int32_t` status
//! (0 = success, non-zero = [`quicmedia_core::Error::code`]) unless noted, and
//! never unwind. Media never crosses this ABI — only control, stats, and small
//! data-channel messages.
//!
//! See `rust/include/quicmedia.h` for the C declarations.

use std::ffi::{c_char, c_void, CStr, CString};
use std::net::SocketAddr;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::Arc;

use quicmedia_core::{
    security, AudioBackendKind, AudioParams, CodecKind, ConnId, Engine, EngineEvents,
    PeerVerification, SecurityConfig, StreamId, StreamParams,
};

const ERR_NULL_ARG: i32 = 100;
const ERR_BAD_ADDR: i32 = 102;
const ERR_PANIC: i32 = 103;

/// Opaque engine handle.
pub struct QmEngine {
    inner: Engine,
    _bridge: Arc<CbBridge>,
}

#[repr(C)]
pub struct QmAudioParams {
    pub sample_rate: u32,
    pub channels: u16,
    pub frame_ms: u32,
    pub bitrate: u32,
    pub codec: i32, // 0 = opus, 1 = passthrough
}

#[repr(C)]
pub struct QmSecurity {
    pub verification: i32, // 0 = insecure, 1 = pinned_cert, 2 = pinned_sha256
    pub cert_der: *const u8,
    pub cert_der_len: usize,
    pub key_der: *const u8,
    pub key_der_len: usize,
    pub pin: *const u8,
    pub pin_len: usize,
}

#[repr(C)]
#[derive(Default)]
pub struct QmConnStats {
    pub datagrams_sent: u64,
    pub datagrams_recv: u64,
    pub bytes_sent: u64,
    pub bytes_recv: u64,
    pub send_errors: u64,
}

#[repr(C)]
pub struct QmCallbacks {
    pub user: *mut c_void,
    pub on_connection: Option<extern "C" fn(user: *mut c_void, conn: u64, incoming: i32)>,
    pub on_connection_closed: Option<extern "C" fn(user: *mut c_void, conn: u64)>,
    pub on_stream_opened: Option<
        extern "C" fn(
            user: *mut c_void,
            conn: u64,
            stream: u16,
            codec: i32,
            sample_rate: u32,
            channels: u16,
            frame_ms: u32,
            bitrate: u32,
        ),
    >,
    pub on_stream_active:
        Option<extern "C" fn(user: *mut c_void, conn: u64, stream: u16, active: i32)>,
    pub on_stream_closed: Option<extern "C" fn(user: *mut c_void, conn: u64, stream: u16)>,
    pub on_data: Option<extern "C" fn(user: *mut c_void, conn: u64, data: *const u8, len: usize)>,
    pub on_error: Option<extern "C" fn(user: *mut c_void, code: i32, message: *const c_char)>,
}

struct CbBridge {
    cbs: QmCallbacks,
}

// The embedder owns the thread-safety of its callback target; callbacks may be
// invoked from the engine's runtime threads.
unsafe impl Send for CbBridge {}
unsafe impl Sync for CbBridge {}

impl EngineEvents for CbBridge {
    fn on_connection(&self, conn_id: ConnId, incoming: bool) {
        if let Some(cb) = self.cbs.on_connection {
            cb(self.cbs.user, conn_id, incoming as i32);
        }
    }
    fn on_connection_closed(&self, conn_id: ConnId) {
        if let Some(cb) = self.cbs.on_connection_closed {
            cb(self.cbs.user, conn_id);
        }
    }
    fn on_stream_opened(&self, conn_id: ConnId, stream_id: StreamId, p: StreamParams) {
        if let Some(cb) = self.cbs.on_stream_opened {
            cb(
                self.cbs.user,
                conn_id,
                stream_id,
                codec_to_i32(p.codec),
                p.sample_rate,
                p.channels,
                p.frame_ms,
                p.bitrate,
            );
        }
    }
    fn on_stream_active(&self, conn_id: ConnId, stream_id: StreamId, active: bool) {
        if let Some(cb) = self.cbs.on_stream_active {
            cb(self.cbs.user, conn_id, stream_id, active as i32);
        }
    }
    fn on_stream_closed(&self, conn_id: ConnId, stream_id: StreamId) {
        if let Some(cb) = self.cbs.on_stream_closed {
            cb(self.cbs.user, conn_id, stream_id);
        }
    }
    fn on_data(&self, conn_id: ConnId, data: &[u8]) {
        if let Some(cb) = self.cbs.on_data {
            cb(self.cbs.user, conn_id, data.as_ptr(), data.len());
        }
    }
    fn on_error(&self, code: i32, message: &str) {
        if let Some(cb) = self.cbs.on_error {
            let c = CString::new(message).unwrap_or_default();
            cb(self.cbs.user, code, c.as_ptr());
        }
    }
}

fn codec_from_i32(v: i32) -> CodecKind {
    match v {
        1 => CodecKind::Passthrough,
        _ => CodecKind::Opus,
    }
}

fn codec_to_i32(c: CodecKind) -> i32 {
    match c {
        CodecKind::Opus => 0,
        CodecKind::Passthrough => 1,
    }
}

unsafe fn opt_str<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        None
    } else {
        CStr::from_ptr(p).to_str().ok()
    }
}

unsafe fn slice_or_empty(ptr: *const u8, len: usize) -> Vec<u8> {
    if ptr.is_null() || len == 0 {
        Vec::new()
    } else {
        std::slice::from_raw_parts(ptr, len).to_vec()
    }
}

unsafe fn build_security(sec: &QmSecurity) -> std::result::Result<SecurityConfig, i32> {
    let cert_der = slice_or_empty(sec.cert_der, sec.cert_der_len);
    let key_der = slice_or_empty(sec.key_der, sec.key_der_len);
    let pin = slice_or_empty(sec.pin, sec.pin_len);
    let verification = match sec.verification {
        1 => PeerVerification::PinnedCert(pin),
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
    Ok(SecurityConfig {
        cert_der,
        key_der,
        verification,
        alpn: vec![quicmedia_core::DEFAULT_ALPN.to_vec()],
    })
}

// ---- Exported functions ----

#[no_mangle]
pub extern "C" fn qm_init() -> i32 {
    quicmedia_core::init_crypto();
    0
}

/// # Safety
/// Out-pointers must be valid and non-null.
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

/// # Safety
/// `cert`/`out` must be valid for the given lengths (`out` holds 32 bytes).
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

/// Create an engine. On success `*out` holds the handle; free with [`qm_engine_destroy`].
///
/// # Safety
/// All pointers must be valid and non-null.
#[no_mangle]
pub unsafe extern "C" fn qm_engine_create(
    params: *const QmAudioParams,
    backend: i32,
    cbs: *const QmCallbacks,
    out: *mut *mut QmEngine,
) -> i32 {
    if params.is_null() || cbs.is_null() || out.is_null() {
        return ERR_NULL_ARG;
    }
    *out = ptr::null_mut();
    let result = catch_unwind(AssertUnwindSafe(|| {
        let p = &*params;
        let audio = AudioParams {
            sample_rate: p.sample_rate,
            channels: p.channels,
            frame_ms: p.frame_ms,
            bitrate: p.bitrate,
            codec: codec_from_i32(p.codec),
        };
        let backend_kind = match backend {
            1 => AudioBackendKind::Miniaudio,
            2 => AudioBackendKind::Null,
            _ => AudioBackendKind::Auto,
        };
        let bridge = Arc::new(CbBridge {
            cbs: ptr::read(cbs),
        });
        let events: Arc<dyn EngineEvents> = bridge.clone();
        match Engine::new(audio, backend_kind, events) {
            Ok(engine) => {
                let boxed = Box::new(QmEngine {
                    inner: engine,
                    _bridge: bridge,
                });
                *out = Box::into_raw(boxed);
                0
            }
            Err(e) => e.code(),
        }
    }));
    result.unwrap_or(ERR_PANIC)
}

/// # Safety
/// `engine`, address strings, `sec`, and `out_conn` must be valid.
#[no_mangle]
pub unsafe extern "C" fn qm_engine_connect(
    engine: *mut QmEngine,
    bind: *const c_char,
    peer: *const c_char,
    server_name: *const c_char,
    sec: *const QmSecurity,
    enable_data: i32,
    out_conn: *mut u64,
) -> i32 {
    if engine.is_null() || sec.is_null() || out_conn.is_null() {
        return ERR_NULL_ARG;
    }
    let eng = &*engine;
    let bind_addr: SocketAddr = match opt_str(bind).and_then(|s| s.parse().ok()) {
        Some(a) => a,
        None => return ERR_BAD_ADDR,
    };
    let peer_addr: SocketAddr = match opt_str(peer).and_then(|s| s.parse().ok()) {
        Some(a) => a,
        None => return ERR_BAD_ADDR,
    };
    let name = opt_str(server_name).unwrap_or("localhost");
    let security = match build_security(&*sec) {
        Ok(s) => s,
        Err(c) => return c,
    };
    match eng
        .inner
        .connect(bind_addr, peer_addr, name, &security, enable_data != 0)
    {
        Ok(id) => {
            *out_conn = id;
            0
        }
        Err(e) => e.code(),
    }
}

/// # Safety
/// `engine`, `bind`, and `sec` must be valid.
#[no_mangle]
pub unsafe extern "C" fn qm_engine_listen(
    engine: *mut QmEngine,
    bind: *const c_char,
    sec: *const QmSecurity,
    enable_data: i32,
) -> i32 {
    if engine.is_null() || sec.is_null() {
        return ERR_NULL_ARG;
    }
    let eng = &*engine;
    let bind_addr: SocketAddr = match opt_str(bind).and_then(|s| s.parse().ok()) {
        Some(a) => a,
        None => return ERR_BAD_ADDR,
    };
    let security = match build_security(&*sec) {
        Ok(s) => s,
        Err(c) => return c,
    };
    match eng.inner.listen(bind_addr, &security, enable_data != 0) {
        Ok(()) => 0,
        Err(e) => e.code(),
    }
}

/// # Safety
/// `engine` and `out_stream` must be valid.
#[no_mangle]
pub unsafe extern "C" fn qm_engine_open_stream(
    engine: *mut QmEngine,
    conn: u64,
    codec: i32,
    bitrate: u32,
    active: i32,
    out_stream: *mut u16,
) -> i32 {
    if engine.is_null() || out_stream.is_null() {
        return ERR_NULL_ARG;
    }
    match (*engine)
        .inner
        .open_stream(conn, codec_from_i32(codec), bitrate, active != 0)
    {
        Ok(id) => {
            *out_stream = id;
            0
        }
        Err(e) => e.code(),
    }
}

/// # Safety
/// `engine` and `out_stream` must be valid.
#[no_mangle]
pub unsafe extern "C" fn qm_engine_add_capture_stream(
    engine: *mut QmEngine,
    conn: u64,
    codec: i32,
    bitrate: u32,
    active: i32,
    out_stream: *mut u16,
) -> i32 {
    if engine.is_null() || out_stream.is_null() {
        return ERR_NULL_ARG;
    }
    match (*engine)
        .inner
        .add_capture_stream(conn, codec_from_i32(codec), bitrate, active != 0)
    {
        Ok(id) => {
            *out_stream = id;
            0
        }
        Err(e) => e.code(),
    }
}

/// # Safety
/// `engine` must be valid.
#[no_mangle]
pub unsafe extern "C" fn qm_engine_bind_playback(
    engine: *mut QmEngine,
    conn: u64,
    stream: u16,
) -> i32 {
    if engine.is_null() {
        return ERR_NULL_ARG;
    }
    match (*engine).inner.bind_playback(conn, stream) {
        Ok(()) => 0,
        Err(e) => e.code(),
    }
}

/// # Safety
/// `engine` must be valid.
#[no_mangle]
pub unsafe extern "C" fn qm_engine_set_stream_active(
    engine: *mut QmEngine,
    conn: u64,
    stream: u16,
    active: i32,
) -> i32 {
    if engine.is_null() {
        return ERR_NULL_ARG;
    }
    match (*engine).inner.set_stream_active(conn, stream, active != 0) {
        Ok(()) => 0,
        Err(e) => e.code(),
    }
}

/// # Safety
/// `engine` must be valid.
#[no_mangle]
pub unsafe extern "C" fn qm_engine_close_stream(
    engine: *mut QmEngine,
    conn: u64,
    stream: u16,
) -> i32 {
    if engine.is_null() {
        return ERR_NULL_ARG;
    }
    match (*engine).inner.close_stream(conn, stream) {
        Ok(()) => 0,
        Err(e) => e.code(),
    }
}

/// # Safety
/// `engine` must be valid.
#[no_mangle]
pub unsafe extern "C" fn qm_engine_add_route(
    engine: *mut QmEngine,
    src_conn: u64,
    src_stream: u16,
    dst_conn: u64,
    dst_stream: u16,
) -> i32 {
    if engine.is_null() {
        return ERR_NULL_ARG;
    }
    (*engine)
        .inner
        .add_route(src_conn, src_stream, dst_conn, dst_stream);
    0
}

/// # Safety
/// `engine` must be valid.
#[no_mangle]
pub unsafe extern "C" fn qm_engine_remove_route(
    engine: *mut QmEngine,
    src_conn: u64,
    src_stream: u16,
    dst_conn: u64,
    dst_stream: u16,
) -> i32 {
    if engine.is_null() {
        return ERR_NULL_ARG;
    }
    (*engine)
        .inner
        .remove_route(src_conn, src_stream, dst_conn, dst_stream);
    0
}

/// # Safety
/// `engine` and `data` (for `len` bytes) must be valid.
#[no_mangle]
pub unsafe extern "C" fn qm_engine_send_data(
    engine: *mut QmEngine,
    conn: u64,
    data: *const u8,
    len: usize,
) -> i32 {
    if engine.is_null() || (data.is_null() && len != 0) {
        return ERR_NULL_ARG;
    }
    let bytes = if len == 0 {
        Vec::new()
    } else {
        std::slice::from_raw_parts(data, len).to_vec()
    };
    match (*engine).inner.send_data(conn, bytes) {
        Ok(()) => 0,
        Err(e) => e.code(),
    }
}

/// # Safety
/// `engine` must be valid.
#[no_mangle]
pub unsafe extern "C" fn qm_engine_close_connection(engine: *mut QmEngine, conn: u64) -> i32 {
    if engine.is_null() {
        return ERR_NULL_ARG;
    }
    (*engine).inner.close_connection(conn);
    0
}

/// # Safety
/// `engine` and `out` must be valid.
#[no_mangle]
pub unsafe extern "C" fn qm_engine_connection_stats(
    engine: *mut QmEngine,
    conn: u64,
    out: *mut QmConnStats,
) -> i32 {
    if engine.is_null() || out.is_null() {
        return ERR_NULL_ARG;
    }
    match (*engine).inner.connection_stats(conn) {
        Some((ds, dr, bs, br, se)) => {
            *out = QmConnStats {
                datagrams_sent: ds,
                datagrams_recv: dr,
                bytes_sent: bs,
                bytes_recv: br,
                send_errors: se,
            };
            0
        }
        None => 10, // not connected
    }
}

/// Destroy an engine handle. NULL-safe.
///
/// # Safety
/// `engine` must have come from [`qm_engine_create`] and not be used afterwards.
#[no_mangle]
pub unsafe extern "C" fn qm_engine_destroy(engine: *mut QmEngine) {
    if !engine.is_null() {
        drop(Box::from_raw(engine));
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

/// Static, human-readable description for an error code.
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
