//! Room connection management.

use crate::transport::webtransport::WebTransportConnection;
use crate::transport::websocket::WebSocketConnection;
use crate::{is_webtransport_supported, ClientError, Transport};
use multiplayer_kit_protocol::RoomId;

use super::channel::{Channel, ChannelInner};
use super::{ConnectionConfig, RoomConnectionLike};

/// A connection to a room. Can open multiple channels.
pub struct RoomConnection {
    inner: ConnectionInner,
    room_id: RoomId,
    transport: Transport,
}

enum ConnectionInner {
    WebTransport(WebTransportConnection),
    WebSocket(WebSocketConnection),
}

impl RoomConnection {
    /// Connect to a room.
    pub async fn connect(
        url: &str,
        ticket: &str,
        room_id: RoomId,
        config: ConnectionConfig,
    ) -> Result<Self, ClientError> {
        match config.transport {
            Transport::Auto => {
                Self::connect_auto(url, ticket, room_id, config.cert_hash, config.validate_certs).await
            }
            Transport::WebTransport => {
                Self::connect_webtransport(url, ticket, room_id, config.cert_hash, config.validate_certs).await
            }
            Transport::WebSocket => Self::connect_websocket(url, ticket, room_id).await,
        }
    }

    async fn connect_auto(
        url: &str,
        ticket: &str,
        room_id: RoomId,
        cert_hash: Option<String>,
        validate_certs: bool,
    ) -> Result<Self, ClientError> {
        if is_webtransport_supported() {
            match Self::connect_webtransport(url, ticket, room_id, cert_hash.clone(), validate_certs).await {
                Ok(conn) => return Ok(conn),
                Err(_e) => {
                    #[cfg(target_arch = "wasm32")]
                    web_sys::console::warn_1(
                        &format!("WebTransport failed, falling back to WebSocket: {:?}", _e).into(),
                    );
                }
            }
        }
        Self::connect_websocket(url, ticket, room_id).await
    }

    async fn connect_webtransport(
        url: &str,
        ticket: &str,
        room_id: RoomId,
        cert_hash: Option<String>,
        validate_certs: bool,
    ) -> Result<Self, ClientError> {
        let room_url = format!("{}/room/{}?ticket={}", url, room_id.0, ticket);
        let conn =
            WebTransportConnection::connect(&room_url, cert_hash.as_deref(), validate_certs).await?;

        Ok(Self {
            inner: ConnectionInner::WebTransport(conn),
            room_id,
            transport: Transport::WebTransport,
        })
    }

    async fn connect_websocket(url: &str, ticket: &str, room_id: RoomId) -> Result<Self, ClientError> {
        // Convert http(s):// to ws(s)://
        let ws_url = url
            .replace("https://", "wss://")
            .replace("http://", "ws://");

        let base_url = format!("{}/ws/room/{}?ticket={}", ws_url, room_id.0, ticket);
        let conn = WebSocketConnection::connect(&base_url).await?;

        Ok(Self {
            inner: ConnectionInner::WebSocket(conn),
            room_id,
            transport: Transport::WebSocket,
        })
    }

    /// Open a new channel (persistent stream).
    pub async fn open_channel(&self) -> Result<Channel, ClientError> {
        match &self.inner {
            ConnectionInner::WebTransport(conn) => {
                let stream = conn.open_bi().await?;
                Ok(Channel {
                    inner: ChannelInner::WebTransport(stream),
                })
            }
            ConnectionInner::WebSocket(conn) => {
                let stream = conn.open_stream().await?;
                Ok(Channel {
                    inner: ChannelInner::WebSocket(stream),
                })
            }
        }
    }

    /// Get the actual transport being used.
    pub fn transport(&self) -> Transport {
        self.transport
    }

    /// Get the room ID.
    pub fn room_id(&self) -> RoomId {
        self.room_id
    }

    /// Check if connected.
    pub fn is_connected(&self) -> bool {
        match &self.inner {
            ConnectionInner::WebTransport(conn) => conn.is_connected(),
            ConnectionInner::WebSocket(conn) => conn.is_connected(),
        }
    }
}

impl RoomConnectionLike for RoomConnection {
    type Channel = Channel;

    async fn open_channel(&self) -> Result<Self::Channel, ClientError> {
        RoomConnection::open_channel(self).await
    }
}
