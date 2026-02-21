//! Tests demonstrating game actor usage.
//!
//! These tests use types from server_actor and client_actor to demonstrate
//! the full game actor pattern.

use crate::actor::{Actor, ActorProtocol, AddressedMessage, Sink};
use crate::game_actor::SelfTarget;
use crate::server_actor::{
    ServerMessage, ServerSink, ServerOutput, ServerSource,
    UserTarget, UserDestination,
};
use crate::client_actor::{
    ClientMessage, ClientSink, ClientOutput, ClientSource,
    ServerTarget,
};

// ============================================================================
// Game message type (shared between server and client)
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
enum GameMsg {
    Chat(String),
    Move { x: i32, y: i32 },
    Ping,
    Pong,
}

// ============================================================================
// Server-side game actor
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct UserId(u64);

struct ServerGameActor;

struct ServerState {
    message_count: u32,
}

impl ActorProtocol for ServerGameActor {
    type ActorId = ServerSource<UserId>;
    type Message = ServerMessage<GameMsg>;
}

impl<S> Actor<S> for ServerGameActor
where
    S: Sink<UserTarget<UserId, GameMsg>> + Sink<SelfTarget<GameMsg>>,
{
    type Config = ();
    type State = ServerState;
    type Extras = ();

    fn startup(_config: Self::Config) -> Self::State {
        ServerState { message_count: 0 }
    }

    fn handle(
        state: &mut Self::State,
        _extras: &mut Self::Extras,
        message: AddressedMessage<Self::ActorId, Self::Message>,
        sink: &mut S,
    ) {
        state.message_count += 1;

        match (&message.from, &message.content) {
            (ServerSource::User(user), ServerMessage::UserConnected) => {
                Sink::<UserTarget<UserId, GameMsg>>::send(
                    sink,
                    UserDestination::User(user.clone()),
                    GameMsg::Chat("Welcome!".into()),
                );
                Sink::<UserTarget<UserId, GameMsg>>::send(
                    sink,
                    UserDestination::Broadcast,
                    GameMsg::Chat(format!("User {} joined", user.0)),
                );
            }
            (ServerSource::User(user), ServerMessage::UserDisconnected) => {
                Sink::<UserTarget<UserId, GameMsg>>::send(
                    sink,
                    UserDestination::Broadcast,
                    GameMsg::Chat(format!("User {} left", user.0)),
                );
            }
            (ServerSource::User(user), ServerMessage::Message(GameMsg::Ping)) => {
                Sink::<UserTarget<UserId, GameMsg>>::send(
                    sink,
                    UserDestination::User(user.clone()),
                    GameMsg::Pong,
                );
            }
            (ServerSource::User(_), ServerMessage::Message(GameMsg::Chat(text))) => {
                Sink::<UserTarget<UserId, GameMsg>>::send(
                    sink,
                    UserDestination::Broadcast,
                    GameMsg::Chat(text.clone()),
                );
            }
            (ServerSource::Internal, ServerMessage::Message(GameMsg::Ping)) => {
                Sink::<SelfTarget<GameMsg>>::send(sink, (), GameMsg::Pong);
            }
            _ => {}
        }
    }

    fn shutdown(_state: &mut Self::State, _extras: &mut Self::Extras) {}
}

// ============================================================================
// Client-side game actor
// ============================================================================

struct ClientGameActor;

struct ClientState {
    connected: bool,
    position: (i32, i32),
}

impl ActorProtocol for ClientGameActor {
    type ActorId = ClientSource;
    type Message = ClientMessage<GameMsg>;
}

impl<S> Actor<S> for ClientGameActor
where
    S: Sink<ServerTarget<GameMsg>> + Sink<SelfTarget<GameMsg>>,
{
    type Config = ();
    type State = ClientState;
    type Extras = ();

    fn startup(_config: Self::Config) -> Self::State {
        ClientState {
            connected: false,
            position: (0, 0),
        }
    }

    fn handle(
        state: &mut Self::State,
        _extras: &mut Self::Extras,
        message: AddressedMessage<Self::ActorId, Self::Message>,
        sink: &mut S,
    ) {
        match (&message.from, &message.content) {
            (ClientSource::Server, ClientMessage::Connected) => {
                state.connected = true;
                Sink::<ServerTarget<GameMsg>>::send(
                    sink,
                    (),
                    GameMsg::Chat("Hello from client!".into()),
                );
            }
            (ClientSource::Server, ClientMessage::Disconnected) => {
                state.connected = false;
            }
            (ClientSource::Server, ClientMessage::Message(GameMsg::Pong)) => {
                // Got pong from server
            }
            (ClientSource::Internal, ClientMessage::Message(GameMsg::Move { x, y })) => {
                state.position = (*x, *y);
                if state.connected {
                    Sink::<ServerTarget<GameMsg>>::send(
                        sink,
                        (),
                        GameMsg::Move { x: *x, y: *y },
                    );
                }
            }
            _ => {}
        }
    }

    fn shutdown(_state: &mut Self::State, _extras: &mut Self::Extras) {}
}

// ============================================================================
// Tests
// ============================================================================

#[test]
fn server_actor_user_connect() {
    let mut state = <ServerGameActor as Actor<ServerSink<UserId, GameMsg>>>::startup(());
    let mut extras = ();
    let mut sink = ServerSink::<UserId, GameMsg>::new();

    let msg = AddressedMessage {
        from: ServerSource::User(UserId(42)),
        content: ServerMessage::UserConnected,
    };

    <ServerGameActor as Actor<ServerSink<UserId, GameMsg>>>::handle(
        &mut state,
        &mut extras,
        msg,
        &mut sink,
    );

    let outputs: Vec<_> = sink.drain().collect();
    assert_eq!(outputs.len(), 2);

    match &outputs[0] {
        ServerOutput::ToUser { user, message } => {
            assert_eq!(user, &UserId(42));
            assert_eq!(message, &GameMsg::Chat("Welcome!".into()));
        }
        _ => panic!("Expected ToUser"),
    }

    match &outputs[1] {
        ServerOutput::Broadcast { message } => {
            assert_eq!(message, &GameMsg::Chat("User 42 joined".into()));
        }
        _ => panic!("Expected Broadcast"),
    }
}

#[test]
fn server_actor_ping_pong() {
    let mut state = <ServerGameActor as Actor<ServerSink<UserId, GameMsg>>>::startup(());
    let mut extras = ();
    let mut sink = ServerSink::<UserId, GameMsg>::new();

    let msg = AddressedMessage {
        from: ServerSource::User(UserId(1)),
        content: ServerMessage::Message(GameMsg::Ping),
    };

    <ServerGameActor as Actor<ServerSink<UserId, GameMsg>>>::handle(
        &mut state,
        &mut extras,
        msg,
        &mut sink,
    );

    let outputs: Vec<_> = sink.drain().collect();
    assert_eq!(outputs.len(), 1);

    match &outputs[0] {
        ServerOutput::ToUser { user, message } => {
            assert_eq!(user, &UserId(1));
            assert_eq!(message, &GameMsg::Pong);
        }
        _ => panic!("Expected ToUser with Pong"),
    }
}

#[test]
fn server_actor_internal_message() {
    let mut state = <ServerGameActor as Actor<ServerSink<UserId, GameMsg>>>::startup(());
    let mut extras = ();
    let mut sink = ServerSink::<UserId, GameMsg>::new();

    let msg = AddressedMessage {
        from: ServerSource::Internal,
        content: ServerMessage::Message(GameMsg::Ping),
    };

    <ServerGameActor as Actor<ServerSink<UserId, GameMsg>>>::handle(
        &mut state,
        &mut extras,
        msg,
        &mut sink,
    );

    let outputs: Vec<_> = sink.drain().collect();
    assert_eq!(outputs.len(), 1);

    match &outputs[0] {
        ServerOutput::Internal { message } => {
            assert_eq!(message, &GameMsg::Pong);
        }
        _ => panic!("Expected Internal"),
    }
}

#[test]
fn client_actor_connect_sends_hello() {
    let mut state = <ClientGameActor as Actor<ClientSink<GameMsg>>>::startup(());
    let mut extras = ();
    let mut sink = ClientSink::<GameMsg>::new();

    let msg = AddressedMessage {
        from: ClientSource::Server,
        content: ClientMessage::Connected,
    };

    <ClientGameActor as Actor<ClientSink<GameMsg>>>::handle(
        &mut state,
        &mut extras,
        msg,
        &mut sink,
    );

    assert!(state.connected);

    let outputs: Vec<_> = sink.drain().collect();
    assert_eq!(outputs.len(), 1);

    match &outputs[0] {
        ClientOutput::ToServer { message } => {
            assert_eq!(message, &GameMsg::Chat("Hello from client!".into()));
        }
        _ => panic!("Expected ToServer"),
    }
}

#[test]
fn client_actor_internal_move() {
    let mut state = <ClientGameActor as Actor<ClientSink<GameMsg>>>::startup(());
    state.connected = true;
    let mut extras = ();
    let mut sink = ClientSink::<GameMsg>::new();

    let msg = AddressedMessage {
        from: ClientSource::Internal,
        content: ClientMessage::Message(GameMsg::Move { x: 10, y: 20 }),
    };

    <ClientGameActor as Actor<ClientSink<GameMsg>>>::handle(
        &mut state,
        &mut extras,
        msg,
        &mut sink,
    );

    assert_eq!(state.position, (10, 20));

    let outputs: Vec<_> = sink.drain().collect();
    assert_eq!(outputs.len(), 1);

    match &outputs[0] {
        ClientOutput::ToServer { message } => {
            assert_eq!(message, &GameMsg::Move { x: 10, y: 20 });
        }
        _ => panic!("Expected ToServer"),
    }
}

#[test]
fn client_actor_move_while_disconnected_does_not_send() {
    let mut state = <ClientGameActor as Actor<ClientSink<GameMsg>>>::startup(());
    let mut extras = ();
    let mut sink = ClientSink::<GameMsg>::new();

    let msg = AddressedMessage {
        from: ClientSource::Internal,
        content: ClientMessage::Message(GameMsg::Move { x: 5, y: 5 }),
    };

    <ClientGameActor as Actor<ClientSink<GameMsg>>>::handle(
        &mut state,
        &mut extras,
        msg,
        &mut sink,
    );

    assert_eq!(state.position, (5, 5));

    let outputs: Vec<_> = sink.drain().collect();
    assert_eq!(outputs.len(), 0);
}
