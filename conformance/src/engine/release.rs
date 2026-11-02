//! The release step: planned release-stage and install-probe runs, in-process.
//!
//! Each runs exactly as its binary would, against the binary under test and a private directory of the isolated
//! root; its exit status and JSON report become assertions and a process record.

use serde_json::{Value, json};

use super::session::Session;
use crate::error::RunnerError;
use crate::evidence::{CapturedText, ProcessReport};
use crate::scenario::matcher::{Assertion, Subject};
use crate::scenario::release::{Release, ReleaseCheck, ReleaseTool};

impl Session<'_> {
    fn selected_release<'r>(
        &mut self,
        release: &'r Release,
    ) -> Result<Vec<&'r ReleaseCheck>, RunnerError> {
        let Some(requested) = &self.ctx.inject else {
            return Ok(release.planned.iter().collect());
        };
        let found = release
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

    pub(crate) async fn release(&mut self, release: &Release) -> Result<(), RunnerError> {
        let substitutions = [
            ("$binary", self.ctx.binary.to_string_lossy().into_owned()),
            ("$repo", self.ctx.repo.root().to_string_lossy().into_owned()),
            ("$root", self.root.path().to_string_lossy().into_owned()),
        ];
        for (index, check) in self.selected_release(release)?.into_iter().enumerate() {
            let args: Vec<String> = check
                .args
                .iter()
                .map(|arg| {
                    substitutions
                        .iter()
                        .fold(arg.clone(), |arg, (from, to)| arg.replace(from, to))
                })
                .collect();
            let tool = check.tool;
            let (exit, report, ran_argv) = tokio::task::spawn_blocking(move || match tool {
                ReleaseTool::ReleaseStage => {
                    let run = release_stage::execute(&args);
                    (run.exit, run.report, run.argv)
                }
                ReleaseTool::InstallProbe => {
                    let run = ovis_terminal_probes::install::execute(&args);
                    (run.exit, run.report, run.argv)
                }
            })
            .await
            .map_err(|source| RunnerError::Io {
                operation: "run a release stage",
                source: std::io::Error::other(source),
            })?;
            let stdout = serde_json::to_vec_pretty(&report)?;
            self.processes.push(ProcessReport {
                role: match tool {
                    ReleaseTool::ReleaseStage => "release-stage",
                    ReleaseTool::InstallProbe => "install-probe",
                },
                argv: ran_argv,
                pid: None,
                exit_code: Some(i32::from(exit)),
                stdout: CapturedText::of(&stdout),
                stderr: CapturedText::of(&[]),
            });
            let scope = format!("release[{index}]");
            let mut checked = vec![Assertion::equal(
                format!("{scope}/exit"),
                json!(check.exit),
                json!(exit),
            )];
            checked.extend(check.report.check(
                Subject {
                    scope: &scope,
                    value: &report,
                },
                self.ctx.repo.root(),
            ));
            self.assert(checked)?;
        }
        Ok(())
    }
}
