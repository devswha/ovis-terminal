//! `attach.open` decided against the terminal's phase and lease table.
//!
//! Live: observe attaches; an uncontested interact attach is ready; a contested one is
//! `held` with an observe stream, the holder and its lease-released retry boundary; an
//! explicit takeover displaces the holder, whose stream closes `taken_over`; a caller
//! without the input grant is `refused`. Recovering: no stream, a recovery boundary.
//! Ended: an observe stream with the final screen, then CLOSE. Every stream gets a fresh
//! identity and an offset-fenced checkpoint before any live byte.

use std::sync::Arc;

use ovis_terminal_core::leases::{
    Acquire, Caller, ControlSlot, Granted, LeaseError, LeaseEvent, Owner, RevokeReason, Takeover,
};
use ovis_terminal_core::registry::LeaseId;
use ovis_terminal_protocol::domain::health::ConsumerRef;
use ovis_terminal_protocol::schema::events::StreamCloseReason;
use ovis_terminal_protocol::schema::ids::StreamId;
use ovis_terminal_protocol::schema::methods::attach::{
    AttachOpenResult, AttachOutcome, TakeoverRequest,
};
use ovis_terminal_protocol::schema::records::{
    AttachControl, Checkpoint, ControlKind, LeaseRole, RetryBoundary, StreamClose,
};
use ovis_terminal_protocol::schema::scalar::{ByteCount, OpenEnum};
use ovis_terminal_store::OpId;
use tokio::sync::Notify;

use super::error::StreamError;
use super::hub::{HubState, Phase, TerminalStreams};
use super::offset::StreamOffset;
use super::records::{checkpoint, checkpoint_control, lease_record};
use super::resync::revoke_input;
use super::viewer::Viewer;

/// One authenticated attach request with identities already minted for it.
#[derive(Debug, Clone, Copy)]
pub(super) struct OpenRequest<'a> {
    pub caller: Caller<'a>,
    pub owner: &'a Owner,
    pub role: LeaseRole,
    pub takeover: Option<&'a TakeoverRequest>,
    pub fresh: LeaseId,
    pub stream: &'a StreamId,
}

/// The outcome and, when a stream was attached, the signal that wakes its delivery.
#[derive(Debug)]
pub(super) struct Opened {
    pub result: AttachOpenResult,
    pub wake: Option<Arc<Notify>>,
}

impl TerminalStreams {
    pub(super) fn open(&self, request: OpenRequest<'_>, op: OpId) -> Result<Opened, StreamError> {
        let mut state = self.lock()?;
        let credit = ByteCount::new(self.setup.limits.credit_window());
        let mut result = AttachOpenResult::new(AttachOutcome::Ready, credit);
        result.terminal = Some(self.identity.clone());
        let ended = match &state.phase {
            Phase::Recovering => {
                result.outcome = OpenEnum::Known(AttachOutcome::Recovering);
                result.retry = Some(RetryBoundary::recovered());
                return Ok(Opened { result, wake: None });
            }
            Phase::Ended(close) => Some(close.clone()),
            Phase::Live => None,
        };
        if state.viewers.len() >= self.setup.limits.max_streams {
            return Err(StreamError::StreamLimit {
                limit: self.setup.limits.max_streams,
            });
        }
        let granted = match ended {
            Some(close) => {
                result.outcome = OpenEnum::Known(AttachOutcome::Ended);
                result.reason = Some(reason_token(&close));
                result.final_output_offset = close.final_output_offset;
                let wake = self.attach(&mut state, (&request, false))?;
                result.stream(request.stream, wake.1.clone());
                let end = state.log.end();
                if let Some(viewer) = state.viewers.get_mut(request.stream) {
                    viewer.close_after(end, close);
                }
                drop(state);
                return Ok(Opened {
                    result,
                    wake: Some(wake.0),
                });
            }
            None => match decide(&mut state, &request, &mut result)? {
                Some(granted) => granted,
                None => return Ok(Opened { result, wake: None }),
            },
        };
        result.lease = Some(lease_record(&granted.lease));
        displace(&mut state, &granted, &mut result);
        let ready = result.outcome == OpenEnum::Known(AttachOutcome::Ready);
        let interact = ready && request.role == LeaseRole::Interact;
        let (wake, first) = self.attach(&mut state, (&request, interact))?;
        result.stream(request.stream, first);
        drop(state);
        if let Err(error) = self.setup.journal.record(op, granted.transition) {
            let mut state = self.lock()?;
            state.viewers.remove(request.stream);
            let undone = state.leases.disconnect(request.owner);
            drop(state);
            undone?;
            return Err(error);
        }
        Ok(Opened {
            result,
            wake: Some(wake),
        })
    }

    /// Adds the viewer with its fenced first paint, then input readiness when ready.
    fn attach(
        &self,
        state: &mut HubState,
        (request, interact): (&OpenRequest<'_>, bool),
    ) -> Result<(Arc<Notify>, Checkpoint), StreamError> {
        let window = self.setup.limits.credit_window();
        let mut viewer = Viewer::new(request.stream.clone(), request.owner.clone(), window);
        let consumer = ConsumerRef::Stream {
            stream_id: request.stream.clone(),
            terminal_id: self.identity.terminal_id.clone(),
        };
        let health = self.setup.health.as_deref();
        super::resync::track(&mut viewer, health, (request.owner, consumer));
        let painted = self.fence(state)?;
        let first = checkpoint(&painted.paint, StreamOffset::ZERO);
        let control = checkpoint_control(request.stream, first.clone());
        viewer.paint(control, (Arc::clone(&painted.bytes), state.log.end()));
        if interact {
            viewer.interact = true;
            let at = viewer.next_offset().wire();
            viewer.control(AttachControl::new(
                request.stream.clone(),
                ControlKind::InputReady,
                at,
            ));
        }
        viewer.report_queued(viewer.backlog(&state.log));
        self.setup
            .backlog
            .set(request.stream, viewer.backlog(&state.log))?;
        let wake = Arc::clone(&viewer.wake);
        state.viewers.insert(request.stream.clone(), viewer);
        Ok((wake, first))
    }
}

/// Grants the requested lease, or records `held`/`refused` and grants an observe lease
/// (held) or nothing (refused).
fn decide(
    state: &mut HubState,
    request: &OpenRequest<'_>,
    result: &mut AttachOpenResult,
) -> Result<Option<Granted>, StreamError> {
    let caller = request.caller;
    if request.role == LeaseRole::Observe {
        return Ok(Some(state.leases.observe(caller, request.fresh)?));
    }
    if let Some(takeover) = request.takeover {
        let takeover = Takeover::new(ControlSlot::Interact, takeover, request.fresh)?;
        return Ok(Some(state.leases.take_over(caller, &takeover)?));
    }
    let acquire = Acquire::new(ControlSlot::Interact, request.fresh);
    match state.leases.acquire(caller, acquire) {
        Ok(granted) => Ok(Some(granted)),
        Err(LeaseError::Conflict { holder }) => {
            let holder = lease_record(&holder);
            result.outcome = OpenEnum::Known(AttachOutcome::Held);
            result.retry = Some(RetryBoundary::lease_released(holder.clone()));
            result.holder = Some(holder);
            Ok(Some(state.leases.observe(caller, request.fresh)?))
        }
        Err(error @ LeaseError::Authority(_)) => {
            result.outcome = OpenEnum::Known(AttachOutcome::Refused);
            result.reason = Some(error.kind().to_owned());
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

/// Closes the streams of a lease an explicit takeover displaced, naming the new owner.
fn displace(state: &mut HubState, granted: &Granted, result: &mut AttachOpenResult) {
    for event in &granted.transition.events {
        let LeaseEvent::Revoked {
            lease,
            reason: RevokeReason::TakenOver,
            ..
        } = event
        else {
            continue;
        };
        result.displaced = Some(lease_record(lease));
        for viewer in state.viewers.values_mut() {
            if viewer.owner == lease.owner && viewer.interact {
                revoke_input(viewer);
                let mut close =
                    StreamClose::new(StreamCloseReason::TakenOver, viewer.next_offset().wire());
                close.by = Some(lease_record(&granted.lease));
                viewer.close_now(close);
                viewer.wake.notify_one();
            }
        }
    }
}

trait Attached {
    fn stream(&mut self, stream: &StreamId, first: Checkpoint);
}

impl Attached for AttachOpenResult {
    fn stream(&mut self, stream: &StreamId, first: Checkpoint) {
        self.stream_id = Some(stream.clone());
        self.first_paint = Some(first);
    }
}

fn reason_token(close: &StreamClose) -> String {
    serde_json::to_value(&close.reason)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "ended".to_owned())
}
