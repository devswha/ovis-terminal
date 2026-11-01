//! Persisted selection and the activation journal.

use std::path::PathBuf;

use ovis_terminal_protocol::domain::update::{
    ActivationPhase, ActiveBuild, PendingActivation, ReadinessEvidence,
};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::ids::UpdateId;
use ovis_terminal_protocol::schema::methods::server::ServerTransitionResult;
use serde::{Deserialize, Serialize};

/// One selectable build and where its executable lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Selected {
    pub(super) build: ActiveBuild,
    pub(super) binary: PathBuf,
    /// The update that installed it; `None` for the build the runtime started with.
    pub(super) update_id: Option<UpdateId>,
}

/// `active.json`: the build that runs and the one a rollback restores.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Selection {
    pub(super) active: Selected,
    pub(super) previous: Option<Selected>,
}

impl Selection {
    /// `next` becomes active and the current active build its rollback target.
    pub(super) fn advanced(&self, next: Selected) -> Self {
        Self {
            active: next,
            previous: Some(self.active.clone()),
        }
    }
}

/// Which transition a journal records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Step {
    Activate,
    Rollback,
}

/// `activation.json`: everything recovery needs to finish or revert one transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Journal {
    pub(super) update_id: UpdateId,
    pub(super) op_id: OpId,
    pub(super) step: Step,
    pub(super) phase: ActivationPhase,
    pub(super) before: Selection,
    pub(super) after: Selection,
    pub(super) readiness: Option<ReadinessEvidence>,
    pub(super) restart: Option<ServerTransitionResult>,
}

impl Journal {
    pub(super) fn pending(&self) -> PendingActivation {
        PendingActivation::new(self.update_id.clone(), self.op_id.clone(), self.phase)
    }
}
