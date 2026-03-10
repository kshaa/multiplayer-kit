//! Low-level network bridge for client actors.
//!
//! Handles only socket I/O - caller is responsible for actor logic.

use super::channel::{run_channel, Connection};
use super::InternalEvent;
use crate::spawning::Spawner;
use crate::utils::{frame_message, ChannelMessage};
use std::collections::HashMap;
use tokio::sync::mpsc;

/// Events from the network bridge to the caller.
#[derive(Debug, Clone)]
pub enum BridgeEvent<M> {
    /// All channels connected successfully.
    Connected,
    /// Message received from server.
    Message(M),
    /// Disconnected (any channel failed or closed).
    Disconnected,
}

/// Low-level network bridge.
///
/// Handles socket I/O in background tasks, provides channels for:
/// - Receiving server messages
/// - Sending messages to server
///
/// The caller is responsible for:
/// - Creating and managing actor state
/// - Driving the actor (calling handle methods)
/// - Processing incoming messages
pub struct NetworkBridge<M: ChannelMessage> {
    /// Receive events from the bridge (Connected, Message, Disconnected).
    pub incoming: mpsc::UnboundedReceiver<BridgeEvent<M>>,
    /// Send messages to the server.
    pub outgoing: mpsc::UnboundedSender<M>,
}

impl<M: ChannelMessage> NetworkBridge<M> {
    /// Spawn socket I/O tasks and return the bridge.
    ///
    /// Opens all channels defined by `M::all_channels()`, spawns read/write
    /// tasks for each, and returns immediately with channels for communication.
    ///
    /// On success, the bridge will emit `BridgeEvent::Connected`.
    /// On failure, the bridge will emit `BridgeEvent::Disconnected`.
    ///
    /// **Must be called from within a local context** (i.e., from a future
    /// spawned by `spawn_with_local_context`).
    pub async fn spawn<Conn, S>(conn: Conn, spawner: S) -> Self
    where
        Conn: Into<Connection>,
        S: Spawner,
    {
        let conn = conn.into();
        let (bridge_tx, bridge_rx) = mpsc::unbounded_channel::<BridgeEvent<M>>();
        let (outgoing_tx, outgoing_rx) = mpsc::unbounded_channel::<M>();

        let bridge_tx_clone = bridge_tx.clone();
        let spawner_clone = spawner.clone();
        spawner.spawn_local(async move {
            run_bridge_loop::<M, S>(conn, bridge_tx_clone, outgoing_rx, spawner_clone).await;
        });

        NetworkBridge {
            incoming: bridge_rx,
            outgoing: outgoing_tx,
        }
    }
}

async fn run_bridge_loop<M: ChannelMessage, S: Spawner>(
    conn: Connection,
    bridge_tx: mpsc::UnboundedSender<BridgeEvent<M>>,
    mut outgoing_rx: mpsc::UnboundedReceiver<M>,
    spawner: S,
) {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<InternalEvent<M>>();
    let mut channel_senders: HashMap<M::Channel, mpsc::UnboundedSender<Vec<u8>>> = HashMap::new();

    // Open all channels
    for &channel_type in M::all_channels() {
        let channel_id = M::channel_to_id(channel_type);
        tracing::info!("[NetworkBridge] Opening channel {}", channel_id);

        let (write_tx, write_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        channel_senders.insert(channel_type, write_tx);

        let channel = match conn.inner.open_channel().await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("[NetworkBridge] Failed to open channel {}: {}", channel_id, e);
                let _ = bridge_tx.send(BridgeEvent::Disconnected);
                return;
            }
        };

        spawner.spawn_local(run_channel::<M, _, S>(
            channel,
            channel_type,
            channel_id,
            event_tx.clone(),
            write_rx,
            spawner.clone(),
        ));
    }

    // Yield to let read tasks start
    spawner.yield_now().await;

    // Send Connected event
    if bridge_tx.send(BridgeEvent::Connected).is_err() {
        return;
    }

    // Forward outgoing messages to channels
    let channel_senders_clone = channel_senders.clone();
    spawner.spawn_local(async move {
        while let Some(message) = outgoing_rx.recv().await {
            if let Some(channel_type) = message.channel() {
                if let Ok(data) = message.encode() {
                    let framed = frame_message(&data);
                    if let Some(tx) = channel_senders_clone.get(&channel_type) {
                        let _ = tx.send(framed);
                    }
                }
            }
        }
    });

    // Forward network events to bridge events
    while let Some(event) = event_rx.recv().await {
        let bridge_event = match event {
            InternalEvent::Message(m) => BridgeEvent::Message(m),
            InternalEvent::Disconnected => BridgeEvent::Disconnected,
        };
        let is_disconnect = matches!(bridge_event, BridgeEvent::Disconnected);
        if bridge_tx.send(bridge_event).is_err() || is_disconnect {
            break;
        }
    }
}
