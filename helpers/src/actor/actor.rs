//! Core Actor trait.

use super::addressed_message::AddressedMessage;
use super::protocol::ActorProtocol;

/// Pure actor definition.
///
/// An actor is defined by:
/// - `ActorId` - how to identify the sender (from `ActorProtocol`)
/// - `Message` - what messages this actor handles (from `ActorProtocol`)
/// - `Config` - initialization parameters (only used in `startup`)
/// - `State` - internal mutable state (should be serializable)
/// - `Extras` - external tools/triggers for impure operations
///
/// The trait is generic over `S` (sink type) so implementations can specify
/// what sink capabilities they need (which actors they can send to).
///
/// # Design Principles
///
/// 1. **Explicit state** - State is a separate type, not part of the actor struct.
///    `startup` returns it, `handle` takes `&mut State`.
///
/// 2. **Functional core** - The handle function is pure logic: takes state + message,
///    mutates state, sends outputs via sink.
///
/// 3. **Extras for impurity** - External tools (timers, triggers, APIs) go in `Extras`,
///    keeping `State` clean and serializable.
///
/// 4. **Type-safe routing** - Messages are sent via `Sink::send(to, message)`.
///    The actor protocol determines valid message types at compile time.
///
/// # Example
///
/// ```ignore
/// struct MyActor;
///
/// impl ActorProtocol for MyActor {
///     type ActorId = SenderId;
///     type Message = MyMessage;
/// }
///
/// impl<S> Actor<S> for MyActor
/// where
///     S: Sink<OtherActor> + Sink<Self>,
/// {
///     type Config = MyConfig;
///     type State = MyState;
///     type Extras = MyExtras;  // e.g., timers, external APIs
///
///     fn startup(config: Self::Config) -> Self::State {
///         MyState::new(&config)
///     }
///
///     fn handle(
///         state: &mut Self::State,
///         extras: &mut Self::Extras,
///         message: AddressedMessage<Self::ActorId, Self::Message>,
///         sink: &mut S,
///     ) {
///         match message.content {
///             MyMessage::Data(data) => { /* ... */ }
///             MyMessage::Tick => { extras.trigger_fireworks(); }
///         }
///     }
///
///     fn shutdown(state: &mut Self::State, extras: &mut Self::Extras) {
///         // Cleanup external resources
///     }
/// }
/// ```
pub trait Actor<S>: ActorProtocol {
    /// Configuration used only in `startup` to initialize state.
    type Config;

    /// Internal state, created by startup, mutated by handle. Should be serializable.
    type State;

    /// External tools/triggers for impure operations (timers, APIs, etc.).
    type Extras;

    /// Initialize state from configuration.
    fn startup(config: Self::Config) -> Self::State;

    /// Handle a message, mutating state and sending outputs via sink.
    fn handle(
        state: &mut Self::State,
        extras: &mut Self::Extras,
        message: AddressedMessage<Self::ActorId, Self::Message>,
        sink: &mut S,
    );

    /// Clean up when the actor is shutting down.
    fn shutdown(state: &mut Self::State, extras: &mut Self::Extras);
}
