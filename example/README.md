# Chat Example

Simple chat app demonstrating multiplayer-kit.

## Run

Start the server:
```bash
cargo run --bin chat-server
```

Start the CLI client (in another terminal):
```bash
cargo run --bin chat-cli
```

## Endpoints

REST API at `http://127.0.0.1:8080`. Ticket is passed via `Authorization: Bearer <ticket>` header.

| Method | Path | Description |
|--------|------|-------------|
| POST | `/ticket` | Get auth ticket. Body: `{"username": "name"}` |
| POST | `/rooms` | Create room. Body: `{"name": "Room Name"}`. Requires ticket. |
| GET | `/rooms` | List rooms. Requires ticket. |
| DELETE | `/rooms/{id}` | Delete room. Requires ticket (creator only). |
| POST | `/quickplay` | Auto-join or create room. Requires ticket. |
| GET | `/cert-hash` | Get WebTransport certificate hash (no auth). |

WebTransport (QUIC/UDP) at `https://127.0.0.1:8080`:
- `/room/{id}?ticket=...` - Join room
- `/lobby` - Live room updates (auth via bi-stream)

WebSocket (TCP) at `ws://127.0.0.1:8080`:
- `/ws/room/{id}?ticket=...` - Join room (fallback for Safari)
- `/ws/lobby?ticket=...` - Live room updates (fallback for Safari)

## Web Client

Build the WASM chat-client first:
```bash
cd example/chat-client
wasm-pack build --target web --out-dir ../chat-web/pkg --no-default-features --features wasm
```

Then serve the web directory:
```bash
npx serve example/chat-web
```

Open `http://localhost:3000` in Chrome/Edge (WebTransport required).

## CLI Commands

```
/rooms           - List available rooms
/create <name>   - Create a new room
/join <id>       - Join a room
/quickplay       - Auto-join or create a room
/leave           - Leave current room
/quit            - Exit
<message>        - Send chat message (when in room)
```
