//! Actor wrapper for the server's channel-based room API.
//!
//! Wraps `Room<T>` to provide an event-driven actor model.
//!
//! # Example
//!
//! ```ignore
//! use multiplayer_kit_helpers::{with_actor, RoomContext, RoomEvent};
//! use multiplayer_kit_protocol::Route;
//!
//! Server::builder()
//!     .room_handler(with_actor(|mut ctx: RoomContext<MyUser>| async move {
//!         loop {
//!             match ctx.recv().await {
//!                 Some(RoomEvent::Message { sender, channel, payload }) => {
//!                     // Broadcast to all except sender
//!                     ctx.send(Outgoing::new(payload, Route::AllExcept(channel))).await;
//!                 }
//!                 Some(RoomEvent::Shutdown) | None => break,
//!                 _ => {}
//!             }
//!         }
//!     }))
//!     .build()
//! ```

use multiplayer_kit_protocol::{ChannelId, RoomId, UserContext};
use multiplayer_kit_server::{Accept, Room, RoomHandle, ServerChannel};
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use tokio::sync::mpsc;

/// Events delivered to a room actor.
#[derive(Debug, Clone)]
pub enum RoomEvent<T: UserContext> {
    /// A user joined the room (first channel opened).
    UserJoined(T),
    /// A user left the room (last channel closed).
    UserLeft(T),
    /// A channel was opened by a user.
    ChannelOpened { user: T, channel: ChannelId },
    /// A channel was closed.
    ChannelClosed { user: T, channel: ChannelId },
    /// Raw data received on a channel.
    Message {
        sender: T,
        channel: ChannelId,
        /// Raw bytes from the channel.
        payload: Vec<u8>,
    },
    /// Room is shutting down. Clean up and exit.
    Shutdown,
}

/// Outgoing message with routing information.
#[derive(Debug, Clone)]
pub struct Outgoing {
    pub payload: Vec<u8>,
    pub route: Route,
}

impl Outgoing {
    pub fn new(payload: Vec<u8>, route: Route) -> Self {
        Self { payload, route }
    }
}

/// How to route an outgoing message.
#[derive(Debug, Clone)]
pub enum Route {
    /// Broadcast to all channels.
    All,
    /// Broadcast to all except specified channel.
    AllExcept(ChannelId),
    /// Send to specific channels only.
    Channels(Vec<ChannelId>),
}

/// Context for a room actor.
///
/// Provides event reception and message sending.
pub struct RoomContext<T: UserContext> {
    /// Room ID.
    pub room_id: RoomId,
    /// Event receiver.
    events: mpsc::Receiver<RoomEvent<T>>,
    /// Room handle for broadcasting.
    handle: RoomHandle<T>,
    /// Track open channels.
    channels: HashSet<ChannelId>,
}

impl<T: UserContext> RoomContext<T> {
    /// Receive the next event.
    pub async fn recv(&mut self) -> Option<RoomEvent<T>> {
        let event = self.events.recv().await?;
        
        // Track channel state
        match &event {
            RoomEvent::ChannelOpened { channel, .. } => {
                self.channels.insert(*channel);
            }
            RoomEvent::ChannelClosed { channel, .. } => {
                self.channels.remove(channel);
            }
            _ => {}
        }
        
        Some(event)
    }

    /// Get all open channel IDs.
    pub fn channels(&self) -> impl Iterator<Item = &ChannelId> {
        self.channels.iter()
    }

    /// Send a message with routing.
    pub async fn send(&self, msg: Outgoing) {
        match msg.route {
            Route::All => {
                self.handle.broadcast(&msg.payload).await;
            }
            Route::AllExcept(exclude) => {
                self.handle.broadcast_except(exclude, &msg.payload).await;
            }
            Route::Channels(targets) => {
                self.handle.send_to(&targets, &msg.payload).await;
            }
        }
    }
}

/// Wrap a room actor function to work with the channel-based Room API.
///
/// Takes a function that receives a `RoomContext` and returns a future,
/// and returns a function suitable for `Server::builder().room_handler()`.
pub fn with_actor<T, F, Fut>(
    actor_fn: F,
) -> impl Fn(Room<T>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static
where
    T: UserContext,
    F: Fn(RoomContext<T>) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    move |room: Room<T>| {
        let actor_fn = actor_fn.clone();
        Box::pin(async move {
            run_actor(room, actor_fn).await;
        })
    }
}

/// Run the actor loop, converting channels to events.
async fn run_actor<T, F, Fut>(mut room: Room<T>, actor_fn: F)
where
    T: UserContext,
    F: Fn(RoomContext<T>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    // Create event channel
    let (event_tx, event_rx) = mpsc::channel::<RoomEvent<T>>(256);

    // Create context for actor
    let ctx = RoomContext {
        room_id: room.room_id(),
        events: event_rx,
        handle: room.handle(),
        channels: HashSet::new(),
    };

    // Spawn actor
    let actor_task = tokio::spawn(async move {
        actor_fn(ctx).await;
    });

    // Track users (for UserJoined/UserLeft events)
    let mut user_channels: std::collections::HashMap<T::Id, (T, HashSet<ChannelId>)> =
        std::collections::HashMap::new();

    // Accept channels and forward to actor
    loop {
        match room.accept().await {
            Some(Accept::NewChannel(user, channel)) => {
                let channel_id = channel.id;
                let user_id = user.id();

                // Track for UserJoined/UserLeft
                let is_new_user = !user_channels.contains_key(&user_id);
                user_channels
                    .entry(user_id)
                    .or_insert_with(|| (user.clone(), HashSet::new()))
                    .1
                    .insert(channel_id);

                // Send UserJoined if new
                if is_new_user {
                    let _ = event_tx.send(RoomEvent::UserJoined(user.clone())).await;
                }

                // Send ChannelOpened
                let _ = event_tx
                    .send(RoomEvent::ChannelOpened {
                        user: user.clone(),
                        channel: channel_id,
                    })
                    .await;

                // Spawn task to read from channel and send events
                let tx = event_tx.clone();
                let user_clone = user.clone();
                tokio::spawn(async move {
                    handle_channel_reads(channel, user_clone, tx).await;
                });
            }
            Some(Accept::Closing) => {
                // Send shutdown to actor
                let _ = event_tx.send(RoomEvent::Shutdown).await;
                break;
            }
            None => {
                // Channel closed unexpectedly
                break;
            }
        }
    }

    // Wait for actor to finish
    let _ = actor_task.await;
}

/// Read from a channel and forward to event queue.
async fn handle_channel_reads<T: UserContext>(
    mut channel: ServerChannel,
    user: T,
    event_tx: mpsc::Sender<RoomEvent<T>>,
) {
    let channel_id = channel.id;

    while let Some(data) = channel.read().await {
        let event = RoomEvent::Message {
            sender: user.clone(),
            channel: channel_id,
            payload: data,
        };
        if event_tx.send(event).await.is_err() {
            break; // Actor gone
        }
    }

    // Send ChannelClosed (UserLeft is handled by the main loop tracking)
    let _ = event_tx
        .send(RoomEvent::ChannelClosed {
            user,
            channel: channel_id,
        })
        .await;
}

// ============================================================================
// Message-framing wrapper (on top of actor)
// ============================================================================

use crate::framing::{frame_message, MessageBuffer};
use std::collections::HashMap;

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
    inner: RoomContext<T>,
    /// Per-channel message buffers for reassembly.
    buffers: HashMap<ChannelId, MessageBuffer>,
    /// All open channels.
    open_channels: HashSet<ChannelId>,
    /// Pending complete messages to deliver.
    pending: Vec<MessageEvent<T>>,
}

impl<T: UserContext> MessageContext<T> {
    /// Create a new MessageContext wrapping a RoomContext.
    pub fn new(inner: RoomContext<T>) -> Self {
        let room_id = inner.room_id;
        Self {
            room_id,
            inner,
            buffers: HashMap::new(),
            open_channels: HashSet::new(),
            pending: Vec::new(),
        }
    }

    /// Get the room ID.
    pub fn room_id(&self) -> RoomId {
        self.room_id
    }

    /// Get all currently open channel IDs.
    pub fn channels(&self) -> impl Iterator<Item = &ChannelId> {
        self.open_channels.iter()
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
            let raw_event = self.inner.recv().await?;

            match raw_event {
                RoomEvent::UserJoined(user) => {
                    return Some(MessageEvent::UserJoined(user));
                }
                RoomEvent::UserLeft(user) => {
                    return Some(MessageEvent::UserLeft(user));
                }
                RoomEvent::ChannelOpened { user, channel } => {
                    self.open_channels.insert(channel);
                    self.buffers.insert(channel, MessageBuffer::new());
                    return Some(MessageEvent::ChannelOpened { user, channel });
                }
                RoomEvent::ChannelClosed { user, channel } => {
                    self.open_channels.remove(&channel);
                    self.buffers.remove(&channel);
                    return Some(MessageEvent::ChannelClosed { user, channel });
                }
                RoomEvent::Message {
                    sender,
                    channel,
                    payload,
                } => {
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
/// and returns a closure suitable for use with `with_actor`.
pub fn with_framing<T, F, Fut>(
    actor_fn: F,
) -> impl Fn(RoomContext<T>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + Clone + 'static
where
    T: UserContext,
    F: Fn(MessageContext<T>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let actor_fn = std::sync::Arc::new(actor_fn);
    move |ctx: RoomContext<T>| {
        let actor_fn = std::sync::Arc::clone(&actor_fn);
        Box::pin(async move {
            let msg_ctx = MessageContext::new(ctx);
            actor_fn(msg_ctx).await;
        })
    }
}
