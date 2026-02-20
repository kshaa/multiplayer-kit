//! Basic sink implementation for actors.

use std::any::{Any, TypeId};

use super::protocol::ActorProtocol;
use super::sink::Sink;

/// A captured output from the sink.
#[derive(Debug)]
pub struct Output {
    /// Type of the actor protocol.
    pub actor_type: TypeId,
    /// The actor ID (who we're sending to).
    pub actor_id: Box<dyn Any + Send>,
    /// Type of the message.
    pub message_type: TypeId,
    /// The message content.
    pub message: Box<dyn Any + Send>,
}

/// Basic sink that captures all sent messages.
///
/// Useful for testing and simple use cases where messages are collected for later processing.
///
/// # Example
///
/// ```ignore
/// let mut sink = BasicSink::new();
/// sink.send(actor_b_id, MsgToB::Hello);
///
/// assert_eq!(sink.len(), 1);
/// let (id, msg) = sink.receive::<ActorB>().unwrap();
/// ```
#[derive(Default)]
pub struct BasicSink {
    /// All captured outputs.
    pub outputs: Vec<Output>,
}

impl BasicSink {
    /// Create a new empty test sink.
    pub fn new() -> Self {
        Self {
            outputs: Vec::new(),
        }
    }

    /// Clear all captured outputs.
    pub fn clear(&mut self) {
        self.outputs.clear();
    }

    /// Get the number of captured outputs.
    pub fn len(&self) -> usize {
        self.outputs.len()
    }

    /// Check if no outputs were captured.
    pub fn is_empty(&self) -> bool {
        self.outputs.is_empty()
    }

    /// Take the first message sent to the given actor type.
    pub fn receive<A: ActorProtocol + 'static>(&mut self) -> Option<(A::ActorId, A::Message)>
    where
        A::ActorId: 'static,
        A::Message: 'static,
    {
        let idx = self.outputs.iter().position(|o| {
            o.actor_type == TypeId::of::<A>()
        })?;

        let output = self.outputs.remove(idx);
        let id = output.actor_id.downcast::<A::ActorId>().ok().map(|b| *b)?;
        let msg = output.message.downcast::<A::Message>().ok().map(|b| *b)?;
        Some((id, msg))
    }
}

impl<A: ActorProtocol + 'static> Sink<A> for BasicSink
where
    A::ActorId: Send + 'static,
    A::Message: Send + 'static,
{
    fn send(&mut self, to: A::ActorId, message: A::Message) {
        self.outputs.push(Output {
            actor_type: TypeId::of::<A>(),
            actor_id: Box::new(to),
            message_type: TypeId::of::<A::Message>(),
            message: Box::new(message),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct ActorBId(u64);

    #[derive(Debug, Clone, PartialEq)]
    struct MsgToB(String);

    struct ActorB;
    impl ActorProtocol for ActorB {
        type ActorId = ActorBId;
        type Message = MsgToB;
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct MyId(u64);

    #[derive(Debug, Clone, PartialEq)]
    struct MyMsg(String);

    struct MyActor;
    impl ActorProtocol for MyActor {
        type ActorId = MyId;
        type Message = MyMsg;
    }

    #[test]
    fn capture_message() {
        let mut sink = BasicSink::new();
        Sink::<ActorB>::send(&mut sink, ActorBId(42), MsgToB("hello".into()));

        assert_eq!(sink.len(), 1);
        let (id, msg) = sink.receive::<ActorB>().unwrap();
        assert_eq!(id, ActorBId(42));
        assert_eq!(msg, MsgToB("hello".into()));
        assert!(sink.is_empty());
    }

    #[test]
    fn multiple_outputs() {
        let mut sink = BasicSink::new();
        Sink::<MyActor>::send(&mut sink, MyId(1), MyMsg("tick".into()));
        Sink::<ActorB>::send(&mut sink, ActorBId(2), MsgToB("bye".into()));

        assert_eq!(sink.len(), 2);

        let (id, _) = sink.receive::<ActorB>().unwrap();
        assert_eq!(id, ActorBId(2));

        let (id, _) = sink.receive::<MyActor>().unwrap();
        assert_eq!(id, MyId(1));

        assert!(sink.is_empty());
    }

    #[test]
    fn clear_outputs() {
        let mut sink = BasicSink::new();
        Sink::<MyActor>::send(&mut sink, MyId(1), MyMsg("a".into()));
        Sink::<MyActor>::send(&mut sink, MyId(2), MyMsg("b".into()));

        assert_eq!(sink.len(), 2);
        sink.clear();
        assert!(sink.is_empty());
    }
}
