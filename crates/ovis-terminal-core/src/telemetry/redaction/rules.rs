//! What counts as secret, and how it is replaced.

use std::fmt;

use serde_json::Value;
use sha2::{Digest as _, Sha256};

/// Replacement text of every redacted value.
pub const REDACTED: &str = "[REDACTED]";

/// Fields that carry credentials or secret input, at any depth of a record.
///
/// Credentials: bearer text, presented credentials, process report tokens. Secret input:
/// typed text, keys, pasted or raw bytes, prompt answers, standard input and environment
/// values, any of which may hold a password the user typed. Payloads: plugin action input,
/// template task text and document or file content, which diagnostics never need to copy.
const SECRET_FIELDS: &[&str] = &[
    "answer",
    "api_key",
    "authorization",
    "bearer",
    "bytes",
    "content",
    "credential",
    "data",
    "data_b64",
    "input",
    "keys",
    "password",
    "process_token",
    "secret",
    "stdin_b64",
    "task",
    "text",
    "token",
    "write",
];

/// Fields whose keys are diagnostic but whose values are secret.
const SECRET_VALUED_MAPS: &[&str] = &["env"];

const BEARER_PREFIX: &str = "ovt1.c";
const BEARER_HEX: usize = 64;

/// Redacts `record` in place: secret fields are replaced and every other string scrubbed.
pub fn redact(record: &mut Value) {
    *record = redacted(record);
}

/// A redacted copy of `record`; the payload of a secret field is never copied.
#[must_use]
pub fn redacted(record: &Value) -> Value {
    match record {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), redacted_field(key, value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redacted).collect()),
        Value::String(text) => Value::String(scrub(text)),
        Value::Null | Value::Bool(_) | Value::Number(_) => record.clone(),
    }
}

fn redacted_field(key: &str, value: &Value) -> Value {
    if SECRET_FIELDS.contains(&key) {
        Value::String(REDACTED.to_owned())
    } else if SECRET_VALUED_MAPS.contains(&key) {
        masked_values(value)
    } else {
        redacted(value)
    }
}

fn masked_values(value: &Value) -> Value {
    match value {
        Value::Object(entries) => Value::Object(
            entries
                .keys()
                .map(|key| (scrub(key), Value::String(REDACTED.to_owned())))
                .collect(),
        ),
        Value::Null => Value::Null,
        _ => Value::String(REDACTED.to_owned()),
    }
}

/// Replaces every runtime bearer (`ovt1.c<digits>.<64 lowercase hex>`) in `text`.
#[must_use]
pub fn scrub(text: &str) -> String {
    scrub_changed(text).unwrap_or_else(|| text.to_owned())
}

fn scrub_changed(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut changed = false;
    while let Some(start) = rest.find(BEARER_PREFIX) {
        let (before, candidate) = rest.split_at(start);
        out.push_str(before);
        let length = bearer_length(candidate);
        if length > 0 {
            out.push_str(REDACTED);
            changed = true;
            rest = candidate.get(length..).unwrap_or_default();
        } else {
            out.push_str(BEARER_PREFIX);
            rest = candidate.get(BEARER_PREFIX.len()..).unwrap_or_default();
        }
    }
    out.push_str(rest);
    changed.then_some(out)
}

/// The byte length of the bearer starting `candidate`, or zero when it is not one.
fn bearer_length(candidate: &str) -> usize {
    let body = candidate.get(BEARER_PREFIX.len()..).unwrap_or_default();
    let digits = body.bytes().take_while(u8::is_ascii_digit).count();
    let hex = body
        .get(digits..)
        .and_then(|tail| tail.strip_prefix('.'))
        .map_or(0, |secret| {
            secret
                .bytes()
                .take_while(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
                .count()
        });
    if digits == 0 || hex < BEARER_HEX {
        return 0;
    }
    BEARER_PREFIX
        .len()
        .saturating_add(digits)
        .saturating_add(1)
        .saturating_add(BEARER_HEX)
}

/// Replaces the string at `pointer` in `value` with `sha256:<hex digest>` and returns the
/// original, so a secret that must be shown once persists only as its digest.
pub fn seal(value: &mut Value, pointer: &str) -> Option<String> {
    let slot = value.pointer_mut(pointer)?;
    let Value::String(secret) = slot else {
        return None;
    };
    let digest = Sha256::digest(secret.as_bytes());
    let sealed = format!("sha256:{}", Hex(&digest));
    Some(std::mem::replace(secret, sealed))
}

struct Hex<'a>(&'a [u8]);

impl fmt::Display for Hex<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
    }
}
