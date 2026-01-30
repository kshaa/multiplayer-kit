//! Simple chat server using multiplayer-kit.
//!
//! Run with: cargo run --bin chat-server

use multiplayer_kit_protocol::{RejectReason, Route, UserContext};
use multiplayer_kit_server::{AuthRequest, Server};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

/// User context - just a username and ID.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatUser {
    id: u64,
    username: String,
}

impl UserContext for ChatUser {
    type Id = u64;
    fn id(&self) -> u64 {
        self.id
    }
}

static NEXT_USER_ID: AtomicU64 = AtomicU64::new(1);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    println!("Starting chat server...");
    println!("  HTTP: http://127.0.0.1:8080");
    println!("  QUIC: https://127.0.0.1:4433");
    println!();
    println!("Endpoints:");
    println!("  POST /ticket         - Get auth ticket (body: {{\"username\": \"name\"}})");
    println!("  POST /rooms          - Create room");
    println!("  GET  /rooms          - List rooms");
    println!("  DELETE /rooms/{{id}}   - Delete room");
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
        .room_handler(|payload, ctx| {
            // Copy data before async block
            let message = std::str::from_utf8(payload).map(|s| s.to_string());
            let room_id = ctx.room_id;
            let username = ctx.sender.username.clone();

            async move {
                let message = message
                    .map_err(|_| RejectReason::ValidationFailed("Invalid UTF-8".into()))?;

                if message.is_empty() {
                    return Err(RejectReason::ValidationFailed("Empty message".into()));
                }
                if message.len() > 1000 {
                    return Err(RejectReason::ValidationFailed("Message too long".into()));
                }

                tracing::info!("[Room {:?}] {}: {}", room_id, username, message);
                Ok(Route::BroadcastIncludingSelf)
            }
        })
        .build()
        .expect("Failed to build server");

    server.run().await?;

    Ok(())
}
