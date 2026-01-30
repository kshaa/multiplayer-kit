//! Room management and lifecycle.

use crate::lobby::Lobby;
use dashmap::DashMap;
use multiplayer_kit_protocol::{RoomId, RoomInfo, Route, UserContext};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};

/// Manages all active rooms.
pub struct RoomManager<T: UserContext> {
    rooms: DashMap<RoomId, Arc<Room<T>>>,
    next_room_id: AtomicU64,
    config: RoomConfig,
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
    participants: RwLock<Vec<Participant<T>>>,
    participant_count: AtomicU32,
    last_activity: RwLock<Instant>,
}

/// A participant in a room.
pub struct Participant<T: UserContext> {
    pub user: T,
    pub connected_at: Instant,
    /// Channel to send messages to this participant.
    pub msg_tx: mpsc::Sender<Vec<u8>>,
}

impl<T: UserContext> RoomManager<T> {
    pub fn new(config: RoomConfig) -> Self {
        Self {
            rooms: DashMap::new(),
            next_room_id: AtomicU64::new(1),
            config,
        }
    }

    /// Create a new room.
    pub fn create_room(&self, creator: &T, metadata: Option<serde_json::Value>) -> RoomId {
        let id = RoomId(self.next_room_id.fetch_add(1, Ordering::SeqCst));
        let room = Arc::new(Room {
            id,
            creator_id: creator.id(),
            created_at: Instant::now(),
            metadata,
            participants: RwLock::new(Vec::new()),
            participant_count: AtomicU32::new(0),
            last_activity: RwLock::new(Instant::now()),
        });
        self.rooms.insert(id, room);
        id
    }

    /// Delete a room (creator only).
    pub fn delete_room(&self, room_id: RoomId, requester_id: &T::Id) -> Result<(), &'static str> {
        if let Some(room) = self.rooms.get(&room_id) {
            if &room.creator_id == requester_id {
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
            let participants = room.participants.read().await;
            let last_activity = *room.last_activity.read().await;
            let idle_time = now.duration_since(last_activity);

            // Max lifetime exceeded
            if age > self.config.max_lifetime {
                to_remove.push(*entry.key());
                continue;
            }

            // No one ever connected
            if participants.is_empty() && age > self.config.first_connect_timeout {
                to_remove.push(*entry.key());
                continue;
            }

            // Was active but now empty
            if participants.is_empty() && idle_time > self.config.empty_timeout {
                to_remove.push(*entry.key());
            }
        }

        for room_id in to_remove {
            self.rooms.remove(&room_id);
            lobby.notify_room_deleted(room_id);
            tracing::info!(?room_id, "Room expired and removed");
        }
    }
}

impl<T: UserContext> Room<T> {
    /// Add a participant to the room with their message channel.
    pub async fn add_participant(&self, user: T, msg_tx: mpsc::Sender<Vec<u8>>) {
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
        let old_len = participants.len();
        participants.retain(|p| &p.user.id() != user_id);
        let removed = old_len - participants.len();
        if removed > 0 {
            self.participant_count.fetch_sub(removed as u32, Ordering::Relaxed);
        }
        *self.last_activity.write().await = Instant::now();
    }

    /// Get current participants (user data only).
    pub async fn get_participants(&self) -> Vec<T> {
        let participants = self.participants.read().await;
        participants.iter().map(|p| p.user.clone()).collect()
    }

    /// Broadcast a message according to the routing decision.
    pub async fn broadcast(&self, payload: &[u8], sender_id: &T::Id, route: Route<T::Id>) {
        let participants = self.participants.read().await;

        match route {
            Route::Broadcast => {
                // Send to all except sender
                for p in participants.iter() {
                    if &p.user.id() != sender_id {
                        let _ = p.msg_tx.send(payload.to_vec()).await;
                    }
                }
            }
            Route::BroadcastIncludingSelf => {
                // Send to all including sender
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
                // Send to all except specific users (and sender)
                for p in participants.iter() {
                    if !ids.contains(&p.user.id()) && &p.user.id() != sender_id {
                        let _ = p.msg_tx.send(payload.to_vec()).await;
                    }
                }
            }
            Route::None => {
                // Don't forward
            }
        }

        // Update activity timestamp
        drop(participants);
        *self.last_activity.write().await = Instant::now();
    }
}
