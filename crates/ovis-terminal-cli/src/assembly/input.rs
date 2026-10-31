//! `input.raw` through the landed sequencer: geometry input-readiness, the interact lease
//! of the same principal and client, the raw grant, then an ordered holder write.

use std::collections::BTreeMap;

use base64::Engine as _;
use ovis_terminal_authz::Session;
use ovis_terminal_core::input::receipts::Receipt;
use ovis_terminal_core::input::sequencer::{Expected, Payload, Request, Submission, View, Writer};
use ovis_terminal_core::leases::{Lease, LeaseView, TerminalLeases};
use ovis_terminal_core::registry::{LeaseId, TerminalId};
use ovis_terminal_protocol::errors::{BoundaryError, OpId};
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_protocol::schema::ids::{ClientId, LeaseId as WireLeaseId};
use ovis_terminal_protocol::schema::methods::input::{
    InputKeysParams, InputPasteParams, InputRawParams, InputResult, InputSendParams,
};
use serde_json::{Value, json};

use super::actor::Runtime;
use super::directory::Directory;
use super::error::refusal;
use super::live::Live;
use super::records::unreported;

fn internal<E: std::fmt::Debug>(cause: E) -> BoundaryError {
    super::error::internal("input admission")(cause)
}

/// The client label of `lease` when `session` owns it; knowing an id grants nothing.
pub(super) fn owned_client(
    view: &LeaseView,
    (session, lease): (&Session, LeaseId),
) -> Result<ClientId, BoundaryError> {
    let mut leases = view
        .interact
        .iter()
        .chain(view.geometry.iter())
        .chain(view.observers.iter());
    leases
        .find(|held: &&Lease| {
            held.lease_id == lease && held.owner.principal == *session.principal()
        })
        .map(|held| held.owner.client.clone())
        .ok_or_else(|| {
            refusal(
                "no_lease",
                "attach with role interact and present its lease_id",
            )
        })
}

/// The terminal a request names, served live by this runtime.
pub(super) fn live_terminal<'a>(
    (directory, served): (&Directory, &'a mut BTreeMap<TerminalId, Live>),
    target: &ovis_terminal_protocol::schema::common::Target,
) -> Result<&'a mut Live, BoundaryError> {
    use ovis_terminal_server::dispatch::lanes::agent_status::TerminalDirectory as _;
    let resolved = directory
        .resolve(target)
        .map_err(|error| refusal(error.kind(), error.hint()))?;
    served
        .get_mut(&resolved.terminal)
        .ok_or_else(|| refusal("not_found", "name a terminal this server spawned"))
}

fn result(receipt: &Receipt) -> Result<Value, BoundaryError> {
    let value = json!({
        "receipt": {
            "op_id": receipt.op_id,
            "queued_seq": receipt.queued_seq,
            "admitted_bytes": receipt.admitted_bytes,
            "output_cursor": {"offset": receipt.output_cursor},
            "mode_revision": receipt.mode_revision,
        },
        "status_at_send": unreported(),
    });
    let checked: InputResult = serde_json::from_value(value).map_err(internal)?;
    serde_json::to_value(checked).map_err(internal)
}

/// One write request as the sequencer admits it.
#[derive(Clone, Copy)]
pub(super) struct Admission<'a> {
    pub(super) target: &'a Target,
    pub(super) op_id: &'a OpId,
    pub(super) lease: Option<&'a WireLeaseId>,
    pub(super) payload: Payload<'a>,
}

/// Client label of a write presenting no lease; only an unleased terminal admits one.
const UNLEASED_CLIENT: &str = "rpc";

impl Runtime {
    /// Admits one write through geometry readiness, the lease of the same principal and
    /// client (or the write grant on an unleased terminal) and the sequencer, then writes
    /// whatever passes its write-boundary recheck.
    pub(super) fn admit_input(
        &mut self,
        session: &Session,
        admission: Admission<'_>,
    ) -> Result<Value, BoundaryError> {
        let runtime_id = self.identity.runtime_id();
        let Self {
            directory,
            live: served,
            holder,
            ..
        } = self;
        let live = live_terminal((directory, served), admission.target)?;
        if let Some(prior) = live.sequencer.receipt(admission.op_id) {
            return result(prior);
        }
        live.geometry
            .input_ready()
            .map_err(|error| refusal(error.kind(), error.hint()))?;
        let lease = admission
            .lease
            .map(|lease| lease.as_str().parse::<LeaseId>())
            .transpose()
            .map_err(|_| refusal("invalid_params", "lease_id must name a lease"))?;
        let view = live
            .hub
            .with_leases(TerminalLeases::view)
            .map_err(internal)?;
        let client = match lease {
            Some(lease) => owned_client(&view, (session, lease))?,
            None => ClientId::new(UNLEASED_CLIENT.to_owned()),
        };
        let (modes, moment) = live.view_parts().map_err(internal)?;
        let writer = Writer::new(session, &client, moment.at);
        let writer = lease.map_or(writer, |lease| writer.presenting(lease));
        let expected = Expected::new(runtime_id, live.resolved.clone());
        let submission = Submission::new(admission.op_id.clone(), expected, admission.payload);
        let sequencer = &mut live.sequencer;
        let receipt = live
            .hub
            .with_leases(|leases| {
                sequencer.admit(
                    &View::new(leases, &modes, moment),
                    Request::new(writer, submission),
                )
            })
            .map_err(|error| error.boundary())?
            .map_err(|error| refusal(error.kind(), error.hint()))?;
        live.pump(holder).map_err(internal)?;
        result(&receipt)
    }

    /// `input.raw`: base64 bytes under the interact lease.
    pub(super) fn input_raw(
        &mut self,
        session: &Session,
        params: &InputRawParams,
    ) -> Result<Value, BoundaryError> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(params.data_b64.as_bytes())
            .map_err(|_| refusal("invalid_params", "data_b64 must be standard base64"))?;
        let admission = Admission {
            target: &params.target,
            op_id: &params.op_id,
            lease: Some(&params.lease_id),
            payload: Payload::Raw(&bytes),
        };
        self.admit_input(session, admission)
    }

    /// `input.send`: literal text and its submission as one entry.
    pub(super) fn input_send(
        &mut self,
        session: &Session,
        params: &InputSendParams,
    ) -> Result<Value, BoundaryError> {
        let admission = Admission {
            target: &params.target,
            op_id: &params.op_id,
            lease: params.lease_id.as_ref(),
            payload: Payload::Text {
                text: &params.text,
                submit: &params.submit,
            },
        };
        self.admit_input(session, admission)
    }

    /// `input.keys`: named keys encoded against the current modes.
    pub(super) fn input_keys(
        &mut self,
        session: &Session,
        params: &InputKeysParams,
    ) -> Result<Value, BoundaryError> {
        let admission = Admission {
            target: &params.target,
            op_id: &params.op_id,
            lease: params.lease_id.as_ref(),
            payload: Payload::Keys(&params.keys),
        };
        self.admit_input(session, admission)
    }

    /// `input.paste`: pasted text under its bracketing policy.
    pub(super) fn input_paste(
        &mut self,
        session: &Session,
        params: &InputPasteParams,
    ) -> Result<Value, BoundaryError> {
        let admission = Admission {
            target: &params.target,
            op_id: &params.op_id,
            lease: params.lease_id.as_ref(),
            payload: Payload::Paste {
                text: &params.text,
                policy: params.bracketed,
            },
        };
        self.admit_input(session, admission)
    }
}
