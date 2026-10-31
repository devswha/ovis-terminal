//! An expected-incarnation adapter at the real typed dispatch boundary.

use std::sync::Arc;

use ovis_terminal_authz::{Session, TargetResource};
use ovis_terminal_core::ops::{OperationEvent, OperationProducer};
use ovis_terminal_core::registry::TargetResolutionError;
use ovis_terminal_protocol::schema::methods::terminal::TerminalKillParams;
use ovis_terminal_server::dispatch::{Access, Completed, DispatchError, Execution, Method};
use serde_json::json;

use super::support::{Fixture, TestResult, at, error_kind, op_id, request};
use super::targeting::Terminal;

#[derive(Debug)]
pub(crate) struct Kill(pub(crate) Terminal);

impl Method<OperationEvent, OperationProducer> for Kill {
    const NAME: &'static str = "terminal.kill";
    type Params = TerminalKillParams;

    fn resolve(&self, params: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        let access = self.0.access(&params.target)?;
        if params
            .expect
            .as_ref()
            .and_then(|expected| expected.terminal_incarnation)
            .is_some_and(|expected| expected.get() != self.0.generation.get())
        {
            return Err(TargetResolutionError::StaleTarget.into());
        }
        Ok(access)
    }

    async fn run(
        &self,
        _: Self::Params,
        execution: Execution<OperationEvent, OperationProducer>,
    ) -> Result<Completed, DispatchError> {
        let TargetResource::Terminal {
            terminal,
            incarnation,
            ..
        } = execution.target().resource()
        else {
            return Err(DispatchError::Registration);
        };
        let path = self.0.root.join(terminal.to_string());
        let incarnation = *incarnation;
        execution
            .commit(move || {
                std::fs::write(path, b"signal")?;
                Ok(json!({"incarnation":incarnation, "observed":"unknown"}))
            })
            .await
    }
}

#[tokio::test]
async fn stale_incarnation_is_rejected_before_admission_or_effect() -> TestResult {
    // Given terminal 2 now at generation 2, addressed through the published kill schema.
    let fixture = Fixture::new()?;
    let server = fixture
        .socket
        .serve(1, Arc::clone(&fixture.dispatch))
        .await?;
    let mut client = fixture.socket.connect().await?;
    client.hello("1", &fixture.socket.bearer).await?;
    let stale_op = op_id();
    // When the caller still expects generation 1.
    client
        .send(&request(
            "stale",
            "terminal.kill",
            &json!({
                "op_id":stale_op, "target":{"terminal_id":"terminal:2"},
                "expect":{"terminal_incarnation":"1"}, "escalation":{"hup_ms":"0","term_ms":"0"},
            }),
        ))
        .await?;
    let stale = client.expect_next().await?;
    let stale_effect = fixture.socket.path("effects/terminal:2").exists();
    client
        .send(&request("stale-op", "op.get", &json!({"op_id":stale_op})))
        .await?;
    let absent = client.expect_next().await?;
    // A fresh incarnation reaches the same adapter, so rejection is not an unavailable stub.
    client
        .send(&request(
            "current",
            "terminal.kill",
            &json!({
                "op_id":op_id(), "target":{"terminal_id":"terminal:2"},
                "expect":{"terminal_incarnation":"2"}, "escalation":{"hup_ms":"0","term_ms":"0"},
            }),
        ))
        .await?;
    let current = client.expect_next().await?;
    let bytes = std::fs::read(fixture.socket.path("effects/terminal:2"))?;
    drop(client);
    server.shutdown().await?;
    fixture.close().await?;
    // Then the stale request has no receipt or effect, while the current one is accepted.
    assert_eq!(error_kind(&stale), "stale_target");
    assert!(!stale_effect);
    assert_eq!(error_kind(&absent), "not_found");
    assert_eq!(at(&current, "/result/incarnation"), "2");
    assert_eq!(bytes, b"signal");
    Ok(())
}
