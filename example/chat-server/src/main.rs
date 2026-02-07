//! Simple chat server using multiplayer-kit.
//!
//! Run with: cargo run --bin chat-server

use multiplayer_kit_helpers::{with_actor, with_framing, MessageContext, MessageEvent, Outgoing, Route};
use multiplayer_kit_protocol::{RejectReason, UserContext};
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
        // Room handler with actor wrapper and message framing via helpers
        .room_handler(with_actor(with_framing(chat_room_actor)))
        .build()
        .expect("Failed to build server");

    server.run().await?;

    Ok(())
}

/// The chat room actor - receives complete messages, no manual framing needed.
async fn chat_room_actor(mut ctx: MessageContext<ChatUser>) {
    let room_id = ctx.room_id();
    tracing::info!("[Room {:?}] Actor started", room_id);

    loop {
        match ctx.recv().await {
            Some(MessageEvent::UserJoined(user)) => {
                tracing::info!("[Room {:?}] {} joined", room_id, user.username);
                let msg = format!("*** {} joined the chat ***", user.username);
                ctx.send(Outgoing::new(msg.into_bytes(), Route::All)).await;
            }
            Some(MessageEvent::UserLeft(user)) => {
                tracing::info!("[Room {:?}] {} left", room_id, user.username);
                let msg = format!("*** {} left the chat ***", user.username);
                ctx.send(Outgoing::new(msg.into_bytes(), Route::All)).await;
            }
            Some(MessageEvent::ChannelOpened { user, channel }) => {
                tracing::debug!("[Room {:?}] Channel {:?} opened by {}", room_id, channel, user.username);
            }
            Some(MessageEvent::ChannelClosed { user, channel }) => {
                tracing::debug!("[Room {:?}] Channel {:?} closed by {}", room_id, channel, user.username);
            }
            Some(MessageEvent::Message { sender, channel, data }) => {
                let text = match std::str::from_utf8(&data) {
                    Ok(s) if !s.is_empty() && s.len() <= 4096 => s,
                    _ => {
                        tracing::warn!("[Room {:?}] Invalid message from {}", room_id, sender.username);
                        continue;
                    }
                };
                tracing::info!("[Room {:?}] {}", room_id, text);
                
                // Forward to all OTHER channels (sender has local echo)
                ctx.send(Outgoing::new(data, Route::AllExcept(channel))).await;
            }
            Some(MessageEvent::Shutdown) => {
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
}
