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

**REST endpoints** (default `http://host:8080`):

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/ticket` | Get JWT ticket (custom auth logic) |
| `POST` | `/rooms` | Create room with config body |
| `GET` | `/rooms` | List rooms |
| `DELETE` | `/rooms/{id}` | Delete room (creator only) |
| `POST` | `/quickplay` | Auto-join or create room |
| `GET` | `/cert-hash` | Get WebTransport cert hash (for dev) |

**Transport endpoints (real-time):**

| Transport | Default | Path | Description |
|-----------|---------|------|-------------|
| WebTransport | `https://host:8080` (UDP) | `/room/{id}?ticket=...` | QUIC-based, multi-stream |
| WebTransport | `https://host:8080` (UDP) | `/lobby` | Live room list updates |
| WebSocket | `ws://host:8080` (TCP) | `/ws/room/{id}?ticket=...` | Fallback for Safari |

Same port, different protocols - TCP for HTTP/WebSocket, UDP for QUIC/WebTransport.

**Auth flow:** Both transports authenticate via query param. Server validates ticket before accepting. Each bi-stream (WebTransport) or connection (WebSocket) becomes a channel.

WebTransport preferred (lower latency, multiplexed streams). WebSocket fallback when unavailable.

### Development vs Production

**Development / LAN** (self-signed certs):
```
http://127.0.0.1:8080   → REST API
https://127.0.0.1:8080  → WebTransport (needs cert hash)
ws://127.0.0.1:8080     → WebSocket
```
Client fetches `/cert-hash` to trust the self-signed certificate.

For non-localhost (LAN, staging), configure the hostnames:
```rust
Server::builder()
    .self_signed_hosts(["192.168.1.50", "game.local"])
    // ...
```
Self-signed certs in production are possible but discouraged - browsers enforce max 14-day validity, requiring cert rotation and client refetch of `/cert-hash`. Use real TLS certs when possible.

**Production** (real TLS certs):
```
https://game.example.com:443  → REST API (TCP)
https://game.example.com:443  → WebTransport (UDP)
wss://game.example.com:443    → WebSocket (TCP)
```
- Same domain, same port (443)
- TCP and UDP don't conflict - OS distinguishes by protocol
- No cert hash needed - browsers trust the CA (e.g., Let's Encrypt)
- No reverse proxy needed - server handles TLS directly

```rust
Server::builder()
    .http_addr("0.0.0.0:443")
    .quic_addr("0.0.0.0:443")
    // QUIC TLS (required)
    .tls_cert("/etc/letsencrypt/live/game.example.com/fullchain.pem")
    .tls_key("/etc/letsencrypt/live/game.example.com/privkey.pem")
    // HTTP TLS (optional but recommended for production)
    .http_tls_cert("/etc/letsencrypt/live/game.example.com/fullchain.pem")
    .http_tls_key("/etc/letsencrypt/live/game.example.com/privkey.pem")
    // ...
```

Note: Traditional reverse proxies (nginx, HAProxy) don't support QUIC well. Deploy the server directly or use a QUIC-aware proxy.

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
let conn = RoomConnection::connect(url, &ticket, room_id, ConnectionConfig::default()).await?;
let channel = conn.open_channel().await?;
channel.write(&data).await?;
let n = channel.read(&mut buf).await?;
```

## Typed Actors & Channels

The `helpers` crate provides `with_typed_actor` - a wrapper that handles:

1. **Message framing** - Length-prefixed messages over raw byte streams
2. **Channel typing** - Each channel identifies itself with a type byte on first message
3. **User connection tracking** - Users are "connected" when all expected channels are established
4. **Automatic serialization** - Encode/decode via your `TypedProtocol` implementation

### Defining a Protocol

```rust
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum MyChannel {
    Chat,
    GameState,
}

pub enum MyEvent {
    Chat(ChatMessage),
    GameState(GameUpdate),
}

pub struct MyProtocol;

impl TypedProtocol for MyProtocol {
    type Channel = MyChannel;
    type Event = MyEvent;

    fn channel_to_id(channel: &Self::Channel) -> u8 {
        match channel {
            MyChannel::Chat => 0,
            MyChannel::GameState => 1,
        }
    }

    fn channel_from_id(id: u8) -> Option<Self::Channel> {
        match id {
            0 => Some(MyChannel::Chat),
            1 => Some(MyChannel::GameState),
            _ => None,
        }
    }

    fn all_channels() -> &'static [Self::Channel] {
        &[MyChannel::Chat, MyChannel::GameState]
    }

    fn decode(channel: Self::Channel, data: &[u8]) -> Result<Self::Event, DecodeError> {
        match channel {
            MyChannel::Chat => Ok(MyEvent::Chat(bincode::deserialize(data)?)),
            MyChannel::GameState => Ok(MyEvent::GameState(bincode::deserialize(data)?)),
        }
    }

    fn encode(event: &Self::Event) -> Result<(Self::Channel, Vec<u8>), EncodeError> {
        match event {
            MyEvent::Chat(msg) => Ok((MyChannel::Chat, bincode::serialize(msg)?)),
            MyEvent::GameState(upd) => Ok((MyChannel::GameState, bincode::serialize(upd)?)),
        }
    }
}
```

### Server Actor

The actor function is **synchronous and non-blocking**. Use `tokio::spawn` for async work:

```rust
fn my_actor(
    ctx: TypedContext<MyUser, MyProtocol>,
    event: TypedEvent<MyUser, MyProtocol>,
    config: Arc<MyRoomConfig>,
) {
    match event {
        TypedEvent::UserConnected(user) => {
            println!("{} joined", user.username);
        }
        TypedEvent::UserDisconnected(user) => {
            println!("{} left", user.username);
        }
        TypedEvent::Message { sender, channel, event } => {
            // Handle typed message
            ctx.broadcast(&event); // Send to all users (non-blocking)
        }
        TypedEvent::Internal(event) => {
            // Self-sent event (from spawned tasks via ctx.self_tx())
        }
        TypedEvent::Shutdown => {
            // Room closing, cleanup
        }
    }
}
```

**Context methods (all non-blocking):**
- `ctx.broadcast(&event)` - Send to all connected users
- `ctx.send_to_user(&user, &event)` - Send to specific user
- `ctx.self_tx()` - Get sender for internal events (use in spawned tasks)
- `ctx.connected_users()` - Get list of connected user IDs

### Client Actor

Clients can also use the typed actor pattern:

```rust
let handle = run_typed_client_actor(
    conn.into(),
    my_context,
    |ctx, event, _spawner| {
        match event {
            TypedClientEvent::Connected => { /* all channels established */ }
            TypedClientEvent::Message(msg) => { /* handle server message */ }
            TypedClientEvent::Internal(msg) => { /* handle self-sent event */ }
            TypedClientEvent::Disconnected => { /* connection lost */ }
        }
    },
    TokioSpawner,  // or WasmSpawner for WASM
);

// Send events via the handle:
handle.sender.send(my_event)?;
```

**Client context methods:**
- `ctx.send(&event)` - Send event to server (routed by type)
- `ctx.self_tx().send(event)` - Send event to self (for timers, async operations)
- `ctx.game_context()` - Access your custom context

### Connection Flow

1. Client opens N channels (one per `all_channels()`)
2. Each channel sends its type byte as first framed message
3. Server tracks pending channels per user
4. When all channels identified → `UserConnected` event
5. If any channel closes → all user channels closed, `UserDisconnected` event

## Quickplay

Quickplay auto-joins an existing room or creates a new one:

```
POST /quickplay
Authorization: Bearer <ticket>
Content-Type: application/json

{ "min_players": 2 }  // optional game-specific filter
```

**Server logic:**

1. Find rooms matching the filter (`RoomConfig::matches_quickplay`)
2. Exclude full rooms (`current_players >= max_players`)
3. If match found → return room ID
4. Otherwise, create new room using `RoomConfig::quickplay_default(request)`
5. Return new room ID

**Example config implementation:**

```rust
impl RoomConfig for MyRoomConfig {
    fn name(&self) -> &str { &self.name }
    fn max_players(&self) -> Option<usize> { Some(4) }

    fn matches_quickplay(&self, request: &serde_json::Value) -> bool {
        // Match by game mode
        request.get("mode")
            .and_then(|v| v.as_str())
            .map(|m| m == self.mode)
            .unwrap_or(true)
    }

    fn quickplay_default(request: &serde_json::Value) -> Option<Self> {
        let mode = request.get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("casual");
        Some(MyRoomConfig {
            name: format!("Quick {} #{}", mode, rand::random::<u16>()),
            mode: mode.to_string(),
        })
    }
}
```

**Client usage:**

```javascript
const result = await api.quickplay(ticket, { mode: "ranked" });
const roomId = result.room_id;
// Now connect to room
```

## Room Lifecycle

- 5 min timeout for first connection
- 5 min timeout if room becomes empty  
- 1 hour max lifetime
- Creator can delete via REST
