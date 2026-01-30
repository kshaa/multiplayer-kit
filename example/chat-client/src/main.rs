//! Simple chat client REPL using multiplayer-kit.
//!
//! Run with: cargo run --bin chat-client

use multiplayer_kit_client::{LobbyClient, RoomClient};
use multiplayer_kit_protocol::{LobbyEvent, RoomId, RoomInfo};
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};
use tokio::sync::mpsc;

const SERVER_HTTP: &str = "http://127.0.0.1:8080";
const SERVER_QUIC: &str = "https://127.0.0.1:4433";

#[derive(Serialize)]
struct AuthRequest {
    username: String,
}

#[derive(Deserialize)]
struct TicketResponse {
    ticket: String,
}

#[derive(Deserialize)]
struct CreateRoomResponse {
    room_id: u64,
}

enum RoomEvent {
    Message(String),
    Disconnected(String),
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Chat Client ===");
    println!();

    print!("Enter your username: ");
    io::stdout().flush()?;
    let mut username = String::new();
    io::stdin().lock().read_line(&mut username)?;
    let username = username.trim().to_string();

    if username.is_empty() {
        println!("Username cannot be empty!");
        return Ok(());
    }

    println!();
    println!("Getting ticket...");

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/ticket", SERVER_HTTP))
        .json(&AuthRequest {
            username: username.clone(),
        })
        .send()
        .await?;

    if !resp.status().is_success() {
        println!("Failed to get ticket: {}", resp.text().await?);
        return Ok(());
    }

    let ticket_resp: TicketResponse = resp.json().await?;
    let ticket = ticket_resp.ticket;

    println!("Got ticket!");
    println!();
    println!("Commands:");
    println!("  /rooms     - List available rooms");
    println!("  /create    - Create a new room");
    println!("  /join <id> - Join a room");
    println!("  /leave     - Leave current room");
    println!("  /quit      - Exit");
    println!("  <message>  - Send a chat message (when in a room)");
    println!();

    let mut rooms: Vec<RoomInfo> = Vec::new();
    let mut lobby: Option<LobbyClient> = None;
    
    // Room state - sender for outgoing messages, receiver for incoming
    let mut room_sender: Option<mpsc::Sender<Vec<u8>>> = None;
    let mut in_room = false;
    
    // Channel for stdin lines (read in blocking thread)
    let (line_tx, mut line_rx) = mpsc::channel::<String>(10);
    
    // Spawn blocking stdin reader
    std::thread::spawn(move || {
        let stdin = io::stdin();
        loop {
            let mut line = String::new();
            match stdin.lock().read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if line_tx.blocking_send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    
    // Channel for room events (messages and disconnections)
    let (room_event_tx, mut room_event_rx) = mpsc::channel::<RoomEvent>(256);

    // Print initial prompt
    print!("> ");
    io::stdout().flush()?;

    loop {
        tokio::select! {
            // Handle incoming room events
            Some(event) = room_event_rx.recv() => {
                match event {
                    RoomEvent::Message(msg) => {
                        print!("\r\x1b[K{}\n", msg);
                    }
                    RoomEvent::Disconnected(reason) => {
                        print!("\r\x1b[K[Disconnected: {}]\n", reason);
                        room_sender = None;
                        in_room = false;
                    }
                }
                // Reprint prompt
                if in_room {
                    print!("[room] > ");
                } else {
                    print!("> ");
                }
                io::stdout().flush()?;
            }
            
            // Handle user input
            Some(line) = line_rx.recv() => {
                let input = line.trim();
                
                if input.is_empty() {
                    if in_room {
                        print!("[room] > ");
                    } else {
                        print!("> ");
                    }
                    io::stdout().flush()?;
                    continue;
                }

                if input.starts_with('/') {
                    let parts: Vec<&str> = input.splitn(2, ' ').collect();
                    let cmd = parts[0];
                    let arg = parts.get(1).map(|s| *s);

                    match cmd {
                        "/quit" | "/exit" | "/q" => {
                            println!("Goodbye!");
                            break;
                        }

                        "/rooms" => {
                            if lobby.is_none() {
                                println!("Connecting to lobby...");
                                match LobbyClient::connect(SERVER_QUIC, &ticket).await {
                                    Ok(l) => {
                                        lobby = Some(l);
                                        println!("Connected to lobby!");
                                    }
                                    Err(e) => {
                                        println!("Failed to connect to lobby: {}", e);
                                    }
                                }
                            }

                            if let Some(ref mut l) = lobby {
                                match l.recv().await {
                                    Ok(LobbyEvent::Snapshot(r)) => {
                                        rooms = r;
                                    }
                                    Ok(event) => {
                                        println!("Received event: {:?}", event);
                                    }
                                    Err(e) => {
                                        println!("Error receiving from lobby: {}", e);
                                        lobby = None;
                                    }
                                }
                            }

                            if rooms.is_empty() {
                                println!("No rooms available. Create one with /create");
                            } else {
                                println!("Available rooms:");
                                for r in &rooms {
                                    println!("  [{}] {} players", r.id.0, r.player_count);
                                }
                            }
                        }

                        "/create" => {
                            println!("Creating room...");
                            let resp = client
                                .post(format!("{}/rooms", SERVER_HTTP))
                                .header("Authorization", format!("Bearer {}", ticket))
                                .json(&serde_json::json!({}))
                                .send()
                                .await?;

                            if resp.status().is_success() {
                                let create_resp: CreateRoomResponse = resp.json().await?;
                                println!("Created room {}. Join with: /join {}", create_resp.room_id, create_resp.room_id);
                            } else {
                                println!("Failed to create room: {}", resp.text().await?);
                            }
                        }

                        "/join" => {
                            if in_room {
                                println!("Already in a room. Use /leave first.");
                            } else {
                                let room_id = match arg {
                                    Some(id) => match id.parse::<u64>() {
                                        Ok(id) => id,
                                        Err(_) => {
                                            println!("Invalid room ID");
                                            if in_room { print!("[room] > "); } else { print!("> "); }
                                            io::stdout().flush()?;
                                            continue;
                                        }
                                    },
                                    None => {
                                        println!("Usage: /join <room_id>");
                                        if in_room { print!("[room] > "); } else { print!("> "); }
                                        io::stdout().flush()?;
                                        continue;
                                    }
                                };

                                println!("Joining room {}...", room_id);
                                match RoomClient::connect(SERVER_QUIC, &ticket, RoomId(room_id)).await {
                                    Ok(room) => {
                                        println!("Joined room {}! Start chatting.", room_id);
                                        
                                        // Create channel for sending to room
                                        let (send_tx, mut send_rx) = mpsc::channel::<Vec<u8>>(256);
                                        room_sender = Some(send_tx);
                                        in_room = true;
                                        
                                        // Spawn task to handle room I/O
                                        let event_tx = room_event_tx.clone();
                                        tokio::spawn(async move {
                                            let mut room = room;
                                            loop {
                                                tokio::select! {
                                                    // Send outgoing messages
                                                    Some(data) = send_rx.recv() => {
                                                        if let Err(e) = room.send(&data).await {
                                                            let _ = event_tx.send(RoomEvent::Disconnected(e.to_string())).await;
                                                            break;
                                                        }
                                                    }
                                                    // Receive incoming messages
                                                    result = room.recv() => {
                                                        match result {
                                                            Ok(data) => {
                                                                if let Ok(msg) = String::from_utf8(data) {
                                                                    let _ = event_tx.send(RoomEvent::Message(msg)).await;
                                                                }
                                                            }
                                                            Err(e) => {
                                                                let _ = event_tx.send(RoomEvent::Disconnected(e.to_string())).await;
                                                                break;
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        });
                                    }
                                    Err(e) => {
                                        println!("Failed to join room: {}", e);
                                    }
                                }
                            }
                        }

                        "/leave" => {
                            if in_room {
                                // Drop the sender, which will cause the room task to exit
                                room_sender = None;
                                in_room = false;
                                println!("Left room.");
                            } else {
                                println!("Not in a room.");
                            }
                        }

                        _ => {
                            println!("Unknown command: {}", cmd);
                        }
                    }
                } else {
                    // Send chat message
                    if let Some(ref sender) = room_sender {
                        let message = format!("{}: {}", username, input);
                        // Local echo - show immediately
                        println!("{}", message);
                        if sender.send(message.into_bytes()).await.is_err() {
                            println!("Failed to send: room disconnected");
                            room_sender = None;
                            in_room = false;
                        }
                    } else {
                        println!("Not in a room. Use /join <id> first.");
                    }
                }
                
                // Print prompt
                if in_room {
                    print!("[room] > ");
                } else {
                    print!("> ");
                }
                io::stdout().flush()?;
            }
        }
    }

    Ok(())
}
