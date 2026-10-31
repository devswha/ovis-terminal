//! `fixtures/wire/frames.json`: language-neutral frame bytes and what they decode to.
//! The same file is replayed over the socket by scenarios and by the TypeScript client.

use serde::Deserialize;

use crate::error::RunnerError;
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
    #[expect(
        dead_code,
        reason = "human-readable fixture documentation, required by the closed schema"
    )]
    pub(crate) summary: String,
    pub(crate) hex: String,
    #[serde(default)]
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "checked by the decode tests and the TypeScript client"
        )
    )]
    pub(crate) decodes: Vec<DecodedFrame>,
    #[serde(default)]
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "checked by the decode tests and the TypeScript client"
        )
    )]
    pub(crate) error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "checked by the decode tests and the TypeScript client"
    )
)]
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

impl WireEntry {
    pub(crate) fn bytes(&self) -> Result<Vec<u8>, RunnerError> {
        unhex(&self.hex).ok_or_else(|| RunnerError::ScenarioInconsistent {
            scenario: self.name.clone(),
            detail: "the wire fixture hex is not lowercase byte pairs",
        })
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
    let fixtures: WireFixtures = serde_json::from_value(repo.json(PATH)?).map_err(|source| {
        RunnerError::ScenarioInvalid {
            path: repo.conformance(PATH),
            field: ".".to_owned(),
            source,
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
    use ovis_terminal_protocol::wire::{Frame, FrameDecoder, FrameError, FrameKind};
    use proptest::prelude::*;

    use super::{WireEntry, load, unhex};
    use crate::scenario::load::Repo;

    fn shown(frame: &Frame) -> (String, String, Vec<u8>) {
        let kind = match frame.kind {
            FrameKind::Data => "data",
            FrameKind::Credit => "credit",
            FrameKind::Close => "close",
            _ => "unknown",
        };
        (
            frame.channel.value().to_string(),
            kind.to_owned(),
            frame.payload.clone(),
        )
    }

    fn expected(entry: &WireEntry) -> Vec<(String, String, Vec<u8>)> {
        entry
            .decodes
            .iter()
            .map(|frame| {
                (
                    frame.channel.clone(),
                    frame.kind.clone(),
                    unhex(&frame.payload_hex).unwrap_or_default(),
                )
            })
            .collect()
    }

    /// Decodes `bytes` fed in `cuts` pieces; an error ends decoding with its kind.
    fn decode(bytes: &[u8], cuts: &[usize]) -> Result<Vec<(String, String, Vec<u8>)>, String> {
        let mut decoder = FrameDecoder::new();
        let mut frames = Vec::new();
        let mut start = 0;
        for end in cuts.iter().copied().chain([bytes.len()]) {
            let piece = bytes.get(start..end.max(start)).unwrap_or_default();
            frames.extend(
                decoder
                    .push(piece)
                    .map_err(|error: FrameError| error.kind().to_owned())?,
            );
            start = end.max(start);
        }
        decoder.finish().map_err(|error| error.kind().to_owned())?;
        Ok(frames.iter().map(shown).collect())
    }

    #[test]
    fn every_fixture_decodes_identically_at_every_single_split_and_bytewise() -> Result<(), String>
    {
        let fixtures = load(&Repo::of_build()).map_err(|error| error.to_string())?;
        assert!(fixtures.frames.len() >= 6);
        for entry in &fixtures.frames {
            let bytes = entry.bytes().unwrap_or_default();
            let want = entry.error.clone().map_or_else(|| Ok(expected(entry)), Err);
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

    proptest! {
        #[test]
        fn coalesced_valid_fixtures_decode_identically_under_any_segmentation(
            cuts in proptest::collection::vec(0_usize..2048, 0..32)
        ) {
            let fixtures = load(&Repo::of_build()).map_err(|error| TestCaseError::fail(error.to_string()))?;
            let valid: Vec<&WireEntry> = fixtures.frames.iter().filter(|entry| entry.error.is_none()).collect();
            let bytes: Vec<u8> = valid.iter().flat_map(|entry| entry.bytes().unwrap_or_default()).collect();
            let want: Vec<_> = valid.iter().flat_map(|entry| expected(entry)).collect();
            let mut cuts: Vec<usize> = cuts.into_iter().map(|cut| cut.checked_rem(bytes.len().saturating_add(1)).unwrap_or(0)).collect();
            cuts.sort_unstable();
            prop_assert_eq!(decode(&bytes, &cuts), Ok(want));
        }
    }
}
