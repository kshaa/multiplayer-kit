//! Testing utilities for running actors without real network connections.
//!
//! Provides a test harness that mimics the production routing behavior
//! but uses simple channels instead of real network connections.

mod harness;

#[cfg(test)]
mod tests;

pub use harness::{TestHarness, TestClientConnection, TestUser, TestUserId};
