//! Per-stream operations of a terminal hub: delivery, credit, close, repaint, detach.

use std::time::Duration;

use ovis_terminal_protocol::schema::events::StreamCloseReason;
use ovis_terminal_protocol::schema::ids::StreamId;
use ovis_terminal_protocol::schema::records::{Checkpoint, StreamClose};
use ovis_terminal_store::OpId;

use super::error::StreamError;
use super::hub::{HubState, TerminalStreams};
use super::offset::StreamOffset;
use super::resync::{resync, revoke_input};
use super::viewer::{Delivery, Viewer};

fn viewer<'a>(state: &'a mut HubState, stream: &StreamId) -> Result<&'a mut Viewer, StreamError> {
    state
        .viewers
        .get_mut(stream)
        .ok_or(StreamError::StreamNotFound)
}

impl TerminalStreams {
    /// The next record or chunk for `stream`, or `None` until more work arrives.
    pub(super) fn next(&self, stream: &StreamId) -> Result<Option<Delivery>, StreamError> {
        let chunk = self.setup.limits.chunk_bytes;
        let mut guard = self.lock()?;
        let state = &mut *guard;
        let delivery = state
            .viewers
            .get_mut(stream)
            .ok_or(StreamError::StreamNotFound)?
            .next(&state.log, chunk);
        drop(guard);
        Ok(delivery)
    }

    /// Applies a cumulative acknowledgement; a refused one changes nothing.
    pub(super) fn acknowledge(
        &self,
        stream: &StreamId,
        at: StreamOffset,
    ) -> Result<u64, StreamError> {
        let mut state = self.lock()?;
        let viewer = viewer(&mut state, stream)?;
        let available = viewer.acknowledge(at).map_err(StreamError::InvalidCredit)?;
        viewer.wake.notify_one();
        drop(state);
        Ok(available)
    }

    /// Closes `stream` at the client's request ahead of undelivered bytes.
    pub(super) fn close(&self, stream: &StreamId) -> Result<StreamOffset, StreamError> {
        let mut state = self.lock()?;
        let viewer = viewer(&mut state, stream)?;
        revoke_input(viewer);
        let at = viewer.next_offset();
        viewer.close_now(StreamClose::new(StreamCloseReason::ClientClosed, at.wire()));
        viewer.wake.notify_one();
        drop(state);
        Ok(at)
    }

    /// Sends a fresh checkpoint on `stream`, replacing any undelivered backlog.
    pub(super) fn repaint(&self, stream: &StreamId) -> Result<Checkpoint, StreamError> {
        let mut guard = self.lock()?;
        if !guard.viewers.contains_key(stream) {
            return Err(StreamError::StreamNotFound);
        }
        let painted = self.fence(&mut guard)?;
        let state = &mut *guard;
        let viewer = state
            .viewers
            .get_mut(stream)
            .ok_or(StreamError::StreamNotFound)?;
        let checkpoint = resync(&state.log, viewer, &painted);
        viewer.wake.notify_one();
        drop(guard);
        Ok(checkpoint)
    }

    /// How long until the stream next needs a stall check, or `None` once its close
    /// was sent.
    ///
    /// A stream with undelivered work and no progress for the stall bound is closed
    /// `stream_stalled` now, and the check is due immediately so the close is sent.
    pub(super) fn stall(&self, stream: &StreamId) -> Result<Option<Duration>, StreamError> {
        let after = self.setup.limits.stall_after;
        let mut guard = self.lock()?;
        let state = &mut *guard;
        let viewer = state
            .viewers
            .get_mut(stream)
            .ok_or(StreamError::StreamNotFound)?;
        if viewer.finished() {
            return Ok(None);
        }
        let due = if viewer.stalled(&state.log, after) {
            stall_close(viewer);
            Duration::ZERO
        } else {
            viewer.stall_left(after)
        };
        drop(guard);
        Ok(Some(due))
    }

    /// Closes `stream` `stream_stalled` because its connection accepted nothing for the
    /// stall bound.
    pub(super) fn force_stall(&self, stream: &StreamId) -> Result<(), StreamError> {
        if let Some(viewer) = self.lock()?.viewers.get_mut(stream) {
            stall_close(viewer);
        }
        Ok(())
    }

    /// Revokes the leases of `stream`'s owner unless another stream of that owner remains;
    /// the stream itself stays so its CLOSE can still be delivered.
    pub(super) fn release(&self, stream: &StreamId, op: OpId) -> Result<(), StreamError> {
        let mut state = self.lock()?;
        let Some(owner) = state.viewers.get(stream).map(|viewer| viewer.owner.clone()) else {
            return Ok(());
        };
        let shared = state
            .viewers
            .iter()
            .any(|(id, other)| id != stream && other.owner == owner);
        let transition = if shared {
            None
        } else {
            Some(state.leases.disconnect(&owner)?)
        };
        drop(state);
        if let Some(transition) = transition {
            self.setup.journal.record(op, transition)?;
        }
        Ok(())
    }

    /// Revokes the owner's leases as [`TerminalStreams::release`] does, then removes
    /// `stream` and releases the log bytes only it still referenced.
    pub(super) fn detach(&self, stream: &StreamId, op: OpId) -> Result<(), StreamError> {
        self.release(stream, op)?;
        let mut state = self.lock()?;
        if state.viewers.remove(stream).is_none() {
            return Ok(());
        }
        self.setup.backlog.set(stream, 0)?;
        let keep = state
            .viewers
            .values()
            .map(Viewer::cursor)
            .min()
            .unwrap_or_else(|| state.log.end());
        let unwatched = state.viewers.is_empty();
        state.log.trim(keep, unwatched);
        drop(state);
        Ok(())
    }
}

fn stall_close(viewer: &mut Viewer) {
    revoke_input(viewer);
    let at = viewer.next_offset();
    viewer.force_close(StreamClose::new(
        StreamCloseReason::StreamStalled,
        at.wire(),
    ));
    viewer.wake.notify_one();
}
