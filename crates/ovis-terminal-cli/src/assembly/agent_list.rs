//! `agent.list` (IDEAL-064, IDEAL-067): the agent binding of every terminal this runtime
//! knows that has one, read like `session.snapshot` reads its bindings. The terminal list
//! comes from one actor job, and each binding comes from the dispatch log's committed
//! snapshot. It is an unrecorded read answered in one page: `page.limit` truncates the
//! list and says so, and a continuation token is refused because none is ever issued.

use std::sync::Arc;

use ovis_terminal_agent::status::StatusService;
use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::registry::{MachineId, TerminalId};
use ovis_terminal_core::terminals::Record;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::methods::agent::{AgentListParams, AgentListResult};
use ovis_terminal_server::dispatch::{Access, DispatchError, Read};
use ovis_terminal_store::Snapshot;
use serde_json::{Value, json};

use super::actor::{Actor, Runtime};
use super::error::{checked, refusal, worker_gone};

impl Runtime {
    /// Every terminal this runtime knows, in one actor job.
    fn known_terminals(&self) -> Result<Vec<TerminalId>, BoundaryError> {
        let records = self
            .control
            .with_terminals(|terminals, session| Ok(terminals.list(session)))
            .map_err(|error| refusal(error.kind(), error.hint()))?
            .map_err(|error| super::terminal::terminal_refusal(&error))?;
        Ok(records.iter().map(Record::id).collect())
    }
}

/// The first `limit` entries (all of them without a limit), the total and whether the
/// limit cut the list short.
fn one_page<T>(mut entries: Vec<T>, limit: Option<u32>) -> (Vec<T>, usize, bool) {
    let total = entries.len();
    let kept = limit.map_or(total, |limit| usize::try_from(limit).unwrap_or(usize::MAX));
    entries.truncate(kept);
    (entries, total, kept < total)
}

/// `agent.list` as a dispatched read.
#[derive(Debug)]
pub(super) struct AgentList {
    pub(super) actor: Actor,
    pub(super) status: Arc<StatusService>,
    pub(super) machine: MachineId,
}

impl Read for AgentList {
    const NAME: &'static str = "agent.list";
    type Params = AgentListParams;

    fn resolve(&self, _: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        Ok(Access::new(
            Action::Read,
            ResolvedTarget::new(self.machine, TargetResource::Machine),
        ))
    }

    fn read(&self, params: Self::Params, snapshot: &Snapshot) -> Result<Value, BoundaryError> {
        let (after, limit) = params
            .page
            .map_or((None, None), |page| (page.after, page.limit));
        if after.is_some() {
            return Err(refusal(
                "invalid_params",
                "agent.list answers in one page; omit page.after",
            ));
        }
        let terminals = self
            .actor
            .call_blocking(|runtime| runtime.known_terminals())
            .ok_or_else(worker_gone)??;
        let mut agents = Vec::new();
        for terminal in terminals {
            let bound = self
                .status
                .binding(snapshot, terminal)
                .map_err(|error| refusal(error.kind(), error.hint()))?;
            agents.extend(bound);
        }
        let (agents, total, truncated) = one_page(agents, limit);
        checked::<AgentListResult>(json!({
            "agents": agents,
            "page": {"revision": snapshot.revision().get().to_string(),
                "total": u32::try_from(total).unwrap_or(u32::MAX), "next": null,
                "truncated": truncated},
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::one_page;

    #[test]
    fn without_a_limit_every_entry_is_kept_and_nothing_is_truncated() {
        assert_eq!(one_page(vec![1, 2, 3], None), (vec![1, 2, 3], 3, false));
        assert_eq!(one_page(Vec::<u8>::new(), None), (Vec::new(), 0, false));
    }

    #[test]
    fn a_limit_below_the_total_truncates_and_reports_the_full_total() {
        assert_eq!(one_page(vec![1, 2, 3], Some(2)), (vec![1, 2], 3, true));
        assert_eq!(one_page(vec![1, 2, 3], Some(0)), (Vec::new(), 3, true));
    }

    #[test]
    fn a_limit_at_or_above_the_total_is_not_truncation() {
        assert_eq!(one_page(vec![1, 2, 3], Some(3)), (vec![1, 2, 3], 3, false));
        assert_eq!(
            one_page(vec![1, 2, 3], Some(u32::MAX)),
            (vec![1, 2, 3], 3, false)
        );
    }
}
