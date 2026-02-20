//! Integration tests demonstrating actor usage.
//!
//! Three actors (A, B, C) that can send messages to each other and themselves.

use super::*;

// =============================================================================
// Actor IDs
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct IdA(u32);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct IdB(u32);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct IdC(u32);

// =============================================================================
// Messages each actor accepts
// =============================================================================

#[derive(Debug, Clone, PartialEq)]
enum MsgA {
    FromExternal(String),
    Reminder(String),
}

#[derive(Debug, Clone, PartialEq)]
struct MsgB(String);

#[derive(Debug, Clone, PartialEq)]
enum MsgC {
    Data(String),
    Log(String),
}

// =============================================================================
// Actor A
// =============================================================================

struct ConfigA {
    name: String,
}

struct StateA {
    name: String,
    received: Vec<String>,
}

struct ActorA;

impl ActorProtocol for ActorA {
    type ActorId = IdA;
    type Message = MsgA;
}

impl<S> Actor<S> for ActorA
where
    S: Sink<ActorB> + Sink<ActorC> + Sink<Self>,
{
    type Config = ConfigA;
    type State = StateA;
    type Extras = ();

    fn startup(config: Self::Config) -> Self::State {
        StateA {
            name: config.name,
            received: Vec::new(),
        }
    }

    fn handle(
        state: &mut Self::State,
        _extras: &mut Self::Extras,
        message: AddressedMessage<Self::ActorId, Self::Message>,
        sink: &mut S,
    ) {
        match &message.content {
            MsgA::FromExternal(text) => {
                println!("[{}] Received: {}", state.name, text);
                state.received.push(format!("external: {}", text));

                // Forward to B
                Sink::<ActorB>::send(sink, IdB(0), MsgB(format!("A forwards: {}", text)));
            }

            MsgA::Reminder(text) => {
                println!("[{}] Self reminder: {}", state.name, text);
                state.received.push(format!("self: {}", text));
            }
        }
    }

    fn shutdown(state: &mut Self::State, _extras: &mut Self::Extras) {
        println!(
            "[{}] Shutting down with {} messages received",
            state.name,
            state.received.len()
        );
    }
}

// =============================================================================
// Actor B
// =============================================================================

struct ConfigB {
    name: String,
}

struct StateB {
    name: String,
    count: u32,
}

struct ActorB;

impl ActorProtocol for ActorB {
    type ActorId = IdB;
    type Message = MsgB;
}

impl<S> Actor<S> for ActorB
where
    S: Sink<ActorA> + Sink<ActorC> + Sink<Self>,
{
    type Config = ConfigB;
    type State = StateB;
    type Extras = ();

    fn startup(config: Self::Config) -> Self::State {
        StateB {
            name: config.name,
            count: 0,
        }
    }

    fn handle(
        state: &mut Self::State,
        _extras: &mut Self::Extras,
        message: AddressedMessage<Self::ActorId, Self::Message>,
        sink: &mut S,
    ) {
        state.count += 1;
        println!("[{}] #{}: {}", state.name, state.count, message.content.0);

        // Send to C
        Sink::<ActorC>::send(
            sink,
            IdC(0),
            MsgC::Data(format!("B processed {} messages", state.count)),
        );
    }

    fn shutdown(_state: &mut Self::State, _extras: &mut Self::Extras) {}
}

// =============================================================================
// Actor C
// =============================================================================

struct ConfigC {
    name: String,
}

struct StateC {
    name: String,
    log: Vec<String>,
}

struct ActorC;

impl ActorProtocol for ActorC {
    type ActorId = IdC;
    type Message = MsgC;
}

impl<S> Actor<S> for ActorC
where
    S: Sink<ActorA> + Sink<ActorB> + Sink<Self>,
{
    type Config = ConfigC;
    type State = StateC;
    type Extras = ();

    fn startup(config: Self::Config) -> Self::State {
        StateC {
            name: config.name,
            log: Vec::new(),
        }
    }

    fn handle(
        state: &mut Self::State,
        _extras: &mut Self::Extras,
        message: AddressedMessage<Self::ActorId, Self::Message>,
        sink: &mut S,
    ) {
        match &message.content {
            MsgC::Data(text) => {
                println!("[{}] Data: {}", state.name, text);
                // Schedule logging
                Sink::<ActorC>::send(sink, IdC(0), MsgC::Log(text.clone()));
            }

            MsgC::Log(text) => {
                state.log.push(text.clone());
                println!(
                    "[{}] Logged: {} (total: {})",
                    state.name,
                    text,
                    state.log.len()
                );
            }
        }
    }

    fn shutdown(_state: &mut Self::State, _extras: &mut Self::Extras) {}
}

// =============================================================================
// Tests
// =============================================================================

#[test]
fn actor_a_receives_and_forwards() {
    let config = ConfigA {
        name: "ActorA".into(),
    };
    let mut state = <ActorA as Actor<BasicSink>>::startup(config);
    let mut sink = BasicSink::new();

    <ActorA as Actor<BasicSink>>::handle(
        &mut state,
        &mut (),
        AddressedMessage::new(IdA(0), MsgA::FromExternal("hello".into())),
        &mut sink,
    );

    assert_eq!(state.received.len(), 1);
    assert!(state.received[0].contains("hello"));

    let (_, msg) = sink.receive::<ActorB>().unwrap();
    assert!(msg.0.contains("forwards"));
}

#[test]
fn actor_a_handles_self_message() {
    let config = ConfigA {
        name: "ActorA".into(),
    };
    let mut state = <ActorA as Actor<BasicSink>>::startup(config);
    let mut sink = BasicSink::new();

    <ActorA as Actor<BasicSink>>::handle(
        &mut state,
        &mut (),
        AddressedMessage::new(IdA(0), MsgA::Reminder("don't forget".into())),
        &mut sink,
    );

    assert_eq!(state.received.len(), 1);
    assert!(state.received[0].contains("don't forget"));
}

#[test]
fn actor_b_counts_and_forwards() {
    let config = ConfigB {
        name: "ActorB".into(),
    };
    let mut state = <ActorB as Actor<BasicSink>>::startup(config);
    let mut sink = BasicSink::new();

    <ActorB as Actor<BasicSink>>::handle(
        &mut state,
        &mut (),
        AddressedMessage::new(IdB(0), MsgB("first".into())),
        &mut sink,
    );

    assert_eq!(state.count, 1);

    let (_, msg) = sink.receive::<ActorC>().unwrap();
    assert!(matches!(msg, MsgC::Data(s) if s.contains("1")));
}

#[test]
fn actor_c_logs_messages() {
    let config = ConfigC {
        name: "ActorC".into(),
    };
    let mut state = <ActorC as Actor<BasicSink>>::startup(config);
    let mut sink = BasicSink::new();

    <ActorC as Actor<BasicSink>>::handle(
        &mut state,
        &mut (),
        AddressedMessage::new(IdC(0), MsgC::Data("update".into())),
        &mut sink,
    );

    // Should have scheduled a Log message to self
    let (_, msg) = sink.receive::<ActorC>().unwrap();
    assert!(matches!(&msg, MsgC::Log(s) if s == "update"));

    // Process the log message
    <ActorC as Actor<BasicSink>>::handle(
        &mut state,
        &mut (),
        AddressedMessage::new(IdC(0), msg),
        &mut sink,
    );

    assert_eq!(state.log.len(), 1);
    assert_eq!(state.log[0], "update");
}

#[test]
fn full_message_chain_a_to_b_to_c() {
    let config_a = ConfigA { name: "A".into() };
    let config_b = ConfigB { name: "B".into() };
    let config_c = ConfigC { name: "C".into() };

    let mut state_a = <ActorA as Actor<BasicSink>>::startup(config_a);
    let mut state_b = <ActorB as Actor<BasicSink>>::startup(config_b);
    let mut state_c = <ActorC as Actor<BasicSink>>::startup(config_c);

    let mut sink = BasicSink::new();

    // 1. External sends to A
    <ActorA as Actor<BasicSink>>::handle(
        &mut state_a,
        &mut (),
        AddressedMessage::new(IdA(0), MsgA::FromExternal("start".into())),
        &mut sink,
    );

    // A forwards to B
    let (_, msg_to_b) = sink.receive::<ActorB>().unwrap();

    // 2. B receives and forwards to C
    <ActorB as Actor<BasicSink>>::handle(
        &mut state_b,
        &mut (),
        AddressedMessage::new(IdB(0), msg_to_b),
        &mut sink,
    );

    // B sends to C
    let (_, msg_to_c) = sink.receive::<ActorC>().unwrap();

    // 3. C receives and schedules log
    <ActorC as Actor<BasicSink>>::handle(
        &mut state_c,
        &mut (),
        AddressedMessage::new(IdC(0), msg_to_c),
        &mut sink,
    );

    let (_, log_msg) = sink.receive::<ActorC>().unwrap();

    // 4. C processes log
    <ActorC as Actor<BasicSink>>::handle(
        &mut state_c,
        &mut (),
        AddressedMessage::new(IdC(0), log_msg),
        &mut sink,
    );

    // Verify final states
    assert_eq!(state_a.received.len(), 1);
    assert_eq!(state_b.count, 1);
    assert_eq!(state_c.log.len(), 1);
}

#[test]
fn shutdown_is_called() {
    let config = ConfigA {
        name: "TestActor".into(),
    };
    let mut state = <ActorA as Actor<BasicSink>>::startup(config);
    state.received.push("msg1".into());
    state.received.push("msg2".into());

    <ActorA as Actor<BasicSink>>::shutdown(&mut state, &mut ());
}
