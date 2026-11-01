//! Holder links of one control boot: adoption, spawning and catch-up before the loop,
//! then one thread per link that forwards holder messages and sends queued requests.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, SyncSender, TryRecvError};
use std::thread::JoinHandle;
use std::time::Duration;

use ovis_terminal_holder::protocol::{
    CoreBuildId, Goodbye, GoodbyeReason, HOLDER_PROTOCOL_GENERATION, Hello, Input, Message,
    PreviousCoreState, Spawn, Spawned, Tail, TailRequest, TerminalTarget, Welcome,
};
use ovis_terminal_holder::unix::{CoreConnection, CoreConnectionError};

use super::super::error::SoakError;
use super::Inbound;

/// Longest wait for one holder answer before the loop starts.
const ANSWER: Duration = Duration::from_secs(15);
/// How long a link thread waits for holder bytes before it looks at its request queue.
const TURN: Duration = Duration::from_millis(100);

/// What the loop asks a link thread to send.
#[derive(Debug)]
pub(super) enum Outbound {
    Input(Box<Input>),
    Goodbye(GoodbyeReason),
}

/// Whether an awaited answer arrived or another message must be kept for later.
enum Pick<T> {
    Take(T),
    Keep(Box<Message>),
}

/// One adopted holder group before its thread starts.
#[derive(Debug)]
pub(super) struct Link {
    connection: CoreConnection,
    /// Messages that arrived while a specific answer was awaited, kept in order.
    pending: VecDeque<Message>,
}

impl Link {
    /// Connects to `socket` and adopts the holder as the successor of `previous`.
    pub(super) fn adopt(
        socket: &Path,
        previous: PreviousCoreState,
    ) -> Result<(Self, Welcome), SoakError> {
        let mut connection = CoreConnection::connect(socket)?;
        connection.send(&Message::Hello(Hello {
            generation: HOLDER_PROTOCOL_GENERATION,
            core_build: CoreBuildId::new(format!("soak/{}", env!("CARGO_PKG_VERSION"))),
            previous,
        }))?;
        let Message::Welcome(welcome) = connection.recv(ANSWER)? else {
            return Err(SoakError::Protocol {
                expected: "a welcome",
            });
        };
        let link = Self {
            connection,
            pending: VecDeque::new(),
        };
        Ok((link, welcome))
    }

    fn answer<T>(&mut self, mut pick: impl FnMut(Message) -> Pick<T>) -> Result<T, SoakError> {
        loop {
            match pick(self.connection.recv(ANSWER)?) {
                Pick::Take(answer) => return Ok(answer),
                Pick::Keep(other) => self.pending.push_back(*other),
            }
        }
    }

    pub(super) fn spawn(&mut self, spawn: Spawn) -> Result<Spawned, SoakError> {
        let id = spawn.terminal_id.clone();
        self.connection.send(&Message::Spawn(spawn))?;
        let answer = self.answer(|message| match message {
            Message::Spawned(spawned) if spawned.terminal_id == id => Pick::Take(Ok(spawned)),
            Message::SpawnFailed(failed) if failed.terminal_id == id => Pick::Take(Err(())),
            other => Pick::Keep(Box::new(other)),
        })?;
        answer.map_err(|()| SoakError::Protocol {
            expected: "a spawned program",
        })
    }

    /// Retained output of `target` from `from`.
    pub(super) fn tail(&mut self, target: &TerminalTarget, from: u64) -> Result<Tail, SoakError> {
        self.connection.send(&Message::TailRequest(TailRequest {
            target: target.clone(),
            from_offset: from,
        }))?;
        let wanted = target.terminal_id.clone();
        self.answer(|message| match message {
            Message::Tail(tail) if tail.target.terminal_id == wanted => Pick::Take(tail),
            other => Pick::Keep(Box::new(other)),
        })
    }

    /// Starts the thread that forwards every message as group `group`.
    pub(super) fn start(
        self,
        group: usize,
        inbound: SyncSender<Inbound>,
    ) -> Result<(Sender<Outbound>, JoinHandle<()>), SoakError> {
        let (outbound, requests) = std::sync::mpsc::channel();
        let thread = std::thread::Builder::new()
            .name(format!("soak-link-{group}"))
            .spawn(move || self.forward(group, (&inbound, &requests)))
            .map_err(SoakError::io("start a holder link thread"))?;
        Ok((outbound, thread))
    }

    fn forward(
        mut self,
        group: usize,
        (inbound, requests): (&SyncSender<Inbound>, &Receiver<Outbound>),
    ) {
        let lost = || inbound.send(Inbound::LinkLost { group }).is_ok();
        while let Some(message) = self.pending.pop_front() {
            if inbound.send(Inbound::Holder { message }).is_err() {
                return;
            }
        }
        loop {
            loop {
                match requests.try_recv() {
                    Ok(Outbound::Input(input)) => {
                        if self.connection.send(&Message::Input(*input)).is_err() {
                            lost();
                            return;
                        }
                    }
                    Ok(Outbound::Goodbye(reason)) => {
                        if self
                            .connection
                            .send(&Message::Goodbye(Goodbye { reason }))
                            .is_err()
                        {
                            lost();
                        }
                        return;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            match self.connection.recv(TURN) {
                Ok(message) => {
                    if inbound.send(Inbound::Holder { message }).is_err() {
                        return;
                    }
                }
                Err(CoreConnectionError::Timeout) => {}
                Err(_) => {
                    lost();
                    return;
                }
            }
        }
    }
}
