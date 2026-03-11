//! Client actor wrapper using the Actor trait.

use super::runtime::ActorRuntime;
use super::sink::ClientSink;
use super::types::{ClientMessage, ClientSource};
use super::ClientActorHandle;
use crate::actor::{Actor, ActorProtocol};
use crate::spawning::Spawner;
use crate::utils::{ChannelMessage, GameClientContext, MaybeSend};

/// Run a client actor using the Actor trait.
///
/// Creates an `ActorRuntime` and spawns it in a blocking loop.
/// Returns a handle for sending messages.
///
/// For poll-based usage (e.g., Bevy), use `ActorRuntime::new()` directly.
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
    let ctx_extras = game_context.get_extras();
    let config: A::Config = game_context.into();
    let extras: A::Extras = ctx_extras.into();

    let (runtime, connector) = ActorRuntime::<A, M, ClientSink<M>>::new(config, extras);

    let sender = runtime.server_sender();
    let local_sender = runtime.local_sender();

    let spawner_clone = spawner.clone();
    spawner.spawn_with_local_context(async move {
        let mut runtime = runtime;
        spawner_clone.spawn_local(connector.run(conn, spawner_clone.clone()));
        runtime.run().await;
    });

    ClientActorHandle { sender, local_sender }
}
