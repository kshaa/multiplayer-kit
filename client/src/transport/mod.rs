//! Transport abstraction for WebTransport and WebSocket.
//!
//! This module provides a unified interface for different transport protocols,
//! allowing lobby and room code to share connection logic.

pub mod webtransport;
pub mod websocket;
