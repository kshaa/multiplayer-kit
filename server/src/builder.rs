use crate::room::{QuickplayFilter, Room, RoomHandlerFactory};
use crate::{AuthFuture, AuthRequest, GameServerContext, Server, ServerConfig};
use multiplayer_kit_protocol::{RejectReason, RoomConfig, RoomId, SimpleConfig, UserContext};
use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;

/// Builder for constructing a Server instance.
///
/// The `Ctx` type parameter allows injecting game-specific services that are
/// available throughout the server (auth handlers, room handlers, etc.).
pub struct ServerBuilder<
    T: UserContext + Unpin,
    C: RoomConfig = SimpleConfig,
    Ctx: GameServerContext = (),
> {
    config: ServerConfig,
    auth_handler: Option<Arc<dyn Fn(AuthRequest, Arc<Ctx>) -> AuthFuture<T> + Send + Sync>>,
    handler_factory: Option<RoomHandlerFactory<T, C, Ctx>>,
    jwt_secret: Option<Vec<u8>>,
    context: Option<Ctx>,
    quickplay_filter: Option<QuickplayFilter<C, Ctx>>,
    _phantom: PhantomData<(T, C)>,
}

impl<T: UserContext + Unpin, C: RoomConfig, Ctx: GameServerContext> ServerBuilder<T, C, Ctx> {
    pub fn new() -> Self {
        Self {
            config: ServerConfig::default(),
            auth_handler: None,
            handler_factory: None,
            jwt_secret: None,
            context: None,
            quickplay_filter: None,
            _phantom: PhantomData,
        }
    }

    /// Set the game context that provides game-specific services.
    ///
    /// The context is available to auth handlers and room handlers.
    ///
    /// # Example
    ///
    /// ```ignore
    /// struct MyContext { db: Pool<Postgres> }
    /// impl GameServerContext for MyContext {}
    ///
    /// Server::<MyUser, MyConfig, MyContext>::builder()
    ///     .context(MyContext { db: pool })
    ///     // ...
    /// ```
    pub fn context(mut self, ctx: Ctx) -> Self {
        self.context = Some(ctx);
        self
    }

    /// Set server configuration.
    pub fn config(mut self, config: ServerConfig) -> Self {
        self.config = config;
        self
    }

    /// Set the HTTP address.
    pub fn http_addr(mut self, addr: impl Into<String>) -> Self {
        self.config.http_addr = addr.into();
        self
    }

    /// Set the QUIC address.
    pub fn quic_addr(mut self, addr: impl Into<String>) -> Self {
        self.config.quic_addr = addr.into();
        self
    }

    /// Set the JWT signing secret.
    pub fn jwt_secret(mut self, secret: impl Into<Vec<u8>>) -> Self {
        self.jwt_secret = Some(secret.into());
        self
    }

    /// Set TLS certificate PEM file path.
    /// If not set, a self-signed certificate is generated for development.
    pub fn tls_cert(mut self, path: impl Into<String>) -> Self {
        self.config.tls_cert = Some(path.into());
        self
    }

    /// Set TLS private key PEM file path for QUIC.
    /// Required if tls_cert is set.
    pub fn tls_key(mut self, path: impl Into<String>) -> Self {
        self.config.tls_key = Some(path.into());
        self
    }

    /// Set TLS certificate PEM file path for HTTP server.
    /// If not set, HTTP runs without TLS (plain HTTP).
    pub fn http_tls_cert(mut self, path: impl Into<String>) -> Self {
        self.config.http_tls_cert = Some(path.into());
        self
    }

    /// Set TLS private key PEM file path for HTTP server.
    /// Required if http_tls_cert is set.
    pub fn http_tls_key(mut self, path: impl Into<String>) -> Self {
        self.config.http_tls_key = Some(path.into());
        self
    }

    /// Set hostnames/IPs for self-signed certificate (dev mode).
    /// Default: ["localhost", "127.0.0.1"].
    /// Use this for LAN deployments without real TLS certs.
    ///
    /// Note: Browser enforces max 14-day validity for self-signed certs.
    /// Client must fetch `/cert-hash` to trust the certificate.
    ///
    /// # Example
    /// ```ignore
    /// .self_signed_hosts(["192.168.1.50", "game.local"])
    /// ```
    pub fn self_signed_hosts<I, S>(mut self, hosts: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.config.self_signed_hosts = hosts.into_iter().map(|s| s.into()).collect();
        self
    }

    /// Set JWT ticket expiry in seconds. Default: 3600 (1 hour).
    pub fn ticket_expiry_secs(mut self, secs: u64) -> Self {
        self.config.ticket_expiry_secs = secs;
        self
    }

    /// Set allowed CORS origins.
    /// If not set, derives from self_signed_hosts (http://host for each).
    /// Use ["*"] to allow all origins (not recommended for production).
    ///
    /// # Example
    /// ```ignore
    /// .cors_origins(["https://game.example.com", "https://staging.example.com"])
    /// ```
    pub fn cors_origins<I, S>(mut self, origins: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.config.cors_origins = origins.into_iter().map(|s| s.into()).collect();
        self
    }

    /// Set the auth handler that validates requests and produces user context.
    ///
    /// The handler receives the auth request and the game context, allowing
    /// access to game-specific services (database, etc.) during authentication.
    ///
    /// # Example
    ///
    /// ```ignore
    /// .auth_handler(|req, ctx| async move {
    ///     // ctx.db is available for user lookups
    ///     let user = ctx.db.find_user(&req.body).await?;
    ///     Ok(user)
    /// })
    /// ```
    pub fn auth_handler<F, Fut>(mut self, handler: F) -> Self
    where
        F: Fn(AuthRequest, Arc<Ctx>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, RejectReason>> + Send + 'static,
    {
        self.auth_handler = Some(Arc::new(move |req, ctx| Box::pin(handler(req, ctx))));
        self
    }

    /// Set a filter for quickplay room matching.
    ///
    /// The filter is called for each candidate room during quickplay.
    /// Return `true` to allow joining the room, `false` to skip it.
    /// Use this to check game-specific runtime state (e.g., game not started).
    ///
    /// # Example
    /// ```ignore
    /// .quickplay_filter(|room_id, config, ctx| {
    ///     // Only allow joining rooms that haven't started
    ///     ctx.rooms.get(&room_id.0)
    ///         .map(|r| r.state.is_lobby())
    ///         .unwrap_or(true)
    /// })
    /// ```
    pub fn quickplay_filter<F>(mut self, filter: F) -> Self
    where
        F: Fn(RoomId, &C, &Ctx) -> bool + Send + Sync + 'static,
    {
        self.quickplay_filter = Some(Box::new(filter));
        self
    }

    /// Set the room handler factory.
    ///
    /// The factory is called for each new room and receives the Room,
    /// room config, and game context. Accept channels with `room.accept()`,
    /// read/write to channels, and broadcast with `room.broadcast()`.
    ///
    /// # Example
    /// ```ignore
    /// .room_handler(|mut room, config: MyRoomConfig, ctx| async move {
    ///     println!("Room '{}' started", config.name());
    ///     while let Some(accept) = room.accept().await {
    ///         match accept {
    ///             Accept::NewChannel(user, mut channel) => {
    ///                 let handle = room.handle();
    ///                 let ctx = Arc::clone(&ctx);
    ///                 tokio::spawn(async move {
    ///                     while let Some(data) = channel.read().await {
    ///                         // ctx.history.record(&data); // Use game context
    ///                         handle.broadcast_except(channel.id, &data);
    ///                     }
    ///                 });
    ///             }
    ///             Accept::Closing => {
    ///                 room.broadcast(b"Room closing...");
    ///                 // ctx.history.persist().await; // Save on close
    ///                 break;
    ///             }
    ///         }
    ///     }
    /// })
    /// ```
    pub fn room_handler<F, Fut>(mut self, factory: F) -> Self
    where
        F: Fn(Room<T>, C, Arc<Ctx>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.handler_factory = Some(Arc::new(move |room, config, ctx| {
            Box::pin(factory(room, config, ctx))
        }));
        self
    }

    /// Build the server.
    pub fn build(self) -> Result<Server<T, C, Ctx>, &'static str> {
        let auth_handler = self.auth_handler.ok_or("auth_handler is required")?;
        let handler_factory = self.handler_factory.ok_or("room_handler is required")?;
        let jwt_secret = self.jwt_secret.ok_or("jwt_secret is required")?;
        let context = self.context.ok_or("context is required")?;

        Ok(Server {
            config: self.config,
            auth_handler,
            handler_factory,
            jwt_secret,
            context: Arc::new(context),
            quickplay_filter: self.quickplay_filter,
            _phantom: PhantomData,
        })
    }
}

impl<T: UserContext + Unpin, C: RoomConfig, Ctx: GameServerContext> Default
    for ServerBuilder<T, C, Ctx>
{
    fn default() -> Self {
        Self::new()
    }
}
