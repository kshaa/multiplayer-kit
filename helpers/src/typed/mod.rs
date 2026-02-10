//! Typed actor helpers for server and client.
//!
//! Provides strongly-typed channels with automatic serialization.
//! Each channel has its own message type, identified on first message.
//!
//! # Module Structure
//!
//! - `platform` - Cross-platform Send/Sync abstractions and `GameClientContext`
//! - `spawner` - Task spawner trait for async execution
//! - `server/` - Server-side typed actor (requires "server" feature)
//! - `client/` - Client-side typed actor (requires "client" or "wasm" feature)

mod platform;
mod spawner;

#[cfg(feature = "server")]
mod server;

#[cfg(any(feature = "client", all(feature = "wasm", target_arch = "wasm32")))]
mod client;

// ============================================================================
// Core types - always available
// ============================================================================

/// Error when decoding a message.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("unknown channel id: {0}")]
    UnknownChannel(u8),
    #[error("deserialization failed: {0}")]
    Deserialize(String),
}

/// Error when encoding a message.
#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("serialization failed: {0}")]
    Serialize(String),
}

/// Protocol trait - game defines channel types and message types.
///
/// # Example
///
/// ```ignore
/// #[derive(Clone, Copy, PartialEq, Eq, Hash)]
/// pub enum GameChannel { Chat, GameState }
///
/// #[derive(Serialize, Deserialize)]
/// pub enum ChatMessage { Text(String), Emote(String) }
///
/// #[derive(Serialize, Deserialize)]  
/// pub enum GameStateMessage { Position { x: f32, y: f32 } }
///
/// pub enum GameEvent {
///     Chat(ChatMessage),
///     GameState(GameStateMessage),
/// }
///
/// pub struct MyProtocol;
/// impl TypedProtocol for MyProtocol {
///     type Channel = GameChannel;
///     type Event = GameEvent;
///     // ...
/// }
/// ```
pub trait TypedProtocol: Send + Sync + 'static {
    /// Channel identifier enum.
    type Channel: Copy + Eq + std::hash::Hash + std::fmt::Debug + Send + Sync + 'static;

    /// Unified event enum (wraps all channel message types).
    type Event: Send + 'static;

    /// Get channel ID byte from channel.
    fn channel_to_id(channel: Self::Channel) -> u8;

    /// Get channel from ID byte.
    fn channel_from_id(id: u8) -> Option<Self::Channel>;

    /// List all channel types (for client to open all).
    fn all_channels() -> &'static [Self::Channel];

    /// Decode bytes from a channel into an event.
    fn decode(channel: Self::Channel, data: &[u8]) -> Result<Self::Event, DecodeError>;

    /// Encode an event to (channel, bytes).
    fn encode(event: &Self::Event) -> Result<(Self::Channel, Vec<u8>), EncodeError>;
}

// ============================================================================
// Platform abstractions - always available
// ============================================================================

pub use platform::{GameClientContext, MaybeSend, MaybeSync};
pub use spawner::Spawner;

#[cfg(feature = "client")]
pub use spawner::TokioSpawner;

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
pub use spawner::WasmSpawner;

// ============================================================================
// Server-side typed actor
// ============================================================================

#[cfg(feature = "server")]
pub use server::{TypedContext, TypedEvent, with_typed_actor};

// ============================================================================
// Client-side typed actor (Rust handlers) - unified for all platforms
// ============================================================================

#[cfg(any(feature = "client", all(feature = "wasm", target_arch = "wasm32")))]
pub use client::{
    ActorHandle, ActorSendError, ClientConnection, SharedPtr, TypedActorSender,
    TypedClientContext, TypedClientEvent, run_typed_client_actor,
};

