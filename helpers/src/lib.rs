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

// Server-specific typed exports
#[cfg(feature = "server")]
pub use typed::{TypedContext, TypedEvent, with_typed_actor};

// Client-specific typed exports (native)
#[cfg(feature = "client")]
pub use typed::{TypedClientContext, TypedClientEvent, with_typed_client_actor};

// Client-specific typed exports (WASM)
#[cfg(feature = "wasm")]
pub use typed::JsTypedClientActor;
