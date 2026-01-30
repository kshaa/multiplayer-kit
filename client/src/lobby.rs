//! Lobby client for receiving room updates.

use crate::ClientError;
use multiplayer_kit_protocol::LobbyEvent;

#[allow(unused_imports)]
#[cfg(feature = "native")]
use tokio::io::AsyncReadExt;

/// Client for connecting to the lobby QUIC endpoint.
#[cfg(feature = "native")]
pub struct LobbyClient {
    _connection: wtransport::Connection,
    recv_stream: wtransport::RecvStream,
}

#[cfg(not(feature = "native"))]
pub struct LobbyClient {
    _url: String,
    _ticket: String,
}

#[cfg(feature = "native")]
impl LobbyClient {
    /// Connect to the lobby.
    pub async fn connect(url: &str, ticket: &str) -> Result<Self, ClientError> {

        // Build endpoint
        let config = wtransport::ClientConfig::builder()
            .with_bind_default()
            .with_native_certs()
            .build();

        let endpoint = wtransport::Endpoint::client(config)
            .map_err(|e| ClientError::ConnectionFailed(e.to_string()))?;

        // Connect to the lobby endpoint
        let lobby_url = format!("{}/lobby", url);
        let connection = endpoint
            .connect(&lobby_url)
            .await
            .map_err(|e| ClientError::ConnectionFailed(e.to_string()))?;

        // Open bidirectional stream to send ticket
        let (mut send_stream, _recv) = connection
            .open_bi()
            .await
            .map_err(|e| ClientError::ConnectionFailed(e.to_string()))?
            .await
            .map_err(|e| ClientError::ConnectionFailed(e.to_string()))?;

        // Send ticket
        send_stream
            .write_all(ticket.as_bytes())
            .await
            .map_err(|e| ClientError::SendFailed(e.to_string()))?;

        // Close the send side to signal we're done
        drop(send_stream);

        // Accept the unidirectional stream for receiving events
        let recv_stream = connection
            .accept_uni()
            .await
            .map_err(|e| ClientError::ConnectionFailed(e.to_string()))?;

        Ok(Self {
            _connection: connection,
            recv_stream,
        })
    }

    /// Receive the next lobby event.
    pub async fn recv(&mut self) -> Result<LobbyEvent, ClientError> {

        // Read length prefix (4 bytes)
        let mut len_buf = [0u8; 4];
        self.recv_stream
            .read_exact(&mut len_buf)
            .await
            .map_err(|e| ClientError::ReceiveFailed(e.to_string()))?;

        let len = u32::from_be_bytes(len_buf) as usize;

        // Read payload
        let mut buf = vec![0u8; len];
        self.recv_stream
            .read_exact(&mut buf)
            .await
            .map_err(|e| ClientError::ReceiveFailed(e.to_string()))?;

        // Deserialize
        bincode::deserialize(&buf).map_err(|e| ClientError::Serialization(e.to_string()))
    }

    /// Close the lobby connection.
    pub async fn close(self) -> Result<(), ClientError> {
        Ok(())
    }
}

#[cfg(not(feature = "native"))]
impl LobbyClient {
    pub async fn connect(url: &str, ticket: &str) -> Result<Self, ClientError> {
        Ok(Self {
            _url: url.to_string(),
            _ticket: ticket.to_string(),
        })
    }

    pub async fn recv(&mut self) -> Result<LobbyEvent, ClientError> {
        Err(ClientError::ConnectionFailed(
            "WASM client not yet implemented".to_string(),
        ))
    }

    pub async fn close(self) -> Result<(), ClientError> {
        Ok(())
    }
}
