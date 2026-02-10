//! WebSocket implementation for both native and WASM.

use crate::ClientError;
use crate::error::{ConnectionError, DisconnectReason, SendError};

#[cfg(feature = "native")]
use crate::error::ReceiveError;

// ============================================================================
// Native implementation
// ============================================================================

#[cfg(feature = "native")]
mod native {
    use super::*;
    use futures::{SinkExt, StreamExt};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::Mutex;
    use tokio_tungstenite::tungstenite::Message;

    type WsStream = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    /// A WebSocket connection.
    ///
    /// Unlike WebTransport, WebSocket doesn't support multiple streams per connection.
    /// Each "stream" is actually a new WebSocket connection to the server.
    pub struct WebSocketConnection {
        base_url: String,
        connected: Arc<AtomicBool>,
    }

    impl WebSocketConnection {
        /// Create a WebSocket connection factory.
        ///
        /// This doesn't actually connect - it validates the URL and stores it.
        /// Actual connections happen when you call `open_stream()`.
        pub async fn connect(url: &str) -> Result<Self, ClientError> {
            // Validate URL
            url::Url::parse(url)
                .map_err(|e| ClientError::Connection(ConnectionError::InvalidUrl(e.to_string())))?;

            Ok(Self {
                base_url: url.to_string(),
                connected: Arc::new(AtomicBool::new(true)),
            })
        }

        /// Open a new WebSocket connection (stream).
        pub async fn open_stream(&self) -> Result<WebSocketStream, ClientError> {
            let (ws_stream, _) = tokio_tungstenite::connect_async(&self.base_url)
                .await
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(e.to_string())))?;

            let (write, read) = ws_stream.split();

            Ok(WebSocketStream {
                write: Arc::new(Mutex::new(write)),
                read: Arc::new(Mutex::new(read)),
                connected: Arc::new(AtomicBool::new(true)),
            })
        }

        /// Open a stream and wait for initial confirmation message.
        pub async fn open_stream_with_confirmation(&self) -> Result<WebSocketStream, ClientError> {
            let (ws_stream, _) = tokio_tungstenite::connect_async(&self.base_url)
                .await
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(e.to_string())))?;

            let (write, mut read) = ws_stream.split();

            // Wait for first message (confirmation)
            if let Some(msg) = read.next().await {
                match msg {
                    Ok(Message::Binary(_)) | Ok(Message::Text(_)) => {
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

            Ok(WebSocketStream {
                write: Arc::new(Mutex::new(write)),
                read: Arc::new(Mutex::new(read)),
                connected: Arc::new(AtomicBool::new(true)),
            })
        }

        pub fn is_connected(&self) -> bool {
            self.connected.load(Ordering::Relaxed)
        }
    }

    /// A bidirectional WebSocket stream.
    pub struct WebSocketStream {
        write: Arc<
            Mutex<
                futures::stream::SplitSink<WsStream, Message>,
            >,
        >,
        read: Arc<Mutex<futures::stream::SplitStream<WsStream>>>,
        connected: Arc<AtomicBool>,
    }

    impl WebSocketStream {
        pub async fn read(&self, buf: &mut [u8]) -> Result<usize, ClientError> {
            let mut read = self.read.lock().await;

            loop {
                match read.next().await {
                    Some(Ok(Message::Binary(data))) => {
                        let len = data.len().min(buf.len());
                        buf[..len].copy_from_slice(&data[..len]);
                        return Ok(len);
                    }
                    Some(Ok(Message::Text(text))) => {
                        let data = text.as_bytes();
                        let len = data.len().min(buf.len());
                        buf[..len].copy_from_slice(&data[..len]);
                        return Ok(len);
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        self.connected.store(false, Ordering::Relaxed);
                        return Err(ClientError::Disconnected(DisconnectReason::ServerClosed));
                    }
                    Some(Ok(_)) => {
                        // Ping/Pong/Frame - skip, continue reading
                        continue;
                    }
                    Some(Err(e)) => {
                        self.connected.store(false, Ordering::Relaxed);
                        return Err(ClientError::Receive(ReceiveError::Stream(e.to_string())));
                    }
                }
            }
        }

        pub async fn write(&self, data: &[u8]) -> Result<(), ClientError> {
            let mut write = self.write.lock().await;
            write
                .send(Message::Binary(data.to_vec().into()))
                .await
                .map_err(|e| {
                    self.connected.store(false, Ordering::Relaxed);
                    ClientError::Send(SendError::Stream(e.to_string()))
                })
        }

        pub fn is_connected(&self) -> bool {
            self.connected.load(Ordering::Relaxed)
        }

        pub async fn close(self) -> Result<(), ClientError> {
            let mut write = self.write.lock().await;
            let _ = write.close().await;
            Ok(())
        }

        /// Read a length-prefixed message (for lobby compatibility).
        pub async fn recv_message(&mut self) -> Result<Vec<u8>, ClientError> {
            // WebSocket messages are already framed, so just read one
            let mut buf = vec![0u8; 65536]; // Max message size
            let n = self.read(&mut buf).await?;
            buf.truncate(n);
            Ok(buf)
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

    /// A WebSocket connection.
    pub struct WebSocketConnection {
        base_url: String,
        connected: Rc<RefCell<bool>>,
    }

    impl WebSocketConnection {
        /// Create a WebSocket connection factory.
        pub async fn connect(url: &str) -> Result<Self, ClientError> {
            Ok(Self {
                base_url: url.to_string(),
                connected: Rc::new(RefCell::new(true)),
            })
        }

        /// Open a new WebSocket connection (stream).
        pub async fn open_stream(&self) -> Result<WebSocketStream, ClientError> {
            let ws = web_sys::WebSocket::new(&self.base_url).map_err(|e| {
                ClientError::Connection(ConnectionError::InvalidUrl(format!("{:?}", e)))
            })?;

            ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

            // Wait for open
            let ws_clone = ws.clone();
            let open_promise = js_sys::Promise::new(&mut |resolve, reject| {
                let onopen = Closure::once(Box::new(move || {
                    resolve.call0(&JsValue::NULL).unwrap();
                }) as Box<dyn FnOnce()>);
                let onerror = Closure::once(Box::new(move |_: web_sys::ErrorEvent| {
                    reject.call0(&JsValue::NULL).unwrap();
                }) as Box<dyn FnOnce(_)>);

                ws_clone.set_onopen(Some(onopen.as_ref().unchecked_ref()));
                ws_clone.set_onerror(Some(onerror.as_ref().unchecked_ref()));

                onopen.forget();
                onerror.forget();
            });

            JsFuture::from(open_promise).await.map_err(|_| {
                ClientError::Connection(ConnectionError::Transport(
                    "WebSocket open failed".to_string(),
                ))
            })?;

            Ok(WebSocketStream::new(ws))
        }

        pub fn is_connected(&self) -> bool {
            *self.connected.borrow()
        }
    }

    /// A bidirectional WebSocket stream.
    pub struct WebSocketStream {
        ws: web_sys::WebSocket,
        messages: Rc<RefCell<Vec<Vec<u8>>>>,
        connected: Rc<RefCell<bool>>,
    }

    impl WebSocketStream {
        fn new(ws: web_sys::WebSocket) -> Self {
            let messages: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
            let connected: Rc<RefCell<bool>> = Rc::new(RefCell::new(true));

            // Set up message handler
            let messages_clone = messages.clone();
            let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
                move |event: web_sys::MessageEvent| {
                    if let Ok(buffer) = event.data().dyn_into::<js_sys::ArrayBuffer>() {
                        let array = js_sys::Uint8Array::new(&buffer);
                        messages_clone.borrow_mut().push(array.to_vec());
                    } else if let Some(text) = event.data().as_string() {
                        messages_clone.borrow_mut().push(text.into_bytes());
                    }
                },
            );
            ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
            on_message.forget();

            // Set up close handler
            let connected_clone = connected.clone();
            let on_close =
                Closure::<dyn FnMut(web_sys::CloseEvent)>::new(move |_: web_sys::CloseEvent| {
                    *connected_clone.borrow_mut() = false;
                });
            ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
            on_close.forget();

            // Set up error handler
            let connected_clone = connected.clone();
            let on_error =
                Closure::<dyn FnMut(web_sys::ErrorEvent)>::new(move |_: web_sys::ErrorEvent| {
                    *connected_clone.borrow_mut() = false;
                });
            ws.set_onerror(Some(on_error.as_ref().unchecked_ref()));
            on_error.forget();

            Self {
                ws,
                messages,
                connected,
            }
        }

        pub async fn read(&self, buf: &mut [u8]) -> Result<usize, ClientError> {
            loop {
                // Check for pending messages
                {
                    let mut msgs = self.messages.borrow_mut();
                    if !msgs.is_empty() {
                        let data = msgs.remove(0);
                        let len = data.len().min(buf.len());
                        buf[..len].copy_from_slice(&data[..len]);
                        return Ok(len);
                    }
                }

                // Check if disconnected
                if !*self.connected.borrow() {
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
        }

        pub async fn write(&self, data: &[u8]) -> Result<(), ClientError> {
            self.ws
                .send_with_u8_array(data)
                .map_err(|e| ClientError::Send(SendError::Stream(format!("{:?}", e))))
        }

        pub fn is_connected(&self) -> bool {
            *self.connected.borrow()
        }

        pub async fn close(self) -> Result<(), ClientError> {
            let _ = self.ws.close();
            Ok(())
        }

        /// Read next message (for lobby compatibility - messages are already framed).
        pub async fn recv_message(&mut self) -> Result<Vec<u8>, ClientError> {
            let mut buf = vec![0u8; 65536];
            let n = self.read(&mut buf).await?;
            buf.truncate(n);
            Ok(buf)
        }
    }
}

// ============================================================================
// Re-exports
// ============================================================================

#[cfg(feature = "native")]
pub use native::{WebSocketConnection, WebSocketStream};

#[cfg(all(feature = "wasm", target_arch = "wasm32", not(feature = "native")))]
pub use wasm::{WebSocketConnection, WebSocketStream};
