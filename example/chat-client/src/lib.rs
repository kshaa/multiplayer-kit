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
    use multiplayer_kit_helpers::{frame_message, MessageBuffer};

    /// Re-export ApiClient from the core client library for convenience.
    pub use multiplayer_kit_client::wasm_exports::JsApiClient;

    /// Check if WebTransport is supported in the current browser.
    fn is_webtransport_supported() -> bool {
        js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("WebTransport"))
            .map(|v| !v.is_undefined())
            .unwrap_or(false)
    }

    /// Chat client for JavaScript with automatic message framing.
    /// 
    /// Uses separate state for send (channel only) and receive (buffer) to allow
    /// concurrent send/receive without RefCell conflicts.
    #[wasm_bindgen]
    pub struct JsChatClient {
        conn: RoomConnection,
        channel: multiplayer_kit_client::Channel,
        recv_state: RefCell<RecvState>,
    }

    struct RecvState {
        buffer: MessageBuffer,
        pending: Vec<Vec<u8>>,
    }

    impl JsChatClient {
        fn new(conn: RoomConnection, channel: multiplayer_kit_client::Channel) -> Self {
            Self {
                conn,
                channel,
                recv_state: RefCell::new(RecvState {
                    buffer: MessageBuffer::new(),
                    pending: Vec::new(),
                }),
            }
        }
    }

    #[wasm_bindgen]
    impl JsChatClient {
        /// Connect with options (single URL, explicit transport choice).
        /// - cert_hash: Base64 SHA-256 hash for self-signed certs (WebTransport only)
        /// - use_websocket: If true, use WebSocket; if false, use WebTransport
        #[wasm_bindgen(js_name = connectWithOptions)]
        pub async fn connect_with_options(
            url: &str,
            ticket: &str,
            room_id: u32,
            cert_hash: Option<String>,
            use_websocket: bool,
        ) -> Result<JsChatClient, JsError> {
            let transport = if use_websocket {
                multiplayer_kit_client::Transport::WebSocket
            } else {
                multiplayer_kit_client::Transport::WebTransport
            };

            let config = multiplayer_kit_client::ConnectionConfig { transport, cert_hash };

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

            Ok(JsChatClient::new(conn, channel))
        }

        /// Connect with auto-detection.
        /// Provide both URLs - uses WebTransport if supported, otherwise WebSocket.
        #[wasm_bindgen(js_name = connectAuto)]
        pub async fn connect_auto(
            webtransport_url: &str,
            websocket_url: &str,
            ticket: &str,
            room_id: u32,
            cert_hash: Option<String>,
        ) -> Result<JsChatClient, JsError> {
            let (url, transport) = if is_webtransport_supported() {
                (webtransport_url, multiplayer_kit_client::Transport::WebTransport)
            } else {
                (websocket_url, multiplayer_kit_client::Transport::WebSocket)
            };

            let config = multiplayer_kit_client::ConnectionConfig { transport, cert_hash };

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

            Ok(JsChatClient::new(conn, channel))
        }

        /// Send a text message (auto-framed).
        /// Does not borrow recv_state, so safe to call while receive is pending.
        #[wasm_bindgen(js_name = sendText)]
        pub async fn send_text(&self, msg: &str) -> Result<(), JsError> {
            let framed = frame_message(msg.as_bytes());
            self.channel
                .write(&framed)
                .await
                .map_err(|e| JsError::new(&e.to_string()))
        }

        /// Receive the next complete text message.
        /// Returns null if the connection is closed.
        #[wasm_bindgen(js_name = receiveText)]
        pub async fn receive_text(&self) -> Result<Option<String>, JsError> {
            // Check for pending messages first (quick borrow)
            {
                let mut state = self.recv_state.borrow_mut();
                if let Some(msg) = state.pending.pop() {
                    return String::from_utf8(msg)
                        .map(Some)
                        .map_err(|e| JsError::new(&format!("Invalid UTF-8: {}", e)));
                }
            }

            // Read more data - loop until we get a complete message
            loop {
                // Read from channel (no borrow held during await)
                let mut temp_buf = vec![0u8; 64 * 1024];
                let n = self.channel
                    .read(&mut temp_buf)
                    .await
                    .map_err(|e| JsError::new(&e.to_string()))?;

                if n == 0 {
                    return Ok(None); // Channel closed
                }

                // Process the data (quick borrow)
                {
                    let mut state = self.recv_state.borrow_mut();
                    let results: Vec<_> = state.buffer.push(&temp_buf[..n]).collect();
                    for result in results {
                        match result {
                            Ok(msg) => state.pending.push(msg),
                            Err(e) => return Err(JsError::new(&format!("Framing error: {}", e))),
                        }
                    }

                    if let Some(msg) = state.pending.pop() {
                        return String::from_utf8(msg)
                            .map(Some)
                            .map_err(|e| JsError::new(&format!("Invalid UTF-8: {}", e)));
                    }
                }
                // Loop to read more if no complete message yet
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
