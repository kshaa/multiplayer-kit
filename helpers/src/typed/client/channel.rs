//! Connection abstraction and channel handling for typed client actor.
//!
//! Provides a unified `Connection` type that works on both native and WASM.

use super::{
    make_shared, ActorHandle, InternalEvent, SharedPtr, TypedActorSender, TypedClientContext,
    TypedClientEvent,
};
use crate::framing::{frame_message, MessageBuffer};
use crate::typed::platform::{GameClientContext, MaybeSend, MaybeSync};
use crate::typed::spawner::Spawner;
use crate::typed::TypedProtocol;
use futures::FutureExt;
use multiplayer_kit_client::{ChannelIO, RoomConnection};
use std::collections::HashMap;
use std::marker::PhantomData;
use tokio::sync::mpsc;

// ============================================================================
// Unified Connection type
// ============================================================================

/// Connection wrapper that works on both native and WASM.
///
/// Uses `Arc<RoomConnection>` on native, `Rc<RoomConnection>` on WASM.
pub struct Connection {
    inner: SharedPtr<RoomConnection>,
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

// ============================================================================
// Trait for connection types (for external use)
// ============================================================================

/// Trait for types that can be converted to a Connection.
///
/// Implemented for:
/// - `RoomConnection`
/// - `SharedPtr<RoomConnection>` (Arc on native, Rc on WASM)
pub trait ClientConnection: Into<Connection> {}

impl ClientConnection for RoomConnection {}
impl ClientConnection for SharedPtr<RoomConnection> {}

// ============================================================================
// Channel operations using RoomConnectionLike trait
// ============================================================================

/// Open a channel and spawn the read/write loop.
///
/// Uses spawn_local to run the channel in a local context, allowing Rc sharing
/// and preventing read cancellation issues.
async fn open_and_run_channel<P: TypedProtocol, S: Spawner>(
    conn: &Connection,
    spawner: S,
    channel_type: P::Channel,
    event_tx: mpsc::UnboundedSender<InternalEvent<P>>,
    write_rx: mpsc::UnboundedReceiver<Vec<u8>>,
) -> Result<(), String> {
    // Use RoomConnectionLike trait - works because SharedPtr<T> derefs to T
    let channel = conn.inner.open_channel().await.map_err(|e| e.to_string())?;
    let channel_id_byte = P::channel_to_id(channel_type);
    // Use spawn_local so channel can use Rc and nested spawn_local
    spawner.spawn_local(run_channel::<P, _, S>(
        channel,
        channel_type,
        channel_id_byte,
        event_tx,
        write_rx,
        spawner.clone(),
    ));
    Ok(())
}

// ============================================================================
// Channel runner
// ============================================================================

/// Run a single channel's read/write loop.
///
/// Spawns a dedicated read task to avoid cancelling reads (which loses data on WASM).
/// Uses Rc for channel sharing since this runs in a single-threaded local context.
async fn run_channel<P: TypedProtocol, C: ChannelIO + 'static, S: Spawner>(
    channel: C,
    channel_type: P::Channel,
    channel_id_byte: u8,
    event_tx: mpsc::UnboundedSender<InternalEvent<P>>,
    mut write_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    spawner: S,
) {
    tracing::info!("[Channel {}] run_channel started", channel_id_byte);
    // Send channel ID to identify this channel to the server
    let id_msg = frame_message(&[channel_id_byte]);
    if channel.write(&id_msg).await.is_err() {
        let _ = event_tx.send(InternalEvent::Disconnected);
        return;
    }

    // Wrap channel in Rc for sharing (safe because we're in a local/single-threaded context)
    let channel = std::rc::Rc::new(channel);
    let mut buffer = MessageBuffer::new();

    // Channel for read results - reads are NEVER cancelled
    let (read_tx, mut read_rx) = mpsc::unbounded_channel::<Result<Vec<u8>, ()>>();

    // Spawn dedicated read task - runs in the same local context
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

    // Main loop - select between read results and write requests
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
                                Ok(msg) => match P::decode(channel_type, &msg) {
                                    Ok(event) => {
                                        tracing::trace!("[Channel {}] Decoded message, forwarding to actor", channel_id_byte);
                                        let _ = event_tx.send(InternalEvent::Message(event));
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

// ============================================================================
// Actor implementation
// ============================================================================

/// Implementation of run_typed_client_actor.
///
/// This is a SYNC function that:
/// 1. Creates channels for communication
/// 2. Spawns the async event loop using the provided spawner
/// 3. Returns ActorHandle immediately
pub(super) fn run_typed_client_actor_impl<P, Ctx, F, S>(
    conn: Connection,
    game_context: Ctx,
    actor_fn: F,
    spawner: S,
) -> ActorHandle<P>
where
    P: TypedProtocol,
    Ctx: GameClientContext,
    F: Fn(&TypedClientContext<P, Ctx>, TypedClientEvent<P>, &S) + MaybeSend + MaybeSync + Clone + 'static,
    S: Spawner,
{
    let (user_event_tx, user_event_rx) = mpsc::unbounded_channel::<P::Event>();

    let typed_sender = TypedActorSender::new(user_event_tx);
    let handle = ActorHandle {
        sender: typed_sender,
    };

    let spawner_clone = spawner.clone();
    // Use spawn_with_local_context so channels can use spawn_local internally
    spawner.spawn_with_local_context(async move {
        run_actor_loop::<P, Ctx, F, S>(conn, game_context, actor_fn, spawner_clone, user_event_rx)
            .await;
    });

    handle
}

/// The async actor event loop.
async fn run_actor_loop<P, Ctx, F, S>(
    conn: Connection,
    game_context: Ctx,
    actor_fn: F,
    spawner: S,
    mut user_event_rx: mpsc::UnboundedReceiver<P::Event>,
) where
    P: TypedProtocol,
    Ctx: GameClientContext,
    F: Fn(&TypedClientContext<P, Ctx>, TypedClientEvent<P>, &S) + MaybeSend + MaybeSync + Clone + 'static,
    S: Spawner,
{
    let game_context = make_shared(game_context);
    let channels_to_open = P::all_channels();
    let mut channel_senders: HashMap<P::Channel, mpsc::UnboundedSender<Vec<u8>>> = HashMap::new();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<InternalEvent<P>>();
    let (self_tx, mut self_rx) = mpsc::unbounded_channel::<P::Event>();

    // Open all channels
    for &channel_type in channels_to_open {
        let channel_id = P::channel_to_id(channel_type);
        tracing::info!("[Typed] Opening channel {} (id={})", std::any::type_name::<P::Channel>(), channel_id);
        
        let (write_tx, write_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        channel_senders.insert(channel_type, write_tx);

        let result = open_and_run_channel::<P, _>(
            &conn,
            spawner.clone(),
            channel_type,
            event_tx.clone(),
            write_rx,
        )
        .await;
        
        tracing::info!("[Typed] Channel {} opened, result={}", channel_id, result.is_ok());

        if let Err(e) = result {
            tracing::error!("Failed to open channel: {}", e);
            let ctx = TypedClientContext {
                channels: HashMap::new(),
                self_tx: self_tx.clone(),
                game_context: game_context.clone(),
                _phantom: PhantomData,
            };
            actor_fn(&ctx, TypedClientEvent::Disconnected, &spawner);
            return;
        }
    }

    // Yield to let spawned read tasks start before we emit Connected
    // This ensures they're ready to receive messages from the server
    #[cfg(not(target_arch = "wasm32"))]
    {
        tracing::info!("[Typed] All channels opened, yielding to let read tasks start");
        tokio::task::yield_now().await;
        tracing::info!("[Typed] Yield complete, emitting Connected");
    }

    let ctx = TypedClientContext {
        channels: channel_senders,
        self_tx,
        game_context,
        _phantom: PhantomData,
    };

    actor_fn(&ctx, TypedClientEvent::Connected, &spawner);

    // Spawn message forwarder
    {
        let ctx = ctx.clone();
        spawner.spawn(async move {
            while let Some(event) = user_event_rx.recv().await {
                if ctx.send(&event).is_err() {
                    break;
                }
            }
        });
    }

    // Core event loop
    loop {
        let event_fut = event_rx.recv().fuse();
        let self_fut = self_rx.recv().fuse();
        futures::pin_mut!(event_fut, self_fut);

        match futures::future::select(event_fut, self_fut).await {
            futures::future::Either::Left((event, _)) => match event {
                Some(InternalEvent::Message(msg)) => {
                    actor_fn(&ctx, TypedClientEvent::Message(msg), &spawner);
                }
                Some(InternalEvent::Disconnected) => {
                    actor_fn(&ctx, TypedClientEvent::Disconnected, &spawner);
                    break;
                }
                None => break,
            },
            futures::future::Either::Right((self_event, _)) => {
                if let Some(event) = self_event {
                    actor_fn(&ctx, TypedClientEvent::Internal(event), &spawner);
                }
            }
        }
    }
}
