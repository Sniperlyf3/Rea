//! Session orchestration: ties transport, codecs, jitter buffer, audio backend
//! and the data channel together behind a small control-plane API.
//!
//! Threading model:
//! * The session owns a multi-threaded `tokio` runtime.
//! * One async task captures → encodes → sends media datagrams.
//! * One async task receives datagrams → pushes into the jitter buffer.
//! * One async task runs the playout clock → decodes → renders.
//! * Two async tasks run the reliable data channel (reader/writer).
//! * Audio capture/render run on native OS audio threads inside the backend.
//!
//! No media ever crosses the FFI boundary: the managed caller only drives
//! lifecycle, reads stats, and exchanges small data-channel messages.

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;

use crate::audio::{self, Frame};
use crate::codec;
use crate::config::{Config, Role};
use crate::framing::{MediaHeader, HEADER_LEN, PT_AUDIO};
use crate::jitter::{JitterBuffer, Playout};
use crate::transport;
use crate::{Error, Result};

/// Lifecycle state of the connection, mirrored to the C ABI as a `u8`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ConnectionState {
    Connecting = 0,
    Connected = 1,
    Disconnected = 2,
    Failed = 3,
}

impl ConnectionState {
    fn from_u8(v: u8) -> ConnectionState {
        match v {
            1 => ConnectionState::Connected,
            2 => ConnectionState::Disconnected,
            3 => ConnectionState::Failed,
            _ => ConnectionState::Connecting,
        }
    }
}

/// Callbacks delivered to the embedder. Implementations must be cheap and must
/// not block (they run on the session's runtime threads).
pub trait Events: Send + Sync {
    fn on_state(&self, state: ConnectionState);
    fn on_data(&self, data: &[u8]);
    fn on_error(&self, code: i32, message: &str);
}

/// Atomics updated by the running tasks.
#[derive(Default)]
struct SharedStats {
    state: AtomicU8,
    datagrams_sent: AtomicU64,
    datagrams_recv: AtomicU64,
    bytes_sent: AtomicU64,
    bytes_recv: AtomicU64,
    send_errors: AtomicU64,
    rtt_us: AtomicU64,
}

/// A point-in-time snapshot of session statistics.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
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

const JITTER_TARGET_FRAMES: usize = 3;
const JITTER_MAX_FRAMES: usize = 20;
const DATA_CHANNEL_DEPTH: usize = 64;

/// A running media session. Dropping it tears everything down.
pub struct Session {
    runtime: Option<tokio::runtime::Runtime>,
    stop_tx: watch::Sender<bool>,
    data_tx: mpsc::Sender<Vec<u8>>,
    stats: Arc<SharedStats>,
    jitter: Arc<Mutex<JitterBuffer>>,
    _main: tokio::task::JoinHandle<()>,
}

impl Session {
    /// Start a session. Returns once the runtime and tasks are spawned;
    /// connection progress is reported asynchronously via `events`.
    pub fn start(config: Config, events: Arc<dyn Events>) -> Result<Session> {
        config.validate()?;

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("qm-worker")
            .build()
            .map_err(Error::Io)?;

        let (stop_tx, stop_rx) = watch::channel(false);
        let (data_tx, data_rx) = mpsc::channel::<Vec<u8>>(DATA_CHANNEL_DEPTH);
        let stats = Arc::new(SharedStats::default());
        let jitter = Arc::new(Mutex::new(JitterBuffer::new(
            JITTER_TARGET_FRAMES,
            JITTER_MAX_FRAMES,
        )));

        let main = runtime.spawn(run_session(
            config,
            events,
            data_rx,
            stop_rx,
            stats.clone(),
            jitter.clone(),
        ));

        Ok(Session {
            runtime: Some(runtime),
            stop_tx,
            data_tx,
            stats,
            jitter,
            _main: main,
        })
    }

    /// Queue a message on the reliable data channel. Non-blocking.
    pub fn send_data(&self, bytes: Vec<u8>) -> Result<()> {
        use mpsc::error::TrySendError;
        self.data_tx.try_send(bytes).map_err(|e| match e {
            TrySendError::Full(_) => Error::InvalidArg("data channel send queue full".into()),
            TrySendError::Closed(_) => Error::NotConnected,
        })
    }

    /// Current statistics snapshot.
    pub fn stats(&self) -> Stats {
        let j = self.jitter.lock().unwrap().stats();
        Stats {
            state: self.stats.state.load(Ordering::Relaxed),
            datagrams_sent: self.stats.datagrams_sent.load(Ordering::Relaxed),
            datagrams_recv: self.stats.datagrams_recv.load(Ordering::Relaxed),
            bytes_sent: self.stats.bytes_sent.load(Ordering::Relaxed),
            bytes_recv: self.stats.bytes_recv.load(Ordering::Relaxed),
            send_errors: self.stats.send_errors.load(Ordering::Relaxed),
            rtt_us: self.stats.rtt_us.load(Ordering::Relaxed),
            jitter_depth: j.depth,
            jitter_played: j.played,
            jitter_concealed: j.concealed,
            jitter_late_drops: j.late_drops,
        }
    }

    /// Current connection state.
    pub fn state(&self) -> ConnectionState {
        ConnectionState::from_u8(self.stats.state.load(Ordering::Relaxed))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.stop_tx.send(true);
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_timeout(Duration::from_millis(500));
        }
    }
}

async fn run_session(
    config: Config,
    events: Arc<dyn Events>,
    data_rx: mpsc::Receiver<Vec<u8>>,
    mut stop_rx: watch::Receiver<bool>,
    stats: Arc<SharedStats>,
    jitter: Arc<Mutex<JitterBuffer>>,
) {
    let set_state = |s: ConnectionState| {
        stats.state.store(s as u8, Ordering::Relaxed);
        events.on_state(s);
    };

    set_state(ConnectionState::Connecting);

    let (endpoint, conn) = match transport::establish(&config).await {
        Ok(v) => v,
        Err(e) => {
            events.on_error(e.code(), &e.to_string());
            set_state(ConnectionState::Failed);
            return;
        }
    };
    set_state(ConnectionState::Connected);

    let mut tasks: JoinSet<()> = JoinSet::new();
    // Keeps audio hardware alive for the session's lifetime.
    let mut _audio_device = None;

    // ---- Audio pipeline ----
    if config.enable_audio {
        match start_audio(&config, &conn, &jitter, &stats, &mut tasks) {
            Ok(device) => _audio_device = device,
            Err(e) => events.on_error(e.code(), &e.to_string()),
        }
    }

    // ---- Reliable data channel ----
    if config.enable_data_channel {
        let stream = match config.role {
            Role::Client => conn
                .open_bi()
                .await
                .map_err(|e| Error::Connection(e.to_string())),
            Role::Server => conn
                .accept_bi()
                .await
                .map_err(|e| Error::Connection(e.to_string())),
        };
        match stream {
            Ok((send, recv)) => {
                tasks.spawn(crate::datachannel::writer_task(send, data_rx));
                tasks.spawn(crate::datachannel::reader_task(recv, events.clone()));
            }
            Err(e) => events.on_error(e.code(), &e.to_string()),
        }
    }

    // ---- RTT sampler ----
    {
        let conn = conn.clone();
        let stats = stats.clone();
        tasks.spawn(async move {
            let mut iv = tokio::time::interval(Duration::from_secs(1));
            loop {
                iv.tick().await;
                stats
                    .rtt_us
                    .store(conn.rtt().as_micros() as u64, Ordering::Relaxed);
            }
        });
    }

    // ---- Wait for shutdown or disconnect ----
    tokio::select! {
        _ = stop_rx.changed() => {
            tracing::info!("session stop requested");
        }
        reason = conn.closed() => {
            tracing::info!("quic connection closed: {reason}");
            set_state(ConnectionState::Disconnected);
        }
    }

    tasks.abort_all();
    conn.close(0u32.into(), b"bye");
    endpoint.close(0u32.into(), b"bye");
    // _audio_device drops here, stopping capture/render.
}

/// Spawn the capture/send, receive, and playout tasks. Returns the audio device
/// keepalive handle.
fn start_audio(
    config: &Config,
    conn: &quinn::Connection,
    jitter: &Arc<Mutex<JitterBuffer>>,
    stats: &Arc<SharedStats>,
    tasks: &mut JoinSet<()>,
) -> Result<Option<Box<dyn audio::AudioDevice>>> {
    let params = config.audio;
    let backend = audio::make_backend(config.audio_backend)?;
    let streams = backend.start(&params)?;
    let (mut captured, to_play, device) = streams.into_parts();

    // Capture → encode → datagram send.
    {
        let conn = conn.clone();
        let stats = stats.clone();
        let mut enc = codec::make_codec(&params)?;
        let frame_samples = params.frame_samples() as u32;
        let max_dgram = conn.max_datagram_size();
        tasks.spawn(async move {
            let mut seq: u16 = 0;
            let mut ts: u32 = 0;
            let mut payload = Vec::with_capacity(4096);
            while let Some(frame) = captured.recv().await {
                if enc.encode(&frame, &mut payload).is_err() {
                    stats.send_errors.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                let total = HEADER_LEN + payload.len();
                if let Some(max) = max_dgram {
                    if total > max {
                        stats.send_errors.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                }
                let mut dgram = vec![0u8; HEADER_LEN];
                MediaHeader {
                    marker: false,
                    payload_type: PT_AUDIO,
                    sequence: seq,
                    timestamp: ts,
                }
                .write(&mut dgram);
                dgram.extend_from_slice(&payload);
                let len = dgram.len() as u64;
                match conn.send_datagram(Bytes::from(dgram)) {
                    Ok(()) => {
                        stats.datagrams_sent.fetch_add(1, Ordering::Relaxed);
                        stats.bytes_sent.fetch_add(len, Ordering::Relaxed);
                    }
                    Err(quinn::SendDatagramError::ConnectionLost(_)) => break,
                    Err(_) => {
                        stats.send_errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
                seq = seq.wrapping_add(1);
                ts = ts.wrapping_add(frame_samples);
            }
        });
    }

    // Receive datagrams → jitter buffer.
    {
        let conn = conn.clone();
        let stats = stats.clone();
        let jitter = jitter.clone();
        tasks.spawn(async move {
            while let Ok(bytes) = conn.read_datagram().await {
                stats.datagrams_recv.fetch_add(1, Ordering::Relaxed);
                stats
                    .bytes_recv
                    .fetch_add(bytes.len() as u64, Ordering::Relaxed);
                if let Ok((hdr, body)) = MediaHeader::parse(&bytes) {
                    if hdr.payload_type == PT_AUDIO {
                        if let Ok(mut j) = jitter.lock() {
                            j.push(hdr.sequence, body.to_vec());
                        }
                    }
                }
            }
        });
    }

    // Playout clock → decode → render.
    {
        let jitter = jitter.clone();
        let mut dec = codec::make_codec(&params)?;
        let frame_ms = params.frame_ms as u64;
        tasks.spawn(async move {
            let mut iv = tokio::time::interval(Duration::from_millis(frame_ms));
            iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut pcm: Vec<i16> = Vec::new();
            loop {
                iv.tick().await;
                let outcome = {
                    match jitter.lock() {
                        Ok(mut j) => j.pop(),
                        Err(_) => break,
                    }
                };
                let decoded = match outcome {
                    Playout::Frame(p) => dec.decode(Some(&p), &mut pcm),
                    Playout::Conceal => dec.decode(None, &mut pcm),
                    Playout::Starved => continue,
                };
                if decoded.is_ok() {
                    let frame: Frame = pcm.clone().into_boxed_slice();
                    if to_play.send(frame).await.is_err() {
                        break;
                    }
                }
            }
        });
    }

    Ok(Some(device))
}
