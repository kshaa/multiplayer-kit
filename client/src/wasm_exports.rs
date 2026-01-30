//! WASM bindings for JavaScript/TypeScript consumption.
//!
//! This module is only available when compiling for WASM.

use crate::ConnectionState;
use multiplayer_kit_protocol::RoomId;
use wasm_bindgen::prelude::*;

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
        let inner = crate::LobbyClient::connect(url, ticket)
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
        let inner = crate::RoomClient::connect(url, ticket, RoomId(room_id))
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
    pub async fn recv(&mut self) -> Result<Vec<u8>, JsError> {
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
