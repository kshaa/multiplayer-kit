//! Shared types for game actors.
//!
//! Contains types used by both server and client actors.

mod target;

#[cfg(test)]
mod tests;

pub use target::SelfTarget;
