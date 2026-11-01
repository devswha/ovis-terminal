use ovis_terminal_protocol::rpc::parse_request;
use serde_json::{Value, json};

use super::MANUAL;

const SHIPPED_MANUAL: &str = include_str!("../../../../docs/skill/ovis-terminal.md");
const ONBOARDING: &str = include_str!("../../../../docs/producer-onboarding.md");

/// The body lines of every fenced block whose info string is `info`.
fn fenced<'a>(document: &'a str, info: &str) -> Vec<Vec<&'a str>> {
    let mut blocks = Vec::new();
    let mut open: Option<Vec<&str>> = None;
    for line in document.lines() {
        match (&mut open, line.trim_start().strip_prefix("```")) {
            (None, Some(tag)) if tag.trim() == info => open = Some(Vec::new()),
            (Some(_), Some("")) => blocks.extend(open.take()),
            (Some(lines), _) => lines.push(line),
            (None, _) => {}
        }
    }
    blocks
}

#[test]
fn the_shipped_manual_is_the_embedded_manual() {
    assert_eq!(SHIPPED_MANUAL, MANUAL);
}

#[test]
fn every_params_example_decodes_as_its_method_params_type() {
    let mut checked = 0_usize;
    for block in fenced(ONBOARDING, "json") {
        let Some(method) = block
            .first()
            .and_then(|line| line.trim().strip_prefix("// params of "))
        else {
            continue;
        };
        let body = block.get(1..).unwrap_or_default().join("\n");
        let params: Value =
            serde_json::from_str(&body).unwrap_or_else(|error| panic!("{method}: {error}"));
        let verb = crate::agent::VERBS
            .into_iter()
            .find(|verb| verb.method == method)
            .unwrap_or_else(|| panic!("{method} is not a typed verb"));
        let envelope = json!({"jsonrpc": "2.0", "id": "x", "method": method, "params": params});
        let request = parse_request(&envelope.to_string()).expect("a request");
        assert!((verb.check)(&request).is_ok(), "{method}: {params}");
        checked += 1;
    }
    assert!(checked >= 4, "{checked} params examples");
}
