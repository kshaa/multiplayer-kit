//! Chat client library built on multiplayer-kit-helpers.
//!
//! Wraps `RoomConnection` and `Channel` with automatic message framing for text.
//!
//! # Example
//!
//! ```ignore
//! use chat_client::ChatClient;
//!
//! let mut client = ChatClient::connect("https://127.0.0.1:4433", &ticket, room_id).await?;
//!
//! client.send_text("hello").await?;
//!
//! while let Some(msg) = client.receive_text().await? {
//!     println!("{}", msg);
//! }
//! ```

pub use multiplayer_kit_helpers::{MessageChannel, MessageChannelError};
pub use multiplayer_kit_client::{ApiClient, ClientError, RoomConnection};
pub use multiplayer_kit_protocol::RoomId;

/// Chat client errors.
#[derive(Debug, thiserror::Error)]
pub enum ChatError {
    #[error("channel error: {0}")]
    Channel(#[from] MessageChannelError),
    #[error("invalid UTF-8: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
}

/// A chat client with automatic message framing.
pub struct ChatClient {
    _conn: RoomConnection,
    channel: MessageChannel,
}

impl ChatClient {
    /// Connect to a room and open a framed message channel.
    pub async fn connect(url: &str, ticket: &str, room_id: RoomId) -> Result<Self, ClientError> {
        let conn = RoomConnection::connect(url, ticket, room_id).await?;
        let channel = conn.open_channel().await?;
        Ok(Self {
            _conn: conn,
            channel: MessageChannel::new(channel),
        })
    }

    /// Send a text message (auto-framed).
    pub async fn send_text(&self, msg: &str) -> Result<(), ClientError> {
        self.channel.send(msg.as_bytes()).await
    }

    /// Receive the next complete text message.
    pub async fn receive_text(&mut self) -> Result<Option<String>, ChatError> {
        match self.channel.recv().await? {
            Some(data) => Ok(Some(String::from_utf8(data)?)),
            None => Ok(None),
        }
    }

    /// Get a reference to the underlying MessageChannel.
    pub fn channel(&self) -> &MessageChannel {
        &self.channel
    }

    /// Get a mutable reference to the underlying MessageChannel.
    pub fn channel_mut(&mut self) -> &mut MessageChannel {
        &mut self.channel
    }
}

// ============================================================================
// WASM exports
// ============================================================================

#[cfg(feature = "wasm")]
mod wasm {
    use super::*;
    use wasm_bindgen::prelude::*;
    use std::cell::RefCell;

    /// Re-export ApiClient from the core client library for convenience.
    pub use multiplayer_kit_client::wasm_exports::JsApiClient;

    /// Chat client for JavaScript with automatic message framing.
    #[wasm_bindgen]
    pub struct JsChatClient {
        conn: RoomConnection,
        channel: RefCell<MessageChannel>,
    }

    #[wasm_bindgen]
    impl JsChatClient {
        /// Connect to a chat room.
        pub async fn connect(
            url: &str,
            ticket: &str,
            room_id: u32,
        ) -> Result<JsChatClient, JsError> {
            Self::connect_with_options(url, ticket, room_id, None, false).await
        }

        /// Connect with options.
        /// - cert_hash: Base64 SHA-256 hash for self-signed certs (WebTransport only)
        /// - use_websocket: If true, use WebSocket instead of WebTransport
        #[wasm_bindgen(js_name = connectWithOptions)]
        pub async fn connect_with_options(
            url: &str,
            ticket: &str,
            room_id: u32,
            cert_hash: Option<String>,
            use_websocket: bool,
        ) -> Result<JsChatClient, JsError> {
            let config = multiplayer_kit_client::ConnectionConfig {
                transport: if use_websocket {
                    multiplayer_kit_client::Transport::WebSocket
                } else {
                    multiplayer_kit_client::Transport::WebTransport
                },
                cert_hash,
            };

            let conn = RoomConnection::connect_with_config(
                url,
                ticket,
                RoomId(room_id as u64),
                config,
            )
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;

            let channel = conn
                .open_channel()
                .await
                .map_err(|e| JsError::new(&e.to_string()))?;

            Ok(JsChatClient {
                conn,
                channel: RefCell::new(MessageChannel::new(channel)),
            })
        }

        /// Send a text message (auto-framed).
        #[wasm_bindgen(js_name = sendText)]
        pub async fn send_text(&self, msg: &str) -> Result<(), JsError> {
            self.channel
                .borrow()
                .send(msg.as_bytes())
                .await
                .map_err(|e| JsError::new(&e.to_string()))
        }

        /// Receive the next complete text message.
        /// Returns null if the connection is closed.
        #[wasm_bindgen(js_name = receiveText)]
        pub async fn receive_text(&self) -> Result<Option<String>, JsError> {
            match self.channel.borrow_mut().recv().await {
                Ok(Some(data)) => {
                    String::from_utf8(data)
                        .map(Some)
                        .map_err(|e| JsError::new(&format!("Invalid UTF-8: {}", e)))
                }
                Ok(None) => Ok(None),
                Err(e) => Err(JsError::new(&e.to_string())),
            }
        }

        /// Check if connected.
        #[wasm_bindgen(js_name = isConnected)]
        pub fn is_connected(&self) -> bool {
            self.conn.is_connected()
        }
    }
}

#[cfg(feature = "wasm")]
pub use wasm::*;
