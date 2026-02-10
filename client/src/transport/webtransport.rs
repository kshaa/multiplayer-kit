//! WebTransport implementation for both native and WASM.

use crate::ClientError;
use crate::error::{ConnectionError, DisconnectReason, ReceiveError, SendError};

// ============================================================================
// Native implementation
// ============================================================================

#[cfg(feature = "native")]
mod native {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::Mutex;

    /// A WebTransport connection that can open multiple bidirectional streams.
    pub struct WebTransportConnection {
        connection: wtransport::Connection,
        connected: Arc<AtomicBool>,
    }

    impl WebTransportConnection {
        /// Connect to a WebTransport endpoint.
        ///
        /// # Arguments
        /// * `url` - The WebTransport URL (e.g., "https://example.com/path")
        /// * `cert_hash` - Optional base64-encoded SHA-256 cert hash for self-signed certs
        /// * `validate_certs` - Whether to validate TLS certs (ignored if cert_hash is set)
        pub async fn connect(
            url: &str,
            cert_hash: Option<&str>,
            validate_certs: bool,
        ) -> Result<Self, ClientError> {
            use base64::Engine;

            let config = if let Some(hash_b64) = cert_hash {
                let hash_bytes = base64::engine::general_purpose::STANDARD
                    .decode(hash_b64)
                    .map_err(|e| {
                        ClientError::Connection(ConnectionError::Transport(format!(
                            "Invalid cert hash: {}",
                            e
                        )))
                    })?;

                wtransport::ClientConfig::builder()
                    .with_bind_default()
                    .with_server_certificate_hashes([wtransport::tls::Sha256Digest::new(
                        hash_bytes.try_into().map_err(|_| {
                            ClientError::Connection(ConnectionError::Transport(
                                "Cert hash must be 32 bytes".into(),
                            ))
                        })?,
                    )])
                    .keep_alive_interval(Some(std::time::Duration::from_secs(5)))
                    .max_idle_timeout(Some(std::time::Duration::from_secs(30)))
                    .expect("valid idle timeout")
                    .build()
            } else if validate_certs {
                wtransport::ClientConfig::builder()
                    .with_bind_default()
                    .with_native_certs()
                    .keep_alive_interval(Some(std::time::Duration::from_secs(5)))
                    .max_idle_timeout(Some(std::time::Duration::from_secs(30)))
                    .expect("valid idle timeout")
                    .build()
            } else {
                wtransport::ClientConfig::builder()
                    .with_bind_default()
                    .with_no_cert_validation()
                    .keep_alive_interval(Some(std::time::Duration::from_secs(5)))
                    .max_idle_timeout(Some(std::time::Duration::from_secs(30)))
                    .expect("valid idle timeout")
                    .build()
            };

            let endpoint = wtransport::Endpoint::client(config)
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(e.to_string())))?;

            let connection = endpoint.connect(url).await.map_err(|e| {
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

            Ok(Self {
                connection,
                connected: Arc::new(AtomicBool::new(true)),
            })
        }

        /// Open a bidirectional stream.
        pub async fn open_bi(&self) -> Result<WebTransportStream, ClientError> {
            let (send, recv) = self
                .connection
                .open_bi()
                .await
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(e.to_string())))?
                .await
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(e.to_string())))?;

            Ok(WebTransportStream {
                send: Arc::new(Mutex::new(send)),
                recv: Arc::new(Mutex::new(recv)),
                connected: Arc::new(AtomicBool::new(true)),
            })
        }

        /// Accept a unidirectional receive stream (for lobby).
        pub async fn accept_uni(&self) -> Result<WebTransportRecvStream, ClientError> {
            let recv = self.connection.accept_uni().await.map_err(|e| {
                let err_str = e.to_string();
                if err_str.contains("rejected") || err_str.contains("invalid") {
                    ClientError::Connection(ConnectionError::InvalidTicket)
                } else {
                    ClientError::Connection(ConnectionError::Transport(err_str))
                }
            })?;

            Ok(WebTransportRecvStream {
                recv: Arc::new(Mutex::new(recv)),
                connected: Arc::new(AtomicBool::new(true)),
            })
        }

        pub fn is_connected(&self) -> bool {
            self.connected.load(Ordering::Relaxed)
        }
    }

    /// A bidirectional WebTransport stream.
    pub struct WebTransportStream {
        send: Arc<Mutex<wtransport::SendStream>>,
        recv: Arc<Mutex<wtransport::RecvStream>>,
        connected: Arc<AtomicBool>,
    }

    impl WebTransportStream {
        pub async fn read(&self, buf: &mut [u8]) -> Result<usize, ClientError> {
            let mut recv = self.recv.lock().await;
            match recv.read(buf).await {
                Ok(Some(n)) => Ok(n),
                Ok(None) => {
                    self.connected.store(false, Ordering::Relaxed);
                    Err(ClientError::Disconnected(DisconnectReason::ServerClosed))
                }
                Err(e) => {
                    self.connected.store(false, Ordering::Relaxed);
                    Err(ClientError::Receive(ReceiveError::Stream(e.to_string())))
                }
            }
        }

        pub async fn write(&self, data: &[u8]) -> Result<(), ClientError> {
            let mut send = self.send.lock().await;
            send.write_all(data).await.map_err(|e| {
                self.connected.store(false, Ordering::Relaxed);
                ClientError::Send(SendError::Stream(e.to_string()))
            })
        }

        pub fn is_connected(&self) -> bool {
            self.connected.load(Ordering::Relaxed)
        }

        pub async fn close(self) -> Result<(), ClientError> {
            let mut send = self.send.lock().await;
            let _ = send.finish().await;
            Ok(())
        }
    }

    /// A unidirectional receive stream (for lobby events).
    pub struct WebTransportRecvStream {
        recv: Arc<Mutex<wtransport::RecvStream>>,
        connected: Arc<AtomicBool>,
    }

    impl WebTransportRecvStream {
        /// Read a length-prefixed message.
        pub async fn recv_message(&mut self) -> Result<Vec<u8>, ClientError> {
            let mut recv = self.recv.lock().await;

            // Read length prefix (4 bytes, big-endian)
            let mut len_buf = [0u8; 4];
            if let Err(e) = recv.read_exact(&mut len_buf).await {
                self.connected.store(false, Ordering::Relaxed);
                return Err(ClientError::Receive(ReceiveError::Stream(e.to_string())));
            }

            let len = u32::from_be_bytes(len_buf) as usize;

            // Read message body
            let mut buf = vec![0u8; len];
            if let Err(e) = recv.read_exact(&mut buf).await {
                self.connected.store(false, Ordering::Relaxed);
                return Err(ClientError::Receive(ReceiveError::Stream(e.to_string())));
            }

            Ok(buf)
        }

        pub fn is_connected(&self) -> bool {
            self.connected.load(Ordering::Relaxed)
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

    /// A WebTransport connection that can open multiple bidirectional streams.
    pub struct WebTransportConnection {
        transport: wt::WebTransport,
        connected: Rc<RefCell<bool>>,
    }

    impl WebTransportConnection {
        /// Connect to a WebTransport endpoint.
        pub async fn connect(
            url: &str,
            cert_hash: Option<&str>,
            _validate_certs: bool, // Ignored in browser - browser handles validation
        ) -> Result<Self, ClientError> {
            use base64::Engine;

            let transport = if let Some(hash_b64) = cert_hash {
                let hash_bytes = base64::engine::general_purpose::STANDARD
                    .decode(hash_b64)
                    .map_err(|e| {
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

                wt::WebTransport::new_with_options(url, &options).map_err(|e| {
                    ClientError::Connection(ConnectionError::InvalidUrl(format!("{:?}", e)))
                })?
            } else {
                wt::WebTransport::new(url).map_err(|e| {
                    ClientError::Connection(ConnectionError::InvalidUrl(format!("{:?}", e)))
                })?
            };

            transport.ready().await.map_err(|e| {
                ClientError::Connection(ConnectionError::Transport(format!("{:?}", e)))
            })?;

            Ok(Self {
                transport,
                connected: Rc::new(RefCell::new(true)),
            })
        }

        /// Open a bidirectional stream.
        pub async fn open_bi(&self) -> Result<WebTransportStream, ClientError> {
            let stream = self
                .transport
                .create_bidirectional_stream()
                .await
                .map_err(|e| {
                    ClientError::Connection(ConnectionError::Transport(format!("{:?}", e)))
                })?;

            Ok(WebTransportStream {
                stream,
                connected: Rc::new(RefCell::new(true)),
            })
        }

        /// Accept a unidirectional receive stream (for lobby).
        pub async fn accept_uni(&self) -> Result<WebTransportRecvStream, ClientError> {
            let incoming_uni: web_sys::ReadableStream =
                self.transport.incoming_unidirectional_streams().unchecked_into();
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

            let stream_value =
                js_sys::Reflect::get(&result_obj, &"value".into()).map_err(|e| {
                    ClientError::Connection(ConnectionError::Transport(format!("{:?}", e)))
                })?;

            let recv_stream: web_sys::ReadableStream = stream_value.unchecked_into();
            let reader = recv_stream
                .get_reader()
                .unchecked_into::<web_sys::ReadableStreamDefaultReader>();

            Ok(WebTransportRecvStream {
                reader,
                buffer: Vec::new(),
                connected: Rc::new(RefCell::new(true)),
            })
        }

        pub fn is_connected(&self) -> bool {
            *self.connected.borrow()
        }

        pub fn close(&self) {
            self.transport.close();
            *self.connected.borrow_mut() = false;
        }
    }

    /// A bidirectional WebTransport stream.
    pub struct WebTransportStream {
        stream: wt::WebTransportBidirectionalStream,
        connected: Rc<RefCell<bool>>,
    }

    impl WebTransportStream {
        pub async fn read(&self, buf: &mut [u8]) -> Result<usize, ClientError> {
            let readable: web_sys::ReadableStream = self.stream.readable().unchecked_into();
            let reader = readable
                .get_reader()
                .unchecked_into::<web_sys::ReadableStreamDefaultReader>();

            let result = JsFuture::from(reader.read()).await.map_err(|e| {
                *self.connected.borrow_mut() = false;
                ClientError::Receive(ReceiveError::Stream(format!("{:?}", e)))
            })?;

            reader.release_lock();

            let result_obj: js_sys::Object = result.unchecked_into();
            let done = js_sys::Reflect::get(&result_obj, &"done".into())
                .unwrap_or(JsValue::TRUE)
                .as_bool()
                .unwrap_or(true);

            if done {
                *self.connected.borrow_mut() = false;
                return Err(ClientError::Disconnected(DisconnectReason::ServerClosed));
            }

            let value = js_sys::Reflect::get(&result_obj, &"value".into())
                .map_err(|e| ClientError::Receive(ReceiveError::Stream(format!("{:?}", e))))?;

            let array: js_sys::Uint8Array = value.unchecked_into();
            let data = array.to_vec();
            let len = data.len().min(buf.len());
            buf[..len].copy_from_slice(&data[..len]);
            Ok(len)
        }

        pub async fn write(&self, data: &[u8]) -> Result<(), ClientError> {
            let writable: web_sys::WritableStream = self.stream.writable().unchecked_into();
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

        pub fn is_connected(&self) -> bool {
            *self.connected.borrow()
        }

        pub async fn close(self) -> Result<(), ClientError> {
            // Stream closes when dropped
            Ok(())
        }
    }

    /// A unidirectional receive stream (for lobby events).
    pub struct WebTransportRecvStream {
        reader: web_sys::ReadableStreamDefaultReader,
        buffer: Vec<u8>,
        connected: Rc<RefCell<bool>>,
    }

    impl WebTransportRecvStream {
        /// Read a length-prefixed message.
        pub async fn recv_message(&mut self) -> Result<Vec<u8>, ClientError> {
            // Fill buffer until we have length prefix
            while self.buffer.len() < 4 {
                let chunk = self.read_chunk().await?;
                if chunk.is_empty() {
                    *self.connected.borrow_mut() = false;
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

            // Fill buffer until we have full message
            while self.buffer.len() < 4 + len {
                let chunk = self.read_chunk().await?;
                if chunk.is_empty() {
                    *self.connected.borrow_mut() = false;
                    return Err(ClientError::Disconnected(DisconnectReason::ServerClosed));
                }
                self.buffer.extend(chunk);
            }

            // Extract message (skip length prefix)
            let msg_data: Vec<u8> = self.buffer.drain(..4 + len).skip(4).collect();
            Ok(msg_data)
        }

        async fn read_chunk(&mut self) -> Result<Vec<u8>, ClientError> {
            let result = JsFuture::from(self.reader.read()).await.map_err(|e| {
                *self.connected.borrow_mut() = false;
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

        pub fn is_connected(&self) -> bool {
            *self.connected.borrow()
        }
    }
}

// ============================================================================
// Re-exports
// ============================================================================

#[cfg(feature = "native")]
pub use native::{WebTransportConnection, WebTransportRecvStream, WebTransportStream};

#[cfg(all(feature = "wasm", target_arch = "wasm32", not(feature = "native")))]
pub use wasm::{WebTransportConnection, WebTransportRecvStream, WebTransportStream};
