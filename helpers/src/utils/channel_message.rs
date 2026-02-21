//! Channel message trait for network serialization.

/// Error when decoding a message.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("unknown channel id: {0}")]
    UnknownChannel(u8),
    #[error("deserialization failed: {0}")]
    Deserialize(String),
}

/// Error when encoding a message.
#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("serialization failed: {0}")]
    Serialize(String),
}

/// A message that can be sent over network channels.
///
/// The message type knows which channel it belongs to, enabling self-describing
/// serialization without a separate protocol type.
///
/// # Example
///
/// ```ignore
/// #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
/// enum Channel { Chat, GameState }
///
/// enum MyMessage {
///     // Network messages
///     ChatText(String),           // channel() -> Some(Chat)
///     Position { x: f32, y: f32 }, // channel() -> Some(GameState)
///     
///     // Internal messages (self-scheduled, not sent over network)
///     Tick,                       // channel() -> None
/// }
///
/// impl ChannelMessage for MyMessage {
///     type Channel = Channel;
///     
///     fn channel(&self) -> Option<Self::Channel> {
///         match self {
///             MyMessage::ChatText(_) => Some(Channel::Chat),
///             MyMessage::Position { .. } => Some(Channel::GameState),
///             MyMessage::Tick => None,  // Internal
///         }
///     }
///     // ...
/// }
/// ```
pub trait ChannelMessage: Sized + Send + 'static {
    /// Channel identifier type.
    type Channel: Copy + Eq + std::hash::Hash + std::fmt::Debug + Send + Sync + 'static;

    /// Which channel this message belongs to.
    /// Returns `None` for internal/self-scheduled messages that don't go over the network.
    fn channel(&self) -> Option<Self::Channel>;

    /// All network channels this message type uses.
    fn all_channels() -> &'static [Self::Channel];

    /// Convert channel to wire ID byte.
    fn channel_to_id(channel: Self::Channel) -> u8;

    /// Convert wire ID byte to channel.
    fn channel_from_id(id: u8) -> Option<Self::Channel>;

    /// Serialize this message to bytes.
    fn encode(&self) -> Result<Vec<u8>, EncodeError>;

    /// Deserialize a message from channel + bytes.
    fn decode(channel: Self::Channel, data: &[u8]) -> Result<Self, DecodeError>;
}
