//! Pure actor abstraction for multiplayer games.
//!
//! This module provides a generic, testable actor model that can be used
//! by both server and client implementations. The actor is defined by:
//!
//! - `ActorProtocol` - defines actor ID and message types
//! - `Actor` - extends protocol with config, state, and behavior
//! - `Sink` - type-safe message sending
//!
//! # Type-Safe Message Routing
//!
//! Each actor protocol determines what message types are valid:
//!
//! ```ignore
//! sink.send(actor_b_id, MsgToB::Hello);  // ✓ Compiles
//! sink.send(actor_b_id, MsgToA::Hello);  // ✗ Won't compile
//! ```

mod actor;
mod addressed_message;
mod protocol;
mod sink;

mod basic_sink;

#[cfg(test)]
mod tests;

pub use actor::Actor;
pub use addressed_message::AddressedMessage;
pub use protocol::ActorProtocol;
pub use sink::Sink;

pub use basic_sink::BasicSink;
