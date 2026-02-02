//! Message-framed channel wrapper for the client.

use crate::framing::{frame_message, FramingError, MessageBuffer};
use multiplayer_kit_client::{Channel, ClientError};

/// A channel wrapper that provides message framing.
///
/// Wraps a raw `Channel` and handles:
/// - Sending: automatically adds length prefix
/// - Receiving: buffers and extracts complete messages
///
/// # Example
///
/// ```ignore
/// let channel = room_conn.open_channel().await?;
/// let mut msg_channel = MessageChannel::new(channel);
///
/// // Send
/// msg_channel.send(b"hello").await?;
///
/// // Receive
/// while let Some(msg) = msg_channel.recv().await? {
///     println!("got: {:?}", msg);
/// }
/// ```
pub struct MessageChannel {
    channel: Channel,
    buffer: MessageBuffer,
    read_buf: Vec<u8>,
    /// Pending messages extracted from buffer but not yet returned.
    pending: Vec<Vec<u8>>,
}

impl MessageChannel {
    /// Wrap a raw channel with message framing.
    pub fn new(channel: Channel) -> Self {
        Self {
            channel,
            buffer: MessageBuffer::new(),
            read_buf: vec![0u8; 64 * 1024], // 64KB read buffer
            pending: Vec::new(),
        }
    }

    /// Get a reference to the underlying channel.
    pub fn inner(&self) -> &Channel {
        &self.channel
    }

    /// Send a message (automatically framed with length prefix).
    pub async fn send(&self, payload: &[u8]) -> Result<(), ClientError> {
        let framed = frame_message(payload);
        self.channel.write(&framed).await
    }

    /// Receive the next complete message.
    ///
    /// Returns `Ok(Some(msg))` when a message is available.
    /// Returns `Ok(None)` when the channel is closed.
    /// Returns `Err` on error.
    ///
    /// Note: This may read multiple messages at once internally. Subsequent
    /// calls will return buffered messages before reading more data.
    pub async fn recv(&mut self) -> Result<Option<Vec<u8>>, MessageChannelError> {
        // Return pending message if available
        if let Some(msg) = self.pending.pop() {
            return Ok(Some(msg));
        }

        // Read more data
        loop {
            let n = self.channel.read(&mut self.read_buf).await?;
            if n == 0 {
                // Channel closed
                return Ok(None);
            }

            // Push data and extract messages
            for result in self.buffer.push(&self.read_buf[..n]) {
                match result {
                    Ok(msg) => self.pending.push(msg),
                    Err(e) => return Err(MessageChannelError::Framing(e)),
                }
            }

            // Return first message if we got any
            if let Some(msg) = self.pending.pop() {
                return Ok(Some(msg));
            }

            // Otherwise loop to read more
        }
    }

    /// Try to receive a message without blocking if none available.
    ///
    /// Returns `Ok(Some(msg))` if a buffered message is available.
    /// Returns `Ok(None)` if no complete message is buffered.
    pub fn try_recv(&mut self) -> Option<Vec<u8>> {
        self.pending.pop()
    }

    /// Consume this wrapper and return the underlying channel.
    pub fn into_inner(self) -> Channel {
        self.channel
    }
}

/// Errors from MessageChannel operations.
#[derive(Debug, thiserror::Error)]
pub enum MessageChannelError {
    #[error("channel error: {0}")]
    Channel(#[from] ClientError),
    #[error("framing error: {0}")]
    Framing(#[from] FramingError),
}
