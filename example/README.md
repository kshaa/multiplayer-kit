# Chat Example

Simple chat app demonstrating multiplayer-kit.

## Run

Start the server:
```bash
cargo run --bin chat-server
```

Start the CLI client (in another terminal):
```bash
cargo run --bin chat-client
```

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| POST | `/ticket` | Get auth ticket. Body: `{"username": "name"}` |
| POST | `/rooms` | Create room. Requires `Authorization: Bearer <ticket>` |
| GET | `/rooms` | List rooms |
| DELETE | `/rooms/{id}` | Delete room |

WebTransport at `https://127.0.0.1:4433`:
- `/room/{id}` - Join room chat

## Web Client

Open `web/index.html` in a browser. You'll need to:
1. Accept the self-signed certificate at https://127.0.0.1:4433
2. Serve the HTML over HTTP (or use a local file server) for fetch to work

## CLI Commands

```
/rooms     - List available rooms
/create    - Create a new room
/join <id> - Join a room
/leave     - Leave current room
/quit      - Exit
<message>  - Send chat message (when in room)
```
