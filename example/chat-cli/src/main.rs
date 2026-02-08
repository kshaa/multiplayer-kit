//! Simple chat CLI client using typed actor pattern.
//!
//! Run with: cargo run --bin chat-cli

use chat_client::{
    ApiClient, ChatEvent, ChatMessage, ChatProtocol, RoomConnection, RoomId,
    TypedClientContext, TypedClientEvent, with_typed_client_actor,
};
use serde::Serialize;
use std::io::{self, BufRead, Write};
use std::sync::Arc;
use tokio::sync::mpsc;

const SERVER_HTTP: &str = "http://127.0.0.1:8080";
const SERVER_QUIC: &str = "https://127.0.0.1:8080";

#[derive(Serialize)]
struct AuthRequest {
    username: String,
}

/// Events from the chat actor to the main loop.
enum UiEvent {
    Message(String),
    Connected,
    Disconnected,
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
    println!("  /rooms           - List available rooms");
    println!("  /create <name>   - Create a new room");
    println!("  /join <id>       - Join a room");
    println!("  /quickplay       - Auto-join or create a room");
    println!("  /leave           - Leave current room");
    println!("  /quit            - Exit");
    println!("  <message>        - Send a chat message (when in a room)");
    println!();

    // State
    let mut in_room = false;
    let mut send_tx: Option<mpsc::Sender<String>> = None;

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

    // Channel for UI events from chat actor
    let (ui_tx, mut ui_rx) = mpsc::channel::<UiEvent>(256);

    // Print initial prompt
    print!("> ");
    io::stdout().flush()?;

    loop {
        tokio::select! {
            // Handle UI events from chat actor
            Some(event) = ui_rx.recv() => {
                match event {
                    UiEvent::Message(msg) => {
                        print!("\r\x1b[K{}\n", msg);
                    }
                    UiEvent::Connected => {
                        print!("\r\x1b[K[Connected]\n");
                    }
                    UiEvent::Disconnected => {
                        print!("\r\x1b[K[Disconnected]\n");
                        in_room = false;
                        send_tx = None;
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
                                        println!("No rooms available. Create one with /create <name>");
                                    } else {
                                        println!("Available rooms:");
                                        for r in &rooms {
                                            println!("  [{}] '{}' - {} players", r.id.0, r.name, r.player_count);
                                        }
                                    }
                                }
                                Err(e) => {
                                    println!("Failed to list rooms: {}", e);
                                }
                            }
                        }

                        "/create" => {
                            let name = match arg {
                                Some(n) => n.to_string(),
                                None => {
                                    println!("Usage: /create <room_name>");
                                    print!("> ");
                                    io::stdout().flush()?;
                                    continue;
                                }
                            };
                            println!("Creating room '{}'...", name);
                            let config = serde_json::json!({ "name": name });
                            match api.create_room(&ticket, &config).await {
                                Ok(create_resp) => {
                                    println!("Created room '{}' (id: {}). Join with: /join {}", name, create_resp.room_id, create_resp.room_id);
                                }
                                Err(e) => {
                                    println!("Failed to create room: {}", e);
                                }
                            }
                        }

                        "/quickplay" => {
                            if in_room {
                                println!("Already in a room. Use /leave first.");
                            } else {
                                println!("Finding or creating a room...");
                                match api.quickplay::<()>(&ticket, None).await {
                                    Ok(resp) => {
                                        let action = if resp.created { "Created" } else { "Found" };
                                        println!("{} room {}. Joining...", action, resp.room_id.0);
                                        
                                        match RoomConnection::connect(SERVER_QUIC, &ticket, resp.room_id).await {
                                            Ok(conn) => {
                                                let (tx, rx) = mpsc::channel::<String>(256);
                                                send_tx = Some(tx);
                                                in_room = true;

                                                let ui = ui_tx.clone();
                                                let user = username.clone();
                                                tokio::spawn(async move {
                                                    run_chat_actor(conn, user, ui, rx).await;
                                                });

                                                println!("Connected! Type messages or /leave to exit.");
                                            }
                                            Err(e) => {
                                                println!("Failed to join room: {}", e);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        println!("Quickplay failed: {}", e);
                                    }
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
                                match RoomConnection::connect(SERVER_QUIC, &ticket, RoomId(room_id)).await {
                                    Ok(conn) => {
                                        // Create channel for sending messages
                                        let (tx, rx) = mpsc::channel::<String>(256);
                                        send_tx = Some(tx);
                                        in_room = true;

                                        // Spawn typed actor
                                        let ui = ui_tx.clone();
                                        let user = username.clone();
                                        tokio::spawn(async move {
                                            run_chat_actor(conn, user, ui, rx).await;
                                        });
                                    }
                                    Err(e) => {
                                        println!("Failed to connect: {}", e);
                                    }
                                }
                            }
                        }

                        "/leave" => {
                            if in_room {
                                // Dropping send_tx will cause the actor to see the channel close
                                send_tx = None;
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
                    // Send chat message via actor
                    if let Some(ref tx) = send_tx {
                        if tx.send(input.to_string()).await.is_err() {
                            println!("Failed to send (disconnected)");
                            send_tx = None;
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

/// Run the chat actor using typed actor pattern.
async fn run_chat_actor(
    conn: RoomConnection,
    username: String,
    ui_tx: mpsc::Sender<UiEvent>,
    input_rx: mpsc::Receiver<String>,
) {
    // Wrap input_rx in Arc for sharing with actor
    let input_rx = Arc::new(tokio::sync::Mutex::new(input_rx));
    let ui_tx = Arc::new(ui_tx);
    let username = Arc::new(username);

    with_typed_client_actor::<ChatProtocol, _, _>(conn, {
        let input_rx = Arc::clone(&input_rx);
        let ui_tx = Arc::clone(&ui_tx);
        let username = Arc::clone(&username);

        move |ctx: TypedClientContext<ChatProtocol>, event: TypedClientEvent<ChatProtocol>| {
            let input_rx = Arc::clone(&input_rx);
            let ui_tx = Arc::clone(&ui_tx);
            let username = Arc::clone(&username);

            async move {
                match event {
                    TypedClientEvent::Connected => {
                        let _ = ui_tx.send(UiEvent::Connected).await;

                        // Start a task to forward user input to the actor's send
                        let ctx_clone = ctx.clone();
                        let username = Arc::clone(&username);
                        let input_rx = Arc::clone(&input_rx);
                        tokio::spawn(async move {
                            loop {
                                let msg = {
                                    let mut rx = input_rx.lock().await;
                                    rx.recv().await
                                };
                                match msg {
                                    Some(text) => {
                                        let msg = ChatEvent::Chat(ChatMessage::Text {
                                            username: (*username).clone(),
                                            content: text,
                                        });
                                        let _ = ctx_clone.send(&msg).await;
                                    }
                                    None => break, // Input channel closed = leave room
                                }
                            }
                        });
                    }
                    TypedClientEvent::Message(ChatEvent::Chat(ChatMessage::Text { username, content })) => {
                        let _ = ui_tx.send(UiEvent::Message(format!("{}: {}", username, content))).await;
                    }
                    TypedClientEvent::Message(ChatEvent::Chat(ChatMessage::System(msg))) => {
                        let _ = ui_tx.send(UiEvent::Message(msg)).await;
                    }
                    TypedClientEvent::Disconnected => {
                        let _ = ui_tx.send(UiEvent::Disconnected).await;
                    }
                }
            }
        }
    })
    .await;
}
