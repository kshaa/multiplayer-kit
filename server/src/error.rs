use thiserror::Error;

#[derive(Error, Debug)]
pub enum ServerError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HTTP server error: {0}")]
    Http(String),

    #[error("QUIC server error: {0}")]
    Quic(String),

    #[error("JWT error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),

    #[error("Configuration error: {0}")]
    Config(String),
}
