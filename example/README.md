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
| POST | `/rooms` | Create room. Requires `Authorization: Bearer <ticket>` |
| GET | `/rooms` | List rooms |
| DELETE | `/rooms/{id}` | Delete room |

WebTransport at `https://127.0.0.1:4433`:
- `/lobby` - Live room list updates
- `/room/{id}` - Join room chat

## Web Client

Build the WASM client first:
```bash
cd /path/to/multiplayer-kit
wasm-pack build client --target web --out-dir ../example/web/pkg -- --no-default-features --features wasm
```

Then serve the web directory:
```bash
npx serve example/web
```

Open `http://localhost:3000` in Chrome/Edge (WebTransport required).

## CLI Commands

```
/rooms     - List available rooms
/create    - Create a new room
/join <id> - Join a room
/leave     - Leave current room
/quit      - Exit
<message>  - Send chat message (when in room)
```
