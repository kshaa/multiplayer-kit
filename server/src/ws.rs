//! WebSocket handlers for room connections.
//!
//! Each WebSocket connection represents one "channel" to a room.
//! Multiple WebSocket connections from the same user = multiple channels.

use crate::lobby::Lobby;
use crate::room::RoomManager;
use crate::ticket::TicketManager;
use actix::{Actor, ActorContext, AsyncContext, Handler, Message, StreamHandler};
use actix_web::{web, HttpRequest, HttpResponse};
use actix_web_actors::ws;
use multiplayer_kit_protocol::{ChannelId, RoomId, UserContext};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(30);

/// Shared state for WebSocket handlers.
pub struct WsState<T: UserContext> {
    pub room_manager: Arc<RoomManager<T>>,
    pub ticket_manager: Arc<TicketManager>,
    pub lobby: Arc<Lobby>,
}

/// WebSocket actor for a room channel.
pub struct RoomWsActor<T: UserContext + 'static> {
    user: T,
    room_id: RoomId,
    room_manager: Arc<RoomManager<T>>,
    lobby: Arc<Lobby>,
    /// Channel for receiving messages from the room actor.
    msg_rx: Option<mpsc::Receiver<Vec<u8>>>,
    /// Sender that gets passed to the room when channel is opened.
    msg_tx_for_room: Option<mpsc::Sender<Vec<u8>>>,
    /// The channel ID assigned by the room (set after channel opens).
    channel_id: Option<ChannelId>,
    last_heartbeat: Instant,
    authenticated: bool,
}

/// Message type for forwarding room messages to WebSocket.
#[derive(Message)]
#[rtype(result = "()")]
pub struct RoomMessage(pub Vec<u8>);

/// Message to set the channel ID after it's assigned.
#[derive(Message)]
#[rtype(result = "()")]
pub struct SetChannelId(pub ChannelId);

impl<T: UserContext + Unpin + 'static> RoomWsActor<T> {
    pub fn new(
        user: T,
        room_id: RoomId,
        room_manager: Arc<RoomManager<T>>,
        lobby: Arc<Lobby>,
    ) -> Self {
        let (msg_tx, msg_rx) = mpsc::channel::<Vec<u8>>(256);
        Self {
            user,
            room_id,
            room_manager,
            lobby,
            msg_rx: Some(msg_rx),
            msg_tx_for_room: Some(msg_tx),
            channel_id: None,
            last_heartbeat: Instant::now(),
            authenticated: false,
        }
    }

    fn heartbeat(&self, ctx: &mut ws::WebsocketContext<Self>) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx: &mut ws::WebsocketContext<Self>| {
            if Instant::now().duration_since(act.last_heartbeat) > CLIENT_TIMEOUT {
                tracing::debug!("WebSocket client heartbeat timeout");
                ctx.stop();
                return;
            }
            ctx.ping(b"");
        });
    }

    fn start_room_listener(&mut self, ctx: &mut ws::WebsocketContext<Self>) {
        if let Some(mut msg_rx) = self.msg_rx.take() {
            let addr = ctx.address();
            actix::spawn(async move {
                while let Some(msg) = msg_rx.recv().await {
                    if addr.try_send(RoomMessage(msg)).is_err() {
                        break;
                    }
                }
            });
        }
    }
}

impl<T: UserContext + Unpin + 'static> Actor for RoomWsActor<T> {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.heartbeat(ctx);

        // Open a channel in the room
        if let Some(room) = self.room_manager.get_room(self.room_id) {
            if let Some(msg_tx) = self.msg_tx_for_room.take() {
                let user = self.user.clone();
                let room_id = self.room_id;
                let lobby = Arc::clone(&self.lobby);
                let room_manager = Arc::clone(&self.room_manager);
                let addr = ctx.address();

                actix::spawn(async move {
                    // Open channel - this fires UserJoined (if first) and ChannelOpened events
                    let channel_id = room.open_channel(user, msg_tx).await;

                    // Tell the actor its channel ID
                    let _ = addr.try_send(SetChannelId(channel_id));

                    // Notify lobby
                    if let Some(info) = room_manager.get_room_info(room_id) {
                        lobby.notify_room_updated(info);
                    }

                    // Send join confirmation
                    let _ = addr.try_send(RoomMessage(b"joined".to_vec()));
                });

                self.authenticated = true;
                self.start_room_listener(ctx);
            }
        } else {
            tracing::warn!("Room {:?} not found for WebSocket connection", self.room_id);
            ctx.stop();
        }
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        // Close the channel
        if let Some(channel_id) = self.channel_id {
            let room_id = self.room_id;
            let room_manager = Arc::clone(&self.room_manager);
            let lobby = Arc::clone(&self.lobby);

            actix::spawn(async move {
                if let Some(room) = room_manager.get_room(room_id) {
                    // Close channel - fires ChannelClosed and UserLeft (if last channel)
                    room.close_channel(channel_id).await;

                    // Notify lobby
                    if let Some(info) = room_manager.get_room_info(room_id) {
                        lobby.notify_room_updated(info);
                    }
                }
            });
        }
    }
}

impl<T: UserContext + Unpin + 'static> Handler<RoomMessage> for RoomWsActor<T> {
    type Result = ();

    fn handle(&mut self, msg: RoomMessage, ctx: &mut Self::Context) {
        ctx.binary(msg.0);
    }
}

impl<T: UserContext + Unpin + 'static> Handler<SetChannelId> for RoomWsActor<T> {
    type Result = ();

    fn handle(&mut self, msg: SetChannelId, _ctx: &mut Self::Context) {
        self.channel_id = Some(msg.0);
    }
}

impl<T: UserContext + Unpin + 'static> StreamHandler<Result<ws::Message, ws::ProtocolError>>
    for RoomWsActor<T>
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
                // Forward text as bytes to room actor
                if self.authenticated {
                    if let Some(channel_id) = self.channel_id {
                        let payload = text.into_bytes().to_vec();
                        let user = self.user.clone();
                        let room_manager = Arc::clone(&self.room_manager);
                        let room_id = self.room_id;

                        actix::spawn(async move {
                            if let Some(room) = room_manager.get_room(room_id) {
                                room.send_message(user, channel_id, payload).await;
                            }
                        });
                    }
                }
            }
            Ok(ws::Message::Binary(data)) => {
                // Forward binary to room actor
                if self.authenticated {
                    if let Some(channel_id) = self.channel_id {
                        let payload = data.to_vec();
                        let user = self.user.clone();
                        let room_manager = Arc::clone(&self.room_manager);
                        let room_id = self.room_id;

                        actix::spawn(async move {
                            if let Some(room) = room_manager.get_room(room_id) {
                                room.send_message(user, channel_id, payload).await;
                            }
                        });
                    }
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
pub async fn room_ws<T: UserContext + Unpin + 'static>(
    req: HttpRequest,
    stream: web::Payload,
    path: web::Path<(u64,)>,
    query: web::Query<WsQuery>,
    state: web::Data<WsState<T>>,
) -> Result<HttpResponse, actix_web::Error> {
    let room_id = RoomId(path.0);
    let ticket = &query.ticket;

    // Validate ticket
    let user: T = state
        .ticket_manager
        .validate(ticket)
        .map_err(|e| actix_web::error::ErrorUnauthorized(format!("{:?}", e)))?;

    tracing::info!(
        "WebSocket room connection for {:?} by user {:?}",
        room_id,
        user.id()
    );

    // Check room exists
    if state.room_manager.get_room(room_id).is_none() {
        return Err(actix_web::error::ErrorNotFound("Room not found"));
    }

    let actor = RoomWsActor::new(
        user,
        room_id,
        Arc::clone(&state.room_manager),
        Arc::clone(&state.lobby),
    );

    ws::start(actor, &req, stream)
}

#[derive(serde::Deserialize)]
pub struct WsQuery {
    pub ticket: String,
}
