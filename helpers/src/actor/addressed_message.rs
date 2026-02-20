//! Addressed message type delivered to actors.

/// A message with sender information, delivered to an actor's handle function.
///
/// Contains the sender (`from`) and the message content.
#[derive(Debug, Clone, PartialEq)]
pub struct AddressedMessage<ActorId, Message> {
    /// Who sent this message.
    pub from: ActorId,
    /// The message content.
    pub content: Message,
}

impl<ActorId, Message> AddressedMessage<ActorId, Message> {
    /// Create a new addressed message.
    pub fn new(from: ActorId, content: Message) -> Self {
        Self { from, content }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct ActorId(u64);

    #[derive(Debug, Clone, PartialEq)]
    enum Msg {
        Hello,
        Tick,
    }

    #[test]
    fn create_message() {
        let msg = AddressedMessage::new(ActorId(1), Msg::Hello);
        assert_eq!(msg.from, ActorId(1));
        assert_eq!(msg.content, Msg::Hello);
    }

    #[test]
    fn message_fields_accessible() {
        let msg = AddressedMessage {
            from: ActorId(42),
            content: Msg::Tick,
        };
        assert_eq!(msg.from.0, 42);
        assert!(matches!(msg.content, Msg::Tick));
    }
}
