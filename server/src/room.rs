//! Room management with actor model.

use crate::lobby::Lobby;
use dashmap::DashMap;
use multiplayer_kit_protocol::{Outgoing, RoomEvent, RoomId, RoomInfo, Route, UserContext};
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
    /// Receiver for room events (join, leave, message, shutdown).
    pub events: mpsc::Receiver<RoomEvent<T>>,
    /// Sender for outgoing messages.
    outbox: OutboxSender<T>,
}

impl<T: UserContext> RoomContext<T> {
    /// Send a message to participants.
    pub async fn send(&self, msg: Outgoing<T::Id>) {
        let _ = self.outbox.tx.send(msg).await;
    }

    /// Send multiple messages.
    pub async fn send_all(&self, msgs: impl IntoIterator<Item = Outgoing<T::Id>>) {
        for msg in msgs {
            self.send(msg).await;
        }
    }
}

/// Internal sender for outbox messages.
struct OutboxSender<T: UserContext> {
    tx: mpsc::Sender<Outgoing<T::Id>>,
}

impl<T: UserContext> Clone for OutboxSender<T> {
    fn clone(&self) -> Self {
        Self { tx: self.tx.clone() }
    }
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
    /// Participants and their message channels.
    participants: RwLock<Vec<Participant<T>>>,
    participant_count: AtomicU32,
    last_activity: RwLock<Instant>,
    /// Receive outgoing messages from actor.
    outbox_rx: RwLock<Option<mpsc::Receiver<Outgoing<T::Id>>>>,
}

/// A participant in a room.
pub struct Participant<T: UserContext> {
    pub user: T,
    pub connected_at: Instant,
    /// Channel to send messages to this participant.
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
        let (outbox_tx, outbox_rx) = mpsc::channel::<Outgoing<T::Id>>(256);

        let room = Arc::new(Room {
            id,
            creator_id: creator.id(),
            created_at: Instant::now(),
            metadata,
            event_tx,
            participants: RwLock::new(Vec::new()),
            participant_count: AtomicU32::new(0),
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
            player_count: room.participant_count.load(Ordering::Relaxed),
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
            let participant_count = room.participant_count.load(Ordering::Relaxed);
            let last_activity = *room.last_activity.read().await;
            let idle_time = now.duration_since(last_activity);

            // Max lifetime exceeded
            if age > self.config.max_lifetime {
                to_remove.push(*entry.key());
                continue;
            }

            // No one ever connected
            if participant_count == 0 && age > self.config.first_connect_timeout {
                to_remove.push(*entry.key());
                continue;
            }

            // Was active but now empty
            if participant_count == 0 && idle_time > self.config.empty_timeout {
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
    /// Add a participant to the room with their message channel.
    pub async fn add_participant(&self, user: T, msg_tx: mpsc::Sender<Vec<u8>>) {
        // Notify actor
        let _ = self.event_tx.send(RoomEvent::UserJoined(user.clone())).await;

        let mut participants = self.participants.write().await;
        participants.push(Participant {
            user,
            connected_at: Instant::now(),
            msg_tx,
        });
        self.participant_count.fetch_add(1, Ordering::Relaxed);
        *self.last_activity.write().await = Instant::now();
    }

    /// Remove a participant from the room.
    pub async fn remove_participant(&self, user_id: &T::Id) {
        let mut participants = self.participants.write().await;
        
        // Find and remove the user, keeping their info for the event
        let removed_user = participants
            .iter()
            .find(|p| &p.user.id() == user_id)
            .map(|p| p.user.clone());

        let old_len = participants.len();
        participants.retain(|p| &p.user.id() != user_id);
        let removed_count = old_len - participants.len();
        
        if removed_count > 0 {
            self.participant_count.fetch_sub(removed_count as u32, Ordering::Relaxed);
        }
        
        drop(participants);
        
        // Notify actor
        if let Some(user) = removed_user {
            let _ = self.event_tx.send(RoomEvent::UserLeft(user)).await;
        }
        
        *self.last_activity.write().await = Instant::now();
    }

    /// Send a message event to the actor.
    pub async fn send_message(&self, sender: T, payload: Vec<u8>) {
        let _ = self.event_tx.send(RoomEvent::Message { sender, payload }).await;
        *self.last_activity.write().await = Instant::now();
    }

    /// Get current participants (user data only).
    pub async fn get_participants(&self) -> Vec<T> {
        let participants = self.participants.read().await;
        participants.iter().map(|p| p.user.clone()).collect()
    }

    /// Take the outbox receiver (only call once, when setting up the broadcast task).
    pub async fn take_outbox_rx(&self) -> Option<mpsc::Receiver<Outgoing<T::Id>>> {
        self.outbox_rx.write().await.take()
    }

    /// Broadcast a message according to the routing decision.
    pub async fn broadcast(&self, payload: &[u8], route: Route<T::Id>) {
        let participants = self.participants.read().await;

        match route {
            Route::Broadcast => {
                // Send to all participants
                for p in participants.iter() {
                    let _ = p.msg_tx.send(payload.to_vec()).await;
                }
            }
            Route::Only(ref ids) => {
                // Send only to specific users
                for p in participants.iter() {
                    if ids.contains(&p.user.id()) {
                        let _ = p.msg_tx.send(payload.to_vec()).await;
                    }
                }
            }
            Route::AllExcept(ref ids) => {
                // Send to all except specific users
                for p in participants.iter() {
                    if !ids.contains(&p.user.id()) {
                        let _ = p.msg_tx.send(payload.to_vec()).await;
                    }
                }
            }
            Route::None => {
                // Don't send
            }
        }
    }
}
