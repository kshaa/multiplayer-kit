//! Room client for game communication.

use crate::ClientError;
use multiplayer_kit_protocol::RoomId;

// ============================================================================
// Native implementation
// ============================================================================

#[cfg(feature = "native")]
mod native {
    use super::*;
    use tokio::io::AsyncReadExt;

    pub struct RoomClient {
        connection: wtransport::Connection,
        msg_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
        _recv_task: tokio::task::JoinHandle<()>,
        _room_id: RoomId,
    }

    impl RoomClient {
        pub async fn connect(url: &str, ticket: &str, room_id: RoomId) -> Result<Self, ClientError> {
            let config = wtransport::ClientConfig::builder()
                .with_bind_default()
                .with_native_certs()
                .build();

            let endpoint = wtransport::Endpoint::client(config)
                .map_err(|e| ClientError::ConnectionFailed(e.to_string()))?;

            let room_url = format!("{}/room/{}", url, room_id.0);
            let connection = endpoint
                .connect(&room_url)
                .await
                .map_err(|e| ClientError::ConnectionFailed(e.to_string()))?;

            let (mut send_stream, mut recv_stream) = connection
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

            let mut confirm_buf = [0u8; 64];
            let _n = recv_stream
                .read(&mut confirm_buf)
                .await
                .map_err(|e| ClientError::ReceiveFailed(e.to_string()))?;

            let (msg_tx, msg_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);

            let recv_connection = connection.clone();
            let recv_task = tokio::spawn(async move {
                loop {
                    match recv_connection.accept_uni().await {
                        Ok(mut stream) => {
                            let mut data = Vec::new();
                            if stream.read_to_end(&mut data).await.is_ok() && !data.is_empty() {
                                if msg_tx.send(data).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
            });

            Ok(Self {
                connection,
                msg_rx,
                _recv_task: recv_task,
                _room_id: room_id,
            })
        }

        pub async fn send(&self, payload: &[u8]) -> Result<(), ClientError> {
            let (mut send_stream, _recv) = self
                .connection
                .open_bi()
                .await
                .map_err(|e| ClientError::SendFailed(e.to_string()))?
                .await
                .map_err(|e| ClientError::SendFailed(e.to_string()))?;

            send_stream
                .write_all(payload)
                .await
                .map_err(|e| ClientError::SendFailed(e.to_string()))?;

            Ok(())
        }

        pub async fn recv(&mut self) -> Result<Vec<u8>, ClientError> {
            self.msg_rx
                .recv()
                .await
                .ok_or_else(|| ClientError::Disconnected("Channel closed".to_string()))
        }

        pub async fn close(self) -> Result<(), ClientError> {
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
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;
    use web_wt_sys as wt;

    pub struct RoomClient {
        transport: wt::WebTransport,
        incoming_reader: web_sys::ReadableStreamDefaultReader,
        _room_id: RoomId,
    }

    impl RoomClient {
        pub async fn connect(url: &str, ticket: &str, room_id: RoomId) -> Result<Self, ClientError> {
            let room_url = format!("{}/room/{}", url, room_id.0);

            let transport = wt::WebTransport::new(&room_url)
                .map_err(|e| ClientError::ConnectionFailed(format!("{:?}", e)))?;

            // web-wt-sys returns Rust futures
            transport
                .ready()
                .await
                .map_err(|e| ClientError::ConnectionFailed(format!("{:?}", e)))?;

            // Open bidirectional stream to send ticket
            let bi_stream = transport
                .create_bidirectional_stream()
                .await
                .map_err(|e| ClientError::ConnectionFailed(format!("{:?}", e)))?;

            // Send ticket
            let writable: web_sys::WritableStream = bi_stream.writable().unchecked_into();
            let writer = writable
                .get_writer()
                .map_err(|e| ClientError::ConnectionFailed(format!("{:?}", e)))?;

            let ticket_bytes = js_sys::Uint8Array::from(ticket.as_bytes());
            JsFuture::from(writer.write_with_chunk(&ticket_bytes))
                .await
                .map_err(|e| ClientError::SendFailed(format!("{:?}", e)))?;

            JsFuture::from(writer.close())
                .await
                .map_err(|e| ClientError::SendFailed(format!("{:?}", e)))?;

            // Read join confirmation from readable side
            let readable: web_sys::ReadableStream = bi_stream.readable().unchecked_into();
            let reader = readable
                .get_reader()
                .unchecked_into::<web_sys::ReadableStreamDefaultReader>();

            let result = JsFuture::from(reader.read())
                .await
                .map_err(|e| ClientError::ReceiveFailed(format!("{:?}", e)))?;

            let result_obj: js_sys::Object = result.unchecked_into();
            let done = js_sys::Reflect::get(&result_obj, &"done".into())
                .unwrap_or(JsValue::TRUE)
                .as_bool()
                .unwrap_or(true);

            if done {
                return Err(ClientError::ConnectionFailed(
                    "No join confirmation".to_string(),
                ));
            }

            // Get incoming unidirectional streams reader
            let incoming_uni: web_sys::ReadableStream =
                transport.incoming_unidirectional_streams().unchecked_into();
            let incoming_reader = incoming_uni
                .get_reader()
                .unchecked_into::<web_sys::ReadableStreamDefaultReader>();

            Ok(Self {
                transport,
                incoming_reader,
                _room_id: room_id,
            })
        }

        pub async fn send(&self, payload: &[u8]) -> Result<(), ClientError> {
            let bi_stream = self
                .transport
                .create_bidirectional_stream()
                .await
                .map_err(|e| ClientError::SendFailed(format!("{:?}", e)))?;

            let writable: web_sys::WritableStream = bi_stream.writable().unchecked_into();
            let writer = writable
                .get_writer()
                .map_err(|e| ClientError::SendFailed(format!("{:?}", e)))?;

            let payload_bytes = js_sys::Uint8Array::from(payload);
            JsFuture::from(writer.write_with_chunk(&payload_bytes))
                .await
                .map_err(|e| ClientError::SendFailed(format!("{:?}", e)))?;

            JsFuture::from(writer.close())
                .await
                .map_err(|e| ClientError::SendFailed(format!("{:?}", e)))?;

            Ok(())
        }

        pub async fn recv(&mut self) -> Result<Vec<u8>, ClientError> {
            // Wait for next incoming unidirectional stream
            let result = JsFuture::from(self.incoming_reader.read())
                .await
                .map_err(|e| ClientError::ReceiveFailed(format!("{:?}", e)))?;

            let result_obj: js_sys::Object = result.unchecked_into();
            let done = js_sys::Reflect::get(&result_obj, &"done".into())
                .unwrap_or(JsValue::TRUE)
                .as_bool()
                .unwrap_or(true);

            if done {
                return Err(ClientError::Disconnected("Stream closed".to_string()));
            }

            let stream_value = js_sys::Reflect::get(&result_obj, &"value".into())
                .map_err(|e| ClientError::ReceiveFailed(format!("{:?}", e)))?;

            let recv_stream: web_sys::ReadableStream = stream_value.unchecked_into();
            let reader = recv_stream
                .get_reader()
                .unchecked_into::<web_sys::ReadableStreamDefaultReader>();

            // Read all data from the stream
            let mut data = Vec::new();
            loop {
                let result = JsFuture::from(reader.read())
                    .await
                    .map_err(|e| ClientError::ReceiveFailed(format!("{:?}", e)))?;

                let result_obj: js_sys::Object = result.unchecked_into();
                let done = js_sys::Reflect::get(&result_obj, &"done".into())
                    .unwrap_or(JsValue::TRUE)
                    .as_bool()
                    .unwrap_or(true);

                if done {
                    break;
                }

                let value = js_sys::Reflect::get(&result_obj, &"value".into())
                    .map_err(|e| ClientError::ReceiveFailed(format!("{:?}", e)))?;

                let array: js_sys::Uint8Array = value.unchecked_into();
                data.extend(array.to_vec());
            }

            Ok(data)
        }

        pub async fn close(self) -> Result<(), ClientError> {
            self.transport.close();
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

    pub struct RoomClient;

    impl RoomClient {
        pub async fn connect(_url: &str, _ticket: &str, _room_id: RoomId) -> Result<Self, ClientError> {
            Err(ClientError::ConnectionFailed(
                "No transport feature enabled".to_string(),
            ))
        }

        pub async fn send(&self, _payload: &[u8]) -> Result<(), ClientError> {
            Err(ClientError::ConnectionFailed(
                "No transport feature enabled".to_string(),
            ))
        }

        pub async fn recv(&mut self) -> Result<Vec<u8>, ClientError> {
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
pub use native::RoomClient;

#[cfg(all(feature = "wasm", target_arch = "wasm32", not(feature = "native")))]
pub use wasm::RoomClient;

#[cfg(not(any(feature = "native", all(feature = "wasm", target_arch = "wasm32"))))]
pub use fallback::RoomClient;
