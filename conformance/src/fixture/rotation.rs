//! The fixture terminal whose input goes through the real ordered input sequencer, and the
//! credential methods that recheck its queued writes (plan todo 57).
//!
//! `rotation` (terminal:6) admits `input.send` through `Sequencer::admit` against the
//! real authority, an unleased lease table and the modes of a real VT, answering with the
//! queued receipt. Its holder stand-in writes only when `fixture.deliver` is called, so a
//! scenario can hold an admitted write before its write boundary, change the credential
//! and then release it. Bytes the stand-in writes are counted in `fixture.pty`.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use ovis_terminal_authz::{Authority, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::input::authority::{raced, recheck};
use ovis_terminal_core::input::encoding::ModeSnapshot;
use ovis_terminal_core::input::receipts::Receipt;
use ovis_terminal_core::input::sequencer::{
    Binding, Expected, Moment, Payload, QueueLimits, Request, Sequencer, SequencerError,
    Submission, View, Writer,
};
use ovis_terminal_core::leases::TerminalLeases;
use ovis_terminal_core::registry::{
    ProcessGeneration, RegistryIdentities, RuntimeIdentity, TerminalGeneration, TerminalId,
};
use ovis_terminal_holder::protocol::{Input, InputWritten};
use ovis_terminal_protocol::domain::credential_rotation::RacedWrite;
use ovis_terminal_protocol::errors::{BoundaryError, ErrorData, ErrorKind, OpId};
use ovis_terminal_protocol::schema::ids::ClientId;
use ovis_terminal_protocol::schema::methods::input::InputSendParams;
use ovis_terminal_protocol::schema::methods::server::WriteSequence;
use ovis_terminal_protocol::schema::scalar::{ByteOffset, TimestampMs};
use ovis_terminal_server::dispatch::lanes::credential_rotation::{
    CredentialLane, QueuedWrites, RevokeCredential, RotateCredential,
};
use ovis_terminal_vt::{Geometry, HistoryLimit, Vt, VtConfig};
use serde_json::{Value, json};

use super::handler::Dispatch;
use super::prompts::Pty;
use super::serve::at;
use crate::error::RunnerError;

/// The sequenced fixture terminal.
pub(crate) const TERMINAL: u64 = 6;

type Registry = RegistryIdentities;

#[derive(Debug)]
struct Lane {
    sequencer: Sequencer,
    leases: TerminalLeases,
    modes: ModeSnapshot,
}

/// One terminal's ordered input and the holder stand-in that writes it on demand.
#[derive(Debug)]
pub(crate) struct Sequenced {
    terminal: TerminalId,
    target: ResolvedTarget<Registry>,
    runtime: RuntimeIdentity,
    lane: Mutex<Lane>,
    pty: Arc<Pty>,
}

fn now() -> TimestampMs {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    TimestampMs::new(u64::try_from(millis).unwrap_or(u64::MAX))
}

fn boundary(error: &SequencerError) -> BoundaryError {
    BoundaryError::domain(ErrorKind::from(error.kind().to_owned()), false).with_hint(error.hint())
}

/// The wire refusal of a sequencer error, kept as the operation's typed failure.
pub(crate) fn refusal(error: &SequencerError) -> ErrorData {
    boundary(error).data().clone()
}

impl Sequenced {
    /// A live sequencer for terminal:6 running process 1, writing into `pty`.
    pub(crate) fn new(
        authority: &Arc<Authority<Registry>>,
        runtime: RuntimeIdentity,
        pty: Arc<Pty>,
    ) -> Result<Arc<Self>, RunnerError> {
        let terminal = TerminalId::try_from(TERMINAL).map_err(at("build the rotation terminal"))?;
        let target = ResolvedTarget::new(
            runtime.machine_id(),
            TargetResource::Terminal {
                placement: None,
                terminal,
                incarnation: TerminalGeneration::new(1).map_err(at("build a generation"))?,
            },
        );
        let binding = Binding::new(Arc::clone(authority), runtime.runtime_id(), target.clone())
            .map_err(at("bind the rotation terminal"))?;
        let mut sequencer = Sequencer::new(binding, QueueLimits::DEFAULT);
        let process = ProcessGeneration::new(1).map_err(at("build a process generation"))?;
        sequencer.stream_ready(process, WriteSequence::new(0));
        let leases = TerminalLeases::new(Arc::clone(authority), target.clone())
            .map_err(at("open the rotation lease table"))?;
        let geometry = Geometry::new(80, 24).map_err(at("build a geometry"))?;
        let vt = Vt::new(VtConfig::new(geometry, HistoryLimit::new(10)));
        Ok(Arc::new(Self {
            terminal,
            target,
            runtime,
            lane: Mutex::new(Lane {
                sequencer,
                leases,
                modes: ModeSnapshot::capture(&vt),
            }),
            pty,
        }))
    }

    /// Whether `terminal` is this sequenced terminal.
    pub(crate) fn serves(&self, terminal: TerminalId) -> bool {
        self.terminal == terminal
    }

    /// The lane state; a panic elsewhere leaves it consistent, since every change to it is
    /// one sequencer call.
    fn lane(&self) -> MutexGuard<'_, Lane> {
        self.lane.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Admits `params` for `session` under `op_id` and answers with the queued receipt.
    pub(crate) fn send(
        &self,
        session: &Session,
        (op_id, params): (OpId, &InputSendParams),
    ) -> Result<Receipt, Box<ErrorData>> {
        for (present, field) in [
            (params.lease_id.is_some(), "params.lease_id"),
            (params.expect.is_some(), "params.expect"),
        ] {
            if present {
                // The fixture terminal has no leases and no expected incarnations to match.
                return Err(Box::new(
                    BoundaryError::invalid_params(Some(field)).data().clone(),
                ));
            }
        }
        let client = ClientId::new(session.principal().as_str().to_owned());
        let at = now();
        let writer = Writer::new(session, &client, at);
        let payload = Payload::Text {
            text: &params.text,
            submit: &params.submit,
        };
        let expected = Expected::new(self.runtime.runtime_id(), self.target.clone());
        let mut lane = self.lane();
        let Lane {
            sequencer,
            leases,
            modes,
        } = &mut *lane;
        let view = View::new(leases, modes, Moment::new(at, ByteOffset::new(0)));
        let request = Request::new(writer, Submission::new(op_id, expected, payload));
        let admitted = sequencer.admit(&view, request);
        drop(lane);
        admitted.map_err(|error| Box::new(refusal(&error)))
    }

    /// The holder stand-in writes `input` in full and acknowledges it.
    fn write(
        &self,
        sequencer: &mut Sequencer,
        input: &Input,
    ) -> Result<Option<Receipt>, BoundaryError> {
        let overflow = || {
            BoundaryError::domain(ErrorKind::from("input_overflow".to_owned()), false)
                .with_hint("the fixture PTY byte count overflowed")
        };
        let written = u64::try_from(input.bytes.len()).map_err(|_| overflow())?;
        self.pty
            .wrote(self.terminal, input.bytes.len())
            .map_err(|_| overflow())?;
        let ack = sequencer.acknowledge(&InputWritten {
            target: input.target.clone(),
            sequence: input.sequence,
            grant_epoch: input.grant.grant_epoch,
            fence_epoch: input.grant.fence_epoch,
            ack_epoch: input.sequence,
            written_bytes: written,
            output_offset: 0,
        });
        ack.map_err(|error| boundary(&error))
    }

    /// Releases the write boundary: every admitted write is rechecked and, if it still
    /// holds, written; returns each settled write.
    pub(crate) fn deliver(&self) -> Result<Value, BoundaryError> {
        let mut guard = self.lane();
        let Lane {
            sequencer,
            leases,
            modes,
        } = &mut *guard;
        let mut settled = Vec::new();
        loop {
            let view = View::new(leases, modes, Moment::new(now(), ByteOffset::new(0)));
            let step = sequencer.next(&view).map_err(|error| boundary(&error))?;
            settled.extend(step.fenced.iter().filter_map(raced));
            let Some(input) = step.dispatch else { break };
            settled.extend(self.write(sequencer, &input)?.as_ref().and_then(raced));
        }
        drop(guard);
        Ok(json!({ "settled": settled }))
    }
}

impl QueuedWrites for Sequenced {
    fn recheck(&self) -> Vec<RacedWrite> {
        let mut guard = self.lane();
        let Lane {
            sequencer,
            leases,
            modes,
        } = &mut *guard;
        let view = View::new(leases, modes, Moment::new(now(), ByteOffset::new(0)));
        // Only an exhausted fence counter fails the sweep; every write then stays queued and
        // is rechecked again at its write boundary, so none was settled now.
        let Ok(swept) = recheck(sequencer, &view) else {
            return Vec::new();
        };
        // The fence write is empty: the stand-in has not started the fenced entry, so its
        // acknowledgement of zero bytes settles it as cancelled. If even that fails, the
        // entry stays with the holder of unknown outcome, which `raced` already reports.
        if let Some(fence) = &swept.fence {
            drop(self.write(sequencer, fence));
        }
        drop(guard);
        swept.raced
    }
}

/// Registers `credential.rotate` and `credential.revoke` over `authority`, rechecking
/// `writes` after each change.
pub(crate) fn register(
    dispatch: &mut Dispatch,
    authority: &Arc<Authority<Registry>>,
    (runtime, writes): (RuntimeIdentity, Arc<Sequenced>),
) -> Result<(), RunnerError> {
    let writes: Arc<dyn QueuedWrites> = writes;
    let lane = CredentialLane::new(Arc::clone(authority), runtime.machine_id(), writes);
    dispatch
        .register(RotateCredential::new(&lane))
        .map_err(at("register credential.rotate"))?;
    dispatch
        .register(RevokeCredential::new(&lane))
        .map_err(at("register credential.revoke"))
}
