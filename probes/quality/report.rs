//! Findings and the JSON verdict every gate prints.

use serde_json::{Map, Value, json};

/// One defect a gate found, at a source location or ledger row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Finding {
    pub(super) rule: &'static str,
    pub(super) location: String,
    pub(super) detail: String,
}

impl Finding {
    pub(super) fn new(
        rule: &'static str,
        location: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            rule,
            location: location.into(),
            detail: detail.into(),
        }
    }

    fn to_json(&self) -> Value {
        json!({"rule": self.rule, "location": self.location, "detail": self.detail})
    }
}

/// The failure kind a gate reports when it found anything, with its hint.
#[derive(Debug, Clone, Copy)]
pub(super) struct FailureKind {
    pub(super) kind: &'static str,
    pub(super) hint: &'static str,
}

/// A gate's complete result: its findings plus the facts it measured.
#[derive(Debug)]
pub(super) struct Verdict {
    pub(super) gate: &'static str,
    pub(super) failure: FailureKind,
    pub(super) findings: Vec<Finding>,
    pub(super) facts: Map<String, Value>,
}

impl Verdict {
    pub(super) fn new(gate: &'static str, failure: FailureKind) -> Self {
        Self {
            gate,
            failure,
            findings: Vec::new(),
            facts: Map::new(),
        }
    }

    pub(super) fn fact(&mut self, key: &str, value: Value) {
        self.facts.insert(key.to_owned(), value);
    }

    pub(super) const fn passed(&self) -> bool {
        self.findings.is_empty()
    }

    /// Exit status: 0 with no finding, 1 otherwise.
    pub(super) const fn exit(&self) -> u8 {
        if self.passed() { 0 } else { 1 }
    }

    pub(super) fn to_json(&self) -> Value {
        let passed = self.passed();
        let mut out = Map::new();
        out.insert("gate".to_owned(), json!(self.gate));
        out.insert(
            "verdict".to_owned(),
            json!(if passed { "pass" } else { "fail" }),
        );
        out.insert("exit".to_owned(), json!(self.exit()));
        out.insert(
            "kind".to_owned(),
            (!passed).then_some(self.failure.kind).into(),
        );
        out.insert(
            "hint".to_owned(),
            (!passed).then_some(self.failure.hint).into(),
        );
        out.insert(
            "finding_count".to_owned(),
            json!(self.findings.len().to_string()),
        );
        let findings = self.findings.iter().map(Finding::to_json).collect();
        out.insert("findings".to_owned(), Value::Array(findings));
        for (key, value) in &self.facts {
            out.insert(key.clone(), value.clone());
        }
        Value::Object(out)
    }
}

/// A count as the canonical decimal string every report uses.
pub(super) fn count(value: usize) -> Value {
    Value::String(value.to_string())
}
