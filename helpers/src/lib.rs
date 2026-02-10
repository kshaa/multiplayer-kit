//! Helper utilities for multiplayer-kit.
//!
//! Provides optional abstractions on top of the raw channel API:
//! - Message framing (length-prefixed)
//! - Typed channels with automatic serialization

mod framing;

pub use framing::{FramingError, MessageBuffer, frame_message};

// Typed module - available for server and client (wasm or native)
#[cfg(any(feature = "server", feature = "client", feature = "wasm"))]
mod typed;

// Common types always available when typed module is
#[cfg(any(feature = "server", feature = "client", feature = "wasm"))]
pub use typed::{DecodeError, EncodeError, TypedProtocol};

// Conditional Send/Sync traits (used for cross-platform compatibility)
#[cfg(any(feature = "client", all(feature = "wasm", target_arch = "wasm32")))]
pub use typed::{MaybeSend, MaybeSync};

// Server-specific typed exports
#[cfg(feature = "server")]
pub use typed::{TypedContext, TypedEvent, with_typed_actor};

// Client-specific typed exports (unified for native and WASM)
#[cfg(any(feature = "client", all(feature = "wasm", target_arch = "wasm32")))]
pub use typed::{
    ActorHandle, ActorSendError, ClientConnection, GameClientContext, SharedPtr,
    TypedActorSender, TypedClientContext, TypedClientEvent, run_typed_client_actor,
};

// Spawner trait and implementations
#[cfg(any(feature = "client", all(feature = "wasm", target_arch = "wasm32")))]
pub use typed::Spawner;

#[cfg(feature = "client")]
pub use typed::TokioSpawner;

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
pub use typed::WasmSpawner;

