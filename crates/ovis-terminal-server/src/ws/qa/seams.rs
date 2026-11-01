//! The fixture's implementations of the server's seams: one resolvable terminal, a lease
//! journal committing through the real coordinator, and an inner handler with no methods.

use std::sync::Arc;

use ovis_terminal_authz::{
    EndpointPolicy, FileEvidence, LocalAccount, PeerAdmission, PeerEvidence, UnixSocketEvidence,
    UnixUid,
};
use ovis_terminal_core::events::{Coordinator, EventError, Filter, SnapshotProducer};
use ovis_terminal_core::leases::{LeaseEvent, Transition};
use ovis_terminal_core::registry::TargetResolutionError;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_server::conn::{Call, RequestHandler};
use ovis_terminal_server::dispatch::lanes::agent_status::{ResolvedTerminal, TerminalDirectory};
use ovis_terminal_server::streams::{LeaseJournal, StreamError};
use ovis_terminal_store::{OpId, Snapshot};
use serde_json::Value;

use super::TestResult;
use super::fixture::TERMINAL;

#[derive(Debug)]
pub(crate) struct NoSections;

impl SnapshotProducer for NoSections {
    type Sections = ();

    fn capture(&self, _snapshot: &Snapshot, _filter: &Filter) -> Result<(), EventError> {
        Ok(())
    }
}

pub(crate) type Core = Coordinator<LeaseEvent, NoSections>;

#[derive(Debug)]
pub(crate) struct Journal(pub Arc<Core>);

impl LeaseJournal for Journal {
    fn record(&self, op: OpId, transition: Transition) -> Result<Option<EventSeq>, StreamError> {
        self.0.record(op, transition)
    }
}

#[derive(Debug)]
pub(crate) struct Directory(pub ResolvedTerminal);

impl TerminalDirectory for Directory {
    fn resolve(&self, target: &Target) -> Result<ResolvedTerminal, TargetResolutionError> {
        match target {
            Target::TerminalId(id) if id.as_str() == TERMINAL => Ok(self.0),
            _ => Err(TargetResolutionError::NeedsTarget),
        }
    }
}

#[derive(Debug)]
pub(crate) struct Nothing;

impl RequestHandler for Nothing {
    fn methods(&self) -> Vec<&'static str> {
        Vec::new()
    }

    fn call(&self, _call: Call) -> impl Future<Output = Result<Value, BoundaryError>> + Send {
        std::future::ready(Err(BoundaryError::method_not_found()))
    }
}

/// An owner admission on a private local socket, used to authenticate the issuer.
pub(crate) fn local_admission() -> TestResult<PeerAdmission> {
    let uid = UnixUid::new(1000);
    let socket = UnixSocketEvidence::new(
        FileEvidence::new(uid, 0o600),
        FileEvidence::new(uid, 0o700),
        uid,
    );
    Ok(EndpointPolicy::new(LocalAccount::Unix(uid)).admit(&PeerEvidence::UnixSocket(socket))?)
}
