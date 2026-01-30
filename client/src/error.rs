//! Error types for the multiplayer client.

use thiserror::Error;

/// Main client error type.
#[derive(Error, Debug)]
pub enum ClientError {
    #[error("Connection failed: {0}")]
    Connection(#[from] ConnectionError),

    #[error("Send failed: {0}")]
    Send(#[from] SendError),

    #[error("Receive failed: {0}")]
    Receive(#[from] ReceiveError),

    #[error("Disconnected: {0}")]
    Disconnected(#[from] DisconnectReason),

    #[error("Serialization error: {0}")]
    Serialization(String),
}

/// Errors that occur during connection establishment.
#[derive(Error, Debug, Clone)]
pub enum ConnectionError {
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    #[error("DNS resolution failed: {0}")]
    DnsResolution(String),

    #[error("TLS handshake failed: {0}")]
    TlsHandshake(String),

    #[error("Connection refused: {0}")]
    Refused(String),

    #[error("Connection timeout")]
    Timeout,

    #[error("Server rejected connection: {0}")]
    ServerRejected(String),

    #[error("Invalid ticket")]
    InvalidTicket,

    #[error("Room not found")]
    RoomNotFound,

    #[error("Room is full")]
    RoomFull,

    #[error("Transport error: {0}")]
    Transport(String),
}

/// Errors that occur when sending messages.
#[derive(Error, Debug, Clone)]
pub enum SendError {
    #[error("Not connected")]
    NotConnected,

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Stream error: {0}")]
    Stream(String),

    #[error("Message too large")]
    MessageTooLarge,
}

/// Errors that occur when receiving messages.
#[derive(Error, Debug, Clone)]
pub enum ReceiveError {
    #[error("Not connected")]
    NotConnected,

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Stream error: {0}")]
    Stream(String),

    #[error("Malformed message: {0}")]
    MalformedMessage(String),
}

/// Reasons for disconnection.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum DisconnectReason {
    #[error("Connection closed by server")]
    ServerClosed,

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Kicked from room: {0}")]
    Kicked(String),

    #[error("Ticket expired")]
    TicketExpired,

    #[error("Room was deleted")]
    RoomDeleted,

    #[error("Idle timeout")]
    IdleTimeout,

    #[error("Client closed connection")]
    ClientClosed,

    #[error("Unknown: {0}")]
    Unknown(String),
}

/// Current state of a client connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    /// Not yet connected.
    Disconnected,
    /// Connection in progress.
    Connecting,
    /// Connected and ready.
    Connected,
    /// Connection lost.
    Lost(DisconnectReason),
}

impl ConnectionState {
    /// Returns true if the connection is established and healthy.
    pub fn is_connected(&self) -> bool {
        matches!(self, ConnectionState::Connected)
    }

    /// Returns true if the connection was lost (not just disconnected).
    pub fn is_lost(&self) -> bool {
        matches!(self, ConnectionState::Lost(_))
    }

    /// Returns the disconnect reason if the connection was lost.
    pub fn disconnect_reason(&self) -> Option<&DisconnectReason> {
        match self {
            ConnectionState::Lost(reason) => Some(reason),
            _ => None,
        }
    }
}
