use crate::{AuthFuture, AuthRequest, RoomHandlerFuture, Server, ServerConfig};
use multiplayer_kit_protocol::{MessageContext, RejectReason, Route, UserContext};
use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;

/// Builder for constructing a Server instance.
pub struct ServerBuilder<T: UserContext> {
    config: ServerConfig,
    auth_handler: Option<Arc<dyn Fn(AuthRequest) -> AuthFuture<T> + Send + Sync>>,
    room_handler: Option<Arc<dyn for<'a> Fn(&'a [u8], MessageContext<'a, T>) -> RoomHandlerFuture<T::Id> + Send + Sync>>,
    jwt_secret: Option<Vec<u8>>,
    _phantom: PhantomData<T>,
}

impl<T: UserContext> ServerBuilder<T> {
    pub fn new() -> Self {
        Self {
            config: ServerConfig::default(),
            auth_handler: None,
            room_handler: None,
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

    /// Set the auth handler that validates requests and produces user context.
    pub fn auth_handler<F, Fut>(mut self, handler: F) -> Self
    where
        F: Fn(AuthRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, RejectReason>> + Send + 'static,
    {
        self.auth_handler = Some(Arc::new(move |req| Box::pin(handler(req))));
        self
    }

    /// Set the room message handler that validates and routes messages.
    pub fn room_handler<F, Fut>(mut self, handler: F) -> Self
    where
        F: for<'a> Fn(&'a [u8], MessageContext<'a, T>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Route<T::Id>, RejectReason>> + Send + 'static,
    {
        self.room_handler = Some(Arc::new(move |payload, ctx| Box::pin(handler(payload, ctx))));
        self
    }

    /// Build the server.
    pub fn build(self) -> Result<Server<T>, &'static str> {
        let auth_handler = self.auth_handler.ok_or("auth_handler is required")?;
        let room_handler = self.room_handler.ok_or("room_handler is required")?;
        let jwt_secret = self.jwt_secret.ok_or("jwt_secret is required")?;

        Ok(Server {
            config: self.config,
            auth_handler,
            room_handler,
            jwt_secret,
            _phantom: PhantomData,
        })
    }
}

impl<T: UserContext> Default for ServerBuilder<T> {
    fn default() -> Self {
        Self::new()
    }
}
