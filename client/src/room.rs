//! Room client for game communication.

use crate::ClientError;
use multiplayer_kit_protocol::RoomId;

#[cfg(feature = "native")]
use tokio::io::AsyncReadExt;

/// Client for connecting to a game room.
#[cfg(feature = "native")]
pub struct RoomClient {
    connection: wtransport::Connection,
    msg_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
    _recv_task: tokio::task::JoinHandle<()>,
    _room_id: RoomId,
}

#[cfg(not(feature = "native"))]
pub struct RoomClient {
    _url: String,
    _ticket: String,
    _room_id: RoomId,
}

#[cfg(feature = "native")]
impl RoomClient {
    /// Connect to a room.
    pub async fn connect(url: &str, ticket: &str, room_id: RoomId) -> Result<Self, ClientError> {

        // Build endpoint
        let config = wtransport::ClientConfig::builder()
            .with_bind_default()
            .with_native_certs()
            .build();

        let endpoint = wtransport::Endpoint::client(config)
            .map_err(|e| ClientError::ConnectionFailed(e.to_string()))?;

        // Connect to the room endpoint
        let room_url = format!("{}/room/{}", url, room_id.0);
        let connection = endpoint
            .connect(&room_url)
            .await
            .map_err(|e| ClientError::ConnectionFailed(e.to_string()))?;

        // Open bidirectional stream to send ticket
        let (mut send_stream, mut recv_stream) = connection
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

        // Wait for join confirmation
        let mut confirm_buf = [0u8; 64];
        let _n = recv_stream
            .read(&mut confirm_buf)
            .await
            .map_err(|e| ClientError::ReceiveFailed(e.to_string()))?;

        // Create channel for incoming messages
        let (msg_tx, msg_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);

        // Spawn task to receive messages
        let recv_connection = connection.clone();
        let recv_task = tokio::spawn(async move {
            loop {
                match recv_connection.accept_uni().await {
                    Ok(mut stream) => {
                        let mut data = Vec::new();
                        if stream.read_to_end(&mut data).await.is_ok() && !data.is_empty() {
                            if msg_tx.send(data).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            connection,
            msg_rx,
            _recv_task: recv_task,
            _room_id: room_id,
        })
    }

    /// Send a message to the room.
    pub async fn send(&self, payload: &[u8]) -> Result<(), ClientError> {

        // Open a bidirectional stream for each message
        let (mut send_stream, _recv) = self
            .connection
            .open_bi()
            .await
            .map_err(|e| ClientError::SendFailed(e.to_string()))?
            .await
            .map_err(|e| ClientError::SendFailed(e.to_string()))?;

        send_stream
            .write_all(payload)
            .await
            .map_err(|e| ClientError::SendFailed(e.to_string()))?;

        Ok(())
    }

    /// Receive the next message from the room.
    pub async fn recv(&mut self) -> Result<Vec<u8>, ClientError> {
        self.msg_rx
            .recv()
            .await
            .ok_or_else(|| ClientError::Disconnected("Channel closed".to_string()))
    }

    /// Close the room connection.
    pub async fn close(self) -> Result<(), ClientError> {
        self._recv_task.abort();
        Ok(())
    }
}

#[cfg(not(feature = "native"))]
impl RoomClient {
    pub async fn connect(url: &str, ticket: &str, room_id: RoomId) -> Result<Self, ClientError> {
        Ok(Self {
            _url: url.to_string(),
            _ticket: ticket.to_string(),
            _room_id: room_id,
        })
    }

    pub async fn send(&self, _payload: &[u8]) -> Result<(), ClientError> {
        Err(ClientError::ConnectionFailed(
            "WASM client not yet implemented".to_string(),
        ))
    }

    pub async fn recv(&mut self) -> Result<Vec<u8>, ClientError> {
        Err(ClientError::ConnectionFailed(
            "WASM client not yet implemented".to_string(),
        ))
    }

    pub async fn close(self) -> Result<(), ClientError> {
        Ok(())
    }
}
