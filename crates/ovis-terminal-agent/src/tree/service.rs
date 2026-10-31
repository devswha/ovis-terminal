//! Parent admission, child links and the children read of one runtime.

use std::sync::Arc;

use ovis_terminal_core::registry::{RuntimeId, RuntimeIdentity, TerminalId};
use ovis_terminal_protocol::domain::agent_tree::{ChildLink, ChildRef, ParentIdentity};
use ovis_terminal_protocol::schema::ids::{MachineId, TerminalId as WireTerminal};
use ovis_terminal_protocol::schema::methods::agent::{AgentChildrenResult, ChildRecord};
use ovis_terminal_protocol::schema::methods::terminal::ChildPolicy;
use ovis_terminal_protocol::schema::records::{ParentRef, StatusSnapshot};
use ovis_terminal_protocol::schema::scalar::Revision;
use ovis_terminal_store::{CasWrite, Snapshot};

use super::TreeError;
use super::store::{self, CHILD_LIMIT, ChildRow, ParentKey};
use crate::prompts::PromptService;
use crate::status::{ProcessOwner, StatusError, StatusService};

/// Parent/child links of every terminal of one runtime.
#[derive(Debug)]
pub struct TreeService {
    pub(super) runtime: RuntimeId,
    pub(super) machine: MachineId,
    pub(super) identity: RuntimeIdentity,
    pub(super) status: Arc<StatusService>,
    pub(super) prompts: Arc<PromptService>,
}

/// A child program to link to a parent incarnation in the launch's commit unit.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Link {
    /// The parent, by incarnation.
    pub parent: ParentRef,
    /// The launched child program.
    pub child: ProcessOwner,
    /// Declared policy toward the parent.
    pub policy: ChildPolicy,
}

impl Link {
    /// Links `child` to `parent` under `policy`.
    #[must_use]
    pub const fn new(parent: ParentRef, child: ProcessOwner, policy: ChildPolicy) -> Self {
        Self {
            parent,
            child,
            policy,
        }
    }
}

/// The committed link and the writes the launch's unit carries.
#[derive(Debug)]
#[non_exhaustive]
pub struct Linked {
    /// The link as committed.
    pub link: ChildLink,
    /// State writes of the link.
    pub writes: Vec<CasWrite>,
}

pub(super) fn wire(terminal: TerminalId) -> WireTerminal {
    WireTerminal::new(terminal.to_string())
}

pub(super) fn child_ref(owner: ProcessOwner) -> ChildRef {
    ChildRef::new(wire(owner.terminal), owner.process)
}

impl TreeService {
    /// Links of the runtime `identity`, reading program incarnations and parent identity
    /// from `status` and expiring ended children's prompts in `prompts`.
    #[must_use]
    pub fn new(
        identity: RuntimeIdentity,
        (status, prompts): (Arc<StatusService>, Arc<PromptService>),
    ) -> Self {
        Self {
            runtime: identity.runtime_id(),
            machine: MachineId::new(identity.machine_id().to_string()),
            identity,
            status,
            prompts,
        }
    }

    pub(super) fn local(&self, parent: ParentRef) -> ParentKey {
        ParentKey {
            machine: self.machine.clone(),
            parent,
        }
    }

    /// The live program `parent` names, refused before anything is launched or linked.
    ///
    /// # Errors
    ///
    /// Returns `parent_finished` when that incarnation already finished, `stale_target`
    /// when the terminal now runs another incarnation, `parent_not_found` when it runs
    /// no known program, or a store or decoding failure.
    pub fn admit(
        &self,
        snapshot: &Snapshot,
        parent: &ParentRef,
    ) -> Result<ProcessOwner, TreeError> {
        let terminal: TerminalId = parent
            .terminal_id
            .as_str()
            .parse()
            .map_err(|_| TreeError::ParentNotFound)?;
        let key = self.local(parent.clone());
        if store::finished(snapshot, (self.runtime, &key))?.covers(parent.incarnation) {
            return Err(TreeError::ParentFinished);
        }
        match self.status.tokens().current(terminal)? {
            Some(owner) if owner.process == parent.incarnation => Ok(owner),
            Some(_) => Err(TreeError::StaleTarget),
            None => Err(TreeError::ParentNotFound),
        }
    }

    /// Links a launched child to its live parent incarnation, keeping the parent's
    /// session and turn identity as they stand now.
    ///
    /// # Errors
    ///
    /// Returns the [`admit`](Self::admit) refusals, `limit_exceeded`, or a store or
    /// decoding failure.
    pub fn link(&self, snapshot: &Snapshot, link: Link) -> Result<Linked, TreeError> {
        let parent = self.admit(snapshot, &link.parent)?;
        let status = self
            .status
            .location(parent.terminal)
            .read(snapshot, Some(parent.process))?
            .row
            .status;
        let key = self.local(link.parent.clone());
        self.record(snapshot, (key, link, Some(ParentIdentity::of(&status))))
    }

    /// Writes `link` into the row of `key`, the parent's machine and incarnation.
    pub(super) fn record(
        &self,
        snapshot: &Snapshot,
        (key, link, identity): (ParentKey, Link, Option<ParentIdentity>),
    ) -> Result<Linked, TreeError> {
        let mut row = store::parent(snapshot, (self.runtime, &key))?;
        let running = row.value.links.iter().filter(|link| link.is_running());
        if running.count() >= CHILD_LIMIT {
            return Err(TreeError::LimitExceeded);
        }
        let child = child_ref(link.child);
        let record = ChildLink::linked(
            (child.clone(), link.parent, key.machine.clone()),
            (link.policy, identity),
            StatusService::unit_event(snapshot),
        );
        row.value.links.push(record.clone());
        let index = store::child(snapshot, (self.runtime, &child))?.with(|_| ChildRow {
            parent_machine: key.machine,
            parent: key.parent,
        });
        Ok(Linked {
            link: record,
            writes: vec![row.write()?, index.write()?],
        })
    }

    /// Children of `terminal`'s current program incarnation, each with its declared
    /// status and unacknowledged notices; a reused address lists only its own children.
    ///
    /// # Errors
    ///
    /// Returns a store or decoding failure.
    pub fn children(
        &self,
        snapshot: &Snapshot,
        terminal: TerminalId,
    ) -> Result<AgentChildrenResult, TreeError> {
        let revision = Revision::new(snapshot.revision().get());
        let Some(current) = self.status.tokens().current(terminal)? else {
            return Ok(AgentChildrenResult::new(Vec::new(), revision));
        };
        let key = self.local(ParentRef::new(wire(terminal), current.process));
        let row = store::parent(snapshot, (self.runtime, &key))?.value;
        let mut children = Vec::with_capacity(row.links.len());
        for link in &row.links {
            let unseen = row
                .notices
                .iter()
                .filter(|notice| notice.child == link.child && notice.acknowledged_seq.is_none())
                .count();
            let status = self.child_status(snapshot, &link.child)?;
            let unseen = u32::try_from(unseen).unwrap_or(u32::MAX);
            children.push(ChildRecord::new(link.child.clone(), status, unseen));
        }
        Ok(AgentChildrenResult::new(children, revision))
    }

    /// The declared status of `child`'s incarnation; a terminal that moved on to a newer
    /// program no longer holds it, so that incarnation reads as unreported.
    fn child_status(
        &self,
        snapshot: &Snapshot,
        child: &ChildRef,
    ) -> Result<StatusSnapshot, TreeError> {
        let Ok(terminal) = child.terminal_id.as_str().parse::<TerminalId>() else {
            return Ok(StatusSnapshot::unreported(child.process_incarnation));
        };
        match self
            .status
            .location(terminal)
            .read(snapshot, Some(child.process_incarnation))
        {
            Ok(stored) => Ok(stored.row.status),
            Err(StatusError::StaleIncarnation) => {
                Ok(StatusSnapshot::unreported(child.process_incarnation))
            }
            Err(error) => Err(error.into()),
        }
    }
}

/// The program `child` names on this machine, when its terminal identity parses.
pub(super) fn owner_of(child: &ChildRef) -> Option<ProcessOwner> {
    let terminal = child.terminal_id.as_str().parse().ok()?;
    Some(ProcessOwner::new(terminal, child.process_incarnation))
}
