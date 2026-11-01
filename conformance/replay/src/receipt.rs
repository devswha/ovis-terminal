//! Normalized receipts: what each step observed, free of run-specific identities, so a
//! Rust and a Bun replay of one case can be compared byte for byte.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The normalized receipt of one case: one line per executed step.
pub type Receipt = Vec<ReceiptLine>;

/// What a reply or event was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Outcome {
    /// A JSON-RPC result.
    Result,
    /// A JSON-RPC error.
    Error,
    /// A notification.
    Event,
}

/// One executed step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ReceiptLine {
    /// Step identity, or `event:<method>` for an awaited notification.
    pub step: String,
    /// Published method sent, or the notification method.
    pub method: String,
    /// Result, error or event.
    pub outcome: Outcome,
    /// `error.data.kind`, or the event's `params.type`.
    pub kind: Option<String>,
    /// `error.data.field_path`.
    pub field_path: Option<String>,
}

fn text(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

impl ReceiptLine {
    /// The line of a reply to `method` sent by `step`.
    #[must_use]
    pub fn reply(step: &str, method: &str, reply: &Value) -> Self {
        let failed = reply.get("error").is_some();
        Self {
            step: step.to_owned(),
            method: method.to_owned(),
            outcome: if failed {
                Outcome::Error
            } else {
                Outcome::Result
            },
            kind: text(reply, "/error/data/kind"),
            field_path: text(reply, "/error/data/field_path"),
        }
    }

    /// The line of an awaited `{"method", "params"}` notification.
    #[must_use]
    pub fn event(method: &str, record: &Value) -> Self {
        Self {
            step: format!("event:{method}"),
            method: method.to_owned(),
            outcome: Outcome::Event,
            kind: text(record, "/params/type"),
            field_path: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Outcome, ReceiptLine};

    #[test]
    fn a_receipt_keeps_kinds_and_paths_and_drops_identities() {
        let refused = json!({"jsonrpc": "2.0", "id": "inject-0", "error": {"code": -32602,
            "data": {"kind": "invalid_params", "field_path": "params.pane_id_typo"}}});
        let line = ReceiptLine::reply("typo", "layout.edit", &refused);
        assert_eq!(
            (
                line.outcome,
                line.kind.as_deref(),
                line.field_path.as_deref()
            ),
            (
                Outcome::Error,
                Some("invalid_params"),
                Some("params.pane_id_typo")
            )
        );
        let other = json!({"result": {"seq": "9"}});
        let again = json!({"result": {"seq": "10"}});
        assert_eq!(
            ReceiptLine::reply("s", "m", &other),
            ReceiptLine::reply("s", "m", &again)
        );
        let event = ReceiptLine::event(
            "event",
            &json!({"method": "event", "params": {"type": "doc.changed"}}),
        );
        assert_eq!(
            (event.outcome, event.kind.as_deref()),
            (Outcome::Event, Some("doc.changed"))
        );
    }
}
