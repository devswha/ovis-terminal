//! The `graphics_transcript` step (plan todo 83): one attach stream's DATA bytes and
//! control records, read until the program's closing marker, then checked byte for byte.
//!
//! Each expected sequence is named and given as base64 of its exact bytes; the
//! assertion records its SHA-256 and the SHA-256 of the transcript window it was found
//! in, so the evidence shows byte identity rather than a substring guess.

use std::collections::BTreeMap;

use base64::Engine as _;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::engine::conn::Wait;
use crate::engine::exchange::RECORD_BOUND_MS;
use crate::engine::session::Session;
use crate::error::RunnerError;
use crate::evidence::sha256_hex;
use crate::ids::ConnName;
use crate::scenario::matcher::{Assertion, Expect, Subject};

/// Read one stream until `until` arrives and check what it carried.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GraphicsTranscript {
    pub(crate) conn: ConnName,
    /// The `--inject` a failure case runs this step under; it must equal it.
    #[serde(default)]
    pub(crate) inject: Option<Value>,
    /// Text the program writes last.
    pub(crate) until: String,
    /// Name to base64 of bytes the transcript must contain byte-identical.
    #[serde(default)]
    pub(crate) contains_b64: BTreeMap<String, String>,
    /// Name to base64 of bytes that must never appear.
    #[serde(default)]
    pub(crate) absent_b64: BTreeMap<String, String>,
    /// Every control record kind received, in order.
    pub(crate) controls: Vec<String>,
    /// Checks on the params of the first `graphics_dropped` record.
    #[serde(default)]
    pub(crate) dropped: Expect,
    /// Names from `contains_b64` that must appear in this order (plan todo 84).
    #[serde(default)]
    pub(crate) in_order: Vec<String>,
    /// Checks on the params of the first `graphics_replayed` record.
    #[serde(default)]
    pub(crate) replayed: Expect,
}

/// What one stream delivered.
#[derive(Debug, Default)]
struct Transcript {
    bytes: Vec<u8>,
    controls: Vec<Value>,
    contiguous: bool,
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn decoded(name: &str, text: &str) -> Result<Vec<u8>, RunnerError> {
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .map_err(|_| RunnerError::ScenarioInconsistent {
            scenario: name.to_owned(),
            detail: "a graphics transcript names bytes that are not base64",
        })
}

impl Transcript {
    fn take(&mut self, record: &Value) -> Result<bool, RunnerError> {
        match record.get("method").and_then(Value::as_str) {
            Some("attach.data") => {
                let offset = record.pointer("/params/offset").and_then(Value::as_str);
                let at = self.bytes.len().to_string();
                self.contiguous &= offset == Some(at.as_str());
                let data = record
                    .pointer("/params/data_b64")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                self.bytes.extend(decoded("attach.data", data)?);
                Ok(true)
            }
            Some("attach.control") => {
                self.controls
                    .push(record.get("params").cloned().unwrap_or(Value::Null));
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn first(&self, kind: &str) -> Value {
        self.controls
            .iter()
            .find(|control| control.get("kind") == Some(&json!(kind)))
            .cloned()
            .unwrap_or(Value::Null)
    }

    fn kinds(&self) -> Value {
        self.controls
            .iter()
            .map(|control| control.get("kind").cloned().unwrap_or(Value::Null))
            .collect()
    }
}

/// Where each `in_order` sequence was found; the list must be strictly increasing.
fn order(step: &GraphicsTranscript, transcript: &Transcript) -> Result<Assertion, RunnerError> {
    let mut found = Vec::new();
    for name in &step.in_order {
        let text = step
            .contains_b64
            .get(name)
            .ok_or(RunnerError::ScenarioInconsistent {
                scenario: name.clone(),
                detail: "an in_order name is not in contains_b64",
            })?;
        found.push(find(&transcript.bytes, &decoded(name, text)?));
    }
    let ordered = found.windows(2).all(|pair| match pair {
        [Some(before), Some(after)] => before < after,
        _ => false,
    }) && found.iter().all(Option::is_some);
    Ok(Assertion::equal(
        "graphics/in_order".to_owned(),
        json!(true),
        json!(ordered),
    ))
}

impl Session<'_> {
    pub(crate) async fn graphics_transcript(
        &mut self,
        step: &GraphicsTranscript,
    ) -> Result<(), RunnerError> {
        let mut checked = Vec::new();
        if let Some(requested) = self.ctx.inject.clone() {
            checked.push(Assertion::equal(
                "graphics/inject".to_owned(),
                requested,
                step.inject.clone().unwrap_or(Value::Null),
            ));
            self.injected = true;
        }
        let transcript = self.read_transcript(step).await?;
        checked.push(Assertion::equal(
            "graphics/data_offsets_contiguous".to_owned(),
            json!(true),
            json!(transcript.contiguous),
        ));
        checked.push(Assertion::equal(
            "graphics/controls".to_owned(),
            json!(step.controls),
            transcript.kinds(),
        ));
        for (name, text) in &step.contains_b64 {
            let sequence = decoded(name, text)?;
            let window = find(&transcript.bytes, &sequence)
                .and_then(|at| transcript.bytes.get(at..at.saturating_add(sequence.len())));
            checked.push(Assertion::equal(
                format!("graphics/contains[{name}]"),
                json!({"bytes": sequence.len().to_string(), "sha256": sha256_hex(&sequence)}),
                window.map_or(
                    Value::Null,
                    |found| json!({"bytes": found.len().to_string(), "sha256": sha256_hex(found)}),
                ),
            ));
        }
        for (name, text) in &step.absent_b64 {
            let found = find(&transcript.bytes, &decoded(name, text)?).is_some();
            checked.push(Assertion::equal(
                format!("graphics/absent[{name}]"),
                json!(false),
                json!(found),
            ));
        }
        checked.push(order(step, &transcript)?);
        for (kind, scope, expect) in [
            ("graphics_dropped", "graphics/dropped", &step.dropped),
            ("graphics_replayed", "graphics/replayed", &step.replayed),
        ] {
            let record = transcript.first(kind);
            checked.extend(expect.check(
                Subject {
                    scope,
                    value: &record,
                },
                self.ctx.repo.root(),
            ));
        }
        self.assert(checked)
    }

    /// Reads stream records until `until` is in the bytes; other records stay for later
    /// steps, in order.
    async fn read_transcript(
        &mut self,
        step: &GraphicsTranscript,
    ) -> Result<Transcript, RunnerError> {
        let wait = Wait::within(RECORD_BOUND_MS, "a graphics transcript");
        let mut transcript = Transcript {
            contiguous: true,
            ..Transcript::default()
        };
        let mut other = Vec::new();
        while find(&transcript.bytes, step.until.as_bytes()).is_none() {
            let record = self
                .conn(&step.conn)?
                .next(wait)
                .await?
                .ok_or(RunnerError::Closed {
                    waiting_for: "a graphics transcript",
                })?;
            if !transcript.take(&record)? {
                other.push(record);
            }
        }
        let conn = self.conn(&step.conn)?;
        for record in other {
            conn.stash(record);
        }
        Ok(transcript)
    }
}
