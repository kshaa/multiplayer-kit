//! Simple chat server using multiplayer-kit.
//!
//! Run with: cargo run --bin chat-server

use multiplayer_kit_protocol::{Outgoing, RejectReason, RoomEvent, Route, UserContext};
use multiplayer_kit_server::{AuthRequest, RoomContext, Server};
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
        // Room actor - runs for each room, handles events
        .room_actor(|mut ctx: RoomContext<ChatUser>| async move {
            let room_id = ctx.room_id;
            tracing::info!("[Room {:?}] Actor started", room_id);

            loop {
                match ctx.events.recv().await {
                    Some(RoomEvent::UserJoined(user)) => {
                        tracing::info!("[Room {:?}] {} joined", room_id, user.username);
                        ctx.send(Outgoing::new(
                            format!("*** {} joined the chat ***", user.username),
                            Route::Broadcast,
                        )).await;
                    }
                    Some(RoomEvent::UserLeft(user)) => {
                        tracing::info!("[Room {:?}] {} left", room_id, user.username);
                        ctx.send(Outgoing::new(
                            format!("*** {} left the chat ***", user.username),
                            Route::Broadcast,
                        )).await;
                    }
                    Some(RoomEvent::Message { sender, payload }) => {
                        // Validate message
                        let message = match std::str::from_utf8(&payload) {
                            Ok(s) if !s.is_empty() && s.len() <= 1000 => s,
                            _ => {
                                tracing::warn!("[Room {:?}] Invalid message from {}", room_id, sender.username);
                                continue;
                            }
                        };

                        tracing::info!("[Room {:?}] {}: {}", room_id, sender.username, message);

                        // Broadcast to everyone except sender (they already see their own message)
                        ctx.send(Outgoing::new(
                            payload,
                            Route::AllExcept(vec![sender.id()]),
                        )).await;
                    }
                    Some(RoomEvent::Shutdown) => {
                        tracing::info!("[Room {:?}] Shutting down", room_id);
                        break;
                    }
                    None => {
                        tracing::info!("[Room {:?}] Event channel closed", room_id);
                        break;
                    }
                }
            }

            tracing::info!("[Room {:?}] Actor stopped", room_id);
        })
        .build()
        .expect("Failed to build server");

    server.run().await?;

    Ok(())
}
