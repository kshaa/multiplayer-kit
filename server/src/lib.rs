pub mod builder;
pub mod error;
pub mod lobby;
pub mod quic;
pub mod rest;
pub mod room;
pub mod ticket;
pub mod ws;

use crate::lobby::Lobby;
use crate::quic::QuicState;
use crate::rest::AppState;
use crate::room::{QuickplayFilter, RoomHandlerFactory, RoomManager, RoomSettings};
use crate::ticket::TicketManager;
use crate::ws::WsState;
use actix_cors::Cors;
use actix_web::{App, HttpServer, web};
use multiplayer_kit_protocol::{RejectReason, RoomConfig, UserContext};
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use wtransport::Identity;
use wtransport::ServerConfig as WtServerConfig;
use wtransport::tls::Sha256DigestFmt;

// ============================================================================
// GameServerContext - inject game-specific services
// ============================================================================

/// Context trait for game-specific services.
///
/// Implement this trait on your context type to provide game-specific
/// services (database pools, history managers, metrics, etc.) throughout
/// the game server.
///
/// The context is passed to:
/// - Auth handlers (for external service lookups)
/// - Room handlers (for persisting game state/results)
/// - Typed actors via `TypedContext::game_context()`
///
/// # Example
///
/// ```ignore
/// struct MyGameContext {
///     db: Pool<Postgres>,
///     history: HistoryManager,
///     metrics: MetricsRecorder,
/// }
///
/// impl GameServerContext for MyGameContext {}
///
/// // Use in server:
/// let ctx = MyGameContext { ... };
/// Server::<MyUser, MyConfig, MyGameContext>::builder()
///     .context(ctx)
///     .auth_handler(|req, ctx| async move {
///         // ctx.db is available here
///         Ok(user)
///     })
///     .room_handler(|room, config, ctx| async move {
///         // ctx.history is available here
///     })
///     .build()
/// ```
pub trait GameServerContext: Send + Sync + 'static {}

/// Default implementation for unit type (no context needed).
impl GameServerContext for () {}

pub use builder::ServerBuilder;
pub use error::ServerError;
pub use multiplayer_kit_protocol::{ChannelId, RoomId, SimpleConfig};
pub use room::{Accept, ChannelError, Room, RoomHandle, ServerChannel};

// Re-export GameServerContext for convenience
// (it's defined at the top of this module)

/// Type alias for auth handler boxed future with game context.
pub type AuthFutureWithContext<T> = Pin<Box<dyn Future<Output = Result<T, RejectReason>> + Send>>;

/// Type alias for auth handler boxed future.
pub type AuthFuture<T> = Pin<Box<dyn Future<Output = Result<T, RejectReason>> + Send>>;

/// The main server struct, generic over user context, room config, and game context.
///
/// The `Ctx` type parameter allows games to inject their own services (database pools,
/// history managers, etc.) that are available throughout the server.
pub struct Server<T: UserContext + Unpin, C: RoomConfig = SimpleConfig, Ctx: GameServerContext = ()>
{
    config: ServerConfig,
    auth_handler: Arc<dyn Fn(AuthRequest, Arc<Ctx>) -> AuthFuture<T> + Send + Sync>,
    handler_factory: RoomHandlerFactory<T, C, Ctx>,
    jwt_secret: Vec<u8>,
    context: Arc<Ctx>,
    quickplay_filter: Option<QuickplayFilter<C, Ctx>>,
    _phantom: PhantomData<(T, C)>,
}

/// Configuration for the server.
#[derive(Clone, Default)]
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
    /// TLS certificate PEM file path for QUIC. If None, generates self-signed cert.
    pub tls_cert: Option<String>,
    /// TLS private key PEM file path for QUIC. Required if tls_cert is set.
    pub tls_key: Option<String>,
    /// TLS certificate PEM file path for HTTP. If None, HTTP runs without TLS.
    pub http_tls_cert: Option<String>,
    /// TLS private key PEM file path for HTTP. Required if http_tls_cert is set.
    pub http_tls_key: Option<String>,
    /// Hostnames/IPs for self-signed cert. Default: ["localhost", "127.0.0.1"].
    /// Only used when tls_cert is None.
    pub self_signed_hosts: Vec<String>,
    /// JWT ticket expiry in seconds. Default: 3600 (1 hour).
    pub ticket_expiry_secs: u64,
    /// Allowed CORS origins. If empty, uses self_signed_hosts with http(s) prefix.
    /// Use ["*"] to allow all origins (not recommended for production).
    pub cors_origins: Vec<String>,
}

impl ServerConfig {
    fn default() -> Self {
        Self {
            http_addr: "0.0.0.0:8080".to_string(),
            quic_addr: "0.0.0.0:8080".to_string(), // Same port, different protocol (UDP vs TCP)
            room_max_lifetime_secs: 3600,          // 1 hour
            room_first_connect_timeout_secs: 300,  // 5 minutes
            room_empty_timeout_secs: 300,          // 5 minutes
            tls_cert: None,
            tls_key: None,
            http_tls_cert: None,
            http_tls_key: None,
            self_signed_hosts: vec!["localhost".to_string(), "127.0.0.1".to_string()],
            ticket_expiry_secs: 3600, // 1 hour
            cors_origins: vec![],     // Empty = derive from self_signed_hosts
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

impl<T: UserContext + Unpin + 'static, C: RoomConfig + 'static, Ctx: GameServerContext>
    Server<T, C, Ctx>
{
    /// Create a new server builder.
    pub fn builder() -> ServerBuilder<T, C, Ctx> {
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
        let room_settings = RoomSettings {
            max_lifetime: Duration::from_secs(self.config.room_max_lifetime_secs),
            first_connect_timeout: Duration::from_secs(self.config.room_first_connect_timeout_secs),
            empty_timeout: Duration::from_secs(self.config.room_empty_timeout_secs),
        };

        let room_manager = Arc::new(RoomManager::<T, C, Ctx>::new(
            room_settings,
            self.handler_factory,
            Arc::clone(&self.context),
            self.quickplay_filter,
        ));
        let ticket_manager = Arc::new(TicketManager::new(
            &self.jwt_secret,
            self.config.ticket_expiry_secs,
        ));
        let lobby = Arc::new(Lobby::new());

        // Load TLS identity - either from files or generate self-signed
        let identity = match (&self.config.tls_cert, &self.config.tls_key) {
            (Some(cert_path), Some(key_path)) => {
                tracing::info!(
                    "Loading TLS certificate from {} and key from {}",
                    cert_path,
                    key_path
                );
                Identity::load_pemfiles(cert_path, key_path)
                    .await
                    .map_err(|e| ServerError::Quic(format!("Failed to load TLS cert/key: {}", e)))?
            }
            (Some(_), None) => {
                return Err(ServerError::Config(
                    "tls_cert specified without tls_key".into(),
                ));
            }
            (None, Some(_)) => {
                return Err(ServerError::Config(
                    "tls_key specified without tls_cert".into(),
                ));
            }
            (None, None) => {
                let hosts: Vec<&str> = self
                    .config
                    .self_signed_hosts
                    .iter()
                    .map(|s| s.as_str())
                    .collect();
                tracing::info!("Generating self-signed certificate for: {:?}", hosts);
                Identity::self_signed(&hosts).map_err(|e| ServerError::Quic(e.to_string()))?
            }
        };

        // Get certificate hash for browser WebTransport
        let cert_hash_b64 = identity.certificate_chain().as_slice().first().map(|cert| {
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
            context: Arc::clone(&self.context),
        });

        // WebSocket state
        let ws_state = web::Data::new(WsState {
            room_manager: Arc::clone(&room_manager),
            ticket_manager: Arc::clone(&ticket_manager),
            lobby: Arc::clone(&lobby),
        });

        // QUIC state
        let quic_state = Arc::new(QuicState {
            room_manager: Arc::clone(&room_manager),
            ticket_manager: Arc::clone(&ticket_manager),
            lobby: Arc::clone(&lobby),
        });

        // Spawn room lifecycle manager
        let lifecycle_room_manager = Arc::clone(&room_manager);
        let lifecycle_lobby = Arc::clone(&lobby);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                lifecycle_room_manager
                    .run_lifecycle_checks(&lifecycle_lobby)
                    .await;
            }
        });

        // Compute CORS origins
        let cors_origins: Vec<String> = if self.config.cors_origins.is_empty() {
            // Derive from self_signed_hosts + configured HTTP port
            let http_port = self.config.http_addr.split(':').last().unwrap_or("8080");
            self.config
                .self_signed_hosts
                .iter()
                .flat_map(|host| {
                    vec![
                        format!("http://{}", host),
                        format!("https://{}", host),
                        format!("http://{}:{}", host, http_port),
                        format!("https://{}:{}", host, http_port),
                    ]
                })
                .collect()
        } else {
            self.config.cors_origins.clone()
        };
        tracing::info!("CORS allowed origins: {:?}", cors_origins);

        // Start HTTP server
        let http_addr = self.config.http_addr.clone();
        let http_tls_cert = self.config.http_tls_cert.clone();
        let http_tls_key = self.config.http_tls_key.clone();

        let http_server_builder = HttpServer::new(move || {
            let cors = if cors_origins.iter().any(|o| o == "*") {
                Cors::permissive()
            } else {
                let mut cors_builder = Cors::default()
                    .allowed_methods(vec!["GET", "POST", "DELETE", "OPTIONS"])
                    .allowed_headers(vec!["Authorization", "Content-Type"])
                    .max_age(3600);
                for origin in &cors_origins {
                    cors_builder = cors_builder.allowed_origin(origin);
                }
                cors_builder
            };
            App::new()
                .wrap(cors)
                .app_data(app_state.clone())
                .app_data(ws_state.clone())
                .route("/ticket", web::post().to(rest::issue_ticket::<T, C, Ctx>))
                .route("/rooms", web::post().to(rest::create_room::<T, C, Ctx>))
                .route("/rooms", web::get().to(rest::list_rooms::<T, C, Ctx>))
                .route("/rooms/{id}", web::delete().to(rest::delete_room::<T, C, Ctx>))
                .route("/quickplay", web::post().to(rest::quickplay::<T, C, Ctx>))
                .route("/cert-hash", web::get().to(rest::get_cert_hash::<T, C, Ctx>))
                // WebSocket endpoints
                .route("/ws/room/{id}", web::get().to(ws::room_ws::<T, C, Ctx>))
                .route("/ws/lobby", web::get().to(ws::lobby_ws::<T, C, Ctx>))
        });

        // Bind with or without TLS
        let http_server = match (&http_tls_cert, &http_tls_key) {
            (Some(cert), Some(key)) => {
                tracing::info!("HTTP server with TLS on {}", http_addr);
                http_server_builder
                    .bind_rustls_0_23(&http_addr, load_rustls_config(cert, key)?)?
                    .run()
            }
            (Some(_), None) => {
                return Err(ServerError::Config(
                    "http_tls_cert specified without http_tls_key".into(),
                ));
            }
            (None, Some(_)) => {
                return Err(ServerError::Config(
                    "http_tls_key specified without http_tls_cert".into(),
                ));
            }
            (None, None) => {
                tracing::info!("HTTP server (no TLS) on {}", http_addr);
                http_server_builder.bind(&http_addr)?.run()
            }
        };

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

/// Load rustls config from PEM files for HTTP TLS.
fn load_rustls_config(
    cert_path: &str,
    key_path: &str,
) -> Result<rustls::ServerConfig, ServerError> {
    use std::fs::File;
    use std::io::BufReader;

    let cert_file = File::open(cert_path)
        .map_err(|e| ServerError::Config(format!("Failed to open cert file: {}", e)))?;
    let key_file = File::open(key_path)
        .map_err(|e| ServerError::Config(format!("Failed to open key file: {}", e)))?;

    let mut cert_reader = BufReader::new(cert_file);
    let mut key_reader = BufReader::new(key_file);

    let certs: Vec<_> = rustls_pemfile::certs(&mut cert_reader)
        .filter_map(|r| r.ok())
        .collect();

    if certs.is_empty() {
        return Err(ServerError::Config(
            "No certificates found in cert file".into(),
        ));
    }

    let key = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|e| ServerError::Config(format!("Failed to read private key: {}", e)))?
        .ok_or_else(|| ServerError::Config("No private key found in key file".into()))?;

    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| ServerError::Config(format!("Failed to build TLS config: {}", e)))?;

    Ok(config)
}
