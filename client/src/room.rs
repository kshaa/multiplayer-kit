//! Room connection and channel management.
//!
//! A `RoomConnection` represents an authenticated connection to a room.
//! From it, you can open multiple `Channel`s - each is a persistent
//! bidirectional byte stream.
//!
//! # Transport
//! - WebTransport: Each channel is a QUIC bidirectional stream
//! - WebSocket: Each channel is a separate WebSocket connection

use crate::error::{ConnectionError, DisconnectReason, ReceiveError, SendError};
use crate::ClientError;
use multiplayer_kit_protocol::RoomId;

// ============================================================================
// Native implementation
// ============================================================================

#[cfg(feature = "native")]
mod native {
    use super::*;
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    const STATE_DISCONNECTED: u8 = 0;
    const STATE_CONNECTING: u8 = 1;
    const STATE_CONNECTED: u8 = 2;
    const STATE_LOST: u8 = 3;

    /// Transport type for the connection.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Transport {
        WebTransport,
        WebSocket,
    }

    /// Connection configuration.
    #[derive(Debug, Clone)]
    pub struct ConnectionConfig {
        /// Preferred transport (will fallback if unavailable).
        pub transport: Transport,
        /// Base64-encoded SHA-256 cert hash for self-signed certs (WebTransport only).
        pub cert_hash: Option<String>,
    }

    impl Default for ConnectionConfig {
        fn default() -> Self {
            Self {
                transport: Transport::WebTransport,
                cert_hash: None,
            }
        }
    }

    /// A connection to a room. Can open multiple channels.
    pub struct RoomConnection {
        transport: Transport,
        room_id: RoomId,
        ticket: String,
        // WebTransport connection (if using WebTransport)
        wt_connection: Option<wtransport::Connection>,
        // WebSocket base URL (if using WebSocket)
        ws_base_url: Option<String>,
        state: Arc<AtomicU8>,
    }

    impl RoomConnection {
        /// Connect to a room using default config (WebTransport).
        pub async fn connect(url: &str, ticket: &str, room_id: RoomId) -> Result<Self, ClientError> {
            Self::connect_with_config(url, ticket, room_id, ConnectionConfig::default()).await
        }

        /// Connect to a room with custom config.
        pub async fn connect_with_config(
            url: &str,
            ticket: &str,
            room_id: RoomId,
            config: ConnectionConfig,
        ) -> Result<Self, ClientError> {
            match config.transport {
                Transport::WebTransport => {
                    Self::connect_webtransport(url, ticket, room_id, config.cert_hash).await
                }
                Transport::WebSocket => {
                    Self::connect_websocket(url, ticket, room_id).await
                }
            }
        }

        async fn connect_webtransport(
            url: &str,
            ticket: &str,
            room_id: RoomId,
            _cert_hash: Option<String>,
        ) -> Result<Self, ClientError> {
            let wt_config = wtransport::ClientConfig::builder()
                .with_bind_default()
                .with_no_cert_validation()
                .keep_alive_interval(Some(std::time::Duration::from_secs(5)))
                .max_idle_timeout(Some(std::time::Duration::from_secs(30)))
                .expect("valid idle timeout")
                .build();

            let endpoint = wtransport::Endpoint::client(wt_config).map_err(|e| {
                ClientError::Connection(ConnectionError::Transport(e.to_string()))
            })?;

            let room_url = format!("{}/room/{}", url, room_id.0);
            let connection = endpoint.connect(&room_url).await.map_err(|e| {
                ClientError::Connection(ConnectionError::Transport(e.to_string()))
            })?;

            // Auth: open first bi-stream, send ticket, get confirmation
            let (mut send, mut recv) = connection
                .open_bi()
                .await
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(e.to_string())))?
                .await
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(e.to_string())))?;

            send.write_all(ticket.as_bytes()).await.map_err(|e| {
                ClientError::Connection(ConnectionError::Transport(e.to_string()))
            })?;
            send.finish().await.map_err(|e| {
                ClientError::Connection(ConnectionError::Transport(e.to_string()))
            })?;

            // Read confirmation
            let mut confirm = vec![0u8; 64];
            let n = recv.read(&mut confirm).await.map_err(|e| {
                ClientError::Connection(ConnectionError::Transport(e.to_string()))
            })?;

            if n.is_none() || n == Some(0) {
                return Err(ClientError::Connection(ConnectionError::ServerRejected(
                    "No join confirmation".to_string(),
                )));
            }

            Ok(Self {
                transport: Transport::WebTransport,
                room_id,
                ticket: ticket.to_string(),
                wt_connection: Some(connection),
                ws_base_url: None,
                state: Arc::new(AtomicU8::new(STATE_CONNECTED)),
            })
        }

        async fn connect_websocket(
            url: &str,
            ticket: &str,
            room_id: RoomId,
        ) -> Result<Self, ClientError> {
            // Convert http(s):// to ws(s)://
            let ws_url = url
                .replace("https://", "wss://")
                .replace("http://", "ws://");
            
            let base_url = format!("{}/ws/room/{}?ticket={}", ws_url, room_id.0, ticket);

            // Just verify we can parse the URL; actual connection happens in open_channel
            url::Url::parse(&base_url).map_err(|e| {
                ClientError::Connection(ConnectionError::InvalidUrl(e.to_string()))
            })?;

            Ok(Self {
                transport: Transport::WebSocket,
                room_id,
                ticket: ticket.to_string(),
                wt_connection: None,
                ws_base_url: Some(base_url),
                state: Arc::new(AtomicU8::new(STATE_CONNECTED)),
            })
        }

        /// Open a new channel (persistent stream).
        pub async fn open_channel(&self) -> Result<Channel, ClientError> {
            match self.transport {
                Transport::WebTransport => self.open_wt_channel().await,
                Transport::WebSocket => self.open_ws_channel().await,
            }
        }

        async fn open_wt_channel(&self) -> Result<Channel, ClientError> {
            let connection = self.wt_connection.as_ref().ok_or_else(|| {
                ClientError::Connection(ConnectionError::Transport("Not connected".to_string()))
            })?;

            let (send, recv) = connection
                .open_bi()
                .await
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(e.to_string())))?
                .await
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(e.to_string())))?;

            Ok(Channel {
                inner: ChannelInner::WebTransport {
                    send: Arc::new(Mutex::new(send)),
                    recv: Arc::new(Mutex::new(recv)),
                },
                state: Arc::new(AtomicU8::new(STATE_CONNECTED)),
            })
        }

        async fn open_ws_channel(&self) -> Result<Channel, ClientError> {
            let url = self.ws_base_url.as_ref().ok_or_else(|| {
                ClientError::Connection(ConnectionError::Transport("Not connected".to_string()))
            })?;

            let (ws_stream, _) = tokio_tungstenite::connect_async(url).await.map_err(|e| {
                ClientError::Connection(ConnectionError::Transport(e.to_string()))
            })?;

            // Wait for join confirmation
            use futures::StreamExt;
            let (write, mut read) = ws_stream.split();
            
            // Read first message (should be "joined")
            if let Some(msg) = read.next().await {
                match msg {
                    Ok(tokio_tungstenite::tungstenite::Message::Binary(_)) |
                    Ok(tokio_tungstenite::tungstenite::Message::Text(_)) => {
                        // Got confirmation
                    }
                    _ => {
                        return Err(ClientError::Connection(ConnectionError::ServerRejected(
                            "No join confirmation".to_string(),
                        )));
                    }
                }
            } else {
                return Err(ClientError::Connection(ConnectionError::ServerRejected(
                    "Connection closed before confirmation".to_string(),
                )));
            }

            Ok(Channel {
                inner: ChannelInner::WebSocket {
                    write: Arc::new(Mutex::new(write)),
                    read: Arc::new(Mutex::new(read)),
                },
                state: Arc::new(AtomicU8::new(STATE_CONNECTED)),
            })
        }

        /// Get the transport type.
        pub fn transport(&self) -> Transport {
            self.transport
        }

        /// Get the room ID.
        pub fn room_id(&self) -> RoomId {
            self.room_id
        }

        /// Check if connected.
        pub fn is_connected(&self) -> bool {
            self.state.load(Ordering::Relaxed) == STATE_CONNECTED
        }
    }

    /// A persistent bidirectional channel to a room.
    pub struct Channel {
        inner: ChannelInner,
        state: Arc<AtomicU8>,
    }

    enum ChannelInner {
        WebTransport {
            send: Arc<Mutex<wtransport::SendStream>>,
            recv: Arc<Mutex<wtransport::RecvStream>>,
        },
        WebSocket {
            write: Arc<Mutex<futures::stream::SplitSink<
                tokio_tungstenite::WebSocketStream<
                    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>
                >,
                tokio_tungstenite::tungstenite::Message,
            >>>,
            read: Arc<Mutex<futures::stream::SplitStream<
                tokio_tungstenite::WebSocketStream<
                    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>
                >,
            >>>,
        },
    }

    impl Channel {
        /// Write bytes to the channel.
        pub async fn write(&self, data: &[u8]) -> Result<(), ClientError> {
            match &self.inner {
                ChannelInner::WebTransport { send, .. } => {
                    let mut send = send.lock().await;
                    send.write_all(data).await.map_err(|e| {
                        self.state.store(STATE_LOST, Ordering::Relaxed);
                        ClientError::Send(SendError::Stream(e.to_string()))
                    })?;
                    Ok(())
                }
                ChannelInner::WebSocket { write, .. } => {
                    use futures::SinkExt;
                    let mut write = write.lock().await;
                    write
                        .send(tokio_tungstenite::tungstenite::Message::Binary(data.to_vec().into()))
                        .await
                        .map_err(|e| {
                            self.state.store(STATE_LOST, Ordering::Relaxed);
                            ClientError::Send(SendError::Stream(e.to_string()))
                        })?;
                    Ok(())
                }
            }
        }

        /// Read bytes from the channel into buffer. Returns number of bytes read.
        pub async fn read(&self, buf: &mut [u8]) -> Result<usize, ClientError> {
            match &self.inner {
                ChannelInner::WebTransport { recv, .. } => {
                    let mut recv = recv.lock().await;
                    match recv.read(buf).await {
                        Ok(Some(n)) => Ok(n),
                        Ok(None) => {
                            self.state.store(STATE_LOST, Ordering::Relaxed);
                            Err(ClientError::Disconnected(DisconnectReason::ServerClosed))
                        }
                        Err(e) => {
                            self.state.store(STATE_LOST, Ordering::Relaxed);
                            Err(ClientError::Receive(ReceiveError::Stream(e.to_string())))
                        }
                    }
                }
                ChannelInner::WebSocket { read, .. } => {
                    use futures::StreamExt;
                    let mut read = read.lock().await;
                    match read.next().await {
                        Some(Ok(tokio_tungstenite::tungstenite::Message::Binary(data))) => {
                            let len = data.len().min(buf.len());
                            buf[..len].copy_from_slice(&data[..len]);
                            Ok(len)
                        }
                        Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                            let data = text.as_bytes();
                            let len = data.len().min(buf.len());
                            buf[..len].copy_from_slice(&data[..len]);
                            Ok(len)
                        }
                        Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) | None => {
                            self.state.store(STATE_LOST, Ordering::Relaxed);
                            Err(ClientError::Disconnected(DisconnectReason::ServerClosed))
                        }
                        Some(Ok(_)) => {
                            // Ping/Pong/Frame - skip, try again
                            drop(read);
                            // Recursive call - not ideal but simple
                            Box::pin(self.read(buf)).await
                        }
                        Some(Err(e)) => {
                            self.state.store(STATE_LOST, Ordering::Relaxed);
                            Err(ClientError::Receive(ReceiveError::Stream(e.to_string())))
                        }
                    }
                }
            }
        }

        /// Read exact number of bytes.
        pub async fn read_exact(&self, buf: &mut [u8]) -> Result<(), ClientError> {
            let mut filled = 0;
            while filled < buf.len() {
                let n = self.read(&mut buf[filled..]).await?;
                if n == 0 {
                    return Err(ClientError::Receive(ReceiveError::Stream(
                        "Unexpected EOF".to_string(),
                    )));
                }
                filled += n;
            }
            Ok(())
        }

        /// Check if connected.
        pub fn is_connected(&self) -> bool {
            self.state.load(Ordering::Relaxed) == STATE_CONNECTED
        }

        /// Close the channel.
        pub async fn close(self) -> Result<(), ClientError> {
            match self.inner {
                ChannelInner::WebTransport { send, .. } => {
                    let mut send = send.lock().await;
                    let _ = send.finish().await;
                }
                ChannelInner::WebSocket { write, .. } => {
                    use futures::SinkExt;
                    let mut write = write.lock().await;
                    let _ = write.close().await;
                }
            }
            Ok(())
        }
    }
}

// ============================================================================
// WASM implementation
// ============================================================================

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
mod wasm {
    use super::*;
    use crate::error::ConnectionState;
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;
    use web_wt_sys as wt;

    /// Transport type for the connection.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Transport {
        WebTransport,
        WebSocket,
    }

    /// Connection configuration.
    #[derive(Debug, Clone, Default)]
    pub struct ConnectionConfig {
        pub transport: Transport,
        pub cert_hash: Option<String>,
    }

    impl Default for Transport {
        fn default() -> Self {
            Transport::WebTransport
        }
    }

    /// A connection to a room.
    pub struct RoomConnection {
        transport: Transport,
        room_id: RoomId,
        ticket: String,
        wt_transport: Option<wt::WebTransport>,
        ws_base_url: Option<String>,
        state: Rc<RefCell<ConnectionState>>,
    }

    impl RoomConnection {
        pub async fn connect(url: &str, ticket: &str, room_id: RoomId) -> Result<Self, ClientError> {
            Self::connect_with_config(url, ticket, room_id, ConnectionConfig::default()).await
        }

        pub async fn connect_with_config(
            url: &str,
            ticket: &str,
            room_id: RoomId,
            config: ConnectionConfig,
        ) -> Result<Self, ClientError> {
            match config.transport {
                Transport::WebTransport => {
                    Self::connect_webtransport(url, ticket, room_id, config.cert_hash).await
                }
                Transport::WebSocket => {
                    Self::connect_websocket(url, ticket, room_id).await
                }
            }
        }

        async fn connect_webtransport(
            url: &str,
            ticket: &str,
            room_id: RoomId,
            cert_hash: Option<String>,
        ) -> Result<Self, ClientError> {
            let room_url = format!("{}/room/{}", url, room_id.0);

            let transport = if let Some(hash_b64) = cert_hash {
                let hash_bytes = base64_decode(&hash_b64).map_err(|e| {
                    ClientError::Connection(ConnectionError::Transport(format!(
                        "Invalid cert hash: {}",
                        e
                    )))
                })?;

                let hash = wt::WebTransportHash::new();
                hash.set_algorithm("sha-256");
                hash.set_value(&hash_bytes);

                let options = wt::WebTransportOptions::new();
                options.set_server_certificate_hashes(vec![hash]);

                wt::WebTransport::new_with_options(&room_url, &options).map_err(|e| {
                    ClientError::Connection(ConnectionError::InvalidUrl(format!("{:?}", e)))
                })?
            } else {
                wt::WebTransport::new(&room_url).map_err(|e| {
                    ClientError::Connection(ConnectionError::InvalidUrl(format!("{:?}", e)))
                })?
            };

            transport.ready().await.map_err(|e| {
                ClientError::Connection(ConnectionError::Transport(format!("{:?}", e)))
            })?;

            // Auth stream
            let bi_stream = transport.create_bidirectional_stream().await.map_err(|e| {
                ClientError::Connection(ConnectionError::Transport(format!("{:?}", e)))
            })?;

            let writable: web_sys::WritableStream = bi_stream.writable().unchecked_into();
            let writer = writable.get_writer().map_err(|e| {
                ClientError::Connection(ConnectionError::Transport(format!("{:?}", e)))
            })?;

            let ticket_bytes = js_sys::Uint8Array::from(ticket.as_bytes());
            JsFuture::from(writer.write_with_chunk(&ticket_bytes))
                .await
                .map_err(|e| {
                    ClientError::Connection(ConnectionError::Transport(format!("{:?}", e)))
                })?;

            JsFuture::from(writer.close()).await.map_err(|e| {
                ClientError::Connection(ConnectionError::Transport(format!("{:?}", e)))
            })?;

            // Read confirmation
            let readable: web_sys::ReadableStream = bi_stream.readable().unchecked_into();
            let reader = readable
                .get_reader()
                .unchecked_into::<web_sys::ReadableStreamDefaultReader>();

            let result = JsFuture::from(reader.read()).await.map_err(|e| {
                ClientError::Connection(ConnectionError::Transport(format!("{:?}", e)))
            })?;

            let result_obj: js_sys::Object = result.unchecked_into();
            let done = js_sys::Reflect::get(&result_obj, &"done".into())
                .unwrap_or(JsValue::TRUE)
                .as_bool()
                .unwrap_or(true);

            if done {
                return Err(ClientError::Connection(ConnectionError::ServerRejected(
                    "No join confirmation".to_string(),
                )));
            }

            Ok(Self {
                transport: Transport::WebTransport,
                room_id,
                ticket: ticket.to_string(),
                wt_transport: Some(transport),
                ws_base_url: None,
                state: Rc::new(RefCell::new(ConnectionState::Connected)),
            })
        }

        async fn connect_websocket(
            url: &str,
            ticket: &str,
            room_id: RoomId,
        ) -> Result<Self, ClientError> {
            // Convert to WebSocket URL
            let ws_url = url
                .replace("https://", "wss://")
                .replace("http://", "ws://");
            let base_url = format!("{}/ws/room/{}?ticket={}", ws_url, room_id.0, ticket);

            Ok(Self {
                transport: Transport::WebSocket,
                room_id,
                ticket: ticket.to_string(),
                wt_transport: None,
                ws_base_url: Some(base_url),
                state: Rc::new(RefCell::new(ConnectionState::Connected)),
            })
        }

        pub async fn open_channel(&self) -> Result<Channel, ClientError> {
            match self.transport {
                Transport::WebTransport => self.open_wt_channel().await,
                Transport::WebSocket => self.open_ws_channel().await,
            }
        }

        async fn open_wt_channel(&self) -> Result<Channel, ClientError> {
            let transport = self.wt_transport.as_ref().ok_or_else(|| {
                ClientError::Connection(ConnectionError::Transport("Not connected".to_string()))
            })?;

            let bi_stream = transport.create_bidirectional_stream().await.map_err(|e| {
                ClientError::Connection(ConnectionError::Transport(format!("{:?}", e)))
            })?;

            Ok(Channel {
                inner: ChannelInner::WebTransport { stream: bi_stream },
                state: Rc::new(RefCell::new(ConnectionState::Connected)),
            })
        }

        async fn open_ws_channel(&self) -> Result<Channel, ClientError> {
            let url = self.ws_base_url.as_ref().ok_or_else(|| {
                ClientError::Connection(ConnectionError::Transport("Not connected".to_string()))
            })?;

            let ws = web_sys::WebSocket::new(url).map_err(|e| {
                ClientError::Connection(ConnectionError::Transport(format!("{:?}", e)))
            })?;

            ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

            // Wait for open
            let open_promise = js_sys::Promise::new(&mut |resolve, reject| {
                let onopen = Closure::once(Box::new(move || {
                    resolve.call0(&JsValue::NULL).unwrap();
                }) as Box<dyn FnOnce()>);
                let onerror = Closure::once(Box::new(move |_: web_sys::ErrorEvent| {
                    reject.call0(&JsValue::NULL).unwrap();
                }) as Box<dyn FnOnce(_)>);

                ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
                ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));

                onopen.forget();
                onerror.forget();
            });

            JsFuture::from(open_promise).await.map_err(|_| {
                ClientError::Connection(ConnectionError::Transport("WebSocket open failed".to_string()))
            })?;

            Ok(Channel {
                inner: ChannelInner::WebSocket { ws },
                state: Rc::new(RefCell::new(ConnectionState::Connected)),
            })
        }

        pub fn transport(&self) -> Transport {
            self.transport
        }

        pub fn room_id(&self) -> RoomId {
            self.room_id
        }

        pub fn is_connected(&self) -> bool {
            self.state.borrow().is_connected()
        }
    }

    pub struct Channel {
        inner: ChannelInner,
        state: Rc<RefCell<ConnectionState>>,
    }

    enum ChannelInner {
        WebTransport {
            stream: wt::WebTransportBidirectionalStream,
        },
        WebSocket {
            ws: web_sys::WebSocket,
        },
    }

    impl Channel {
        pub async fn write(&self, data: &[u8]) -> Result<(), ClientError> {
            match &self.inner {
                ChannelInner::WebTransport { stream } => {
                    let writable: web_sys::WritableStream = stream.writable().unchecked_into();
                    let writer = writable
                        .get_writer()
                        .map_err(|e| ClientError::Send(SendError::Stream(format!("{:?}", e))))?;

                    let bytes = js_sys::Uint8Array::from(data);
                    JsFuture::from(writer.write_with_chunk(&bytes))
                        .await
                        .map_err(|e| ClientError::Send(SendError::Stream(format!("{:?}", e))))?;

                    writer.release_lock();
                    Ok(())
                }
                ChannelInner::WebSocket { ws } => {
                    ws.send_with_u8_array(data)
                        .map_err(|e| ClientError::Send(SendError::Stream(format!("{:?}", e))))?;
                    Ok(())
                }
            }
        }

        pub async fn read(&self, buf: &mut [u8]) -> Result<usize, ClientError> {
            match &self.inner {
                ChannelInner::WebTransport { stream } => {
                    let readable: web_sys::ReadableStream = stream.readable().unchecked_into();
                    let reader = readable
                        .get_reader()
                        .unchecked_into::<web_sys::ReadableStreamDefaultReader>();

                    let result = JsFuture::from(reader.read()).await.map_err(|e| {
                        *self.state.borrow_mut() =
                            ConnectionState::Lost(DisconnectReason::NetworkError(format!("{:?}", e)));
                        ClientError::Receive(ReceiveError::Stream(format!("{:?}", e)))
                    })?;

                    reader.release_lock();

                    let result_obj: js_sys::Object = result.unchecked_into();
                    let done = js_sys::Reflect::get(&result_obj, &"done".into())
                        .unwrap_or(JsValue::TRUE)
                        .as_bool()
                        .unwrap_or(true);

                    if done {
                        *self.state.borrow_mut() = ConnectionState::Lost(DisconnectReason::ServerClosed);
                        return Err(ClientError::Disconnected(DisconnectReason::ServerClosed));
                    }

                    let value = js_sys::Reflect::get(&result_obj, &"value".into()).map_err(|e| {
                        ClientError::Receive(ReceiveError::Stream(format!("{:?}", e)))
                    })?;

                    let array: js_sys::Uint8Array = value.unchecked_into();
                    let data = array.to_vec();
                    let len = data.len().min(buf.len());
                    buf[..len].copy_from_slice(&data[..len]);
                    Ok(len)
                }
                ChannelInner::WebSocket { ws } => {
                    // For WebSocket in WASM, we need to use callbacks
                    // This is a simplified blocking-style implementation using a Promise
                    let (tx, rx) = futures::channel::oneshot::channel();
                    let tx = Rc::new(RefCell::new(Some(tx)));

                    let onmessage = {
                        let tx = Rc::clone(&tx);
                        Closure::once(Box::new(move |e: web_sys::MessageEvent| {
                            if let Some(tx) = tx.borrow_mut().take() {
                                let _ = tx.send(e.data());
                            }
                        }) as Box<dyn FnOnce(_)>)
                    };

                    ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
                    onmessage.forget();

                    let data = rx.await.map_err(|_| {
                        ClientError::Receive(ReceiveError::Stream("Channel closed".to_string()))
                    })?;

                    if let Some(array_buffer) = data.dyn_ref::<js_sys::ArrayBuffer>() {
                        let array = js_sys::Uint8Array::new(array_buffer);
                        let vec = array.to_vec();
                        let len = vec.len().min(buf.len());
                        buf[..len].copy_from_slice(&vec[..len]);
                        Ok(len)
                    } else if let Some(text) = data.as_string() {
                        let bytes = text.as_bytes();
                        let len = bytes.len().min(buf.len());
                        buf[..len].copy_from_slice(&bytes[..len]);
                        Ok(len)
                    } else {
                        Err(ClientError::Receive(ReceiveError::MalformedMessage(
                            "Unknown message type".to_string(),
                        )))
                    }
                }
            }
        }

        pub fn is_connected(&self) -> bool {
            self.state.borrow().is_connected()
        }

        pub async fn close(self) -> Result<(), ClientError> {
            match self.inner {
                ChannelInner::WebTransport { .. } => {
                    // Stream will close when dropped
                }
                ChannelInner::WebSocket { ws } => {
                    let _ = ws.close();
                }
            }
            Ok(())
        }
    }

    fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

        fn decode_char(c: u8) -> Result<u8, String> {
            if c == b'=' {
                return Ok(0);
            }
            ALPHABET
                .iter()
                .position(|&x| x == c)
                .map(|p| p as u8)
                .ok_or_else(|| format!("Invalid base64 character: {}", c as char))
        }

        let input = input.trim().as_bytes();
        let mut output = Vec::with_capacity(input.len() * 3 / 4);

        for chunk in input.chunks(4) {
            if chunk.len() < 4 {
                return Err("Invalid base64 length".to_string());
            }

            let a = decode_char(chunk[0])?;
            let b = decode_char(chunk[1])?;
            let c = decode_char(chunk[2])?;
            let d = decode_char(chunk[3])?;

            output.push((a << 2) | (b >> 4));
            if chunk[2] != b'=' {
                output.push((b << 4) | (c >> 2));
            }
            if chunk[3] != b'=' {
                output.push((c << 6) | d);
            }
        }

        Ok(output)
    }
}

// ============================================================================
// Fallback
// ============================================================================

#[cfg(not(any(feature = "native", all(feature = "wasm", target_arch = "wasm32"))))]
mod fallback {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub enum Transport {
        #[default]
        WebTransport,
        WebSocket,
    }

    #[derive(Debug, Clone, Default)]
    pub struct ConnectionConfig {
        pub transport: Transport,
        pub cert_hash: Option<String>,
    }

    pub struct RoomConnection;
    pub struct Channel;

    impl RoomConnection {
        pub async fn connect(_url: &str, _ticket: &str, _room_id: RoomId) -> Result<Self, ClientError> {
            Err(ClientError::Connection(ConnectionError::Transport(
                "No transport feature enabled".to_string(),
            )))
        }

        pub async fn connect_with_config(
            _url: &str,
            _ticket: &str,
            _room_id: RoomId,
            _config: ConnectionConfig,
        ) -> Result<Self, ClientError> {
            Err(ClientError::Connection(ConnectionError::Transport(
                "No transport feature enabled".to_string(),
            )))
        }

        pub async fn open_channel(&self) -> Result<Channel, ClientError> {
            Err(ClientError::Connection(ConnectionError::Transport(
                "No transport feature enabled".to_string(),
            )))
        }

        pub fn transport(&self) -> Transport {
            Transport::WebTransport
        }

        pub fn room_id(&self) -> RoomId {
            RoomId(0)
        }

        pub fn is_connected(&self) -> bool {
            false
        }
    }

    impl Channel {
        pub async fn write(&self, _data: &[u8]) -> Result<(), ClientError> {
            Err(ClientError::Send(SendError::NotConnected))
        }

        pub async fn read(&self, _buf: &mut [u8]) -> Result<usize, ClientError> {
            Err(ClientError::Receive(ReceiveError::NotConnected))
        }

        pub fn is_connected(&self) -> bool {
            false
        }

        pub async fn close(self) -> Result<(), ClientError> {
            Ok(())
        }
    }
}

// ============================================================================
// Re-export
// ============================================================================

#[cfg(feature = "native")]
pub use native::{Channel, ConnectionConfig, RoomConnection, Transport};

#[cfg(all(feature = "wasm", target_arch = "wasm32", not(feature = "native")))]
pub use wasm::{Channel, ConnectionConfig, RoomConnection, Transport};

#[cfg(not(any(feature = "native", all(feature = "wasm", target_arch = "wasm32"))))]
pub use fallback::{Channel, ConnectionConfig, RoomConnection, Transport};

// Keep old RoomClient as deprecated alias for migration
#[deprecated(note = "Use RoomConnection and Channel instead")]
pub type RoomClient = RoomConnection;
