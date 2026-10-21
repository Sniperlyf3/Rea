//! One QUIC connection as a pure encoded-packet transport plus the reliable
//! control channel. No codecs or audio devices live here, so a `PeerConn` is
//! equally usable by a capturing client, a decoding receiver, or an SFU that
//! only relays encoded packets.
//!
//! The engine drives everything: it receives inbound media packets and control
//! events through channels, and calls [`PeerConn::send_packet`] / `open_stream`
//! / `set_active` / `close_stream` to act.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use quinn::Connection;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::control::{self, ControlMsg, OpenStream, PURPOSE_CONTROL, PURPOSE_DATA};
use crate::framing::{MediaHeader, StreamId, HEADER_LEN};
use crate::{Error, Result};

/// Engine-assigned connection identifier.
pub type ConnId = u64;

/// Per-stream state tracked on a connection.
struct StreamState {
    params: OpenStream,
    /// Our own desired active state.
    want_local: bool,
    /// The peer's desired active state (optimistically true until told otherwise).
    want_remote: bool,
    out_seq: u16,
}

impl StreamState {
    fn active(&self) -> bool {
        self.want_local && self.want_remote
    }
}

/// An inbound media packet handed to the engine for routing/playback.
pub struct InboundMedia {
    pub conn_id: ConnId,
    pub header: MediaHeader,
    pub payload: Bytes,
}

/// Control-plane / lifecycle events surfaced to the engine.
pub enum ConnEvent {
    /// The peer announced a stream it will send.
    StreamOpened(ConnId, OpenStream),
    /// A stream's effective active state (want_local && want_remote) may have changed.
    StreamActive(ConnId, StreamId, bool),
    /// A stream was closed by the peer.
    StreamClosed(ConnId, StreamId),
    /// A reliable data-channel message arrived.
    Data(ConnId, Vec<u8>),
    /// The connection closed.
    Disconnected(ConnId),
}

/// Outcome of a [`PeerConn::send_packet`] call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendOutcome {
    Sent(usize),
    Inactive,
    UnknownStream,
    TooLarge,
    ConnectionLost,
    Error,
}

struct Counters {
    datagrams_sent: AtomicU64,
    datagrams_recv: AtomicU64,
    bytes_sent: AtomicU64,
    bytes_recv: AtomicU64,
    send_errors: AtomicU64,
}

/// A live QUIC connection with its control channel and stream bookkeeping.
pub struct PeerConn {
    id: ConnId,
    conn: Connection,
    role_bit: u16,
    next_local_id: AtomicU16,
    streams: Mutex<HashMap<StreamId, StreamState>>,
    control_tx: mpsc::Sender<ControlMsg>,
    data_tx: mpsc::Sender<Vec<u8>>,
    counters: Counters,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl PeerConn {
    pub fn id(&self) -> ConnId {
        self.id
    }

    pub fn quinn(&self) -> &Connection {
        &self.conn
    }

    /// Wire up a connection: control stream, optional data stream, datagram recv
    /// loop, and disconnect watcher. `is_client` opens the internal streams;
    /// a server accepts them.
    pub fn new(
        id: ConnId,
        conn: Connection,
        is_client: bool,
        enable_data: bool,
        inbound_tx: mpsc::Sender<InboundMedia>,
        event_tx: mpsc::Sender<ConnEvent>,
    ) -> Arc<PeerConn> {
        let (control_tx, control_rx) = mpsc::channel::<ControlMsg>(64);
        let (data_tx, data_rx) = mpsc::channel::<Vec<u8>>(64);

        let peer = Arc::new(PeerConn {
            id,
            conn: conn.clone(),
            role_bit: if is_client { 0x0000 } else { 0x8000 },
            next_local_id: AtomicU16::new(0),
            streams: Mutex::new(HashMap::new()),
            control_tx,
            data_tx,
            counters: Counters {
                datagrams_sent: AtomicU64::new(0),
                datagrams_recv: AtomicU64::new(0),
                bytes_sent: AtomicU64::new(0),
                bytes_recv: AtomicU64::new(0),
                send_errors: AtomicU64::new(0),
            },
            tasks: Mutex::new(Vec::new()),
        });

        let mut handles = Vec::new();

        // Datagram receive loop.
        {
            let peer2 = peer.clone();
            let inbound_tx = inbound_tx.clone();
            handles.push(tokio::spawn(async move {
                while let Ok(bytes) = peer2.conn.read_datagram().await {
                    peer2
                        .counters
                        .datagrams_recv
                        .fetch_add(1, Ordering::Relaxed);
                    peer2
                        .counters
                        .bytes_recv
                        .fetch_add(bytes.len() as u64, Ordering::Relaxed);
                    if let Ok((header, _)) = MediaHeader::parse(&bytes) {
                        let payload = bytes.slice(HEADER_LEN..);
                        let _ = inbound_tx
                            .send(InboundMedia {
                                conn_id: peer2.id,
                                header,
                                payload,
                            })
                            .await;
                    }
                }
            }));
        }

        // Disconnect watcher.
        {
            let conn2 = conn.clone();
            let event_tx2 = event_tx.clone();
            let cid = id;
            handles.push(tokio::spawn(async move {
                let _ = conn2.closed().await;
                let _ = event_tx2.send(ConnEvent::Disconnected(cid)).await;
            }));
        }

        // Internal control + data streams.
        if is_client {
            let peer2 = peer.clone();
            let event_tx2 = event_tx.clone();
            handles.push(tokio::spawn(async move {
                if let Err(e) = peer2
                    .open_internal_streams(enable_data, control_rx, data_rx, event_tx2)
                    .await
                {
                    tracing::warn!("conn {}: open internal streams: {e}", peer2.id);
                }
            }));
        } else {
            let peer2 = peer.clone();
            let event_tx2 = event_tx.clone();
            handles.push(tokio::spawn(async move {
                peer2
                    .accept_internal_streams(control_rx, data_rx, event_tx2)
                    .await;
            }));
        }

        *peer.tasks.lock().unwrap() = handles;
        peer
    }

    async fn open_internal_streams(
        self: &Arc<Self>,
        enable_data: bool,
        control_rx: mpsc::Receiver<ControlMsg>,
        data_rx: mpsc::Receiver<Vec<u8>>,
        event_tx: mpsc::Sender<ConnEvent>,
    ) -> Result<()> {
        let (mut csend, crecv) = self
            .conn
            .open_bi()
            .await
            .map_err(|e| Error::Connection(e.to_string()))?;
        csend
            .write_all(&[PURPOSE_CONTROL])
            .await
            .map_err(|e| Error::Connection(e.to_string()))?;
        self.spawn_control(csend, crecv, control_rx, event_tx.clone());

        if enable_data {
            let (mut dsend, drecv) = self
                .conn
                .open_bi()
                .await
                .map_err(|e| Error::Connection(e.to_string()))?;
            dsend
                .write_all(&[PURPOSE_DATA])
                .await
                .map_err(|e| Error::Connection(e.to_string()))?;
            self.spawn_data(dsend, drecv, data_rx, event_tx);
        }
        Ok(())
    }

    async fn accept_internal_streams(
        self: &Arc<Self>,
        control_rx: mpsc::Receiver<ControlMsg>,
        data_rx: mpsc::Receiver<Vec<u8>>,
        event_tx: mpsc::Sender<ConnEvent>,
    ) {
        // Each receiver is consumed by whichever purpose stream arrives first.
        let mut control_rx = Some(control_rx);
        let mut data_rx = Some(data_rx);

        while let Ok((send, mut recv)) = self.conn.accept_bi().await {
            let mut purpose = [0u8; 1];
            if recv.read_exact(&mut purpose).await.is_err() {
                continue;
            }
            match purpose[0] {
                PURPOSE_CONTROL => {
                    if let Some(rx) = control_rx.take() {
                        self.spawn_control(send, recv, rx, event_tx.clone());
                    }
                }
                PURPOSE_DATA => {
                    if let Some(rx) = data_rx.take() {
                        self.spawn_data(send, recv, rx, event_tx.clone());
                    }
                }
                _ => {}
            }
        }
    }

    fn spawn_control(
        self: &Arc<Self>,
        mut send: quinn::SendStream,
        mut recv: quinn::RecvStream,
        mut control_rx: mpsc::Receiver<ControlMsg>,
        event_tx: mpsc::Sender<ConnEvent>,
    ) {
        // Writer.
        let mut handles = self.tasks.lock().unwrap();
        handles.push(tokio::spawn(async move {
            while let Some(msg) = control_rx.recv().await {
                if control::write_message(&mut send, &msg).await.is_err() {
                    break;
                }
            }
            let _ = send.finish();
        }));

        // Reader.
        let peer = self.clone();
        handles.push(tokio::spawn(async move {
            while let Ok(Some(msg)) = control::read_message(&mut recv).await {
                peer.handle_control(msg, &event_tx).await;
            }
        }));
    }

    fn spawn_data(
        self: &Arc<Self>,
        send: quinn::SendStream,
        recv: quinn::RecvStream,
        data_rx: mpsc::Receiver<Vec<u8>>,
        event_tx: mpsc::Sender<ConnEvent>,
    ) {
        let mut handles = self.tasks.lock().unwrap();
        handles.push(tokio::spawn(crate::datachannel::writer_task(send, data_rx)));
        let cid = self.id;
        handles.push(tokio::spawn(async move {
            crate::datachannel::reader_task_events(recv, cid, event_tx).await;
        }));
    }

    async fn handle_control(&self, msg: ControlMsg, event_tx: &mpsc::Sender<ConnEvent>) {
        match msg {
            ControlMsg::Open(o) => {
                {
                    let mut map = self.streams.lock().unwrap();
                    map.entry(o.stream_id).or_insert(StreamState {
                        params: o,
                        want_local: true,      // auto-subscribe to inbound streams
                        want_remote: o.active, // the sender's announced want
                        out_seq: 0,
                    });
                }
                let _ = event_tx.send(ConnEvent::StreamOpened(self.id, o)).await;
            }
            ControlMsg::SetActive { stream_id, active } => {
                let eff = {
                    let mut map = self.streams.lock().unwrap();
                    if let Some(st) = map.get_mut(&stream_id) {
                        st.want_remote = active;
                        Some(st.active())
                    } else {
                        None
                    }
                };
                if let Some(eff) = eff {
                    let _ = event_tx
                        .send(ConnEvent::StreamActive(self.id, stream_id, eff))
                        .await;
                }
            }
            ControlMsg::Close { stream_id } => {
                self.streams.lock().unwrap().remove(&stream_id);
                let _ = event_tx
                    .send(ConnEvent::StreamClosed(self.id, stream_id))
                    .await;
            }
        }
    }

    /// Open a stream we will produce media on. Returns the allocated id.
    pub fn open_stream(
        &self,
        payload_type: u8,
        sample_rate: u32,
        channels: u16,
        frame_ms: u32,
        bitrate: u32,
        active: bool,
    ) -> StreamId {
        let n = self.next_local_id.fetch_add(1, Ordering::Relaxed);
        let stream_id = self.role_bit | (n & 0x7fff);
        let params = OpenStream {
            stream_id,
            payload_type,
            sample_rate,
            channels,
            frame_ms,
            bitrate,
            active,
        };
        self.streams.lock().unwrap().insert(
            stream_id,
            StreamState {
                params,
                want_local: active,
                want_remote: true, // optimistic: assume the peer wants it until told no
                out_seq: 0,
            },
        );
        let _ = self.control_tx.try_send(ControlMsg::Open(params));
        stream_id
    }

    /// Set our desired active state for a stream and tell the peer.
    pub fn set_active(&self, stream_id: StreamId, active: bool) -> bool {
        let found = {
            let mut map = self.streams.lock().unwrap();
            match map.get_mut(&stream_id) {
                Some(st) => {
                    st.want_local = active;
                    true
                }
                None => false,
            }
        };
        if found {
            let _ = self
                .control_tx
                .try_send(ControlMsg::SetActive { stream_id, active });
        }
        found
    }

    /// Close a stream and notify the peer.
    pub fn close_stream(&self, stream_id: StreamId) {
        self.streams.lock().unwrap().remove(&stream_id);
        let _ = self.control_tx.try_send(ControlMsg::Close { stream_id });
    }

    /// The on-wire payload type of a known stream, if any.
    pub fn stream_payload_type(&self, stream_id: StreamId) -> Option<u8> {
        self.streams
            .lock()
            .unwrap()
            .get(&stream_id)
            .map(|s| s.params.payload_type)
    }

    /// True if the stream exists and is active in both directions.
    pub fn is_active(&self, stream_id: StreamId) -> bool {
        self.streams
            .lock()
            .unwrap()
            .get(&stream_id)
            .map(|s| s.active())
            .unwrap_or(false)
    }

    /// Stamp and send one encoded packet on a stream, honoring the active flag.
    pub fn send_packet(
        &self,
        stream_id: StreamId,
        timestamp: u32,
        marker: bool,
        payload: &[u8],
    ) -> SendOutcome {
        let (seq, payload_type) = {
            let mut map = self.streams.lock().unwrap();
            let st = match map.get_mut(&stream_id) {
                Some(s) => s,
                None => return SendOutcome::UnknownStream,
            };
            if !st.active() {
                return SendOutcome::Inactive;
            }
            let seq = st.out_seq;
            st.out_seq = st.out_seq.wrapping_add(1);
            (seq, st.params.payload_type)
        };

        if let Some(max) = self.conn.max_datagram_size() {
            if HEADER_LEN + payload.len() > max {
                self.counters.send_errors.fetch_add(1, Ordering::Relaxed);
                return SendOutcome::TooLarge;
            }
        }

        let hdr = MediaHeader {
            marker,
            payload_type,
            stream_id,
            sequence: seq,
            timestamp,
        };
        let dgram = hdr.to_datagram(payload);
        let len = dgram.len();
        match self.conn.send_datagram(Bytes::from(dgram)) {
            Ok(()) => {
                self.counters.datagrams_sent.fetch_add(1, Ordering::Relaxed);
                self.counters
                    .bytes_sent
                    .fetch_add(len as u64, Ordering::Relaxed);
                SendOutcome::Sent(len)
            }
            Err(quinn::SendDatagramError::ConnectionLost(_)) => SendOutcome::ConnectionLost,
            Err(_) => {
                self.counters.send_errors.fetch_add(1, Ordering::Relaxed);
                SendOutcome::Error
            }
        }
    }

    /// Queue a reliable data-channel message.
    pub fn send_data(&self, bytes: Vec<u8>) -> Result<()> {
        use mpsc::error::TrySendError;
        self.data_tx.try_send(bytes).map_err(|e| match e {
            TrySendError::Full(_) => Error::InvalidArg("data channel send queue full".into()),
            TrySendError::Closed(_) => Error::NotConnected,
        })
    }

    pub fn counters_snapshot(&self) -> (u64, u64, u64, u64, u64) {
        (
            self.counters.datagrams_sent.load(Ordering::Relaxed),
            self.counters.datagrams_recv.load(Ordering::Relaxed),
            self.counters.bytes_sent.load(Ordering::Relaxed),
            self.counters.bytes_recv.load(Ordering::Relaxed),
            self.counters.send_errors.load(Ordering::Relaxed),
        )
    }

    /// Close the connection and abort its tasks.
    pub fn shutdown(&self) {
        self.conn.close(0u32.into(), b"bye");
        for h in self.tasks.lock().unwrap().drain(..) {
            h.abort();
        }
    }
}

impl Drop for PeerConn {
    fn drop(&mut self) {
        self.conn.close(0u32.into(), b"bye");
        for h in self.tasks.lock().unwrap().drain(..) {
            h.abort();
        }
    }
}
