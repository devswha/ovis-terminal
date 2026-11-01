//! The commands step: planned runs of the binary, or only the one `--inject` names.
//!
//! Without `--inject`, or with a JSON request, every planned run executes. With a command
//! line, only the planned run whose `ovis-terminal <args>` equals it executes; a command
//! line the step does not plan fails the case.

use serde_json::Value;

use super::session::Session;
use crate::error::RunnerError;
use crate::scenario::matcher::Assertion;
use crate::scenario::model::{Commands, Run};

impl Session<'_> {
    pub(crate) async fn commands(&mut self, commands: &Commands) -> Result<(), RunnerError> {
        let selected: Vec<&Run> = match &self.ctx.inject {
            Some(Value::String(requested)) => {
                let found = commands
                    .planned
                    .iter()
                    .find(|run| &run.command_line() == requested);
                self.assert(vec![Assertion::equal(
                    "inject/planned".to_owned(),
                    Value::String(requested.clone()),
                    found.map_or(Value::Null, |run| Value::String(run.command_line())),
                )])?;
                self.injected = true;
                found.into_iter().collect()
            }
            _ => commands.planned.iter().collect(),
        };
        for run in selected {
            self.run_binary(run).await?;
        }
        Ok(())
    }
}
