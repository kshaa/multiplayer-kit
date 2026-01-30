pub mod builder;
pub mod error;
pub mod room;
pub mod lobby;
pub mod ticket;

use multiplayer_kit_protocol::{MessageContext, RejectReason, Route, UserContext};
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

pub use builder::ServerBuilder;
pub use error::ServerError;

/// Type alias for auth handler boxed future.
pub type AuthFuture<T> = Pin<Box<dyn Future<Output = Result<T, RejectReason>> + Send>>;

/// Type alias for room handler boxed future.
pub type RoomHandlerFuture<Id> = Pin<Box<dyn Future<Output = Result<Route<Id>, RejectReason>> + Send>>;

/// The main server struct, generic over user context type.
pub struct Server<T: UserContext> {
    config: ServerConfig,
    auth_handler: Arc<dyn Fn(AuthRequest) -> AuthFuture<T> + Send + Sync>,
    room_handler: Arc<dyn for<'a> Fn(&'a [u8], MessageContext<'a, T>) -> RoomHandlerFuture<T::Id> + Send + Sync>,
    jwt_secret: Vec<u8>,
    _phantom: PhantomData<T>,
}

/// Configuration for the server.
#[derive(Clone)]
pub struct ServerConfig {
    /// Address to bind HTTP server.
    pub http_addr: String,
    /// Address to bind QUIC server.
    pub quic_addr: String,
    /// Maximum room lifetime in seconds.
    pub room_max_lifetime_secs: u64,
    /// Timeout for first connection to a room.
    pub room_first_connect_timeout_secs: u64,
    /// Timeout when room becomes empty.
    pub room_empty_timeout_secs: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            http_addr: "0.0.0.0:8080".to_string(),
            quic_addr: "0.0.0.0:4433".to_string(),
            room_max_lifetime_secs: 3600,        // 1 hour
            room_first_connect_timeout_secs: 300, // 5 minutes
            room_empty_timeout_secs: 300,         // 5 minutes
        }
    }
}

/// Request data passed to the auth handler.
#[derive(Debug, Clone)]
pub struct AuthRequest {
    /// Headers from the HTTP request (for API keys, OAuth tokens, etc.)
    pub headers: std::collections::HashMap<String, String>,
    /// Optional body data.
    pub body: Option<Vec<u8>>,
}

impl<T: UserContext> Server<T> {
    /// Create a new server builder.
    pub fn builder() -> ServerBuilder<T> {
        ServerBuilder::new()
    }

    /// Run the server.
    pub async fn run(self) -> Result<(), ServerError> {
        // TODO: Implement server startup
        // - Start HTTP server (actix-web) for REST endpoints
        // - Start QUIC server (wtransport) for lobby + room connections
        // - Spawn room lifecycle manager
        tracing::info!("Server starting on HTTP {} and QUIC {}", self.config.http_addr, self.config.quic_addr);
        
        todo!("Server::run implementation")
    }
}
