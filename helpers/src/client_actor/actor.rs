//! Client actor wrapper using the Actor trait.

use super::sink::{ClientOutput, ClientSink};
use super::types::{ClientMessage, ClientSource};
use super::{ClientActorHandle, ClientActorSender, InternalEvent};
use crate::actor::{Actor, ActorProtocol, AddressedMessage};
use crate::spawning::Spawner;
use crate::utils::{frame_message, ChannelMessage, GameClientContext, MaybeSend};
use futures::FutureExt;
use std::collections::HashMap;
use tokio::sync::mpsc;

/// Run a client actor using the Actor trait.
///
/// The actor receives `AddressedMessage<ClientSource, ClientMessage<M>>` where:
/// - `ClientSource::Server` for messages from the server
/// - `ClientSource::Internal` for self-scheduled messages
/// - `ClientMessage::Connected` / `Disconnected` / `Message(M)`
///
/// Outputs via the sink are routed:
/// - `Sink<ServerTarget<M>>` -> send to server
/// - `Sink<SelfTarget<M>>` -> schedule message back to self
///
/// The `Ctx` (game context) provides:
/// - Config via `From<Ctx>` (for startup)
/// - Extras via `Ctx::Extras` and `get_extras()` (for handle/shutdown)
pub fn with_client_actor<A, M, Ctx, S>(
    conn: impl Into<super::channel::Connection>,
    game_context: Ctx,
    spawner: S,
) -> ClientActorHandle<M>
where
    M: ChannelMessage,
    Ctx: GameClientContext,
    S: Spawner,
    A: Actor<ClientSink<M>>
        + ActorProtocol<ActorId = ClientSource, Message = ClientMessage<M>>
        + 'static,
    A::Config: From<Ctx> + MaybeSend + 'static,
    A::State: MaybeSend + 'static,
    A::Extras: From<Ctx::Extras> + MaybeSend + 'static,
{
    let conn = conn.into();
    let (user_msg_tx, user_msg_rx) = mpsc::unbounded_channel::<M>();

    let sender = ClientActorSender::new(user_msg_tx);
    let handle = ClientActorHandle { sender };

    let extras = game_context.get_extras();
    let spawner_clone = spawner.clone();
    spawner.spawn_with_local_context(async move {
        run_actor_loop::<A, M, Ctx, S>(conn, game_context, extras, spawner_clone, user_msg_rx).await;
    });

    handle
}

async fn run_actor_loop<A, M, Ctx, S>(
    conn: super::channel::Connection,
    game_context: Ctx,
    extras: Ctx::Extras,
    spawner: S,
    mut user_msg_rx: mpsc::UnboundedReceiver<M>,
) where
    M: ChannelMessage,
    Ctx: GameClientContext,
    S: Spawner,
    A: Actor<ClientSink<M>>
        + ActorProtocol<ActorId = ClientSource, Message = ClientMessage<M>>
        + 'static,
    A::Config: From<Ctx> + MaybeSend + 'static,
    A::State: MaybeSend + 'static,
    A::Extras: From<Ctx::Extras> + MaybeSend + 'static,
{
    let channels_to_open = M::all_channels();
    let mut channel_senders: HashMap<M::Channel, mpsc::UnboundedSender<Vec<u8>>> = HashMap::new();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<InternalEvent<M>>();
    let (self_tx, mut self_rx) = mpsc::unbounded_channel::<M>();

    // Open all channels
    for &channel_type in channels_to_open {
        let channel_id = M::channel_to_id(channel_type);
        tracing::info!(
            "[Client] Opening channel {} (id={})",
            std::any::type_name::<M::Channel>(),
            channel_id
        );

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

        tracing::info!(
            "[Client] Channel {} opened, result={}",
            channel_id,
            result.is_ok()
        );

        if let Err(e) = result {
            tracing::error!("Failed to open channel: {}", e);
            // Can't even start - just return
            return;
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        tracing::info!("[Client] All channels opened, yielding to let read tasks start");
        tokio::task::yield_now().await;
        tracing::info!("[Client] Yield complete");
    }

    // Initialize actor
    let mut state = A::startup(game_context.into());
    let mut extras: A::Extras = extras.into();
    let mut sink = ClientSink::<M>::new();

    // Send Connected event
    let msg = AddressedMessage {
        from: ClientSource::Server,
        content: ClientMessage::Connected,
    };
    A::handle(&mut state, &mut extras, msg, &mut sink);
    route_outputs::<M>(&mut sink, &channel_senders, &self_tx);

    // Spawn task to forward user messages to server
    {
        let channel_senders_clone = channel_senders.clone();
        spawner.spawn(async move {
            while let Some(message) = user_msg_rx.recv().await {
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
    }

    // Main event loop
    loop {
        let event_fut = event_rx.recv().fuse();
        let self_fut = self_rx.recv().fuse();
        futures::pin_mut!(event_fut, self_fut);

        match futures::future::select(event_fut, self_fut).await {
            futures::future::Either::Left((event, _)) => match event {
                Some(InternalEvent::Message(m)) => {
                    let msg = AddressedMessage {
                        from: ClientSource::Server,
                        content: ClientMessage::Message(m),
                    };
                    A::handle(&mut state, &mut extras, msg, &mut sink);
                    route_outputs::<M>(&mut sink, &channel_senders, &self_tx);
                }
                Some(InternalEvent::Disconnected) => {
                    let msg = AddressedMessage {
                        from: ClientSource::Server,
                        content: ClientMessage::Disconnected,
                    };
                    A::handle(&mut state, &mut extras, msg, &mut sink);
                    // Don't route outputs after disconnect
                    A::shutdown(&mut state, &mut extras);
                    break;
                }
                None => {
                    A::shutdown(&mut state, &mut extras);
                    break;
                }
            },
            futures::future::Either::Right((self_msg, _)) => {
                if let Some(message) = self_msg {
                    let msg = AddressedMessage {
                        from: ClientSource::Internal,
                        content: ClientMessage::Message(message),
                    };
                    A::handle(&mut state, &mut extras, msg, &mut sink);
                    route_outputs::<M>(&mut sink, &channel_senders, &self_tx);
                }
            }
        }
    }
}

fn route_outputs<M: ChannelMessage>(
    sink: &mut ClientSink<M>,
    channel_senders: &HashMap<M::Channel, mpsc::UnboundedSender<Vec<u8>>>,
    self_tx: &mpsc::UnboundedSender<M>,
) {
    for output in sink.drain() {
        match output {
            ClientOutput::ToServer { message } => {
                if let Some(channel_type) = message.channel() {
                    if let Ok(data) = message.encode() {
                        let framed = frame_message(&data);
                        if let Some(tx) = channel_senders.get(&channel_type) {
                            let _ = tx.send(framed);
                        }
                    }
                }
            }
            ClientOutput::Internal { message } => {
                let _ = self_tx.send(message);
            }
        }
    }
}

async fn open_and_run_channel<M: ChannelMessage, S: Spawner>(
    conn: &super::channel::Connection,
    spawner: S,
    channel_type: M::Channel,
    event_tx: mpsc::UnboundedSender<InternalEvent<M>>,
    write_rx: mpsc::UnboundedReceiver<Vec<u8>>,
) -> Result<(), String> {
    let channel = conn.inner.open_channel().await.map_err(|e| e.to_string())?;
    let channel_id_byte = M::channel_to_id(channel_type);
    spawner.spawn_local(super::channel::run_channel::<M, _, S>(
        channel,
        channel_type,
        channel_id_byte,
        event_tx,
        write_rx,
        spawner.clone(),
    ));
    Ok(())
}
