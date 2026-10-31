//! First messages keyed by launch operation, each owned by its requesting principal.

use std::collections::HashMap;
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Instant;

use ovis_terminal_authz::PrincipalId;
use ovis_terminal_protocol::domain::template_launch::{
    FirstMessageReason, FirstMessageRecord, FirstMessageState,
};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::scalar::{EventSeq, OpenEnum};

use super::LaunchError;

/// Messages retained at once; settled ones make room for new launches first.
const RETAINED: usize = 1024;

/// A first message to queue: its accepted record, its text and who asked for it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct QueuedMessage {
    /// Principal that launched the terminal; only it may read the message.
    pub principal: PrincipalId,
    /// Literal text submitted once on readiness.
    pub text: String,
    /// The accepted record, keyed by its launch operation.
    pub record: FirstMessageRecord,
}

impl QueuedMessage {
    /// Queues `text` for `principal` under `record`.
    #[must_use]
    pub const fn new(principal: PrincipalId, text: String, record: FirstMessageRecord) -> Self {
        Self {
            principal,
            text,
            record,
        }
    }
}

/// How a message that was never written ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Settlement {
    /// The deadline passed before readiness.
    Expired,
    /// The program ended or the runtime stopped waiting.
    Cancelled(FirstMessageReason),
}

#[derive(Debug)]
struct Entry {
    principal: PrincipalId,
    text: String,
    record: FirstMessageRecord,
}

/// Every retained first message; each leaves `accepted` at most once.
#[derive(Debug, Default)]
pub struct FirstMessages {
    entries: Mutex<HashMap<OpId, Entry>>,
    settled: Condvar,
}

impl FirstMessages {
    /// An empty book.
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<MutexGuard<'_, HashMap<OpId, Entry>>, LaunchError> {
        self.entries.lock().map_err(|_| LaunchError::Poisoned)
    }

    /// Queues `message`; queuing the same launch again answers the retained record.
    ///
    /// # Errors
    ///
    /// `forbidden` when another principal owns the launch, `limit_exceeded` when every
    /// retained message is still pending.
    pub fn accept(&self, message: QueuedMessage) -> Result<FirstMessageRecord, LaunchError> {
        queue(&mut *self.lock()?, message)
    }

    /// The message queued under `launch`, as `principal` may read it.
    ///
    /// # Errors
    ///
    /// `not_found` for an unknown launch, `forbidden` for another principal's message.
    pub fn get(
        &self,
        principal: &PrincipalId,
        launch: &OpId,
    ) -> Result<FirstMessageRecord, LaunchError> {
        let entries = self.lock()?;
        owned(&entries, principal, launch)
    }

    /// Waits until the message settles or `until` passes, then answers its state.
    ///
    /// # Errors
    ///
    /// As [`Self::get`].
    pub fn wait_settled(
        &self,
        (principal, launch): (&PrincipalId, &OpId),
        until: Instant,
    ) -> Result<FirstMessageRecord, LaunchError> {
        let mut entries = self.lock()?;
        loop {
            let record = owned(&entries, principal, launch)?;
            let left = until.saturating_duration_since(Instant::now());
            if !record.is_pending() || left.is_zero() {
                return Ok(record);
            }
            entries = self
                .settled
                .wait_timeout(entries, left)
                .map_err(|_| LaunchError::Poisoned)?
                .0;
        }
    }

    /// Writes a pending message once through `write`, which runs the write-boundary
    /// rechecks and returns the submission's input sequence or why it refused.
    ///
    /// A settled message is answered unchanged and `write` is not called.
    ///
    /// # Errors
    ///
    /// `not_found` for an unknown launch.
    pub fn release(
        &self,
        launch: &OpId,
        write: impl FnOnce(&str) -> Result<EventSeq, FirstMessageReason>,
    ) -> Result<FirstMessageRecord, LaunchError> {
        let released = pending(&mut *self.lock()?, launch).map(|entry| {
            if entry.record.is_pending() {
                match write(&entry.text) {
                    Ok(seq) => {
                        entry.record.state = OpenEnum::Known(FirstMessageState::Sent);
                        entry.record.queued_seq = Some(seq);
                    }
                    Err(reason) => {
                        entry.record.state = OpenEnum::Known(FirstMessageState::Cancelled);
                        entry.record.reason = Some(OpenEnum::Known(reason));
                    }
                }
            }
            entry.record.clone()
        });
        self.settled.notify_all();
        released
    }

    /// Ends a pending message without writing it; a settled message is unchanged.
    ///
    /// # Errors
    ///
    /// `not_found` for an unknown launch.
    pub fn settle(
        &self,
        launch: &OpId,
        end: Settlement,
    ) -> Result<FirstMessageRecord, LaunchError> {
        let (state, reason) = match end {
            Settlement::Expired => (
                FirstMessageState::Expired,
                FirstMessageReason::DeadlinePassed,
            ),
            Settlement::Cancelled(reason) => (FirstMessageState::Cancelled, reason),
        };
        let settled = pending(&mut *self.lock()?, launch).map(|entry| {
            if entry.record.is_pending() {
                entry.record.state = OpenEnum::Known(state);
                entry.record.reason = Some(OpenEnum::Known(reason));
            }
            entry.record.clone()
        });
        self.settled.notify_all();
        settled
    }
}

fn queue(
    entries: &mut HashMap<OpId, Entry>,
    message: QueuedMessage,
) -> Result<FirstMessageRecord, LaunchError> {
    if let Some(entry) = entries.get(&message.record.launch_op) {
        if entry.principal != message.principal {
            return Err(LaunchError::Forbidden);
        }
        return Ok(entry.record.clone());
    }
    if entries.len() >= RETAINED {
        let settled = entries
            .iter()
            .find(|(_, entry)| !entry.record.is_pending())
            .map(|(launch, _)| launch.clone())
            .ok_or(LaunchError::Capacity)?;
        entries.remove(&settled);
    }
    let record = message.record.clone();
    entries.insert(
        record.launch_op.clone(),
        Entry {
            principal: message.principal,
            text: message.text,
            record: message.record,
        },
    );
    Ok(record)
}

fn pending<'a>(
    entries: &'a mut HashMap<OpId, Entry>,
    launch: &OpId,
) -> Result<&'a mut Entry, LaunchError> {
    entries.get_mut(launch).ok_or(LaunchError::NotFound)
}

fn owned(
    entries: &HashMap<OpId, Entry>,
    principal: &PrincipalId,
    launch: &OpId,
) -> Result<FirstMessageRecord, LaunchError> {
    let entry = entries.get(launch).ok_or(LaunchError::NotFound)?;
    if &entry.principal != principal {
        return Err(LaunchError::Forbidden);
    }
    Ok(entry.record.clone())
}
