//! Connection abstraction and channel handling for client actor.
//!
//! Provides a unified `Connection` type that works on both native and WASM.

use super::{
    make_shared, ClientActorHandle, ClientActorSender, ClientContext,
    ClientEvent, InternalEvent, LocalSender, SharedPtr,
};
use crate::utils::{ChannelMessage, frame_message, GameClientContext, MaybeSend, MaybeSync, MessageBuffer};
use crate::spawning::Spawner;
use futures::FutureExt;
use multiplayer_kit_client::{ChannelIO, RoomConnection};
use std::collections::HashMap;
use std::marker::PhantomData;
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

/// Open a channel and spawn the read/write loop.
async fn open_and_run_channel<M: ChannelMessage, S: Spawner>(
    conn: &Connection,
    spawner: S,
    channel_type: M::Channel,
    event_tx: mpsc::UnboundedSender<InternalEvent<M>>,
    write_rx: mpsc::UnboundedReceiver<Vec<u8>>,
) -> Result<(), String> {
    let channel = conn.inner.open_channel().await.map_err(|e| e.to_string())?;
    let channel_id_byte = M::channel_to_id(channel_type);
    spawner.spawn_local(run_channel::<M, _, S>(
        channel,
        channel_type,
        channel_id_byte,
        event_tx,
        write_rx,
        spawner.clone(),
    ));
    Ok(())
}

/// Run a single channel's read/write loop.
pub(super) async fn run_channel<M: ChannelMessage, C: ChannelIO + 'static, S: Spawner>(
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

/// Implementation of run_client_actor.
pub(super) fn run_client_actor_impl<M, Ctx, F, S>(
    conn: Connection,
    game_context: Ctx,
    actor_fn: F,
    spawner: S,
) -> ClientActorHandle<M>
where
    M: ChannelMessage,
    Ctx: GameClientContext,
    F: Fn(&ClientContext<M, Ctx>, ClientEvent<M>, &S) + MaybeSend + MaybeSync + Clone + 'static,
    S: Spawner,
{
    let (user_msg_tx, user_msg_rx) = mpsc::unbounded_channel::<M>();
    let (local_tx, local_rx) = mpsc::unbounded_channel::<M>();

    let sender = ClientActorSender::new(user_msg_tx);
    let local_sender = LocalSender::new(local_tx);
    let handle = ClientActorHandle { sender, local_sender };

    let spawner_clone = spawner.clone();
    spawner.spawn_with_local_context(async move {
        run_actor_loop::<M, Ctx, F, S>(conn, game_context, actor_fn, spawner_clone, user_msg_rx, local_rx)
            .await;
    });

    handle
}

/// The async actor event loop.
async fn run_actor_loop<M, Ctx, F, S>(
    conn: Connection,
    game_context: Ctx,
    actor_fn: F,
    spawner: S,
    mut user_msg_rx: mpsc::UnboundedReceiver<M>,
    mut local_rx: mpsc::UnboundedReceiver<M>,
) where
    M: ChannelMessage,
    Ctx: GameClientContext,
    F: Fn(&ClientContext<M, Ctx>, ClientEvent<M>, &S) + MaybeSend + MaybeSync + Clone + 'static,
    S: Spawner,
{
    let game_context = make_shared(game_context);
    let channels_to_open = M::all_channels();
    let mut channel_senders: HashMap<M::Channel, mpsc::UnboundedSender<Vec<u8>>> = HashMap::new();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<InternalEvent<M>>();
    let (self_tx, mut self_rx) = mpsc::unbounded_channel::<M>();

    for &channel_type in channels_to_open {
        let channel_id = M::channel_to_id(channel_type);
        tracing::info!("[Client] Opening channel {} (id={})", std::any::type_name::<M::Channel>(), channel_id);
        
        let (write_tx, write_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        channel_senders.insert(channel_type, write_tx);

        let result = open_and_run_channel::<M, _>(
            &conn,
            spawner.clone(),
            channel_type,
            event_tx.clone(),
            write_rx,
        )
        .await;
        
        tracing::info!("[Client] Channel {} opened, result={}", channel_id, result.is_ok());

        if let Err(e) = result {
            tracing::error!("Failed to open channel: {}", e);
            let ctx = ClientContext {
                channels: HashMap::new(),
                self_tx: self_tx.clone(),
                game_context: game_context.clone(),
                _phantom: PhantomData,
            };
            actor_fn(&ctx, ClientEvent::Disconnected, &spawner);
            return;
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        tracing::info!("[Client] All channels opened, yielding to let read tasks start");
        tokio::task::yield_now().await;
        tracing::info!("[Client] Yield complete, emitting Connected");
    }

    let ctx = ClientContext {
        channels: channel_senders,
        self_tx,
        game_context,
        _phantom: PhantomData,
    };

    actor_fn(&ctx, ClientEvent::Connected, &spawner);

    {
        let ctx = ctx.clone();
        spawner.spawn(async move {
            while let Some(message) = user_msg_rx.recv().await {
                if ctx.send(&message).is_err() {
                    break;
                }
            }
        });
    }

    loop {
        tokio::select! {
            event = event_rx.recv() => {
                match event {
                    Some(InternalEvent::Message(msg)) => {
                        actor_fn(&ctx, ClientEvent::Message(msg), &spawner);
                    }
                    Some(InternalEvent::Disconnected) => {
                        actor_fn(&ctx, ClientEvent::Disconnected, &spawner);
                        break;
                    }
                    None => break,
                }
            }
            self_msg = self_rx.recv() => {
                if let Some(message) = self_msg {
                    actor_fn(&ctx, ClientEvent::Internal(message), &spawner);
                }
            }
            local_msg = local_rx.recv() => {
                if let Some(message) = local_msg {
                    actor_fn(&ctx, ClientEvent::Local(message), &spawner);
                }
            }
        }
    }
}
