# multiplayer-kit

Generic multiplayer server and client library for real-time games. WebTransport (QUIC) and WebSocket for both browser and native clients.

## Architecture

```
┌─────────────────┐     REST      ┌─────────────────┐     REST      ┌─────────────────┐
│   Web Client    │───────────────│                 │───────────────│   Native Client │
│   (Browser)     │               │     Server      │               │   (Desktop)     │
│                 │  QUIC/WS      │                 │  QUIC/WS      │                 │
│                 │═══════════════│                 │═══════════════│                 │
└─────────────────┘               └─────────────────┘               └─────────────────┘
```

**Server** is a relay with configurable room handlers. It does not interpret game messages.

**REST endpoints:**
- `POST /ticket` - get JWT (custom auth logic)
- `POST /rooms` - create room
- `GET /rooms` - list rooms
- `DELETE /rooms/{id}` - delete room

**Transport endpoints:**
- WebTransport (QUIC) - high performance, multi-stream
- WebSocket - fallback for Safari/older browsers

## Crates

| Crate | Purpose |
|-------|---------|
| `protocol` | Shared types: `UserContext` trait, room/channel IDs |
| `server` | REST (actix-web) + QUIC/WebSocket server |
| `client` | `RoomConnection` + `Channel`, works native and WASM |
| `helpers` | Typed actors, message framing |

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

Build the server with typed actors:

```rust
let server = Server::<MyUser>::builder()
    .jwt_secret(b"secret")
    .auth_handler(|req| async move {
        Ok(MyUser { user_id: 1, username: "alice".into() })
    })
    .room_handler(with_typed_actor::<MyUser, MyProtocol, _, _>(my_actor))
    .build()?
    .run()
    .await?;
```

Connect from client:

```rust
let conn = RoomConnection::connect(url, &ticket, room_id).await?;
let channel = conn.open_channel().await?;
channel.write(&data).await?;
let n = channel.read(&mut buf).await?;
```

## Room Lifecycle

- 5 min timeout for first connection
- 5 min timeout if room becomes empty  
- 1 hour max lifetime
- Creator can delete via REST
