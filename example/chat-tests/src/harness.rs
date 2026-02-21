//! Test harness for chat server and client communication.
//!
//! Routes messages between server and client actors using in-memory channels,
//! simulating the network layer without actual connections.

use chat_protocol::{ChatEvent, ChatUser};
use multiplayer_kit_helpers::{
    actor::{Actor, ActorProtocol, AddressedMessage},
    client_actor::{ClientMessage, ClientOutput, ClientSink, ClientSource},
    server_actor::{ServerMessage, ServerOutput, ServerSink, ServerSource},
};
use multiplayer_kit_protocol::UserContext;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Test harness for chat actors.
pub struct ChatTestHarness {
    server_to_client: HashMap<u64, mpsc::Sender<ChatEvent>>,
    client_to_server: mpsc::Sender<(ChatUser, ChatEvent)>,
    client_to_server_rx: Arc<Mutex<mpsc::Receiver<(ChatUser, ChatEvent)>>>,
    server_internal_tx: mpsc::Sender<ChatEvent>,
    server_internal_rx: Arc<Mutex<mpsc::Receiver<ChatEvent>>>,
    users: Vec<ChatUser>,
    next_user_id: u64,
}

impl ChatTestHarness {
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

    /// Connect a new user and return their client connection.
    pub fn connect_user(&mut self, username: &str) -> ChatClientConnection {
        let id = self.next_user_id;
        self.next_user_id += 1;

        let user = ChatUser {
            id,
            username: username.to_string(),
        };

        let (tx, rx) = mpsc::channel(64);
        self.server_to_client.insert(id, tx);
        self.users.push(user.clone());

        ChatClientConnection {
            user,
            from_server: Arc::new(Mutex::new(rx)),
            to_server: self.client_to_server.clone(),
        }
    }

    /// Run a server tick - process all pending messages.
    pub async fn run_server_tick<A>(&self, state: &mut A::State, extras: &mut A::Extras)
    where
        A: Actor<ServerSink<ChatUser, ChatEvent>>
            + ActorProtocol<ActorId = ServerSource<ChatUser>, Message = ServerMessage<ChatEvent>>,
    {
        let mut sink = ServerSink::<ChatUser, ChatEvent>::new();

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

        self.route_server_outputs(&mut sink).await;
    }

    /// Deliver user connected event to server.
    pub async fn deliver_user_connected<A>(
        &self,
        user: &ChatUser,
        state: &mut A::State,
        extras: &mut A::Extras,
    ) where
        A: Actor<ServerSink<ChatUser, ChatEvent>>
            + ActorProtocol<ActorId = ServerSource<ChatUser>, Message = ServerMessage<ChatEvent>>,
    {
        let mut sink = ServerSink::<ChatUser, ChatEvent>::new();

        let msg = AddressedMessage {
            from: ServerSource::User(user.clone()),
            content: ServerMessage::UserConnected,
        };
        A::handle(state, extras, msg, &mut sink);

        self.route_server_outputs(&mut sink).await;
    }

    /// Deliver user disconnected event to server.
    pub async fn deliver_user_disconnected<A>(
        &self,
        user: &ChatUser,
        state: &mut A::State,
        extras: &mut A::Extras,
    ) where
        A: Actor<ServerSink<ChatUser, ChatEvent>>
            + ActorProtocol<ActorId = ServerSource<ChatUser>, Message = ServerMessage<ChatEvent>>,
    {
        let mut sink = ServerSink::<ChatUser, ChatEvent>::new();

        let msg = AddressedMessage {
            from: ServerSource::User(user.clone()),
            content: ServerMessage::UserDisconnected,
        };
        A::handle(state, extras, msg, &mut sink);

        self.route_server_outputs(&mut sink).await;
    }

    async fn route_server_outputs(&self, sink: &mut ServerSink<ChatUser, ChatEvent>) {
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

impl Default for ChatTestHarness {
    fn default() -> Self {
        Self::new()
    }
}

/// Client connection for testing.
pub struct ChatClientConnection {
    pub user: ChatUser,
    from_server: Arc<Mutex<mpsc::Receiver<ChatEvent>>>,
    to_server: mpsc::Sender<(ChatUser, ChatEvent)>,
}

impl ChatClientConnection {
    /// Send a message to the server.
    pub async fn send(&self, message: ChatEvent) {
        let _ = self.to_server.send((self.user.clone(), message)).await;
    }

    /// Receive a message (non-blocking).
    pub async fn try_recv(&self) -> Option<ChatEvent> {
        let mut rx = self.from_server.lock().await;
        rx.try_recv().ok()
    }

    /// Receive a message with timeout.
    pub async fn recv_timeout(&self, timeout: std::time::Duration) -> Option<ChatEvent> {
        let rx = self.from_server.clone();
        tokio::time::timeout(timeout, async move {
            let mut guard = rx.lock().await;
            guard.recv().await
        })
        .await
        .ok()
        .flatten()
    }

    /// Run a client actor tick - process incoming messages and return outgoing.
    pub async fn run_client_tick<A>(
        &self,
        state: &mut A::State,
        extras: &mut A::Extras,
    ) -> Vec<ChatEvent>
    where
        A: Actor<ClientSink<ChatEvent>>
            + ActorProtocol<ActorId = ClientSource, Message = ClientMessage<ChatEvent>>,
    {
        let mut sink = ClientSink::<ChatEvent>::new();
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
                ClientOutput::Internal { .. } => {}
            }
        }

        // Send to server
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
    ) -> Vec<ChatEvent>
    where
        A: Actor<ClientSink<ChatEvent>>
            + ActorProtocol<ActorId = ClientSource, Message = ClientMessage<ChatEvent>>,
    {
        let mut sink = ClientSink::<ChatEvent>::new();

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

    /// Deliver disconnected event to client actor.
    pub fn deliver_disconnected<A>(
        &self,
        state: &mut A::State,
        extras: &mut A::Extras,
    ) where
        A: Actor<ClientSink<ChatEvent>>
            + ActorProtocol<ActorId = ClientSource, Message = ClientMessage<ChatEvent>>,
    {
        let mut sink = ClientSink::<ChatEvent>::new();

        let msg = AddressedMessage {
            from: ClientSource::Server,
            content: ClientMessage::Disconnected,
        };
        A::handle(state, extras, msg, &mut sink);
    }
}
