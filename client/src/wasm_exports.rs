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
        // Convert JsValue to serde_json::Value
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
    /// Returns null if not available.
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
    pub async fn delete_room(&self, ticket: &str, room_id: u64) -> Result<(), JsError> {
        self.inner
            .delete_room(ticket, RoomId(room_id))
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
    /// Get the state name: "disconnected", "connecting", "connected", or "lost"
    #[wasm_bindgen(getter)]
    pub fn state(&self) -> String {
        self.state.clone()
    }

    /// Get the disconnect reason if state is "lost"
    #[wasm_bindgen(getter)]
    pub fn reason(&self) -> Option<String> {
        self.reason.clone()
    }

    /// Returns true if connected
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

/// Lobby client for JavaScript.
#[wasm_bindgen]
pub struct JsLobbyClient {
    inner: crate::LobbyClient,
}

#[wasm_bindgen]
impl JsLobbyClient {
    /// Connect to the lobby. Use `JsLobbyClient.connect(url, ticket)` instead of `new`.
    pub async fn connect(url: &str, ticket: &str) -> Result<JsLobbyClient, JsError> {
        Self::connect_with_cert(url, ticket, None).await
    }

    /// Connect to the lobby with an optional certificate hash (base64-encoded SHA-256).
    /// Use this for self-signed certificates in development.
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
        Ok(Self { inner })
    }

    /// Get the current connection state.
    #[wasm_bindgen(js_name = getState)]
    pub fn state(&self) -> JsConnectionState {
        self.inner.state().into()
    }

    /// Receive the next lobby event as a JS object.
    pub async fn recv(&mut self) -> Result<JsValue, JsError> {
        let event = self
            .inner
            .recv()
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;
        serde_wasm_bindgen::to_value(&event).map_err(|e| JsError::new(&e.to_string()))
    }

    /// Close the connection.
    pub async fn close(self) -> Result<(), JsError> {
        self.inner
            .close()
            .await
            .map_err(|e| JsError::new(&e.to_string()))
    }
}

/// Room client for JavaScript.
#[wasm_bindgen]
pub struct JsRoomClient {
    inner: crate::RoomClient,
}

#[wasm_bindgen]
impl JsRoomClient {
    /// Connect to a room. Use `JsRoomClient.connect(url, ticket, roomId)` instead of `new`.
    pub async fn connect(url: &str, ticket: &str, room_id: u64) -> Result<JsRoomClient, JsError> {
        Self::connect_with_cert(url, ticket, room_id, None).await
    }

    /// Connect to a room with an optional certificate hash (base64-encoded SHA-256).
    /// Use this for self-signed certificates in development.
    #[wasm_bindgen(js_name = connectWithCert)]
    pub async fn connect_with_cert(
        url: &str,
        ticket: &str,
        room_id: u64,
        cert_hash_base64: Option<String>,
    ) -> Result<JsRoomClient, JsError> {
        let inner = crate::RoomClient::connect_with_options(
            url,
            ticket,
            RoomId(room_id),
            cert_hash_base64.as_deref(),
        )
        .await
        .map_err(|e| JsError::new(&e.to_string()))?;
        Ok(Self { inner })
    }

    /// Get the current connection state.
    #[wasm_bindgen(js_name = getState)]
    pub fn state(&self) -> JsConnectionState {
        self.inner.state().into()
    }

    /// Send a message (as Uint8Array).
    pub async fn send(&self, payload: &[u8]) -> Result<(), JsError> {
        self.inner
            .send(payload)
            .await
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// Receive the next message (as Uint8Array).
    pub async fn recv(&self) -> Result<Vec<u8>, JsError> {
        self.inner
            .recv()
            .await
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// Close the connection.
    pub async fn close(self) -> Result<(), JsError> {
        self.inner
            .close()
            .await
            .map_err(|e| JsError::new(&e.to_string()))
    }
}
