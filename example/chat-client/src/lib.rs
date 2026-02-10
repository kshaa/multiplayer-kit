//! Chat client library built on multiplayer-kit typed actors.
//!
//! This library provides a unified chat actor that works on both native and WASM platforms.
//! The platform-specific parts are handled via the `ChatClientAdapter` trait - you implement
//! callbacks for rendering messages, and the library handles all protocol logic.
//!
//! # Example (Native with adapter)
//!
//! ```ignore
//! use chat_client::{ChatClientAdapter, GameClientContext, start_chat_actor};
//!
//! struct MyAdapter;
//!
//! impl GameClientContext for MyAdapter {}
//!
//! impl ChatClientAdapter for MyAdapter {
//!     fn on_connected(&self) { println!("Connected!"); }
//!     fn on_disconnected(&self) { println!("Disconnected"); }
//!     fn on_text_message(&self, username: &str, content: &str) {
//!         println!("{}: {}", username, content);
//!     }
//!     fn on_system_message(&self, text: &str) {
//!         println!("[system] {}", text);
//!     }
//! }
//!
//! // Start the actor and get a handle
//! let handle = start_chat_actor(conn, MyAdapter);
//! handle.send_text("Hello!")?;
//! ```
//!
//! # Example (WASM with callbacks)
//!
//! ```javascript
//! const handle = runChatActor(conn, {
//!     onConnected: () => console.log("Connected!"),
//!     onDisconnected: () => console.log("Disconnected"),
//!     onTextMessage: (username, content) => console.log(`${username}: ${content}`),
//!     onSystemMessage: (text) => console.log(`[system] ${text}`),
//! });
//!
//! // Send a message:
//! handle.sendText("Hello!");
//! ```

// Re-exports from protocol
pub use chat_protocol::{ChatChannel, ChatEvent, ChatMessage, ChatProtocol, ChatUser};
pub use multiplayer_kit_client::{ApiClient, ClientError, RoomConnection};
pub use multiplayer_kit_protocol::RoomId;

// Re-export GameClientContext so implementers of ChatClientAdapter don't need to add helpers dep
pub use multiplayer_kit_helpers::GameClientContext;

// Core actor implementation (shared by all platforms)
mod actor;

// ============================================================================
// ChatClientAdapter - platform-specific callbacks
// ============================================================================

/// Adapter trait for platform-specific chat UI.
///
/// Implement this trait to handle chat events in your platform's native way.
/// The chat actor handles all protocol logic and calls these methods when events occur.
pub trait ChatClientAdapter: multiplayer_kit_helpers::GameClientContext {
    /// Called when successfully connected to the room.
    fn on_connected(&self);

    /// Called when disconnected from the room.
    fn on_disconnected(&self);

    /// Called when a text message is received.
    fn on_text_message(&self, username: &str, content: &str);

    /// Called when a system message is received.
    fn on_system_message(&self, text: &str);
}

// ============================================================================
// Errors
// ============================================================================

/// Chat client errors.
#[derive(Debug, thiserror::Error)]
pub enum ChatError {
    #[error("client error: {0}")]
    Client(#[from] ClientError),
    #[error("channel closed")]
    ChannelClosed,
    #[error("not connected")]
    NotConnected,
}

// ============================================================================
// Platform implementations
// ============================================================================

#[cfg(not(target_arch = "wasm32"))]
mod native;

#[cfg(not(target_arch = "wasm32"))]
pub use native::{start_chat_actor, ChatClientContext, ChatHandle};

#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(target_arch = "wasm32")]
pub use wasm::*;

// Re-export for low-level access
#[cfg(target_arch = "wasm32")]
pub use multiplayer_kit_client::wasm_exports::{JsLobbyClient, JsRoomConnection};
