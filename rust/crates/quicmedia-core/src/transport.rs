//! QUIC endpoint setup and connection establishment (quinn + rustls).

use std::sync::Arc;
use std::time::Duration;

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{ClientConfig, Connection, Endpoint, ServerConfig, TransportConfig};

use crate::config::{Config, Role};
use crate::{security, Error, Result};

/// Transport tuning shared by both roles.
fn transport_config() -> Arc<TransportConfig> {
    let mut t = TransportConfig::default();
    // Generous datagram buffers for real-time media.
    t.datagram_receive_buffer_size(Some(2 * 1024 * 1024));
    t.datagram_send_buffer_size(1024 * 1024);
    // Keep NAT bindings alive and detect dead paths quickly.
    t.keep_alive_interval(Some(Duration::from_secs(5)));
    if let Ok(idle) = quinn::IdleTimeout::try_from(Duration::from_secs(30)) {
        t.max_idle_timeout(Some(idle));
    }
    Arc::new(t)
}

fn server_config(config: &Config) -> Result<ServerConfig> {
    let rustls_sc = security::build_server_config(&config.security)?;
    let qsc = QuicServerConfig::try_from(rustls_sc).map_err(|e| Error::Tls(e.to_string()))?;
    let mut sc = ServerConfig::with_crypto(Arc::new(qsc));
    sc.transport_config(transport_config());
    Ok(sc)
}

fn client_config(config: &Config) -> Result<ClientConfig> {
    let rustls_cc = security::build_client_config(&config.security)?;
    let qcc = QuicClientConfig::try_from(rustls_cc).map_err(|e| Error::Tls(e.to_string()))?;
    let mut cc = ClientConfig::new(Arc::new(qcc));
    cc.transport_config(transport_config());
    Ok(cc)
}

/// Establish the connection for this role. Returns the owned endpoint (keep it
/// alive for the connection's lifetime) and the live connection.
pub async fn establish(config: &Config) -> Result<(Endpoint, Connection)> {
    match config.role {
        Role::Client => connect_client(config).await,
        Role::Server => accept_server(config).await,
    }
}

async fn connect_client(config: &Config) -> Result<(Endpoint, Connection)> {
    let mut endpoint = Endpoint::client(config.bind_addr)?;
    endpoint.set_default_client_config(client_config(config)?);
    let peer = config
        .peer_addr
        .ok_or_else(|| Error::Config("client requires peer_addr".into()))?;
    let connecting = endpoint
        .connect(peer, &config.server_name)
        .map_err(|e| Error::Connect(e.to_string()))?;
    let conn = connecting.await.map_err(|e| Error::Connect(e.to_string()))?;
    Ok((endpoint, conn))
}

async fn accept_server(config: &Config) -> Result<(Endpoint, Connection)> {
    let endpoint = Endpoint::server(server_config(config)?, config.bind_addr)?;
    let incoming = endpoint.accept().await.ok_or(Error::Closed)?;
    let conn = incoming
        .accept()
        .map_err(|e| Error::Connection(e.to_string()))?
        .await
        .map_err(|e| Error::Connection(e.to_string()))?;
    Ok((endpoint, conn))
}
