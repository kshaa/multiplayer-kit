//! Core chat actor logic - ONE implementation, NO platform-specific code.
//!
//! This module is 100% platform-agnostic. All platform abstraction
//! is handled by the helpers library.

use crate::{ChatClientAdapter, ChatError, ChatEvent, ChatMessage};
use multiplayer_kit_helpers::{
    actor::{Actor, ActorProtocol, AddressedMessage, Sink},
    game_actor::SelfTarget,
    with_client_actor, ClientActorHandle, ActorSendError, ClientConnection, GameClientContext,
    ClientMessage, ClientSource, ServerTarget,
};

// Platform spawner
#[cfg(not(target_arch = "wasm32"))]
use multiplayer_kit_helpers::TokioSpawner as PlatformSpawner;
#[cfg(target_arch = "wasm32")]
use multiplayer_kit_helpers::WasmSpawner as PlatformSpawner;

use std::marker::PhantomData;

// ============================================================================
// ChatClientContext - implements GameClientContext
// ============================================================================

/// Context for the chat client actor.
///
/// Wraps the adapter and provides it as extras to the actor.
pub struct ChatClientContext<A: ChatClientAdapter> {
    adapter: A,
}

impl<A: ChatClientAdapter> ChatClientContext<A> {
    /// Create a new chat client context.
    pub fn new(adapter: A) -> Self {
        Self { adapter }
    }
}

impl<A: ChatClientAdapter> GameClientContext for ChatClientContext<A> {
    type Extras = A;

    fn get_extras(&self) -> Self::Extras {
        self.adapter.clone()
    }
}

// ============================================================================
// ChatClientActor - Actor trait implementation
// ============================================================================

/// Chat client actor.
pub struct ChatClientActor<A: ChatClientAdapter>(PhantomData<A>);

/// Actor state (nothing needed for simple chat).
pub struct ChatState;

impl<A: ChatClientAdapter> From<ChatClientContext<A>> for ChatState {
    fn from(_ctx: ChatClientContext<A>) -> Self {
        ChatState
    }
}

impl<A: ChatClientAdapter> From<A> for ChatState {
    fn from(_adapter: A) -> Self {
        ChatState
    }
}

impl<A: ChatClientAdapter> ActorProtocol for ChatClientActor<A> {
    type ActorId = ClientSource;
    type Message = ClientMessage<ChatEvent>;
}

impl<A, S> Actor<S> for ChatClientActor<A>
where
    A: ChatClientAdapter,
    S: Sink<ServerTarget<ChatEvent>> + Sink<SelfTarget<ChatEvent>>,
{
    type Config = ChatClientContext<A>;
    type State = ChatState;
    type Extras = A;

    fn startup(_config: Self::Config) -> Self::State {
        ChatState
    }

    fn handle(
        _state: &mut Self::State,
        adapter: &mut Self::Extras,
        message: AddressedMessage<Self::ActorId, Self::Message>,
        _sink: &mut S,
    ) {
        match (&message.from, &message.content) {
            (ClientSource::Server, ClientMessage::Connected) => {
                adapter.on_connected();
            }
            (ClientSource::Server, ClientMessage::Message(chat_event)) => {
                handle_chat_event(adapter, chat_event.clone());
            }
            (ClientSource::Server, ClientMessage::Disconnected) => {
                adapter.on_disconnected();
            }
            (ClientSource::Internal, ClientMessage::Message(_)) => {
                // Internal messages not used in chat
            }
            _ => {}
        }
    }

    fn shutdown(_state: &mut Self::State, _extras: &mut Self::Extras) {}
}

fn handle_chat_event<A: ChatClientAdapter + ?Sized>(adapter: &A, event: ChatEvent) {
    match event {
        ChatEvent::Chat(msg) => match msg {
            ChatMessage::TextSent { username, content } => {
                adapter.on_text_message(&username, &content);
            }
            ChatMessage::System(text) => {
                adapter.on_system_message(&text);
            }
            ChatMessage::SendText { .. } => {
                // Client shouldn't receive SendText from server, ignore
            }
        },
    }
}

// ============================================================================
// ChatHandle - returned from start_chat_actor
// ============================================================================

/// Handle to a running chat actor.
///
/// Use this to send messages to the chat room.
pub struct ChatHandle {
    inner: ClientActorHandle<ChatEvent>,
}

impl ChatHandle {
    /// Send a text message to the chat room.
    pub fn send_text(&self, content: &str) -> Result<(), ChatError> {
        let event = ChatEvent::Chat(ChatMessage::SendText {
            content: content.to_string(),
        });
        self.inner.sender.send(event).map_err(|e| match e {
            ActorSendError::ChannelClosed => ChatError::ChannelClosed,
            ActorSendError::NotConnected => ChatError::NotConnected,
        })
    }
}

impl Clone for ChatHandle {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

// ============================================================================
// start_chat_actor - returns handle directly
// ============================================================================

/// Start the chat actor and return a handle.
///
/// The handle can be used to send messages to the chat room.
///
/// # Example (Native)
///
/// ```ignore
/// let handle = start_chat_actor(conn, adapter);
/// handle.send_text("Hello!")?;
/// ```
///
/// # Example (WASM)
///
/// ```ignore
/// let handle = start_chat_actor(Rc::new(conn), adapter);
/// handle.send_text("Hello!")?;
/// ```
pub fn start_chat_actor<Conn, A>(conn: Conn, adapter: A) -> ChatHandle
where
    Conn: ClientConnection + 'static,
    A: ChatClientAdapter + Clone,
{
    let ctx = ChatClientContext::new(adapter);

    let handle = with_client_actor::<
        ChatClientActor<A>,
        ChatEvent,
        ChatClientContext<A>,
        PlatformSpawner,
    >(conn, ctx, PlatformSpawner);

    ChatHandle { inner: handle }
}
