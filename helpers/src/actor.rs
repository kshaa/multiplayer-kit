//! Message-framed actor wrapper for the server.
//!
//! Wraps the raw `RoomContext` and handles framing transparently.
//!
//! # Example
//!
//! ```ignore
//! use multiplayer_kit_helpers::{with_framing, MessageContext, MessageEvent};
//! use multiplayer_kit_protocol::{Outgoing, Route};
//!
//! Server::builder()
//!     .room_actor(with_framing(|mut ctx: MessageContext<MyUser>| async move {
//!         loop {
//!             match ctx.recv().await {
//!                 Some(MessageEvent::Message { channel, data, .. }) => {
//!                     // `data` is a complete message, already unframed
//!                     let targets = ctx.channels().filter(|&&c| c != channel).copied().collect();
//!                     ctx.send(Outgoing::new(data, Route::Channels(targets))).await;
//!                 }
//!                 Some(MessageEvent::Shutdown) | None => break,
//!                 _ => {}
//!             }
//!         }
//!     }))
//!     .build()
//! ```

use crate::framing::{frame_message, MessageBuffer};
use multiplayer_kit_protocol::{ChannelId, Outgoing, RoomEvent, RoomId, UserContext};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;

/// Events delivered to a message-based actor.
/// 
/// Unlike `RoomEvent`, the `Message` variant contains complete, unframed messages.
#[derive(Debug, Clone)]
pub enum MessageEvent<T: UserContext> {
    /// A user joined the room (first channel opened).
    UserJoined(T),
    /// A user left the room (last channel closed).
    UserLeft(T),
    /// A channel was opened by a user.
    ChannelOpened { user: T, channel: ChannelId },
    /// A channel was closed.
    ChannelClosed { user: T, channel: ChannelId },
    /// A complete message was received on a channel.
    Message {
        sender: T,
        channel: ChannelId,
        /// The complete, unframed message data.
        data: Vec<u8>,
    },
    /// Room is shutting down.
    Shutdown,
}

/// Context for a message-based actor.
/// 
/// Wraps `RoomContext` and handles message framing transparently.
pub struct MessageContext<T: UserContext> {
    room_id: RoomId,
    inner: multiplayer_kit_server::RoomContext<T>,
    /// Per-channel message buffers for reassembly.
    buffers: HashMap<ChannelId, MessageBuffer>,
    /// All open channels.
    channels: HashSet<ChannelId>,
    /// Pending complete messages to deliver.
    pending: Vec<MessageEvent<T>>,
}

impl<T: UserContext> MessageContext<T> {
    /// Create a new MessageContext wrapping a RoomContext.
    pub fn new(inner: multiplayer_kit_server::RoomContext<T>) -> Self {
        let room_id = inner.room_id;
        Self {
            room_id,
            inner,
            buffers: HashMap::new(),
            channels: HashSet::new(),
            pending: Vec::new(),
        }
    }

    /// Get the room ID.
    pub fn room_id(&self) -> RoomId {
        self.room_id
    }

    /// Get all currently open channel IDs.
    pub fn channels(&self) -> impl Iterator<Item = &ChannelId> {
        self.channels.iter()
    }

    /// Receive the next message event.
    /// 
    /// Returns `None` when the event channel is closed.
    pub async fn recv(&mut self) -> Option<MessageEvent<T>> {
        // Return pending event if available
        if let Some(event) = self.pending.pop() {
            return Some(event);
        }

        loop {
            let raw_event = self.inner.events.recv().await?;

            match raw_event {
                RoomEvent::UserJoined(user) => {
                    return Some(MessageEvent::UserJoined(user));
                }
                RoomEvent::UserLeft(user) => {
                    return Some(MessageEvent::UserLeft(user));
                }
                RoomEvent::ChannelOpened { user, channel } => {
                    self.channels.insert(channel);
                    self.buffers.insert(channel, MessageBuffer::new());
                    return Some(MessageEvent::ChannelOpened { user, channel });
                }
                RoomEvent::ChannelClosed { user, channel } => {
                    self.channels.remove(&channel);
                    self.buffers.remove(&channel);
                    return Some(MessageEvent::ChannelClosed { user, channel });
                }
                RoomEvent::Message { sender, channel, payload } => {
                    // Buffer and extract complete messages
                    let buffer = self.buffers.entry(channel).or_default();
                    
                    for result in buffer.push(&payload) {
                        match result {
                            Ok(data) => {
                                self.pending.push(MessageEvent::Message {
                                    sender: sender.clone(),
                                    channel,
                                    data,
                                });
                            }
                            Err(e) => {
                                tracing::warn!("Framing error on channel {:?}: {}", channel, e);
                            }
                        }
                    }

                    // Return first pending message if we got any
                    if let Some(event) = self.pending.pop() {
                        return Some(event);
                    }
                    // Otherwise continue to wait for more data
                }
                RoomEvent::Shutdown => {
                    return Some(MessageEvent::Shutdown);
                }
            }
        }
    }

    /// Send a message (auto-framed).
    pub async fn send(&self, msg: Outgoing) {
        let framed = Outgoing::new(frame_message(&msg.payload), msg.route);
        self.inner.send(framed).await;
    }
}

/// Wrap a message-based actor function with framing.
///
/// Takes a function that receives a `MessageContext` and returns a future,
/// and returns a function suitable for `Server::builder().room_actor()`.
pub fn with_framing<T, F, Fut>(actor_fn: F) -> impl Fn(multiplayer_kit_server::RoomContext<T>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static
where
    T: UserContext,
    F: Fn(MessageContext<T>) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    move |ctx: multiplayer_kit_server::RoomContext<T>| {
        let actor_fn = actor_fn.clone();
        Box::pin(async move {
            let msg_ctx = MessageContext::new(ctx);
            actor_fn(msg_ctx).await;
        })
    }
}
