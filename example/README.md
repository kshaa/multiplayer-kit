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

| Method | Path | Description |
|--------|------|-------------|
| POST | `/ticket` | Get auth ticket. Body: `{"username": "name"}` |
| POST | `/rooms` | Create room. Body: `{"name": "Room Name"}`. Requires ticket. |
| GET | `/rooms` | List rooms |
| DELETE | `/rooms/{id}` | Delete room |
| POST | `/quickplay` | Auto-join or create room. Requires ticket. |
| GET | `/cert-hash` | Get WebTransport certificate hash |

WebTransport at `https://127.0.0.1:4433`:
- `/lobby` - Live room list updates
- `/room/{id}` - Join room chat

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
