//! Connection abstraction and channel handling for client actor.
//!
//! Provides a unified `Connection` type that works on both native and WASM.

use super::{make_shared, InternalEvent, SharedPtr};
use crate::utils::{ChannelMessage, frame_message, MessageBuffer};
use crate::spawning::Spawner;
use futures::FutureExt;
use multiplayer_kit_client::{ChannelIO, RoomConnection};
use tokio::sync::mpsc;

/// Connection wrapper that works on both native and WASM.
///
/// Uses `Arc<RoomConnection>` on native, `Rc<RoomConnection>` on WASM.
pub struct Connection {
    pub(super) inner: SharedPtr<RoomConnection>,
}

impl From<RoomConnection> for Connection {
    fn from(conn: RoomConnection) -> Self {
        Self {
            inner: make_shared(conn),
        }
    }
}

impl From<SharedPtr<RoomConnection>> for Connection {
    fn from(inner: SharedPtr<RoomConnection>) -> Self {
        Self { inner }
    }
}

/// Trait for types that can be converted to a Connection.
///
/// Implemented for:
/// - `RoomConnection`
/// - `SharedPtr<RoomConnection>` (Arc on native, Rc on WASM)
pub trait ClientConnection: Into<Connection> {}

impl ClientConnection for RoomConnection {}
impl ClientConnection for SharedPtr<RoomConnection> {}

/// Run a single channel's read/write loop.
pub async fn run_channel<M: ChannelMessage, C: ChannelIO + 'static, S: Spawner>(
    channel: C,
    channel_type: M::Channel,
    channel_id_byte: u8,
    event_tx: mpsc::UnboundedSender<InternalEvent<M>>,
    mut write_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    spawner: S,
) {
    tracing::info!("[Channel {}] run_channel started", channel_id_byte);
    let id_msg = frame_message(&[channel_id_byte]);
    if channel.write(&id_msg).await.is_err() {
        let _ = event_tx.send(InternalEvent::Disconnected);
        return;
    }

    let channel = std::rc::Rc::new(channel);
    let mut buffer = MessageBuffer::new();

    let (read_tx, mut read_rx) = mpsc::unbounded_channel::<Result<Vec<u8>, ()>>();

    {
        let channel = channel.clone();
        let channel_id = channel_id_byte;
        spawner.spawn_local(async move {
            tracing::info!("[Channel {}] Read task started", channel_id);
            loop {
                let mut buf = vec![0u8; 64 * 1024];
                match channel.read(&mut buf).await {
                    Ok(n) if n > 0 => {
                        tracing::trace!("[Channel {}] Read {} bytes", channel_id, n);
                        buf.truncate(n);
                        if read_tx.send(Ok(buf)).is_err() {
                            tracing::info!("[Channel {}] Read task: receiver dropped", channel_id);
                            break;
                        }
                    }
                    Ok(n) => {
                        tracing::info!("[Channel {}] Read returned {} (EOF?)", channel_id, n);
                        let _ = read_tx.send(Err(()));
                        break;
                    }
                    Err(e) => {
                        tracing::info!("[Channel {}] Read error: {:?}", channel_id, e);
                        let _ = read_tx.send(Err(()));
                        break;
                    }
                }
            }
            tracing::info!("[Channel {}] Read task ended", channel_id);
        });
    }

    loop {
        let read_fut = read_rx.recv().fuse();
        let write_fut = write_rx.recv().fuse();
        futures::pin_mut!(read_fut, write_fut);

        match futures::future::select(read_fut, write_fut).await {
            futures::future::Either::Left((read_result, _)) => {
                match read_result {
                    Some(Ok(data)) => {
                        for result in buffer.push(&data) {
                            match result {
                                Ok(msg) => match M::decode(channel_type, &msg) {
                                    Ok(message) => {
                                        tracing::trace!("[Channel {}] Decoded message, forwarding to actor", channel_id_byte);
                                        let _ = event_tx.send(InternalEvent::Message(message));
                                    }
                                    Err(e) => {
                                        tracing::warn!("[Channel {}] Failed to decode message: {:?}", channel_id_byte, e);
                                    }
                                },
                                Err(_) => {
                                    let _ = event_tx.send(InternalEvent::Disconnected);
                                    return;
                                }
                            }
                        }
                    }
                    Some(Err(())) | None => {
                        let _ = event_tx.send(InternalEvent::Disconnected);
                        return;
                    }
                }
            }
            futures::future::Either::Right((write_data, _)) => {
                match write_data {
                    Some(data) => {
                        if channel.write(&data).await.is_err() {
                            let _ = event_tx.send(InternalEvent::Disconnected);
                            return;
                        }
                    }
                    None => return,
                }
            }
        }
    }
}
