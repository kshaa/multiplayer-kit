pub mod api;
pub mod error;
pub mod lobby;
pub mod room;
pub mod transport;

// ============================================================================
// Transport type (shared by room and lobby)
// ============================================================================

/// Transport protocol for connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Transport {
    /// Auto-detect: try WebTransport first, fall back to WebSocket.
    #[default]
    Auto,
    /// WebTransport (QUIC-based, preferred).
    WebTransport,
    /// WebSocket (fallback for environments without WebTransport).
    WebSocket,
}

impl Transport {
    /// Get the transport name as a string.
    pub fn name(&self) -> &'static str {
        match self {
            Transport::Auto => "auto",
            Transport::WebTransport => "webtransport",
            Transport::WebSocket => "websocket",
        }
    }

    /// Parse from string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(Transport::Auto),
            "webtransport" => Some(Transport::WebTransport),
            "websocket" => Some(Transport::WebSocket),
            _ => None,
        }
    }
}

/// Check if WebTransport is supported on this platform.
///
/// - Native: always returns `true`
/// - WASM: checks if `WebTransport` is available in the browser
pub fn is_webtransport_supported() -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        true
    }

    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Reflect::get(&js_sys::global(), &wasm_bindgen::JsValue::from_str("WebTransport"))
            .map(|v| !v.is_undefined())
            .unwrap_or(false)
    }
}

// ============================================================================
// Re-exports
// ============================================================================

pub use api::{ApiClient, CertHashResponse, CreateRoomResponse, TicketResponse};
pub use error::{
    ClientError, ConnectionError, ConnectionState, DisconnectReason, ReceiveError, SendError,
};
pub use lobby::{LobbyClient, LobbyConfig};
pub use room::{Channel, ChannelIO, ConnectionConfig, RoomConnection, RoomConnectionLike};

// WASM re-exports
#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
pub use api::JsApiClient;
#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
pub use error::JsConnectionState;
#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
pub use lobby::JsLobbyClient;
#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
pub use room::{JsChannel, JsRoomConnection};
