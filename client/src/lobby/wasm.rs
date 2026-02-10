//! WASM bindings for lobby client.

use super::{LobbyClient, LobbyConfig};
use crate::error::JsConnectionState;
use crate::Transport;
use serde::Deserialize;
use wasm_bindgen::prelude::*;

// ============================================================================
// Options
// ============================================================================

fn default_transport() -> String {
    "auto".to_string()
}

/// Options for connecting to the lobby.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LobbyConnectOptions {
    url: String,
    ticket: String,
    #[serde(default)]
    cert_hash: Option<String>,
    #[serde(default = "default_transport")]
    transport: String,
}

// ============================================================================
// JsLobbyClient
// ============================================================================

/// Lobby client for JavaScript.
#[wasm_bindgen]
pub struct JsLobbyClient {
    inner: std::rc::Rc<std::cell::RefCell<LobbyClient>>,
}

#[wasm_bindgen]
impl JsLobbyClient {
    /// Connect to the lobby.
    ///
    /// Options object:
    /// - `url` (required): Server URL
    /// - `ticket` (required): Auth ticket from getTicket()
    /// - `certHash` (optional): Base64 SHA-256 cert hash for self-signed certs
    /// - `transport` (optional): "auto" (default), "webtransport", or "websocket"
    pub async fn connect(options: JsValue) -> Result<JsLobbyClient, JsError> {
        let opts: LobbyConnectOptions = serde_wasm_bindgen::from_value(options)
            .map_err(|e| JsError::new(&format!("Invalid options: {}", e)))?;

        let transport = Transport::from_str(&opts.transport).ok_or_else(|| {
            JsError::new(&format!(
                "Invalid transport '{}'. Use 'auto', 'webtransport', or 'websocket'",
                opts.transport
            ))
        })?;

        let config = LobbyConfig {
            transport,
            cert_hash: opts.cert_hash,
        };

        let inner = LobbyClient::connect(&opts.url, &opts.ticket, config)
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;

        Ok(Self {
            inner: std::rc::Rc::new(std::cell::RefCell::new(inner)),
        })
    }

    /// Get the transport type ("webtransport" or "websocket").
    #[wasm_bindgen(getter)]
    pub fn transport(&self) -> String {
        self.inner.borrow().transport().name().to_string()
    }

    /// Get the connection state.
    #[wasm_bindgen(js_name = getState)]
    pub fn state(&self) -> JsConnectionState {
        self.inner.borrow().state().into()
    }

    /// Receive the next lobby event.
    pub async fn recv(&self) -> Result<JsValue, JsError> {
        let event = self
            .inner
            .borrow_mut()
            .recv()
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;
        serde_wasm_bindgen::to_value(&event).map_err(|e| JsError::new(&e.to_string()))
    }

    /// Close the lobby connection.
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
