//! Core chat actor logic - ONE implementation, NO platform-specific code.
//!
//! This module is 100% platform-agnostic. All platform abstraction
//! is handled by the helpers library.

use crate::{ChatClientAdapter, ChatError, ChatEvent, ChatMessage, ChatProtocol};
use multiplayer_kit_helpers::{
    run_typed_client_actor, ActorHandle, ActorSendError, ClientConnection, GameClientContext,
    Spawner, TypedClientContext, TypedClientEvent,
};

// Platform spawner
#[cfg(not(target_arch = "wasm32"))]
use multiplayer_kit_helpers::TokioSpawner as PlatformSpawner;
#[cfg(target_arch = "wasm32")]
use multiplayer_kit_helpers::WasmSpawner as PlatformSpawner;

// ============================================================================
// ChatClientContext - game context for the chat actor
// ============================================================================

/// Context for the chat client actor.
///
/// Wraps the adapter and provides extensibility for future additions.
pub struct ChatClientContext<A: ChatClientAdapter> {
    adapter: A,
}

impl<A: ChatClientAdapter> ChatClientContext<A> {
    /// Create a new chat client context.
    pub fn new(adapter: A) -> Self {
        Self { adapter }
    }

    /// Get the adapter reference.
    pub fn adapter(&self) -> &A {
        &self.adapter
    }
}

impl<A: ChatClientAdapter> GameClientContext for ChatClientContext<A> {}

// ============================================================================
// ChatHandle - returned from start_chat_actor
// ============================================================================

/// Handle to a running chat actor.
///
/// Use this to send messages to the chat room.
pub struct ChatHandle {
    inner: ActorHandle<ChatProtocol>,
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
    A: ChatClientAdapter,
{
    let chat_context = ChatClientContext::new(adapter);

    let handle = run_typed_client_actor::<ChatProtocol, ChatClientContext<A>, _, _>(
        conn.into(),
        chat_context,
        handle_event::<A, PlatformSpawner>,
        PlatformSpawner,
    );

    ChatHandle { inner: handle }
}

// ============================================================================
// Event handling
// ============================================================================

/// Handle actor events.
///
/// This is a sync function - actors process messages synchronously.
fn handle_event<A: ChatClientAdapter, S: Spawner>(
    ctx: &TypedClientContext<ChatProtocol, ChatClientContext<A>>,
    event: TypedClientEvent<ChatProtocol>,
    _spawner: &S, // Available if actor needs to spawn background tasks
) {
    let chat_ctx: &ChatClientContext<A> = &**ctx.game_context();
    let adapter = chat_ctx.adapter();

    match event {
        TypedClientEvent::Connected => {
            adapter.on_connected();
        }
        TypedClientEvent::Message(chat_event) => {
            handle_chat_event(adapter, chat_event);
        }
        TypedClientEvent::Internal(_) => {}
        TypedClientEvent::Disconnected => {
            adapter.on_disconnected();
        }
    }
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
