//! Message framing utilities.
//!
//! Uses a simple 4-byte big-endian length prefix protocol.

use thiserror::Error;

/// Maximum message size (10 MB).
pub const MAX_MESSAGE_SIZE: usize = 10 * 1024 * 1024;

/// Framing errors.
#[derive(Debug, Error)]
pub enum FramingError {
    #[error("message too large: {0} bytes (max {MAX_MESSAGE_SIZE})")]
    MessageTooLarge(usize),
    #[error("invalid frame: length prefix indicates {0} bytes")]
    InvalidFrame(usize),
}

/// Frame a message with 4-byte big-endian length prefix.
///
/// # Example
///
/// ```
/// use multiplayer_kit_helpers::frame_message;
///
/// let framed = frame_message(b"hello");
/// assert_eq!(&framed[..4], &[0, 0, 0, 5]); // length = 5
/// assert_eq!(&framed[4..], b"hello");
/// ```
pub fn frame_message(payload: &[u8]) -> Vec<u8> {
    let len = (payload.len() as u32).to_be_bytes();
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&len);
    out.extend_from_slice(payload);
    out
}

/// Buffer for accumulating bytes and extracting complete messages.
///
/// Handles the case where data arrives in arbitrary chunks.
#[derive(Debug, Default)]
pub struct MessageBuffer {
    buffer: Vec<u8>,
}

impl MessageBuffer {
    /// Create a new empty buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Push bytes into the buffer and return an iterator over complete messages.
    ///
    /// Messages are extracted and removed from the buffer as you iterate.
    ///
    /// # Example
    ///
    /// ```
    /// use multiplayer_kit_helpers::{MessageBuffer, frame_message};
    ///
    /// let mut buffer = MessageBuffer::new();
    ///
    /// // Push a complete framed message
    /// let messages: Vec<_> = buffer.push(&frame_message(b"hello")).collect();
    /// assert_eq!(messages.len(), 1);
    /// assert_eq!(messages[0].as_ref().unwrap(), b"hello");
    ///
    /// // Push partial data
    /// let messages: Vec<_> = buffer.push(&[0, 0, 0, 5, b'w', b'o']).collect();
    /// assert_eq!(messages.len(), 0); // incomplete
    ///
    /// // Push the rest
    /// let messages: Vec<_> = buffer.push(b"rld").collect();
    /// assert_eq!(messages.len(), 1);
    /// assert_eq!(messages[0].as_ref().unwrap(), b"world");
    /// ```
    pub fn push<'a>(&'a mut self, data: &[u8]) -> MessageIterator<'a> {
        self.buffer.extend_from_slice(data);
        MessageIterator { buffer: self }
    }

    /// Try to extract one complete message from the buffer.
    fn try_extract(&mut self) -> Option<Result<Vec<u8>, FramingError>> {
        if self.buffer.len() < 4 {
            return None;
        }

        let len = u32::from_be_bytes([
            self.buffer[0],
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
        ]) as usize;

        // Sanity check
        if len > MAX_MESSAGE_SIZE {
            // Clear buffer to recover from corruption
            self.buffer.clear();
            return Some(Err(FramingError::InvalidFrame(len)));
        }

        if self.buffer.len() < 4 + len {
            return None; // Incomplete
        }

        // Extract message
        let message = self.buffer[4..4 + len].to_vec();
        self.buffer.drain(..4 + len);
        Some(Ok(message))
    }

    /// Clear the buffer.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    /// Check if buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Get current buffer length.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }
}

/// Iterator over complete messages in a buffer.
pub struct MessageIterator<'a> {
    buffer: &'a mut MessageBuffer,
}

impl<'a> Iterator for MessageIterator<'a> {
    type Item = Result<Vec<u8>, FramingError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.buffer.try_extract()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_message() {
        let framed = frame_message(b"test");
        assert_eq!(framed, vec![0, 0, 0, 4, b't', b'e', b's', b't']);
    }

    #[test]
    fn test_buffer_complete_message() {
        let mut buffer = MessageBuffer::new();
        let messages: Vec<_> = buffer.push(&frame_message(b"hello")).collect();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].as_ref().unwrap(), b"hello");
    }

    #[test]
    fn test_buffer_partial_then_complete() {
        let mut buffer = MessageBuffer::new();

        // Partial header
        let messages: Vec<_> = buffer.push(&[0, 0]).collect();
        assert_eq!(messages.len(), 0);

        // Rest of header + partial payload
        let messages: Vec<_> = buffer.push(&[0, 5, b'h', b'e']).collect();
        assert_eq!(messages.len(), 0);

        // Rest of payload
        let messages: Vec<_> = buffer.push(b"llo").collect();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].as_ref().unwrap(), b"hello");
    }

    #[test]
    fn test_buffer_multiple_messages() {
        let mut buffer = MessageBuffer::new();

        let mut data = frame_message(b"one");
        data.extend(frame_message(b"two"));
        data.extend(frame_message(b"three"));

        let messages: Vec<_> = buffer.push(&data).collect();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].as_ref().unwrap(), b"one");
        assert_eq!(messages[1].as_ref().unwrap(), b"two");
        assert_eq!(messages[2].as_ref().unwrap(), b"three");
    }
}
