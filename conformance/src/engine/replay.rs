//! The `replay` step: audit the Ovis consumer fixture set, then replay its cases.
//!
//! Each case of the selected kind runs against a fresh server in a fresh isolated root
//! over the binary's `connect` bridge; every expectation and the normalized receipt
//! become assertions.

use std::path::{Path, PathBuf};

use ovis_terminal_replay::{
    Bindings, Case, CaseKind, Check, FixtureSet, Options, ScenarioState, Server,
};
use serde_json::{Value, json};

use super::conn::Wait;
use super::exchange::RECORD_BOUND_MS;
use super::root::IsolatedRoot;
use super::session::Session;
use crate::error::RunnerError;
use crate::ids::ConnName;
use crate::scenario::matcher::Assertion;
use crate::scenario::model::{FORMAT, FixtureKind, Replay, Serve};

mod wire;

use wire::Wire;

fn assertion(check: Check) -> Assertion {
    Assertion {
        check: format!("replay/{}", check.check),
        expected: check.expected,
        actual: check.actual,
        passed: check.passed,
    }
}

fn replay_failure(error: &ovis_terminal_replay::ReplayError) -> RunnerError {
    RunnerError::ScenarioInconsistent {
        scenario: format!("{}: {error}", error.kind()),
        detail: error.hint(),
    }
}

/// What a scenario identity is in the checkout the runner was built from.
fn resolve(scenarios: &Path, id: &str) -> ScenarioState {
    let path = scenarios.join(format!("{id}.json"));
    let Ok(bytes) = std::fs::read(&path) else {
        return ScenarioState::Absent;
    };
    let format = serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|value| value.get("format").cloned());
    if format == Some(json!(FORMAT)) {
        ScenarioState::Executable
    } else {
        ScenarioState::Declared
    }
}

impl Session<'_> {
    fn fixture_dir(&self) -> PathBuf {
        self.ctx
            .fixtures
            .clone()
            .unwrap_or_else(|| self.ctx.repo.conformance("fixtures/ovis"))
    }

    pub(crate) async fn replay(&mut self, replay: &Replay) -> Result<(), RunnerError> {
        let set = FixtureSet::load(&self.fixture_dir()).map_err(|e| replay_failure(&e))?;
        let repo = self.ctx.repo.root().to_path_buf();
        let scenarios = self.ctx.repo.scenarios_dir().to_path_buf();
        let audited = set
            .audit(&repo, &|id| resolve(&scenarios, id))
            .map_err(|e| replay_failure(&e))?;
        self.assert(audited.into_iter().map(assertion).collect())?;
        let inject = self.ctx.inject.clone();
        let selected: Vec<&Case> = set
            .cases
            .iter()
            .filter(|case| case.kind == replay.kind)
            .filter(|case| inject.as_ref().is_none_or(|request| case.plans(request)))
            .collect();
        if inject.is_some() {
            self.injected = !selected.is_empty();
        }
        self.assert(vec![Assertion::equal(
            format!("replay/{}/cases", kind_name(replay.kind)),
            json!(true),
            json!(!selected.is_empty()),
        )])?;
        for case in selected {
            self.replay_case(&set, (case, inject.as_ref())).await?;
        }
        Ok(())
    }

    /// Half-closes `name` and decodes every record until the peer ends the stream, so the
    /// byte accounting of `finish_conn` never races a notification still in flight.
    async fn drain_conn(&mut self, name: &ConnName) -> Result<(), RunnerError> {
        let conn = self.conn(name)?;
        conn.hangup().await?;
        let wait = Wait::within(RECORD_BOUND_MS, "a replay connection to end");
        while conn.next(wait).await?.is_some() {}
        Ok(())
    }

    /// One case: a fresh root and server, its connections, then teardown of both.
    async fn replay_case(
        &mut self,
        set: &FixtureSet,
        (case, inject): (&Case, Option<&Value>),
    ) -> Result<(), RunnerError> {
        let kind = match case.server {
            Server::Assembled => FixtureKind::Assembled,
            Server::Protocol => FixtureKind::Protocol,
            _ => {
                return Err(RunnerError::ScenarioInconsistent {
                    scenario: case.id.as_str().to_owned(),
                    detail: "a replay case names a server this runner cannot start",
                });
            }
        };
        let bindings = set
            .manifest
            .bindings((case.server, self.ctx.repo.root()))
            .map_err(|e| replay_failure(&e))?;
        let previous = std::mem::replace(&mut self.root, IsolatedRoot::create()?);
        let ran = async {
            let serve = Serve {
                fixture: kind,
                qa_graphics_sequence_max_bytes: None,
            };
            self.serve(&serve).await?;
            let mut wire = Wire::new(self, case.id.as_str());
            let options = Options::new(&set.legacy, Bindings::new(bindings), inject);
            let outcome = ovis_terminal_replay::replay(case, &mut wire, options).await;
            let opened = wire.opened();
            for conn in opened {
                self.drain_conn(&conn).await?;
                self.finish_conn(&conn).await?;
            }
            outcome.map_err(|e| replay_failure(&e))
        }
        .await;
        let stopped = if self.fixture.is_some() {
            self.stop_fixture().await.map(drop)
        } else {
            Ok(())
        };
        drop(std::mem::replace(&mut self.root, previous));
        let run = ran?;
        stopped?;
        let mut checks: Vec<Assertion> = run.checks.into_iter().map(assertion).collect();
        checks.push(Assertion::equal(
            format!("replay/{}/sent", case.id.as_str()),
            json!(true),
            json!(!run.sent.is_empty()),
        ));
        if inject.is_none() {
            checks.push(Assertion::equal(
                format!("replay/{}/receipt", case.id.as_str()),
                json!(set.receipts.get(&case.id)),
                json!(run.receipt),
            ));
        }
        self.assert(checks)
    }
}

const fn kind_name(kind: CaseKind) -> &'static str {
    match kind {
        CaseKind::Happy => "happy",
        CaseKind::Failure => "failure",
        _ => "other",
    }
}
