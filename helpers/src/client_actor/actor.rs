//! Client actor wrapper using the Actor trait.

use super::bridge::{BridgeEvent, NetworkBridge};
use super::sink::{ClientOutput, ClientSink};
use super::types::{ClientMessage, ClientSource};
use super::{ClientActorHandle, ClientActorSender, LocalSender};
use crate::actor::{Actor, ActorProtocol, AddressedMessage};
use crate::spawning::Spawner;
use crate::utils::{ChannelMessage, GameClientContext, MaybeSend};
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
    let (local_tx, local_rx) = mpsc::unbounded_channel::<M>();

    let sender = ClientActorSender::new(user_msg_tx);
    let local_sender = LocalSender::new(local_tx);
    let handle = ClientActorHandle { sender, local_sender };

    let extras = game_context.get_extras();
    let spawner_clone = spawner.clone();
    spawner.spawn_with_local_context(async move {
        run_actor_loop::<A, M, Ctx, S>(conn, game_context, extras, spawner_clone, user_msg_rx, local_rx).await;
    });

    handle
}

async fn run_actor_loop<A, M, Ctx, S>(
    conn: super::channel::Connection,
    game_context: Ctx,
    extras: Ctx::Extras,
    spawner: S,
    mut user_msg_rx: mpsc::UnboundedReceiver<M>,
    mut local_rx: mpsc::UnboundedReceiver<M>,
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
    let mut bridge = NetworkBridge::<M>::spawn(conn, spawner.clone()).await;
    let (self_tx, mut self_rx) = mpsc::unbounded_channel::<M>();

    // Initialize actor
    let mut state = A::startup(game_context.into());
    let mut extras: A::Extras = extras.into();
    let mut sink = ClientSink::<M>::new();

    // Forward user messages to server via bridge
    {
        let outgoing = bridge.outgoing.clone();
        spawner.spawn_local(async move {
            while let Some(message) = user_msg_rx.recv().await {
                let _ = outgoing.send(message);
            }
        });
    }

    // Main event loop
    loop {
        tokio::select! {
            event = bridge.incoming.recv() => {
                match event {
                    Some(BridgeEvent::Connected) => {
                        let msg = AddressedMessage {
                            from: ClientSource::Server,
                            content: ClientMessage::Connected,
                        };
                        A::handle(&mut state, &mut extras, msg, &mut sink);
                        route_outputs(&mut sink, &bridge.outgoing, &self_tx);
                    }
                    Some(BridgeEvent::Message(m)) => {
                        let msg = AddressedMessage {
                            from: ClientSource::Server,
                            content: ClientMessage::Message(m),
                        };
                        A::handle(&mut state, &mut extras, msg, &mut sink);
                        route_outputs(&mut sink, &bridge.outgoing, &self_tx);
                    }
                    Some(BridgeEvent::Disconnected) | None => {
                        let msg = AddressedMessage {
                            from: ClientSource::Server,
                            content: ClientMessage::Disconnected,
                        };
                        A::handle(&mut state, &mut extras, msg, &mut sink);
                        A::shutdown(&mut state, &mut extras);
                        break;
                    }
                }
            }
            self_msg = self_rx.recv() => {
                if let Some(message) = self_msg {
                    let msg = AddressedMessage {
                        from: ClientSource::Internal,
                        content: ClientMessage::Message(message),
                    };
                    A::handle(&mut state, &mut extras, msg, &mut sink);
                    route_outputs(&mut sink, &bridge.outgoing, &self_tx);
                }
            }
            local_msg = local_rx.recv() => {
                if let Some(message) = local_msg {
                    let msg = AddressedMessage {
                        from: ClientSource::Local,
                        content: ClientMessage::Message(message),
                    };
                    A::handle(&mut state, &mut extras, msg, &mut sink);
                    route_outputs(&mut sink, &bridge.outgoing, &self_tx);
                }
            }
        }
    }
}

fn route_outputs<M: ChannelMessage>(
    sink: &mut ClientSink<M>,
    outgoing: &mpsc::UnboundedSender<M>,
    self_tx: &mpsc::UnboundedSender<M>,
) {
    for output in sink.drain() {
        match output {
            ClientOutput::ToServer { message } => {
                let _ = outgoing.send(message);
            }
            ClientOutput::Internal { message } => {
                let _ = self_tx.send(message);
            }
        }
    }
}
