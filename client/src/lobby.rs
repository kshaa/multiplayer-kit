//! Lobby client for receiving room updates.

use crate::ClientError;
use crate::error::{ConnectionError, ConnectionState, DisconnectReason, ReceiveError};
use multiplayer_kit_protocol::LobbyEvent;

// ============================================================================
// Native implementation
// ============================================================================

#[cfg(feature = "native")]
mod native {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, Ordering};
    #[allow(unused_imports)]
    use tokio::io::AsyncReadExt;

    // Connection state as atomic for thread-safe access
    const STATE_DISCONNECTED: u8 = 0;
    const STATE_CONNECTING: u8 = 1;
    const STATE_CONNECTED: u8 = 2;
    const STATE_LOST: u8 = 3;

    pub struct LobbyClient {
        _connection: wtransport::Connection,
        recv_stream: wtransport::RecvStream,
        state: Arc<AtomicU8>,
        disconnect_reason: Option<DisconnectReason>,
    }

    impl LobbyClient {
        pub async fn connect(url: &str, ticket: &str) -> Result<Self, ClientError> {
            let config = wtransport::ClientConfig::builder()
                .with_bind_default()
                .with_no_cert_validation()
                .keep_alive_interval(Some(std::time::Duration::from_secs(5)))
                .max_idle_timeout(Some(std::time::Duration::from_secs(30)))
                .expect("valid idle timeout")
                .build();

            let endpoint = wtransport::Endpoint::client(config)
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(e.to_string())))?;

            // Auth via query param (same as rooms)
            let lobby_url = format!("{}/lobby?ticket={}", url, ticket);
            let connection = endpoint.connect(&lobby_url).await.map_err(|e| {
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

            // Server sends events on unidirectional stream
            let recv_stream = connection.accept_uni().await.map_err(|e| {
                let err_str = e.to_string();
                if err_str.contains("rejected") || err_str.contains("invalid") {
                    ClientError::Connection(ConnectionError::InvalidTicket)
                } else {
                    ClientError::Connection(ConnectionError::Transport(err_str))
                }
            })?;

            Ok(Self {
                _connection: connection,
                recv_stream,
                state: Arc::new(AtomicU8::new(STATE_CONNECTED)),
                disconnect_reason: None,
            })
        }

        pub fn state(&self) -> ConnectionState {
            match self.state.load(Ordering::Relaxed) {
                STATE_DISCONNECTED => ConnectionState::Disconnected,
                STATE_CONNECTING => ConnectionState::Connecting,
                STATE_CONNECTED => ConnectionState::Connected,
                STATE_LOST => ConnectionState::Lost(
                    self.disconnect_reason
                        .clone()
                        .unwrap_or(DisconnectReason::Unknown("Unknown".to_string())),
                ),
                _ => ConnectionState::Disconnected,
            }
        }

        pub async fn recv(&mut self) -> Result<LobbyEvent, ClientError> {
            if self.state.load(Ordering::Relaxed) != STATE_CONNECTED {
                return Err(ClientError::Receive(ReceiveError::NotConnected));
            }

            let mut len_buf = [0u8; 4];
            if let Err(e) = self.recv_stream.read_exact(&mut len_buf).await {
                self.mark_disconnected(DisconnectReason::NetworkError(e.to_string()));
                return Err(ClientError::Receive(ReceiveError::Stream(e.to_string())));
            }

            let len = u32::from_be_bytes(len_buf) as usize;

            let mut buf = vec![0u8; len];
            if let Err(e) = self.recv_stream.read_exact(&mut buf).await {
                self.mark_disconnected(DisconnectReason::NetworkError(e.to_string()));
                return Err(ClientError::Receive(ReceiveError::Stream(e.to_string())));
            }

            // Lobby uses JSON (RoomInfo.config is serde_json::Value)
            serde_json::from_slice(&buf)
                .map_err(|e| ClientError::Receive(ReceiveError::MalformedMessage(e.to_string())))
        }

        fn mark_disconnected(&mut self, reason: DisconnectReason) {
            self.state.store(STATE_LOST, Ordering::Relaxed);
            self.disconnect_reason = Some(reason);
        }

        pub async fn close(mut self) -> Result<(), ClientError> {
            self.state.store(STATE_DISCONNECTED, Ordering::Relaxed);
            self.disconnect_reason = Some(DisconnectReason::ClientClosed);
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

    enum LobbyTransport {
        WebTransport {
            _transport: wt::WebTransport,
            reader: web_sys::ReadableStreamDefaultReader,
        },
        WebSocket {
            ws: web_sys::WebSocket,
            messages: Rc<RefCell<Vec<Vec<u8>>>>,
        },
    }

    pub struct LobbyClient {
        transport: LobbyTransport,
        buffer: Vec<u8>,
        state: Rc<RefCell<ConnectionState>>,
    }

    impl LobbyClient {
        pub async fn connect(url: &str, ticket: &str) -> Result<Self, ClientError> {
            Self::connect_with_options(url, ticket, None).await
        }

        /// Auto-detect: try WebTransport first, fall back to WebSocket
        pub async fn connect_auto(
            wt_url: &str,
            ws_url: &str,
            ticket: &str,
            cert_hash_base64: Option<&str>,
        ) -> Result<Self, ClientError> {
            // Check if WebTransport is available
            let has_wt = js_sys::Reflect::get(&js_sys::global(), &"WebTransport".into())
                .map(|v| !v.is_undefined())
                .unwrap_or(false);

            if has_wt {
                match Self::connect_webtransport(wt_url, ticket, cert_hash_base64).await {
                    Ok(client) => return Ok(client),
                    Err(e) => {
                        web_sys::console::warn_1(
                            &format!(
                                "WebTransport lobby failed, falling back to WebSocket: {:?}",
                                e
                            )
                            .into(),
                        );
                    }
                }
            }

            // Fall back to WebSocket
            Self::connect_websocket(ws_url, ticket).await
        }

        pub async fn connect_with_options(
            url: &str,
            ticket: &str,
            cert_hash_base64: Option<&str>,
        ) -> Result<Self, ClientError> {
            Self::connect_webtransport(url, ticket, cert_hash_base64).await
        }

        async fn connect_webtransport(
            url: &str,
            ticket: &str,
            cert_hash_base64: Option<&str>,
        ) -> Result<Self, ClientError> {
            let lobby_url = format!("{}/lobby?ticket={}", url, ticket);

            let transport = if let Some(hash_b64) = cert_hash_base64 {
                let hash_bytes = base64_decode(hash_b64).map_err(|e| {
                    ClientError::Connection(ConnectionError::Transport(format!(
                        "Invalid cert hash base64: {}",
                        e
                    )))
                })?;

                let hash = wt::WebTransportHash::new();
                hash.set_algorithm("sha-256");
                hash.set_value(&hash_bytes);

                let options = wt::WebTransportOptions::new();
                options.set_server_certificate_hashes(vec![hash]);

                wt::WebTransport::new_with_options(&lobby_url, &options).map_err(|e| {
                    ClientError::Connection(ConnectionError::InvalidUrl(format!("{:?}", e)))
                })?
            } else {
                wt::WebTransport::new(&lobby_url).map_err(|e| {
                    ClientError::Connection(ConnectionError::InvalidUrl(format!("{:?}", e)))
                })?
            };

            transport.ready().await.map_err(|e| {
                ClientError::Connection(ConnectionError::Transport(format!("{:?}", e)))
            })?;

            // Server sends events on unidirectional stream
            let incoming_uni: web_sys::ReadableStream =
                transport.incoming_unidirectional_streams().unchecked_into();
            let uni_reader = incoming_uni
                .get_reader()
                .unchecked_into::<web_sys::ReadableStreamDefaultReader>();

            let result = JsFuture::from(uni_reader.read()).await.map_err(|e| {
                ClientError::Connection(ConnectionError::Transport(format!("{:?}", e)))
            })?;

            let result_obj: js_sys::Object = result.unchecked_into();
            let done = js_sys::Reflect::get(&result_obj, &"done".into())
                .unwrap_or(JsValue::TRUE)
                .as_bool()
                .unwrap_or(true);

            if done {
                return Err(ClientError::Connection(ConnectionError::ServerRejected(
                    "No unidirectional stream received".to_string(),
                )));
            }

            let stream_value = js_sys::Reflect::get(&result_obj, &"value".into()).map_err(|e| {
                ClientError::Connection(ConnectionError::Transport(format!("{:?}", e)))
            })?;

            let recv_stream: web_sys::ReadableStream = stream_value.unchecked_into();
            let reader = recv_stream
                .get_reader()
                .unchecked_into::<web_sys::ReadableStreamDefaultReader>();

            Ok(Self {
                transport: LobbyTransport::WebTransport {
                    _transport: transport,
                    reader,
                },
                buffer: Vec::new(),
                state: Rc::new(RefCell::new(ConnectionState::Connected)),
            })
        }

        async fn connect_websocket(url: &str, ticket: &str) -> Result<Self, ClientError> {
            let ws_url = format!("{}/ws/lobby?ticket={}", url, ticket);

            let ws = web_sys::WebSocket::new(&ws_url).map_err(|e| {
                ClientError::Connection(ConnectionError::InvalidUrl(format!("{:?}", e)))
            })?;

            ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

            let messages: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
            let state: Rc<RefCell<ConnectionState>> =
                Rc::new(RefCell::new(ConnectionState::Connecting));

            // Set up message handler
            let messages_clone = messages.clone();
            let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
                move |event: web_sys::MessageEvent| {
                    if let Ok(buffer) = event.data().dyn_into::<js_sys::ArrayBuffer>() {
                        let array = js_sys::Uint8Array::new(&buffer);
                        messages_clone.borrow_mut().push(array.to_vec());
                    }
                },
            );
            ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
            on_message.forget();

            // Set up close handler
            let state_clone = state.clone();
            let on_close =
                Closure::<dyn FnMut(web_sys::CloseEvent)>::new(move |_: web_sys::CloseEvent| {
                    *state_clone.borrow_mut() =
                        ConnectionState::Lost(DisconnectReason::ServerClosed);
                });
            ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
            on_close.forget();

            // Set up error handler
            let state_clone = state.clone();
            let on_error =
                Closure::<dyn FnMut(web_sys::ErrorEvent)>::new(move |_: web_sys::ErrorEvent| {
                    *state_clone.borrow_mut() = ConnectionState::Lost(
                        DisconnectReason::NetworkError("WebSocket error".to_string()),
                    );
                });
            ws.set_onerror(Some(on_error.as_ref().unchecked_ref()));
            on_error.forget();

            // Wait for connection
            let ws_clone = ws.clone();
            let state_clone = state.clone();
            let open_promise = js_sys::Promise::new(&mut |resolve, reject| {
                let on_open = Closure::<dyn FnMut()>::new(move || {
                    resolve.call0(&JsValue::NULL).unwrap();
                });
                ws_clone.set_onopen(Some(on_open.as_ref().unchecked_ref()));
                on_open.forget();

                let on_error = Closure::<dyn FnMut(web_sys::ErrorEvent)>::new(
                    move |e: web_sys::ErrorEvent| {
                        reject.call1(&JsValue::NULL, &e).unwrap();
                    },
                );
                ws_clone.set_onerror(Some(on_error.as_ref().unchecked_ref()));
                on_error.forget();
            });

            JsFuture::from(open_promise).await.map_err(|e| {
                ClientError::Connection(ConnectionError::Transport(format!("{:?}", e)))
            })?;

            *state_clone.borrow_mut() = ConnectionState::Connected;

            Ok(Self {
                transport: LobbyTransport::WebSocket { ws, messages },
                buffer: Vec::new(),
                state,
            })
        }

        pub fn state(&self) -> ConnectionState {
            self.state.borrow().clone()
        }

        pub async fn recv(&mut self) -> Result<LobbyEvent, ClientError> {
            if !self.state.borrow().is_connected() {
                return Err(ClientError::Receive(ReceiveError::NotConnected));
            }

            // Check transport type and get data accordingly
            let is_websocket = matches!(self.transport, LobbyTransport::WebSocket { .. });

            if is_websocket {
                // WebSocket: messages are already framed
                let messages = match &self.transport {
                    LobbyTransport::WebSocket { messages, .. } => messages.clone(),
                    _ => unreachable!(),
                };

                loop {
                    // Check for pending messages
                    {
                        let mut msgs = messages.borrow_mut();
                        if !msgs.is_empty() {
                            let data = msgs.remove(0);
                            // Lobby uses JSON (RoomInfo.config is serde_json::Value)
                            return serde_json::from_slice(&data).map_err(|e| {
                                ClientError::Receive(ReceiveError::MalformedMessage(e.to_string()))
                            });
                        }
                    }

                    // Check if disconnected
                    if !self.state.borrow().is_connected() {
                        return Err(ClientError::Disconnected(DisconnectReason::ServerClosed));
                    }

                    // Yield to event loop
                    let promise = js_sys::Promise::new(&mut |resolve, _| {
                        let window = web_sys::window().unwrap();
                        let closure = Closure::once(move || {
                            resolve.call0(&JsValue::NULL).unwrap();
                        });
                        window
                            .set_timeout_with_callback_and_timeout_and_arguments_0(
                                closure.as_ref().unchecked_ref(),
                                10,
                            )
                            .unwrap();
                        closure.forget();
                    });
                    let _ = JsFuture::from(promise).await;
                }
            } else {
                // WebTransport: length-prefixed framing
                while self.buffer.len() < 4 {
                    let chunk = self.read_wt_chunk().await?;
                    if chunk.is_empty() {
                        *self.state.borrow_mut() =
                            ConnectionState::Lost(DisconnectReason::ServerClosed);
                        return Err(ClientError::Disconnected(DisconnectReason::ServerClosed));
                    }
                    self.buffer.extend(chunk);
                }

                let len = u32::from_be_bytes([
                    self.buffer[0],
                    self.buffer[1],
                    self.buffer[2],
                    self.buffer[3],
                ]) as usize;

                while self.buffer.len() < 4 + len {
                    let chunk = self.read_wt_chunk().await?;
                    if chunk.is_empty() {
                        *self.state.borrow_mut() =
                            ConnectionState::Lost(DisconnectReason::ServerClosed);
                        return Err(ClientError::Disconnected(DisconnectReason::ServerClosed));
                    }
                    self.buffer.extend(chunk);
                }

                let msg_data: Vec<u8> = self.buffer.drain(..4 + len).skip(4).collect();

                // Lobby uses JSON (RoomInfo.config is serde_json::Value)
                serde_json::from_slice(&msg_data).map_err(|e| {
                    ClientError::Receive(ReceiveError::MalformedMessage(e.to_string()))
                })
            }
        }

        async fn read_wt_chunk(&mut self) -> Result<Vec<u8>, ClientError> {
            let reader = match &self.transport {
                LobbyTransport::WebTransport { reader, .. } => reader,
                _ => return Err(ClientError::Receive(ReceiveError::NotConnected)),
            };

            let result = JsFuture::from(reader.read()).await.map_err(|e| {
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
                return Ok(Vec::new());
            }

            let value = js_sys::Reflect::get(&result_obj, &"value".into())
                .map_err(|e| ClientError::Receive(ReceiveError::Stream(format!("{:?}", e))))?;

            let array: js_sys::Uint8Array = value.unchecked_into();
            Ok(array.to_vec())
        }

        pub async fn close(self) -> Result<(), ClientError> {
            *self.state.borrow_mut() = ConnectionState::Lost(DisconnectReason::ClientClosed);
            match self.transport {
                LobbyTransport::WebTransport { _transport, .. } => {
                    _transport.close();
                }
                LobbyTransport::WebSocket { ws, .. } => {
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

    pub struct LobbyClient;

    impl LobbyClient {
        pub async fn connect(_url: &str, _ticket: &str) -> Result<Self, ClientError> {
            Err(ClientError::Connection(ConnectionError::Transport(
                "No transport feature enabled".to_string(),
            )))
        }

        pub fn state(&self) -> ConnectionState {
            ConnectionState::Disconnected
        }

        pub async fn recv(&mut self) -> Result<LobbyEvent, ClientError> {
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
pub use native::LobbyClient;

#[cfg(all(feature = "wasm", target_arch = "wasm32", not(feature = "native")))]
pub use wasm::LobbyClient;

#[cfg(not(any(feature = "native", all(feature = "wasm", target_arch = "wasm32"))))]
pub use fallback::LobbyClient;
