//! `fixtures/wire/frames.json`: language-neutral frame bytes and what they decode to.
//! The same file is replayed over the socket by scenarios and by the TypeScript client.
//!
//! Every entry key is either asserted (`hex` with exactly one of `decodes` or `error`, checked
//! by decoding the bytes before they are sent) or a free-text `descriptors` entry that the
//! evidence records and nothing asserts. Any other key fails to load.

use ovis_terminal_protocol::wire::{Frame, FrameDecoder, FrameKind};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::RunnerError;
use crate::evidence::{Descriptors, hex};
use crate::scenario::load::Repo;

pub(crate) const FORMAT: &str = "ovis-wire-fixture/1";
const PATH: &str = "fixtures/wire/frames.json";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireFixtures {
    pub(crate) format: String,
    pub(crate) frames: Vec<WireEntry>,
}

/// One byte sequence and either the frames it decodes to or its typed decode failure.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireEntry {
    pub(crate) name: String,
    pub(crate) hex: String,
    #[serde(default)]
    pub(crate) decodes: Vec<DecodedFrame>,
    #[serde(default)]
    pub(crate) error: Option<String>,
    #[serde(default)]
    pub(crate) descriptors: Descriptors,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DecodedFrame {
    /// Channel as a canonical decimal string.
    pub(crate) channel: String,
    /// `data`, `credit` or `close`.
    pub(crate) kind: String,
    pub(crate) payload_hex: String,
}

/// Decodes lowercase hex without separators.
pub(crate) fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
        })
        .collect()
}

fn shown(frame: &Frame) -> Value {
    let kind = match frame.kind {
        FrameKind::Data => "data",
        FrameKind::Credit => "credit",
        FrameKind::Close => "close",
        _ => "unknown",
    };
    json!({"channel": frame.channel.value().to_string(), "kind": kind, "payload_hex": hex(&frame.payload)})
}

/// What the protocol's decoder makes of `bytes` fed in pieces ending at `cuts`:
/// `{"frames": [..]}`, or `{"error": kind}` for the first typed failure.
pub(crate) fn decode(bytes: &[u8], cuts: &[usize]) -> Value {
    let mut decoder = FrameDecoder::new();
    let mut frames = Vec::new();
    let mut start = 0;
    for end in cuts.iter().copied().chain([bytes.len()]) {
        let piece = bytes.get(start..end.max(start)).unwrap_or_default();
        match decoder.push(piece) {
            Ok(ready) => frames.extend(ready.iter().map(shown)),
            Err(error) => return json!({ "error": error.kind() }),
        }
        start = end.max(start);
    }
    match decoder.finish() {
        Ok(()) => json!({ "frames": frames }),
        Err(error) => json!({ "error": error.kind() }),
    }
}

impl WireEntry {
    pub(crate) fn bytes(&self) -> Result<Vec<u8>, RunnerError> {
        unhex(&self.hex)
            .ok_or_else(|| self.inconsistent("the wire fixture hex is not lowercase byte pairs"))
    }

    fn inconsistent(&self, detail: &'static str) -> RunnerError {
        RunnerError::ScenarioInconsistent {
            scenario: format!("{PATH}#{}", self.name),
            detail,
        }
    }

    /// The declared outcome in the shape [`decode`] returns; exactly one of `decodes` or `error`.
    pub(crate) fn declared(&self) -> Result<Value, RunnerError> {
        match (&self.error, self.decodes.is_empty()) {
            (Some(kind), true) => Ok(json!({ "error": kind })),
            (None, false) => Ok(json!({ "frames": self.decodes.iter().map(|frame| json!({
                "channel": frame.channel, "kind": frame.kind, "payload_hex": frame.payload_hex,
            })).collect::<Vec<_>>() })),
            (Some(_), false) | (None, true) => {
                Err(self.inconsistent("a wire fixture declares exactly one of decodes or error"))
            }
        }
    }
}

impl WireFixtures {
    pub(crate) fn named(&self, name: &str) -> Result<&WireEntry, RunnerError> {
        self.frames
            .iter()
            .find(|entry| entry.name == name)
            .ok_or_else(|| RunnerError::ScenarioMissing {
                scenario: format!("{PATH}#{name}"),
            })
    }
}

pub(crate) fn load(repo: &Repo) -> Result<WireFixtures, RunnerError> {
    let fixtures: WireFixtures =
        serde_path_to_error::deserialize(repo.json(PATH)?).map_err(|error| {
            RunnerError::ScenarioInvalid {
                path: repo.conformance(PATH),
                field: error.path().to_string(),
                source: error.into_inner(),
            }
        })?;
    if fixtures.format != FORMAT {
        return Err(RunnerError::ScenarioNotExecutable {
            scenario: PATH.to_owned(),
        });
    }
    Ok(fixtures)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use serde_json::{Value, json};

    use super::{WireEntry, decode, load};
    use crate::scenario::load::Repo;

    #[test]
    fn every_fixture_decodes_as_declared_at_every_single_split_and_bytewise() -> Result<(), String>
    {
        let fixtures = load(&Repo::of_build()).map_err(|error| error.to_string())?;
        assert!(fixtures.frames.len() >= 6);
        for entry in &fixtures.frames {
            let bytes = entry.bytes().map_err(|error| error.to_string())?;
            let want = entry.declared().map_err(|error| error.to_string())?;
            for cut in 0..=bytes.len() {
                assert_eq!(
                    decode(&bytes, &[cut]),
                    want,
                    "{} split at {cut}",
                    entry.name
                );
            }
            let bytewise: Vec<usize> = (1..bytes.len()).collect();
            assert_eq!(decode(&bytes, &bytewise), want, "{} bytewise", entry.name);
        }
        Ok(())
    }

    #[test]
    fn an_entry_with_both_or_neither_outcome_and_an_unknown_key_are_refused() {
        let entry = |value: Value| serde_json::from_value::<WireEntry>(value);
        let both = entry(json!({"name": "x", "hex": "", "error": "unknown_kind",
            "decodes": [{"channel": "0", "kind": "data", "payload_hex": ""}]}));
        assert!(both.is_ok_and(|entry| entry.declared().is_err()));
        let neither = entry(json!({"name": "x", "hex": ""}));
        assert!(neither.is_ok_and(|entry| entry.declared().is_err()));
        assert!(entry(json!({"name": "x", "hex": "", "error": "e", "summary": "prose"})).is_err());
    }

    proptest! {
        #[test]
        fn coalesced_valid_fixtures_decode_identically_under_any_segmentation(
            cuts in proptest::collection::vec(0_usize..2048, 0..32)
        ) {
            let fixtures = load(&Repo::of_build()).map_err(|error| TestCaseError::fail(error.to_string()))?;
            let valid: Vec<&WireEntry> = fixtures.frames.iter().filter(|entry| entry.error.is_none()).collect();
            let bytes: Vec<u8> = valid.iter().flat_map(|entry| entry.bytes().unwrap_or_default()).collect();
            let want: Vec<Value> = valid.iter()
                .filter_map(|entry| entry.declared().ok())
                .filter_map(|declared| declared.get("frames").and_then(Value::as_array).cloned())
                .flatten()
                .collect();
            let mut cuts: Vec<usize> = cuts.into_iter().map(|cut| cut.checked_rem(bytes.len().saturating_add(1)).unwrap_or(0)).collect();
            cuts.sort_unstable();
            prop_assert_eq!(decode(&bytes, &cuts), json!({ "frames": want }));
        }
    }
}
