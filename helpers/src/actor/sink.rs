//! Sink trait for sending messages.

use super::protocol::ActorProtocol;

/// Output sink for sending messages to actors.
///
/// The sink is generic over the actor protocol, enabling type-safe routing:
/// the protocol determines what actor ID and message types are valid.
///
/// # Type Safety
///
/// ```ignore
/// // Compiles - sending correct message type to ActorB
/// sink.send(actor_b_id, MsgToB::Hello);
///
/// // Won't compile - wrong message type for ActorB
/// sink.send(actor_b_id, MsgToA::Hello);
/// ```
///
/// # Implementors
///
/// - `BasicSink` - captures messages for later processing
/// - Platform-specific sinks in glue code
pub trait Sink<A: ActorProtocol> {
    /// Send a message to an actor.
    fn send(&mut self, to: A::ActorId, message: A::Message);
}
