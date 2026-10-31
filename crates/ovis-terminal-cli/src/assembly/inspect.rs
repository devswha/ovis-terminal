//! `terminal.process_info`: native PID-plus-start-token evidence of the launched program
//! and its foreground group; fields the platform cannot report are listed as missing.

use ovis_terminal_authz::Session;
use ovis_terminal_core::terminals::{Field, ProcessSample};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::common::Target;
use serde_json::{Value, json};

use super::actor::Runtime;
use super::error::refusal;
use super::terminal::terminal_refusal;

fn sample(sample: &ProcessSample, (incarnation, started): (String, String)) -> Value {
    let argv = match &sample.argv {
        Field::Known(argv) => json!(argv),
        _ => json!([]),
    };
    let cwd = match &sample.cwd {
        Field::Known(cwd) => json!(cwd.to_str()),
        _ => Value::Null,
    };
    json!({
        "incarnation": incarnation,
        "pid": u32::try_from(sample.process.pid().get()).ok(),
        "start_token": sample.process.start_token().as_str(),
        "argv": argv, "cwd": cwd, "started_ms": started,
    })
}

impl Runtime {
    pub(super) fn process_info(
        &mut self,
        session: &Session,
        target: &Target,
    ) -> Result<Value, BoundaryError> {
        let id = self.resolve_id(target)?;
        let record = self
            .terminals
            .get(session, id)
            .map_err(|error| terminal_refusal(&error))?;
        let info = self
            .terminals
            .process_info(session, id)
            .map_err(|error| terminal_refusal(&error))?;
        let incarnation = record.lifecycle.last_process().map_or_else(
            || "0".to_owned(),
            |process| process.generation().get().to_string(),
        );
        let started = self
            .live
            .get(&id)
            .map_or_else(|| "0".to_owned(), |live| live.spawned_ms.get().to_string());
        let Field::Known(launcher) = &info.launcher else {
            return Err(refusal(
                "process_unavailable",
                "the launched program has no native record",
            ));
        };
        let foreground = match &info.foreground {
            Field::Known(group) => group
                .first()
                .map(|leader| sample(leader, (incarnation.clone(), started.clone()))),
            _ => None,
        };
        let supported: Vec<&str> = [
            ("argv", matches!(launcher.argv, Field::Known(_))),
            ("cwd", matches!(launcher.cwd, Field::Known(_))),
            ("foreground", matches!(info.foreground, Field::Known(_))),
        ]
        .into_iter()
        .filter_map(|(field, known)| known.then_some(field))
        .collect();
        Ok(json!({
            "terminal_id": id.to_string(),
            "launcher": sample(launcher, (incarnation, started)),
            "foreground": foreground,
            "supported_fields": supported,
        }))
    }
}
