//! Simple chat server using multiplayer-kit.
//!
//! Run with: cargo run --bin chat-server
//!
//! Protocol: Messages use 4-byte big-endian length prefix + UTF-8 string.

use multiplayer_kit_protocol::{Outgoing, RejectReason, RoomEvent, Route, UserContext};
use multiplayer_kit_server::{AuthRequest, RoomContext, Server};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
        // Room actor - runs for each room, handles events
        // 
        // Messages arrive as raw byte chunks from persistent streams.
        // We reassemble them using a simple 4-byte length prefix protocol.
        .room_actor(|mut ctx: RoomContext<ChatUser>| async move {
            let room_id = ctx.room_id;
            tracing::info!("[Room {:?}] Actor started", room_id);

            // Per-user message buffers for reassembly
            let mut user_buffers: HashMap<u64, Vec<u8>> = HashMap::new();

            loop {
                match ctx.events.recv().await {
                    Some(RoomEvent::UserJoined(user)) => {
                        tracing::info!("[Room {:?}] {} joined", room_id, user.username);
                        
                        // Init buffer for this user
                        user_buffers.insert(user.id, Vec::new());
                        
                        // Broadcast join notification
                        let msg = format!("*** {} joined the chat ***", user.username);
                        ctx.send(Outgoing::new(
                            frame_message(&msg),
                            Route::Broadcast,
                        )).await;
                    }
                    Some(RoomEvent::UserLeft(user)) => {
                        tracing::info!("[Room {:?}] {} left", room_id, user.username);
                        
                        // Clean up buffer
                        user_buffers.remove(&user.id);
                        
                        // Broadcast leave notification
                        let msg = format!("*** {} left the chat ***", user.username);
                        ctx.send(Outgoing::new(
                            frame_message(&msg),
                            Route::Broadcast,
                        )).await;
                    }
                    Some(RoomEvent::Message { sender, payload }) => {
                        // Append to this user's buffer
                        let buffer = user_buffers.entry(sender.id).or_default();
                        buffer.extend_from_slice(&payload);
                        
                        // Try to extract complete messages
                        while let Some(message) = try_extract_message(buffer) {
                            // Validate
                            let text = match std::str::from_utf8(&message) {
                                Ok(s) if !s.is_empty() && s.len() <= 4096 => s,
                                _ => {
                                    tracing::warn!("[Room {:?}] Invalid message from {}", room_id, sender.username);
                                    continue;
                                }
                            };
                            
                            tracing::info!("[Room {:?}] {}", room_id, text);
                            
                            // Forward to others (sender already has local echo)
                            ctx.send(Outgoing::new(
                                frame_message(text),
                                Route::AllExcept(vec![sender.id()]),
                            )).await;
                        }
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

/// Frame a message with 4-byte length prefix.
fn frame_message(msg: &str) -> Vec<u8> {
    let bytes = msg.as_bytes();
    let len = (bytes.len() as u32).to_be_bytes();
    let mut out = Vec::with_capacity(4 + bytes.len());
    out.extend_from_slice(&len);
    out.extend_from_slice(bytes);
    out
}

/// Try to extract a complete message from the buffer.
/// Returns Some(message) and removes it from the buffer, or None if incomplete.
fn try_extract_message(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    if buffer.len() < 4 {
        return None;
    }
    
    let len = u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) as usize;
    
    if len > 10 * 1024 * 1024 {
        // Invalid length, clear buffer to recover
        buffer.clear();
        return None;
    }
    
    if buffer.len() < 4 + len {
        return None;
    }
    
    // Extract message
    let message = buffer[4..4 + len].to_vec();
    buffer.drain(..4 + len);
    Some(message)
}
