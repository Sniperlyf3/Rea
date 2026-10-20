//! End-to-end loopback: two real QUIC sessions over 127.0.0.1 exchange media
//! datagrams (null audio backend + small PCM frames) and data-channel messages.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use quicmedia_core::{
    security, AudioBackendKind, AudioParams, CodecKind, Config, ConnectionState, Events,
    PeerVerification, Role, SecurityConfig, Session,
};

struct TestEvents {
    name: &'static str,
    connected: Arc<AtomicBool>,
    received: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl Events for TestEvents {
    fn on_state(&self, state: ConnectionState) {
        if state == ConnectionState::Connected {
            self.connected.store(true, Ordering::SeqCst);
        }
    }
    fn on_data(&self, data: &[u8]) {
        self.received.lock().unwrap().push(data.to_vec());
    }
    fn on_error(&self, code: i32, message: &str) {
        eprintln!("[{}] error {code}: {message}", self.name);
    }
}

fn free_udp_port() -> u16 {
    let s = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let p = s.local_addr().unwrap().port();
    drop(s);
    p
}

fn small_audio() -> AudioParams {
    // 8 kHz mono @ 20 ms = 160 samples => 320-byte PCM payload, well under the
    // QUIC datagram size limit, so passthrough media flows without fragmentation.
    AudioParams {
        sample_rate: 8_000,
        channels: 1,
        frame_ms: 20,
        bitrate: 24_000,
        codec: CodecKind::Passthrough,
    }
}

fn wait_until(flag: &AtomicBool, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if flag.load(Ordering::SeqCst) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn quic_loopback_media_and_data() {
    let port = free_udp_port();

    let (cert, key) = security::generate_self_signed(&["localhost".into()]).unwrap();
    let pin = security::cert_sha256(&cert);
    let alpn = vec![quicmedia_core::DEFAULT_ALPN.to_vec()];

    // ---- Server ----
    let srv_connected = Arc::new(AtomicBool::new(false));
    let srv_rx = Arc::new(Mutex::new(Vec::new()));
    let server = Session::start(
        Config {
            role: Role::Server,
            bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
            peer_addr: None,
            server_name: "localhost".into(),
            audio: small_audio(),
            security: SecurityConfig {
                cert_der: cert.clone(),
                key_der: key.clone(),
                verification: PeerVerification::Insecure,
                alpn: alpn.clone(),
            },
            audio_backend: AudioBackendKind::Null,
            enable_audio: true,
            enable_data_channel: true,
        },
        Arc::new(TestEvents {
            name: "server",
            connected: srv_connected.clone(),
            received: srv_rx.clone(),
        }),
    )
    .expect("server start");

    // Give the server a moment to begin listening.
    std::thread::sleep(Duration::from_millis(100));

    // ---- Client ----
    let cli_connected = Arc::new(AtomicBool::new(false));
    let cli_rx = Arc::new(Mutex::new(Vec::new()));
    let client = Session::start(
        Config {
            role: Role::Client,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            peer_addr: Some(format!("127.0.0.1:{port}").parse().unwrap()),
            server_name: "localhost".into(),
            audio: small_audio(),
            security: SecurityConfig {
                cert_der: Vec::new(),
                key_der: Vec::new(),
                verification: PeerVerification::PinnedSha256(pin),
                alpn: alpn.clone(),
            },
            audio_backend: AudioBackendKind::Null,
            enable_audio: true,
            enable_data_channel: true,
        },
        Arc::new(TestEvents {
            name: "client",
            connected: cli_connected.clone(),
            received: cli_rx.clone(),
        }),
    )
    .expect("client start");

    assert!(
        wait_until(&cli_connected, Duration::from_secs(5)),
        "client did not connect"
    );
    assert!(
        wait_until(&srv_connected, Duration::from_secs(5)),
        "server did not connect"
    );

    // ---- Data channel: client -> server, then server -> client ----
    client.send_data(b"hello from client".to_vec()).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    server.send_data(b"hi from server".to_vec()).unwrap();

    // ---- Let media datagrams flow ----
    std::thread::sleep(Duration::from_millis(600));

    let srv_msgs = srv_rx.lock().unwrap().clone();
    assert!(
        srv_msgs.iter().any(|m| m == b"hello from client"),
        "server did not receive client data-channel message, got {srv_msgs:?}"
    );
    let cli_msgs = cli_rx.lock().unwrap().clone();
    assert!(
        cli_msgs.iter().any(|m| m == b"hi from server"),
        "client did not receive server data-channel message, got {cli_msgs:?}"
    );

    let ss = server.stats();
    let cs = client.stats();
    // Null backend produces one silent frame per 20 ms on both ends; after ~0.9 s
    // of connectivity we expect media datagrams in both directions.
    assert!(cs.datagrams_sent > 0, "client sent no media datagrams");
    assert!(ss.datagrams_recv > 0, "server received no media datagrams");
    assert!(ss.datagrams_sent > 0, "server sent no media datagrams");
    assert!(cs.datagrams_recv > 0, "client received no media datagrams");

    drop(client);
    drop(server);
}
