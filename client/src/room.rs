//! Room client for game communication.

use crate::error::{ConnectionError, ConnectionState, DisconnectReason, ReceiveError, SendError};
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
    use tokio::io::AsyncReadExt;

    const STATE_DISCONNECTED: u8 = 0;
    const STATE_CONNECTING: u8 = 1;
    const STATE_CONNECTED: u8 = 2;
    const STATE_LOST: u8 = 3;

    pub struct RoomClient {
        connection: wtransport::Connection,
        msg_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
        _recv_task: tokio::task::JoinHandle<()>,
        state: Arc<AtomicU8>,
        disconnect_reason: Arc<tokio::sync::RwLock<Option<DisconnectReason>>>,
        _room_id: RoomId,
    }

    impl RoomClient {
        pub async fn connect(url: &str, ticket: &str, room_id: RoomId) -> Result<Self, ClientError> {
            let config = wtransport::ClientConfig::builder()
                .with_bind_default()
                .with_no_cert_validation()
                .keep_alive_interval(Some(std::time::Duration::from_secs(5)))
                .max_idle_timeout(Some(std::time::Duration::from_secs(30)))
                .expect("valid idle timeout")
                .build();

            let endpoint = wtransport::Endpoint::client(config).map_err(|e| {
                ClientError::Connection(ConnectionError::Transport(e.to_string()))
            })?;

            let room_url = format!("{}/room/{}", url, room_id.0);
            let connection = endpoint.connect(&room_url).await.map_err(|e| {
                let err_str = e.to_string();
                if err_str.contains("DNS") || err_str.contains("resolve") {
                    ClientError::Connection(ConnectionError::DnsResolution(err_str))
                } else if err_str.contains("TLS") || err_str.contains("certificate") {
                    ClientError::Connection(ConnectionError::TlsHandshake(err_str))
                } else if err_str.contains("refused") {
                    ClientError::Connection(ConnectionError::Refused(err_str))
                } else {
                    ClientError::Connection(ConnectionError::Transport(err_str))
                }
            })?;

            let (mut send_stream, mut recv_stream) = connection
                .open_bi()
                .await
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(e.to_string())))?
                .await
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(e.to_string())))?;

            send_stream.write_all(ticket.as_bytes()).await.map_err(|e| {
                ClientError::Connection(ConnectionError::Transport(e.to_string()))
            })?;

            drop(send_stream);

            let mut confirm_buf = [0u8; 64];
            let n = recv_stream.read(&mut confirm_buf).await.map_err(|e| {
                ClientError::Connection(ConnectionError::Transport(e.to_string()))
            })?;

            if n.is_none() || n == Some(0) {
                return Err(ClientError::Connection(ConnectionError::ServerRejected(
                    "No join confirmation".to_string(),
                )));
            }

            let state = Arc::new(AtomicU8::new(STATE_CONNECTED));
            let disconnect_reason = Arc::new(tokio::sync::RwLock::new(None));

            let (msg_tx, msg_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);

            let recv_connection = connection.clone();
            let recv_state = Arc::clone(&state);
            let recv_disconnect = Arc::clone(&disconnect_reason);

            let recv_task = tokio::spawn(async move {
                loop {
                    match recv_connection.accept_uni().await {
                        Ok(mut stream) => {
                            let mut data = Vec::new();
                            if stream.read_to_end(&mut data).await.is_ok() && !data.is_empty() {
                                if msg_tx.send(data).await.is_err() {
                                    recv_state.store(STATE_LOST, Ordering::Relaxed);
                                    *recv_disconnect.write().await =
                                        Some(DisconnectReason::ClientClosed);
                                    break;
                                }
                            }
                        }
                        Err(e) => {
                            recv_state.store(STATE_LOST, Ordering::Relaxed);
                            *recv_disconnect.write().await =
                                Some(DisconnectReason::NetworkError(e.to_string()));
                            break;
                        }
                    }
                }
            });

            Ok(Self {
                connection,
                msg_rx,
                _recv_task: recv_task,
                state,
                disconnect_reason,
                _room_id: room_id,
            })
        }

        pub fn state(&self) -> ConnectionState {
            match self.state.load(Ordering::Relaxed) {
                STATE_DISCONNECTED => ConnectionState::Disconnected,
                STATE_CONNECTING => ConnectionState::Connecting,
                STATE_CONNECTED => ConnectionState::Connected,
                STATE_LOST => {
                    // Try to get the reason without blocking
                    if let Ok(guard) = self.disconnect_reason.try_read() {
                        ConnectionState::Lost(
                            guard
                                .clone()
                                .unwrap_or(DisconnectReason::Unknown("Unknown".to_string())),
                        )
                    } else {
                        ConnectionState::Lost(DisconnectReason::Unknown("Unknown".to_string()))
                    }
                }
                _ => ConnectionState::Disconnected,
            }
        }

        pub async fn send(&self, payload: &[u8]) -> Result<(), ClientError> {
            if self.state.load(Ordering::Relaxed) != STATE_CONNECTED {
                return Err(ClientError::Send(SendError::NotConnected));
            }

            let (mut send_stream, _recv) = self
                .connection
                .open_bi()
                .await
                .map_err(|e| ClientError::Send(SendError::Stream(e.to_string())))?
                .await
                .map_err(|e| ClientError::Send(SendError::Stream(e.to_string())))?;

            send_stream
                .write_all(payload)
                .await
                .map_err(|e| ClientError::Send(SendError::Stream(e.to_string())))?;

            Ok(())
        }

        pub async fn recv(&mut self) -> Result<Vec<u8>, ClientError> {
            if self.state.load(Ordering::Relaxed) == STATE_LOST {
                let reason = self
                    .disconnect_reason
                    .read()
                    .await
                    .clone()
                    .unwrap_or(DisconnectReason::Unknown("Unknown".to_string()));
                return Err(ClientError::Disconnected(reason));
            }

            match self.msg_rx.recv().await {
                Some(msg) => Ok(msg),
                None => {
                    self.state.store(STATE_LOST, Ordering::Relaxed);
                    let reason = self
                        .disconnect_reason
                        .read()
                        .await
                        .clone()
                        .unwrap_or(DisconnectReason::ServerClosed);
                    Err(ClientError::Disconnected(reason))
                }
            }
        }

        pub async fn close(self) -> Result<(), ClientError> {
            self.state.store(STATE_DISCONNECTED, Ordering::Relaxed);
            *self.disconnect_reason.write().await = Some(DisconnectReason::ClientClosed);
            self._recv_task.abort();
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
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;
    use web_wt_sys as wt;

    pub struct RoomClient {
        transport: wt::WebTransport,
        incoming_reader: web_sys::ReadableStreamDefaultReader,
        state: Rc<RefCell<ConnectionState>>,
        _room_id: RoomId,
    }

    impl RoomClient {
        pub async fn connect(url: &str, ticket: &str, room_id: RoomId) -> Result<Self, ClientError> {
            let room_url = format!("{}/room/{}", url, room_id.0);

            let transport = wt::WebTransport::new(&room_url).map_err(|e| {
                ClientError::Connection(ConnectionError::InvalidUrl(format!("{:?}", e)))
            })?;

            transport.ready().await.map_err(|e| {
                ClientError::Connection(ConnectionError::Transport(format!("{:?}", e)))
            })?;

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

            let incoming_uni: web_sys::ReadableStream =
                transport.incoming_unidirectional_streams().unchecked_into();
            let incoming_reader = incoming_uni
                .get_reader()
                .unchecked_into::<web_sys::ReadableStreamDefaultReader>();

            Ok(Self {
                transport,
                incoming_reader,
                state: Rc::new(RefCell::new(ConnectionState::Connected)),
                _room_id: room_id,
            })
        }

        pub fn state(&self) -> ConnectionState {
            self.state.borrow().clone()
        }

        pub async fn send(&self, payload: &[u8]) -> Result<(), ClientError> {
            if !self.state.borrow().is_connected() {
                return Err(ClientError::Send(SendError::NotConnected));
            }

            let bi_stream = self
                .transport
                .create_bidirectional_stream()
                .await
                .map_err(|e| {
                    *self.state.borrow_mut() =
                        ConnectionState::Lost(DisconnectReason::NetworkError(format!("{:?}", e)));
                    ClientError::Send(SendError::Stream(format!("{:?}", e)))
                })?;

            let writable: web_sys::WritableStream = bi_stream.writable().unchecked_into();
            let writer = writable
                .get_writer()
                .map_err(|e| ClientError::Send(SendError::Stream(format!("{:?}", e))))?;

            let payload_bytes = js_sys::Uint8Array::from(payload);
            JsFuture::from(writer.write_with_chunk(&payload_bytes))
                .await
                .map_err(|e| ClientError::Send(SendError::Stream(format!("{:?}", e))))?;

            JsFuture::from(writer.close())
                .await
                .map_err(|e| ClientError::Send(SendError::Stream(format!("{:?}", e))))?;

            Ok(())
        }

        pub async fn recv(&mut self) -> Result<Vec<u8>, ClientError> {
            if !self.state.borrow().is_connected() {
                return Err(ClientError::Receive(ReceiveError::NotConnected));
            }

            let result = JsFuture::from(self.incoming_reader.read())
                .await
                .map_err(|e| {
                    *self.state.borrow_mut() =
                        ConnectionState::Lost(DisconnectReason::NetworkError(format!("{:?}", e)));
                    ClientError::Receive(ReceiveError::Stream(format!("{:?}", e)))
                })?;

            let result_obj: js_sys::Object = result.unchecked_into();
            let done = js_sys::Reflect::get(&result_obj, &"done".into())
                .unwrap_or(JsValue::TRUE)
                .as_bool()
                .unwrap_or(true);

            if done {
                *self.state.borrow_mut() = ConnectionState::Lost(DisconnectReason::ServerClosed);
                return Err(ClientError::Disconnected(DisconnectReason::ServerClosed));
            }

            let stream_value = js_sys::Reflect::get(&result_obj, &"value".into()).map_err(|e| {
                ClientError::Receive(ReceiveError::Stream(format!("{:?}", e)))
            })?;

            let recv_stream: web_sys::ReadableStream = stream_value.unchecked_into();
            let reader = recv_stream
                .get_reader()
                .unchecked_into::<web_sys::ReadableStreamDefaultReader>();

            let mut data = Vec::new();
            loop {
                let result = JsFuture::from(reader.read()).await.map_err(|e| {
                    ClientError::Receive(ReceiveError::Stream(format!("{:?}", e)))
                })?;

                let result_obj: js_sys::Object = result.unchecked_into();
                let done = js_sys::Reflect::get(&result_obj, &"done".into())
                    .unwrap_or(JsValue::TRUE)
                    .as_bool()
                    .unwrap_or(true);

                if done {
                    break;
                }

                let value = js_sys::Reflect::get(&result_obj, &"value".into()).map_err(|e| {
                    ClientError::Receive(ReceiveError::Stream(format!("{:?}", e)))
                })?;

                let array: js_sys::Uint8Array = value.unchecked_into();
                data.extend(array.to_vec());
            }

            Ok(data)
        }

        pub async fn close(self) -> Result<(), ClientError> {
            *self.state.borrow_mut() = ConnectionState::Lost(DisconnectReason::ClientClosed);
            self.transport.close();
            Ok(())
        }
    }
}

// ============================================================================
// Fallback
// ============================================================================

#[cfg(not(any(feature = "native", all(feature = "wasm", target_arch = "wasm32"))))]
mod fallback {
    use super::*;

    pub struct RoomClient;

    impl RoomClient {
        pub async fn connect(
            _url: &str,
            _ticket: &str,
            _room_id: RoomId,
        ) -> Result<Self, ClientError> {
            Err(ClientError::Connection(ConnectionError::Transport(
                "No transport feature enabled".to_string(),
            )))
        }

        pub fn state(&self) -> ConnectionState {
            ConnectionState::Disconnected
        }

        pub async fn send(&self, _payload: &[u8]) -> Result<(), ClientError> {
            Err(ClientError::Send(SendError::NotConnected))
        }

        pub async fn recv(&mut self) -> Result<Vec<u8>, ClientError> {
            Err(ClientError::Receive(ReceiveError::NotConnected))
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
pub use native::RoomClient;

#[cfg(all(feature = "wasm", target_arch = "wasm32", not(feature = "native")))]
pub use wasm::RoomClient;

#[cfg(not(any(feature = "native", all(feature = "wasm", target_arch = "wasm32"))))]
pub use fallback::RoomClient;
