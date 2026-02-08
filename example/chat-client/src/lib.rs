//! Chat client library built on multiplayer-kit typed actors.
//!
//! Provides both:
//! - Simple imperative API for CLI (`ChatClient::connect`, `send_text`, `receive_text`)
//! - Typed actor API for more control (`with_typed_client_actor`)
//!
//! # Example (Simple)
//!
//! ```ignore
//! use chat_client::ChatClient;
//!
//! let mut client = ChatClient::connect("https://127.0.0.1:8080", &ticket, room_id).await?;
//!
//! client.send_text("hello").await?;
//!
//! while let Some(msg) = client.receive_text().await? {
//!     println!("{}", msg);
//! }
//! ```
//!
//! # Example (Typed Actor)
//!
//! ```ignore
//! use chat_client::{run_chat_client, ChatProtocol, ChatEvent};
//! use multiplayer_kit_helpers::{TypedClientContext, TypedClientEvent};
//!
//! run_chat_client(conn, my_handler).await;
//!
//! async fn my_handler(ctx: TypedClientContext<ChatProtocol>, event: TypedClientEvent<ChatProtocol>) {
//!     match event {
//!         TypedClientEvent::Message(ChatEvent::Chat(msg)) => { ... }
//!         _ => {}
//!     }
//! }
//! ```

pub use chat_protocol::{ChatChannel, ChatEvent, ChatMessage, ChatProtocol, ChatUser};
pub use multiplayer_kit_client::{ApiClient, ClientError, RoomConnection};
pub use multiplayer_kit_protocol::RoomId;

#[cfg(not(target_arch = "wasm32"))]
pub use multiplayer_kit_helpers::{
    with_typed_client_actor, TypedClientContext, TypedClientEvent, TypedProtocol,
};

use chat_protocol::{DecodeError, EncodeError};
use multiplayer_kit_helpers::{frame_message, MessageBuffer};

/// Chat client errors.
#[derive(Debug, thiserror::Error)]
pub enum ChatError {
    #[error("client error: {0}")]
    Client(#[from] ClientError),
    #[error("encode error: {0}")]
    Encode(#[from] EncodeError),
    #[error("decode error: {0}")]
    Decode(#[from] DecodeError),
    #[error("channel closed")]
    ChannelClosed,
    #[error("not connected")]
    NotConnected,
}

// Native-only ChatClient implementation
#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use tokio::sync::mpsc;

    /// Simple chat client with text-based API.
    ///
    /// Uses the typed protocol internally but exposes a simple send/receive text interface.
    pub struct ChatClient {
        /// Username for this client.
        username: String,
        /// Send messages to the writer task.
        write_tx: mpsc::Sender<ChatMessage>,
        /// Receive messages from the reader task.
        read_rx: mpsc::Receiver<ChatMessage>,
    }

    impl ChatClient {
        /// Connect to a room.
        pub async fn connect(
            url: &str,
            ticket: &str,
            room_id: RoomId,
            username: String,
        ) -> Result<Self, ChatError> {
            let conn = RoomConnection::connect(url, ticket, room_id).await?;
            Self::from_connection(conn, username).await
        }

        /// Create from an existing connection.
        pub async fn from_connection(
            conn: RoomConnection,
            username: String,
        ) -> Result<Self, ChatError> {
            let channel = conn.open_channel().await?;
            let channel = std::sync::Arc::new(channel);

            // Create channels for communication with the I/O tasks
            let (write_tx, mut write_rx) = mpsc::channel::<ChatMessage>(256);
            let (read_tx, read_rx) = mpsc::channel::<ChatMessage>(256);

            // Send channel identification (Chat channel = 0)
            let channel_id_msg = frame_message(&[ChatProtocol::channel_to_id(ChatChannel::Chat)]);
            channel.write(&channel_id_msg).await?;

            // Spawn writer task
            let write_channel = std::sync::Arc::clone(&channel);
            tokio::spawn(async move {
                while let Some(msg) = write_rx.recv().await {
                    let data = match bincode::serialize(&msg) {
                        Ok(d) => d,
                        Err(_) => continue,
                    };
                    let framed = frame_message(&data);
                    if write_channel.write(&framed).await.is_err() {
                        break;
                    }
                }
            });

            // Spawn reader task
            let read_channel = std::sync::Arc::clone(&channel);
            tokio::spawn(async move {
                let mut buffer = MessageBuffer::new();
                let mut buf = vec![0u8; 64 * 1024];

                loop {
                    let n = match read_channel.read(&mut buf).await {
                        Ok(n) if n > 0 => n,
                        _ => break,
                    };

                    for result in buffer.push(&buf[..n]) {
                        if let Ok(data) = result {
                            if let Ok(msg) = bincode::deserialize::<ChatMessage>(&data) {
                                if read_tx.send(msg).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                }
            });

            Ok(Self {
                username,
                write_tx,
                read_rx,
            })
        }

        /// Send a text message.
        pub async fn send_text(&self, content: &str) -> Result<(), ChatError> {
            let msg = ChatMessage::Text {
                username: self.username.clone(),
                content: content.to_string(),
            };
            self.write_tx
                .send(msg)
                .await
                .map_err(|_| ChatError::ChannelClosed)
        }

        /// Receive the next message.
        /// Returns the formatted message string, or None if disconnected.
        pub async fn receive_text(&mut self) -> Result<Option<String>, ChatError> {
            match self.read_rx.recv().await {
                Some(ChatMessage::Text { username, content }) => {
                    Ok(Some(format!("{}: {}", username, content)))
                }
                Some(ChatMessage::System(msg)) => Ok(Some(msg)),
                None => Ok(None),
            }
        }

        /// Get the username.
        pub fn username(&self) -> &str {
            &self.username
        }
    }

    /// Run the typed chat client actor.
    ///
    /// This is the lower-level API that gives full control over events.
    pub async fn run_chat_client<F, Fut>(conn: RoomConnection, actor_fn: F)
    where
        F: Fn(TypedClientContext<ChatProtocol>, TypedClientEvent<ChatProtocol>) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        with_typed_client_actor::<ChatProtocol, F, Fut>(conn, actor_fn).await;
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::{ChatClient, run_chat_client};

// ============================================================================
// WASM exports
// ============================================================================

#[cfg(feature = "wasm")]
mod wasm {
    use super::*;
    use chat_protocol::TypedProtocol;
    use std::cell::RefCell;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsValue;

    /// Re-export ApiClient from the core client library for convenience.
    pub use multiplayer_kit_client::wasm_exports::JsApiClient;

    /// Check if WebTransport is supported in the current browser.
    fn is_webtransport_supported() -> bool {
        js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("WebTransport"))
            .map(|v: JsValue| !v.is_undefined())
            .unwrap_or(false)
    }

    /// Chat client for JavaScript with typed protocol.
    #[wasm_bindgen]
    pub struct JsChatClient {
        username: String,
        channel: multiplayer_kit_client::Channel,
        recv_state: RefCell<RecvState>,
    }

    struct RecvState {
        buffer: MessageBuffer,
        pending: Vec<ChatMessage>,
    }

    impl JsChatClient {
        fn new(username: String, channel: multiplayer_kit_client::Channel) -> Self {
            Self {
                username,
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
        /// Connect with auto-detection.
        #[wasm_bindgen(js_name = connectAuto)]
        pub async fn connect_auto(
            webtransport_url: &str,
            websocket_url: &str,
            ticket: &str,
            room_id: u32,
            cert_hash: Option<String>,
            username: String,
        ) -> Result<JsChatClient, JsError> {
            let (url, transport) = if is_webtransport_supported() {
                (
                    webtransport_url,
                    multiplayer_kit_client::Transport::WebTransport,
                )
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

            // Send channel identification
            let channel_id_msg =
                frame_message(&[ChatProtocol::channel_to_id(ChatChannel::Chat)]);
            channel
                .write(&channel_id_msg)
                .await
                .map_err(|e| JsError::new(&e.to_string()))?;

            Ok(JsChatClient::new(username, channel))
        }

        /// Send a text message.
        #[wasm_bindgen(js_name = sendText)]
        pub async fn send_text(&self, content: &str) -> Result<(), JsError> {
            let msg = ChatMessage::Text {
                username: self.username.clone(),
                content: content.to_string(),
            };
            let data = bincode::serialize(&msg)
                .map_err(|e| JsError::new(&format!("Serialize error: {}", e)))?;
            let framed = frame_message(&data);
            self.channel
                .write(&framed)
                .await
                .map_err(|e| JsError::new(&e.to_string()))
        }

        /// Receive the next message as a formatted string.
        /// Returns null if disconnected.
        #[wasm_bindgen(js_name = receiveText)]
        pub async fn receive_text(&self) -> Result<Option<String>, JsError> {
            // Check pending first
            {
                let mut state = self.recv_state.borrow_mut();
                if let Some(msg) = state.pending.pop() {
                    return Ok(Some(format_message(&msg)));
                }
            }

            // Read more data
            loop {
                let mut temp_buf = vec![0u8; 64 * 1024];
                let n = self
                    .channel
                    .read(&mut temp_buf)
                    .await
                    .map_err(|e| JsError::new(&e.to_string()))?;

                if n == 0 {
                    return Ok(None);
                }

                {
                    let mut state = self.recv_state.borrow_mut();
                    let results: Vec<_> = state.buffer.push(&temp_buf[..n]).collect();
                    for result in results {
                        match result {
                            Ok(data) => {
                                if let Ok(msg) = bincode::deserialize::<ChatMessage>(&data) {
                                    state.pending.push(msg);
                                }
                            }
                            Err(e) => {
                                return Err(JsError::new(&format!("Framing error: {}", e)))
                            }
                        }
                    }

                    if let Some(msg) = state.pending.pop() {
                        return Ok(Some(format_message(&msg)));
                    }
                }
            }
        }

        /// Get the username.
        #[wasm_bindgen(getter)]
        pub fn username(&self) -> String {
            self.username.clone()
        }
    }

    fn format_message(msg: &ChatMessage) -> String {
        match msg {
            ChatMessage::Text { username, content } => format!("{}: {}", username, content),
            ChatMessage::System(s) => s.clone(),
        }
    }
}

#[cfg(feature = "wasm")]
pub use wasm::*;

// Re-export typed actor for WASM
#[cfg(feature = "wasm")]
pub use multiplayer_kit_helpers::JsTypedClientActor;

// Re-export JsRoomConnection for typed actor usage
#[cfg(feature = "wasm")]
pub use multiplayer_kit_client::wasm_exports::JsRoomConnection;
