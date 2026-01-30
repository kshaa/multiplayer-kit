//! Lobby QUIC endpoint for streaming room updates.

use multiplayer_kit_protocol::{LobbyEvent, RoomId, RoomInfo};
use tokio::sync::broadcast;

/// Lobby state and event broadcasting.
pub struct Lobby {
    /// Broadcast channel for lobby events.
    event_tx: broadcast::Sender<LobbyEvent>,
}

impl Lobby {
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(256);
        Self { event_tx }
    }

    /// Subscribe to lobby events.
    pub fn subscribe(&self) -> broadcast::Receiver<LobbyEvent> {
        self.event_tx.subscribe()
    }

    /// Broadcast that a room was created.
    pub fn notify_room_created(&self, room: RoomInfo) {
        let _ = self.event_tx.send(LobbyEvent::RoomCreated(room));
    }

    /// Broadcast that a room was updated.
    pub fn notify_room_updated(&self, room: RoomInfo) {
        let _ = self.event_tx.send(LobbyEvent::RoomUpdated(room));
    }

    /// Broadcast that a room was deleted.
    pub fn notify_room_deleted(&self, room_id: RoomId) {
        let _ = self.event_tx.send(LobbyEvent::RoomDeleted(room_id));
    }
}

impl Default for Lobby {
    fn default() -> Self {
        Self::new()
    }
}
