//! WASM bindings for JavaScript/TypeScript consumption.

#![cfg(feature = "wasm")]

use wasm_bindgen::prelude::*;
use multiplayer_kit_protocol::RoomId;

/// Lobby client for JavaScript.
#[wasm_bindgen]
pub struct JsLobbyClient {
    inner: crate::LobbyClient,
}

#[wasm_bindgen]
impl JsLobbyClient {
    /// Connect to the lobby.
    #[wasm_bindgen(constructor)]
    pub async fn connect(url: &str, ticket: &str) -> Result<JsLobbyClient, JsError> {
        let inner = crate::LobbyClient::connect(url, ticket)
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;
        Ok(Self { inner })
    }

    /// Receive the next lobby event as a JS object.
    pub async fn recv(&mut self) -> Result<JsValue, JsError> {
        let event = self.inner.recv()
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;
        serde_wasm_bindgen::to_value(&event)
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// Close the connection.
    pub async fn close(self) -> Result<(), JsError> {
        self.inner.close()
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
    /// Connect to a room.
    #[wasm_bindgen(constructor)]
    pub async fn connect(url: &str, ticket: &str, room_id: u64) -> Result<JsRoomClient, JsError> {
        let inner = crate::RoomClient::connect(url, ticket, RoomId(room_id))
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;
        Ok(Self { inner })
    }

    /// Send a message (as Uint8Array).
    pub async fn send(&self, payload: &[u8]) -> Result<(), JsError> {
        self.inner.send(payload)
            .await
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// Receive the next message (as Uint8Array).
    pub async fn recv(&mut self) -> Result<Vec<u8>, JsError> {
        self.inner.recv()
            .await
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// Close the connection.
    pub async fn close(self) -> Result<(), JsError> {
        self.inner.close()
            .await
            .map_err(|e| JsError::new(&e.to_string()))
    }
}
