//! Simple chat server using multiplayer-kit with typed actors.
//!
//! Run with: cargo run --bin chat-server

use chat_protocol::{ChatChannel, ChatEvent, ChatMessage, ChatProtocol, ChatRoomConfig, ChatUser};
use multiplayer_kit_helpers::{TypedContext, TypedEvent, with_typed_actor};
use multiplayer_kit_protocol::RejectReason;
use multiplayer_kit_server::{AuthRequest, GameServerContext, Server};
use serde::Deserialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_USER_ID: AtomicU64 = AtomicU64::new(1);

// ============================================================================
// Game Context - demonstrates how games can inject their own services
// ============================================================================

/// Example game context for the chat server.
///
/// In a real game, this might contain:
/// - Database connection pools
/// - History/replay managers
/// - Metrics recorders
/// - External service clients
struct ChatServerContext {
    /// Server name for welcome messages
    server_name: String,
    /// Total messages counter (just for demo)
    message_count: AtomicU64,
}

impl GameServerContext for ChatServerContext {}

impl ChatServerContext {
    fn new(server_name: impl Into<String>) -> Self {
        Self {
            server_name: server_name.into(),
            message_count: AtomicU64::new(0),
        }
    }

    fn increment_message_count(&self) -> u64 {
        self.message_count.fetch_add(1, Ordering::Relaxed)
    }

    fn server_name(&self) -> &str {
        &self.server_name
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    println!("Starting chat server...");
    println!("  HTTP/WS: http://127.0.0.1:8080 (TCP)");
    println!("  QUIC:    https://127.0.0.1:8080 (UDP)");
    println!("  (Same port, different protocols)");
    println!();
    println!("Endpoints:");
    println!("  POST /ticket         - Get auth ticket (body: {{\"username\": \"name\"}})");
    println!("  POST /rooms          - Create room (body: {{\"name\": \"Room Name\"}})");
    println!("  GET  /rooms          - List rooms");
    println!("  DELETE /rooms/{{id}}   - Delete room");
    println!("  POST /quickplay      - Quick join or create room");
    println!("  GET  /cert-hash      - Get cert hash for WebTransport");
    println!();

    // Create the game context with any services your game needs
    let game_context = ChatServerContext::new("Multiplayer-Kit Chat Demo");

    let server = Server::<ChatUser, ChatRoomConfig, ChatServerContext>::builder()
        .http_addr("127.0.0.1:8080")
        .quic_addr("127.0.0.1:8080")
        .jwt_secret(b"super-secret-key-for-dev-only")
        // Allow dev server origins (npx serve uses port 3000)
        .cors_origins(vec![
            "http://localhost:3000".to_string(),
            "http://127.0.0.1:3000".to_string(),
            "http://localhost:8080".to_string(),
            "http://127.0.0.1:8080".to_string(),
        ])
        // Set the game context - available in auth and room handlers
        .context(game_context)
        // Auth handler now receives the game context
        .auth_handler(|req: AuthRequest, ctx: Arc<ChatServerContext>| async move {
            let body = req
                .body
                .ok_or(RejectReason::Custom("Missing body".into()))?;

            #[derive(Deserialize)]
            struct AuthBody {
                username: String,
            }

            let auth: AuthBody = serde_json::from_slice(&body)
                .map_err(|e| RejectReason::Custom(format!("Invalid JSON: {}", e)))?;

            if auth.username.is_empty() {
                return Err(RejectReason::Custom("Username cannot be empty".into()));
            }

            if auth.username.len() > 32 {
                return Err(RejectReason::Custom(
                    "Username too long (max 32 chars)".into(),
                ));
            }

            let user = ChatUser {
                id: NEXT_USER_ID.fetch_add(1, Ordering::SeqCst),
                username: auth.username,
            };

            tracing::info!(
                "[{}] Issued ticket for user: {} (id={})",
                ctx.server_name(),
                user.username,
                user.id
            );
            Ok(user)
        })
        // Typed actor - handles events with automatic framing and serialization
        // Now includes game context generic parameter
        .room_handler(with_typed_actor::<
            ChatUser,
            ChatProtocol,
            ChatRoomConfig,
            ChatServerContext,
            _,
            _,
        >(chat_actor))
        .build()
        .expect("Failed to build server");

    server.run().await?;

    Ok(())
}

/// Chat room actor - receives typed events, one at a time.
///
/// Now has access to the game context via `ctx.game_context()`.
async fn chat_actor(
    ctx: TypedContext<ChatUser, ChatProtocol, ChatServerContext>,
    event: TypedEvent<ChatUser, ChatProtocol>,
    config: Arc<ChatRoomConfig>,
) {
    let room_id = ctx.room_id();
    let game_ctx = ctx.game_context();

    match event {
        TypedEvent::UserConnected(user) => {
            tracing::info!(
                "[{}] [Room {:?} '{}'] {} connected",
                game_ctx.server_name(),
                room_id,
                config.name,
                user.username
            );

            // Broadcast join message
            let msg = ChatEvent::Chat(ChatMessage::System(format!(
                "*** {} joined '{}' ***",
                user.username, config.name
            )));
            ctx.broadcast(&msg).await;
        }

        TypedEvent::UserDisconnected(user) => {
            tracing::info!(
                "[{}] [Room {:?} '{}'] {} disconnected",
                game_ctx.server_name(),
                room_id,
                config.name,
                user.username
            );

            // Broadcast leave message
            let msg = ChatEvent::Chat(ChatMessage::System(format!(
                "*** {} left the chat ***",
                user.username
            )));
            ctx.broadcast(&msg).await;
        }

        TypedEvent::Message {
            sender,
            channel: ChatChannel::Chat,
            event: ChatEvent::Chat(ChatMessage::Text { content, .. }),
            ..
        } => {
            // Use game context to track message count
            let count = game_ctx.increment_message_count();
            tracing::info!(
                "[{}] [Room {:?}] Message #{}: {}: {}",
                game_ctx.server_name(),
                room_id,
                count + 1,
                sender.username,
                content
            );

            // Broadcast to all (include sender - they'll see their username)
            let msg = ChatEvent::Chat(ChatMessage::Text {
                username: sender.username,
                content,
            });
            ctx.broadcast(&msg).await;
        }

        TypedEvent::Message { .. } => {
            // Ignore other message types (system messages from clients, etc.)
        }

        TypedEvent::Internal(_) => {
            // Handle self-sent events (e.g., timers, scheduled tasks)
        }

        TypedEvent::Shutdown => {
            tracing::info!(
                "[{}] [Room {:?} '{}'] Shutting down",
                game_ctx.server_name(),
                room_id,
                config.name
            );
        }
    }
}
