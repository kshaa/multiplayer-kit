//! WASM bindings for room connection and channels.

use super::{Channel, ConnectionConfig, RoomConnection};
use crate::Transport;
use multiplayer_kit_protocol::RoomId;
use serde::Deserialize;
use wasm_bindgen::prelude::*;

// ============================================================================
// Options
// ============================================================================

fn default_transport() -> String {
    "auto".to_string()
}

/// Options for connecting to a room.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RoomConnectOptions {
    url: String,
    ticket: String,
    room_id: u32,
    #[serde(default)]
    cert_hash: Option<String>,
    #[serde(default = "default_transport")]
    transport: String,
}

// ============================================================================
// JsRoomConnection
// ============================================================================

/// Room connection for JavaScript.
#[wasm_bindgen]
pub struct JsRoomConnection {
    inner: RoomConnection,
}

#[wasm_bindgen]
impl JsRoomConnection {
    /// Connect to a room.
    ///
    /// Options object:
    /// - `url` (required): Server URL
    /// - `ticket` (required): Auth ticket from getTicket()
    /// - `roomId` (required): Room ID to join
    /// - `certHash` (optional): Base64 SHA-256 cert hash for self-signed certs
    /// - `transport` (optional): "auto" (default), "webtransport", or "websocket"
    pub async fn connect(options: JsValue) -> Result<JsRoomConnection, JsError> {
        let opts: RoomConnectOptions = serde_wasm_bindgen::from_value(options)
            .map_err(|e| JsError::new(&format!("Invalid options: {}", e)))?;

        let transport = Transport::from_str(&opts.transport).ok_or_else(|| {
            JsError::new(&format!(
                "Invalid transport '{}'. Use 'auto', 'webtransport', or 'websocket'",
                opts.transport
            ))
        })?;

        let config = ConnectionConfig {
            transport,
            cert_hash: opts.cert_hash,
            validate_certs: false, // Browser handles TLS validation
        };

        let inner = RoomConnection::connect(
            &opts.url,
            &opts.ticket,
            RoomId(opts.room_id as u64),
            config,
        )
        .await
        .map_err(|e| JsError::new(&e.to_string()))?;

        Ok(JsRoomConnection { inner })
    }

    /// Open a new channel (persistent stream).
    #[wasm_bindgen(js_name = openChannel)]
    pub async fn open_channel(&self) -> Result<JsChannel, JsError> {
        let channel = self
            .inner
            .open_channel()
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;

        Ok(JsChannel { inner: channel })
    }

    /// Get the transport type ("webtransport" or "websocket").
    #[wasm_bindgen(getter)]
    pub fn transport(&self) -> String {
        self.inner.transport().name().to_string()
    }

    /// Get the room ID.
    #[wasm_bindgen(getter, js_name = roomId)]
    pub fn room_id(&self) -> u64 {
        self.inner.room_id().0
    }

    /// Check if connected.
    #[wasm_bindgen(js_name = isConnected)]
    pub fn is_connected(&self) -> bool {
        self.inner.is_connected()
    }
}

// Methods for Rust WASM code (not exposed to JavaScript)
impl JsRoomConnection {
    /// Open a new channel and return the raw Channel (for Rust WASM code).
    pub async fn open_channel_raw(&self) -> Result<Channel, crate::ClientError> {
        self.inner.open_channel().await
    }

    /// Consume this wrapper and return the inner RoomConnection.
    pub fn into_inner(self) -> RoomConnection {
        self.inner
    }
}

// ============================================================================
// JsChannel
// ============================================================================

/// A persistent channel for JavaScript.
#[wasm_bindgen]
pub struct JsChannel {
    inner: Channel,
}

#[wasm_bindgen]
impl JsChannel {
    /// Write raw data to the channel.
    pub async fn write(&self, data: &[u8]) -> Result<(), JsError> {
        self.inner
            .write(data)
            .await
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// Read raw data from the channel. Returns Uint8Array.
    pub async fn read(&self) -> Result<Vec<u8>, JsError> {
        let mut buf = vec![0u8; 64 * 1024];
        let n = self
            .inner
            .read(&mut buf)
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;
        buf.truncate(n);
        Ok(buf)
    }

    /// Check if connected.
    #[wasm_bindgen(js_name = isConnected)]
    pub fn is_connected(&self) -> bool {
        self.inner.is_connected()
    }

    /// Close the channel.
    pub async fn close(self) -> Result<(), JsError> {
        self.inner
            .close()
            .await
            .map_err(|e| JsError::new(&e.to_string()))
    }
}
