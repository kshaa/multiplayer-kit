//! Integration tests using actual ChatActor and ChatClientActor.

use crate::harness::ChatTestHarness;
use chat_client::ChatClientAdapter;
use chat_protocol::{ChatEvent, ChatMessage, ChatRoomConfig};
use chat_server::{ChatActor, ChatRoomExtras};
use multiplayer_kit_helpers::{
    actor::Actor,
    client_actor::ClientSink,
    server_actor::ServerSink,
    ServerActorConfig,
};
use multiplayer_kit_protocol::RoomId;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ============================================================================
// Test Adapter - collects messages for verification
// ============================================================================

/// Messages collected by the test adapter.
#[derive(Default)]
pub struct CollectedMessages {
    pub connected: bool,
    pub disconnected: bool,
    pub text_messages: Vec<(String, String)>,
    pub system_messages: Vec<String>,
}

/// Test adapter that collects all received events.
#[derive(Clone)]
pub struct TestAdapter {
    messages: Arc<Mutex<CollectedMessages>>,
}

impl TestAdapter {
    pub fn new() -> Self {
        Self {
            messages: Arc::new(Mutex::new(CollectedMessages::default())),
        }
    }

    pub fn collected(&self) -> Arc<Mutex<CollectedMessages>> {
        self.messages.clone()
    }
}

impl ChatClientAdapter for TestAdapter {
    fn on_connected(&self) {
        self.messages.lock().unwrap().connected = true;
    }

    fn on_disconnected(&self) {
        self.messages.lock().unwrap().disconnected = true;
    }

    fn on_text_message(&self, username: &str, content: &str) {
        self.messages
            .lock()
            .unwrap()
            .text_messages
            .push((username.to_string(), content.to_string()));
    }

    fn on_system_message(&self, text: &str) {
        self.messages
            .lock()
            .unwrap()
            .system_messages
            .push(text.to_string());
    }
}

// ============================================================================
// Type aliases for cleaner test code
// ============================================================================

type ServerSinkType = ServerSink<chat_protocol::ChatUser, ChatEvent>;
type ClientSinkType = ClientSink<ChatEvent>;
type ClientActor = chat_client::ChatClientActor<TestAdapter>;

// ============================================================================
// Helper functions
// ============================================================================

fn create_server_config(room_name: &str) -> ServerActorConfig<ChatRoomConfig> {
    ServerActorConfig {
        room_id: RoomId(1),
        room_config: ChatRoomConfig {
            name: room_name.to_string(),
            max_players: Some(10),
        },
    }
}

fn create_server_extras() -> ChatRoomExtras {
    ChatRoomExtras {
        server_name: "Test Server".to_string(),
    }
}

/// Create a client actor state and extras, returning both plus the collected messages handle.
fn create_client() -> (
    chat_client::ChatState,
    TestAdapter,
    Arc<Mutex<CollectedMessages>>,
) {
    let adapter = TestAdapter::new();
    let collected = adapter.collected();
    let ctx = chat_client::ChatClientContext::new(adapter.clone());
    let state = <ClientActor as Actor<ClientSinkType>>::startup(ctx);
    // The extras is the adapter itself (from get_extras)
    (state, adapter, collected)
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn test_user_join_broadcasts_system_message() {
    let mut harness = ChatTestHarness::new();

    // Start server
    let config = create_server_config("Test Room");
    let mut server_state = <ChatActor as Actor<ServerSinkType>>::startup(config);
    let mut server_extras = create_server_extras();

    // Connect user
    let alice = harness.connect_user("Alice");

    // Deliver connection to server
    harness
        .deliver_user_connected::<ChatActor>(&alice.user, &mut server_state, &mut server_extras)
        .await;

    // Alice receives join broadcast
    let msg = alice.recv_timeout(Duration::from_millis(100)).await;
    assert!(msg.is_some());

    match msg.unwrap() {
        ChatEvent::Chat(ChatMessage::System(text)) => {
            assert!(text.contains("Alice"));
            assert!(text.contains("joined"));
        }
        _ => panic!("Expected system message"),
    }
}

#[tokio::test]
async fn test_chat_message_broadcast() {
    let mut harness = ChatTestHarness::new();

    // Start server
    let config = create_server_config("Chat Room");
    let mut server_state = <ChatActor as Actor<ServerSinkType>>::startup(config);
    let mut server_extras = create_server_extras();

    // Connect two users
    let alice = harness.connect_user("Alice");
    let bob = harness.connect_user("Bob");

    // Deliver connections
    harness
        .deliver_user_connected::<ChatActor>(&alice.user, &mut server_state, &mut server_extras)
        .await;
    harness
        .deliver_user_connected::<ChatActor>(&bob.user, &mut server_state, &mut server_extras)
        .await;

    // Clear initial join messages
    while alice.try_recv().await.is_some() {}
    while bob.try_recv().await.is_some() {}

    // Alice sends a chat message
    alice
        .send(ChatEvent::Chat(ChatMessage::SendText {
            content: "Hello everyone!".to_string(),
        }))
        .await;

    // Server processes
    harness
        .run_server_tick::<ChatActor>(&mut server_state, &mut server_extras)
        .await;

    // Both users receive the broadcast
    let alice_msg = alice.recv_timeout(Duration::from_millis(100)).await;
    let bob_msg = bob.recv_timeout(Duration::from_millis(100)).await;

    assert!(alice_msg.is_some());
    assert!(bob_msg.is_some());

    match alice_msg.unwrap() {
        ChatEvent::Chat(ChatMessage::TextSent { username, content }) => {
            assert_eq!(username, "Alice");
            assert_eq!(content, "Hello everyone!");
        }
        _ => panic!("Expected TextSent message"),
    }

    match bob_msg.unwrap() {
        ChatEvent::Chat(ChatMessage::TextSent { username, content }) => {
            assert_eq!(username, "Alice");
            assert_eq!(content, "Hello everyone!");
        }
        _ => panic!("Expected TextSent message"),
    }
}

#[tokio::test]
async fn test_user_disconnect_broadcasts() {
    let mut harness = ChatTestHarness::new();

    // Start server
    let config = create_server_config("Test Room");
    let mut server_state = <ChatActor as Actor<ServerSinkType>>::startup(config);
    let mut server_extras = create_server_extras();

    // Connect two users
    let alice = harness.connect_user("Alice");
    let bob = harness.connect_user("Bob");

    // Deliver connections
    harness
        .deliver_user_connected::<ChatActor>(&alice.user, &mut server_state, &mut server_extras)
        .await;
    harness
        .deliver_user_connected::<ChatActor>(&bob.user, &mut server_state, &mut server_extras)
        .await;

    // Clear initial messages
    while alice.try_recv().await.is_some() {}
    while bob.try_recv().await.is_some() {}

    // Alice disconnects
    harness
        .deliver_user_disconnected::<ChatActor>(&alice.user, &mut server_state, &mut server_extras)
        .await;

    // Bob receives disconnect notification
    let msg = bob.recv_timeout(Duration::from_millis(100)).await;
    assert!(msg.is_some());

    match msg.unwrap() {
        ChatEvent::Chat(ChatMessage::System(text)) => {
            assert!(text.contains("Alice"));
            assert!(text.contains("left"));
        }
        _ => panic!("Expected system message"),
    }
}

#[tokio::test]
async fn test_client_actor_receives_messages() {
    let mut harness = ChatTestHarness::new();

    // Start server
    let config = create_server_config("Client Test");
    let mut server_state = <ChatActor as Actor<ServerSinkType>>::startup(config);
    let mut server_extras = create_server_extras();

    // Connect user
    let alice = harness.connect_user("Alice");

    // Create client actor with test adapter
    let (mut client_state, mut client_extras, collected) = create_client();

    // Deliver connected to client
    alice.deliver_connected::<ClientActor>(&mut client_state, &mut client_extras);
    assert!(collected.lock().unwrap().connected);

    // Server sends join message
    harness
        .deliver_user_connected::<ChatActor>(&alice.user, &mut server_state, &mut server_extras)
        .await;

    // Client processes incoming
    alice
        .run_client_tick::<ClientActor>(&mut client_state, &mut client_extras)
        .await;

    // Verify client received the system message
    let locked = collected.lock().unwrap();
    assert_eq!(locked.system_messages.len(), 1);
    assert!(locked.system_messages[0].contains("Alice"));
}

#[tokio::test]
async fn test_full_chat_flow_with_actual_actors() {
    let mut harness = ChatTestHarness::new();

    // Start server
    let config = create_server_config("Full Flow Test");
    let mut server_state = <ChatActor as Actor<ServerSinkType>>::startup(config);
    let mut server_extras = create_server_extras();

    // Connect Alice with actual client actor
    let alice_conn = harness.connect_user("Alice");
    let (mut alice_state, mut alice_extras, alice_collected) = create_client();

    // Connect Bob with actual client actor
    let bob_conn = harness.connect_user("Bob");
    let (mut bob_state, mut bob_extras, bob_collected) = create_client();

    // Deliver connected events to clients
    alice_conn.deliver_connected::<ClientActor>(&mut alice_state, &mut alice_extras);
    bob_conn.deliver_connected::<ClientActor>(&mut bob_state, &mut bob_extras);

    // Deliver user connections to server
    harness
        .deliver_user_connected::<ChatActor>(&alice_conn.user, &mut server_state, &mut server_extras)
        .await;
    harness
        .deliver_user_connected::<ChatActor>(&bob_conn.user, &mut server_state, &mut server_extras)
        .await;

    // Process initial messages on clients
    alice_conn
        .run_client_tick::<ClientActor>(&mut alice_state, &mut alice_extras)
        .await;
    bob_conn
        .run_client_tick::<ClientActor>(&mut bob_state, &mut bob_extras)
        .await;

    // Clear collected messages for the actual test
    alice_collected.lock().unwrap().system_messages.clear();
    alice_collected.lock().unwrap().text_messages.clear();
    bob_collected.lock().unwrap().system_messages.clear();
    bob_collected.lock().unwrap().text_messages.clear();

    // Alice sends a chat message
    alice_conn
        .send(ChatEvent::Chat(ChatMessage::SendText {
            content: "Hello Bob!".to_string(),
        }))
        .await;

    // Server processes
    harness
        .run_server_tick::<ChatActor>(&mut server_state, &mut server_extras)
        .await;

    // Both clients process
    alice_conn
        .run_client_tick::<ClientActor>(&mut alice_state, &mut alice_extras)
        .await;
    bob_conn
        .run_client_tick::<ClientActor>(&mut bob_state, &mut bob_extras)
        .await;

    // Verify both received the message via the actual client actor
    {
        let alice_msgs = alice_collected.lock().unwrap();
        assert_eq!(alice_msgs.text_messages.len(), 1);
        assert_eq!(alice_msgs.text_messages[0].0, "Alice");
        assert_eq!(alice_msgs.text_messages[0].1, "Hello Bob!");
    }

    {
        let bob_msgs = bob_collected.lock().unwrap();
        assert_eq!(bob_msgs.text_messages.len(), 1);
        assert_eq!(bob_msgs.text_messages[0].0, "Alice");
        assert_eq!(bob_msgs.text_messages[0].1, "Hello Bob!");
    }
}

#[tokio::test]
async fn test_client_disconnect_callback() {
    let mut harness = ChatTestHarness::new();

    // Connect user
    let alice = harness.connect_user("Alice");

    // Create client actor
    let (mut client_state, mut client_extras, collected) = create_client();

    // Connect
    alice.deliver_connected::<ClientActor>(&mut client_state, &mut client_extras);
    assert!(collected.lock().unwrap().connected);
    assert!(!collected.lock().unwrap().disconnected);

    // Disconnect
    alice.deliver_disconnected::<ClientActor>(&mut client_state, &mut client_extras);
    assert!(collected.lock().unwrap().disconnected);
}
