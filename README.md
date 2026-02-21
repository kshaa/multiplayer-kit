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

Build the server with actors:

```rust
let server = Server::<MyUser, MyRoomConfig, MyServerContext>::builder()
    .jwt_secret(b"secret")
    .context(my_context)
    .auth_handler(|req, ctx| async move {
        Ok(MyUser { user_id: 1, username: "alice".into() })
    })
    .room_handler(with_server_actor::<MyActor, MyUser, MyMessage, MyRoomConfig, MyServerContext>())
    .build()?;

server.run().await?;
```

Connect from client:

```rust
let conn = RoomConnection::connect(url, &ticket, room_id, ConnectionConfig::default()).await?;
let channel = conn.open_channel().await?;
channel.write(&data).await?;
let n = channel.read(&mut buf).await?;
```

## Actors & Channels

The `helpers` crate provides a generic `Actor` trait and glue code that handles:

1. **Message framing** - Length-prefixed messages over raw byte streams
2. **Channel typing** - Each channel identifies itself with a type byte on first message
3. **User connection tracking** - Users are "connected" when all expected channels are established
4. **Automatic serialization** - Encode/decode via your `ChannelMessage` implementation

### Defining Messages

Messages implement `ChannelMessage` directly (no separate protocol type):

```rust
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum MyChannel {
    Chat,
    GameState,
}

#[derive(Clone, Debug)]
pub enum MyMessage {
    Chat(ChatMessage),
    GameState(GameUpdate),
}

impl ChannelMessage for MyMessage {
    type Channel = MyChannel;

    fn channel(&self) -> Option<MyChannel> {
        match self {
            MyMessage::Chat(_) => Some(MyChannel::Chat),
            MyMessage::GameState(_) => Some(MyChannel::GameState),
        }
    }

    fn all_channels() -> &'static [MyChannel] {
        &[MyChannel::Chat, MyChannel::GameState]
    }

    fn channel_to_id(channel: MyChannel) -> u8 {
        match channel {
            MyChannel::Chat => 0,
            MyChannel::GameState => 1,
        }
    }

    fn channel_from_id(id: u8) -> Option<MyChannel> {
        match id {
            0 => Some(MyChannel::Chat),
            1 => Some(MyChannel::GameState),
            _ => None,
        }
    }

    fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        match self {
            MyMessage::Chat(msg) => bincode::serialize(msg).map_err(|e| EncodeError::Serialize(e.to_string())),
            MyMessage::GameState(upd) => bincode::serialize(upd).map_err(|e| EncodeError::Serialize(e.to_string())),
        }
    }

    fn decode(channel: MyChannel, data: &[u8]) -> Result<Self, DecodeError> {
        match channel {
            MyChannel::Chat => Ok(MyMessage::Chat(bincode::deserialize(data).map_err(|e| DecodeError::Deserialize(e.to_string()))?)),
            MyChannel::GameState => Ok(MyMessage::GameState(bincode::deserialize(data).map_err(|e| DecodeError::Deserialize(e.to_string()))?)),
        }
    }
}
```

### Server Actor

Server actors implement the `Actor` trait. The `handle` method is **synchronous** - outputs are collected via a type-safe `Sink`:

```rust
struct MyServerActor;

impl ActorProtocol for MyServerActor {
    type ActorId = ServerSource<MyUser>;
    type Message = ServerMessage<MyMessage>;
}

impl<S> Actor<S> for MyServerActor
where
    S: Sink<UserTarget<MyUser, MyMessage>> + Sink<SelfTarget<MyMessage>>,
{
    type Config = ServerActorConfig<MyRoomConfig>;
    type State = MyState;
    type Extras = MyExtras;

    fn startup(config: Self::Config) -> Self::State {
        MyState { /* initial state */ }
    }

    fn handle(
        state: &mut Self::State,
        extras: &mut Self::Extras,
        message: AddressedMessage<Self::ActorId, Self::Message>,
        sink: &mut S,
    ) {
        match (&message.from, &message.content) {
            (ServerSource::User(user), ServerMessage::UserConnected) => {
                // User joined
            }
            (ServerSource::User(user), ServerMessage::UserDisconnected) => {
                // User left
            }
            (ServerSource::User(user), ServerMessage::Message(msg)) => {
                // Handle message, send responses via sink
                Sink::<UserTarget<MyUser, MyMessage>>::send(
                    sink,
                    UserDestination::Broadcast,
                    response_msg,
                );
            }
            (ServerSource::Internal, ServerMessage::Message(msg)) => {
                // Self-scheduled message
            }
            _ => {}
        }
    }

    fn shutdown(state: &mut Self::State, extras: &mut Self::Extras) {
        // Cleanup
    }
}
```

**Sink destinations:**
- `UserDestination::Broadcast` - Send to all connected users
- `UserDestination::User(user)` - Send to specific user
- `SelfTarget` - Schedule message to self (for timers, async results)

### Client Actor

Clients also implement the `Actor` trait:

```rust
struct MyClientActor;

impl ActorProtocol for MyClientActor {
    type ActorId = ClientSource;
    type Message = ClientMessage<MyMessage>;
}

impl<S> Actor<S> for MyClientActor
where
    S: Sink<ServerTarget<MyMessage>> + Sink<SelfTarget<MyMessage>>,
{
    type Config = MyClientConfig;
    type State = MyClientState;
    type Extras = MyClientExtras;

    fn startup(config: Self::Config) -> Self::State {
        MyClientState { /* initial state */ }
    }

    fn handle(
        state: &mut Self::State,
        extras: &mut Self::Extras,
        message: AddressedMessage<Self::ActorId, Self::Message>,
        sink: &mut S,
    ) {
        match (&message.from, &message.content) {
            (ClientSource::Server, ClientMessage::Connected) => {
                // All channels established
            }
            (ClientSource::Server, ClientMessage::Message(msg)) => {
                // Handle server message
            }
            (ClientSource::Server, ClientMessage::Disconnected) => {
                // Connection lost
            }
            (ClientSource::Internal, ClientMessage::Message(msg)) => {
                // Self-scheduled message
            }
            _ => {}
        }
    }

    fn shutdown(state: &mut Self::State, extras: &mut Self::Extras) {}
}
```

Start the actor:

```rust
let handle = with_client_actor::<MyClientActor, MyMessage, MyContext, TokioSpawner>(
    conn,
    my_context,
    TokioSpawner,  // or WasmSpawner for WASM
);

// Send messages via handle:
handle.sender.send(my_message)?;
```

**Sink destinations:**
- `ServerTarget` - Send to server
- `SelfTarget` - Schedule message to self

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
