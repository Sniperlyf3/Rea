//! Reliable, ordered generic data channel over a single QUIC bidirectional stream.
//!
//! Messages are length-delimited: a 4-byte big-endian length prefix followed by
//! that many payload bytes. This runs alongside the unreliable media datagrams
//! on the same QUIC connection, so control messages never block or reorder audio.

use quinn::{RecvStream, SendStream};
use tokio::sync::mpsc;

use crate::Events;
use std::sync::Arc;

/// Maximum accepted message size (16 MiB) — guards against a hostile length prefix.
pub const MAX_MESSAGE: usize = 16 * 1024 * 1024;

/// Drains `outgoing` and writes each message to the stream, framed.
pub async fn writer_task(mut send: SendStream, mut outgoing: mpsc::Receiver<Vec<u8>>) {
    while let Some(msg) = outgoing.recv().await {
        if msg.len() > MAX_MESSAGE {
            tracing::warn!("dropping oversized data-channel message ({} bytes)", msg.len());
            continue;
        }
        let len = (msg.len() as u32).to_be_bytes();
        if send.write_all(&len).await.is_err() || send.write_all(&msg).await.is_err() {
            break;
        }
    }
    let _ = send.finish();
}

/// Reads framed messages from the stream and delivers them via `events.on_data`.
pub async fn reader_task(mut recv: RecvStream, events: Arc<dyn Events>) {
    let mut len_buf = [0u8; 4];
    loop {
        if recv.read_exact(&mut len_buf).await.is_err() {
            break;
        }
        let len = u32::from_be_bytes(len_buf) as usize;
        if len > MAX_MESSAGE {
            tracing::warn!("peer announced oversized message ({len} bytes); closing data channel");
            break;
        }
        let mut buf = vec![0u8; len];
        if recv.read_exact(&mut buf).await.is_err() {
            break;
        }
        events.on_data(&buf);
    }
}
