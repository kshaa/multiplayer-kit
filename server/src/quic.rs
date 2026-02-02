//! QUIC/WebTransport handlers for lobby and room connections.
//!
//! Room connections use persistent bidirectional streams:
//! - First bi-stream: auth (send ticket, receive "joined")
//! - Subsequent bi-streams: channels (each stays open for the session)
//!   Communication is bidirectional on the same stream.

use crate::lobby::Lobby;
use crate::room::RoomManager;
use crate::ticket::TicketManager;
use multiplayer_kit_protocol::{LobbyEvent, RoomId, UserContext};
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use wtransport::endpoint::IncomingSession;
use wtransport::Connection;

/// Shared state for QUIC handlers.
pub struct QuicState<T: UserContext> {
    pub room_manager: Arc<RoomManager<T>>,
    pub ticket_manager: Arc<TicketManager>,
    pub lobby: Arc<Lobby>,
}

/// Handle an incoming WebTransport session.
pub async fn handle_session<T: UserContext>(
    incoming: IncomingSession,
    state: Arc<QuicState<T>>,
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
async fn handle_lobby_connection<T: UserContext>(
    connection: Connection,
    state: Arc<QuicState<T>>,
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
/// 4. Client sends on bi-stream -> server reads -> actor processes
/// 5. Actor sends response -> server writes back on SAME bi-stream
async fn handle_room_connection<T: UserContext>(
    connection: Connection,
    room_id: RoomId,
    state: Arc<QuicState<T>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Get the room
    let room = state
        .room_manager
        .get_room(room_id)
        .ok_or("Room not found")?;

    // Authenticate via first bidirectional stream
    let (mut auth_send, auth_recv) = connection.accept_bi().await?;
    let ticket = read_string(auth_recv).await?;

    let user: T = state.ticket_manager.validate(&ticket).map_err(|e| {
        tracing::warn!("Invalid ticket for room: {:?}", e);
        e
    })?;

    let user_id = user.id();
    tracing::info!("Room client authenticated, joining room {:?}", room_id);

    // Create a channel for outgoing messages to this client
    let (msg_tx, msg_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);

    // Add participant to room with their message channel
    room.add_participant(user.clone(), msg_tx).await;

    // Notify lobby of player count change
    if let Some(info) = state.room_manager.get_room_info(room_id) {
        state.lobby.notify_room_updated(info);
    }

    // Take the outbox receiver to broadcast actor messages
    if let Some(mut outbox_rx) = room.take_outbox_rx().await {
        let room_for_broadcast = Arc::clone(&room);
        tokio::spawn(async move {
            while let Some(outgoing) = outbox_rx.recv().await {
                room_for_broadcast
                    .broadcast(&outgoing.payload, outgoing.route)
                    .await;
            }
        });
    }

    // Send join confirmation on auth stream
    auth_send.write_all(b"joined").await?;

    // Wrap msg_rx in Arc<Mutex> so channel handler can share it
    let msg_rx = Arc::new(tokio::sync::Mutex::new(msg_rx));

    // Track active channel streams
    let mut channel_tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    // Main loop - accept new channel streams
    loop {
        tokio::select! {
            result = connection.accept_bi() => {
                match result {
                    Ok((send, recv)) => {
                        // Spawn a task to handle this channel's persistent bi-stream
                        let channel_user = user.clone();
                        let channel_room = Arc::clone(&room);
                        let channel_msg_rx = Arc::clone(&msg_rx);
                        
                        let task = tokio::spawn(async move {
                            handle_channel_stream(send, recv, channel_user, channel_room, channel_msg_rx).await;
                        });
                        channel_tasks.push(task);
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

    // Cleanup
    for task in channel_tasks {
        task.abort();
    }
    room.remove_participant(&user_id).await;

    // Notify lobby of player count change
    if let Some(info) = state.room_manager.get_room_info(room_id) {
        state.lobby.notify_room_updated(info);
    }

    Ok(())
}

/// Handle a single persistent channel bi-stream.
/// 
/// This is bidirectional:
/// - Reads from client and forwards to room actor
/// - Receives from msg_rx and writes back to client on same stream
async fn handle_channel_stream<T: UserContext>(
    mut send: wtransport::SendStream,
    mut recv: wtransport::RecvStream,
    user: T,
    room: Arc<crate::room::Room<T>>,
    msg_rx: Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Vec<u8>>>>,
) {
    let mut buf = vec![0u8; 64 * 1024]; // 64KB read buffer
    
    loop {
        tokio::select! {
            // Read from client
            result = recv.read(&mut buf) => {
                match result {
                    Ok(Some(n)) => {
                        if n == 0 {
                            // Stream closed gracefully
                            break;
                        }
                        // Forward raw bytes to room actor
                        let payload = buf[..n].to_vec();
                        room.send_message(user.clone(), payload).await;
                    }
                    Ok(None) => {
                        // Stream closed gracefully
                        break;
                    }
                    Err(e) => {
                        tracing::debug!("Channel read error: {:?}", e);
                        break;
                    }
                }
            }
            // Write to client (messages from actor broadcast)
            result = async {
                let mut rx = msg_rx.lock().await;
                rx.recv().await
            } => {
                match result {
                    Some(msg) => {
                        if let Err(e) = send.write_all(&msg).await {
                            tracing::debug!("Channel write error: {:?}", e);
                            break;
                        }
                    }
                    None => {
                        // msg_rx closed
                        break;
                    }
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
