//! Client-side sink for game actors.

use crate::actor::Sink;
use crate::game_actor::SelfTarget;
use super::types::ServerTarget;

/// Output from a client game actor.
#[derive(Debug)]
pub enum ClientOutput<M> {
    /// Send to server.
    ToServer { message: M },
    /// Internal message (to self).
    Internal { message: M },
}

/// Sink for client game actors.
///
/// Collects messages destined for the server and self.
/// The glue code drains this after each `handle` call and routes appropriately.
pub struct ClientSink<M> {
    outputs: Vec<ClientOutput<M>>,
}

impl<M> ClientSink<M> {
    pub fn new() -> Self {
        Self { outputs: Vec::new() }
    }

    pub fn drain(&mut self) -> impl Iterator<Item = ClientOutput<M>> + '_ {
        self.outputs.drain(..)
    }
}

impl<M> Default for ClientSink<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M> Sink<ServerTarget<M>> for ClientSink<M> {
    fn send(&mut self, _to: (), message: M) {
        self.outputs.push(ClientOutput::ToServer { message });
    }
}

impl<M> Sink<SelfTarget<M>> for ClientSink<M> {
    fn send(&mut self, _to: (), message: M) {
        self.outputs.push(ClientOutput::Internal { message });
    }
}
