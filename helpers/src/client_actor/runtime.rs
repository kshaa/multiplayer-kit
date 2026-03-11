//! Actor runtime for sync-friendly actor management.

use super::bridge::BridgeEvent;
use super::connector::BridgeConnector;
use super::sink::{ClientOutput, ClientSink};
use super::types::{ClientMessage, ClientSource};
use super::{ActorSendError, LocalSender};
use crate::actor::{Actor, ActorProtocol, AddressedMessage};
use crate::utils::{ChannelMessage, MaybeSend};
use tokio::sync::mpsc;

/// Status returned by poll/step operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeStatus {
    Connected,
    Disconnected,
}

/// Actor runtime: owns actor state and can be polled synchronously.
pub struct ActorRuntime<A, M, S>
where
    A: Actor<S> + ActorProtocol<ActorId = ClientSource, Message = ClientMessage<M>>,
    M: ChannelMessage,
    S: Default,
{
    state: A::State,
    extras: A::Extras,
    sink: S,
    bridge_rx: mpsc::UnboundedReceiver<BridgeEvent<M>>,
    self_rx: mpsc::UnboundedReceiver<M>,
    local_rx: mpsc::UnboundedReceiver<M>,
    server_tx: mpsc::UnboundedSender<M>,
    self_tx: mpsc::UnboundedSender<M>,
    local_tx: mpsc::UnboundedSender<M>,
    connected: bool,
}

impl<A, M> ActorRuntime<A, M, ClientSink<M>>
where
    A: Actor<ClientSink<M>> + ActorProtocol<ActorId = ClientSource, Message = ClientMessage<M>>,
    M: ChannelMessage,
    A::State: MaybeSend,
    A::Extras: MaybeSend,
{
    /// Create a new actor runtime and bridge connector.
    pub fn new(config: A::Config, extras: A::Extras) -> (Self, BridgeConnector<M>) {
        let (bridge_tx, bridge_rx) = mpsc::unbounded_channel();
        let (server_tx, server_rx) = mpsc::unbounded_channel();
        let (self_tx, self_rx) = mpsc::unbounded_channel();
        let (local_tx, local_rx) = mpsc::unbounded_channel();

        let state = A::startup(config);

        let runtime = ActorRuntime {
            state,
            extras,
            sink: ClientSink::new(),
            bridge_rx,
            self_rx,
            local_rx,
            server_tx,
            self_tx: self_tx.clone(),
            local_tx: local_tx.clone(),
            connected: false,
        };

        let connector = BridgeConnector { bridge_tx, server_rx };
        (runtime, connector)
    }

    /// Non-blocking: process all pending messages.
    pub fn poll(&mut self) -> RuntimeStatus {
        while let Ok(event) = self.bridge_rx.try_recv() {
            if self.handle_bridge_event(event) == RuntimeStatus::Disconnected {
                return RuntimeStatus::Disconnected;
            }
        }
        while let Ok(msg) = self.self_rx.try_recv() {
            self.handle_message(ClientSource::Internal, msg);
        }
        while let Ok(msg) = self.local_rx.try_recv() {
            self.handle_message(ClientSource::Local, msg);
        }
        if self.connected { RuntimeStatus::Connected } else { RuntimeStatus::Disconnected }
    }

    /// Blocking: wait for at least one message, then drain all pending.
    pub async fn step(&mut self) -> RuntimeStatus {
        tokio::select! {
            event = self.bridge_rx.recv() => {
                match event {
                    Some(e) => {
                        if self.handle_bridge_event(e) == RuntimeStatus::Disconnected {
                            return RuntimeStatus::Disconnected;
                        }
                    }
                    None => return RuntimeStatus::Disconnected,
                }
            }
            msg = self.self_rx.recv() => {
                if let Some(m) = msg { self.handle_message(ClientSource::Internal, m); }
            }
            msg = self.local_rx.recv() => {
                if let Some(m) = msg { self.handle_message(ClientSource::Local, m); }
            }
        }
        self.poll()
    }

    /// Blocking: run until disconnected.
    pub async fn run(&mut self) {
        while self.step().await != RuntimeStatus::Disconnected {}
    }

    pub fn state(&self) -> &A::State { &self.state }
    pub fn state_mut(&mut self) -> &mut A::State { &mut self.state }
    pub fn is_connected(&self) -> bool { self.connected }

    pub fn local_sender(&self) -> LocalSender<M> {
        LocalSender::new_from_sender(self.local_tx.clone())
    }

    pub fn send_local(&self, msg: M) -> Result<(), ActorSendError> {
        self.local_tx.send(msg).map_err(|_| ActorSendError::ChannelClosed)
    }

    pub fn server_sender(&self) -> super::ClientActorSender<M> {
        super::ClientActorSender::new_from_sender(self.server_tx.clone())
    }

    pub fn send_to_server(&self, msg: M) -> Result<(), ActorSendError> {
        self.server_tx.send(msg).map_err(|_| ActorSendError::ChannelClosed)
    }

    fn handle_bridge_event(&mut self, event: BridgeEvent<M>) -> RuntimeStatus {
        match event {
            BridgeEvent::Connected => {
                self.connected = true;
                self.dispatch(ClientSource::Server, ClientMessage::Connected);
                RuntimeStatus::Connected
            }
            BridgeEvent::Message(m) => {
                self.dispatch(ClientSource::Server, ClientMessage::Message(m));
                RuntimeStatus::Connected
            }
            BridgeEvent::Disconnected => {
                self.connected = false;
                self.dispatch(ClientSource::Server, ClientMessage::Disconnected);
                A::shutdown(&mut self.state, &mut self.extras);
                RuntimeStatus::Disconnected
            }
        }
    }

    fn handle_message(&mut self, source: ClientSource, message: M) {
        self.dispatch(source, ClientMessage::Message(message));
    }

    fn dispatch(&mut self, from: ClientSource, content: ClientMessage<M>) {
        A::handle(&mut self.state, &mut self.extras, AddressedMessage { from, content }, &mut self.sink);
        for output in self.sink.drain() {
            match output {
                ClientOutput::ToServer { message } => { let _ = self.server_tx.send(message); }
                ClientOutput::Internal { message } => { let _ = self.self_tx.send(message); }
            }
        }
    }
}
