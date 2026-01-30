use serde::{Deserialize, Serialize};
use std::hash::Hash;

/// Trait that library users implement for their user data.
/// This data is embedded in JWTs and available during message validation.
pub trait UserContext: Serialize + for<'de> Deserialize<'de> + Clone + Send + Sync + 'static {
    type Id: Eq + Hash + Clone + Send + Sync + 'static;
    fn id(&self) -> Self::Id;
}

/// Context provided to the room message handler.
pub struct MessageContext<'a, T: UserContext> {
    /// The user who sent this message.
    pub sender: &'a T,
    /// All participants currently in the room.
    pub participants: &'a [T],
    /// The room ID.
    pub room_id: RoomId,
}

/// Routing decision returned by the room handler.
#[derive(Debug, Clone)]
pub enum Route<Id> {
    /// Send to all participants except the sender.
    Broadcast,
    /// Send to all participants including the sender.
    BroadcastIncludingSelf,
    /// Send only to these specific users.
    Only(Vec<Id>),
    /// Send to everyone except these users.
    AllExcept(Vec<Id>),
    /// Valid message, but don't forward (e.g., ACK).
    None,
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
