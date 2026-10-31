//! The session binding lane over the fixture's agent terminals, and the provider store
//! the scenarios write transcripts into.
//!
//! `fixture.transcript` stands in for the agent program writing its own native store:
//! it appends, rewrites in place (a shorter text truncates) or replaces (a new file under
//! the same name) one file
//! in `<root>/transcripts`, the directory authorized as the store `fixture-store`.
//! The runtime under test observes those files; nothing here tells it what changed.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ovis_terminal_agent::conversation::binding::{BindingService, Stores};
use ovis_terminal_agent::status::StatusService;
use ovis_terminal_core::registry::RuntimeIdentity;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::rpc::RpcRequest;
use ovis_terminal_protocol::schema::decode_params;
use ovis_terminal_server::dispatch::lanes::session_binding::{
    Conversation, SessionBindingLane, UpdateSession,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::agents::Directory;
use super::handler::Dispatch;
use super::serve::at;
use crate::error::RunnerError;

/// Store name scenarios put in `session.source`.
const STORE: &str = "fixture-store";

/// The transcript directory under `root`.
pub(crate) fn store(root: &Path) -> PathBuf {
    root.join("transcripts")
}

/// Registers `agent.update_session` and `agent.conversation` over the agent terminals.
pub(crate) fn register(
    dispatch: &mut Dispatch,
    (status, directory): (Arc<StatusService>, Arc<Directory>),
    (runtime, root): (RuntimeIdentity, &Path),
) -> Result<(), RunnerError> {
    let dir = store(root);
    std::fs::create_dir(&dir).map_err(at("create the transcript store"))?;
    let stores = Stores::new(&[(STORE, dir)]).map_err(at("authorize the transcript store"))?;
    let service = Arc::new(BindingService::new(runtime.runtime_id(), status, stores));
    let lane = SessionBindingLane::new(directory, service);
    dispatch
        .register(UpdateSession::new(&lane))
        .map_err(at("register agent.update_session"))?;
    dispatch
        .register(Conversation::new(&lane))
        .map_err(at("register agent.conversation"))
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    /// Write a new file, or rewrite an existing one in place (same inode).
    Write,
    /// Append to the existing file.
    Append,
    /// Write a new file and rename it over the old name.
    Replace,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TranscriptParams {
    file: String,
    mode: Mode,
    /// Raw text; scenarios spell out every newline.
    text: String,
}

fn invalid(field: &str) -> BoundaryError {
    BoundaryError::invalid_params(Some(field))
}

/// Applies one provider write and reports the file's new length.
pub(crate) fn write(root: &Path, request: &RpcRequest) -> Result<Value, BoundaryError> {
    let params: TranscriptParams = decode_params(request).map_err(BoundaryError::from)?;
    if params.file.is_empty()
        || !params
            .file
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return Err(invalid("params.file"));
    }
    let path = store(root).join(&params.file);
    let failed = |_| invalid("params.file");
    match params.mode {
        Mode::Write => std::fs::write(&path, &params.text).map_err(failed)?,
        Mode::Append => std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .and_then(|mut file| file.write_all(params.text.as_bytes()))
            .map_err(failed)?,
        Mode::Replace => {
            let staged = path.with_extension("staged");
            std::fs::write(&staged, &params.text)
                .and_then(|()| std::fs::rename(&staged, &path))
                .map_err(failed)?;
        }
    }
    let len = std::fs::metadata(&path).map_err(failed)?.len();
    Ok(json!({ "file": params.file, "bytes": len.to_string() }))
}
