//! Server-side sink for game actors.

use crate::actor::Sink;
use crate::game_actor::SelfTarget;
use super::types::{UserTarget, UserDestination};

/// Output from a server game actor.
#[derive(Debug)]
pub enum ServerOutput<User, M> {
    /// Send to a specific user.
    ToUser { user: User, message: M },
    /// Broadcast to all users.
    Broadcast { message: M },
    /// Internal message (to self).
    Internal { message: M },
}

/// Sink for server game actors.
///
/// Collects messages destined for users (individual or broadcast) and self.
/// The glue code drains this after each `handle` call and routes appropriately.
pub struct ServerSink<User, M> {
    outputs: Vec<ServerOutput<User, M>>,
}

impl<User, M> ServerSink<User, M> {
    pub fn new() -> Self {
        Self { outputs: Vec::new() }
    }

    pub fn drain(&mut self) -> impl Iterator<Item = ServerOutput<User, M>> + '_ {
        self.outputs.drain(..)
    }
}

impl<User, M> Default for ServerSink<User, M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<User, M> Sink<UserTarget<User, M>> for ServerSink<User, M> {
    fn send(&mut self, to: UserDestination<User>, message: M) {
        match to {
            UserDestination::User(user) => {
                self.outputs.push(ServerOutput::ToUser { user, message });
            }
            UserDestination::Broadcast => {
                self.outputs.push(ServerOutput::Broadcast { message });
            }
        }
    }
}

impl<User, M> Sink<SelfTarget<M>> for ServerSink<User, M> {
    fn send(&mut self, _to: (), message: M) {
        self.outputs.push(ServerOutput::Internal { message });
    }
}
