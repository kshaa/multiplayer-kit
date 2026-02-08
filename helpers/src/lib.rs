//! Helper utilities for multiplayer-kit.
//!
//! Provides optional abstractions on top of the raw channel API:
//! - Message framing (length-prefixed)
//! - Typed channels with automatic serialization
//! - Actor pattern helpers

mod framing;

pub use framing::{frame_message, MessageBuffer, FramingError};

#[cfg(any(feature = "client", feature = "wasm"))]
mod channel;

#[cfg(any(feature = "client", feature = "wasm"))]
pub use channel::{MessageChannel, MessageChannelError};

// Old actor module (kept for backwards compat)
#[cfg(feature = "server")]
mod actor;

#[cfg(feature = "server")]
pub use actor::{
    with_actor, with_framing,
    RoomContext, RoomEvent,
    MessageContext, MessageEvent,
    Outgoing, Route,
};

// Typed module - available for server and client (wasm or native)
#[cfg(any(feature = "server", feature = "client", feature = "wasm"))]
mod typed;

// Common types always available when typed module is
#[cfg(any(feature = "server", feature = "client", feature = "wasm"))]
pub use typed::{TypedProtocol, DecodeError, EncodeError};

// Server-specific typed exports
#[cfg(feature = "server")]
pub use typed::{TypedEvent, TypedContext, with_typed_actor};

// Client-specific typed exports (native)
#[cfg(feature = "client")]
pub use typed::{TypedClientEvent, TypedClientContext, with_typed_client_actor};
