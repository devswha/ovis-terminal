//! Native platform probe (plan todo 12).
//!
//! `happy` records what this host's pseudo-terminals, processes and endpoints
//! actually do through the pinned portable-pty backend and the frozen holder
//! protocol; `failure` injects the failures a holder must report as typed
//! rejections. Every measured record is printed as one `PROBE <label> <json>`
//! line. Windows runs the smoke tier only.

#[cfg(test)]
#[cfg(unix)]
pub mod unix_failure;
#[cfg(test)]
#[cfg(unix)]
pub mod unix_happy;
#[cfg(test)]
#[cfg(windows)]
pub mod windows_smoke;

use ovis_terminal_holder::protocol::{
    Message, ProcessIncarnation, SpawnFailed, SpawnFailureKind, SpawnFailureReason, TerminalId,
    TerminalIncarnation, decode_message, encode_message,
};
use ovis_terminal_probes::platform::{ByteOrder, HostFacts, ProbeError};
use serde::Serialize;

/// Upper bound on any single wait in the probe.
#[cfg(unix)]
const BOUND: std::time::Duration = std::time::Duration::from_secs(20);

#[cfg(test)]
fn record(label: &str, value: &impl Serialize) {
    println!("PROBE {label} {}", serde_json::to_string(value).unwrap());
}

#[cfg(test)]
fn terminal(name: &str) -> (TerminalId, TerminalIncarnation, ProcessIncarnation) {
    (
        TerminalId::new(name.to_owned()),
        TerminalIncarnation::new(1),
        ProcessIncarnation::new(1),
    )
}

#[cfg(test)]
/// Sends `message` through the holder frame codec and returns what decodes.
fn through_holder_frame(message: &Message) -> Message {
    decode_message(&encode_message(message).unwrap()).unwrap()
}

#[cfg(test)]
/// A spawn failure must surface as the holder's typed `SpawnFailed`, never as
/// a process record.
fn assert_spawn_failure_is_typed(error: &ProbeError) {
    assert_eq!(error.kind(), "spawn_failed", "{error:?}");
    let (terminal_id, terminal_incarnation, _process) = terminal("probe-missing");
    let kind = error.spawn_failure_kind().unwrap();
    assert_eq!(kind, SpawnFailureKind::Unavailable);
    let message = Message::SpawnFailed(SpawnFailed {
        terminal_id,
        terminal_incarnation,
        kind,
        reason: SpawnFailureReason::ExecutableRejected,
        hint: error.hint().to_owned(),
        os_error: None,
    });
    assert_eq!(through_holder_frame(&message), message);
    record(
        "spawn_failed",
        &serde_json::json!({ "kind": error.kind(), "hint": error.hint(), "holder_kind": kind }),
    );
}

/// A directory under the system temp dir, removed when dropped.
#[cfg(test)]
#[cfg(unix)]
struct ScratchDir(std::path::PathBuf);

#[cfg(test)]
#[cfg(unix)]
impl ScratchDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!("ovis-probe-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

#[cfg(test)]
#[cfg(unix)]
impl Drop for ScratchDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Windows smoke tier: runs only in .github/workflows/platform-probes.yml with --include-ignored"
)]
fn happy() {
    let host = HostFacts::current();
    assert_eq!(host.byte_order, ByteOrder::compiled());
    assert_eq!(
        (host.os, host.arch),
        (std::env::consts::OS, std::env::consts::ARCH)
    );
    record("host", &host);
    #[cfg(unix)]
    unix_happy::run();
    #[cfg(windows)]
    windows_smoke::happy();
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Windows smoke tier: runs only in .github/workflows/platform-probes.yml with --include-ignored"
)]
fn failure() {
    #[cfg(unix)]
    unix_failure::run();
    #[cfg(windows)]
    windows_smoke::failure();
}
