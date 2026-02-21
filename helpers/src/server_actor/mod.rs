//! Server-side actor for handling game rooms.
//!
//! Provides Actor trait-based message handling for server room actors.
//!
//! # Architecture
//!
//! - `ServerMessage<M>` - Messages delivered to the actor (wraps lifecycle + game messages)
//! - `ServerSource<User>` - Who sent the message (user or internal)
//! - `ServerSink` - Collects outputs to route to users
//! - `with_server_actor` - Wrapper to create an actor-based room handler
//!
//! # Example
//!
//! ```ignore
//! struct MyActor;
//!
//! impl ActorProtocol for MyActor {
//!     type ActorId = ServerSource<MyUser>;
//!     type Message = ServerMessage<MyGameMsg>;
//! }
//!
//! impl<S> Actor<S> for MyActor
//! where
//!     S: Sink<UserTarget<MyUser, MyGameMsg>> + Sink<SelfTarget<MyGameMsg>>,
//! {
//!     type Config = ServerActorConfig<MyRoomConfig>;
//!     type State = MyState;
//!     type Extras = MyExtras;
//!
//!     fn startup(config: Self::Config) -> Self::State { ... }
//!     fn handle(state, extras, msg, sink) { ... }
//!     fn shutdown(state, extras) { ... }
//! }
//!
//! // Use in server builder:
//! .room_handler(with_server_actor::<MyActor, MyUser, MyGameMsg, MyRoomConfig, MyCtx>())
//! ```

mod actor;
mod channel;
mod types;
mod sink;

use crate::utils::ChannelMessage;
use multiplayer_kit_protocol::UserContext;

pub use actor::with_server_actor;
pub use types::{UserTarget, UserDestination, ServerMessage, ServerSource, ServerActorConfig};
pub use sink::{ServerSink, ServerOutput};

/// Internal event type for actor communication.
pub(crate) enum InternalServerEvent<T: UserContext, M: ChannelMessage> {
    UserReady(T),
    UserGone(T),
    Message {
        sender: T,
        #[allow(dead_code)]
        channel: M::Channel,
        message: M,
    },
}
