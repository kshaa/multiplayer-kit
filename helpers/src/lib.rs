//! Helper utilities for multiplayer-kit.
//!
//! Provides optional abstractions on top of the raw channel API:
//! - Message framing (length-prefixed)
//! - Typed channels with automatic serialization
//! - Pure actor abstraction (testable, type-safe message routing)

/// Utility modules (framing, platform abstractions, typed protocol).
pub mod utils;

/// Pure actor module - generic, testable actor abstraction.
pub mod actor;

/// Async task spawning abstractions.
#[cfg(any(feature = "client", all(feature = "wasm", target_arch = "wasm32")))]
pub mod spawning;

/// Server-side typed actor.
#[cfg(feature = "server")]
pub mod server_actor;

/// Client-side typed actor.
#[cfg(any(feature = "client", all(feature = "wasm", target_arch = "wasm32")))]
pub mod client_actor;

// Re-exports for convenience
pub use utils::{DecodeError, EncodeError, FramingError, MessageBuffer, TypedProtocol, frame_message};

#[cfg(any(feature = "client", all(feature = "wasm", target_arch = "wasm32")))]
pub use utils::{GameClientContext, MaybeSend, MaybeSync};

#[cfg(feature = "server")]
pub use server_actor::{TypedContext, TypedEvent, with_typed_actor};

#[cfg(any(feature = "client", all(feature = "wasm", target_arch = "wasm32")))]
pub use client_actor::{
    ActorHandle, ActorSendError, ClientConnection, SharedPtr, TypedActorSender,
    TypedClientContext, TypedClientEvent, run_typed_client_actor,
};

#[cfg(any(feature = "client", all(feature = "wasm", target_arch = "wasm32")))]
pub use spawning::Spawner;

#[cfg(feature = "client")]
pub use spawning::TokioSpawner;

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
pub use spawning::WasmSpawner;
