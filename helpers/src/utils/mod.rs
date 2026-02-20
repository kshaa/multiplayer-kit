//! Utility modules.

mod framing;
mod platform;
mod typed_protocol;

pub use framing::{FramingError, MessageBuffer, frame_message};
pub use platform::{GameClientContext, MaybeSend, MaybeSync};
pub use typed_protocol::{DecodeError, EncodeError, TypedProtocol};
