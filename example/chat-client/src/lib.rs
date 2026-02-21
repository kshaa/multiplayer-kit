//! Chat client library built on multiplayer-kit typed actors.
//!
//! This library provides a unified chat actor that works on both native and WASM platforms.
//! The platform-specific parts are handled via the `ChatClientAdapter` trait - you implement
//! callbacks for rendering messages, and the library handles all protocol logic.
//!
//! # Example (Native with adapter)
//!
//! ```ignore
//! use chat_client::{ChatClientAdapter, start_chat_actor};
//!
//! #[derive(Clone)]
//! struct MyAdapter;
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
pub use chat_protocol::{ChatChannel, ChatEvent, ChatMessage, ChatUser};
pub use multiplayer_kit_client::{ApiClient, ClientError, ConnectionConfig, RoomConnection};
pub use multiplayer_kit_protocol::RoomId;

// Core actor implementation (shared by all platforms)
mod actor;

// Export actor types for testing
pub use actor::{ChatClientActor, ChatState};

// ============================================================================
// ChatClientAdapter - platform-specific callbacks
// ============================================================================

/// Adapter trait for platform-specific chat UI.
///
/// Implement this trait to handle chat events in your platform's native way.
/// The chat actor handles all protocol logic and calls these methods when events occur.
///
/// The adapter is passed as "extras" to the actor, so it must be `Clone` and
/// platform-appropriate (`Send + Sync` on native, no bounds on WASM).
pub trait ChatClientAdapter: multiplayer_kit_helpers::MaybeSend + multiplayer_kit_helpers::MaybeSync + Clone + 'static {
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
pub use multiplayer_kit_client::{JsLobbyClient, JsRoomConnection};
