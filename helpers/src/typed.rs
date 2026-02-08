//! Typed actor helpers for server and client.
//!
//! Provides strongly-typed channels with automatic serialization.
//! Each channel has its own message type, identified on first message.

#[cfg(feature = "server")]
use std::future::Future;
#[cfg(feature = "server")]
use std::pin::Pin;

// ============================================================================
// TypedProtocol trait - game implements this
// ============================================================================

/// Error when decoding a message.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("unknown channel id: {0}")]
    UnknownChannel(u8),
    #[error("deserialization failed: {0}")]
    Deserialize(String),
}

/// Error when encoding a message.
#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("serialization failed: {0}")]
    Serialize(String),
}

/// Protocol trait - game defines channel types and message types.
///
/// # Example
///
/// ```ignore
/// #[derive(Clone, Copy, PartialEq, Eq, Hash)]
/// pub enum GameChannel { Chat, GameState }
///
/// #[derive(Serialize, Deserialize)]
/// pub enum ChatMessage { Text(String), Emote(String) }
///
/// #[derive(Serialize, Deserialize)]  
/// pub enum GameStateMessage { Position { x: f32, y: f32 } }
///
/// pub enum GameEvent {
///     Chat(ChatMessage),
///     GameState(GameStateMessage),
/// }
///
/// pub struct MyProtocol;
/// impl TypedProtocol for MyProtocol {
///     type Channel = GameChannel;
///     type Event = GameEvent;
///     // ...
/// }
/// ```
pub trait TypedProtocol: Send + Sync + 'static {
    /// Channel identifier enum.
    type Channel: Copy + Eq + std::hash::Hash + std::fmt::Debug + Send + Sync + 'static;

    /// Unified event enum (wraps all channel message types).
    type Event: Send + 'static;

    /// Get channel ID byte from channel.
    fn channel_to_id(channel: Self::Channel) -> u8;

    /// Get channel from ID byte.
    fn channel_from_id(id: u8) -> Option<Self::Channel>;

    /// List all channel types (for client to open all).
    fn all_channels() -> &'static [Self::Channel];

    /// Decode bytes from a channel into an event.
    fn decode(channel: Self::Channel, data: &[u8]) -> Result<Self::Event, DecodeError>;

    /// Encode an event to (channel, bytes).
    fn encode(event: &Self::Event) -> Result<(Self::Channel, Vec<u8>), EncodeError>;
}

// ============================================================================
// Server-side typed actor
// ============================================================================

#[cfg(feature = "server")]
mod server {
    use super::*;
    use crate::framing::{MessageBuffer, frame_message};
    use dashmap::DashMap;
    use multiplayer_kit_protocol::{ChannelId, RoomId, UserContext};
    use std::collections::HashMap;
    use std::marker::PhantomData;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    /// Events delivered to a typed server actor.
    #[derive(Debug)]
    pub enum TypedEvent<T: UserContext, P: TypedProtocol> {
        /// User connected (all channels established).
        UserConnected(T),
        /// User disconnected (any channel failed or clean disconnect).
        UserDisconnected(T),
        /// Message received.
        Message {
            sender: T,
            /// The channel type (e.g., Chat, GameState).
            channel: P::Channel,
            event: P::Event,
        },
        /// Internal event sent by actor to itself.
        Internal(P::Event),
        /// Room is shutting down.
        Shutdown,
    }

    /// Context for server-side typed actor.
    pub struct TypedContext<T: UserContext, P: TypedProtocol> {
        room_id: RoomId,
        /// Sender to deliver events back to self.
        self_tx: mpsc::Sender<P::Event>,
        handle: multiplayer_kit_server::RoomHandle<T>,
        /// Maps channel_id -> (user, channel_type)
        user_channels: Arc<DashMap<ChannelId, (T, P::Channel)>>,
        _phantom: PhantomData<P>,
    }

    impl<T: UserContext, P: TypedProtocol> TypedContext<T, P> {
        /// Get the room ID.
        pub fn room_id(&self) -> RoomId {
            self.room_id
        }

        /// Broadcast event to all channels of the event's type.
        pub async fn broadcast(&self, event: &P::Event) {
            if let Ok((channel_type, data)) = P::encode(event) {
                let framed = frame_message(&data);
                // Collect matching channel IDs to avoid holding DashMap ref across await
                let channel_ids: Vec<_> = self
                    .user_channels
                    .iter()
                    .filter(|r| r.value().1 == channel_type)
                    .map(|r| *r.key())
                    .collect();
                self.handle.send_to(&channel_ids, &framed).await;
            }
        }

        /// Broadcast event to all channels except sender's channel.
        pub async fn broadcast_except(&self, exclude: ChannelId, event: &P::Event) {
            if let Ok((channel_type, data)) = P::encode(event) {
                let framed = frame_message(&data);
                let channel_ids: Vec<_> = self
                    .user_channels
                    .iter()
                    .filter(|r| r.value().1 == channel_type && *r.key() != exclude)
                    .map(|r| *r.key())
                    .collect();
                self.handle.send_to(&channel_ids, &framed).await;
            }
        }

        /// Send to a specific user (all their channels of the event's type).
        pub async fn send_to_user(&self, user: &T, event: &P::Event) {
            if let Ok((channel_type, data)) = P::encode(event) {
                let framed = frame_message(&data);
                let channel_ids: Vec<_> = self
                    .user_channels
                    .iter()
                    .filter(|r| r.value().1 == channel_type && r.value().0.id() == user.id())
                    .map(|r| *r.key())
                    .collect();
                self.handle.send_to(&channel_ids, &framed).await;
            }
        }

        /// Get a sender to deliver events back to the actor itself.
        /// Use this to spawn tasks that can notify the actor later.
        pub fn self_tx(&self) -> mpsc::Sender<P::Event> {
            self.self_tx.clone()
        }
    }

    impl<T: UserContext, P: TypedProtocol> Clone for TypedContext<T, P> {
        fn clone(&self) -> Self {
            Self {
                room_id: self.room_id,
                self_tx: self.self_tx.clone(),
                handle: self.handle.clone(),
                user_channels: self.user_channels.clone(),
                _phantom: PhantomData,
            }
        }
    }

    // Internal event type for actor communication
    enum ServerInternalEvent<T: UserContext, P: TypedProtocol> {
        UserReady(T),
        UserGone(T),
        Message {
            sender: T,
            channel_type: P::Channel,
            event: P::Event,
        },
    }

    /// Wrap a typed actor function for the server.
    ///
    /// The actor function receives:
    /// - `TypedContext<T, P>` for sending messages
    /// - `TypedEvent<T, P>` the current event
    /// - `Arc<C>` the room config (shared, immutable)
    pub fn with_typed_actor<T, P, C, F, Fut>(
        actor_fn: F,
    ) -> impl Fn(multiplayer_kit_server::Room<T>, C) -> Pin<Box<dyn Future<Output = ()> + Send>>
    + Send
    + Sync
    + Clone
    + 'static
    where
        T: UserContext,
        P: TypedProtocol,
        C: multiplayer_kit_protocol::RoomConfig + 'static,
        F: Fn(TypedContext<T, P>, TypedEvent<T, P>, Arc<C>) -> Fut + Send + Sync + Clone + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let actor_fn = Arc::new(actor_fn);
        move |room: multiplayer_kit_server::Room<T>, config: C| {
            let actor_fn = Arc::clone(&actor_fn);
            Box::pin(async move {
                run_typed_server_actor::<T, P, C, F, Fut>(room, config, actor_fn).await;
            })
        }
    }

    /// Internal: run the typed server actor loop.
    async fn run_typed_server_actor<T, P, C, F, Fut>(
        mut room: multiplayer_kit_server::Room<T>,
        config: C,
        actor_fn: Arc<F>,
    ) where
        T: UserContext,
        P: TypedProtocol,
        C: multiplayer_kit_protocol::RoomConfig + 'static,
        F: Fn(TypedContext<T, P>, TypedEvent<T, P>, Arc<C>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let config = Arc::new(config);
        use multiplayer_kit_server::Accept;

        let (event_tx, mut event_rx) = mpsc::channel::<ServerInternalEvent<T, P>>(256);
        let (self_tx, mut self_rx) = mpsc::channel::<P::Event>(64);

        let user_channels: Arc<DashMap<ChannelId, (T, P::Channel)>> = Arc::new(DashMap::new());

        let pending_users: Arc<DashMap<T::Id, (T, HashMap<P::Channel, ChannelId>)>> =
            Arc::new(DashMap::new());

        let connected_users: Arc<DashMap<T::Id, T>> = Arc::new(DashMap::new());

        let ctx = TypedContext {
            room_id: room.room_id(),
            self_tx,
            handle: room.handle(),
            user_channels: user_channels.clone(),
            _phantom: PhantomData,
        };

        let expected_channels = P::all_channels().len();

        loop {
            tokio::select! {
                accept = room.accept() => {
                    match accept {
                        Some(Accept::NewChannel(user, channel)) => {
                            let channel_id = channel.id;
                            let tx = event_tx.clone();
                            let uc = Arc::clone(&user_channels);
                            let pu = Arc::clone(&pending_users);
                            let cu = Arc::clone(&connected_users);

                            tokio::spawn(async move {
                                handle_typed_channel::<T, P>(
                                    channel, user, channel_id, tx, uc, pu, cu, expected_channels,
                                )
                                .await;
                            });
                        }
                        Some(Accept::Closing) => {
                            actor_fn(ctx.clone(), TypedEvent::Shutdown, Arc::clone(&config)).await;
                            break;
                        }
                        None => break,
                    }
                }
                event = event_rx.recv() => {
                    match event {
                        Some(ServerInternalEvent::UserReady(user)) => {
                            actor_fn(ctx.clone(), TypedEvent::UserConnected(user), Arc::clone(&config)).await;
                        }
                        Some(ServerInternalEvent::UserGone(user)) => {
                            actor_fn(ctx.clone(), TypedEvent::UserDisconnected(user), Arc::clone(&config)).await;
                        }
                        Some(ServerInternalEvent::Message { sender, channel_type, event }) => {
                            actor_fn(
                                ctx.clone(),
                                TypedEvent::Message {
                                    sender,
                                    channel: channel_type,
                                    event,
                                },
                                Arc::clone(&config),
                            )
                            .await;
                        }
                        None => break,
                    }
                }
                self_event = self_rx.recv() => {
                    if let Some(event) = self_event {
                        actor_fn(ctx.clone(), TypedEvent::Internal(event), Arc::clone(&config)).await;
                    }
                }
            }
        }
    }

    async fn handle_typed_channel<T: UserContext, P: TypedProtocol>(
        mut channel: multiplayer_kit_server::ServerChannel,
        user: T,
        channel_id: ChannelId,
        event_tx: mpsc::Sender<ServerInternalEvent<T, P>>,
        user_channels: Arc<DashMap<ChannelId, (T, P::Channel)>>,
        pending_users: Arc<DashMap<T::Id, (T, HashMap<P::Channel, ChannelId>)>>,
        connected_users: Arc<DashMap<T::Id, T>>,
        expected_channels: usize,
    ) {
        let user_id = user.id();
        tracing::debug!("Channel {:?} opened for user {:?}", channel_id, user_id);
        let mut buffer = MessageBuffer::new();

        // First message identifies channel type
        let channel_type: P::Channel;
        'outer: loop {
            let Some(data) = channel.read().await else {
                return;
            };

            for result in buffer.push(&data) {
                match result {
                    Ok(msg) if msg.len() == 1 => {
                        if let Some(ct) = P::channel_from_id(msg[0]) {
                            channel_type = ct;
                            tracing::info!(
                                "Channel {:?} identified as {:?} for user {:?}",
                                channel_id,
                                channel_type,
                                user_id
                            );
                            break 'outer;
                        }
                        tracing::warn!("Unknown channel type: {}", msg[0]);
                        return;
                    }
                    Ok(_) => {
                        tracing::warn!("Invalid channel identification message");
                        return;
                    }
                    Err(e) => {
                        tracing::warn!("Framing error during channel identification: {}", e);
                        return;
                    }
                }
            }
        }

        // Register channel
        user_channels.insert(channel_id, (user.clone(), channel_type));

        // Track for user connection status
        let user_ready = {
            let mut entry = pending_users
                .entry(user_id.clone())
                .or_insert_with(|| (user.clone(), HashMap::new()));
            entry.1.insert(channel_type, channel_id);

            if entry.1.len() >= expected_channels {
                drop(entry); // Release DashMap ref before remove
                let (_, (user, _)) = pending_users.remove(&user_id).unwrap();
                connected_users.insert(user_id.clone(), user.clone());
                Some(user)
            } else {
                None
            }
        };

        if let Some(ref user) = user_ready {
            tracing::info!(
                "User {:?} fully connected ({} channels established)",
                user.id(),
                expected_channels
            );
            let _ = event_tx
                .send(ServerInternalEvent::UserReady(user.clone()))
                .await;
        }

        // Read messages
        loop {
            let Some(data) = channel.read().await else {
                break;
            };

            for result in buffer.push(&data) {
                match result {
                    Ok(msg) => match P::decode(channel_type, &msg) {
                        Ok(event) => {
                            let _ = event_tx
                                .send(ServerInternalEvent::Message {
                                    sender: user.clone(),
                                    channel_type,
                                    event,
                                })
                                .await;
                        }
                        Err(e) => {
                            tracing::warn!("Decode error: {}", e);
                        }
                    },
                    Err(e) => {
                        tracing::warn!("Framing error: {}", e);
                        break;
                    }
                }
            }
        }

        // Channel closed - remove from tracking
        user_channels.remove(&channel_id);

        // Check if user is now disconnected
        let user_gone = if connected_users.remove(&user_id).is_some() {
            // Remove all remaining channels for this user
            let to_remove: Vec<_> = user_channels
                .iter()
                .filter(|r| r.value().0.id() == user_id)
                .map(|r| *r.key())
                .collect();
            for id in to_remove {
                user_channels.remove(&id);
            }
            Some(user.clone())
        } else {
            pending_users.remove(&user_id);
            None
        };

        if let Some(ref user) = user_gone {
            tracing::info!(
                "User {:?} disconnected (channel {:?} closed)",
                user.id(),
                channel_type
            );
            let _ = event_tx
                .send(ServerInternalEvent::UserGone(user.clone()))
                .await;
        }
    }
}

#[cfg(feature = "server")]
pub use server::{TypedContext, TypedEvent, with_typed_actor};

// ============================================================================
// Client-side typed actor (native only)
// ============================================================================

#[cfg(feature = "client")]
mod client {
    use super::*;
    use crate::framing::{MessageBuffer, frame_message};
    use std::collections::HashMap;
    use std::marker::PhantomData;
    use tokio::sync::mpsc;

    /// Events delivered to a typed client actor.
    pub enum TypedClientEvent<P: TypedProtocol> {
        /// All channels connected.
        Connected,
        /// Message received.
        Message(P::Event),
        /// Disconnected (any channel failed).
        Disconnected,
    }

    /// Context for client-side typed actor.
    pub struct TypedClientContext<P: TypedProtocol> {
        channels: HashMap<P::Channel, mpsc::Sender<Vec<u8>>>,
        _phantom: PhantomData<P>,
    }

    impl<P: TypedProtocol> TypedClientContext<P> {
        /// Send an event (routed to correct channel by type).
        pub async fn send(&self, event: &P::Event) -> Result<(), super::EncodeError> {
            let (channel_type, data) = P::encode(event)?;
            let framed = frame_message(&data);
            if let Some(tx) = self.channels.get(&channel_type) {
                let _ = tx.send(framed).await;
            }
            Ok(())
        }
    }

    impl<P: TypedProtocol> Clone for TypedClientContext<P> {
        fn clone(&self) -> Self {
            Self {
                channels: self.channels.clone(),
                _phantom: PhantomData,
            }
        }
    }

    /// Run a typed client actor.
    pub async fn with_typed_client_actor<P, F, Fut>(
        conn: multiplayer_kit_client::RoomConnection,
        actor_fn: F,
    ) where
        P: TypedProtocol,
        F: Fn(TypedClientContext<P>, TypedClientEvent<P>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let channels_to_open = P::all_channels();
        let mut channel_senders: HashMap<P::Channel, mpsc::Sender<Vec<u8>>> = HashMap::new();
        let (event_tx, mut event_rx) = mpsc::channel::<TypedClientEvent<P>>(256);

        let mut tasks = Vec::new();
        for &channel_type in channels_to_open {
            let channel = match conn.open_channel().await {
                Ok(ch) => ch,
                Err(e) => {
                    tracing::error!("Failed to open channel: {}", e);
                    let ctx = TypedClientContext {
                        channels: HashMap::new(),
                        _phantom: PhantomData,
                    };
                    actor_fn(ctx, TypedClientEvent::Disconnected).await;
                    return;
                }
            };

            let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>(256);
            channel_senders.insert(channel_type, write_tx);

            let tx = event_tx.clone();
            let channel_id_byte = P::channel_to_id(channel_type);

            tasks.push(tokio::spawn(async move {
                run_client_channel::<P>(channel, channel_type, channel_id_byte, tx, write_rx).await
            }));
        }

        let ctx = TypedClientContext {
            channels: channel_senders,
            _phantom: PhantomData,
        };

        actor_fn(ctx.clone(), TypedClientEvent::Connected).await;

        while let Some(event) = event_rx.recv().await {
            let is_disconnect = matches!(event, TypedClientEvent::Disconnected);
            actor_fn(ctx.clone(), event).await;
            if is_disconnect {
                break;
            }
        }

        for task in tasks {
            task.abort();
        }
    }

    async fn run_client_channel<P: TypedProtocol>(
        channel: multiplayer_kit_client::Channel,
        channel_type: P::Channel,
        channel_id_byte: u8,
        event_tx: mpsc::Sender<TypedClientEvent<P>>,
        mut write_rx: mpsc::Receiver<Vec<u8>>,
    ) {
        // Send channel identification
        let id_msg = frame_message(&[channel_id_byte]);
        if channel.write(&id_msg).await.is_err() {
            let _ = event_tx.send(TypedClientEvent::Disconnected).await;
            return;
        }

        let mut buffer = MessageBuffer::new();
        let mut read_buf = vec![0u8; 64 * 1024];

        loop {
            tokio::select! {
                result = channel.read(&mut read_buf) => {
                    match result {
                        Ok(n) if n > 0 => {
                            for result in buffer.push(&read_buf[..n]) {
                                match result {
                                    Ok(data) => {
                                        match P::decode(channel_type, &data) {
                                            Ok(event) => {
                                                let _ = event_tx.send(TypedClientEvent::Message(event)).await;
                                            }
                                            Err(e) => {
                                                tracing::warn!("Decode error: {}", e);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!("Framing error: {}", e);
                                        let _ = event_tx.send(TypedClientEvent::Disconnected).await;
                                        return;
                                    }
                                }
                            }
                        }
                        _ => {
                            let _ = event_tx.send(TypedClientEvent::Disconnected).await;
                            return;
                        }
                    }
                }
                Some(data) = write_rx.recv() => {
                    if channel.write(&data).await.is_err() {
                        let _ = event_tx.send(TypedClientEvent::Disconnected).await;
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(feature = "client")]
pub use client::{TypedClientContext, TypedClientEvent, with_typed_client_actor};

// ============================================================================
// Client-side typed actor (WASM)
// ============================================================================

#[cfg(feature = "wasm")]
mod wasm_client {
    use crate::framing::{MessageBuffer, frame_message};
    use futures::{FutureExt, StreamExt};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use wasm_bindgen::JsValue;
    use wasm_bindgen::prelude::*;

    /// WASM typed client actor.
    ///
    /// Connects to a room, opens typed channels, and delivers events to a callback.
    #[wasm_bindgen]
    pub struct JsTypedClientActor {
        /// Send channel for outgoing messages, keyed by channel id byte.
        write_channels: Rc<RefCell<HashMap<u8, futures::channel::mpsc::UnboundedSender<Vec<u8>>>>>,
        /// Receive channel for events.
        event_rx: Rc<RefCell<futures::channel::mpsc::UnboundedReceiver<JsValue>>>,
        /// Whether we're still connected.
        connected: Rc<RefCell<bool>>,
    }

    #[wasm_bindgen]
    impl JsTypedClientActor {
        /// Connect and open all channels for a protocol.
        ///
        /// `channel_ids` is an array of channel ID bytes to open.
        /// Returns a client actor that can receive events and send messages.
        #[wasm_bindgen(js_name = connect)]
        pub async fn connect(
            conn: &multiplayer_kit_client::wasm_exports::JsRoomConnection,
            channel_ids: Vec<u8>,
        ) -> Result<JsTypedClientActor, JsError> {
            let (event_tx, event_rx) = futures::channel::mpsc::unbounded::<JsValue>();
            let write_channels: Rc<
                RefCell<HashMap<u8, futures::channel::mpsc::UnboundedSender<Vec<u8>>>>,
            > = Rc::new(RefCell::new(HashMap::new()));
            let connected = Rc::new(RefCell::new(true));

            for &channel_id in &channel_ids {
                let channel: multiplayer_kit_client::wasm_exports::JsChannel =
                    conn.open_channel().await?;

                let (write_tx, write_rx) = futures::channel::mpsc::unbounded::<Vec<u8>>();
                write_channels.borrow_mut().insert(channel_id, write_tx);

                let tx = event_tx.clone();
                let conn_flag = Rc::clone(&connected);

                wasm_bindgen_futures::spawn_local(async move {
                    run_wasm_client_channel(channel, channel_id, tx, write_rx, conn_flag).await;
                });
            }

            // Send Connected event
            let connected_event = js_sys::Object::new();
            js_sys::Reflect::set(&connected_event, &"type".into(), &"connected".into()).unwrap();
            let _ = event_tx.unbounded_send(connected_event.into());

            Ok(JsTypedClientActor {
                write_channels,
                event_rx: Rc::new(RefCell::new(event_rx)),
                connected,
            })
        }

        /// Receive the next event.
        ///
        /// Returns an object with:
        /// - `type`: "connected" | "message" | "disconnected"
        /// - `channelId`: (for message) the channel ID byte
        /// - `data`: (for message) Uint8Array of message bytes
        ///
        /// Returns `null` when disconnected.
        #[wasm_bindgen(js_name = recv)]
        pub async fn recv(&self) -> Result<JsValue, JsError> {
            use futures::StreamExt;

            let event = {
                let mut rx = self.event_rx.borrow_mut();
                rx.next().await
            };

            match event {
                Some(e) => Ok(e),
                None => Ok(JsValue::NULL),
            }
        }

        /// Send a message on a specific channel.
        ///
        /// `channel_id` is the channel ID byte.
        /// `data` is the raw message bytes (will be framed automatically).
        #[wasm_bindgen(js_name = send)]
        pub fn send(&self, channel_id: u8, data: &[u8]) -> Result<(), JsError> {
            let framed = frame_message(data);
            let channels = self.write_channels.borrow();
            if let Some(tx) = channels.get(&channel_id) {
                tx.unbounded_send(framed)
                    .map_err(|_| JsError::new("Channel closed"))?;
            } else {
                return Err(JsError::new(&format!("Unknown channel: {}", channel_id)));
            }
            Ok(())
        }

        /// Check if still connected.
        #[wasm_bindgen(getter)]
        pub fn connected(&self) -> bool {
            *self.connected.borrow()
        }
    }

    async fn run_wasm_client_channel(
        channel: multiplayer_kit_client::wasm_exports::JsChannel,
        channel_id: u8,
        event_tx: futures::channel::mpsc::UnboundedSender<JsValue>,
        mut write_rx: futures::channel::mpsc::UnboundedReceiver<Vec<u8>>,
        connected: Rc<RefCell<bool>>,
    ) {
        // Send channel identification
        let id_msg = frame_message(&[channel_id]);
        if channel.write(&id_msg).await.is_err() {
            send_disconnect(&event_tx, &connected);
            return;
        }

        let mut buffer = MessageBuffer::new();

        loop {
            futures::select! {
                result = channel.read().fuse() => {
                    match result {
                        Ok(data) if !data.is_empty() => {
                            let results: Vec<_> = buffer.push(&data).collect();
                            for result in results {
                                match result {
                                    Ok(msg_data) => {
                                        // Create message event
                                        let event = js_sys::Object::new();
                                        js_sys::Reflect::set(&event, &"type".into(), &"message".into()).unwrap();
                                        js_sys::Reflect::set(&event, &"channelId".into(), &JsValue::from(channel_id)).unwrap();
                                        let arr = js_sys::Uint8Array::from(&msg_data[..]);
                                        js_sys::Reflect::set(&event, &"data".into(), &arr).unwrap();
                                        let _ = event_tx.unbounded_send(event.into());
                                    }
                                    Err(_) => {
                                        send_disconnect(&event_tx, &connected);
                                        return;
                                    }
                                }
                            }
                        }
                        _ => {
                            send_disconnect(&event_tx, &connected);
                            return;
                        }
                    }
                }
                msg = write_rx.next().fuse() => {
                    if let Some(data) = msg {
                        if channel.write(&data).await.is_err() {
                            send_disconnect(&event_tx, &connected);
                            return;
                        }
                    } else {
                        // Write channel closed
                        return;
                    }
                }
            }
        }
    }

    fn send_disconnect(
        event_tx: &futures::channel::mpsc::UnboundedSender<JsValue>,
        connected: &Rc<RefCell<bool>>,
    ) {
        if *connected.borrow() {
            *connected.borrow_mut() = false;
            let event = js_sys::Object::new();
            js_sys::Reflect::set(&event, &"type".into(), &"disconnected".into()).unwrap();
            let _ = event_tx.unbounded_send(event.into());
        }
    }
}

#[cfg(feature = "wasm")]
pub use wasm_client::JsTypedClientActor;
