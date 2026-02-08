//! Simple chat CLI client using chat-client.
//!
//! Run with: cargo run --bin chat-cli

use chat_client::{ApiClient, ChatClient, RoomId};
use serde::Serialize;
use std::io::{self, BufRead, Write};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

const SERVER_HTTP: &str = "http://127.0.0.1:8080";
const SERVER_QUIC: &str = "https://127.0.0.1:4433";

#[derive(Serialize)]
struct AuthRequest {
    username: String,
}

enum ChatEvent {
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

    let api = ApiClient::new(SERVER_HTTP);
    let ticket_resp = api
        .get_ticket(&AuthRequest {
            username: username.clone(),
        })
        .await?;
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

    // Chat client state
    let mut client: Option<Arc<Mutex<ChatClient>>> = None;
    let mut in_room = false;

    // Channel for stdin lines
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

    // Channel for chat events
    let (chat_event_tx, mut chat_event_rx) = mpsc::channel::<ChatEvent>(256);

    // Print initial prompt
    print!("> ");
    io::stdout().flush()?;

    loop {
        tokio::select! {
            // Handle incoming chat events
            Some(event) = chat_event_rx.recv() => {
                match event {
                    ChatEvent::Message(msg) => {
                        print!("\r\x1b[K{}\n", msg);
                    }
                    ChatEvent::Disconnected(reason) => {
                        print!("\r\x1b[K[Disconnected: {}]\n", reason);
                        client = None;
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
                    let arg = parts.get(1).copied();

                    match cmd {
                        "/quit" | "/exit" | "/q" => {
                            println!("Goodbye!");
                            break;
                        }

                        "/rooms" => {
                            match api.list_rooms().await {
                                Ok(rooms) => {
                                    if rooms.is_empty() {
                                        println!("No rooms available. Create one with /create");
                                    } else {
                                        println!("Available rooms:");
                                        for r in &rooms {
                                            println!("  [{}] {} players", r.id.0, r.player_count);
                                        }
                                    }
                                }
                                Err(e) => {
                                    println!("Failed to list rooms: {}", e);
                                }
                            }
                        }

                        "/create" => {
                            println!("Creating room...");
                            match api.create_room(&ticket).await {
                                Ok(create_resp) => {
                                    println!("Created room {}. Join with: /join {}", create_resp.room_id, create_resp.room_id);
                                }
                                Err(e) => {
                                    println!("Failed to create room: {}", e);
                                }
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
                                            print!("> ");
                                            io::stdout().flush()?;
                                            continue;
                                        }
                                    },
                                    None => {
                                        println!("Usage: /join <room_id>");
                                        print!("> ");
                                        io::stdout().flush()?;
                                        continue;
                                    }
                                };

                                println!("Joining room {}...", room_id);
                                match ChatClient::connect(
                                    SERVER_QUIC,
                                    &ticket,
                                    RoomId(room_id),
                                    username.clone(),
                                ).await {
                                    Ok(chat_client) => {
                                        println!("Joined room {}! Start chatting.", room_id);

                                        let chat_client = Arc::new(Mutex::new(chat_client));
                                        client = Some(Arc::clone(&chat_client));
                                        in_room = true;

                                        // Spawn read task
                                        let event_tx = chat_event_tx.clone();
                                        tokio::spawn(async move {
                                            loop {
                                                let result = {
                                                    let mut c = chat_client.lock().await;
                                                    c.receive_text().await
                                                };

                                                match result {
                                                    Ok(Some(msg)) => {
                                                        if event_tx.send(ChatEvent::Message(msg)).await.is_err() {
                                                            break;
                                                        }
                                                    }
                                                    Ok(None) => {
                                                        let _ = event_tx.send(ChatEvent::Disconnected("Stream closed".to_string())).await;
                                                        break;
                                                    }
                                                    Err(e) => {
                                                        let _ = event_tx.send(ChatEvent::Disconnected(e.to_string())).await;
                                                        break;
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
                                client = None;
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
                    if let Some(ref c) = client {
                        let c = c.lock().await;
                        if let Err(e) = c.send_text(input).await {
                            println!("Failed to send: {}", e);
                            drop(c);
                            client = None;
                            in_room = false;
                        }
                        // No local echo - server broadcasts back to us
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
