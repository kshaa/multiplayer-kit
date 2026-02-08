//! Chat protocol shared between server and client.
//!
//! Defines channel types and message types for a simple chat application.

pub use multiplayer_kit_helpers::{DecodeError, EncodeError, TypedProtocol};
use serde::{Deserialize, Serialize};

// ============================================================================
// Channel Types
// ============================================================================

/// Channel types for chat.
/// For this simple example, we only have one channel type.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum ChatChannel {
    /// Main chat channel for text messages.
    Chat = 0,
}

// ============================================================================
// Message Types (per channel)
// ============================================================================

/// Messages on the Chat channel.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ChatMessage {
    /// A text message from a user.
    Text {
        /// The username of the sender.
        username: String,
        /// The message content.
        content: String,
    },
    /// System message (join/leave notifications, etc.)
    System(String),
}

// ============================================================================
// Unified Event Enum
// ============================================================================

/// All events that can occur (wraps per-channel message types).
#[derive(Clone, Debug)]
pub enum ChatEvent {
    /// Chat channel event.
    Chat(ChatMessage),
}

// ============================================================================
// Protocol Implementation
// ============================================================================

/// Chat protocol definition.
pub struct ChatProtocol;

impl TypedProtocol for ChatProtocol {
    type Channel = ChatChannel;
    type Event = ChatEvent;

    fn channel_to_id(channel: ChatChannel) -> u8 {
        channel as u8
    }

    fn channel_from_id(id: u8) -> Option<ChatChannel> {
        match id {
            0 => Some(ChatChannel::Chat),
            _ => None,
        }
    }

    fn all_channels() -> &'static [ChatChannel] {
        &[ChatChannel::Chat]
    }

    fn decode(channel: ChatChannel, data: &[u8]) -> Result<ChatEvent, DecodeError> {
        match channel {
            ChatChannel::Chat => {
                let msg: ChatMessage = bincode::deserialize(data)
                    .map_err(|e| DecodeError::Deserialize(e.to_string()))?;
                Ok(ChatEvent::Chat(msg))
            }
        }
    }

    fn encode(event: &ChatEvent) -> Result<(ChatChannel, Vec<u8>), EncodeError> {
        match event {
            ChatEvent::Chat(msg) => {
                let data = bincode::serialize(msg)
                    .map_err(|e| EncodeError::Serialize(e.to_string()))?;
                Ok((ChatChannel::Chat, data))
            }
        }
    }
}

// ============================================================================
// User type (shared)
// ============================================================================

use multiplayer_kit_protocol::UserContext;

/// User context for chat.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatUser {
    pub id: u64,
    pub username: String,
}

impl UserContext for ChatUser {
    type Id = u64;
    fn id(&self) -> u64 {
        self.id
    }
}
