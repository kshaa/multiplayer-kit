//! Client-side typed actor for game clients.
//!
//! Provides strongly-typed message handling for client actors.
//! Works on both native and WASM platforms with a unified API.

mod channel;

use super::platform::{GameClientContext, MaybeSend, MaybeSync};
use super::spawner::Spawner;
use super::TypedProtocol;
use crate::framing::frame_message;
use std::collections::HashMap;
use std::marker::PhantomData;
use tokio::sync::mpsc;

#[cfg(target_arch = "wasm32")]
use std::rc::Rc;

// Re-export the connection trait
pub use channel::ClientConnection;

// ============================================================================
// TypedActorSender - sender for typed events to actor
// ============================================================================

/// Error when sending to actor fails.
#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum ActorSendError {
    #[error("channel closed")]
    ChannelClosed,
    #[error("not connected")]
    NotConnected,
}

/// Handle for sending events to a typed client actor.
pub struct TypedActorSender<P: TypedProtocol> {
    #[cfg(not(target_arch = "wasm32"))]
    tx: mpsc::UnboundedSender<P::Event>,
    #[cfg(target_arch = "wasm32")]
    tx: Rc<mpsc::UnboundedSender<P::Event>>,
    _marker: PhantomData<P>,
}

// Manual Clone impl to avoid requiring P: Clone
impl<P: TypedProtocol> Clone for TypedActorSender<P> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            _marker: PhantomData,
        }
    }
}

impl<P: TypedProtocol> TypedActorSender<P> {
    /// Create a new sender (native).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn new(tx: mpsc::UnboundedSender<P::Event>) -> Self {
        Self {
            tx,
            _marker: PhantomData,
        }
    }

    /// Create a new sender (WASM).
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn new(tx: mpsc::UnboundedSender<P::Event>) -> Self {
        Self {
            tx: Rc::new(tx),
            _marker: PhantomData,
        }
    }

    /// Send an event to the actor
    pub fn send(&self, event: P::Event) -> Result<(), ActorSendError> {
        self.tx.send(event).map_err(|_| ActorSendError::ChannelClosed)
    }
}

// ============================================================================
// ActorHandle - returned from run_typed_client_actor
// ============================================================================

/// Handle to a running typed client actor.
///
/// Contains the sender for sending events to the actor.
/// The actor runs in the background until disconnected.
pub struct ActorHandle<P: TypedProtocol> {
    /// Sender for sending events to the actor.
    pub sender: TypedActorSender<P>,
}

impl<P: TypedProtocol> Clone for ActorHandle<P> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
        }
    }
}

// ============================================================================
// TypedClientEvent and TypedClientContext
// ============================================================================

/// Events delivered to a typed client actor.
pub enum TypedClientEvent<P: TypedProtocol> {
    /// All channels connected.
    Connected,
    /// Message received from server.
    Message(P::Event),
    /// Self-sent event (from ctx.self_tx()).
    Internal(P::Event),
    /// Disconnected (any channel failed).
    Disconnected,
}

/// Context for client-side typed actor.
pub struct TypedClientContext<P: TypedProtocol, Ctx: GameClientContext = ()> {
    channels: HashMap<P::Channel, mpsc::UnboundedSender<Vec<u8>>>,
    self_tx: mpsc::UnboundedSender<P::Event>,
    game_context: SharedPtr<Ctx>,
    _phantom: PhantomData<P>,
}

impl<P: TypedProtocol, Ctx: GameClientContext> TypedClientContext<P, Ctx> {
    /// Send an event (routed to correct channel by type).
    pub fn send(&self, event: &P::Event) -> Result<(), super::EncodeError> {
        let (channel_type, data) = P::encode(event)?;
        let framed = frame_message(&data);
        if let Some(tx) = self.channels.get(&channel_type) {
            let _ = tx.send(framed);
        }
        Ok(())
    }

    /// Get sender to send events to self (for timers, async operations).
    pub fn self_tx(&self) -> mpsc::UnboundedSender<P::Event> {
        self.self_tx.clone()
    }

    /// Get the game context for accessing game-specific services.
    pub fn game_context(&self) -> &SharedPtr<Ctx> {
        &self.game_context
    }
}

impl<P: TypedProtocol, Ctx: GameClientContext> Clone for TypedClientContext<P, Ctx> {
    fn clone(&self) -> Self {
        Self {
            channels: self.channels.clone(),
            self_tx: self.self_tx.clone(),
            game_context: self.game_context.clone(),
            _phantom: PhantomData,
        }
    }
}

// Platform-specific shared pointer type
#[cfg(not(target_arch = "wasm32"))]
pub type SharedPtr<T> = std::sync::Arc<T>;
#[cfg(target_arch = "wasm32")]
pub type SharedPtr<T> = std::rc::Rc<T>;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn make_shared<T>(value: T) -> SharedPtr<T> {
    std::sync::Arc::new(value)
}
#[cfg(target_arch = "wasm32")]
pub(crate) fn make_shared<T>(value: T) -> SharedPtr<T> {
    std::rc::Rc::new(value)
}

/// Internal event sent from channel tasks to main loop.
pub(crate) enum InternalEvent<P: TypedProtocol> {
    Message(P::Event),
    Disconnected,
}

// ============================================================================
// run_typed_client_actor - SYNC function that spawns actor and returns handle
// ============================================================================

/// Run a typed client actor.
///
/// This spawns the actor loop using the provided spawner and returns a handle
/// immediately. The actor function is called synchronously for each event.
///
/// # Parameters
///
/// - `conn`: Connection to the server
/// - `game_context`: Game-specific context passed to the actor
/// - `actor_fn`: Sync function called for each event, receives (ctx, event, spawner)
/// - `spawner`: Platform-specific spawner for background tasks
///
/// # Returns
///
/// `ActorHandle<P>` containing the sender for sending events to the actor.
///
/// # Example
///
/// ```ignore
/// let handle = run_typed_client_actor(
///     conn.into(),
///     my_context,
///     |ctx, event, spawner| {
///         // Handle event synchronously
///         match event {
///             TypedClientEvent::Connected => { /* ... */ }
///             TypedClientEvent::Message(msg) => { /* ... */ }
///             TypedClientEvent::Disconnected => { /* ... */ }
///             TypedClientEvent::Internal(msg) => { /* ... */ }
///         }
///     },
///     TokioSpawner,  // or WasmSpawner
/// );
///
/// // Later, send events:
/// handle.sender.send(my_event)?;
/// ```
pub fn run_typed_client_actor<P, Ctx, F, S>(
    conn: channel::Connection,
    game_context: Ctx,
    actor_fn: F,
    spawner: S,
) -> ActorHandle<P>
where
    P: TypedProtocol,
    Ctx: GameClientContext,
    F: Fn(&TypedClientContext<P, Ctx>, TypedClientEvent<P>, &S) + MaybeSend + MaybeSync + Clone + 'static,
    S: Spawner,
{
    channel::run_typed_client_actor_impl::<P, Ctx, F, S>(conn, game_context, actor_fn, spawner)
}

