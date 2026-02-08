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
    .tls_cert("/etc/letsencrypt/live/game.example.com/fullchain.pem")
    .tls_key("/etc/letsencrypt/live/game.example.com/privkey.pem")
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
