//! Chat client library built on multiplayer-kit-helpers.
//!
//! Wraps `RoomConnection` and `Channel` with automatic message framing for text.
//!
//! # Example
//!
//! ```ignore
//! use chat_client::ChatClient;
//!
//! let mut client = ChatClient::connect("https://127.0.0.1:4433", &ticket, room_id).await?;
//!
//! client.send_text("hello").await?;
//!
//! while let Some(msg) = client.receive_text().await? {
//!     println!("{}", msg);
//! }
//! ```

pub use multiplayer_kit_helpers::{MessageChannel, MessageChannelError};
pub use multiplayer_kit_client::{ApiClient, ClientError, RoomConnection};
pub use multiplayer_kit_protocol::RoomId;

/// Chat client errors.
#[derive(Debug, thiserror::Error)]
pub enum ChatError {
    #[error("channel error: {0}")]
    Channel(#[from] MessageChannelError),
    #[error("invalid UTF-8: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
}

/// A chat client with automatic message framing.
pub struct ChatClient {
    _conn: RoomConnection,
    channel: MessageChannel,
}

impl ChatClient {
    /// Connect to a room and open a framed message channel.
    pub async fn connect(url: &str, ticket: &str, room_id: RoomId) -> Result<Self, ClientError> {
        let conn = RoomConnection::connect(url, ticket, room_id).await?;
        let channel = conn.open_channel().await?;
        Ok(Self {
            _conn: conn,
            channel: MessageChannel::new(channel),
        })
    }

    /// Send a text message (auto-framed).
    pub async fn send_text(&self, msg: &str) -> Result<(), ClientError> {
        self.channel.send(msg.as_bytes()).await
    }

    /// Receive the next complete text message.
    pub async fn receive_text(&mut self) -> Result<Option<String>, ChatError> {
        match self.channel.recv().await? {
            Some(data) => Ok(Some(String::from_utf8(data)?)),
            None => Ok(None),
        }
    }

    /// Get a reference to the underlying MessageChannel.
    pub fn channel(&self) -> &MessageChannel {
        &self.channel
    }

    /// Get a mutable reference to the underlying MessageChannel.
    pub fn channel_mut(&mut self) -> &mut MessageChannel {
        &mut self.channel
    }
}
