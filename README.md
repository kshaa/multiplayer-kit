# multiplayer-kit

Generic multiplayer server and client library for real-time games. WebTransport (QUIC) for both browser and native clients.

## Architecture

```
┌─────────────────┐     REST      ┌─────────────────┐     REST      ┌─────────────────┐
│   Next.js App   │───────────────│                 │───────────────│   Bevy Game     │
│   (Lobby UI)    │               │     Server      │               │   (Room/Game)   │
│                 │  Lobby QUIC   │                 │  Room QUIC    │                 │
│                 │═══════════════│                 │═══════════════│                 │
└─────────────────┘               └─────────────────┘               └─────────────────┘
```

**Server** is a dumb relay with configurable validation. It does not interpret game messages.

**REST endpoints:**
- `POST /ticket` - get JWT (custom auth logic)
- `POST /rooms` - create room
- `DELETE /rooms/{id}` - delete room (creator only)

**QUIC endpoints:**
- Lobby - streams room list and updates
- Room - relays game messages between participants

## Crates

| Crate | Purpose |
|-------|---------|
| `protocol` | Shared types: `UserContext` trait, `Route`, `MessageContext` |
| `server` | REST (actix-web) + QUIC (wtransport) server |
| `client` | `LobbyClient` + `RoomClient`, works native and WASM |

## Usage

Define your user context:

```rust
#[derive(Serialize, Deserialize, Clone)]
struct MyUser {
    user_id: u64,
    username: String,
}

impl UserContext for MyUser {
    type Id = u64;
    fn id(&self) -> u64 { self.user_id }
}
```

Build the server:

```rust
let server = Server::<MyUser>::builder()
    .jwt_secret(b"secret")
    .auth_handler(|req| async move {
        // validate request, return user data for JWT
        Ok(MyUser { user_id: 1, username: "alice".into() })
    })
    .room_handler(|payload, ctx| async move {
        // validate message, return routing decision
        Ok(Route::Broadcast)
    })
    .build()?
    .run()
    .await?;
```

Connect from client:

```rust
let lobby = LobbyClient::connect(url, ticket).await?;
let event = lobby.recv().await?;

let room = RoomClient::connect(url, ticket, room_id).await?;
room.send(&payload).await?;
let msg = room.recv().await?;
```

## Room Lifecycle

- 5 min timeout for first connection
- 5 min timeout if room becomes empty
- 1 hour max lifetime
- Creator can delete via REST
