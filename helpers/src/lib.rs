//! Helper utilities for multiplayer-kit.
//!
//! Provides optional message framing on top of raw byte streams.
//!
//! # Example (Client)
//!
//! ```ignore
//! use multiplayer_kit_helpers::MessageChannel;
//!
//! let channel = room_conn.open_channel().await?;
//! let mut msg_channel = MessageChannel::new(channel);
//!
//! // Send a message (automatically framed)
//! msg_channel.send(b"hello world").await?;
//!
//! // Receive a complete message
//! if let Some(msg) = msg_channel.recv().await? {
//!     println!("got: {:?}", msg);
//! }
//! ```
//!
//! # Example (Server Actor)
//!
//! ```ignore
//! use multiplayer_kit_helpers::{with_actor, with_framing, RoomContext, MessageContext, MessageEvent, Outgoing, Route};
//!
//! Server::builder()
//!     .room_handler(with_actor(with_framing(|mut ctx: MessageContext<MyUser>| async move {
//!         while let Some(event) = ctx.recv().await {
//!             match event {
//!                 MessageEvent::Message { sender, channel, data } => {
//!                     // `data` is a complete, unframed message
//!                     ctx.send(Outgoing::new(data, Route::AllExcept(channel))).await;
//!                 }
//!                 _ => {}
//!             }
//!         }
//!     })))
//!     .build()
//! ```

mod framing;

pub use framing::{frame_message, MessageBuffer, FramingError};

#[cfg(any(feature = "client", feature = "wasm"))]
mod channel;

#[cfg(any(feature = "client", feature = "wasm"))]
pub use channel::{MessageChannel, MessageChannelError};

#[cfg(feature = "server")]
mod actor;

#[cfg(feature = "server")]
pub use actor::{
    with_actor, with_framing,
    RoomContext, RoomEvent,
    MessageContext, MessageEvent,
    Outgoing, Route,
};
