//! QUIC/WebTransport handlers for lobby and room connections.

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
    let (mut send_stream, recv_stream) = connection.accept_bi().await?;
    let ticket = read_string(recv_stream).await?;

    let user: T = state.ticket_manager.validate(&ticket).map_err(|e| {
        tracing::warn!("Invalid ticket for room: {:?}", e);
        e
    })?;

    let user_id = user.id();
    tracing::info!("Room client authenticated, joining room {:?}", room_id);

    // Create a channel for outgoing messages to this client
    let (msg_tx, mut msg_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);

    // Add participant to room with their message channel
    // This will also notify the actor of the join
    room.add_participant(user.clone(), msg_tx).await;

    // Notify lobby of player count change
    if let Some(info) = state.room_manager.get_room_info(room_id) {
        state.lobby.notify_room_updated(info);
    }

    // Take the outbox receiver to broadcast actor messages
    // (Only the first connection to a room will get this; others won't need it
    // since the broadcast task will already be running)
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

    // Send join confirmation
    send_stream.write_all(b"joined").await?;

    // Spawn task to send outgoing messages to this client
    let send_connection = connection.clone();
    let send_task = tokio::spawn(async move {
        while let Some(msg) = msg_rx.recv().await {
            match send_connection.open_uni().await {
                Ok(stream) => {
                    let mut stream = match stream.await {
                        Ok(s) => s,
                        Err(_) => break,
                    };
                    if stream.write_all(&msg).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Main receive loop - forward messages to actor
    loop {
        tokio::select! {
            result = connection.accept_bi() => {
                match result {
                    Ok((_send, recv)) => {
                        let payload = match read_bytes(recv).await {
                            Ok(p) => p,
                            Err(_) => continue,
                        };

                        // Send message to actor
                        room.send_message(user.clone(), payload).await;
                    }
                    Err(_) => break,
                }
            }
            _ = connection.closed() => {
                tracing::debug!("Room connection closed");
                break;
            }
        }
    }

    // Cleanup
    send_task.abort();
    room.remove_participant(&user_id).await;

    // Notify lobby of player count change
    if let Some(info) = state.room_manager.get_room_info(room_id) {
        state.lobby.notify_room_updated(info);
    }

    Ok(())
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
