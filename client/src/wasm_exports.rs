//! WASM bindings for JavaScript/TypeScript consumption.
//!
//! This module is only available when compiling for WASM.

use crate::ConnectionState;
use multiplayer_kit_protocol::RoomId;
use wasm_bindgen::prelude::*;

// ============================================================================
// JsApiClient
// ============================================================================

/// HTTP API client for JavaScript.
#[wasm_bindgen]
pub struct JsApiClient {
    inner: crate::ApiClient,
}

#[wasm_bindgen]
impl JsApiClient {
    /// Create a new API client. Example: `new JsApiClient("http://127.0.0.1:8080")`
    #[wasm_bindgen(constructor)]
    pub fn new(base_url: &str) -> JsApiClient {
        JsApiClient {
            inner: crate::ApiClient::new(base_url),
        }
    }

    /// Request a ticket from the server. Pass your auth payload as a JS object.
    #[wasm_bindgen(js_name = getTicket)]
    pub async fn get_ticket(&self, auth_payload: JsValue) -> Result<JsValue, JsError> {
        let payload: serde_json::Value = serde_wasm_bindgen::from_value(auth_payload)
            .map_err(|e| JsError::new(&e.to_string()))?;

        let resp = self
            .inner
            .get_ticket(&payload)
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;

        serde_wasm_bindgen::to_value(&resp).map_err(|e| JsError::new(&e.to_string()))
    }

    /// Get the server's certificate hash (base64 SHA-256) for self-signed certs.
    #[wasm_bindgen(js_name = getCertHash)]
    pub async fn get_cert_hash(&self) -> Result<Option<String>, JsError> {
        self.inner
            .get_cert_hash()
            .await
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// List all available rooms.
    #[wasm_bindgen(js_name = listRooms)]
    pub async fn list_rooms(&self) -> Result<JsValue, JsError> {
        let rooms = self
            .inner
            .list_rooms()
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;

        serde_wasm_bindgen::to_value(&rooms).map_err(|e| JsError::new(&e.to_string()))
    }

    /// Create a new room. Returns { room_id: number }.
    #[wasm_bindgen(js_name = createRoom)]
    pub async fn create_room(&self, ticket: &str) -> Result<JsValue, JsError> {
        let resp = self
            .inner
            .create_room(ticket)
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;

        serde_wasm_bindgen::to_value(&resp).map_err(|e| JsError::new(&e.to_string()))
    }

    /// Delete a room.
    #[wasm_bindgen(js_name = deleteRoom)]
    pub async fn delete_room(&self, ticket: &str, room_id: u32) -> Result<(), JsError> {
        self.inner
            .delete_room(ticket, RoomId(room_id as u64))
            .await
            .map_err(|e| JsError::new(&e.to_string()))
    }
}

// ============================================================================
// JsConnectionState
// ============================================================================

/// Connection state as exposed to JavaScript.
#[wasm_bindgen]
#[derive(Debug, Clone)]
pub struct JsConnectionState {
    state: String,
    reason: Option<String>,
}

#[wasm_bindgen]
impl JsConnectionState {
    #[wasm_bindgen(getter)]
    pub fn state(&self) -> String {
        self.state.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn reason(&self) -> Option<String> {
        self.reason.clone()
    }

    #[wasm_bindgen(js_name = isConnected)]
    pub fn is_connected(&self) -> bool {
        self.state == "connected"
    }
}

impl From<ConnectionState> for JsConnectionState {
    fn from(state: ConnectionState) -> Self {
        match state {
            ConnectionState::Disconnected => JsConnectionState {
                state: "disconnected".to_string(),
                reason: None,
            },
            ConnectionState::Connecting => JsConnectionState {
                state: "connecting".to_string(),
                reason: None,
            },
            ConnectionState::Connected => JsConnectionState {
                state: "connected".to_string(),
                reason: None,
            },
            ConnectionState::Lost(reason) => JsConnectionState {
                state: "lost".to_string(),
                reason: Some(reason.to_string()),
            },
        }
    }
}

// ============================================================================
// JsRoomConnection
// ============================================================================

/// Room connection for JavaScript.
#[wasm_bindgen]
pub struct JsRoomConnection {
    inner: crate::RoomConnection,
}

#[wasm_bindgen]
impl JsRoomConnection {
    /// Connect to a room using WebTransport.
    pub async fn connect(
        url: &str,
        ticket: &str,
        room_id: u32,
    ) -> Result<JsRoomConnection, JsError> {
        Self::connect_with_options(url, ticket, room_id, None, false).await
    }

    /// Connect to a room with options.
    /// - cert_hash: Base64 SHA-256 hash for self-signed certs (WebTransport only)
    /// - use_websocket: If true, use WebSocket instead of WebTransport
    #[wasm_bindgen(js_name = connectWithOptions)]
    pub async fn connect_with_options(
        url: &str,
        ticket: &str,
        room_id: u32,
        cert_hash: Option<String>,
        use_websocket: bool,
    ) -> Result<JsRoomConnection, JsError> {
        let config = crate::ConnectionConfig {
            transport: if use_websocket {
                crate::Transport::WebSocket
            } else {
                crate::Transport::WebTransport
            },
            cert_hash,
        };

        let inner = crate::RoomConnection::connect_with_config(url, ticket, RoomId(room_id as u64), config)
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
        match self.inner.transport() {
            crate::Transport::WebTransport => "webtransport".to_string(),
            crate::Transport::WebSocket => "websocket".to_string(),
        }
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

// ============================================================================
// JsChannel
// ============================================================================

/// A persistent channel for JavaScript.
#[wasm_bindgen]
pub struct JsChannel {
    inner: crate::Channel,
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

// ============================================================================
// Lobby
// ============================================================================

/// Lobby client for JavaScript.
#[wasm_bindgen]
pub struct JsLobbyClient {
    inner: std::rc::Rc<std::cell::RefCell<crate::LobbyClient>>,
}

#[wasm_bindgen]
impl JsLobbyClient {
    pub async fn connect(url: &str, ticket: &str) -> Result<JsLobbyClient, JsError> {
        Self::connect_with_cert(url, ticket, None).await
    }

    #[wasm_bindgen(js_name = connectWithCert)]
    pub async fn connect_with_cert(
        url: &str,
        ticket: &str,
        cert_hash_base64: Option<String>,
    ) -> Result<JsLobbyClient, JsError> {
        let inner =
            crate::LobbyClient::connect_with_options(url, ticket, cert_hash_base64.as_deref())
                .await
                .map_err(|e| JsError::new(&e.to_string()))?;
        Ok(Self {
            inner: std::rc::Rc::new(std::cell::RefCell::new(inner)),
        })
    }

    #[wasm_bindgen(js_name = getState)]
    pub fn state(&self) -> JsConnectionState {
        self.inner.borrow().state().into()
    }

    pub async fn recv(&self) -> Result<JsValue, JsError> {
        let event = self
            .inner
            .borrow_mut()
            .recv()
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;
        serde_wasm_bindgen::to_value(&event).map_err(|e| JsError::new(&e.to_string()))
    }

    pub async fn close(self) -> Result<(), JsError> {
        let inner = std::rc::Rc::try_unwrap(self.inner)
            .map_err(|_| JsError::new("Cannot close: lobby client has multiple references"))?
            .into_inner();
        inner
            .close()
            .await
            .map_err(|e| JsError::new(&e.to_string()))
    }
}

// ============================================================================
// Deprecated - keep for backwards compat
// ============================================================================

/// DEPRECATED: Use JsRoomConnection + JsChannel instead.
#[wasm_bindgen]
#[deprecated(note = "Use JsRoomConnection and JsChannel instead")]
pub struct JsRoomClient {
    connection: Option<crate::RoomConnection>,
    channel: Option<crate::Channel>,
}

#[wasm_bindgen]
#[allow(deprecated)]
impl JsRoomClient {
    pub async fn connect(url: &str, ticket: &str, room_id: u32) -> Result<JsRoomClient, JsError> {
        Self::connect_with_cert(url, ticket, room_id, None).await
    }

    #[wasm_bindgen(js_name = connectWithCert)]
    pub async fn connect_with_cert(
        url: &str,
        ticket: &str,
        room_id: u32,
        cert_hash_base64: Option<String>,
    ) -> Result<JsRoomClient, JsError> {
        let config = crate::ConnectionConfig {
            transport: crate::Transport::WebTransport,
            cert_hash: cert_hash_base64,
        };

        let conn = crate::RoomConnection::connect_with_config(url, ticket, RoomId(room_id as u64), config)
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;

        let channel = conn
            .open_channel()
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;

        Ok(JsRoomClient {
            connection: Some(conn),
            channel: Some(channel),
        })
    }

    #[wasm_bindgen(js_name = getState)]
    pub fn state(&self) -> JsConnectionState {
        if self.channel.as_ref().map(|c| c.is_connected()).unwrap_or(false) {
            JsConnectionState {
                state: "connected".to_string(),
                reason: None,
            }
        } else {
            JsConnectionState {
                state: "disconnected".to_string(),
                reason: None,
            }
        }
    }

    pub async fn send(&self, payload: &[u8]) -> Result<(), JsError> {
        let channel = self.channel.as_ref().ok_or_else(|| JsError::new("Not connected"))?;
        channel
            .write(payload)
            .await
            .map_err(|e| JsError::new(&e.to_string()))
    }

    pub async fn recv(&self) -> Result<Vec<u8>, JsError> {
        let channel = self.channel.as_ref().ok_or_else(|| JsError::new("Not connected"))?;
        let mut buf = vec![0u8; 64 * 1024];
        let n = channel
            .read(&mut buf)
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;
        buf.truncate(n);
        Ok(buf)
    }

    pub async fn close(self) -> Result<(), JsError> {
        if let Some(channel) = self.channel {
            channel.close().await.map_err(|e| JsError::new(&e.to_string()))?;
        }
        Ok(())
    }
}
