//! Integration tests for chat server and client actors.
//!
//! Tests the actual `ChatActor` (server) and `ChatClientActor` (client)
//! communicating through a test harness without real network connections.

#[cfg(test)]
mod harness;

#[cfg(test)]
mod tests;
