use serde::{Deserialize, Serialize};
use std::hash::Hash;

/// Trait that library users implement for their user data.
/// This data is embedded in JWTs and available during message validation.
pub trait UserContext: Serialize + for<'de> Deserialize<'de> + Clone + Send + Sync + 'static {
    type Id: Eq + Hash + Clone + Send + Sync + 'static + std::fmt::Debug;
    fn id(&self) -> Self::Id;
}

/// Routing decision for outgoing messages.
#[derive(Debug, Clone)]
pub enum Route<Id> {
    /// Send to all participants.
    Broadcast,
    /// Send only to these specific users.
    Only(Vec<Id>),
    /// Send to everyone except these users.
    AllExcept(Vec<Id>),
    /// Don't send to anyone.
    None,
}

/// An outgoing message from a room actor.
#[derive(Debug, Clone)]
pub struct Outgoing<Id> {
    pub payload: Vec<u8>,
    pub route: Route<Id>,
}

impl<Id> Outgoing<Id> {
    pub fn new(payload: impl Into<Vec<u8>>, route: Route<Id>) -> Self {
        Self {
            payload: payload.into(),
            route,
        }
    }
    
    /// Broadcast to all participants.
    pub fn broadcast(payload: impl Into<Vec<u8>>) -> Self {
        Self::new(payload, Route::Broadcast)
    }
}

/// Events sent to a room actor.
#[derive(Debug, Clone)]
pub enum RoomEvent<T: UserContext> {
    /// A user joined the room.
    UserJoined(T),
    /// A user left the room.
    UserLeft(T),
    /// A message was received from a user.
    Message { sender: T, payload: Vec<u8> },
    /// Room is shutting down.
    Shutdown,
}

/// Reason for rejecting a connection or message.
#[derive(Debug, Clone)]
pub enum RejectReason {
    /// Invalid or expired ticket.
    InvalidTicket,
    /// Room does not exist.
    RoomNotFound,
    /// Room is full.
    RoomFull,
    /// Message failed validation.
    ValidationFailed(String),
    /// Custom rejection reason.
    Custom(String),
}

/// Unique identifier for a room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RoomId(pub u64);

/// Room metadata broadcast via lobby.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomInfo {
    pub id: RoomId,
    pub player_count: u32,
    pub max_players: Option<u32>,
    pub created_at: u64,
    pub metadata: Option<serde_json::Value>,
}

/// Lobby update events sent over QUIC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LobbyEvent {
    /// Initial snapshot of all rooms.
    Snapshot(Vec<RoomInfo>),
    /// A room was created.
    RoomCreated(RoomInfo),
    /// A room was updated (player count changed, etc.).
    RoomUpdated(RoomInfo),
    /// A room was deleted.
    RoomDeleted(RoomId),
}

/// Claims embedded in the JWT ticket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TicketClaims<T> {
    /// Expiration timestamp (Unix seconds).
    pub exp: u64,
    /// User data provided by auth handler.
    pub user: T,
}
