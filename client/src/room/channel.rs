//! Persistent bidirectional channel to a room.

use crate::error::ReceiveError;
use crate::transport::webtransport::WebTransportStream;
use crate::transport::websocket::WebSocketStream;
use crate::ClientError;

use super::ChannelIO;

/// A persistent bidirectional channel to a room.
///
/// Wraps either a WebTransport stream or a WebSocket connection.
pub struct Channel {
    pub(super) inner: ChannelInner,
}

pub(super) enum ChannelInner {
    WebTransport(WebTransportStream),
    WebSocket(WebSocketStream),
}

impl Channel {
    /// Write bytes to the channel.
    pub async fn write(&self, data: &[u8]) -> Result<(), ClientError> {
        match &self.inner {
            ChannelInner::WebTransport(stream) => stream.write(data).await,
            ChannelInner::WebSocket(stream) => stream.write(data).await,
        }
    }

    /// Read bytes from the channel into buffer. Returns number of bytes read.
    pub async fn read(&self, buf: &mut [u8]) -> Result<usize, ClientError> {
        match &self.inner {
            ChannelInner::WebTransport(stream) => stream.read(buf).await,
            ChannelInner::WebSocket(stream) => stream.read(buf).await,
        }
    }

    /// Read exact number of bytes.
    pub async fn read_exact(&self, buf: &mut [u8]) -> Result<(), ClientError> {
        let mut filled = 0;
        while filled < buf.len() {
            let n = self.read(&mut buf[filled..]).await?;
            if n == 0 {
                return Err(ClientError::Receive(ReceiveError::Stream(
                    "Unexpected EOF".to_string(),
                )));
            }
            filled += n;
        }
        Ok(())
    }

    /// Check if connected.
    pub fn is_connected(&self) -> bool {
        match &self.inner {
            ChannelInner::WebTransport(stream) => stream.is_connected(),
            ChannelInner::WebSocket(stream) => stream.is_connected(),
        }
    }

    /// Close the channel.
    pub async fn close(self) -> Result<(), ClientError> {
        match self.inner {
            ChannelInner::WebTransport(stream) => stream.close().await,
            ChannelInner::WebSocket(stream) => stream.close().await,
        }
    }
}

impl ChannelIO for Channel {
    async fn read(&self, buf: &mut [u8]) -> Result<usize, ClientError> {
        Channel::read(self, buf).await
    }

    async fn write(&self, data: &[u8]) -> Result<(), ClientError> {
        Channel::write(self, data).await
    }

    fn is_connected(&self) -> bool {
        Channel::is_connected(self)
    }
}
