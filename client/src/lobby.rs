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

            let lobby_url = format!("{}/lobby", url);
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

            let (mut send_stream, _recv) = connection
                .open_bi()
                .await
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(e.to_string())))?
                .await
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(e.to_string())))?;

            send_stream
                .write_all(ticket.as_bytes())
                .await
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(e.to_string())))?;

            drop(send_stream);

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

            bincode::deserialize(&buf)
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

    pub struct LobbyClient {
        _transport: wt::WebTransport,
        reader: web_sys::ReadableStreamDefaultReader,
        buffer: Vec<u8>,
        state: Rc<RefCell<ConnectionState>>,
    }

    impl LobbyClient {
        pub async fn connect(url: &str, ticket: &str) -> Result<Self, ClientError> {
            Self::connect_with_options(url, ticket, None).await
        }

        pub async fn connect_with_options(
            url: &str,
            ticket: &str,
            cert_hash_base64: Option<&str>,
        ) -> Result<Self, ClientError> {
            let lobby_url = format!("{}/lobby", url);

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
                _transport: transport,
                reader,
                buffer: Vec::new(),
                state: Rc::new(RefCell::new(ConnectionState::Connected)),
            })
        }

        pub fn state(&self) -> ConnectionState {
            self.state.borrow().clone()
        }

        pub async fn recv(&mut self) -> Result<LobbyEvent, ClientError> {
            if !self.state.borrow().is_connected() {
                return Err(ClientError::Receive(ReceiveError::NotConnected));
            }

            while self.buffer.len() < 4 {
                let chunk = self.read_chunk().await?;
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
                let chunk = self.read_chunk().await?;
                if chunk.is_empty() {
                    *self.state.borrow_mut() =
                        ConnectionState::Lost(DisconnectReason::ServerClosed);
                    return Err(ClientError::Disconnected(DisconnectReason::ServerClosed));
                }
                self.buffer.extend(chunk);
            }

            let msg_data: Vec<u8> = self.buffer.drain(..4 + len).skip(4).collect();

            bincode::deserialize(&msg_data)
                .map_err(|e| ClientError::Receive(ReceiveError::MalformedMessage(e.to_string())))
        }

        async fn read_chunk(&mut self) -> Result<Vec<u8>, ClientError> {
            let result = JsFuture::from(self.reader.read()).await.map_err(|e| {
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
            self._transport.close();
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
