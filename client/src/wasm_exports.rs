//! WASM bindings for JavaScript/TypeScript consumption.
//!
//! This module is only available when compiling for WASM.

use crate::ConnectionState;
use multiplayer_kit_protocol::RoomId;
use wasm_bindgen::prelude::*;

// ============================================================================
// Helpers
// ============================================================================

/// Check if WebTransport is supported in the current browser.
fn is_webtransport_supported() -> bool {
    js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("WebTransport"))
        .map(|v| !v.is_undefined())
        .unwrap_or(false)
}

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

    /// Create a new room with config. Returns { room_id: number }.
    /// `config` should be a JS object like `{ name: "My Room" }`.
    #[wasm_bindgen(js_name = createRoom)]
    pub async fn create_room(&self, ticket: &str, config: JsValue) -> Result<JsValue, JsError> {
        let config: serde_json::Value = serde_wasm_bindgen::from_value(config)
            .map_err(|e| JsError::new(&format!("Invalid config: {}", e)))?;

        let resp = self
            .inner
            .create_room(ticket, &config)
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

    /// Quickplay - find or create a room.
    /// Returns { room_id: number, created: boolean }.
    /// `filter` is optional game-specific criteria (JS object or null).
    #[wasm_bindgen]
    pub async fn quickplay(&self, ticket: &str, filter: JsValue) -> Result<JsValue, JsError> {
        let filter_opt: Option<serde_json::Value> = if filter.is_null() || filter.is_undefined() {
            None
        } else {
            Some(
                serde_wasm_bindgen::from_value(filter)
                    .map_err(|e| JsError::new(&format!("Invalid filter: {}", e)))?,
            )
        };

        let resp = self
            .inner
            .quickplay(ticket, filter_opt.as_ref())
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;

        serde_wasm_bindgen::to_value(&resp).map_err(|e| JsError::new(&e.to_string()))
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
    /// Connect to a room with auto-detection.
    /// Automatically picks WebTransport if supported, otherwise WebSocket.
    #[wasm_bindgen(js_name = connectAuto)]
    pub async fn connect_auto(
        wt_url: &str,
        ws_url: &str,
        ticket: &str,
        room_id: u32,
        cert_hash: Option<String>,
    ) -> Result<JsRoomConnection, JsError> {
        let (url, use_websocket) = if is_webtransport_supported() {
            (wt_url, false)
        } else {
            (ws_url, true)
        };
        Self::connect_with_options(url, ticket, room_id, cert_hash, Some(use_websocket)).await
    }

    /// Connect to a room with a single URL (auto-detects transport if use_websocket is undefined).
    pub async fn connect(
        url: &str,
        ticket: &str,
        room_id: u32,
    ) -> Result<JsRoomConnection, JsError> {
        Self::connect_with_options(url, ticket, room_id, None, None).await
    }

    /// Connect to a room with options.
    /// - cert_hash: Base64 SHA-256 hash for self-signed certs (WebTransport only)
    /// - use_websocket: If true, use WebSocket; if false, use WebTransport; if undefined, auto-detect
    #[wasm_bindgen(js_name = connectWithOptions)]
    pub async fn connect_with_options(
        url: &str,
        ticket: &str,
        room_id: u32,
        cert_hash: Option<String>,
        use_websocket: Option<bool>,
    ) -> Result<JsRoomConnection, JsError> {
        let transport = match use_websocket {
            Some(true) => crate::Transport::WebSocket,
            Some(false) => crate::Transport::WebTransport,
            None => {
                // Auto-detect: check if WebTransport is available
                if is_webtransport_supported() {
                    crate::Transport::WebTransport
                } else {
                    crate::Transport::WebSocket
                }
            }
        };

        let config = crate::ConnectionConfig {
            transport,
            cert_hash,
        };

        let inner =
            crate::RoomConnection::connect_with_config(url, ticket, RoomId(room_id as u64), config)
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
