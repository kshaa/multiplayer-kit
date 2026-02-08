use crate::room::{Room, RoomHandlerFactory};
use crate::{AuthFuture, AuthRequest, Server, ServerConfig};
use multiplayer_kit_protocol::{RejectReason, RoomConfig, SimpleConfig, UserContext};
use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;

/// Builder for constructing a Server instance.
pub struct ServerBuilder<T: UserContext + Unpin, C: RoomConfig = SimpleConfig> {
    config: ServerConfig,
    auth_handler: Option<Arc<dyn Fn(AuthRequest) -> AuthFuture<T> + Send + Sync>>,
    handler_factory: Option<RoomHandlerFactory<T, C>>,
    jwt_secret: Option<Vec<u8>>,
    _phantom: PhantomData<(T, C)>,
}

impl<T: UserContext + Unpin, C: RoomConfig> ServerBuilder<T, C> {
    pub fn new() -> Self {
        Self {
            config: ServerConfig::default(),
            auth_handler: None,
            handler_factory: None,
            jwt_secret: None,
            _phantom: PhantomData,
        }
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

    /// Set TLS private key PEM file path.
    /// Required if tls_cert is set.
    pub fn tls_key(mut self, path: impl Into<String>) -> Self {
        self.config.tls_key = Some(path.into());
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

    /// Set the auth handler that validates requests and produces user context.
    pub fn auth_handler<F, Fut>(mut self, handler: F) -> Self
    where
        F: Fn(AuthRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, RejectReason>> + Send + 'static,
    {
        self.auth_handler = Some(Arc::new(move |req| Box::pin(handler(req))));
        self
    }

    /// Set the room handler factory.
    /// 
    /// The factory is called for each new room and receives both the Room
    /// and the room config. Accept channels with `room.accept()`,
    /// read/write to channels, and broadcast with `room.broadcast()`.
    /// 
    /// # Example
    /// ```ignore
    /// .room_handler(|mut room, config: MyRoomConfig| async move {
    ///     println!("Room '{}' started", config.name());
    ///     while let Some(accept) = room.accept().await {
    ///         match accept {
    ///             Accept::NewChannel(user, mut channel) => {
    ///                 let handle = room.handle();
    ///                 tokio::spawn(async move {
    ///                     while let Some(data) = channel.read().await {
    ///                         handle.broadcast_except(channel.id, &data).await;
    ///                     }
    ///                 });
    ///             }
    ///             Accept::Closing => {
    ///                 room.broadcast(b"Room closing...").await;
    ///                 break;
    ///             }
    ///         }
    ///     }
    /// })
    /// ```
    pub fn room_handler<F, Fut>(mut self, factory: F) -> Self
    where
        F: Fn(Room<T>, C) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.handler_factory = Some(Arc::new(move |room, config| Box::pin(factory(room, config))));
        self
    }

    /// Build the server.
    pub fn build(self) -> Result<Server<T, C>, &'static str> {
        let auth_handler = self.auth_handler.ok_or("auth_handler is required")?;
        let handler_factory = self.handler_factory.ok_or("room_handler is required")?;
        let jwt_secret = self.jwt_secret.ok_or("jwt_secret is required")?;

        Ok(Server {
            config: self.config,
            auth_handler,
            handler_factory,
            jwt_secret,
            _phantom: PhantomData,
        })
    }
}

impl<T: UserContext + Unpin, C: RoomConfig> Default for ServerBuilder<T, C> {
    fn default() -> Self {
        Self::new()
    }
}
