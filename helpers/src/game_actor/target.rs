//! Shared target types for game actor message routing.

use crate::actor::ActorProtocol;
use std::marker::PhantomData;

/// Target for sending messages to self (internal scheduling).
///
/// Used by both server and client actors to schedule internal messages.
pub struct SelfTarget<M>(PhantomData<M>);

impl<M> ActorProtocol for SelfTarget<M> {
    type ActorId = ();
    type Message = M;
}
