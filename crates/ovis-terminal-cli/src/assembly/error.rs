//! Typed per-request refusals of the served methods.

use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};

/// A typed per-request refusal with a stable kind and its recovery hint.
pub(super) fn refusal(kind: &str, hint: &'static str) -> BoundaryError {
    BoundaryError::domain(ErrorKind::from(kind.to_owned()), false).with_hint(hint)
}

/// An `internal` refusal; its cause goes to the server's stderr as one JSON line, never
/// to the client.
pub(super) fn internal(stage: &'static str) -> impl FnOnce(&dyn std::fmt::Debug) -> BoundaryError {
    move |cause| {
        let line = serde_json::json!({"warning": {"stage": stage, "detail": format!("{cause:?}")}});
        match std::io::Write::write_all(
            &mut std::io::stderr().lock(),
            format!("{line}\n").as_bytes(),
        ) {
            Ok(()) | Err(_) => {}
        }
        refusal("internal", "report the build with the server's stderr")
    }
}

/// The refusal for the runtime worker no longer answering.
pub(super) fn worker_gone() -> BoundaryError {
    refusal(
        "unavailable",
        "the server is stopping; retry against a running server",
    )
}
