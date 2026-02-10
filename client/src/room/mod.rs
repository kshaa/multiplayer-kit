//! Room connection and channel management.
//!
//! A `RoomConnection` represents an authenticated connection to a room.
//! From it, you can open multiple `Channel`s - each is a persistent
//! bidirectional byte stream.
//!
//! # Transport
//! - WebTransport: Each channel is a QUIC bidirectional stream
//! - WebSocket: Each channel is a separate WebSocket connection

mod channel;
mod connection;

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
mod wasm;

pub use channel::Channel;
pub use connection::RoomConnection;

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
pub use wasm::{JsChannel, JsRoomConnection};

use crate::{ClientError, Transport};
use std::future::Future;

// ============================================================================
// Traits
// ============================================================================

/// Trait for channel I/O operations.
pub trait ChannelIO {
    /// Read bytes from the channel into buffer. Returns number of bytes read.
    fn read(&self, buf: &mut [u8]) -> impl Future<Output = Result<usize, ClientError>>;

    /// Write bytes to the channel.
    fn write(&self, data: &[u8]) -> impl Future<Output = Result<(), ClientError>>;

    /// Check if the channel is still connected.
    fn is_connected(&self) -> bool;
}

/// Trait for room connections.
pub trait RoomConnectionLike {
    /// The channel type returned by `open_channel`.
    type Channel: ChannelIO;

    /// Open a new channel (persistent bidirectional stream).
    fn open_channel(&self) -> impl Future<Output = Result<Self::Channel, ClientError>>;
}

// ============================================================================
// Config
// ============================================================================

/// Connection configuration.
#[derive(Debug, Clone, Default)]
pub struct ConnectionConfig {
    /// Preferred transport (will fallback if unavailable).
    pub transport: Transport,
    /// Base64-encoded SHA-256 cert hash for self-signed certs (WebTransport only).
    pub cert_hash: Option<String>,
    /// Whether to validate TLS certificates (native only, ignored if cert_hash is set).
    pub validate_certs: bool,
}

impl ConnectionConfig {
    /// Production config with real TLS certs (uses system CA store).
    pub fn production() -> Self {
        Self {
            transport: Transport::WebTransport,
            cert_hash: None,
            validate_certs: true,
        }
    }
}
