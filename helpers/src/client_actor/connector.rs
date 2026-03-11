//! Bridge connector for async network I/O.

use super::bridge::{BridgeEvent, NetworkBridge};
use super::channel::Connection;
use crate::spawning::Spawner;
use crate::utils::ChannelMessage;
use tokio::sync::mpsc;

/// Connects an ActorRuntime to the network. Must be spawned as an async task.
pub struct BridgeConnector<M: ChannelMessage> {
    pub(super) bridge_tx: mpsc::UnboundedSender<BridgeEvent<M>>,
    pub(super) server_rx: mpsc::UnboundedReceiver<M>,
}

impl<M: ChannelMessage> BridgeConnector<M> {
    /// Run the bridge, connecting to the server and forwarding messages.
    ///
    /// This must be spawned as an async task. It will run until disconnected.
    pub async fn run<Conn, S>(mut self, conn: Conn, spawner: S)
    where
        Conn: Into<Connection>,
        S: Spawner,
    {
        let mut bridge = NetworkBridge::<M>::spawn(conn, spawner).await;

        if self.bridge_tx.send(BridgeEvent::Connected).is_err() {
            return;
        }

        loop {
            tokio::select! {
                event = bridge.incoming.recv() => {
                    match event {
                        Some(BridgeEvent::Message(m)) => {
                            if self.bridge_tx.send(BridgeEvent::Message(m)).is_err() {
                                break;
                            }
                        }
                        Some(BridgeEvent::Connected) => {}
                        Some(BridgeEvent::Disconnected) | None => {
                            let _ = self.bridge_tx.send(BridgeEvent::Disconnected);
                            break;
                        }
                    }
                }
                msg = self.server_rx.recv() => {
                    match msg {
                        Some(m) => { let _ = bridge.outgoing.send(m); }
                        None => break,
                    }
                }
            }
        }
    }
}
