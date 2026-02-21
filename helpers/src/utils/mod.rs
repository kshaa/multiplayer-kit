//! Utility modules.

mod channel_message;
mod framing;
mod platform;

pub use channel_message::{ChannelMessage, DecodeError, EncodeError};
pub use framing::{FramingError, MessageBuffer, frame_message};
pub use platform::{GameClientContext, MaybeSend, MaybeSync};
