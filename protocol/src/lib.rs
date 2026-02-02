use serde::{Deserialize, Serialize};
use std::hash::Hash;

/// Trait that library users implement for their user data.
/// This data is embedded in JWTs and available during message validation.
pub trait UserContext: Serialize + for<'de> Deserialize<'de> + Clone + Send + Sync + 'static {
    type Id: Eq + Hash + Clone + Send + Sync + 'static + std::fmt::Debug;
    fn id(&self) -> Self::Id;
}

/// Unique identifier for a channel (bidirectional stream).
/// Assigned by the server when a client opens a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChannelId(pub u64);

/// Routing decision for outgoing messages.
#[derive(Debug, Clone)]
pub enum Route {
    /// Send to specific channels.
    Channels(Vec<ChannelId>),
    /// Don't send to anyone.
    None,
}

/// An outgoing message from a room actor.
#[derive(Debug, Clone)]
pub struct Outgoing {
    pub payload: Vec<u8>,
    pub route: Route,
}

impl Outgoing {
    pub fn new(payload: impl Into<Vec<u8>>, route: Route) -> Self {
        Self {
            payload: payload.into(),
            route,
        }
    }

    /// Send to specific channels.
    pub fn to_channels(payload: impl Into<Vec<u8>>, channels: Vec<ChannelId>) -> Self {
        Self::new(payload, Route::Channels(channels))
    }

    /// Send to a single channel.
    pub fn to_channel(payload: impl Into<Vec<u8>>, channel: ChannelId) -> Self {
        Self::new(payload, Route::Channels(vec![channel]))
    }
}

/// Events sent to a room actor.
#[derive(Debug, Clone)]
pub enum RoomEvent<T: UserContext> {
    /// A user joined the room (first channel opened).
    UserJoined(T),
    /// A user left the room (last channel closed).
    UserLeft(T),
    /// A channel was opened by a user.
    ChannelOpened { user: T, channel: ChannelId },
    /// A channel was closed.
    ChannelClosed { user: T, channel: ChannelId },
    /// A message was received on a channel.
    Message {
        sender: T,
        channel: ChannelId,
        payload: Vec<u8>,
    },
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
#[serde(transparent)]
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
