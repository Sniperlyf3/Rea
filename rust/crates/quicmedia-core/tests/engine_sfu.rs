//! End-to-end SFU test over loopback: sender → SFU → receiver.
//!
//! Three engines in one process. The sender captures (Null backend, silence) and
//! sends encoded packets to the SFU; the SFU forwards them natively to the
//! receiver; the receiver binds playback. Then the receiver deactivates the
//! stream and we verify the SFU stops forwarding to it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use quicmedia_core::{
    security, AudioBackendKind, AudioParams, CodecKind, ConnId, Engine, EngineEvents,
    PeerVerification, SecurityConfig, StreamId, StreamParams,
};

#[derive(Default)]
struct Collector {
    connections: Mutex<Vec<(ConnId, bool)>>,
    opened: Mutex<Vec<(ConnId, StreamId)>>,
    errors: AtomicU64,
}

impl EngineEvents for Collector {
    fn on_connection(&self, conn_id: ConnId, incoming: bool) {
        self.connections.lock().unwrap().push((conn_id, incoming));
    }
    fn on_connection_closed(&self, _conn_id: ConnId) {}
    fn on_stream_opened(&self, conn_id: ConnId, stream_id: StreamId, _p: StreamParams) {
        self.opened.lock().unwrap().push((conn_id, stream_id));
    }
    fn on_stream_active(&self, _c: ConnId, _s: StreamId, _active: bool) {}
    fn on_stream_closed(&self, _c: ConnId, _s: StreamId) {}
    fn on_data(&self, _c: ConnId, _d: &[u8]) {}
    fn on_error(&self, code: i32, msg: &str) {
        eprintln!("engine error {code}: {msg}");
        self.errors.fetch_add(1, Ordering::SeqCst);
    }
}

fn free_udp_port() -> u16 {
    let s = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let p = s.local_addr().unwrap().port();
    drop(s);
    p
}

fn test_params() -> AudioParams {
    // 8 kHz mono @ 20 ms = 160 samples => 320-byte PCM payload, well under the
    // QUIC datagram limit so passthrough media flows without fragmentation.
    AudioParams {
        sample_rate: 8_000,
        channels: 1,
        frame_ms: 20,
        bitrate: 0,
        codec: CodecKind::Passthrough,
    }
}

fn wait_until<F: Fn() -> bool>(f: F, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn sender_sfu_receiver_forwarding_and_active() {
    let port = free_udp_port();
    let (cert, key) = security::generate_self_signed(&["localhost".into()]).unwrap();
    let pin = security::cert_sha256(&cert);
    let alpn = vec![quicmedia_core::DEFAULT_ALPN.to_vec()];

    let server_sec = SecurityConfig {
        cert_der: cert.clone(),
        key_der: key,
        verification: PeerVerification::Insecure,
        alpn: alpn.clone(),
    };
    let client_sec = SecurityConfig {
        cert_der: Vec::new(),
        key_der: Vec::new(),
        verification: PeerVerification::PinnedSha256(pin),
        alpn,
    };

    // ---- SFU (server) ----
    let sfu_app = Arc::new(Collector::default());
    let sfu = Engine::new(test_params(), AudioBackendKind::Null, sfu_app.clone()).unwrap();
    sfu.listen(
        format!("127.0.0.1:{port}").parse().unwrap(),
        &server_sec,
        false,
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(100));

    // ---- Sender (client) ----
    let sender_app = Arc::new(Collector::default());
    let sender = Engine::new(test_params(), AudioBackendKind::Null, sender_app.clone()).unwrap();
    let sender_conn = sender
        .connect(
            "127.0.0.1:0".parse().unwrap(),
            format!("127.0.0.1:{port}").parse().unwrap(),
            "localhost",
            &client_sec,
            false,
        )
        .unwrap();

    // SFU should see the sender connect.
    assert!(
        wait_until(
            || sfu_app.connections.lock().unwrap().len() == 1,
            Duration::from_secs(5)
        ),
        "SFU did not accept the sender"
    );
    let sfu_conn_sender = sfu_app.connections.lock().unwrap()[0].0;

    // Sender opens a capture stream (mic -> encode -> send to SFU).
    let s_send = sender
        .add_capture_stream(sender_conn, CodecKind::Passthrough, 0, true)
        .unwrap();

    // SFU learns of the stream.
    assert!(
        wait_until(
            || sfu_app
                .opened
                .lock()
                .unwrap()
                .iter()
                .any(|&(c, s)| c == sfu_conn_sender && s == s_send),
            Duration::from_secs(5)
        ),
        "SFU did not see the sender's stream"
    );

    // ---- Receiver (client) ----
    let receiver_app = Arc::new(Collector::default());
    let receiver =
        Engine::new(test_params(), AudioBackendKind::Null, receiver_app.clone()).unwrap();
    let receiver_conn = receiver
        .connect(
            "127.0.0.1:0".parse().unwrap(),
            format!("127.0.0.1:{port}").parse().unwrap(),
            "localhost",
            &client_sec,
            false,
        )
        .unwrap();

    assert!(
        wait_until(
            || sfu_app.connections.lock().unwrap().len() == 2,
            Duration::from_secs(5)
        ),
        "SFU did not accept the receiver"
    );
    let sfu_conn_receiver = sfu_app.connections.lock().unwrap()[1].0;

    // SFU opens a downstream stream toward the receiver and routes sender -> receiver.
    let s_fwd = sfu
        .open_stream(sfu_conn_receiver, CodecKind::Passthrough, 0, true)
        .unwrap();
    sfu.add_route(sfu_conn_sender, s_send, sfu_conn_receiver, s_fwd);

    // Receiver learns of the forwarded stream and binds playback.
    assert!(
        wait_until(
            || receiver_app
                .opened
                .lock()
                .unwrap()
                .iter()
                .any(|&(c, s)| c == receiver_conn && s == s_fwd),
            Duration::from_secs(5)
        ),
        "receiver did not see the forwarded stream"
    );
    receiver.bind_playback(receiver_conn, s_fwd).unwrap();

    // Media should now flow sender -> SFU -> receiver.
    assert!(
        wait_until(
            || receiver
                .connection_stats(receiver_conn)
                .map(|s| s.1 > 0)
                .unwrap_or(false),
            Duration::from_secs(5)
        ),
        "receiver received no forwarded media"
    );
    assert!(
        sfu.connection_stats(sfu_conn_receiver)
            .map(|s| s.0 > 0)
            .unwrap_or(false),
        "SFU forwarded no datagrams to the receiver"
    );

    // ---- Receiver deactivates the stream: the SFU must stop forwarding ----
    receiver
        .set_stream_active(receiver_conn, s_fwd, false)
        .unwrap();
    // Allow the SetActive control message to reach the SFU.
    std::thread::sleep(Duration::from_millis(300));
    let before = receiver.connection_stats(receiver_conn).unwrap().1;
    std::thread::sleep(Duration::from_millis(500));
    let after = receiver.connection_stats(receiver_conn).unwrap().1;
    assert_eq!(
        before, after,
        "SFU kept forwarding after the receiver deactivated the stream ({before} -> {after})"
    );

    assert_eq!(
        sfu_app.errors.load(Ordering::SeqCst),
        0,
        "SFU reported errors"
    );

    drop(sender);
    drop(receiver);
    drop(sfu);
}
