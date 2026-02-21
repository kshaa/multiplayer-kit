//! Server actor types for message routing.

use crate::actor::ActorProtocol;
use std::marker::PhantomData;

/// Target for sending messages to users (server-side).
///
/// Used by server actors to send messages to individual users or broadcast.
pub struct UserTarget<User, M>(PhantomData<(User, M)>);

/// Destination for user-targeted messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserDestination<User> {
    /// Send to a specific user.
    User(User),
    /// Broadcast to all users.
    Broadcast,
}

impl<User, M> ActorProtocol for UserTarget<User, M> {
    type ActorId = UserDestination<User>;
    type Message = M;
}

/// Messages delivered to a server actor.
///
/// Wraps lifecycle events and game messages into a single type.
/// The sender information comes from `AddressedMessage.from`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerMessage<M> {
    /// A user connected. User info is in `AddressedMessage.from`.
    UserConnected,
    /// A user disconnected. User info is in `AddressedMessage.from`.
    UserDisconnected,
    /// A game message. Sender is in `AddressedMessage.from`.
    Message(M),
}

/// Source of a message to the server actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerSource<User> {
    /// Message from a user.
    User(User),
    /// Internal/self-scheduled message.
    Internal,
}

/// Configuration for a server actor.
#[derive(Debug, Clone)]
pub struct ServerActorConfig<C> {
    /// The room ID.
    pub room_id: multiplayer_kit_protocol::RoomId,
    /// The room configuration.
    pub room_config: C,
}
