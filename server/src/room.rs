//! Room management and lifecycle.

use dashmap::DashMap;
use multiplayer_kit_protocol::{RoomId, RoomInfo, UserContext};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

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
    last_activity: RwLock<Instant>,
}

/// A participant in a room.
pub struct Participant<T: UserContext> {
    pub user: T,
    pub connected_at: Instant,
    // Connection handle would go here
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
        self.rooms.get(&room_id).map(|room| {
            let participants = room.participants.blocking_read();
            RoomInfo {
                id: room.id,
                player_count: participants.len() as u32,
                max_players: None,
                created_at: room.created_at.elapsed().as_secs(),
                metadata: room.metadata.clone(),
            }
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
    pub async fn run_lifecycle_checks(&self) {
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
            tracing::info!(?room_id, "Room expired and removed");
        }
    }
}

impl<T: UserContext> Room<T> {
    /// Add a participant to the room.
    pub async fn add_participant(&self, user: T) {
        let mut participants = self.participants.write().await;
        participants.push(Participant {
            user,
            connected_at: Instant::now(),
        });
        *self.last_activity.write().await = Instant::now();
    }

    /// Remove a participant from the room.
    pub async fn remove_participant(&self, user_id: &T::Id) {
        let mut participants = self.participants.write().await;
        participants.retain(|p| &p.user.id() != user_id);
        *self.last_activity.write().await = Instant::now();
    }

    /// Get current participants.
    pub async fn get_participants(&self) -> Vec<T> {
        let participants = self.participants.read().await;
        participants.iter().map(|p| p.user.clone()).collect()
    }
}
