//! Typed per-request refusals of the served methods.

use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};

/// A typed per-request refusal with a stable kind and its recovery hint.
pub(super) fn refusal(kind: &str, hint: &'static str) -> BoundaryError {
    BoundaryError::domain(ErrorKind::from(kind.to_owned()), false).with_hint(hint)
}

/// An `internal` refusal; its cause goes to the server's stderr as one JSON line, never
/// to the client.
pub(super) fn internal<E: std::fmt::Debug>(stage: &'static str) -> impl FnOnce(E) -> BoundaryError {
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

/// `value` decoded as the method's closed result type before it leaves the server.
pub(super) fn checked<T: serde::de::DeserializeOwned + serde::Serialize>(
    value: serde_json::Value,
) -> Result<serde_json::Value, BoundaryError> {
    let typed: T = serde_json::from_value(value).map_err(internal("encode a read result"))?;
    serde_json::to_value(typed).map_err(internal("encode a read result"))
}
