//! `agent.children` (plan todo 41): the children of a terminal's current program
//! incarnation, answered from a committed snapshot without a receipt.
//!
//! A terminal whose program was replaced lists only the new incarnation's children; the
//! finished incarnation's children stay bound to it, never to the reused address.

use std::sync::Arc;

use ovis_terminal_agent::tree::{TreeError, TreeService};
use ovis_terminal_authz::{Action, Session};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::methods::agent::AgentTargetParams;
use ovis_terminal_store::Snapshot;
use serde_json::Value;

use super::agent_status::{TerminalDirectory, terminal_access};
use crate::dispatch::{Access, DispatchError, Read};

/// Shared state of the agent tree methods.
#[derive(Debug)]
pub struct AgentTreeLane<T> {
    terminals: T,
    tree: Arc<TreeService>,
}

impl<T: TerminalDirectory> AgentTreeLane<T> {
    /// Serves `tree` for the terminals `terminals` resolves.
    pub fn new(terminals: T, tree: Arc<TreeService>) -> Arc<Self> {
        Arc::new(Self { terminals, tree })
    }
}

/// `agent.children`: the children of the target's current program incarnation.
#[derive(Debug)]
pub struct ListChildren<T>(Arc<AgentTreeLane<T>>);

impl<T> ListChildren<T> {
    /// The method over `lane`'s shared state.
    #[must_use]
    pub fn new(lane: &Arc<AgentTreeLane<T>>) -> Self {
        Self(Arc::clone(lane))
    }
}

fn refused(error: &TreeError) -> BoundaryError {
    BoundaryError::Domain {
        data: Box::new(error.error_data()),
    }
}

impl<T: TerminalDirectory> Read for ListChildren<T> {
    const NAME: &'static str = "agent.children";
    type Params = AgentTargetParams;

    fn resolve(&self, params: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        terminal_access(&self.0.terminals, &params.target, Action::Read)
    }

    fn read(&self, params: Self::Params, snapshot: &Snapshot) -> Result<Value, BoundaryError> {
        let terminal = self
            .0
            .terminals
            .resolve(&params.target)
            .map_err(|error| DispatchError::from(error).boundary())?
            .terminal;
        let children = self
            .0
            .tree
            .children(snapshot, terminal)
            .map_err(|error| refused(&error))?;
        serde_json::to_value(children).map_err(|error| refused(&TreeError::Encode(error)))
    }
}
