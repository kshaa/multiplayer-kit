//! Test harness for running server and client actors together.
//!
//! Mimics the production glue code behavior but uses mock transports.

use crate::actor::{Actor, ActorProtocol, AddressedMessage};
use crate::client_actor::{ClientMessage, ClientOutput, ClientSink, ClientSource};
use crate::server_actor::{ServerMessage, ServerOutput, ServerSink, ServerSource};
use crate::utils::ChannelMessage;
use multiplayer_kit_protocol::UserContext;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// User ID type for tests.
pub type TestUserId = u64;

/// Simple user context for testing.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TestUser {
    pub id: TestUserId,
    pub name: String,
}

impl fmt::Display for TestUser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({})", self.name, self.id)
    }
}

impl UserContext for TestUser {
    type Id = TestUserId;
    fn id(&self) -> TestUserId {
        self.id
    }
}

/// Test harness that runs a server actor and multiple client actors.
///
/// Messages between actors are routed through channels, simulating
/// the real network layer behavior.
pub struct TestHarness<M: ChannelMessage + Clone> {
    /// Server -> Client messages (per user)
    server_to_client: HashMap<TestUserId, mpsc::Sender<M>>,
    /// Client -> Server messages
    client_to_server: mpsc::Sender<(TestUser, M)>,
    client_to_server_rx: Arc<Mutex<mpsc::Receiver<(TestUser, M)>>>,
    /// Server internal messages
    server_internal_tx: mpsc::Sender<M>,
    server_internal_rx: Arc<Mutex<mpsc::Receiver<M>>>,
    /// Connected users
    users: Vec<TestUser>,
    /// Next user ID
    next_user_id: TestUserId,
}

impl<M: ChannelMessage + Clone> TestHarness<M> {
    pub fn new() -> Self {
        let (client_to_server_tx, client_to_server_rx) = mpsc::channel(64);
        let (server_internal_tx, server_internal_rx) = mpsc::channel(64);

        Self {
            server_to_client: HashMap::new(),
            client_to_server: client_to_server_tx,
            client_to_server_rx: Arc::new(Mutex::new(client_to_server_rx)),
            server_internal_tx,
            server_internal_rx: Arc::new(Mutex::new(server_internal_rx)),
            users: Vec::new(),
            next_user_id: 1,
        }
    }

    /// Add a new user and return their connection.
    pub fn connect_user(&mut self, name: &str) -> TestClientConnection<M> {
        let id = self.next_user_id;
        self.next_user_id += 1;

        let user = TestUser {
            id,
            name: name.to_string(),
        };

        let (tx, rx) = mpsc::channel(64);
        self.server_to_client.insert(id, tx);
        self.users.push(user.clone());

        TestClientConnection {
            user,
            from_server: Arc::new(Mutex::new(rx)),
            to_server: self.client_to_server.clone(),
        }
    }

    /// Get all connected users.
    pub fn users(&self) -> &[TestUser] {
        &self.users
    }

    /// Run the server actor, processing all pending events.
    ///
    /// This is the core routing logic that mimics `run_server_actor`.
    pub async fn run_server_tick<A, C>(&self, state: &mut A::State, extras: &mut A::Extras)
    where
        A: Actor<ServerSink<TestUser, M>>
            + ActorProtocol<ActorId = ServerSource<TestUser>, Message = ServerMessage<M>>,
    {
        let mut sink = ServerSink::<TestUser, M>::new();

        // Process client -> server messages
        {
            let mut rx = self.client_to_server_rx.lock().await;
            while let Ok((user, message)) = rx.try_recv() {
                let msg = AddressedMessage {
                    from: ServerSource::User(user),
                    content: ServerMessage::Message(message),
                };
                A::handle(state, extras, msg, &mut sink);
            }
        }

        // Process internal messages
        {
            let mut rx = self.server_internal_rx.lock().await;
            while let Ok(message) = rx.try_recv() {
                let msg = AddressedMessage {
                    from: ServerSource::Internal,
                    content: ServerMessage::Message(message),
                };
                A::handle(state, extras, msg, &mut sink);
            }
        }

        // Route outputs
        self.route_server_outputs(&mut sink).await;
    }

    /// Deliver a user connected event to the server actor.
    pub async fn deliver_user_connected<A, C>(
        &self,
        user: &TestUser,
        state: &mut A::State,
        extras: &mut A::Extras,
    ) where
        A: Actor<ServerSink<TestUser, M>>
            + ActorProtocol<ActorId = ServerSource<TestUser>, Message = ServerMessage<M>>,
    {
        let mut sink = ServerSink::<TestUser, M>::new();

        let msg = AddressedMessage {
            from: ServerSource::User(user.clone()),
            content: ServerMessage::UserConnected,
        };
        A::handle(state, extras, msg, &mut sink);

        self.route_server_outputs(&mut sink).await;
    }

    /// Deliver a user disconnected event to the server actor.
    pub async fn deliver_user_disconnected<A, C>(
        &self,
        user: &TestUser,
        state: &mut A::State,
        extras: &mut A::Extras,
    ) where
        A: Actor<ServerSink<TestUser, M>>
            + ActorProtocol<ActorId = ServerSource<TestUser>, Message = ServerMessage<M>>,
    {
        let mut sink = ServerSink::<TestUser, M>::new();

        let msg = AddressedMessage {
            from: ServerSource::User(user.clone()),
            content: ServerMessage::UserDisconnected,
        };
        A::handle(state, extras, msg, &mut sink);

        self.route_server_outputs(&mut sink).await;
    }

    async fn route_server_outputs(&self, sink: &mut ServerSink<TestUser, M>) {
        for output in sink.drain() {
            match output {
                ServerOutput::ToUser { user, message } => {
                    if let Some(tx) = self.server_to_client.get(&user.id()) {
                        let _ = tx.send(message).await;
                    }
                }
                ServerOutput::Broadcast { message } => {
                    for tx in self.server_to_client.values() {
                        let _ = tx.send(message.clone()).await;
                    }
                }
                ServerOutput::Internal { message } => {
                    let _ = self.server_internal_tx.send(message).await;
                }
            }
        }
    }
}

impl<M: ChannelMessage + Clone> Default for TestHarness<M> {
    fn default() -> Self {
        Self::new()
    }
}

/// Client connection for testing.
pub struct TestClientConnection<M: ChannelMessage> {
    pub user: TestUser,
    from_server: Arc<Mutex<mpsc::Receiver<M>>>,
    to_server: mpsc::Sender<(TestUser, M)>,
}

impl<M: ChannelMessage> TestClientConnection<M> {
    /// Send a message to the server.
    pub async fn send(&self, message: M) {
        let _ = self.to_server.send((self.user.clone(), message)).await;
    }

    /// Receive a message from the server (non-blocking).
    pub async fn try_recv(&self) -> Option<M> {
        let mut rx = self.from_server.lock().await;
        rx.try_recv().ok()
    }

    /// Receive a message from the server (blocking with timeout).
    pub async fn recv_timeout(&self, timeout: std::time::Duration) -> Option<M> {
        let rx = self.from_server.clone();
        tokio::time::timeout(timeout, async move {
            let mut guard = rx.lock().await;
            guard.recv().await
        })
        .await
        .ok()
        .flatten()
    }

    /// Run a client actor tick, processing all pending events.
    pub async fn run_client_tick<A>(
        &self,
        state: &mut A::State,
        extras: &mut A::Extras,
    ) -> Vec<M>
    where
        A: Actor<ClientSink<M>>
            + ActorProtocol<ActorId = ClientSource, Message = ClientMessage<M>>,
        M: Clone,
    {
        let mut sink = ClientSink::<M>::new();
        let mut to_server = Vec::new();

        // Process server -> client messages
        {
            let mut rx = self.from_server.lock().await;
            while let Ok(message) = rx.try_recv() {
                let msg = AddressedMessage {
                    from: ClientSource::Server,
                    content: ClientMessage::Message(message),
                };
                A::handle(state, extras, msg, &mut sink);
            }
        }

        // Collect outputs
        for output in sink.drain() {
            match output {
                ClientOutput::ToServer { message } => {
                    to_server.push(message);
                }
                ClientOutput::Internal { message: _ } => {
                    // Internal messages would need additional handling
                }
            }
        }

        // Send messages to server
        for msg in &to_server {
            let _ = self.to_server.send((self.user.clone(), msg.clone())).await;
        }

        to_server
    }

    /// Deliver connected event to client actor.
    pub fn deliver_connected<A>(
        &self,
        state: &mut A::State,
        extras: &mut A::Extras,
    ) -> Vec<M>
    where
        A: Actor<ClientSink<M>>
            + ActorProtocol<ActorId = ClientSource, Message = ClientMessage<M>>,
    {
        let mut sink = ClientSink::<M>::new();

        let msg = AddressedMessage {
            from: ClientSource::Server,
            content: ClientMessage::Connected,
        };
        A::handle(state, extras, msg, &mut sink);

        let mut to_server = Vec::new();
        for output in sink.drain() {
            if let ClientOutput::ToServer { message } = output {
                to_server.push(message);
            }
        }
        to_server
    }
}
