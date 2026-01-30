//! Lobby client for receiving room updates.

use crate::ClientError;
use multiplayer_kit_protocol::LobbyEvent;

// ============================================================================
// Native implementation
// ============================================================================

#[cfg(feature = "native")]
mod native {
    use super::*;
    #[allow(unused_imports)]
    use tokio::io::AsyncReadExt;

    pub struct LobbyClient {
        _connection: wtransport::Connection,
        recv_stream: wtransport::RecvStream,
    }

    impl LobbyClient {
        pub async fn connect(url: &str, ticket: &str) -> Result<Self, ClientError> {
            let config = wtransport::ClientConfig::builder()
                .with_bind_default()
                .with_native_certs()
                .build();

            let endpoint = wtransport::Endpoint::client(config)
                .map_err(|e| ClientError::ConnectionFailed(e.to_string()))?;

            let lobby_url = format!("{}/lobby", url);
            let connection = endpoint
                .connect(&lobby_url)
                .await
                .map_err(|e| ClientError::ConnectionFailed(e.to_string()))?;

            let (mut send_stream, _recv) = connection
                .open_bi()
                .await
                .map_err(|e| ClientError::ConnectionFailed(e.to_string()))?
                .await
                .map_err(|e| ClientError::ConnectionFailed(e.to_string()))?;

            send_stream
                .write_all(ticket.as_bytes())
                .await
                .map_err(|e| ClientError::SendFailed(e.to_string()))?;

            drop(send_stream);

            let recv_stream = connection
                .accept_uni()
                .await
                .map_err(|e| ClientError::ConnectionFailed(e.to_string()))?;

            Ok(Self {
                _connection: connection,
                recv_stream,
            })
        }

        pub async fn recv(&mut self) -> Result<LobbyEvent, ClientError> {
            let mut len_buf = [0u8; 4];
            self.recv_stream
                .read_exact(&mut len_buf)
                .await
                .map_err(|e| ClientError::ReceiveFailed(e.to_string()))?;

            let len = u32::from_be_bytes(len_buf) as usize;

            let mut buf = vec![0u8; len];
            self.recv_stream
                .read_exact(&mut buf)
                .await
                .map_err(|e| ClientError::ReceiveFailed(e.to_string()))?;

            bincode::deserialize(&buf).map_err(|e| ClientError::Serialization(e.to_string()))
        }

        pub async fn close(self) -> Result<(), ClientError> {
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
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;
    use web_wt_sys as wt;

    pub struct LobbyClient {
        _transport: wt::WebTransport,
        reader: web_sys::ReadableStreamDefaultReader,
        buffer: Vec<u8>,
    }

    impl LobbyClient {
        pub async fn connect(url: &str, ticket: &str) -> Result<Self, ClientError> {
            // Connect to lobby endpoint
            let lobby_url = format!("{}/lobby", url);

            let transport = wt::WebTransport::new(&lobby_url)
                .map_err(|e| ClientError::ConnectionFailed(format!("{:?}", e)))?;

            // Wait for connection to be ready (web-wt-sys returns Rust futures)
            transport
                .ready()
                .await
                .map_err(|e| ClientError::ConnectionFailed(format!("{:?}", e)))?;

            // Open bidirectional stream to send ticket
            let bi_stream = transport
                .create_bidirectional_stream()
                .await
                .map_err(|e| ClientError::ConnectionFailed(format!("{:?}", e)))?;

            let writable: web_sys::WritableStream = bi_stream.writable().unchecked_into();
            let writer = writable
                .get_writer()
                .map_err(|e| ClientError::ConnectionFailed(format!("{:?}", e)))?;

            // Send ticket
            let ticket_bytes = js_sys::Uint8Array::from(ticket.as_bytes());
            JsFuture::from(writer.write_with_chunk(&ticket_bytes))
                .await
                .map_err(|e| ClientError::SendFailed(format!("{:?}", e)))?;

            // Close writer to signal we're done
            JsFuture::from(writer.close())
                .await
                .map_err(|e| ClientError::SendFailed(format!("{:?}", e)))?;

            // Get incoming unidirectional streams reader
            let incoming_uni: web_sys::ReadableStream =
                transport.incoming_unidirectional_streams().unchecked_into();
            let uni_reader = incoming_uni
                .get_reader()
                .unchecked_into::<web_sys::ReadableStreamDefaultReader>();

            // Wait for the server's unidirectional stream
            let result = JsFuture::from(uni_reader.read())
                .await
                .map_err(|e| ClientError::ConnectionFailed(format!("{:?}", e)))?;

            let result_obj: js_sys::Object = result.unchecked_into();
            let done = js_sys::Reflect::get(&result_obj, &"done".into())
                .unwrap_or(JsValue::TRUE)
                .as_bool()
                .unwrap_or(true);

            if done {
                return Err(ClientError::ConnectionFailed(
                    "No unidirectional stream received".to_string(),
                ));
            }

            let stream_value = js_sys::Reflect::get(&result_obj, &"value".into())
                .map_err(|e| ClientError::ConnectionFailed(format!("{:?}", e)))?;

            let recv_stream: web_sys::ReadableStream = stream_value.unchecked_into();
            let reader = recv_stream
                .get_reader()
                .unchecked_into::<web_sys::ReadableStreamDefaultReader>();

            Ok(Self {
                _transport: transport,
                reader,
                buffer: Vec::new(),
            })
        }

        pub async fn recv(&mut self) -> Result<LobbyEvent, ClientError> {
            // Read until we have at least 4 bytes for length
            while self.buffer.len() < 4 {
                let chunk = self.read_chunk().await?;
                if chunk.is_empty() {
                    return Err(ClientError::Disconnected("Stream closed".to_string()));
                }
                self.buffer.extend(chunk);
            }

            let len =
                u32::from_be_bytes([self.buffer[0], self.buffer[1], self.buffer[2], self.buffer[3]])
                    as usize;

            // Read until we have the full message
            while self.buffer.len() < 4 + len {
                let chunk = self.read_chunk().await?;
                if chunk.is_empty() {
                    return Err(ClientError::Disconnected("Stream closed".to_string()));
                }
                self.buffer.extend(chunk);
            }

            // Extract the message
            let msg_data: Vec<u8> = self.buffer.drain(..4 + len).skip(4).collect();

            bincode::deserialize(&msg_data).map_err(|e| ClientError::Serialization(e.to_string()))
        }

        async fn read_chunk(&mut self) -> Result<Vec<u8>, ClientError> {
            let result = JsFuture::from(self.reader.read())
                .await
                .map_err(|e| ClientError::ReceiveFailed(format!("{:?}", e)))?;

            let result_obj: js_sys::Object = result.unchecked_into();
            let done = js_sys::Reflect::get(&result_obj, &"done".into())
                .unwrap_or(JsValue::TRUE)
                .as_bool()
                .unwrap_or(true);

            if done {
                return Ok(Vec::new());
            }

            let value = js_sys::Reflect::get(&result_obj, &"value".into())
                .map_err(|e| ClientError::ReceiveFailed(format!("{:?}", e)))?;

            let array: js_sys::Uint8Array = value.unchecked_into();
            Ok(array.to_vec())
        }

        pub async fn close(self) -> Result<(), ClientError> {
            self._transport.close();
            Ok(())
        }
    }
}

// ============================================================================
// Fallback (neither native nor wasm)
// ============================================================================

#[cfg(not(any(feature = "native", all(feature = "wasm", target_arch = "wasm32"))))]
mod fallback {
    use super::*;

    pub struct LobbyClient;

    impl LobbyClient {
        pub async fn connect(_url: &str, _ticket: &str) -> Result<Self, ClientError> {
            Err(ClientError::ConnectionFailed(
                "No transport feature enabled".to_string(),
            ))
        }

        pub async fn recv(&mut self) -> Result<LobbyEvent, ClientError> {
            Err(ClientError::ConnectionFailed(
                "No transport feature enabled".to_string(),
            ))
        }

        pub async fn close(self) -> Result<(), ClientError> {
            Ok(())
        }
    }
}

// ============================================================================
// Re-export the correct implementation
// ============================================================================

#[cfg(feature = "native")]
pub use native::LobbyClient;

#[cfg(all(feature = "wasm", target_arch = "wasm32", not(feature = "native")))]
pub use wasm::LobbyClient;

#[cfg(not(any(feature = "native", all(feature = "wasm", target_arch = "wasm32"))))]
pub use fallback::LobbyClient;
