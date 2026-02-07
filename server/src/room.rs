//! Room management with channel-based architecture.
//!
//! Rooms accept channels (connections) and provide broadcast capabilities.
//! The actor pattern is optional and lives in helpers.

use crate::lobby::Lobby;
use dashmap::DashMap;
use multiplayer_kit_protocol::{ChannelId, RoomId, RoomInfo, UserContext};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot, RwLock};

/// Type alias for the room handler factory function.
pub type RoomHandlerFactory<T> = Arc<
    dyn Fn(Room<T>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync,
>;

// ============================================================================
// ServerChannel - bidirectional channel to a single client
// ============================================================================

/// A channel (bidirectional stream) to a single client.
///
/// This is the handler's view of a channel. Read data from the client,
/// write data to the client.
pub struct ServerChannel {
    /// Unique channel ID.
    pub id: ChannelId,
    /// Read data sent by the client.
    read_rx: mpsc::Receiver<Vec<u8>>,
    /// Write data to send to the client.
    write_tx: mpsc::Sender<Vec<u8>>,
    /// Signal to close this channel.
    close_tx: Option<oneshot::Sender<()>>,
}

impl ServerChannel {
    /// Read data from this channel.
    /// Returns `None` when the channel is closed (client disconnected or kicked).
    pub async fn read(&mut self) -> Option<Vec<u8>> {
        self.read_rx.recv().await
    }

    /// Write data to this channel.
    pub async fn write(&self, data: &[u8]) -> Result<(), ChannelError> {
        self.write_tx
            .send(data.to_vec())
            .await
            .map_err(|_| ChannelError::Closed)
    }

    /// Close this channel (kick the client).
    pub fn close(mut self) {
        if let Some(tx) = self.close_tx.take() {
            let _ = tx.send(());
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    #[error("channel closed")]
    Closed,
}

// ============================================================================
// Room - the handler's view of a room
// ============================================================================

/// What `room.accept()` returns.
pub enum Accept<T: UserContext> {
    /// A new channel connected.
    NewChannel(T, ServerChannel),
    /// Room is being closed externally. Do cleanup and return.
    Closing,
}

/// A room that accepts channels and provides broadcast.
///
/// This is what the room handler receives. Accept new channels,
/// read/write to them, broadcast to all.
pub struct Room<T: UserContext> {
    /// Room ID.
    pub id: RoomId,
    /// Receive new channels.
    channel_rx: mpsc::Receiver<(T, ServerChannel)>,
    /// Shutdown signal.
    shutdown_rx: oneshot::Receiver<()>,
    /// Shared state for broadcasting.
    shared: Arc<RoomShared<T>>,
}

/// Shared room state (for cloning to channel tasks).
pub struct RoomShared<T: UserContext> {
    /// All active channels for broadcasting.
    channels: RwLock<HashMap<ChannelId, ChannelBroadcast>>,
    /// Track which users have channels (for user count).
    user_channels: RwLock<HashMap<T::Id, HashSet<ChannelId>>>,
    /// User count.
    user_count: AtomicU32,
    /// Last activity time.
    last_activity: RwLock<Instant>,
}

/// Broadcast handle for a channel.
struct ChannelBroadcast {
    write_tx: mpsc::Sender<Vec<u8>>,
}

impl<T: UserContext> Room<T> {
    /// Accept a new channel or shutdown signal.
    ///
    /// Returns `Some(Accept::NewChannel(...))` when a client connects.
    /// Returns `Some(Accept::Closing)` when the room is being shut down.
    /// Returns `None` if already shut down.
    pub async fn accept(&mut self) -> Option<Accept<T>> {
        tokio::select! {
            biased;
            _ = &mut self.shutdown_rx => Some(Accept::Closing),
            result = self.channel_rx.recv() => {
                result.map(|(user, channel)| Accept::NewChannel(user, channel))
            }
        }
    }

    /// Get the room ID.
    pub fn room_id(&self) -> RoomId {
        self.id
    }

    /// Get a handle for broadcasting (can be cloned to spawned tasks).
    pub fn handle(&self) -> RoomHandle<T> {
        RoomHandle {
            shared: Arc::clone(&self.shared),
        }
    }

    /// Broadcast to all channels.
    pub async fn broadcast(&self, data: &[u8]) {
        self.shared.broadcast(data).await;
    }

    /// Broadcast to all channels except one.
    pub async fn broadcast_except(&self, exclude: ChannelId, data: &[u8]) {
        self.shared.broadcast_except(exclude, data).await;
    }

    /// Broadcast to specific channels.
    pub async fn send_to(&self, channels: &[ChannelId], data: &[u8]) {
        self.shared.send_to(channels, data).await;
    }

    /// Get all channel IDs.
    pub async fn channel_ids(&self) -> Vec<ChannelId> {
        self.shared.channels.read().await.keys().copied().collect()
    }

    /// Get current user count.
    pub fn user_count(&self) -> u32 {
        self.shared.user_count.load(Ordering::Relaxed)
    }
}

/// Cloneable handle for room operations (for use in spawned tasks).
#[derive(Clone)]
pub struct RoomHandle<T: UserContext> {
    shared: Arc<RoomShared<T>>,
}

impl<T: UserContext> RoomHandle<T> {
    /// Broadcast to all channels.
    pub async fn broadcast(&self, data: &[u8]) {
        self.shared.broadcast(data).await;
    }

    /// Broadcast to all channels except one.
    pub async fn broadcast_except(&self, exclude: ChannelId, data: &[u8]) {
        self.shared.broadcast_except(exclude, data).await;
    }

    /// Broadcast to specific channels.
    pub async fn send_to(&self, channels: &[ChannelId], data: &[u8]) {
        self.shared.send_to(channels, data).await;
    }

    /// Get all channel IDs.
    pub async fn channel_ids(&self) -> Vec<ChannelId> {
        self.shared.channels.read().await.keys().copied().collect()
    }
}

impl<T: UserContext> RoomShared<T> {
    async fn broadcast(&self, data: &[u8]) {
        let channels = self.channels.read().await;
        for channel in channels.values() {
            let _ = channel.write_tx.send(data.to_vec()).await;
        }
    }

    async fn broadcast_except(&self, exclude: ChannelId, data: &[u8]) {
        let channels = self.channels.read().await;
        for (id, channel) in channels.iter() {
            if *id != exclude {
                let _ = channel.write_tx.send(data.to_vec()).await;
            }
        }
    }

    async fn send_to(&self, channel_ids: &[ChannelId], data: &[u8]) {
        let channels = self.channels.read().await;
        for id in channel_ids {
            if let Some(channel) = channels.get(id) {
                let _ = channel.write_tx.send(data.to_vec()).await;
            }
        }
    }
}

// ============================================================================
// RoomManager - manages all rooms
// ============================================================================

/// Manages all active rooms.
pub struct RoomManager<T: UserContext> {
    rooms: DashMap<RoomId, RoomEntry<T>>,
    next_room_id: AtomicU64,
    config: RoomConfig,
    handler_factory: RoomHandlerFactory<T>,
}

/// Internal room entry with control handles.
struct RoomEntry<T: UserContext> {
    /// Send new channels to the room handler.
    channel_tx: mpsc::Sender<(T, ServerChannel)>,
    /// Signal shutdown.
    shutdown_tx: Option<oneshot::Sender<()>>,
    /// Shared state for external access.
    shared: Arc<RoomShared<T>>,
    /// Creator ID.
    creator_id: T::Id,
    /// Creation time.
    created_at: Instant,
    /// Metadata.
    metadata: Option<serde_json::Value>,
    /// Next channel ID.
    next_channel_id: AtomicU64,
}

#[derive(Clone)]
pub struct RoomConfig {
    pub max_lifetime: Duration,
    pub first_connect_timeout: Duration,
    pub empty_timeout: Duration,
}

impl<T: UserContext> RoomManager<T> {
    pub fn new(config: RoomConfig, handler_factory: RoomHandlerFactory<T>) -> Self {
        Self {
            rooms: DashMap::new(),
            next_room_id: AtomicU64::new(1),
            config,
            handler_factory,
        }
    }

    /// Create a new room and spawn its handler.
    pub fn create_room(&self, creator: &T, metadata: Option<serde_json::Value>) -> RoomId {
        let id = RoomId(self.next_room_id.fetch_add(1, Ordering::SeqCst));

        // Create channels for room handler
        let (channel_tx, channel_rx) = mpsc::channel::<(T, ServerChannel)>(256);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let shared = Arc::new(RoomShared {
            channels: RwLock::new(HashMap::new()),
            user_channels: RwLock::new(HashMap::new()),
            user_count: AtomicU32::new(0),
            last_activity: RwLock::new(Instant::now()),
        });

        // Create room for handler
        let room = Room {
            id,
            channel_rx,
            shutdown_rx,
            shared: Arc::clone(&shared),
        };

        // Store entry
        let entry = RoomEntry {
            channel_tx,
            shutdown_tx: Some(shutdown_tx),
            shared,
            creator_id: creator.id(),
            created_at: Instant::now(),
            metadata,
            next_channel_id: AtomicU64::new(1),
        };
        self.rooms.insert(id, entry);

        // Spawn handler
        let handler_future = (self.handler_factory)(room);
        tokio::spawn(handler_future);

        id
    }

    /// Delete a room (creator only).
    pub async fn delete_room(&self, room_id: RoomId, requester_id: &T::Id) -> Result<(), &'static str> {
        if let Some(mut entry) = self.rooms.get_mut(&room_id) {
            if &entry.creator_id != requester_id {
                return Err("only creator can delete room");
            }

            // Send shutdown signal for graceful close
            if let Some(tx) = entry.shutdown_tx.take() {
                let _ = tx.send(());
            }

            // Give handler time to clean up (brief yield)
            tokio::task::yield_now().await;

            // Force close all channels
            let channels = entry.shared.channels.write().await;
            drop(channels); // Just drop the lock, senders will be dropped when entry is removed

            drop(entry);
            self.rooms.remove(&room_id);
            Ok(())
        } else {
            Err("room not found")
        }
    }

    /// Open a channel in a room. Called by QUIC/WS handlers.
    /// Returns (write_rx, close_rx) for the transport handler.
    pub async fn open_channel(
        &self,
        room_id: RoomId,
        user: T,
    ) -> Option<(ChannelId, mpsc::Sender<Vec<u8>>, mpsc::Receiver<Vec<u8>>, oneshot::Receiver<()>)> {
        let entry = self.rooms.get(&room_id)?;

        let channel_id = ChannelId(entry.next_channel_id.fetch_add(1, Ordering::SeqCst));
        let user_id = user.id();

        // Create channel pairs
        let (read_tx, read_rx) = mpsc::channel::<Vec<u8>>(256);   // Transport → Handler
        let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>(256); // Handler → Transport
        let (close_tx, close_rx) = oneshot::channel();

        // Track user for user count
        {
            let mut user_channels = entry.shared.user_channels.write().await;
            let is_new_user = !user_channels.contains_key(&user_id);
            user_channels.entry(user_id).or_default().insert(channel_id);
            if is_new_user {
                entry.shared.user_count.fetch_add(1, Ordering::Relaxed);
            }
        }

        // Add to broadcast list
        {
            let mut channels = entry.shared.channels.write().await;
            channels.insert(channel_id, ChannelBroadcast {
                write_tx: write_tx.clone(),
            });
        }

        // Create ServerChannel for handler
        let server_channel = ServerChannel {
            id: channel_id,
            read_rx,
            write_tx,
            close_tx: Some(close_tx),
        };

        // Send to handler
        if entry.channel_tx.send((user, server_channel)).await.is_err() {
            // Handler gone, clean up
            return None;
        }

        *entry.shared.last_activity.write().await = Instant::now();

        Some((channel_id, read_tx, write_rx, close_rx))
    }

    /// Close a channel. Called by QUIC/WS handlers when connection drops.
    pub async fn close_channel(&self, room_id: RoomId, channel_id: ChannelId) {
        if let Some(entry) = self.rooms.get(&room_id) {
            // Remove from broadcast list
            {
                let mut channels = entry.shared.channels.write().await;
                channels.remove(&channel_id);
            }

            // Update user tracking
            {
                let mut user_channels = entry.shared.user_channels.write().await;
                let mut user_to_remove = None;

                for (user_id, channels) in user_channels.iter_mut() {
                    if channels.remove(&channel_id) && channels.is_empty() {
                        user_to_remove = Some(user_id.clone());
                        break;
                    }
                }

                if let Some(user_id) = user_to_remove {
                    user_channels.remove(&user_id);
                    entry.shared.user_count.fetch_sub(1, Ordering::Relaxed);
                }
            }

            *entry.shared.last_activity.write().await = Instant::now();
        }
    }

    /// Get room info for lobby.
    pub fn get_room_info(&self, room_id: RoomId) -> Option<RoomInfo> {
        self.rooms.get(&room_id).map(|entry| RoomInfo {
            id: room_id,
            player_count: entry.shared.user_count.load(Ordering::Relaxed),
            max_players: None,
            created_at: entry.created_at.elapsed().as_secs(),
            metadata: entry.metadata.clone(),
        })
    }

    /// Get all rooms for lobby snapshot.
    pub fn get_all_rooms(&self) -> Vec<RoomInfo> {
        self.rooms
            .iter()
            .filter_map(|entry| self.get_room_info(*entry.key()))
            .collect()
    }

    /// Check if a room exists.
    pub fn room_exists(&self, room_id: RoomId) -> bool {
        self.rooms.contains_key(&room_id)
    }

    /// Run lifecycle checks (call periodically).
    pub async fn run_lifecycle_checks(&self, lobby: &Lobby) {
        let now = Instant::now();
        let mut to_remove = Vec::new();

        for entry in self.rooms.iter() {
            let age = now.duration_since(entry.created_at);
            let user_count = entry.shared.user_count.load(Ordering::Relaxed);
            let last_activity = *entry.shared.last_activity.read().await;
            let idle_time = now.duration_since(last_activity);

            if age > self.config.max_lifetime {
                to_remove.push(*entry.key());
                continue;
            }

            if user_count == 0 && age > self.config.first_connect_timeout {
                to_remove.push(*entry.key());
                continue;
            }

            if user_count == 0 && idle_time > self.config.empty_timeout {
                to_remove.push(*entry.key());
            }
        }

        for room_id in to_remove {
            if let Some(mut entry) = self.rooms.get_mut(&room_id) {
                // Send shutdown for graceful close
                if let Some(tx) = entry.shutdown_tx.take() {
                    let _ = tx.send(());
                }
            }
            tokio::task::yield_now().await;
            self.rooms.remove(&room_id);
            lobby.notify_room_deleted(room_id);
            tracing::info!(?room_id, "Room expired and removed");
        }
    }
}
