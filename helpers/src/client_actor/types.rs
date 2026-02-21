//! Client actor types for message routing.

use crate::actor::ActorProtocol;
use std::marker::PhantomData;

/// Target for sending messages to the server (client-side).
///
/// Used by client actors to send messages to the server.
pub struct ServerTarget<M>(PhantomData<M>);

impl<M> ActorProtocol for ServerTarget<M> {
    type ActorId = ();
    type Message = M;
}

/// Messages delivered to a client actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMessage<M> {
    /// Connected to server.
    Connected,
    /// Disconnected from server.
    Disconnected,
    /// Message from server.
    Message(M),
}

/// Source of a message to the client actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientSource {
    /// Message from server.
    Server,
    /// Internal/self-scheduled message.
    Internal,
}
