//! QUIC/WebTransport handlers for lobby and room connections.
//!
//! Room connections use persistent bidirectional streams:
//! - First bi-stream: auth (send ticket, receive "joined")
//! - Subsequent bi-streams: channels (each stays open for the session)
//!   Communication is bidirectional on the same stream.

use crate::lobby::Lobby;
use crate::room::RoomManager;
use crate::ticket::TicketManager;
use multiplayer_kit_protocol::{ChannelId, LobbyEvent, RoomConfig, RoomId, UserContext};
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use wtransport::endpoint::IncomingSession;
use wtransport::Connection;

/// Shared state for QUIC handlers.
pub struct QuicState<T: UserContext, C: RoomConfig> {
    pub room_manager: Arc<RoomManager<T, C>>,
    pub ticket_manager: Arc<TicketManager>,
    pub lobby: Arc<Lobby>,
}

/// Handle an incoming WebTransport session.
pub async fn handle_session<T: UserContext, C: RoomConfig>(
    incoming: IncomingSession,
    state: Arc<QuicState<T, C>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let session_request = incoming.await?;
    let path = session_request.path().to_string();

    tracing::info!("Incoming session for path: {}", path);

    let connection = session_request.accept().await?;

    if path == "/lobby" {
        handle_lobby_connection(connection, state).await
    } else if path.starts_with("/room/") {
        let room_id_str = path.strip_prefix("/room/").unwrap_or("0");
        let room_id = room_id_str.parse::<u64>().unwrap_or(0);
        handle_room_connection(connection, RoomId(room_id), state).await
    } else {
        tracing::warn!("Unknown path: {}", path);
        Ok(())
    }
}

/// Handle a lobby WebTransport connection.
async fn handle_lobby_connection<T: UserContext, C: RoomConfig>(
    connection: Connection,
    state: Arc<QuicState<T, C>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Open a unidirectional stream to send lobby events
    let mut send_stream = connection.open_uni().await?.await?;

    // First, authenticate via bidirectional stream - client sends ticket
    let (_, recv_stream) = connection.accept_bi().await?;
    let ticket = read_string(recv_stream).await?;

    let _user: T = state.ticket_manager.validate(&ticket).map_err(|e| {
        tracing::warn!("Invalid ticket for lobby: {:?}", e);
        e
    })?;

    tracing::info!("Lobby client authenticated");

    // Send initial snapshot
    let rooms = state.room_manager.get_all_rooms();
    let snapshot = LobbyEvent::Snapshot(rooms);
    write_message(&mut send_stream, &snapshot).await?;

    // Subscribe to lobby updates
    let mut rx = state.lobby.subscribe();

    loop {
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Ok(event) => {
                        if let Err(e) = write_message(&mut send_stream, &event).await {
                            tracing::debug!("Lobby client disconnected: {:?}", e);
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
            _ = connection.closed() => {
                tracing::debug!("Lobby connection closed");
                break;
            }
        }
    }

    Ok(())
}

/// Handle a room WebTransport connection.
/// 
/// Protocol:
/// 1. Client opens first bi-stream, sends ticket
/// 2. Server validates, sends "joined"
/// 3. Client opens additional bi-streams as "channels" - each is bidirectional
/// 4. Client sends on bi-stream -> server reads -> forwards to room handler
/// 5. Room handler sends response -> routed back on SAME bi-stream
async fn handle_room_connection<T: UserContext, C: RoomConfig>(
    connection: Connection,
    room_id: RoomId,
    state: Arc<QuicState<T, C>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Check room exists
    if !state.room_manager.room_exists(room_id) {
        return Err("Room not found".into());
    }

    // Authenticate via first bidirectional stream
    let (mut auth_send, auth_recv) = connection.accept_bi().await?;
    let ticket = read_string(auth_recv).await?;

    let user: T = state.ticket_manager.validate(&ticket).map_err(|e| {
        tracing::warn!("Invalid ticket for room: {:?}", e);
        e
    })?;

    tracing::info!("Room client authenticated, joining room {:?}", room_id);

    // Send join confirmation on auth stream
    auth_send.write_all(b"joined").await?;

    // Track active channel tasks
    let mut channel_tasks: Vec<(ChannelId, tokio::task::JoinHandle<()>)> = Vec::new();

    // Main loop - accept new channel streams
    loop {
        tokio::select! {
            result = connection.accept_bi() => {
                match result {
                    Ok((send, recv)) => {
                        // Open channel via room manager
                        let channel_result = state.room_manager
                            .open_channel(room_id, user.clone())
                            .await;
                        
                        let Some((channel_id, read_tx, write_rx, close_rx)) = channel_result else {
                            tracing::warn!("Failed to open channel in room {:?}", room_id);
                            break;
                        };
                        
                        // Notify lobby of player count change
                        if let Some(info) = state.room_manager.get_room_info(room_id) {
                            state.lobby.notify_room_updated(info);
                        }
                        
                        // Spawn a task to handle this channel's persistent bi-stream
                        let rm = Arc::clone(&state.room_manager);
                        let rid = room_id;
                        
                        let task = tokio::spawn(async move {
                            handle_channel_stream(send, recv, channel_id, read_tx, write_rx, close_rx).await;
                            // Channel ended, close it
                            rm.close_channel(rid, channel_id).await;
                        });
                        channel_tasks.push((channel_id, task));
                    }
                    Err(e) => {
                        tracing::debug!("Error accepting bi-stream: {:?}", e);
                        break;
                    }
                }
            }
            _ = connection.closed() => {
                tracing::debug!("Room connection closed");
                break;
            }
        }
    }

    // Cleanup - abort and close all channels
    for (channel_id, task) in channel_tasks {
        task.abort();
        state.room_manager.close_channel(room_id, channel_id).await;
    }

    // Notify lobby of player count change
    if let Some(info) = state.room_manager.get_room_info(room_id) {
        state.lobby.notify_room_updated(info);
    }

    Ok(())
}

/// Handle a single persistent channel bi-stream.
/// 
/// This is bidirectional:
/// - Reads from QUIC stream and forwards to room handler via read_tx
/// - Receives from write_rx and writes back to QUIC stream
async fn handle_channel_stream(
    mut send: wtransport::SendStream,
    mut recv: wtransport::RecvStream,
    _channel_id: ChannelId,
    read_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    mut write_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
    mut close_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let mut buf = vec![0u8; 64 * 1024]; // 64KB read buffer
    
    loop {
        tokio::select! {
            biased;
            
            // Check for close signal
            _ = &mut close_rx => {
                tracing::debug!("Channel close signal received");
                break;
            }
            
            // Read from QUIC stream (client sends)
            result = recv.read(&mut buf) => {
                match result {
                    Ok(Some(n)) if n > 0 => {
                        // Forward to room handler
                        let payload = buf[..n].to_vec();
                        if read_tx.send(payload).await.is_err() {
                            break; // Handler gone
                        }
                    }
                    _ => break, // Stream closed or error
                }
            }
            
            // Write to QUIC stream (handler sends)
            result = write_rx.recv() => {
                match result {
                    Some(msg) => {
                        if let Err(e) = send.write_all(&msg).await {
                            tracing::debug!("Channel write error: {:?}", e);
                            break;
                        }
                    }
                    None => break, // Handler closed
                }
            }
        }
    }
}

// Helper functions for reading/writing messages

async fn read_string(
    stream: wtransport::RecvStream,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let bytes = read_bytes(stream).await?;
    Ok(String::from_utf8(bytes)?)
}

async fn read_bytes(
    mut stream: wtransport::RecvStream,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    // Read all data from the stream
    let mut data = Vec::new();
    stream.read_to_end(&mut data).await?;
    Ok(data)
}

async fn write_message<M: serde::Serialize>(
    stream: &mut wtransport::SendStream,
    msg: &M,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let bytes = bincode::serialize(msg)?;
    let len = (bytes.len() as u32).to_be_bytes();
    stream.write_all(&len).await?;
    stream.write_all(&bytes).await?;
    Ok(())
}
