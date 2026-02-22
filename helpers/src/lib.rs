//! Helper utilities for multiplayer-kit.
//!
//! Provides optional abstractions on top of the raw channel API:
//! - Message framing (length-prefixed)
//! - Channel-based message encoding/decoding
//! - Pure actor abstraction (testable, type-safe message routing)
//! - Game actor abstractions (server/client targets and sinks)

/// Utility modules (framing, platform abstractions, channel message).
pub mod utils;

/// Pure actor module - generic, testable actor abstraction.
pub mod actor;

/// Game actor module - targets and sinks for server/client actors.
pub mod game_actor;

/// Async task spawning abstractions.
#[cfg(any(feature = "client", all(feature = "wasm", target_arch = "wasm32")))]
pub mod spawning;

/// Server-side actor.
#[cfg(feature = "server")]
pub mod server_actor;

/// Client-side actor.
#[cfg(any(feature = "client", all(feature = "wasm", target_arch = "wasm32")))]
pub mod client_actor;

/// Testing utilities for actors.
#[cfg(all(feature = "server", feature = "client"))]
pub mod testing;

// Re-exports for convenience
pub use utils::{ChannelMessage, DecodeError, EncodeError, FramingError, MessageBuffer, frame_message};

#[cfg(any(feature = "client", all(feature = "wasm", target_arch = "wasm32")))]
pub use utils::{GameClientContext, MaybeSend, MaybeSync};

#[cfg(feature = "server")]
pub use server_actor::{
    ServerActorConfig, ServerMessage, ServerSource, ServerSink, ServerOutput,
    UserTarget, UserDestination, with_server_actor,
};

#[cfg(any(feature = "client", all(feature = "wasm", target_arch = "wasm32")))]
pub use client_actor::{
    ClientActorHandle, ActorSendError, ClientConnection, SharedPtr, ClientActorSender,
    ClientContext, ClientEvent, run_client_actor, with_client_actor,
    ClientMessage, ClientSource, ClientSink, ClientOutput, ServerTarget, LocalSender,
};

#[cfg(any(feature = "client", all(feature = "wasm", target_arch = "wasm32")))]
pub use spawning::Spawner;

#[cfg(feature = "client")]
pub use spawning::TokioSpawner;

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
pub use spawning::WasmSpawner;
