use std::path::PathBuf;
use std::sync::Arc;

use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::ops::{OperationEvent, OperationProducer};
use ovis_terminal_core::registry::{
    MachineRuntime, ResourceName, ResourceTarget, ResourceTargetKey, TargetIndex, TargetRequest,
    TargetResolutionError, TerminalGeneration, TerminalId,
};
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_protocol::schema::methods::terminal::TerminalKeepParams;
use ovis_terminal_server::dispatch::{Access, Completed, DispatchError, Execution, Method};
use serde_json::json;

use super::support::{Fixture, TestResult, at, error_kind, op_id, request, runtime};

#[derive(Debug, Clone)]
pub(crate) struct Terminal {
    index: TargetIndex,
    scope: MachineRuntime,
    pub(crate) root: PathBuf,
    pub(crate) generation: TerminalGeneration,
}

impl Terminal {
    pub(crate) fn new(root: PathBuf) -> TestResult<Self> {
        let runtime = runtime(1)?;
        let scope = MachineRuntime::new(runtime.machine_id(), runtime.runtime_id());
        let targets = [1, 2]
            .into_iter()
            .map(|id| {
                Ok(ResourceTarget::new(
                    scope,
                    TerminalId::try_from(id)?,
                    ResourceName::new("duplicate")?,
                ))
            })
            .collect::<TestResult<Vec<_>>>()?;
        std::fs::create_dir(&root)?;
        Ok(Self {
            index: TargetIndex::from_targets(targets),
            scope,
            root,
            generation: TerminalGeneration::new(2)?,
        })
    }

    pub(crate) fn access(&self, target: &Target) -> Result<Access, DispatchError> {
        let request = match target {
            Target::TerminalId(id) => TargetRequest::Id(ResourceTargetKey::new(
                self.scope,
                id.as_str().parse().map_err(TargetResolutionError::from)?,
            )),
            Target::Name(name) => TargetRequest::Name {
                machine: None,
                name: ResourceName::new(name.as_str()).map_err(TargetResolutionError::from)?,
            },
            Target::Address(_) => return Err(TargetResolutionError::NotFound.into()),
            _ => return Err(TargetResolutionError::NeedsTarget.into()),
        };
        let target = self.index.resolve(request)?;
        Ok(Access::new(
            Action::Input,
            ResolvedTarget::new(
                target.machine_id(),
                TargetResource::Terminal {
                    placement: None,
                    terminal: target.terminal_id(),
                    incarnation: self.generation,
                },
            ),
        ))
    }
}

impl Method<OperationEvent, OperationProducer> for Terminal {
    const NAME: &'static str = "terminal.keep";
    type Params = TerminalKeepParams;

    fn resolve(&self, params: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        self.access(&params.target)
    }

    async fn run(
        &self,
        params: Self::Params,
        execution: Execution<OperationEvent, OperationProducer>,
    ) -> Result<Completed, DispatchError> {
        let TargetResource::Terminal { terminal, .. } = execution.target().resource() else {
            return Err(DispatchError::Registration);
        };
        let target = *terminal;
        let path = self.root.join(target.to_string());
        execution
            .commit(move || {
                std::fs::write(path, if params.keep { b"keep" } else { b"drop" })?;
                Ok(json!({"terminal_id":target, "keep":params.keep}))
            })
            .await
    }
}

#[tokio::test]
async fn explicit_identity_mutates_only_that_target_and_ambiguity_lists_candidates() -> TestResult {
    // Given a real target index with two same-named resources and separate effect paths.
    let fixture = Fixture::new()?;
    let server = fixture
        .socket
        .serve(1, Arc::clone(&fixture.dispatch))
        .await?;
    let mut client = fixture.socket.connect().await?;
    client.hello("1", &fixture.socket.bearer).await?;
    // When a typo, an ambiguous name, and finally an explicit ID are submitted.
    for (id, target, kind, path) in [
        (
            "typo",
            json!({"pane_id":"terminal:2"}),
            "invalid_params",
            Some("params.target.pane_id"),
        ),
        (
            "ambiguous",
            json!({"name":"duplicate"}),
            "ambiguous_target",
            None,
        ),
        (
            "missing",
            json!({"terminal_id":"terminal:99"}),
            "not_found",
            None,
        ),
    ] {
        client
            .send(&request(
                id,
                "terminal.keep",
                &json!({
                    "op_id":op_id(), "target":target, "keep":true,
                }),
            ))
            .await?;
        let rejected = client.expect_next().await?;
        assert_eq!(error_kind(&rejected), kind);
        if let Some(path) = path {
            assert_eq!(at(&rejected, "/error/data/field_path"), path);
        }
        if id == "ambiguous" {
            let ids: Vec<_> = at(&rejected, "/error/data/candidates")
                .as_array()
                .ok_or("candidates")?
                .iter()
                .map(|candidate| at(candidate, "/terminal_id"))
                .collect();
            assert_eq!(ids, vec![&json!("terminal:1"), &json!("terminal:2")]);
        }
    }
    assert_eq!(
        std::fs::read_dir(fixture.socket.path("effects"))?.count(),
        0
    );
    client
        .send(&request(
            "target",
            "terminal.keep",
            &json!({
                "op_id":op_id(), "target":{"terminal_id":"terminal:2"}, "keep":true,
            }),
        ))
        .await?;
    let result = client.expect_next().await?;
    // Then the actual effect exists only at terminal 2, with no focus fallback.
    assert_eq!(at(&result, "/result/terminal_id"), "terminal:2");
    assert!(!fixture.socket.path("effects/terminal:1").exists());
    assert_eq!(
        std::fs::read(fixture.socket.path("effects/terminal:2"))?,
        b"keep"
    );
    drop(client);
    server.shutdown().await?;
    fixture.close().await
}
