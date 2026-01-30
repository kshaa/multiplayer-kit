//! WASM bindings for JavaScript/TypeScript consumption.
//!
//! This module is only available when the `wasm` feature is enabled.

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
mod inner {
    use multiplayer_kit_protocol::RoomId;
    use wasm_bindgen::prelude::*;

    /// Lobby client for JavaScript.
    #[wasm_bindgen]
    pub struct JsLobbyClient {
        // TODO: Implement with xwt-web-sys
    }

    #[wasm_bindgen]
    impl JsLobbyClient {
        /// Connect to the lobby.
        #[wasm_bindgen(constructor)]
        pub async fn connect(_url: &str, _ticket: &str) -> Result<JsLobbyClient, JsError> {
            // TODO: Implement with xwt-web-sys
            Err(JsError::new("WASM lobby client not yet implemented"))
        }

        /// Receive the next lobby event as a JS object.
        pub async fn recv(&mut self) -> Result<JsValue, JsError> {
            Err(JsError::new("WASM lobby client not yet implemented"))
        }

        /// Close the connection.
        pub async fn close(self) -> Result<(), JsError> {
            Ok(())
        }
    }

    /// Room client for JavaScript.
    #[wasm_bindgen]
    pub struct JsRoomClient {
        // TODO: Implement with xwt-web-sys
        _room_id: u64,
    }

    #[wasm_bindgen]
    impl JsRoomClient {
        /// Connect to a room.
        #[wasm_bindgen(constructor)]
        pub async fn connect(
            _url: &str,
            _ticket: &str,
            room_id: u64,
        ) -> Result<JsRoomClient, JsError> {
            // TODO: Implement with xwt-web-sys
            Ok(Self { _room_id: room_id })
        }

        /// Send a message (as Uint8Array).
        pub async fn send(&self, _payload: &[u8]) -> Result<(), JsError> {
            Err(JsError::new("WASM room client not yet implemented"))
        }

        /// Receive the next message (as Uint8Array).
        pub async fn recv(&mut self) -> Result<Vec<u8>, JsError> {
            Err(JsError::new("WASM room client not yet implemented"))
        }

        /// Close the connection.
        pub async fn close(self) -> Result<(), JsError> {
            Ok(())
        }
    }
}

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
pub use inner::*;
