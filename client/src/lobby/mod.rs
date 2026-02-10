//! Lobby client for receiving room updates.
//!
//! The lobby uses a unidirectional stream from the server to push room
//! events (room created, updated, deleted) to clients.

mod client;

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
mod wasm;

pub use client::{LobbyClient, LobbyConfig};

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
pub use wasm::JsLobbyClient;
