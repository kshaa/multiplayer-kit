//! Chat server actor implementation.

use chat_protocol::{ChatEvent, ChatMessage, ChatRoomConfig, ChatUser};
use multiplayer_kit_helpers::{
    actor::{Actor, ActorProtocol, AddressedMessage, Sink},
    game_actor::SelfTarget,
    ServerActorConfig, ServerMessage, ServerSource, UserDestination, UserTarget,
};

/// Extras passed to the actor per room.
pub struct ChatRoomExtras {
    pub server_name: String,
}

/// Chat room actor.
pub struct ChatActor;

/// Actor state.
pub struct ChatState {
    pub room_name: String,
    pub message_count: u64,
}

impl ActorProtocol for ChatActor {
    type ActorId = ServerSource<ChatUser>;
    type Message = ServerMessage<ChatEvent>;
}

impl<S> Actor<S> for ChatActor
where
    S: Sink<UserTarget<ChatUser, ChatEvent>> + Sink<SelfTarget<ChatEvent>>,
{
    type Config = ServerActorConfig<ChatRoomConfig>;
    type State = ChatState;
    type Extras = ChatRoomExtras;

    fn startup(config: Self::Config) -> Self::State {
        tracing::info!(
            "[Room {:?}] Starting '{}'",
            config.room_id,
            config.room_config.name
        );
        ChatState {
            room_name: config.room_config.name,
            message_count: 0,
        }
    }

    fn handle(
        state: &mut Self::State,
        extras: &mut Self::Extras,
        message: AddressedMessage<Self::ActorId, Self::Message>,
        sink: &mut S,
    ) {
        match (&message.from, &message.content) {
            (ServerSource::User(user), ServerMessage::UserConnected) => {
                tracing::info!(
                    "[{}] [Room '{}'] {} connected",
                    extras.server_name,
                    state.room_name,
                    user.username
                );

                let msg = ChatEvent::Chat(ChatMessage::System(format!(
                    "*** {} joined '{}' ***",
                    user.username, state.room_name
                )));
                Sink::<UserTarget<ChatUser, ChatEvent>>::send(
                    sink,
                    UserDestination::Broadcast,
                    msg,
                );
            }

            (ServerSource::User(user), ServerMessage::UserDisconnected) => {
                tracing::info!(
                    "[{}] [Room '{}'] {} disconnected",
                    extras.server_name,
                    state.room_name,
                    user.username
                );

                let msg = ChatEvent::Chat(ChatMessage::System(format!(
                    "*** {} left the chat ***",
                    user.username
                )));
                Sink::<UserTarget<ChatUser, ChatEvent>>::send(
                    sink,
                    UserDestination::Broadcast,
                    msg,
                );
            }

            (
                ServerSource::User(user),
                ServerMessage::Message(ChatEvent::Chat(ChatMessage::SendText { content })),
            ) => {
                state.message_count += 1;
                tracing::info!(
                    "[{}] [Room '{}'] Message #{}: {}: {}",
                    extras.server_name,
                    state.room_name,
                    state.message_count,
                    user.username,
                    content
                );

                let msg = ChatEvent::Chat(ChatMessage::TextSent {
                    username: user.username.clone(),
                    content: content.clone(),
                });
                Sink::<UserTarget<ChatUser, ChatEvent>>::send(
                    sink,
                    UserDestination::Broadcast,
                    msg,
                );
            }

            _ => {}
        }
    }

    fn shutdown(state: &mut Self::State, extras: &mut Self::Extras) {
        tracing::info!(
            "[{}] [Room '{}'] Shutting down after {} messages",
            extras.server_name,
            state.room_name,
            state.message_count
        );
    }
}
