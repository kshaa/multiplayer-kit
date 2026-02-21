//! Chat server library - exposes the server actor for reuse and testing.

mod actor;

pub use actor::{ChatActor, ChatRoomExtras, ChatState};
