//! End-to-end tests for server and client actors communicating.

use super::{TestHarness, TestUser};
use crate::actor::{Actor, ActorProtocol, AddressedMessage, Sink};
use crate::client_actor::{ClientMessage, ClientSink, ClientSource, ServerTarget};
use crate::server_actor::ServerSink;
use crate::game_actor::SelfTarget;
use crate::server_actor::{ServerMessage, ServerSource, UserDestination, UserTarget};
use crate::utils::{ChannelMessage, DecodeError, EncodeError};

// ============================================================================
// Shared message type (implements ChannelMessage for routing)
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
enum GameMsg {
    Chat(String),
    Ping,
    Pong,
    Echo(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum GameChannel {
    Main = 0,
}

impl ChannelMessage for GameMsg {
    type Channel = GameChannel;

    fn channel(&self) -> Option<GameChannel> {
        Some(GameChannel::Main)
    }

    fn all_channels() -> &'static [GameChannel] {
        &[GameChannel::Main]
    }

    fn channel_to_id(channel: GameChannel) -> u8 {
        channel as u8
    }

    fn channel_from_id(id: u8) -> Option<GameChannel> {
        match id {
            0 => Some(GameChannel::Main),
            _ => None,
        }
    }

    fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        Ok(format!("{:?}", self).into_bytes())
    }

    fn decode(_channel: GameChannel, _data: &[u8]) -> Result<Self, DecodeError> {
        Err(DecodeError::Deserialize("not implemented".to_string()))
    }
}

// ============================================================================
// Server Actor
// ============================================================================

struct EchoServerActor;

struct ServerState {
    total_messages: u32,
}

impl ActorProtocol for EchoServerActor {
    type ActorId = ServerSource<TestUser>;
    type Message = ServerMessage<GameMsg>;
}

impl<S> Actor<S> for EchoServerActor
where
    S: Sink<UserTarget<TestUser, GameMsg>> + Sink<SelfTarget<GameMsg>>,
{
    type Config = ();
    type State = ServerState;
    type Extras = ();

    fn startup(_config: ()) -> ServerState {
        ServerState { total_messages: 0 }
    }

    fn handle(
        state: &mut ServerState,
        _extras: &mut (),
        message: AddressedMessage<ServerSource<TestUser>, ServerMessage<GameMsg>>,
        sink: &mut S,
    ) {
        state.total_messages += 1;

        match (&message.from, &message.content) {
            (ServerSource::User(user), ServerMessage::UserConnected) => {
                // Welcome the user
                Sink::<UserTarget<TestUser, GameMsg>>::send(
                    sink,
                    UserDestination::User(user.clone()),
                    GameMsg::Chat(format!("Welcome, {}!", user.name)),
                );
                // Announce to others
                Sink::<UserTarget<TestUser, GameMsg>>::send(
                    sink,
                    UserDestination::Broadcast,
                    GameMsg::Chat(format!("{} joined", user.name)),
                );
            }
            (ServerSource::User(user), ServerMessage::UserDisconnected) => {
                Sink::<UserTarget<TestUser, GameMsg>>::send(
                    sink,
                    UserDestination::Broadcast,
                    GameMsg::Chat(format!("{} left", user.name)),
                );
            }
            (ServerSource::User(user), ServerMessage::Message(GameMsg::Ping)) => {
                // Reply with Pong to sender only
                Sink::<UserTarget<TestUser, GameMsg>>::send(
                    sink,
                    UserDestination::User(user.clone()),
                    GameMsg::Pong,
                );
            }
            (ServerSource::User(user), ServerMessage::Message(GameMsg::Chat(text))) => {
                // Broadcast chat with username
                Sink::<UserTarget<TestUser, GameMsg>>::send(
                    sink,
                    UserDestination::Broadcast,
                    GameMsg::Chat(format!("{}: {}", user.name, text)),
                );
            }
            (ServerSource::User(user), ServerMessage::Message(GameMsg::Echo(text))) => {
                // Echo back to sender only
                Sink::<UserTarget<TestUser, GameMsg>>::send(
                    sink,
                    UserDestination::User(user.clone()),
                    GameMsg::Echo(text.clone()),
                );
            }
            _ => {}
        }
    }

    fn shutdown(_state: &mut ServerState, _extras: &mut ()) {}
}

// ============================================================================
// Client Actor
// ============================================================================

struct SimpleClientActor;

struct ClientState {
    received_messages: Vec<GameMsg>,
}

impl ActorProtocol for SimpleClientActor {
    type ActorId = ClientSource;
    type Message = ClientMessage<GameMsg>;
}

impl<S> Actor<S> for SimpleClientActor
where
    S: Sink<ServerTarget<GameMsg>> + Sink<SelfTarget<GameMsg>>,
{
    type Config = ();
    type State = ClientState;
    type Extras = ();

    fn startup(_config: ()) -> ClientState {
        ClientState {
            received_messages: Vec::new(),
        }
    }

    fn handle(
        state: &mut ClientState,
        _extras: &mut (),
        message: AddressedMessage<ClientSource, ClientMessage<GameMsg>>,
        sink: &mut S,
    ) {
        match (&message.from, &message.content) {
            (ClientSource::Server, ClientMessage::Connected) => {
                // Send a ping when connected
                Sink::<ServerTarget<GameMsg>>::send(sink, (), GameMsg::Ping);
            }
            (ClientSource::Server, ClientMessage::Message(msg)) => {
                state.received_messages.push(msg.clone());
            }
            _ => {}
        }
    }

    fn shutdown(_state: &mut ClientState, _extras: &mut ()) {}
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn test_server_client_ping_pong() {
    let mut harness = TestHarness::<GameMsg>::new();

    // Server state (explicit type for Actor<ServerSink>)
    let mut server_state =
        <EchoServerActor as Actor<ServerSink<TestUser, GameMsg>>>::startup(());
    let mut server_extras = ();

    // Connect a client
    let client1 = harness.connect_user("Alice");

    // Client state (explicit type for Actor<ClientSink>)
    let mut client_state = <SimpleClientActor as Actor<ClientSink<GameMsg>>>::startup(());
    let mut client_extras = ();

    // Deliver user connected to server
    harness
        .deliver_user_connected::<EchoServerActor, ()>(&client1.user, &mut server_state, &mut server_extras)
        .await;

    // Client receives welcome and join broadcast
    let msg1 = client1.recv_timeout(std::time::Duration::from_millis(100)).await;
    let msg2 = client1.recv_timeout(std::time::Duration::from_millis(100)).await;
    assert!(msg1.is_some());
    assert!(msg2.is_some());

    // Deliver connected event to client
    let to_server = client1.deliver_connected::<SimpleClientActor>(&mut client_state, &mut client_extras);
    assert_eq!(to_server.len(), 1);
    assert_eq!(to_server[0], GameMsg::Ping);

    // Send ping to server
    client1.send(GameMsg::Ping).await;

    // Server processes the ping
    harness
        .run_server_tick::<EchoServerActor, ()>(&mut server_state, &mut server_extras)
        .await;

    // Client receives pong
    let pong = client1.recv_timeout(std::time::Duration::from_millis(100)).await;
    assert_eq!(pong, Some(GameMsg::Pong));

    assert_eq!(server_state.total_messages, 2); // UserConnected + Ping
}

#[tokio::test]
async fn test_multiple_clients_broadcast() {
    let mut harness = TestHarness::<GameMsg>::new();

    // Server state
    let mut server_state =
        <EchoServerActor as Actor<ServerSink<TestUser, GameMsg>>>::startup(());
    let mut server_extras = ();

    // Connect two clients
    let client1 = harness.connect_user("Alice");
    let client2 = harness.connect_user("Bob");

    // Deliver connections to server
    harness
        .deliver_user_connected::<EchoServerActor, ()>(&client1.user, &mut server_state, &mut server_extras)
        .await;
    harness
        .deliver_user_connected::<EchoServerActor, ()>(&client2.user, &mut server_state, &mut server_extras)
        .await;

    // Clear initial messages
    while client1.try_recv().await.is_some() {}
    while client2.try_recv().await.is_some() {}

    // Alice sends a chat message
    client1.send(GameMsg::Chat("Hello everyone!".to_string())).await;

    // Server processes
    harness
        .run_server_tick::<EchoServerActor, ()>(&mut server_state, &mut server_extras)
        .await;

    // Both clients receive the broadcast
    let msg1 = client1.recv_timeout(std::time::Duration::from_millis(100)).await;
    let msg2 = client2.recv_timeout(std::time::Duration::from_millis(100)).await;

    assert_eq!(
        msg1,
        Some(GameMsg::Chat("Alice: Hello everyone!".to_string()))
    );
    assert_eq!(
        msg2,
        Some(GameMsg::Chat("Alice: Hello everyone!".to_string()))
    );
}

#[tokio::test]
async fn test_echo_to_sender_only() {
    let mut harness = TestHarness::<GameMsg>::new();

    // Server state
    let mut server_state =
        <EchoServerActor as Actor<ServerSink<TestUser, GameMsg>>>::startup(());
    let mut server_extras = ();

    // Connect two clients
    let client1 = harness.connect_user("Alice");
    let client2 = harness.connect_user("Bob");

    // Deliver connections
    harness
        .deliver_user_connected::<EchoServerActor, ()>(&client1.user, &mut server_state, &mut server_extras)
        .await;
    harness
        .deliver_user_connected::<EchoServerActor, ()>(&client2.user, &mut server_state, &mut server_extras)
        .await;

    // Clear initial messages
    while client1.try_recv().await.is_some() {}
    while client2.try_recv().await.is_some() {}

    // Alice sends an echo request
    client1.send(GameMsg::Echo("secret".to_string())).await;

    // Server processes
    harness
        .run_server_tick::<EchoServerActor, ()>(&mut server_state, &mut server_extras)
        .await;

    // Only Alice receives the echo
    let msg1 = client1.recv_timeout(std::time::Duration::from_millis(100)).await;
    let msg2 = client2.recv_timeout(std::time::Duration::from_millis(100)).await;

    assert_eq!(msg1, Some(GameMsg::Echo("secret".to_string())));
    assert_eq!(msg2, None); // Bob doesn't receive it
}

#[tokio::test]
async fn test_user_disconnect_broadcast() {
    let mut harness = TestHarness::<GameMsg>::new();

    // Server state
    let mut server_state =
        <EchoServerActor as Actor<ServerSink<TestUser, GameMsg>>>::startup(());
    let mut server_extras = ();

    // Connect two clients
    let client1 = harness.connect_user("Alice");
    let client2 = harness.connect_user("Bob");

    // Deliver connections
    harness
        .deliver_user_connected::<EchoServerActor, ()>(&client1.user, &mut server_state, &mut server_extras)
        .await;
    harness
        .deliver_user_connected::<EchoServerActor, ()>(&client2.user, &mut server_state, &mut server_extras)
        .await;

    // Clear initial messages
    while client1.try_recv().await.is_some() {}
    while client2.try_recv().await.is_some() {}

    // Alice disconnects
    harness
        .deliver_user_disconnected::<EchoServerActor, ()>(&client1.user, &mut server_state, &mut server_extras)
        .await;

    // Bob receives the disconnect broadcast
    let msg = client2.recv_timeout(std::time::Duration::from_millis(100)).await;
    assert_eq!(msg, Some(GameMsg::Chat("Alice left".to_string())));
}
