//! The gates step: each planned quality tool runs in-process, exactly as its binary would,
//! and its exit status and JSON report become assertions and a process record.

use ovis_terminal_probes::quality::{GateRun, Tool, execute};
use serde_json::{Value, json};

use super::session::Session;
use crate::error::RunnerError;
use crate::evidence::{CapturedText, ProcessReport};
use crate::scenario::gates::{GateCheck, Gates};
use crate::scenario::matcher::{Assertion, Subject};

fn arguments(check: &GateCheck, repo: &str) -> Result<Vec<String>, RunnerError> {
    if let Some(args) = &check.args {
        return Ok(args.iter().map(|arg| arg.replace("$repo", repo)).collect());
    }
    let fixture = check
        .request
        .get("fixture")
        .and_then(Value::as_str)
        .ok_or_else(|| RunnerError::ScenarioInconsistent {
            scenario: check.tool.clone(),
            detail: "a gate without args names no fixture",
        })?;
    Ok(vec!["--fixture".to_owned(), fixture.to_owned()])
}

impl Session<'_> {
    fn selected<'g>(&mut self, gates: &'g Gates) -> Result<Vec<&'g GateCheck>, RunnerError> {
        let Some(requested) = &self.ctx.inject else {
            return Ok(gates.planned.iter().collect());
        };
        let found = gates
            .planned
            .iter()
            .find(|check| &check.request == requested);
        self.assert(vec![Assertion::equal(
            "inject/planned".to_owned(),
            requested.clone(),
            found.map_or(Value::Null, |check| check.request.clone()),
        )])?;
        self.injected = true;
        Ok(found.into_iter().collect())
    }

    pub(crate) async fn gates(&mut self, gates: &Gates) -> Result<(), RunnerError> {
        let repo = self.ctx.repo.root().to_string_lossy().into_owned();
        for (index, check) in self.selected(gates)?.into_iter().enumerate() {
            let tool =
                Tool::parse(&check.tool).ok_or_else(|| RunnerError::ScenarioInconsistent {
                    scenario: check.tool.clone(),
                    detail: "a gate names no quality tool",
                })?;
            let args = arguments(check, &repo)?;
            let run: GateRun = tokio::task::spawn_blocking(move || execute(tool, &args))
                .await
                .map_err(|source| RunnerError::Io {
                    operation: "run a quality gate",
                    source: std::io::Error::other(source),
                })?;
            let stdout = serde_json::to_vec_pretty(&run.report)?;
            self.processes.push(ProcessReport {
                role: "gate",
                argv: run.argv.clone(),
                pid: None,
                exit_code: Some(i32::from(run.exit)),
                stdout: CapturedText::of(&stdout),
                stderr: CapturedText::of(&[]),
            });
            let scope = format!("gate[{index}]");
            let mut checked = vec![Assertion::equal(
                format!("{scope}/exit"),
                json!(check.exit),
                json!(run.exit),
            )];
            checked.extend(check.report.check(
                Subject {
                    scope: &scope,
                    value: &run.report,
                },
                self.ctx.repo.root(),
            ));
            self.assert(checked)?;
        }
        Ok(())
    }
}
