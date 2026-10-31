//! `template.list`, `template.get` and `terminal.first_message`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ovis_terminal_authz::{Action, Session};
use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::ops::{OperationEvent, OperationSnapshot};
use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use ovis_terminal_protocol::schema::common::NoParams;
use ovis_terminal_protocol::schema::methods::template::{TemplateGetParams, TemplateListResult};
use ovis_terminal_protocol::schema::methods::terminal::TerminalFirstMessageParams;
use ovis_terminal_protocol::schema::scalar::DurationMs;
use ovis_terminal_services::templates::{Template, TemplateError};
use ovis_terminal_store::Snapshot;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{Launcher, TemplateLaunchLane, refusal};
use crate::dispatch::{Access, Completed, DispatchError, Execution, Method, Read};

/// Declares one method over the lane's shared state.
macro_rules! lane_method {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug)]
        pub struct $name<L>(Arc<TemplateLaunchLane<L>>);

        impl<L> $name<L> {
            /// The method over `lane`'s shared state.
            #[must_use]
            pub fn new(lane: &Arc<TemplateLaunchLane<L>>) -> Self {
                Self(Arc::clone(lane))
            }
        }
    };
}

lane_method!(
    /// `template.list`: every template by name and version.
    ListTemplates
);
lane_method!(
    /// `template.get`: one template, its latest version unless pinned.
    GetTemplate
);
lane_method!(
    /// `terminal.first_message`: the requesting principal's queued first message.
    GetFirstMessage
);

fn boundary(error: &TemplateError) -> BoundaryError {
    BoundaryError::domain(ErrorKind::from(error.kind().to_owned()), false).with_hint(error.hint())
}

fn encoded(value: &impl Serialize) -> Result<Value, BoundaryError> {
    serde_json::to_value(value).map_err(|_| {
        BoundaryError::domain(ErrorKind::from("internal".to_owned()), false)
            .with_hint("inspect the template definition encoding")
    })
}

impl<L: Launcher> Read for ListTemplates<L> {
    const NAME: &'static str = "template.list";
    type Params = NoParams;

    fn resolve(&self, _: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        Ok(self.0.access(Action::Read))
    }

    fn read(&self, _: Self::Params, _: &Snapshot) -> Result<Value, BoundaryError> {
        let templates = self.0.catalog.list().map_err(|error| boundary(&error))?;
        let records = templates.iter().map(Template::record).collect();
        encoded(&TemplateListResult::new(records))
    }
}

impl<L: Launcher> Read for GetTemplate<L> {
    const NAME: &'static str = "template.get";
    type Params = TemplateGetParams;

    fn resolve(&self, _: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        Ok(self.0.access(Action::Read))
    }

    fn read(&self, params: Self::Params, _: &Snapshot) -> Result<Value, BoundaryError> {
        let template = self
            .0
            .catalog
            .get(&params.name, params.version.as_deref())
            .map_err(|error| boundary(&error))?;
        encoded(&template.record())
    }
}

impl<L, D, P> Method<D, P> for GetFirstMessage<L>
where
    L: Launcher,
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned + Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    const NAME: &'static str = "terminal.first_message";
    type Params = TerminalFirstMessageParams;

    fn resolve(&self, _: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        Ok(self.0.access(Action::Read))
    }

    async fn run(
        &self,
        params: Self::Params,
        execution: Execution<D, P>,
    ) -> Result<Completed, DispatchError> {
        let shared = Arc::clone(&self.0.shared);
        let principal = execution.decision.principal().clone();
        let within = params.settled_within_ms.map_or(0, DurationMs::get);
        let until = Instant::now()
            .checked_add(Duration::from_millis(within))
            .ok_or(DispatchError::DeadlineOverflow)?;
        let record = tokio::task::spawn_blocking(move || {
            shared
                .book
                .wait_settled((&principal, &params.launch_op), until)
                .map_err(|error| refusal(error.kind(), error.hint(), error))
        })
        .await??;
        execution.commit(move || Ok(record)).await
    }
}
