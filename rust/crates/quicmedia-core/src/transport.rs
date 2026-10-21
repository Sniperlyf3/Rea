//! QUIC endpoint setup (quinn + rustls). The engine builds a client endpoint to
//! dial peers and/or a server endpoint to accept many incoming connections.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{ClientConfig, Connection, Endpoint, ServerConfig, TransportConfig};

use crate::config::SecurityConfig;
use crate::{security, Error, Result};

/// Transport tuning shared by both roles.
fn transport_config() -> Arc<TransportConfig> {
    let mut t = TransportConfig::default();
    t.datagram_receive_buffer_size(Some(2 * 1024 * 1024));
    t.datagram_send_buffer_size(1024 * 1024);
    t.keep_alive_interval(Some(Duration::from_secs(5)));
    if let Ok(idle) = quinn::IdleTimeout::try_from(Duration::from_secs(30)) {
        t.max_idle_timeout(Some(idle));
    }
    Arc::new(t)
}

fn make_server_config(sec: &SecurityConfig) -> Result<ServerConfig> {
    let rustls_sc = security::build_server_config(sec)?;
    let qsc = QuicServerConfig::try_from(rustls_sc).map_err(|e| Error::Tls(e.to_string()))?;
    let mut sc = ServerConfig::with_crypto(Arc::new(qsc));
    sc.transport_config(transport_config());
    Ok(sc)
}

fn make_client_config(sec: &SecurityConfig) -> Result<ClientConfig> {
    let rustls_cc = security::build_client_config(sec)?;
    let qcc = QuicClientConfig::try_from(rustls_cc).map_err(|e| Error::Tls(e.to_string()))?;
    let mut cc = ClientConfig::new(Arc::new(qcc));
    cc.transport_config(transport_config());
    Ok(cc)
}

/// Build a client endpoint bound to `bind`, with `sec` as the default client config.
pub fn client_endpoint(bind: SocketAddr, sec: &SecurityConfig) -> Result<Endpoint> {
    let mut endpoint = Endpoint::client(bind)?;
    endpoint.set_default_client_config(make_client_config(sec)?);
    Ok(endpoint)
}

/// Dial a peer on an existing client endpoint.
pub async fn dial(endpoint: &Endpoint, peer: SocketAddr, server_name: &str) -> Result<Connection> {
    let connecting = endpoint
        .connect(peer, server_name)
        .map_err(|e| Error::Connect(e.to_string()))?;
    connecting.await.map_err(|e| Error::Connect(e.to_string()))
}

/// Build a server endpoint listening on `bind`.
pub fn server_endpoint(bind: SocketAddr, sec: &SecurityConfig) -> Result<Endpoint> {
    Endpoint::server(make_server_config(sec)?, bind).map_err(Error::Io)
}
