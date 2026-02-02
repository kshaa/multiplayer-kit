use crate::room::{ActorFactory, RoomContext};
use crate::{AuthFuture, AuthRequest, Server, ServerConfig};
use multiplayer_kit_protocol::{RejectReason, UserContext};
use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;

/// Builder for constructing a Server instance.
pub struct ServerBuilder<T: UserContext + Unpin> {
    config: ServerConfig,
    auth_handler: Option<Arc<dyn Fn(AuthRequest) -> AuthFuture<T> + Send + Sync>>,
    actor_factory: Option<ActorFactory<T>>,
    jwt_secret: Option<Vec<u8>>,
    _phantom: PhantomData<T>,
}

impl<T: UserContext + Unpin> ServerBuilder<T> {
    pub fn new() -> Self {
        Self {
            config: ServerConfig::default(),
            auth_handler: None,
            actor_factory: None,
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

    /// Set the room actor factory.
    /// 
    /// The factory is called for each new room and should return a future
    /// that runs the room's actor loop. The actor receives events via 
    /// `ctx.events` and sends messages via `ctx.send()`.
    /// 
    /// # Example
    /// ```ignore
    /// .room_actor(|mut ctx| async move {
    ///     loop {
    ///         match ctx.events.recv().await {
    ///             Some(RoomEvent::UserJoined(user)) => {
    ///                 ctx.send(Outgoing::broadcast(format!("{} joined", user.name))).await;
    ///             }
    ///             Some(RoomEvent::Message { sender, payload }) => {
    ///                 // Broadcast to all except sender
    ///                 ctx.send(Outgoing::new(payload, Route::AllExcept(vec![sender.id()]))).await;
    ///             }
    ///             Some(RoomEvent::Shutdown) | None => break,
    ///             _ => {}
    ///         }
    ///     }
    /// })
    /// ```
    pub fn room_actor<F, Fut>(mut self, factory: F) -> Self
    where
        F: Fn(RoomContext<T>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.actor_factory = Some(Arc::new(move |ctx| Box::pin(factory(ctx))));
        self
    }

    /// Build the server.
    pub fn build(self) -> Result<Server<T>, &'static str> {
        let auth_handler = self.auth_handler.ok_or("auth_handler is required")?;
        let actor_factory = self.actor_factory.ok_or("room_actor is required")?;
        let jwt_secret = self.jwt_secret.ok_or("jwt_secret is required")?;

        Ok(Server {
            config: self.config,
            auth_handler,
            actor_factory,
            jwt_secret,
            _phantom: PhantomData,
        })
    }
}

impl<T: UserContext + Unpin> Default for ServerBuilder<T> {
    fn default() -> Self {
        Self::new()
    }
}
