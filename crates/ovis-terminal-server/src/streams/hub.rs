//! One terminal's streams: screen ingestion, query filtering, the shared log and viewers.
//!
//! Every holder output chunk is ingested by the screen, its answered queries are
//! filtered out, and the filtered bytes are appended once to the shared log. Viewers
//! read the log at their own cursors; publishing never waits for any viewer. A viewer
//! whose backlog passes its bound, or the largest ones while the session budget is
//! exceeded, is resynchronized with a gap record and a fresh checkpoint.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use ovis_terminal_core::leases::TerminalLeases;
use ovis_terminal_core::screen::{QueryReply, ReplyBatch, Screen};
use ovis_terminal_holder::protocol::{Exit, Output};
use ovis_terminal_protocol::schema::events::StreamCloseReason;
use ovis_terminal_protocol::schema::ids::{StreamId, TerminalId};
use ovis_terminal_protocol::schema::records::{AttachedTerminal, StreamClose};
use ovis_terminal_protocol::schema::scalar::{ByteOffset, Incarnation};
use ovis_terminal_store::OpId;
use ovis_terminal_vt::{FirstPaint, OutputOffset};

use super::error::StreamError;
use super::filter::{QueryFilter, RawSlice};
use super::journal::LeaseJournal;
use super::limits::{SessionBacklog, StreamLimits};
use super::log::OutputLog;
use super::resync::revoke_input;
use super::viewer::Viewer;

/// Shared services a terminal's streams use.
#[derive(Debug, Clone)]
pub struct StreamSetup {
    pub(super) limits: StreamLimits,
    pub(super) backlog: Arc<SessionBacklog>,
    pub(super) journal: Arc<dyn LeaseJournal>,
}

impl StreamSetup {
    /// Streams bounded by `limits` and the session `backlog`, committing leases to `journal`.
    #[must_use]
    pub fn new(
        limits: StreamLimits,
        backlog: Arc<SessionBacklog>,
        journal: Arc<dyn LeaseJournal>,
    ) -> Self {
        Self {
            limits,
            backlog,
            journal,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) enum Phase {
    Live,
    Recovering,
    Ended(StreamClose),
}

#[derive(Debug)]
pub(super) struct HubState {
    pub filter: QueryFilter,
    pub log: OutputLog,
    pub viewers: BTreeMap<StreamId, Viewer>,
    pub phase: Phase,
    pub leases: TerminalLeases,
}

/// A fenced paint shared by every viewer resynchronized at the same boundary.
pub(super) struct Painted {
    pub paint: FirstPaint,
    pub bytes: Arc<[u8]>,
}

/// The attach streams of one terminal incarnation.
#[derive(Debug)]
pub struct TerminalStreams {
    pub(super) screen: Screen,
    pub(super) identity: AttachedTerminal,
    pub(super) setup: StreamSetup,
    pub(super) state: Mutex<HubState>,
}

impl TerminalStreams {
    /// Serves streams of `screen`'s terminal under `leases`.
    #[must_use]
    pub fn new(screen: Screen, leases: TerminalLeases, setup: StreamSetup) -> Arc<Self> {
        let target = screen.target();
        let identity = AttachedTerminal::new(
            TerminalId::new(target.terminal_id.as_str().to_owned()),
            Incarnation::new(target.terminal_incarnation.get()),
            target
                .process_incarnation
                .map(|process| Incarnation::new(process.get())),
        );
        Arc::new(Self {
            screen,
            identity,
            setup,
            state: Mutex::new(HubState {
                filter: QueryFilter::default(),
                log: OutputLog::default(),
                viewers: BTreeMap::new(),
                phase: Phase::Live,
                leases,
            }),
        })
    }

    /// The terminal and program incarnations every stream of this hub is bound to.
    #[must_use]
    pub const fn identity(&self) -> &AttachedTerminal {
        &self.identity
    }

    pub(super) fn lock(&self) -> Result<MutexGuard<'_, HubState>, StreamError> {
        self.state.lock().map_err(|_| StreamError::Poisoned)
    }

    /// Ingests one holder chunk and appends its filtered bytes once for every viewer.
    ///
    /// Returns the terminal-query replies the core must write on the holder's
    /// internal lane; each is returned exactly once.
    ///
    /// # Errors
    ///
    /// Returns the screen's target, gap or ended refusal; nothing is appended then.
    pub fn publish(&self, output: &Output) -> Result<ReplyBatch, StreamError> {
        let mut state = self.lock()?;
        self.screen.ingest(output)?;
        let replies = self.screen.drain_replies()?;
        let answered: Vec<OutputOffset> = replies
            .replies
            .iter()
            .filter_map(|reply| match reply {
                QueryReply::Reply { offset, .. } => Some(*offset),
                _ => None,
            })
            .collect();
        let filtered = state.filter.apply(RawSlice {
            at: OutputOffset::new(output.offset),
            bytes: &output.bytes,
            answered: &answered,
        });
        state.log.append(filtered);
        self.rebalance(&mut state)?;
        drop(state);
        Ok(replies)
    }

    /// Records the exit: every viewer receives its final bytes, then CLOSE `exited`.
    ///
    /// # Errors
    ///
    /// Returns the screen's refusal when output before the exit is missing.
    pub fn finish(&self, exit: &Exit) -> Result<(), StreamError> {
        let mut state = self.lock()?;
        self.screen.finish(exit)?;
        let flushed = state.filter.flush();
        state.log.append(flushed);
        let reason = if exit.signal.is_some() {
            StreamCloseReason::Killed
        } else {
            StreamCloseReason::Exited
        };
        let mut close = StreamClose::new(reason, ByteOffset::new(0));
        close.final_output_offset = Some(ByteOffset::new(exit.final_offset));
        close.exit_code = exit.code;
        close.signal = exit.signal;
        state.phase = Phase::Ended(close.clone());
        let end = state.log.end();
        for viewer in state.viewers.values_mut() {
            revoke_input(viewer);
            viewer.close_after(end, close.clone());
            viewer.wake.notify_one();
        }
        drop(state);
        Ok(())
    }

    /// Marks the terminal recovering: input readiness is revoked and every stream
    /// closes `lost` ahead of undelivered bytes; leases of its viewers are revoked.
    ///
    /// Blocks on the durable commit of the revocations; call it from a blocking context.
    ///
    /// # Errors
    ///
    /// Returns a lease or journal failure; readiness is revoked before either.
    pub fn recovering(&self, op: &OpId) -> Result<(), StreamError> {
        self.suspend(op, StreamCloseReason::Lost)
    }

    /// A planned control restart begins: input readiness is revoked and every stream
    /// closes `server_restart`, never `lost` or a shutdown, ahead of undelivered bytes.
    /// Attaches wait for `server.recovery`; the successor serves new stream identities
    /// for the same program incarnation.
    ///
    /// Blocks on the durable commit of the revocations; call it from a blocking context.
    ///
    /// # Errors
    ///
    /// Returns a lease or journal failure; readiness is revoked before either.
    pub fn restarting(&self, op: &OpId) -> Result<(), StreamError> {
        self.suspend(op, StreamCloseReason::ServerRestart)
    }

    fn suspend(&self, op: &OpId, reason: StreamCloseReason) -> Result<(), StreamError> {
        let mut state = self.lock()?;
        state.phase = Phase::Recovering;
        let mut owners = Vec::new();
        for viewer in state.viewers.values_mut() {
            revoke_input(viewer);
            let offset = viewer.next_offset().wire();
            viewer.close_now(StreamClose::new(reason, offset));
            viewer.wake.notify_one();
            owners.push(viewer.owner.clone());
        }
        let mut transitions = Vec::new();
        for owner in &owners {
            transitions.push(state.leases.disconnect(owner)?);
        }
        drop(state);
        for transition in transitions {
            self.setup.journal.record(op.clone(), transition)?;
        }
        Ok(())
    }

    /// Ends recovery; new attaches are served again with fresh stream identities.
    ///
    /// # Errors
    ///
    /// Returns [`StreamError::Poisoned`] when the state lock is poisoned.
    pub fn recovered(&self) -> Result<(), StreamError> {
        self.lock()?.phase = Phase::Live;
        Ok(())
    }

    /// Streams currently attached.
    ///
    /// # Errors
    ///
    /// Returns [`StreamError::Poisoned`] when the state lock is poisoned.
    pub fn streams(&self) -> Result<usize, StreamError> {
        Ok(self.lock()?.viewers.len())
    }

    /// Releases held query bytes and paints the screen at exactly the log's raw end.
    pub(super) fn fence(&self, state: &mut HubState) -> Result<Painted, StreamError> {
        let flushed = state.filter.flush();
        state.log.append(flushed);
        let paint = self.screen.first_paint()?;
        let bytes = Arc::from(paint.bytes.as_slice());
        Ok(Painted { paint, bytes })
    }

    fn rebalance(&self, state: &mut HubState) -> Result<(), StreamError> {
        let limits = self.setup.limits;
        let mut over_stream: Vec<StreamId> = state
            .viewers
            .values()
            .filter(|viewer| {
                !viewer.is_closed() && viewer.backlog(&state.log) > limits.backlog_bytes
            })
            .map(|viewer| viewer.id.clone())
            .collect();
        let mut by_size: Vec<(u64, StreamId)> = Vec::new();
        let mut over_session = 0;
        for viewer in state.viewers.values() {
            let backlog = viewer.backlog(&state.log);
            over_session = self.setup.backlog.set(&viewer.id, backlog)?;
            by_size.push((backlog, viewer.id.clone()));
        }
        by_size.sort_by_key(|(backlog, _)| std::cmp::Reverse(*backlog));
        for (backlog, id) in by_size {
            if over_session == 0 {
                break;
            }
            over_session = over_session.saturating_sub(backlog);
            over_stream.push(id);
        }
        if !over_stream.is_empty() {
            let painted = self.fence(state)?;
            for id in &over_stream {
                if let Some(viewer) = state.viewers.get_mut(id) {
                    super::resync::resync(&state.log, viewer, &painted);
                    self.setup.backlog.set(id, viewer.backlog(&state.log))?;
                }
            }
        }
        let keep = state
            .viewers
            .values()
            .map(Viewer::cursor)
            .min()
            .unwrap_or_else(|| state.log.end());
        state.log.trim(keep);
        for viewer in state.viewers.values() {
            viewer.wake.notify_one();
        }
        Ok(())
    }
}
