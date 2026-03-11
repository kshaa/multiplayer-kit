//! Client-side actor for game clients.
//!
//! Provides strongly-typed message handling for client actors.
//! Works on both native and WASM platforms with a unified API.

mod actor;
mod bridge;
mod channel;
mod connector;
mod runtime;
mod sink;
mod types;

use crate::utils::ChannelMessage;
use std::marker::PhantomData;
use tokio::sync::mpsc;

#[cfg(target_arch = "wasm32")]
use std::rc::Rc;

pub use actor::with_client_actor;
pub use bridge::{BridgeEvent, NetworkBridge};
pub use channel::{ClientConnection, Connection};
pub use connector::BridgeConnector;
pub use runtime::{ActorRuntime, RuntimeStatus};
pub use sink::{ClientSink, ClientOutput};
pub use types::{ClientMessage, ClientSource, ServerTarget};

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

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn new_from_sender(tx: mpsc::UnboundedSender<M>) -> Self {
        Self::new(tx)
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn new_from_sender(tx: mpsc::UnboundedSender<M>) -> Self {
        Self::new(tx)
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

    /// Create from an existing sender (used by ActorRuntime).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn new_from_sender(tx: mpsc::UnboundedSender<M>) -> Self {
        Self::new(tx)
    }

    /// Create from an existing sender (used by ActorRuntime).
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn new_from_sender(tx: mpsc::UnboundedSender<M>) -> Self {
        Self::new(tx)
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
