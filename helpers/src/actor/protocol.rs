//! Actor protocol trait.

/// Defines an actor's message interface.
///
/// An actor protocol specifies:
/// - `ActorId` - how to identify/address this actor
/// - `Message` - what messages this actor accepts
///
/// This trait is implemented by both full actors and simple message targets.
/// The `Sink` trait uses this to enable type-safe message routing.
pub trait ActorProtocol {
    /// How to identify/address this actor.
    type ActorId;

    /// Message type this actor accepts.
    type Message;
}
