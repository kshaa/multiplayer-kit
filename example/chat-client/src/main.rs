//! Simple chat client REPL using multiplayer-kit.
//!
//! Run with: cargo run --bin chat-client
//!
//! Commands:
//!   /rooms           - List available rooms
//!   /create          - Create a new room
//!   /join <id>       - Join a room
//!   /leave           - Leave current room
//!   /quit            - Exit
//!   <message>        - Send a chat message (when in a room)

use multiplayer_kit_client::{LobbyClient, RoomClient};
use multiplayer_kit_protocol::{LobbyEvent, RoomId, RoomInfo};
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};

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
    let mut current_room: Option<RoomClient> = None;
    let mut lobby: Option<LobbyClient> = None;

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        if current_room.is_some() {
            print!("[room] > ");
        } else {
            print!("> ");
        }
        stdout.flush()?;

        let mut input = String::new();
        if stdin.lock().read_line(&mut input)? == 0 {
            break;
        }
        let input = input.trim();

        if input.is_empty() {
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
                                continue;
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
                                continue;
                            }
                        }
                    }

                    if rooms.is_empty() {
                        println!("No rooms available. Create one with /create");
                    } else {
                        println!("Available rooms:");
                        for room in &rooms {
                            println!("  [{}] {} players", room.id.0, room.player_count);
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
                    if current_room.is_some() {
                        println!("Already in a room. Use /leave first.");
                        continue;
                    }

                    let room_id = match arg {
                        Some(id) => match id.parse::<u64>() {
                            Ok(id) => id,
                            Err(_) => {
                                println!("Invalid room ID");
                                continue;
                            }
                        },
                        None => {
                            println!("Usage: /join <room_id>");
                            continue;
                        }
                    };

                    println!("Joining room {}...", room_id);
                    match RoomClient::connect(SERVER_QUIC, &ticket, RoomId(room_id)).await {
                        Ok(room) => {
                            println!("Joined room {}! Start chatting.", room_id);
                            current_room = Some(room);
                        }
                        Err(e) => {
                            println!("Failed to join room: {}", e);
                        }
                    }
                }

                "/leave" => {
                    if let Some(room) = current_room.take() {
                        let _ = room.close().await;
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
            if let Some(ref room) = current_room {
                let message = format!("{}: {}", username, input);
                match room.send(message.as_bytes()).await {
                    Ok(_) => {}
                    Err(e) => {
                        println!("Failed to send: {}", e);
                    }
                }
            } else {
                println!("Not in a room. Use /join <id> first.");
            }
        }

        // Check for incoming room messages
        if let Some(ref mut room) = current_room {
            match tokio::time::timeout(std::time::Duration::from_millis(10), room.recv()).await {
                Ok(Ok(data)) => {
                    if let Ok(msg) = String::from_utf8(data) {
                        println!("{}", msg);
                    }
                }
                Ok(Err(e)) => {
                    println!("Room disconnected: {}", e);
                    current_room = None;
                }
                Err(_) => {}
            }
        }
    }

    Ok(())
}
