//! Lobby client for receiving room updates.

use crate::ClientError;
use multiplayer_kit_protocol::LobbyEvent;

/// Client for connecting to the lobby QUIC endpoint.
pub struct LobbyClient {
    // Connection handle would go here
    _url: String,
    _ticket: String,
}

impl LobbyClient {
    /// Connect to the lobby.
    pub async fn connect(url: &str, ticket: &str) -> Result<Self, ClientError> {
        // TODO: Implement WebTransport connection via xwt
        // 1. Connect to QUIC endpoint
        // 2. Send ticket for auth
        // 3. Receive initial room snapshot
        
        Ok(Self {
            _url: url.to_string(),
            _ticket: ticket.to_string(),
        })
    }

    /// Receive the next lobby event.
    pub async fn recv(&mut self) -> Result<LobbyEvent, ClientError> {
        // TODO: Receive from QUIC stream, deserialize
        todo!("LobbyClient::recv implementation")
    }

    /// Close the lobby connection.
    pub async fn close(self) -> Result<(), ClientError> {
        // TODO: Clean shutdown
        Ok(())
    }
}
