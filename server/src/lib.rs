pub mod builder;
pub mod error;
pub mod lobby;
pub mod quic;
pub mod rest;
pub mod room;
pub mod ticket;

use crate::lobby::Lobby;
use crate::quic::QuicState;
use crate::rest::AppState;
use crate::room::{RoomConfig, RoomManager};
use crate::ticket::TicketManager;
use actix_cors::Cors;
use actix_web::{web, App, HttpServer};
use multiplayer_kit_protocol::{MessageContext, RejectReason, Route, UserContext};
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use wtransport::tls::Sha256DigestFmt;
use wtransport::Identity;
use wtransport::ServerConfig as WtServerConfig;

pub use builder::ServerBuilder;
pub use error::ServerError;

/// Type alias for auth handler boxed future.
pub type AuthFuture<T> = Pin<Box<dyn Future<Output = Result<T, RejectReason>> + Send>>;

/// Type alias for room handler boxed future.
pub type RoomHandlerFuture<Id> =
    Pin<Box<dyn Future<Output = Result<Route<Id>, RejectReason>> + Send>>;

/// The main server struct, generic over user context type.
pub struct Server<T: UserContext> {
    config: ServerConfig,
    auth_handler: Arc<dyn Fn(AuthRequest) -> AuthFuture<T> + Send + Sync>,
    room_handler:
        Arc<dyn for<'a> Fn(&'a [u8], MessageContext<'a, T>) -> RoomHandlerFuture<T::Id> + Send + Sync>,
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
            room_max_lifetime_secs: 3600,         // 1 hour
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

impl<T: UserContext + 'static> Server<T> {
    /// Create a new server builder.
    pub fn builder() -> ServerBuilder<T> {
        ServerBuilder::new()
    }

    /// Run the server.
    pub async fn run(self) -> Result<(), ServerError> {
        tracing::info!(
            "Server starting on HTTP {} and QUIC {}",
            self.config.http_addr,
            self.config.quic_addr
        );

        // Create shared state
        let room_config = RoomConfig {
            max_lifetime: Duration::from_secs(self.config.room_max_lifetime_secs),
            first_connect_timeout: Duration::from_secs(self.config.room_first_connect_timeout_secs),
            empty_timeout: Duration::from_secs(self.config.room_empty_timeout_secs),
        };

        let room_manager = Arc::new(RoomManager::<T>::new(room_config));
        let ticket_manager = Arc::new(TicketManager::new(&self.jwt_secret));
        let lobby = Arc::new(Lobby::new());

        // Generate self-signed certificate for development
        let identity = Identity::self_signed(["localhost", "127.0.0.1"])
            .map_err(|e| ServerError::Quic(e.to_string()))?;

        // Get certificate hash for browser WebTransport
        let cert_hash_b64 = identity
            .certificate_chain()
            .as_slice()
            .first()
            .map(|cert| {
                let hash = cert.hash();
                tracing::info!(
                    "Certificate SHA-256 hash: {}",
                    hash.fmt(Sha256DigestFmt::DottedHex)
                );
                let hash_bytes: &[u8] = hash.as_ref();
                use base64::Engine;
                let b64 = base64::engine::general_purpose::STANDARD.encode(hash_bytes);
                tracing::info!("Certificate hash (base64 for browser): {}", b64);
                b64
            });

        let cert_hash = Arc::new(tokio::sync::RwLock::new(cert_hash_b64));

        // REST state
        let app_state = web::Data::new(AppState {
            room_manager: Arc::clone(&room_manager),
            ticket_manager: Arc::clone(&ticket_manager),
            lobby: Arc::clone(&lobby),
            auth_handler: Arc::clone(&self.auth_handler),
            cert_hash: Arc::clone(&cert_hash),
        });

        // QUIC state
        let quic_state = Arc::new(QuicState {
            room_manager: Arc::clone(&room_manager),
            ticket_manager: Arc::clone(&ticket_manager),
            lobby: Arc::clone(&lobby),
            room_handler: Arc::clone(&self.room_handler),
        });

        // Spawn room lifecycle manager
        let lifecycle_room_manager = Arc::clone(&room_manager);
        let lifecycle_lobby = Arc::clone(&lobby);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                lifecycle_room_manager.run_lifecycle_checks(&lifecycle_lobby).await;
            }
        });

        // Start HTTP server
        let http_addr = self.config.http_addr.clone();
        let http_server = HttpServer::new(move || {
            let cors = Cors::permissive(); // Allow all origins for dev
            App::new()
                .wrap(cors)
                .app_data(app_state.clone())
                .route("/ticket", web::post().to(rest::issue_ticket::<T>))
                .route("/rooms", web::post().to(rest::create_room::<T>))
                .route("/rooms", web::get().to(rest::list_rooms::<T>))
                .route("/rooms/{id}", web::delete().to(rest::delete_room::<T>))
                .route("/cert-hash", web::get().to(rest::get_cert_hash::<T>))
        })
        .bind(&http_addr)?
        .run();

        // Start QUIC server
        let quic_addr = self.config.quic_addr.clone();
        let quic_server = async move {
            let config = WtServerConfig::builder()
                .with_bind_address(quic_addr.parse().unwrap())
                .with_identity(identity)
                .keep_alive_interval(Some(Duration::from_secs(5)))
                .max_idle_timeout(Some(Duration::from_secs(30)))
                .expect("valid idle timeout")
                .build();

            let endpoint = wtransport::Endpoint::server(config)
                .map_err(|e| ServerError::Quic(e.to_string()))?;

            tracing::info!("QUIC server listening");

            loop {
                let incoming = endpoint.accept().await;
                let state = Arc::clone(&quic_state);

                tokio::spawn(async move {
                    if let Err(e) = quic::handle_session(incoming, state).await {
                        tracing::warn!("Session error: {:?}", e);
                    }
                });
            }

            #[allow(unreachable_code)]
            Ok::<(), ServerError>(())
        };

        // Run both servers
        tokio::select! {
            result = http_server => {
                result.map_err(|e| ServerError::Http(e.to_string()))?;
            }
            result = quic_server => {
                result?;
            }
        }

        Ok(())
    }
}
