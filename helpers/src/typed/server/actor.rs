//! Server actor wrapper and main loop.

use super::{ServerInternalEvent, TypedContext, TypedEvent};
use crate::typed::TypedProtocol;
use dashmap::DashMap;
use multiplayer_kit_protocol::UserContext;
use multiplayer_kit_server::GameServerContext;
use std::collections::HashMap;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Wrap a typed actor function for the server.
///
/// The actor function receives:
/// - `TypedContext<T, P, Ctx>` for sending messages and accessing game context
/// - `TypedEvent<T, P>` the current event
/// - `Arc<C>` the room config (shared, immutable)
///
/// # Example
///
/// ```ignore
/// .room_handler(with_typed_actor::<
///     MyUser,
///     MyProtocol,
///     MyRoomConfig,
///     MyGameContext,
///     _,
///     _,
/// >(my_actor))
/// ```
pub fn with_typed_actor<T, P, C, Ctx, F, Fut>(
    actor_fn: F,
) -> impl Fn(
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
    P: TypedProtocol,
    C: multiplayer_kit_protocol::RoomConfig + 'static,
    Ctx: GameServerContext,
    F: Fn(TypedContext<T, P, Ctx>, TypedEvent<T, P>, Arc<C>) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let actor_fn = Arc::new(actor_fn);
    move |room: multiplayer_kit_server::Room<T>, config: C, ctx: Arc<Ctx>| {
        let actor_fn = Arc::clone(&actor_fn);
        Box::pin(async move {
            run_typed_server_actor::<T, P, C, Ctx, F, Fut>(room, config, ctx, actor_fn).await;
        })
    }
}

/// Internal: run the typed server actor loop.
async fn run_typed_server_actor<T, P, C, Ctx, F, Fut>(
    mut room: multiplayer_kit_server::Room<T>,
    config: C,
    game_context: Arc<Ctx>,
    actor_fn: Arc<F>,
) where
    T: UserContext,
    P: TypedProtocol,
    C: multiplayer_kit_protocol::RoomConfig + 'static,
    Ctx: GameServerContext,
    F: Fn(TypedContext<T, P, Ctx>, TypedEvent<T, P>, Arc<C>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let config = Arc::new(config);
    use multiplayer_kit_server::Accept;

    let (event_tx, mut event_rx) = mpsc::channel::<ServerInternalEvent<T, P>>(256);
    let (self_tx, mut self_rx) = mpsc::channel::<P::Event>(64);

    let user_channels: Arc<DashMap<multiplayer_kit_protocol::ChannelId, (T, P::Channel)>> =
        Arc::new(DashMap::new());

    let pending_users: Arc<DashMap<T::Id, (T, HashMap<P::Channel, multiplayer_kit_protocol::ChannelId>)>> =
        Arc::new(DashMap::new());

    let connected_users: Arc<DashMap<T::Id, T>> = Arc::new(DashMap::new());

    let ctx = TypedContext {
        room_id: room.room_id(),
        self_tx,
        handle: room.handle(),
        user_channels: user_channels.clone(),
        game_context,
        _phantom: PhantomData,
    };

    let expected_channels = P::all_channels().len();

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
                            super::channel::handle_typed_channel::<T, P>(
                                channel, user, channel_id, tx, uc, pu, cu, expected_channels,
                            )
                            .await;
                        });
                    }
                    Some(Accept::Closing) => {
                        actor_fn(ctx.clone(), TypedEvent::Shutdown, Arc::clone(&config)).await;
                        break;
                    }
                    None => break,
                }
            }
            event = event_rx.recv() => {
                match event {
                    Some(ServerInternalEvent::UserReady(user)) => {
                        actor_fn(ctx.clone(), TypedEvent::UserConnected(user), Arc::clone(&config)).await;
                    }
                    Some(ServerInternalEvent::UserGone(user)) => {
                        actor_fn(ctx.clone(), TypedEvent::UserDisconnected(user), Arc::clone(&config)).await;
                    }
                    Some(ServerInternalEvent::Message { sender, channel_type, event }) => {
                        actor_fn(
                            ctx.clone(),
                            TypedEvent::Message {
                                sender,
                                channel: channel_type,
                                event,
                            },
                            Arc::clone(&config),
                        )
                        .await;
                    }
                    None => break,
                }
            }
            self_event = self_rx.recv() => {
                if let Some(event) = self_event {
                    actor_fn(ctx.clone(), TypedEvent::Internal(event), Arc::clone(&config)).await;
                }
            }
        }
    }
}
