//! Lobby client implementation.

use crate::error::{ConnectionState, DisconnectReason, ReceiveError};
use crate::transport::webtransport::{WebTransportConnection, WebTransportRecvStream};
use crate::transport::websocket::{WebSocketConnection, WebSocketStream};
use crate::{is_webtransport_supported, ClientError, Transport};
use multiplayer_kit_protocol::LobbyEvent;

// ============================================================================
// Config
// ============================================================================

/// Configuration for lobby connection.
#[derive(Debug, Clone, Default)]
pub struct LobbyConfig {
    /// Transport to use (default: Auto).
    pub transport: Transport,
    /// Base64-encoded SHA-256 cert hash for self-signed certs (WebTransport only).
    pub cert_hash: Option<String>,
}

// ============================================================================
// LobbyClient
// ============================================================================

/// Internal enum to hold the active transport stream.
enum LobbyTransport {
    WebTransport(WebTransportRecvStream),
    WebSocket(WebSocketStream),
}

/// A client connection to the lobby for receiving room updates.
///
/// The lobby pushes events about rooms being created, updated, or deleted.
pub struct LobbyClient {
    inner: LobbyTransport,
    state: ConnectionState,
    transport: Transport,
}

impl LobbyClient {
    /// Connect to the lobby.
    pub async fn connect(
        url: &str,
        ticket: &str,
        config: LobbyConfig,
    ) -> Result<Self, ClientError> {
        match config.transport {
            Transport::Auto => Self::connect_auto(url, ticket, config.cert_hash).await,
            Transport::WebTransport => {
                Self::connect_webtransport(url, ticket, config.cert_hash.as_deref()).await
            }
            Transport::WebSocket => Self::connect_websocket(url, ticket).await,
        }
    }

    async fn connect_auto(
        url: &str,
        ticket: &str,
        cert_hash: Option<String>,
    ) -> Result<Self, ClientError> {
        if is_webtransport_supported() {
            match Self::connect_webtransport(url, ticket, cert_hash.as_deref()).await {
                Ok(client) => return Ok(client),
                Err(_e) => {
                    #[cfg(target_arch = "wasm32")]
                    web_sys::console::warn_1(
                        &format!("WebTransport lobby failed, falling back to WebSocket: {:?}", _e)
                            .into(),
                    );
                }
            }
        }
        Self::connect_websocket(url, ticket).await
    }

    async fn connect_webtransport(
        url: &str,
        ticket: &str,
        cert_hash: Option<&str>,
    ) -> Result<Self, ClientError> {
        let lobby_url = format!("{}/lobby?ticket={}", url, ticket);
        let conn = WebTransportConnection::connect(&lobby_url, cert_hash, false).await?;
        let recv_stream = conn.accept_uni().await?;

        Ok(Self {
            inner: LobbyTransport::WebTransport(recv_stream),
            state: ConnectionState::Connected,
            transport: Transport::WebTransport,
        })
    }

    async fn connect_websocket(url: &str, ticket: &str) -> Result<Self, ClientError> {
        // Convert http(s):// to ws(s)://
        let ws_url = url
            .replace("https://", "wss://")
            .replace("http://", "ws://");

        let lobby_url = format!("{}/ws/lobby?ticket={}", ws_url, ticket);
        let conn = WebSocketConnection::connect(&lobby_url).await?;
        let stream = conn.open_stream().await?;

        Ok(Self {
            inner: LobbyTransport::WebSocket(stream),
            state: ConnectionState::Connected,
            transport: Transport::WebSocket,
        })
    }

    /// Get the actual transport being used.
    pub fn transport(&self) -> Transport {
        self.transport
    }

    /// Get the current connection state.
    pub fn state(&self) -> ConnectionState {
        self.state.clone()
    }

    /// Receive the next lobby event.
    ///
    /// This blocks until an event is available or the connection is lost.
    pub async fn recv(&mut self) -> Result<LobbyEvent, ClientError> {
        if !self.state.is_connected() {
            return Err(ClientError::Receive(ReceiveError::NotConnected));
        }

        let msg_data = match &mut self.inner {
            LobbyTransport::WebTransport(stream) => stream.recv_message().await,
            LobbyTransport::WebSocket(stream) => stream.recv_message().await,
        };

        match msg_data {
            Ok(data) => {
                // Lobby uses JSON (RoomInfo.config is serde_json::Value)
                serde_json::from_slice(&data)
                    .map_err(|e| ClientError::Receive(ReceiveError::MalformedMessage(e.to_string())))
            }
            Err(e) => {
                self.state = ConnectionState::Lost(DisconnectReason::NetworkError(e.to_string()));
                Err(e)
            }
        }
    }

    /// Close the lobby connection.
    pub async fn close(mut self) -> Result<(), ClientError> {
        self.state = ConnectionState::Lost(DisconnectReason::ClientClosed);
        // Transport will be dropped and cleaned up
        Ok(())
    }
}
