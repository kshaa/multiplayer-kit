use thiserror::Error;

#[derive(Error, Debug)]
pub enum ClientError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Invalid ticket")]
    InvalidTicket,

    #[error("Room not found")]
    RoomNotFound,

    #[error("Disconnected: {0}")]
    Disconnected(String),

    #[error("Send failed: {0}")]
    SendFailed(String),

    #[error("Receive failed: {0}")]
    ReceiveFailed(String),

    #[error("Serialization error: {0}")]
    Serialization(String),
}
