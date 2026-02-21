//! Server actor wrapper and main loop.

use super::sink::{ServerOutput, ServerSink};
use super::types::{ServerActorConfig, ServerMessage, ServerSource};
use super::InternalServerEvent;
use crate::actor::{Actor, ActorProtocol, AddressedMessage};
use crate::utils::ChannelMessage;
use dashmap::DashMap;
use multiplayer_kit_protocol::{RoomConfig, UserContext};
use multiplayer_kit_server::GameServerContext;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Wrap an Actor implementation for use as a server room handler.
///
/// The actor receives `AddressedMessage<ServerSource<T>, ServerMessage<M>>` where:
/// - `ServerSource::User(user)` for messages from users
/// - `ServerSource::Internal` for self-scheduled messages
/// - `ServerMessage::UserConnected` / `UserDisconnected` / `Message(M)`
///
/// Outputs via the sink are routed:
/// - `Sink<UserTarget<T, M>>` with `UserDestination::User(u)` -> send to specific user
/// - `Sink<UserTarget<T, M>>` with `UserDestination::Broadcast` -> broadcast to all
/// - `Sink<SelfTarget<M>>` -> schedule message back to self
pub fn with_server_actor<A, T, M, C, Ctx>()
-> impl Fn(
    multiplayer_kit_server::Room<T>,
    C,
    Arc<Ctx>,
) -> Pin<Box<dyn Future<Output = ()> + Send>>
   + Send
   + Sync
   + Clone
   + 'static
where
    T: UserContext,
    M: ChannelMessage,
    C: RoomConfig + 'static,
    Ctx: GameServerContext,
    A: Actor<ServerSink<T, M>>
        + ActorProtocol<ActorId = ServerSource<T>, Message = ServerMessage<M>>
        + 'static,
    A::Config: From<ServerActorConfig<C>>,
    A::State: Send + 'static,
    A::Extras: From<Ctx::RoomExtras> + Send + 'static,
{
    move |room: multiplayer_kit_server::Room<T>, config: C, ctx: Arc<Ctx>| {
        Box::pin(async move {
            run_server_actor::<A, T, M, C, Ctx>(room, config, ctx).await;
        })
    }
}

async fn run_server_actor<A, T, M, C, Ctx>(
    mut room: multiplayer_kit_server::Room<T>,
    config: C,
    game_context: Arc<Ctx>,
) where
    T: UserContext,
    M: ChannelMessage,
    C: RoomConfig + 'static,
    Ctx: GameServerContext,
    A: Actor<ServerSink<T, M>>
        + ActorProtocol<ActorId = ServerSource<T>, Message = ServerMessage<M>>
        + 'static,
    A::Config: From<ServerActorConfig<C>>,
    A::State: Send + 'static,
    A::Extras: From<Ctx::RoomExtras> + Send + 'static,
{
    use multiplayer_kit_server::Accept;

    let room_id = room.room_id();

    // Create actor config and state
    let actor_config = ServerActorConfig {
        room_id,
        room_config: config,
    };
    let mut state = A::startup(actor_config.into());
    let mut extras: A::Extras = game_context.get_room_extras(room_id).into();

    let (event_tx, mut event_rx) = mpsc::channel::<InternalServerEvent<T, M>>(256);
    let (self_tx, mut self_rx) = mpsc::channel::<M>(64);

    let user_channels: Arc<DashMap<multiplayer_kit_protocol::ChannelId, (T, M::Channel)>> =
        Arc::new(DashMap::new());

    let pending_users: Arc<DashMap<T::Id, (T, HashMap<M::Channel, multiplayer_kit_protocol::ChannelId>)>> =
        Arc::new(DashMap::new());

    let connected_users: Arc<DashMap<T::Id, T>> = Arc::new(DashMap::new());

    // We need to keep track of self_tx for routing internal messages
    let self_tx_for_routing = self_tx.clone();

    let handle = room.handle();
    let expected_channels = M::all_channels().len();

    let mut sink = ServerSink::<T, M>::new();

    loop {
        tokio::select! {
            accept = room.accept() => {
                match accept {
                    Some(Accept::NewChannel(user, channel)) => {
                        let channel_id = channel.id;
                        let tx = event_tx.clone();
                        let uc = Arc::clone(&user_channels);
                        let pu = Arc::clone(&pending_users);
                        let cu = Arc::clone(&connected_users);

                        tokio::spawn(async move {
                            super::channel::handle_server_channel::<T, M>(
                                channel, user, channel_id, tx, uc, pu, cu, expected_channels,
                            )
                            .await;
                        });
                    }
                    Some(Accept::Closing) => {
                        A::shutdown(&mut state, &mut extras);
                        break;
                    }
                    None => break,
                }
            }
            event = event_rx.recv() => {
                match event {
                    Some(InternalServerEvent::UserReady(user)) => {
                        let msg = AddressedMessage {
                            from: ServerSource::User(user),
                            content: ServerMessage::UserConnected,
                        };
                        A::handle(&mut state, &mut extras, msg, &mut sink);
                        route_outputs::<T, M>(&mut sink, &handle, &user_channels, &self_tx_for_routing);
                    }
                    Some(InternalServerEvent::UserGone(user)) => {
                        let msg = AddressedMessage {
                            from: ServerSource::User(user),
                            content: ServerMessage::UserDisconnected,
                        };
                        A::handle(&mut state, &mut extras, msg, &mut sink);
                        route_outputs::<T, M>(&mut sink, &handle, &user_channels, &self_tx_for_routing);
                    }
                    Some(InternalServerEvent::Message { sender, channel: _, message }) => {
                        let msg = AddressedMessage {
                            from: ServerSource::User(sender),
                            content: ServerMessage::Message(message),
                        };
                        A::handle(&mut state, &mut extras, msg, &mut sink);
                        route_outputs::<T, M>(&mut sink, &handle, &user_channels, &self_tx_for_routing);
                    }
                    None => break,
                }
            }
            self_msg = self_rx.recv() => {
                if let Some(message) = self_msg {
                    let msg = AddressedMessage {
                        from: ServerSource::Internal,
                        content: ServerMessage::Message(message),
                    };
                    A::handle(&mut state, &mut extras, msg, &mut sink);
                    route_outputs::<T, M>(&mut sink, &handle, &user_channels, &self_tx_for_routing);
                }
            }
        }
    }
}

/// Route outputs from the sink to their destinations.
fn route_outputs<T, M>(
    sink: &mut ServerSink<T, M>,
    handle: &multiplayer_kit_server::RoomHandle<T>,
    user_channels: &Arc<DashMap<multiplayer_kit_protocol::ChannelId, (T, M::Channel)>>,
    self_tx: &mpsc::Sender<M>,
) where
    T: UserContext,
    M: ChannelMessage,
{
    use crate::utils::frame_message;

    for output in sink.drain() {
        match output {
            ServerOutput::ToUser { user, message } => {
                if let Some(channel_type) = message.channel() {
                    if let Ok(data) = message.encode() {
                        let framed = frame_message(&data);
                        let channel_ids: Vec<_> = user_channels
                            .iter()
                            .filter(|r| r.value().1 == channel_type && r.value().0.id() == user.id())
                            .map(|r| *r.key())
                            .collect();
                        handle.send_to(&channel_ids, &framed);
                    }
                }
            }
            ServerOutput::Broadcast { message } => {
                if let Some(channel_type) = message.channel() {
                    if let Ok(data) = message.encode() {
                        let framed = frame_message(&data);
                        let channel_ids: Vec<_> = user_channels
                            .iter()
                            .filter(|r| r.value().1 == channel_type)
                            .map(|r| *r.key())
                            .collect();
                        handle.send_to(&channel_ids, &framed);
                    }
                }
            }
            ServerOutput::Internal { message } => {
                let _ = self_tx.try_send(message);
            }
        }
    }
}
