//! Room management with actor model and channel-based routing.

use crate::lobby::Lobby;
use dashmap::DashMap;
use multiplayer_kit_protocol::{ChannelId, Outgoing, RoomEvent, RoomId, RoomInfo, Route, UserContext};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};

/// Type alias for the actor factory function.
/// Called when a new room is created - should return a future that runs the actor.
pub type ActorFactory<T> = Arc<
    dyn Fn(RoomContext<T>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync,
>;

/// Context provided to room actors.
pub struct RoomContext<T: UserContext> {
    /// The room ID.
    pub room_id: RoomId,
    /// Receiver for room events (join, leave, channel open/close, message, shutdown).
    pub events: mpsc::Receiver<RoomEvent<T>>,
    /// Sender for outgoing messages.
    outbox: OutboxSender,
}

impl<T: UserContext> RoomContext<T> {
    /// Send a message to channels.
    pub async fn send(&self, msg: Outgoing) {
        let _ = self.outbox.tx.send(msg).await;
    }

    /// Send multiple messages.
    pub async fn send_all(&self, msgs: impl IntoIterator<Item = Outgoing>) {
        for msg in msgs {
            self.send(msg).await;
        }
    }
}

/// Internal sender for outbox messages.
#[derive(Clone)]
struct OutboxSender {
    tx: mpsc::Sender<Outgoing>,
}

/// Manages all active rooms.
pub struct RoomManager<T: UserContext> {
    rooms: DashMap<RoomId, Arc<Room<T>>>,
    next_room_id: AtomicU64,
    config: RoomConfig,
    actor_factory: ActorFactory<T>,
}

#[derive(Clone)]
pub struct RoomConfig {
    pub max_lifetime: Duration,
    pub first_connect_timeout: Duration,
    pub empty_timeout: Duration,
}

/// A single room instance.
pub struct Room<T: UserContext> {
    pub id: RoomId,
    pub creator_id: T::Id,
    pub created_at: Instant,
    pub metadata: Option<serde_json::Value>,
    /// Send events to the actor.
    event_tx: mpsc::Sender<RoomEvent<T>>,
    /// Channels indexed by ChannelId.
    channels: RwLock<HashMap<ChannelId, Channel<T>>>,
    /// Track which users have channels open (for UserJoined/UserLeft events).
    user_channels: RwLock<HashMap<T::Id, HashSet<ChannelId>>>,
    /// Next channel ID.
    next_channel_id: AtomicU64,
    /// Number of unique users (not channels).
    user_count: AtomicU32,
    last_activity: RwLock<Instant>,
    /// Receive outgoing messages from actor.
    outbox_rx: RwLock<Option<mpsc::Receiver<Outgoing>>>,
}

/// A channel (bidirectional stream) in a room.
pub struct Channel<T: UserContext> {
    pub id: ChannelId,
    pub user: T,
    pub opened_at: Instant,
    /// Sender to write messages to this channel.
    pub msg_tx: mpsc::Sender<Vec<u8>>,
}

impl<T: UserContext> RoomManager<T> {
    pub fn new(config: RoomConfig, actor_factory: ActorFactory<T>) -> Self {
        Self {
            rooms: DashMap::new(),
            next_room_id: AtomicU64::new(1),
            config,
            actor_factory,
        }
    }

    /// Create a new room and spawn its actor.
    pub fn create_room(&self, creator: &T, metadata: Option<serde_json::Value>) -> RoomId {
        let id = RoomId(self.next_room_id.fetch_add(1, Ordering::SeqCst));

        // Create channels for actor communication
        let (event_tx, event_rx) = mpsc::channel::<RoomEvent<T>>(256);
        let (outbox_tx, outbox_rx) = mpsc::channel::<Outgoing>(256);

        let room = Arc::new(Room {
            id,
            creator_id: creator.id(),
            created_at: Instant::now(),
            metadata,
            event_tx,
            channels: RwLock::new(HashMap::new()),
            user_channels: RwLock::new(HashMap::new()),
            next_channel_id: AtomicU64::new(1),
            user_count: AtomicU32::new(0),
            last_activity: RwLock::new(Instant::now()),
            outbox_rx: RwLock::new(Some(outbox_rx)),
        });

        // Create actor context
        let ctx = RoomContext {
            room_id: id,
            events: event_rx,
            outbox: OutboxSender { tx: outbox_tx },
        };

        // Spawn the actor
        let actor_future = (self.actor_factory)(ctx);
        tokio::spawn(actor_future);

        self.rooms.insert(id, room);
        id
    }

    /// Delete a room (creator only).
    pub async fn delete_room(&self, room_id: RoomId, requester_id: &T::Id) -> Result<(), &'static str> {
        if let Some(room) = self.rooms.get(&room_id) {
            if &room.creator_id == requester_id {
                // Send shutdown to actor
                let _ = room.event_tx.send(RoomEvent::Shutdown).await;
                drop(room);
                self.rooms.remove(&room_id);
                Ok(())
            } else {
                Err("only creator can delete room")
            }
        } else {
            Err("room not found")
        }
    }

    /// Get room info for lobby.
    pub fn get_room_info(&self, room_id: RoomId) -> Option<RoomInfo> {
        self.rooms.get(&room_id).map(|room| RoomInfo {
            id: room.id,
            player_count: room.user_count.load(Ordering::Relaxed),
            max_players: None,
            created_at: room.created_at.elapsed().as_secs(),
            metadata: room.metadata.clone(),
        })
    }

    /// Get all rooms for lobby snapshot.
    pub fn get_all_rooms(&self) -> Vec<RoomInfo> {
        self.rooms
            .iter()
            .filter_map(|entry| self.get_room_info(*entry.key()))
            .collect()
    }

    /// Get a room by ID.
    pub fn get_room(&self, room_id: RoomId) -> Option<Arc<Room<T>>> {
        self.rooms.get(&room_id).map(|r| Arc::clone(&r))
    }

    /// Run lifecycle checks (call periodically).
    pub async fn run_lifecycle_checks(&self, lobby: &Lobby) {
        let now = Instant::now();
        let mut to_remove = Vec::new();

        for entry in self.rooms.iter() {
            let room = entry.value();
            let age = now.duration_since(room.created_at);
            let user_count = room.user_count.load(Ordering::Relaxed);
            let last_activity = *room.last_activity.read().await;
            let idle_time = now.duration_since(last_activity);

            // Max lifetime exceeded
            if age > self.config.max_lifetime {
                to_remove.push(*entry.key());
                continue;
            }

            // No one ever connected
            if user_count == 0 && age > self.config.first_connect_timeout {
                to_remove.push(*entry.key());
                continue;
            }

            // Was active but now empty
            if user_count == 0 && idle_time > self.config.empty_timeout {
                to_remove.push(*entry.key());
            }
        }

        for room_id in to_remove {
            if let Some((_, room)) = self.rooms.remove(&room_id) {
                // Send shutdown to actor
                let _ = room.event_tx.send(RoomEvent::Shutdown).await;
            }
            lobby.notify_room_deleted(room_id);
            tracing::info!(?room_id, "Room expired and removed");
        }
    }
}

impl<T: UserContext> Room<T> {
    /// Open a new channel for a user. Returns the ChannelId.
    /// Fires UserJoined if this is the user's first channel.
    /// Fires ChannelOpened for the new channel.
    pub async fn open_channel(&self, user: T, msg_tx: mpsc::Sender<Vec<u8>>) -> ChannelId {
        let channel_id = ChannelId(self.next_channel_id.fetch_add(1, Ordering::SeqCst));
        let user_id = user.id();

        // Check if this is the user's first channel (new user joining)
        let is_new_user = {
            let mut user_channels = self.user_channels.write().await;
            let channels = user_channels.entry(user_id.clone()).or_default();
            let is_new = channels.is_empty();
            channels.insert(channel_id);
            is_new
        };

        // Fire UserJoined if new user
        if is_new_user {
            self.user_count.fetch_add(1, Ordering::Relaxed);
            let _ = self.event_tx.send(RoomEvent::UserJoined(user.clone())).await;
        }

        // Fire ChannelOpened
        let _ = self.event_tx.send(RoomEvent::ChannelOpened {
            user: user.clone(),
            channel: channel_id,
        }).await;

        // Store the channel
        let mut channels = self.channels.write().await;
        channels.insert(channel_id, Channel {
            id: channel_id,
            user,
            opened_at: Instant::now(),
            msg_tx,
        });

        *self.last_activity.write().await = Instant::now();
        channel_id
    }

    /// Close a channel.
    /// Fires ChannelClosed.
    /// Fires UserLeft if this was the user's last channel.
    pub async fn close_channel(&self, channel_id: ChannelId) {
        let removed = {
            let mut channels = self.channels.write().await;
            channels.remove(&channel_id)
        };

        if let Some(channel) = removed {
            let user_id = channel.user.id();

            // Fire ChannelClosed
            let _ = self.event_tx.send(RoomEvent::ChannelClosed {
                user: channel.user.clone(),
                channel: channel_id,
            }).await;

            // Check if this was the user's last channel
            let is_last_channel = {
                let mut user_channels = self.user_channels.write().await;
                if let Some(channels) = user_channels.get_mut(&user_id) {
                    channels.remove(&channel_id);
                    channels.is_empty()
                } else {
                    true
                }
            };

            // Fire UserLeft if last channel
            if is_last_channel {
                self.user_count.fetch_sub(1, Ordering::Relaxed);
                let _ = self.event_tx.send(RoomEvent::UserLeft(channel.user)).await;

                // Clean up user_channels entry
                let mut user_channels = self.user_channels.write().await;
                user_channels.remove(&user_id);
            }

            *self.last_activity.write().await = Instant::now();
        }
    }

    /// Send a message event to the actor.
    pub async fn send_message(&self, sender: T, channel: ChannelId, payload: Vec<u8>) {
        let _ = self.event_tx.send(RoomEvent::Message { sender, channel, payload }).await;
        *self.last_activity.write().await = Instant::now();
    }

    /// Take the outbox receiver (only call once, when setting up the broadcast task).
    pub async fn take_outbox_rx(&self) -> Option<mpsc::Receiver<Outgoing>> {
        self.outbox_rx.write().await.take()
    }

    /// Send a message to channels according to the routing decision.
    pub async fn send_to_channels(&self, payload: &[u8], route: Route) {
        let channels = self.channels.read().await;

        match route {
            Route::Channels(ref channel_ids) => {
                for channel_id in channel_ids {
                    if let Some(channel) = channels.get(channel_id) {
                        let _ = channel.msg_tx.send(payload.to_vec()).await;
                    }
                }
            }
            Route::None => {
                // Don't send
            }
        }
    }

    /// Get all current channel IDs.
    pub async fn get_all_channel_ids(&self) -> Vec<ChannelId> {
        let channels = self.channels.read().await;
        channels.keys().copied().collect()
    }

    /// Get channel IDs for a specific user.
    pub async fn get_user_channel_ids(&self, user_id: &T::Id) -> Vec<ChannelId> {
        let user_channels = self.user_channels.read().await;
        user_channels.get(user_id).map(|s| s.iter().copied().collect()).unwrap_or_default()
    }
}
