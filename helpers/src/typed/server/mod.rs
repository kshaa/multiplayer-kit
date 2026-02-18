//! Server-side typed actor for handling game rooms.
//!
//! Provides strongly-typed message handling for server room actors.
//!
//! # Architecture
//!
//! - `TypedEvent<T, P>` - Events delivered to the actor
//! - `TypedContext<T, P, Ctx>` - Context for sending messages and accessing game state
//! - `with_typed_actor` - Wrapper to create a typed actor handler
//!
//! # Example
//!
//! ```ignore
//! .room_handler(with_typed_actor::<
//!     MyUser,
//!     MyProtocol,
//!     MyRoomConfig,
//!     MyGameContext,
//!     _,
//!     _,
//! >(my_actor))
//! ```

mod actor;
mod channel;

use super::TypedProtocol;
use crate::framing::frame_message;
use dashmap::DashMap;
use multiplayer_kit_protocol::{ChannelId, RoomId, UserContext};
use multiplayer_kit_server::GameServerContext;
use std::marker::PhantomData;
use std::sync::Arc;
use tokio::sync::mpsc;

// Re-exports
pub use actor::with_typed_actor;

/// Events delivered to a typed server actor.
#[derive(Debug)]
pub enum TypedEvent<T: UserContext, P: TypedProtocol> {
    /// User connected (all channels established).
    UserConnected(T),
    /// User disconnected (any channel failed or clean disconnect).
    UserDisconnected(T),
    /// Message received.
    Message {
        sender: T,
        /// The channel type (e.g., Chat, GameState).
        channel: P::Channel,
        event: P::Event,
    },
    /// Internal event sent by actor to itself.
    Internal(P::Event),
    /// Room is shutting down.
    Shutdown,
}

/// Context for server-side typed actor.
///
/// Provides access to room operations and the game context for game-specific services.
pub struct TypedContext<T: UserContext, P: TypedProtocol, Ctx: GameServerContext = ()> {
    room_id: RoomId,
    /// Sender to deliver events back to self.
    self_tx: mpsc::Sender<P::Event>,
    handle: multiplayer_kit_server::RoomHandle<T>,
    /// Maps channel_id -> (user, channel_type)
    user_channels: Arc<DashMap<ChannelId, (T, P::Channel)>>,
    /// Game context for game-specific services.
    game_context: Arc<Ctx>,
    _phantom: PhantomData<P>,
}

impl<T: UserContext, P: TypedProtocol, Ctx: GameServerContext> TypedContext<T, P, Ctx> {
    /// Get the room ID.
    pub fn room_id(&self) -> RoomId {
        self.room_id
    }

    /// Get the game context for accessing game-specific services.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let ctx = ctx.game_context();
    /// ctx.history_manager.record_event(&event).await;
    /// ```
    pub fn game_context(&self) -> &Arc<Ctx> {
        &self.game_context
    }

    /// Broadcast event to all channels of the event's type. Non-blocking.
    pub fn broadcast(&self, event: &P::Event) {
        if let Ok((channel_type, data)) = P::encode(event) {
            let framed = frame_message(&data);
            let channel_ids: Vec<_> = self
                .user_channels
                .iter()
                .filter(|r| r.value().1 == channel_type)
                .map(|r| *r.key())
                .collect();
            self.handle.send_to(&channel_ids, &framed);
        }
    }

    /// Broadcast event to all channels except sender's channel. Non-blocking.
    pub fn broadcast_except(&self, exclude: ChannelId, event: &P::Event) {
        if let Ok((channel_type, data)) = P::encode(event) {
            let framed = frame_message(&data);
            let channel_ids: Vec<_> = self
                .user_channels
                .iter()
                .filter(|r| r.value().1 == channel_type && *r.key() != exclude)
                .map(|r| *r.key())
                .collect();
            self.handle.send_to(&channel_ids, &framed);
        }
    }

    /// Send to a specific user (all their channels of the event's type). Non-blocking.
    pub fn send_to_user(&self, user: &T, event: &P::Event) {
        if let Ok((channel_type, data)) = P::encode(event) {
            let framed = frame_message(&data);
            let channel_ids: Vec<_> = self
                .user_channels
                .iter()
                .filter(|r| r.value().1 == channel_type && r.value().0.id() == user.id())
                .map(|r| *r.key())
                .collect();
            self.handle.send_to(&channel_ids, &framed);
        }
    }

    /// Get a sender to deliver events back to the actor itself.
    /// Use this to spawn tasks that can notify the actor later.
    pub fn self_tx(&self) -> mpsc::Sender<P::Event> {
        self.self_tx.clone()
    }
}

impl<T: UserContext, P: TypedProtocol, Ctx: GameServerContext> Clone for TypedContext<T, P, Ctx> {
    fn clone(&self) -> Self {
        Self {
            room_id: self.room_id,
            self_tx: self.self_tx.clone(),
            handle: self.handle.clone(),
            user_channels: self.user_channels.clone(),
            game_context: self.game_context.clone(),
            _phantom: PhantomData,
        }
    }
}

/// Internal event type for actor communication.
pub(super) enum ServerInternalEvent<T: UserContext, P: TypedProtocol> {
    UserReady(T),
    UserGone(T),
    Message {
        sender: T,
        channel_type: P::Channel,
        event: P::Event,
    },
}
