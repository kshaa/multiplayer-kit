//! Simple chat server using multiplayer-kit with typed actors.
//!
//! Run with: cargo run --bin chat-server

use chat_protocol::{ChatChannel, ChatEvent, ChatMessage, ChatProtocol, ChatUser};
use multiplayer_kit_helpers::{with_typed_actor, TypedContext, TypedEvent};
use multiplayer_kit_protocol::RejectReason;
use multiplayer_kit_server::{AuthRequest, Server};
use serde::Deserialize;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_USER_ID: AtomicU64 = AtomicU64::new(1);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    println!("Starting chat server...");
    println!("  HTTP: http://127.0.0.1:8080");
    println!("  QUIC: https://127.0.0.1:4433");
    println!("  WS:   ws://127.0.0.1:8080/ws/room/{{id}}?ticket=...");
    println!();
    println!("Endpoints:");
    println!("  POST /ticket         - Get auth ticket (body: {{\"username\": \"name\"}})");
    println!("  POST /rooms          - Create room");
    println!("  GET  /rooms          - List rooms");
    println!("  DELETE /rooms/{{id}}   - Delete room");
    println!("  GET  /cert-hash      - Get cert hash for WebTransport");
    println!();

    let server = Server::<ChatUser>::builder()
        .http_addr("127.0.0.1:8080")
        .quic_addr("127.0.0.1:4433")
        .jwt_secret(b"super-secret-key-for-dev-only")
        .auth_handler(|req: AuthRequest| async move {
            let body = req.body.ok_or(RejectReason::Custom("Missing body".into()))?;

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
                return Err(RejectReason::Custom("Username too long (max 32 chars)".into()));
            }

            let user = ChatUser {
                id: NEXT_USER_ID.fetch_add(1, Ordering::SeqCst),
                username: auth.username,
            };

            tracing::info!("Issued ticket for user: {} (id={})", user.username, user.id);
            Ok(user)
        })
        // Typed actor - handles events with automatic framing and serialization
        .room_handler(with_typed_actor::<ChatUser, ChatProtocol, _, _>(chat_actor))
        .build()
        .expect("Failed to build server");

    server.run().await?;

    Ok(())
}

/// Chat room actor - receives typed events, one at a time.
async fn chat_actor(
    ctx: TypedContext<ChatUser, ChatProtocol>,
    event: TypedEvent<ChatUser, ChatProtocol>,
) {
    let room_id = ctx.room_id();

    match event {
        TypedEvent::UserConnected(user) => {
            tracing::info!("[Room {:?}] {} connected", room_id, user.username);

            // Broadcast join message
            let msg = ChatEvent::Chat(ChatMessage::System(format!(
                "*** {} joined the chat ***",
                user.username
            )));
            ctx.broadcast(&msg).await;
        }

        TypedEvent::UserDisconnected(user) => {
            tracing::info!("[Room {:?}] {} disconnected", room_id, user.username);

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
        } => {
            tracing::info!("[Room {:?}] {}: {}", room_id, sender.username, content);

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

        TypedEvent::Shutdown => {
            tracing::info!("[Room {:?}] Shutting down", room_id);
        }
    }
}
