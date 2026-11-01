//! Holder messages are the only source of spawn, hangup and exit facts.

mod adoption;
mod rejection;
mod settlement;

use std::time::{Duration, Instant};

use ovis_terminal_holder::protocol::{Message, Tail, TailRequest};
use ovis_terminal_protocol::schema::scalar::ByteOffset;
use ovis_terminal_store::{CasWrite, OpId, Payload, Version};

use super::{Incoming, Intent, Terminals};
use crate::events::{Commit, ResourceId};
use crate::registry::{LifecycleEvent as Transition, TerminalId, TerminalState};
use crate::terminals::snapshot::{read_record, record_key};
use crate::terminals::{LaunchState, LifecycleEvent, Record, TerminalError};

impl Terminals {
    /// Processes one actual holder notification with a bounded wait.
    ///
    /// # Errors
    ///
    /// Returns typed delivery, identity, continuity, authority or persistence failures.
    pub fn receive(&mut self, timeout: Duration) -> Result<(), TerminalError> {
        match self
            .messages
            .recv_timeout(timeout)
            .map_err(TerminalError::Notification)?
        {
            Incoming::Holder(message) => self.observe(message),
            Incoming::Rejected(id, error) => self.rejected(id, error),
            Incoming::Disconnected(error) => Err(error.into()),
        }
    }

    #[cfg(all(test, unix))]
    /// Applies one holder notification exactly like [`Terminals::receive`] and hands it
    /// back once committed, so screens and input sequencers follow the same ordered facts;
    /// `None` when the notification was an internal spawn refusal.
    ///
    /// # Errors
    ///
    /// The failures of [`Terminals::receive`]; a message that failed to apply is not returned.
    pub(crate) fn receive_message(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<Message>, TerminalError> {
        let incoming = self
            .messages
            .recv_timeout(timeout)
            .map_err(TerminalError::Notification)?;
        self.apply_message(incoming)
    }

    /// Applies one holder notification that already arrived, exactly like
    /// [`Terminals::receive`], and never waits: a caller holding a lock around it
    /// cannot keep that lock while the holder is silent. An empty queue is the same
    /// `Notification(Timeout)` a zero-length wait reports.
    ///
    /// # Errors
    ///
    /// The failures of [`Terminals::receive`].
    pub fn try_receive_message(&mut self) -> Result<Option<Message>, TerminalError> {
        let incoming = self.messages.try_recv().map_err(|error| {
            TerminalError::Notification(match error {
                std::sync::mpsc::TryRecvError::Empty => std::sync::mpsc::RecvTimeoutError::Timeout,
                std::sync::mpsc::TryRecvError::Disconnected => {
                    std::sync::mpsc::RecvTimeoutError::Disconnected
                }
            })
        })?;
        self.apply_message(incoming)
    }

    fn apply_message(&mut self, incoming: Incoming) -> Result<Option<Message>, TerminalError> {
        match incoming {
            Incoming::Holder(message) => {
                let applied = message.clone();
                self.observe(message)?;
                Ok(Some(applied))
            }
            Incoming::Rejected(id, error) => self.rejected(id, error).map(|()| None),
            Incoming::Disconnected(error) => Err(error.into()),
        }
    }

    /// Awaits spawn evidence, consuming the same ordered holder stream used by the actor.
    ///
    /// # Errors
    ///
    /// Returns a bounded wait or evidence failure; a pending launch is never called spawned.
    pub fn await_spawn(
        &mut self,
        id: TerminalId,
        timeout: Duration,
    ) -> Result<Record, TerminalError> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or(TerminalError::InvalidParams)?;
        loop {
            let record = self.record(id)?;
            match record.spawn {
                LaunchState::Spawned | LaunchState::Failed => return Ok(record),
                LaunchState::Admitted | LaunchState::Pending => {
                    self.receive(deadline.saturating_duration_since(Instant::now()))?;
                }
            }
        }
    }

    /// Reads the holder's retained raw bytes without input or signal side effects.
    ///
    /// # Errors
    ///
    /// Returns authority, missing-process or holder-tail errors.
    pub fn output(
        &self,
        session: &ovis_terminal_authz::Session,
        id: TerminalId,
    ) -> Result<Tail, TerminalError> {
        let record = self.get(session, id)?;
        let target = record.holder_target.ok_or(TerminalError::NotFound)?;
        Ok(self.host.holder.tail(&TailRequest {
            target,
            from_offset: 0,
        })?)
    }

    fn observe(&mut self, message: Message) -> Result<(), TerminalError> {
        match message {
            Message::Welcome(welcome) => self.adopted(welcome),
            Message::Spawned(spawned) => self.spawned(spawned),
            Message::SpawnFailed(failed) => self.spawn_failed(failed),
            Message::Output(output) => {
                let id = output.target.terminal_id.as_str().parse()?;
                let mut record = self.record(id)?;
                if record.holder_target.as_ref() != Some(&output.target) {
                    return Err(TerminalError::ProcessReplaced);
                }
                if record.exit.is_some() || output.offset != record.output_offset.get() {
                    return Err(TerminalError::OutputGap);
                }
                let length =
                    u64::try_from(output.bytes.len()).map_err(|_| TerminalError::OutputGap)?;
                record.output_offset = ByteOffset::new(
                    output
                        .offset
                        .checked_add(length)
                        .ok_or(TerminalError::OutputGap)?,
                );
                let event = match output.hangup {
                    Some(hangup) => {
                        let process = record
                            .lifecycle
                            .process()
                            .ok_or(TerminalError::ProcessReplaced)?;
                        if u64::from(hangup.pid.get()) != process.pid().get()
                            || hangup.start_token.as_str() != process.start_token().as_str()
                        {
                            return Err(TerminalError::ProcessReplaced);
                        }
                        record.hangup = Some(hangup);
                        record.agent_owner = None;
                        LifecycleEvent::Hangup(record.clone())
                    }
                    None => LifecycleEvent::Updated(record.clone()),
                };
                self.persist(&record, event)
            }
            Message::Exit(exit) => {
                let mut record = self.record(exit.target.terminal_id.as_str().parse()?)?;
                if record.holder_target.as_ref() != Some(&exit.target) {
                    return Err(TerminalError::ProcessReplaced);
                }
                if let Some(prior) = &record.exit {
                    return if prior == &exit {
                        Ok(())
                    } else {
                        Err(TerminalError::ProcessReplaced)
                    };
                }
                if record.output_offset.get() != exit.final_offset {
                    return Err(TerminalError::OutputGap);
                }
                let transition = if exit.exit_unknown.is_none()
                    && (exit.code.is_some() || exit.signal.is_some())
                {
                    Transition::ProcessEnded
                } else {
                    Transition::HolderLost
                };
                record.lifecycle = record.lifecycle.transition(transition)?;
                record.exit = Some(exit);
                record.agent_owner = None;
                if let Some(pending) = self.pending.remove(&record.id()) {
                    return match pending.intent {
                        Intent::Kill(_) => self.complete_kill(pending, &record),
                        Intent::Launch => Err(TerminalError::ProcessReplaced),
                    };
                }
                self.persist(&record, LifecycleEvent::Exited(record.clone()))
            }
            Message::InputWritten(_) | Message::InputRejected(_) => Ok(()),
            _ => Err(TerminalError::InvalidParams),
        }
    }

    pub(super) fn persist(
        &self,
        record: &Record,
        event: LifecycleEvent,
    ) -> Result<(), TerminalError> {
        self.core.commit_unit(|snapshot| {
            let (_, version) = read_record(snapshot, record.id())?;
            let mut persisted = record.clone();
            persisted.revision = ovis_terminal_protocol::schema::scalar::Revision::new(
                snapshot
                    .revision()
                    .get()
                    .checked_add(1)
                    .ok_or(crate::events::EventError::LimitExceeded)?,
            );
            Ok::<_, crate::events::EventError>(
                Commit::new(
                    OpId::new(uuid::Uuid::now_v7().to_string()),
                    event,
                    vec![ResourceId::Terminal(record.id())],
                )
                .write(CasWrite::new(
                    record_key(record.id()),
                    Version::new(version.get()),
                    Some(Payload::from(serde_json::to_vec(&persisted)?)),
                )),
            )
        })?;
        Ok(())
    }

    pub(super) fn finished(record: &Record) -> bool {
        record.lifecycle.state() == TerminalState::Finished
    }
}
