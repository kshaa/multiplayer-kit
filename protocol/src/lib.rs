use serde::{Deserialize, Serialize};
use std::hash::Hash;

/// Trait that library users implement for their user data.
/// This data is embedded in JWTs and available during message validation.
pub trait UserContext:
    Serialize + for<'de> Deserialize<'de> + Clone + Send + Sync + 'static
{
    type Id: Eq + Hash + Clone + Send + Sync + 'static + std::fmt::Debug;
    fn id(&self) -> Self::Id;
}

/// Trait for room configuration.
/// Games implement this to define what config is needed when creating a room.
pub trait RoomConfig:
    Serialize + for<'de> Deserialize<'de> + Clone + Send + Sync + 'static
{
    /// Room name (required, displayed in lobby).
    fn name(&self) -> &str;

    /// Validate config after deserialization. Called before room creation.
    fn validate(&self) -> Result<(), String> {
        Ok(())
    }

    /// Minimum players for matchmaking/game start. Default: 1
    fn min_players(&self) -> usize {
        1
    }

    /// Maximum players allowed. Default: None (unlimited)
    fn max_players(&self) -> Option<usize> {
        None
    }

    /// Does this room match a quickplay request?
    /// `request` is game-specific filter criteria from the client.
    fn matches_quickplay(&self, _request: &serde_json::Value) -> bool {
        true
    }

    /// Create config for a new quickplay room.
    /// Returns None if quickplay is disabled (default).
    /// `request` is game-specific filter criteria from the client.
    fn quickplay_default(_request: &serde_json::Value) -> Option<Self>
    where
        Self: Sized,
    {
        None
    }
}

/// Simple config with just a name, for games that don't need custom config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleConfig {
    pub name: String,
}

impl SimpleConfig {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    /// Generate an auto-room name with random suffix.
    pub fn auto() -> Self {
        Self {
            name: format!("Auto-room {}", rand_u16()),
        }
    }
}

impl Default for SimpleConfig {
    fn default() -> Self {
        Self::auto()
    }
}

impl RoomConfig for SimpleConfig {
    fn name(&self) -> &str {
        &self.name
    }

    fn quickplay_default(_request: &serde_json::Value) -> Option<Self> {
        Some(Self::auto())
    }
}

/// Simple random u16 without external deps.
fn rand_u16() -> u16 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    RandomState::new().build_hasher().finish() as u16
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

impl std::fmt::Display for RejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RejectReason::InvalidTicket => write!(f, "Invalid or expired ticket"),
            RejectReason::RoomNotFound => write!(f, "Room not found"),
            RejectReason::RoomFull => write!(f, "Room is full"),
            RejectReason::ValidationFailed(msg) => write!(f, "Validation failed: {}", msg),
            RejectReason::Custom(msg) => write!(f, "{}", msg),
        }
    }
}

/// Unique identifier for a room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RoomId(pub u64);

/// Room metadata broadcast via lobby.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomInfo<C = serde_json::Value> {
    pub id: RoomId,
    pub name: String,
    pub player_count: u32,
    pub min_players: u32,
    pub max_players: Option<u32>,
    pub is_joinable: bool,
    pub created_at: u64,
    /// Game-specific config. Use `()` or `serde_json::Value` if you don't need typed config.
    pub config: C,
}

impl<C> RoomInfo<C> {
    /// Convert to a different config type (for serialization).
    pub fn map_config<D>(self, f: impl FnOnce(C) -> D) -> RoomInfo<D> {
        RoomInfo {
            id: self.id,
            name: self.name,
            player_count: self.player_count,
            min_players: self.min_players,
            max_players: self.max_players,
            is_joinable: self.is_joinable,
            created_at: self.created_at,
            config: f(self.config),
        }
    }

    /// Erase config type to JSON value.
    pub fn into_json(self) -> RoomInfo<serde_json::Value>
    where
        C: Serialize,
    {
        self.map_config(|c| serde_json::to_value(c).unwrap_or(serde_json::Value::Null))
    }
}

/// Lobby update events sent over QUIC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LobbyEvent<C = serde_json::Value> {
    /// Initial snapshot of all rooms.
    Snapshot(Vec<RoomInfo<C>>),
    /// A room was created.
    RoomCreated(RoomInfo<C>),
    /// A room was updated (player count changed, etc.).
    RoomUpdated(RoomInfo<C>),
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

/// Response from quickplay endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickplayResponse {
    pub room_id: RoomId,
    /// True if a new room was created, false if joined existing.
    pub created: bool,
}
