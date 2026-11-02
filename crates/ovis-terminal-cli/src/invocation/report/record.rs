//! The JSON record of a failure this binary reports itself: kind, hint, message and the
//! typed facts each kind carries.

use ovis_terminal_protocol::errors::TargetCandidate;
use serde_json::{Map, Value, json};

use super::super::error::CliError;

fn candidate(candidate: &TargetCandidate) -> Value {
    json!({
        "machine_id": candidate.machine_id.as_str(),
        "runtime_lifetime": candidate.runtime_lifetime.as_str(),
        "terminal_id": candidate.terminal_id.as_str(),
    })
}

pub(super) fn cli_record(error: &CliError) -> Map<String, Value> {
    let mut record = Map::new();
    record.insert("origin".to_owned(), json!("cli"));
    record.insert("kind".to_owned(), json!(error.kind()));
    record.insert("hint".to_owned(), json!(error.hint()));
    record.insert("message".to_owned(), json!(error.to_string()));
    match error {
        CliError::NeedsTarget { candidates, listed } => {
            let listing: Vec<Value> = candidates.iter().map(candidate).collect();
            record.insert("candidates".to_owned(), Value::Array(listing));
            record.insert("candidates_listed".to_owned(), json!(listed));
        }
        CliError::Params(error) => {
            record.insert("field_path".to_owned(), json!(error.field_path()));
        }
        CliError::NotRunning { state, .. } => {
            record.insert("state".to_owned(), json!(state));
        }
        CliError::CompatibilityBreak { breaks } => {
            let listed: Vec<Value> = breaks
                .iter()
                .map(|change| json!({ "path": change.path, "rule": change.rule.token() }))
                .collect();
            record.insert("breaks".to_owned(), Value::Array(listed));
        }
        CliError::GenerationMismatch { released, current } => {
            record.insert("released_generation".to_owned(), json!(released));
            record.insert("current_generation".to_owned(), json!(current));
        }
        #[cfg(unix)]
        CliError::Attach(error) => {
            for (field, value) in error.details() {
                record.insert(field.to_owned(), value);
            }
        }
        #[cfg(unix)]
        CliError::Assembly(_) => {}
        CliError::Usage { .. }
        | CliError::MissingCommand
        | CliError::SkillWithCommand
        | CliError::JsonSwitch { .. }
        | CliError::ParamsJson(_)
        | CliError::Command(_)
        | CliError::UnknownMethod { .. }
        | CliError::Endpoint(_)
        | CliError::Io { .. }
        | CliError::Closed
        | CliError::Restarted
        | CliError::Reply(_)
        | CliError::Result(_)
        | CliError::Timeout { .. }
        | CliError::UnsupportedTransport
        | CliError::Schema(_)
        | CliError::ReleasedFlag
        | CliError::BaselineUnreadable { .. }
        | CliError::BaselineJson { .. }
        | CliError::BaselineSchema { .. }
        | CliError::Encode(_) => {}
        #[cfg(windows)]
        CliError::PipeBusy { .. } => {}
        #[cfg(any(unix, windows))]
        CliError::Service(error) => service_detail(error, &mut record),
    }
    record
}

/// The typed facts a Windows service failure carries beyond its kind.
#[cfg(windows)]
fn service_detail(error: &crate::service::ServiceError, record: &mut Map<String, Value>) {
    use crate::service::ServiceError;
    match error {
        ServiceError::Stale { reason } => {
            record.insert("state".to_owned(), json!("stale_endpoint"));
            record.insert("reason".to_owned(), json!(reason));
        }
        ServiceError::AlreadyRunning {
            holder: Some(holder),
        } => {
            record.insert("holder".to_owned(), json!(holder));
        }
        _ => {}
    }
}

/// The typed facts a service failure carries beyond its kind.
#[cfg(unix)]
fn service_detail(
    error: &ovis_terminal_holder::service::unix::ServiceError,
    record: &mut Map<String, Value>,
) {
    use ovis_terminal_holder::service::unix::ServiceError;
    match error {
        ServiceError::Stale { reason } => {
            record.insert("state".to_owned(), json!("stale_endpoint"));
            record.insert("reason".to_owned(), json!(reason));
        }
        ServiceError::AlreadyRunning {
            holder: Some(holder),
        } => {
            record.insert("holder".to_owned(), json!(holder));
        }
        ServiceError::ManagerUnavailable { missing } => {
            record.insert("missing_prerequisite".to_owned(), json!(missing));
        }
        _ => {}
    }
}
