//! The media engine: owns many QUIC connections, a native routing table that
//! fans encoded packets from one connection to many others (SFU), and an audio
//! hub (one device + per-stream codecs) for the capture/playback roles.
//!
//! One library instance can therefore act as a capturing client, a decoding
//! receiver, an SFU that only relays encoded packets, or any mix — the control
//! plane (open/activate/route/bind) is driven by the embedder, while the data
//! plane (forwarding, encode/decode, device I/O) stays entirely in native code.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{mpsc, Notify};
use tokio::task::JoinHandle;

use crate::audio::{self, AudioDevice, Frame};
use crate::codec::{self, Codec};
use crate::config::{AudioBackendKind, AudioParams, CodecKind, SecurityConfig};
use crate::conn::{ConnEvent, ConnId, InboundMedia, PeerConn};
use crate::framing::StreamId;
use crate::jitter::{JitterBuffer, Playout};
use crate::transport;
use crate::{Error, Result};

/// Parameters describing a stream, surfaced to the embedder.
#[derive(Clone, Copy, Debug)]
pub struct StreamParams {
    pub codec: CodecKind,
    pub sample_rate: u32,
    pub channels: u16,
    pub frame_ms: u32,
    pub bitrate: u32,
}

/// Engine callbacks delivered to the embedder. Must be cheap and non-blocking.
pub trait EngineEvents: Send + Sync {
    fn on_connection(&self, conn_id: ConnId, incoming: bool);
    fn on_connection_closed(&self, conn_id: ConnId);
    fn on_stream_opened(&self, conn_id: ConnId, stream_id: StreamId, params: StreamParams);
    fn on_stream_active(&self, conn_id: ConnId, stream_id: StreamId, active: bool);
    fn on_stream_closed(&self, conn_id: ConnId, stream_id: StreamId);
    fn on_data(&self, conn_id: ConnId, data: &[u8]);
    fn on_error(&self, code: i32, message: &str);
}

const JITTER_TARGET_FRAMES: usize = 2;
const JITTER_MAX_FRAMES: usize = 16;

/// The running audio device plus its capture receiver and playout sender.
type DeviceBundle = (
    Box<dyn AudioDevice>,
    mpsc::Receiver<Frame>,
    mpsc::Sender<Frame>,
);

/// A (connection, stream) pair — the key for routes and audio bindings.
type StreamKey = (ConnId, StreamId);

/// Native routing table: one source stream fans out to many destination streams.
type RouteTable = HashMap<StreamKey, Vec<StreamKey>>;

struct CaptureBinding {
    conn_id: ConnId,
    stream_id: StreamId,
    encoder: Box<dyn Codec>,
    timestamp: u32,
}

struct PlaybackBinding {
    decoder: Box<dyn Codec>,
    jitter: JitterBuffer,
}

/// Owns the single audio device and the per-stream codecs/jitter.
#[derive(Default)]
struct AudioHub {
    captures: HashMap<(ConnId, StreamId), CaptureBinding>,
    playbacks: HashMap<(ConnId, StreamId), PlaybackBinding>,
}

struct EngineInner {
    conns: Mutex<HashMap<ConnId, Arc<PeerConn>>>,
    routes: Mutex<RouteTable>,
    audio: Mutex<AudioHub>,
    endpoints: Mutex<Vec<quinn::Endpoint>>,
    next_conn_id: AtomicU64,
    audio_params: AudioParams,
    backend_kind: AudioBackendKind,
    inbound_tx: mpsc::Sender<InboundMedia>,
    event_tx: mpsc::Sender<ConnEvent>,
    wake: Notify,
    app: Arc<dyn EngineEvents>,
}

/// A running media engine. Dropping it tears everything down.
pub struct Engine {
    runtime: Option<tokio::runtime::Runtime>,
    inner: Arc<EngineInner>,
    tasks: Vec<JoinHandle<()>>,
}

impl Engine {
    /// Create an engine. `audio_params` are the device parameters (sample rate,
    /// channels, frame/ptime) shared by all audio streams; per-stream codec and
    /// bitrate are chosen when opening a stream.
    pub fn new(
        audio_params: AudioParams,
        backend_kind: AudioBackendKind,
        app: Arc<dyn EngineEvents>,
    ) -> Result<Engine> {
        audio_params.validate()?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("qm-engine")
            .build()
            .map_err(Error::Io)?;

        let (inbound_tx, inbound_rx) = mpsc::channel::<InboundMedia>(256);
        let (event_tx, event_rx) = mpsc::channel::<ConnEvent>(256);

        let inner = Arc::new(EngineInner {
            conns: Mutex::new(HashMap::new()),
            routes: Mutex::new(HashMap::new()),
            audio: Mutex::new(AudioHub::default()),
            endpoints: Mutex::new(Vec::new()),
            next_conn_id: AtomicU64::new(1),
            audio_params,
            backend_kind,
            inbound_tx,
            event_tx,
            wake: Notify::new(),
            app,
        });

        let tasks = vec![
            runtime.spawn(dispatch_task(inner.clone(), inbound_rx)),
            runtime.spawn(event_task(inner.clone(), event_rx)),
            runtime.spawn(audio_pump(inner.clone())),
        ];

        Ok(Engine {
            runtime: Some(runtime),
            inner,
            tasks,
        })
    }

    /// Dial a peer and return the new connection id.
    pub fn connect(
        &self,
        bind: SocketAddr,
        peer: SocketAddr,
        server_name: &str,
        sec: &SecurityConfig,
        enable_data: bool,
    ) -> Result<ConnId> {
        let rt = self.runtime.as_ref().unwrap();
        let inner = self.inner.clone();
        let sec = sec.clone();
        let server_name = server_name.to_string();
        // Endpoint creation and PeerConn task spawning must run inside the runtime.
        rt.block_on(async move {
            let endpoint = transport::client_endpoint(bind, &sec)?;
            let conn = transport::dial(&endpoint, peer, &server_name).await?;
            inner.endpoints.lock().unwrap().push(endpoint);
            Ok(inner.register_conn(conn, true, enable_data))
        })
    }

    /// Start accepting incoming connections on `bind`.
    pub fn listen(&self, bind: SocketAddr, sec: &SecurityConfig, enable_data: bool) -> Result<()> {
        let rt = self.runtime.as_ref().unwrap();
        let endpoint = {
            let _guard = rt.enter();
            transport::server_endpoint(bind, sec)?
        };
        self.inner.endpoints.lock().unwrap().push(endpoint.clone());
        let inner = self.inner.clone();
        rt.spawn(async move {
            while let Some(incoming) = endpoint.accept().await {
                let inner = inner.clone();
                tokio::spawn(async move {
                    match incoming.accept() {
                        Ok(connecting) => match connecting.await {
                            Ok(conn) => {
                                inner.register_conn(conn, false, enable_data);
                            }
                            Err(e) => inner.app.on_error(8, &e.to_string()),
                        },
                        Err(e) => inner.app.on_error(8, &e.to_string()),
                    }
                });
            }
        });
        Ok(())
    }

    /// Open an outbound stream on a connection. Returns the allocated stream id.
    pub fn open_stream(
        &self,
        conn_id: ConnId,
        codec: CodecKind,
        bitrate: u32,
        active: bool,
    ) -> Result<StreamId> {
        let peer = self.inner.conn(conn_id)?;
        let p = self.inner.audio_params;
        let pt = codec::payload_type(codec);
        Ok(peer.open_stream(pt, p.sample_rate, p.channels, p.frame_ms, bitrate, active))
    }

    /// Set our desired active state for a stream and signal the peer.
    pub fn set_stream_active(
        &self,
        conn_id: ConnId,
        stream_id: StreamId,
        active: bool,
    ) -> Result<()> {
        let peer = self.inner.conn(conn_id)?;
        if peer.set_active(stream_id, active) {
            self.inner.wake.notify_one();
            Ok(())
        } else {
            Err(Error::InvalidArg("unknown stream".into()))
        }
    }

    /// Close a stream.
    pub fn close_stream(&self, conn_id: ConnId, stream_id: StreamId) -> Result<()> {
        let peer = self.inner.conn(conn_id)?;
        peer.close_stream(stream_id);
        self.inner.remove_bindings_for(conn_id, stream_id);
        Ok(())
    }

    /// Route an inbound stream's encoded packets to an outbound stream on another
    /// connection (native fan-out; add several routes for 1→many).
    pub fn add_route(
        &self,
        src_conn: ConnId,
        src_stream: StreamId,
        dst_conn: ConnId,
        dst_stream: StreamId,
    ) {
        self.inner
            .routes
            .lock()
            .unwrap()
            .entry((src_conn, src_stream))
            .or_default()
            .push((dst_conn, dst_stream));
    }

    /// Remove a previously added route.
    pub fn remove_route(
        &self,
        src_conn: ConnId,
        src_stream: StreamId,
        dst_conn: ConnId,
        dst_stream: StreamId,
    ) {
        if let Some(v) = self
            .inner
            .routes
            .lock()
            .unwrap()
            .get_mut(&(src_conn, src_stream))
        {
            v.retain(|d| *d != (dst_conn, dst_stream));
        }
    }

    /// Open an outbound stream fed by the local microphone (capture → encode).
    pub fn add_capture_stream(
        &self,
        conn_id: ConnId,
        codec: CodecKind,
        bitrate: u32,
        active: bool,
    ) -> Result<StreamId> {
        let stream_id = self.open_stream(conn_id, codec, bitrate, active)?;
        let params = self.inner.binding_params(codec, bitrate);
        let encoder = codec::make_codec(&params)?;
        self.inner.audio.lock().unwrap().captures.insert(
            (conn_id, stream_id),
            CaptureBinding {
                conn_id,
                stream_id,
                encoder,
                timestamp: 0,
            },
        );
        self.inner.wake.notify_one();
        Ok(stream_id)
    }

    /// Decode and play an inbound stream (created by the peer) on the speaker.
    pub fn bind_playback(&self, conn_id: ConnId, stream_id: StreamId) -> Result<()> {
        let peer = self.inner.conn(conn_id)?;
        let pt = peer
            .stream_payload_type(stream_id)
            .ok_or_else(|| Error::InvalidArg("unknown stream".into()))?;
        let codec = codec::codec_kind_from_payload_type(pt).unwrap_or(CodecKind::Opus);
        let params = self
            .inner
            .binding_params(codec, self.inner.audio_params.bitrate);
        let decoder = codec::make_codec(&params)?;
        self.inner.audio.lock().unwrap().playbacks.insert(
            (conn_id, stream_id),
            PlaybackBinding {
                decoder,
                jitter: JitterBuffer::new(JITTER_TARGET_FRAMES, JITTER_MAX_FRAMES),
            },
        );
        self.inner.wake.notify_one();
        Ok(())
    }

    /// Send a reliable data-channel message on a connection.
    pub fn send_data(&self, conn_id: ConnId, bytes: Vec<u8>) -> Result<()> {
        self.inner.conn(conn_id)?.send_data(bytes)
    }

    /// Close a connection.
    pub fn close_connection(&self, conn_id: ConnId) {
        if let Some(peer) = self.inner.conns.lock().unwrap().remove(&conn_id) {
            peer.shutdown();
        }
        self.inner.purge_conn(conn_id);
    }

    /// Per-connection counters: (datagrams_sent, datagrams_recv, bytes_sent, bytes_recv, send_errors).
    pub fn connection_stats(&self, conn_id: ConnId) -> Option<(u64, u64, u64, u64, u64)> {
        self.inner
            .conns
            .lock()
            .unwrap()
            .get(&conn_id)
            .map(|p| p.counters_snapshot())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        for (_, peer) in self.inner.conns.lock().unwrap().drain() {
            peer.shutdown();
        }
        for h in &self.tasks {
            h.abort();
        }
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_timeout(Duration::from_millis(500));
        }
    }
}

impl EngineInner {
    fn conn(&self, conn_id: ConnId) -> Result<Arc<PeerConn>> {
        self.conns
            .lock()
            .unwrap()
            .get(&conn_id)
            .cloned()
            .ok_or(Error::NotConnected)
    }

    fn binding_params(&self, codec: CodecKind, bitrate: u32) -> AudioParams {
        AudioParams {
            codec,
            bitrate,
            ..self.audio_params
        }
    }

    fn register_conn(&self, conn: quinn::Connection, is_client: bool, enable_data: bool) -> ConnId {
        let id = self.next_conn_id.fetch_add(1, Ordering::Relaxed);
        let peer = PeerConn::new(
            id,
            conn,
            is_client,
            enable_data,
            self.inbound_tx.clone(),
            self.event_tx.clone(),
        );
        self.conns.lock().unwrap().insert(id, peer);
        self.app.on_connection(id, !is_client);
        id
    }

    fn remove_bindings_for(&self, conn_id: ConnId, stream_id: StreamId) {
        let mut hub = self.audio.lock().unwrap();
        hub.captures.remove(&(conn_id, stream_id));
        hub.playbacks.remove(&(conn_id, stream_id));
    }

    /// Drop everything referencing a closed connection.
    fn purge_conn(&self, conn_id: ConnId) {
        {
            let mut routes = self.routes.lock().unwrap();
            routes.retain(|(sc, _), _| *sc != conn_id);
            for dsts in routes.values_mut() {
                dsts.retain(|(dc, _)| *dc != conn_id);
            }
        }
        let mut hub = self.audio.lock().unwrap();
        hub.captures.retain(|(c, _), _| *c != conn_id);
        hub.playbacks.retain(|(c, _), _| *c != conn_id);
        self.wake.notify_one();
    }

    /// Whether any capture or playback binding is currently active (needs the device).
    fn needs_device(&self) -> bool {
        let hub = self.audio.lock().unwrap();
        let conns = self.conns.lock().unwrap();
        let active =
            |c: &ConnId, s: &StreamId| conns.get(c).map(|p| p.is_active(*s)).unwrap_or(false);
        hub.captures.keys().any(|(c, s)| active(c, s))
            || hub.playbacks.keys().any(|(c, s)| active(c, s))
    }

    fn open_device(&self) -> Result<DeviceBundle> {
        let backend = audio::make_backend(self.backend_kind)?;
        let streams = backend.start(&self.audio_params)?;
        let (captured, to_play, device) = streams.into_parts();
        Ok((device, captured, to_play))
    }

    /// Process one captured mic frame: encode+send active captures, and mix the
    /// active playbacks into the output frame.
    fn process_frame(&self, frame: &Frame, frame_len: usize, frame_samples: u32) -> Frame {
        let mut mix = vec![0i32; frame_len];
        let mut payload = Vec::with_capacity(512);
        let mut pcm: Vec<i16> = Vec::with_capacity(frame_len);

        let mut hub = self.audio.lock().unwrap();
        let conns = self.conns.lock().unwrap();

        // Capture → encode → send (skip inactive streams entirely).
        for binding in hub.captures.values_mut() {
            let peer = match conns.get(&binding.conn_id) {
                Some(p) => p,
                None => continue,
            };
            if !peer.is_active(binding.stream_id) {
                continue;
            }
            if binding.encoder.encode(frame, &mut payload).is_ok() {
                peer.send_packet(binding.stream_id, binding.timestamp, false, &payload);
                binding.timestamp = binding.timestamp.wrapping_add(frame_samples);
            }
        }

        // Playback → decode active streams → mix.
        for ((conn_id, stream_id), pb) in hub.playbacks.iter_mut() {
            let active = conns
                .get(conn_id)
                .map(|p| p.is_active(*stream_id))
                .unwrap_or(false);
            if !active {
                continue;
            }
            let outcome = pb.jitter.pop();
            let decoded = match outcome {
                Playout::Frame(p) => pb.decoder.decode(Some(&p), &mut pcm),
                Playout::Conceal => pb.decoder.decode(None, &mut pcm),
                Playout::Starved => continue,
            };
            if decoded.is_ok() {
                for (m, s) in mix.iter_mut().zip(pcm.iter()) {
                    *m += *s as i32;
                }
            }
        }
        drop(conns);
        drop(hub);

        mix.iter()
            .map(|&s| s.clamp(i16::MIN as i32, i16::MAX as i32) as i16)
            .collect::<Vec<i16>>()
            .into_boxed_slice()
    }
}

/// Routes inbound encoded packets: into playback jitter and/or forwarded natively.
async fn dispatch_task(inner: Arc<EngineInner>, mut inbound_rx: mpsc::Receiver<InboundMedia>) {
    while let Some(m) = inbound_rx.recv().await {
        let key = (m.conn_id, m.header.stream_id);

        // Playback: buffer into the jitter buffer for this stream, if bound.
        {
            let mut hub = inner.audio.lock().unwrap();
            if let Some(pb) = hub.playbacks.get_mut(&key) {
                pb.jitter.push(m.header.sequence, m.payload.to_vec());
            }
        }

        // Forward (SFU): fan out to each routed downstream connection natively.
        let dests = inner.routes.lock().unwrap().get(&key).cloned();
        if let Some(dests) = dests {
            let conns = inner.conns.lock().unwrap();
            for (dc, ds) in dests {
                if let Some(dp) = conns.get(&dc) {
                    dp.send_packet(ds, m.header.timestamp, m.header.marker, &m.payload);
                }
            }
        }
    }
}

/// Translates connection events into app callbacks and engine reactions.
async fn event_task(inner: Arc<EngineInner>, mut event_rx: mpsc::Receiver<ConnEvent>) {
    while let Some(ev) = event_rx.recv().await {
        match ev {
            ConnEvent::StreamOpened(c, o) => {
                let codec =
                    codec::codec_kind_from_payload_type(o.payload_type).unwrap_or(CodecKind::Opus);
                inner.app.on_stream_opened(
                    c,
                    o.stream_id,
                    StreamParams {
                        codec,
                        sample_rate: o.sample_rate,
                        channels: o.channels,
                        frame_ms: o.frame_ms,
                        bitrate: o.bitrate,
                    },
                );
            }
            ConnEvent::StreamActive(c, s, active) => {
                inner.wake.notify_one();
                inner.app.on_stream_active(c, s, active);
            }
            ConnEvent::StreamClosed(c, s) => {
                inner.remove_bindings_for(c, s);
                inner.app.on_stream_closed(c, s);
            }
            ConnEvent::Data(c, bytes) => inner.app.on_data(c, &bytes),
            ConnEvent::Disconnected(c) => {
                inner.conns.lock().unwrap().remove(&c);
                inner.purge_conn(c);
                inner.app.on_connection_closed(c);
            }
        }
    }
}

/// Owns the audio device lifecycle: opens it when a binding is active, stops it
/// (stopping capture) when nothing is active, and pumps one frame at a time.
async fn audio_pump(inner: Arc<EngineInner>) {
    let params = inner.audio_params;
    let frame_len = params.frame_len();
    let frame_samples = params.frame_samples() as u32;
    let mut device: Option<DeviceBundle> = None;

    loop {
        let need = inner.needs_device();
        if need && device.is_none() {
            match inner.open_device() {
                Ok(d) => device = Some(d),
                Err(e) => {
                    inner.app.on_error(e.code(), &e.to_string());
                    // Avoid a busy retry loop on persistent device failure.
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        } else if !need && device.is_some() {
            device = None; // dropping the device stops capture/render
        }

        match device.as_mut() {
            Some((_dev, captured, to_play)) => {
                tokio::select! {
                    frame = captured.recv() => {
                        match frame {
                            Some(frame) => {
                                let out = inner.process_frame(&frame, frame_len, frame_samples);
                                let _ = to_play.send(out).await;
                            }
                            None => { device = None; }
                        }
                    }
                    _ = inner.wake.notified() => { /* re-evaluate need_device */ }
                }
            }
            None => {
                tokio::select! {
                    _ = inner.wake.notified() => {}
                    _ = tokio::time::sleep(Duration::from_millis(250)) => {}
                }
            }
        }
    }
}
