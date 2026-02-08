//! WebSocket handlers for room connections.
//!
//! Each WebSocket connection represents one "channel" to a room.
//! Multiple WebSocket connections from the same user = multiple channels.

use crate::lobby::Lobby;
use crate::room::RoomManager;
use crate::ticket::TicketManager;
use actix::{Actor, ActorContext, AsyncContext, Handler, Message, StreamHandler};
use actix_web::{HttpRequest, HttpResponse, web};
use actix_web_actors::ws;
use multiplayer_kit_protocol::{ChannelId, RoomConfig, RoomId, UserContext};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(30);

/// Shared state for WebSocket handlers.
pub struct WsState<T: UserContext, C: RoomConfig> {
    pub room_manager: Arc<RoomManager<T, C>>,
    pub ticket_manager: Arc<TicketManager>,
    pub lobby: Arc<Lobby>,
}

/// WebSocket actor for a room channel.
pub struct RoomWsActor<T: UserContext + 'static, C: RoomConfig + 'static> {
    room_id: RoomId,
    room_manager: Arc<RoomManager<T, C>>,
    lobby: Arc<Lobby>,
    /// Channel ID (set after channel opens).
    channel_id: Option<ChannelId>,
    /// Send data to room handler.
    read_tx: Option<mpsc::Sender<Vec<u8>>>,
    /// Receive data from room handler.
    write_rx: Option<mpsc::Receiver<Vec<u8>>>,
    /// Close signal.
    close_rx: Option<oneshot::Receiver<()>>,
    last_heartbeat: Instant,
}

/// Message type for forwarding room messages to WebSocket.
#[derive(Message)]
#[rtype(result = "()")]
pub struct RoomMessage(pub Vec<u8>);

/// Message for channel close signal.
#[derive(Message)]
#[rtype(result = "()")]
pub struct ChannelClosed;

impl<T: UserContext + Unpin + 'static, C: RoomConfig + 'static> RoomWsActor<T, C> {
    pub fn new(
        room_id: RoomId,
        room_manager: Arc<RoomManager<T, C>>,
        lobby: Arc<Lobby>,
        channel_id: ChannelId,
        read_tx: mpsc::Sender<Vec<u8>>,
        write_rx: mpsc::Receiver<Vec<u8>>,
        close_rx: oneshot::Receiver<()>,
    ) -> Self {
        Self {
            room_id,
            room_manager,
            lobby,
            channel_id: Some(channel_id),
            read_tx: Some(read_tx),
            write_rx: Some(write_rx),
            close_rx: Some(close_rx),
            last_heartbeat: Instant::now(),
        }
    }

    fn heartbeat(&self, ctx: &mut ws::WebsocketContext<Self>) {
        ctx.run_interval(
            HEARTBEAT_INTERVAL,
            |act, ctx: &mut ws::WebsocketContext<Self>| {
                if Instant::now().duration_since(act.last_heartbeat) > CLIENT_TIMEOUT {
                    tracing::debug!("WebSocket client heartbeat timeout");
                    ctx.stop();
                    return;
                }
                ctx.ping(b"");
            },
        );
    }

    fn start_listeners(&mut self, ctx: &mut ws::WebsocketContext<Self>) {
        // Listen for messages from room handler
        if let Some(mut write_rx) = self.write_rx.take() {
            let addr = ctx.address();
            actix::spawn(async move {
                while let Some(msg) = write_rx.recv().await {
                    if addr.try_send(RoomMessage(msg)).is_err() {
                        break;
                    }
                }
            });
        }

        // Listen for close signal
        if let Some(close_rx) = self.close_rx.take() {
            let addr = ctx.address();
            actix::spawn(async move {
                let _ = close_rx.await;
                let _ = addr.try_send(ChannelClosed);
            });
        }
    }
}

impl<T: UserContext + Unpin + 'static, C: RoomConfig + 'static> Actor for RoomWsActor<T, C> {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.heartbeat(ctx);
        self.start_listeners(ctx);
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        // Close the channel
        if let Some(channel_id) = self.channel_id.take() {
            let room_id = self.room_id;
            let room_manager = Arc::clone(&self.room_manager);
            let lobby = Arc::clone(&self.lobby);

            actix::spawn(async move {
                room_manager.close_channel(room_id, channel_id).await;

                // Notify lobby
                if let Some(info) = room_manager.get_room_info(room_id) {
                    lobby.notify_room_updated(info);
                }
            });
        }
    }
}

impl<T: UserContext + Unpin + 'static, C: RoomConfig + 'static> Handler<RoomMessage>
    for RoomWsActor<T, C>
{
    type Result = ();

    fn handle(&mut self, msg: RoomMessage, ctx: &mut Self::Context) {
        ctx.binary(msg.0);
    }
}

impl<T: UserContext + Unpin + 'static, C: RoomConfig + 'static> Handler<ChannelClosed>
    for RoomWsActor<T, C>
{
    type Result = ();

    fn handle(&mut self, _msg: ChannelClosed, ctx: &mut Self::Context) {
        ctx.stop();
    }
}

impl<T: UserContext + Unpin + 'static, C: RoomConfig + 'static>
    StreamHandler<Result<ws::Message, ws::ProtocolError>> for RoomWsActor<T, C>
{
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Ping(msg)) => {
                self.last_heartbeat = Instant::now();
                ctx.pong(&msg);
            }
            Ok(ws::Message::Pong(_)) => {
                self.last_heartbeat = Instant::now();
            }
            Ok(ws::Message::Text(text)) => {
                // Forward text as bytes to room handler
                if let Some(read_tx) = &self.read_tx {
                    let payload = text.into_bytes().to_vec();
                    let tx = read_tx.clone();
                    actix::spawn(async move {
                        let _ = tx.send(payload).await;
                    });
                }
            }
            Ok(ws::Message::Binary(data)) => {
                // Forward binary to room handler
                if let Some(read_tx) = &self.read_tx {
                    let payload = data.to_vec();
                    let tx = read_tx.clone();
                    actix::spawn(async move {
                        let _ = tx.send(payload).await;
                    });
                }
            }
            Ok(ws::Message::Close(reason)) => {
                tracing::debug!("WebSocket close: {:?}", reason);
                ctx.stop();
            }
            _ => (),
        }
    }
}

/// HTTP handler to upgrade to WebSocket for room channels.
pub async fn room_ws<T: UserContext + Unpin + 'static, C: RoomConfig + 'static>(
    req: HttpRequest,
    stream: web::Payload,
    path: web::Path<(u64,)>,
    query: web::Query<WsQuery>,
    state: web::Data<WsState<T, C>>,
) -> Result<HttpResponse, actix_web::Error> {
    let room_id = RoomId(path.0);
    let ticket = &query.ticket;

    // Validate ticket
    let user: T = state
        .ticket_manager
        .validate(ticket)
        .map_err(|_| actix_web::error::ErrorUnauthorized("Invalid ticket"))?;

    tracing::info!(
        "WebSocket room connection for {:?} by user {:?}",
        room_id,
        user.id()
    );

    // Open channel via room manager
    let channel_result = state.room_manager.open_channel(room_id, user).await;

    let Some((channel_id, read_tx, write_rx, close_rx)) = channel_result else {
        return Err(actix_web::error::ErrorNotFound("Room not found or closed"));
    };

    // Notify lobby
    if let Some(info) = state.room_manager.get_room_info(room_id) {
        state.lobby.notify_room_updated(info);
    }

    let actor = RoomWsActor::new(
        room_id,
        Arc::clone(&state.room_manager),
        Arc::clone(&state.lobby),
        channel_id,
        read_tx,
        write_rx,
        close_rx,
    );

    ws::start(actor, &req, stream)
}

#[derive(serde::Deserialize)]
pub struct WsQuery {
    pub ticket: String,
}

// ============================================================================
// Lobby WebSocket
// ============================================================================

use multiplayer_kit_protocol::LobbyEvent;
use tokio::sync::broadcast;

/// WebSocket actor for lobby updates.
pub struct LobbyWsActor<T: UserContext + 'static, C: RoomConfig + 'static> {
    room_manager: Arc<RoomManager<T, C>>,
    lobby_rx: Option<broadcast::Receiver<LobbyEvent>>,
    last_heartbeat: Instant,
}

/// Message for lobby events.
#[derive(Message)]
#[rtype(result = "()")]
pub struct LobbyMessage(pub Vec<u8>);

impl<T: UserContext + Unpin + 'static, C: RoomConfig + 'static> LobbyWsActor<T, C> {
    pub fn new(
        room_manager: Arc<RoomManager<T, C>>,
        lobby_rx: broadcast::Receiver<LobbyEvent>,
    ) -> Self {
        Self {
            room_manager,
            lobby_rx: Some(lobby_rx),
            last_heartbeat: Instant::now(),
        }
    }

    fn heartbeat(&self, ctx: &mut ws::WebsocketContext<Self>) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            if Instant::now().duration_since(act.last_heartbeat) > CLIENT_TIMEOUT {
                tracing::debug!("Lobby WebSocket client heartbeat timeout");
                ctx.stop();
                return;
            }
            ctx.ping(b"");
        });
    }

    fn start_lobby_listener(&mut self, ctx: &mut ws::WebsocketContext<Self>) {
        if let Some(mut lobby_rx) = self.lobby_rx.take() {
            let addr = ctx.address();
            actix::spawn(async move {
                loop {
                    match lobby_rx.recv().await {
                        Ok(event) => {
                            if let Ok(bytes) = bincode::serialize(&event) {
                                if addr.try_send(LobbyMessage(bytes)).is_err() {
                                    break;
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    }
                }
            });
        }
    }
}

impl<T: UserContext + Unpin + 'static, C: RoomConfig + 'static> Actor for LobbyWsActor<T, C> {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.heartbeat(ctx);

        // Send initial snapshot
        let rooms = self.room_manager.get_all_rooms();
        let snapshot = LobbyEvent::Snapshot(rooms);
        if let Ok(bytes) = bincode::serialize(&snapshot) {
            ctx.binary(bytes);
        }

        self.start_lobby_listener(ctx);
    }
}

impl<T: UserContext + Unpin + 'static, C: RoomConfig + 'static> Handler<LobbyMessage>
    for LobbyWsActor<T, C>
{
    type Result = ();

    fn handle(&mut self, msg: LobbyMessage, ctx: &mut Self::Context) {
        ctx.binary(msg.0);
    }
}

impl<T: UserContext + Unpin + 'static, C: RoomConfig + 'static>
    StreamHandler<Result<ws::Message, ws::ProtocolError>> for LobbyWsActor<T, C>
{
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Ping(msg)) => {
                self.last_heartbeat = Instant::now();
                ctx.pong(&msg);
            }
            Ok(ws::Message::Pong(_)) => {
                self.last_heartbeat = Instant::now();
            }
            Ok(ws::Message::Close(reason)) => {
                tracing::debug!("Lobby WebSocket close: {:?}", reason);
                ctx.stop();
            }
            _ => (),
        }
    }
}

/// HTTP handler to upgrade to WebSocket for lobby.
pub async fn lobby_ws<T: UserContext + Unpin + 'static, C: RoomConfig + 'static>(
    req: HttpRequest,
    stream: web::Payload,
    query: web::Query<WsQuery>,
    state: web::Data<WsState<T, C>>,
) -> Result<HttpResponse, actix_web::Error> {
    let ticket = &query.ticket;

    // Validate ticket
    let _user: T = state
        .ticket_manager
        .validate(ticket)
        .map_err(|_| actix_web::error::ErrorUnauthorized("Invalid ticket"))?;

    tracing::info!("WebSocket lobby connection");

    let lobby_rx = state.lobby.subscribe();
    let actor = LobbyWsActor::new(Arc::clone(&state.room_manager), lobby_rx);

    ws::start(actor, &req, stream)
}
