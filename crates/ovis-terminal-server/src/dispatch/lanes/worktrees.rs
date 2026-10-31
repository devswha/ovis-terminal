//! The `worktree.*` methods over the worktree service.
//!
//! Refusals are found before the commit boundary, Git runs inside it and stops when the
//! operation is cancelled, and every change commits one `worktree.changed` event.

use std::sync::Arc;

use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::events::{Coordinator, EventData, SnapshotProducer};
use ovis_terminal_core::ops::{OperationEvent, OperationSnapshot};
use ovis_terminal_core::registry::MachineId;
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::methods::worktree::{
    WorktreeChange, WorktreeCreateParams, WorktreeListParams, WorktreeListResult,
    WorktreeOpenParams, WorktreeRemoveParams,
};
use ovis_terminal_services::worktrees::{Call, Cancel, Change, WorktreeEvent, Worktrees};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::runtime::Handle;

use crate::dispatch::{Access, Completed, DispatchError, Dispatcher, Execution, Method};

/// The worktree service, the coordinator its events commit to and the owning machine.
#[derive(Debug)]
pub struct Lane<D, P> {
    service: Arc<Worktrees>,
    core: Arc<Coordinator<D, P>>,
    machine: MachineId,
}

impl<D, P> Clone for Lane<D, P> {
    fn clone(&self) -> Self {
        Self {
            service: Arc::clone(&self.service),
            core: Arc::clone(&self.core),
            machine: self.machine,
        }
    }
}

macro_rules! worktree_method {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug)]
        pub struct $name<D, P>(Lane<D, P>);
    };
}

worktree_method!(
    /// `worktree.list`.
    WorktreeList
);
worktree_method!(
    /// `worktree.create`.
    WorktreeCreate
);
worktree_method!(
    /// `worktree.open`.
    WorktreeOpen
);
worktree_method!(
    /// `worktree.remove`.
    WorktreeRemove
);

impl<D, P> Lane<D, P>
where
    D: EventData
        + From<OperationEvent>
        + From<WorktreeEvent>
        + Clone
        + Serialize
        + DeserializeOwned,
    D: Send + Sync + std::fmt::Debug + 'static,
    P: SnapshotProducer + Send + Sync + std::fmt::Debug + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    /// Binds the service to the coordinator dispatch shares.
    pub const fn new(
        service: Arc<Worktrees>,
        core: Arc<Coordinator<D, P>>,
        machine: MachineId,
    ) -> Self {
        Self {
            service,
            core,
            machine,
        }
    }

    /// Registers all four methods.
    ///
    /// # Errors
    ///
    /// Returns a registration failure when a method is already registered.
    pub fn register(self, dispatcher: &mut Dispatcher<D, P>) -> Result<(), DispatchError> {
        dispatcher.register(WorktreeList(self.clone()))?;
        dispatcher.register(WorktreeCreate(self.clone()))?;
        dispatcher.register(WorktreeOpen(self.clone()))?;
        dispatcher.register(WorktreeRemove(self))
    }

    const fn access(&self, action: Action) -> Access {
        Access::new(
            action,
            ResolvedTarget::new(self.machine, TargetResource::Machine),
        )
    }

    fn publish(&self, op: &OpId, change: Change) -> Result<WorktreeChange, DispatchError> {
        let commit = change.commit(op, self.machine)?;
        let cursor = self.core.commit(commit).map_err(std::io::Error::other)?;
        Ok(change.into_wire(cursor.seq))
    }
}

/// Implements one mutation: check before the boundary, apply and publish inside it.
macro_rules! mutation {
    ($method:ident, $name:literal, $params:ty, $action:ident, ($check:ident, $apply:ident)) => {
        impl<D, P> Method<D, P> for $method<D, P>
        where
            D: EventData + From<OperationEvent> + From<WorktreeEvent> + Clone + Serialize,
            D: DeserializeOwned + Send + Sync + std::fmt::Debug + 'static,
            P: SnapshotProducer + Send + Sync + std::fmt::Debug + 'static,
            P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
        {
            const NAME: &'static str = $name;
            type Params = $params;

            fn resolve(&self, _: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
                Ok(self.0.access(Action::$action))
            }

            async fn run(
                &self,
                params: Self::Params,
                execution: Execution<D, P>,
            ) -> Result<Completed, DispatchError> {
                let cancel = Cancel::new(execution.cancellation());
                let check = Call {
                    params: &params,
                    cancel: cancel.clone(),
                };
                self.0.service.$check(check).await?;
                let lane = self.0.clone();
                let op = execution.op_id().clone();
                execution
                    .commit(move || {
                        let call = Call {
                            params: &params,
                            cancel,
                        };
                        let change = Handle::current().block_on(lane.service.$apply(call))?;
                        lane.publish(&op, change)
                    })
                    .await
            }
        }
    };
}

mutation!(
    WorktreeCreate,
    "worktree.create",
    WorktreeCreateParams,
    LayoutChange,
    (check_create, create)
);
mutation!(
    WorktreeOpen,
    "worktree.open",
    WorktreeOpenParams,
    LayoutChange,
    (check_open, open)
);
// Removal deletes files and closes workspaces: it needs the admin grant, not layout change.
mutation!(
    WorktreeRemove,
    "worktree.remove",
    WorktreeRemoveParams,
    Administer,
    (check_remove, remove)
);

impl<D, P> Method<D, P> for WorktreeList<D, P>
where
    D: EventData + From<OperationEvent> + From<WorktreeEvent> + Clone + Serialize,
    D: DeserializeOwned + Send + Sync + std::fmt::Debug + 'static,
    P: SnapshotProducer + Send + Sync + std::fmt::Debug + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    const NAME: &'static str = "worktree.list";
    type Params = WorktreeListParams;

    fn resolve(&self, _: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        Ok(self.0.access(Action::Read))
    }

    async fn run(
        &self,
        params: Self::Params,
        execution: Execution<D, P>,
    ) -> Result<Completed, DispatchError> {
        let cancel = Cancel::new(execution.cancellation());
        let service = Arc::clone(&self.0.service);
        execution
            .commit(move || {
                let call = Call {
                    params: &params,
                    cancel,
                };
                let listed = Handle::current().block_on(service.list(call))?;
                Ok(WorktreeListResult::new(listed))
            })
            .await
    }
}
