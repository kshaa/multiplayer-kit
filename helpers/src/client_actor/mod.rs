//! Client-side actor for game clients.
//!
//! Provides strongly-typed message handling for client actors.
//! Works on both native and WASM platforms with a unified API.

mod actor;
mod channel;
mod types;
mod sink;

use crate::utils::{ChannelMessage, GameClientContext, MaybeSend, MaybeSync, frame_message};
use crate::spawning::Spawner;
use std::collections::HashMap;
use std::marker::PhantomData;
use tokio::sync::mpsc;

#[cfg(target_arch = "wasm32")]
use std::rc::Rc;

pub use actor::with_client_actor;
pub use channel::ClientConnection;
pub use types::{ServerTarget, ClientMessage, ClientSource};
pub use sink::{ClientSink, ClientOutput};

/// Error when sending to actor fails.
#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum ActorSendError {
    #[error("channel closed")]
    ChannelClosed,
    #[error("not connected")]
    NotConnected,
}

/// Handle for sending messages to a client actor.
pub struct ClientActorSender<M: ChannelMessage> {
    #[cfg(not(target_arch = "wasm32"))]
    tx: mpsc::UnboundedSender<M>,
    #[cfg(target_arch = "wasm32")]
    tx: Rc<mpsc::UnboundedSender<M>>,
    _marker: PhantomData<M>,
}

impl<M: ChannelMessage> Clone for ClientActorSender<M> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            _marker: PhantomData,
        }
    }
}

impl<M: ChannelMessage> ClientActorSender<M> {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn new(tx: mpsc::UnboundedSender<M>) -> Self {
        Self {
            tx,
            _marker: PhantomData,
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn new(tx: mpsc::UnboundedSender<M>) -> Self {
        Self {
            tx: Rc::new(tx),
            _marker: PhantomData,
        }
    }

    /// Send a message to the actor.
    pub fn send(&self, message: M) -> Result<(), ActorSendError> {
        self.tx.send(message).map_err(|_| ActorSendError::ChannelClosed)
    }
}

/// Sender for local messages directly to the actor (not to server).
pub struct LocalSender<M: ChannelMessage> {
    #[cfg(not(target_arch = "wasm32"))]
    tx: mpsc::UnboundedSender<M>,
    #[cfg(target_arch = "wasm32")]
    tx: Rc<mpsc::UnboundedSender<M>>,
    _marker: PhantomData<M>,
}

impl<M: ChannelMessage> Clone for LocalSender<M> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            _marker: PhantomData,
        }
    }
}

impl<M: ChannelMessage> LocalSender<M> {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn new(tx: mpsc::UnboundedSender<M>) -> Self {
        Self {
            tx,
            _marker: PhantomData,
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn new(tx: mpsc::UnboundedSender<M>) -> Self {
        Self {
            tx: Rc::new(tx),
            _marker: PhantomData,
        }
    }

    /// Send a local message directly to the actor (bypasses network).
    pub fn send(&self, message: M) -> Result<(), ActorSendError> {
        self.tx.send(message).map_err(|_| ActorSendError::ChannelClosed)
    }
}

/// Handle to a running client actor.
///
/// Contains senders for:
/// - `sender`: Messages to the server (via network)
/// - `local_sender`: Local messages directly to the actor (bypasses network)
pub struct ClientActorHandle<M: ChannelMessage> {
    /// Sender for sending messages to the server.
    pub sender: ClientActorSender<M>,
    /// Sender for sending local messages directly to the actor.
    pub local_sender: LocalSender<M>,
}

impl<M: ChannelMessage> Clone for ClientActorHandle<M> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            local_sender: self.local_sender.clone(),
        }
    }
}

/// Events delivered to a client actor.
pub enum ClientEvent<M: ChannelMessage> {
    /// All channels connected.
    Connected,
    /// Message received from server.
    Message(M),
    /// Self-sent message (from ctx.self_tx()).
    Internal(M),
    /// Local message from the environment (e.g., Bevy sending Tick/KeyEvent).
    Local(M),
    /// Disconnected (any channel failed).
    Disconnected,
}

/// Context for client-side actor.
pub struct ClientContext<M: ChannelMessage, Ctx: GameClientContext = ()> {
    pub(crate) channels: HashMap<M::Channel, mpsc::UnboundedSender<Vec<u8>>>,
    pub(crate) self_tx: mpsc::UnboundedSender<M>,
    pub(crate) game_context: SharedPtr<Ctx>,
    pub(crate) _phantom: PhantomData<M>,
}

impl<M: ChannelMessage, Ctx: GameClientContext> ClientContext<M, Ctx> {
    /// Send a message (routed to correct channel by type).
    pub fn send(&self, message: &M) -> Result<(), crate::utils::EncodeError> {
        if let Some(channel_type) = message.channel() {
            let data = message.encode()?;
            let framed = frame_message(&data);
            if let Some(tx) = self.channels.get(&channel_type) {
                let _ = tx.send(framed);
            }
        }
        Ok(())
    }

    /// Get sender to send messages to self (for timers, async operations).
    pub fn self_tx(&self) -> mpsc::UnboundedSender<M> {
        self.self_tx.clone()
    }

    /// Get the game context for accessing game-specific services.
    pub fn game_context(&self) -> &SharedPtr<Ctx> {
        &self.game_context
    }
}

impl<M: ChannelMessage, Ctx: GameClientContext> Clone for ClientContext<M, Ctx> {
    fn clone(&self) -> Self {
        Self {
            channels: self.channels.clone(),
            self_tx: self.self_tx.clone(),
            game_context: self.game_context.clone(),
            _phantom: PhantomData,
        }
    }
}

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
pub(crate) enum InternalEvent<M: ChannelMessage> {
    Message(M),
    Disconnected,
}

/// Run a client actor.
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
/// `ClientActorHandle<M>` containing the sender for sending messages to the actor.
pub fn run_client_actor<M, Ctx, F, S>(
    conn: channel::Connection,
    game_context: Ctx,
    actor_fn: F,
    spawner: S,
) -> ClientActorHandle<M>
where
    M: ChannelMessage,
    Ctx: GameClientContext,
    F: Fn(&ClientContext<M, Ctx>, ClientEvent<M>, &S) + MaybeSend + MaybeSync + Clone + 'static,
    S: Spawner,
{
    channel::run_client_actor_impl::<M, Ctx, F, S>(conn, game_context, actor_fn, spawner)
}
