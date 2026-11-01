//! Discovery reads answer from this build's own document and agree with hello; the
//! machine and service routes answer typed outcomes over the socket.

use serde_json::{Value, json};

use super::support::{Served, TestResult, op_id};

fn call(served: &Served, method: &str, params: &Value) -> TestResult<Value> {
    served.ok(&["call", method, "--params", &params.to_string()])
}

/// The JSON error line of a refused call, whatever its exit status.
fn error(served: &Served, method: &str, params: &Value) -> TestResult<Value> {
    let output = served.cli(&["call", method, "--params", &params.to_string()])?;
    assert!(!output.status.success(), "{method} succeeded: {output:?}");
    let record: Value = serde_json::from_slice(&output.stderr)?;
    Ok(record.get("error").cloned().ok_or("error record")?)
}

fn names(list: Option<&Value>) -> Vec<Value> {
    list.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("name").cloned())
        .collect()
}

#[test]
fn discovery_reads_answer_from_the_committed_document_and_agree_with_hello() -> TestResult {
    let served = Served::start()?;
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../schema/ovis-terminal.openrpc.json"
    );
    let document: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    assert_eq!(call(&served, "rpc.discover", &json!({}))?, document);
    let errors = call(&served, "schema.errors", &json!({}))?;
    assert_eq!(errors.get("kinds"), document.get("x-error-kinds"));
    let events = call(&served, "schema.events", &json!({}))?;
    let published = names(document.get("x-events"));
    assert_ne!(published, Vec::<Value>::new());
    assert_eq!(names(events.get("events")), published);
    let capabilities = call(&served, "schema.capabilities", &json!({}))?;
    let hello = served.hello_reply(served.bearer())?;
    for field in ["capabilities", "limits", "generation"] {
        assert_eq!(
            capabilities.get(field),
            hello.pointer(&format!("/result/{field}")),
            "{field}: {capabilities} vs {hello}"
        );
    }
    let numeric = error(&served, "schema.errors", &json!({"extra": 1}))?;
    assert_eq!(
        numeric.get("kind"),
        Some(&json!("invalid_params")),
        "{numeric}"
    );
    served.stop()?;
    Ok(())
}

#[test]
fn service_describe_and_machine_add_answer_typed_outcomes_over_the_socket() -> TestResult {
    let served = Served::start()?;
    let described = error(&served, "service.describe", &json!({}))?;
    assert_eq!(
        described.get("kind"),
        Some(&json!("unavailable")),
        "{described}"
    );
    let added = error(
        &served,
        "machine.add",
        &json!({"op_id": op_id()?, "name": "gone", "transport": {"argv": ["/bin/false"]},
            "credential": "unused", "expected_machine_id": null}),
    )?;
    assert_ne!(
        added.get("kind"),
        Some(&json!("method_not_found")),
        "{added}"
    );
    let listed = error(
        &served,
        "machine.get",
        &json!({"machine": {"name": "gone"}}),
    )?;
    assert_eq!(listed.get("kind"), Some(&json!("not_found")), "{listed}");
    served.stop()?;
    Ok(())
}
