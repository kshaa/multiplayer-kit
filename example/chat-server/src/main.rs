//! Simple chat server using multiplayer-kit with typed actors.
//!
//! Run with: cargo run --bin chat-server

use chat_server::{ChatActor, ChatRoomExtras};
use chat_protocol::{ChatEvent, ChatRoomConfig, ChatUser};
use multiplayer_kit_helpers::with_server_actor;
use multiplayer_kit_protocol::RejectReason;
use multiplayer_kit_server::{AuthRequest, GameServerContext, Server};
use serde::Deserialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

static NEXT_USER_ID: AtomicU64 = AtomicU64::new(1);

// ============================================================================
// Game Context
// ============================================================================

/// Example game context for the chat server.
struct ChatServerContext {
    server_name: String,
}

impl GameServerContext for ChatServerContext {
    type RoomExtras = ChatRoomExtras;

    fn get_room_extras(&self, _room_id: multiplayer_kit_protocol::RoomId) -> Self::RoomExtras {
        ChatRoomExtras {
            server_name: self.server_name.clone(),
        }
    }
}

impl ChatServerContext {
    fn new(server_name: impl Into<String>) -> Self {
        Self {
            server_name: server_name.into(),
        }
    }
}

// ============================================================================
// Main
// ============================================================================

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

    let game_context = ChatServerContext::new("Multiplayer-Kit Chat Demo");

    let server = Server::<ChatUser, ChatRoomConfig, ChatServerContext>::builder()
        .http_addr("127.0.0.1:8080")
        .quic_addr("127.0.0.1:8080")
        .jwt_secret(b"super-secret-key-for-dev-only")
        .cors_origins(vec![
            "http://localhost:3000".to_string(),
            "http://127.0.0.1:3000".to_string(),
            "http://localhost:8080".to_string(),
            "http://127.0.0.1:8080".to_string(),
        ])
        .context(game_context)
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
                ctx.server_name,
                user.username,
                user.id
            );
            Ok(user)
        })
        .room_handler(with_server_actor::<
            ChatActor,
            ChatUser,
            ChatEvent,
            ChatRoomConfig,
            ChatServerContext,
        >())
        .build()
        .expect("Failed to build server");

    server.run().await?;

    Ok(())
}
