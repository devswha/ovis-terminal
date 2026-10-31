//! OSC 7501 decoding provenance and ingestion counters of declared agent status (plan todo 33).
//!
//! An accepted OSC report keeps its raw payload and verbatim fields; decoded text is
//! present only where decoding succeeded, and each failure is a typed entry.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::schema::ids::AgentId;
use crate::schema::methods::agent::AgentResult;
use crate::schema::methods::status::{StatusResult, TurnRecord};
use crate::schema::records::{
    AgentBinding, AgentState, StatusEntry, StatusOrigin, StatusReport, StatusSnapshot,
};
use crate::schema::scalar::{
    EventSeq, FinishGeneration, Incarnation, OpenEnum, RecordCount, ReportSeq, Revision,
    TimestampMs, TurnNumber,
};

/// OSC 7501 fields carried as base64 text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum OscTextField {
    /// `title=<base64>`.
    Title,
    /// `msg=<base64>`.
    Msg,
}

/// Why a base64 text field did not decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DecodeFailure {
    /// The value is not standard padded base64.
    InvalidBase64,
    /// The decoded bytes are not UTF-8.
    InvalidUtf8,
}

/// One text field that could not be decoded; its decoded value is absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct OscDecodeError {
    /// The field.
    pub field: OpenEnum<OscTextField>,
    /// The failure.
    pub error: OpenEnum<DecodeFailure>,
}

impl OscDecodeError {
    /// Records that `field` failed with `error`.
    #[must_use]
    pub const fn new(field: OscTextField, error: DecodeFailure) -> Self {
        Self {
            field: OpenEnum::Known(field),
            error: OpenEnum::Known(error),
        }
    }
}

/// Fields of one OSC 7501 record exactly as the program sent them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct OscFields {
    /// `state=`.
    pub state: String,
    /// `id=`, naming a child record.
    pub id: Option<String>,
    /// `kind=`.
    pub kind: Option<String>,
    /// `progress=`.
    pub progress: Option<String>,
    /// `app=`.
    pub app: Option<String>,
    /// `title=`, still base64.
    pub title_b64: Option<String>,
    /// `msg=`, still base64.
    pub msg_b64: Option<String>,
}

/// Text decoded from the base64 fields; a field that failed stays absent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct OscDecoded {
    /// Decoded title.
    pub title: Option<String>,
    /// Decoded message.
    pub msg: Option<String>,
}

/// Raw payload, verbatim fields and decoding outcome of an accepted OSC 7501 report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct OscProvenance {
    /// The payload after `7501;`, byte for byte.
    pub raw: String,
    /// Fields as sent.
    pub fields: OscFields,
    /// Successfully decoded text.
    pub decoded: OscDecoded,
    /// Fields that did not decode.
    pub decode_errors: Vec<OscDecodeError>,
}

impl OscProvenance {
    /// Provenance of `raw` with its verbatim `fields`; nothing decoded yet.
    #[must_use]
    pub fn new(raw: String, fields: OscFields) -> Self {
        Self {
            raw,
            fields,
            decoded: OscDecoded::default(),
            decode_errors: Vec::new(),
        }
    }
}

/// OSC 7501 records a terminal's ingestion dropped without changing status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct IngestCounters {
    /// Payloads above the 4 KiB record bound, including unterminated ones cut at it.
    pub oversize: RecordCount,
    /// Payloads whose fields could not be read as a status record.
    pub malformed: RecordCount,
    /// Records refused because their program incarnation was no longer current, or
    /// because no producer sequence above the accepted one remained.
    pub stale: RecordCount,
}

impl IngestCounters {
    /// Builds counters from dropped-record counts.
    #[must_use]
    pub const fn new(oversize: u64, malformed: u64, stale: u64) -> Self {
        Self {
            oversize: RecordCount::new(oversize),
            malformed: RecordCount::new(malformed),
            stale: RecordCount::new(stale),
        }
    }
}

impl StatusReport {
    /// A report of `state` with no optional fields.
    #[must_use]
    pub const fn new(state: AgentState) -> Self {
        Self {
            state: OpenEnum::Known(state),
            id: None,
            kind: None,
            progress: None,
            app: None,
            title: None,
            msg: None,
        }
    }
}

impl StatusEntry {
    /// An accepted record of `report` from `origin`; provenance fields start empty.
    #[must_use]
    pub const fn new(report: StatusReport, origin: StatusOrigin) -> Self {
        Self {
            report,
            origin: OpenEnum::Known(origin),
            raw: None,
            decode_errors: Vec::new(),
            seq: EventSeq::new(0),
            since_ms: TimestampMs::new(0),
            report_seq: ReportSeq::new(0),
            turn: TurnNumber::new(0),
            osc: None,
        }
    }
}

impl StatusSnapshot {
    /// The never-reported status of one program incarnation: `reported: false, root: null`.
    #[must_use]
    pub const fn unreported(process_incarnation: Incarnation) -> Self {
        Self {
            reported: false,
            root: None,
            records: BTreeMap::new(),
            turn: TurnNumber::new(0),
            process_incarnation: Some(process_incarnation),
            report_seq: None,
            finish_generation: FinishGeneration::new(0),
            last_turn: None,
            agent_id: None,
            session: None,
        }
    }
}

impl TurnRecord {
    /// Turn `turn` with no recorded facts yet.
    #[must_use]
    pub const fn new(turn: TurnNumber) -> Self {
        Self {
            turn,
            outcome: None,
            started_seq: None,
            ended_seq: None,
            send_op: None,
            finish_generation: None,
            ended_on: None,
            process_incarnation: None,
            agent_id: None,
            session: None,
        }
    }
}

impl AgentBinding {
    /// A binding of `label` to one program incarnation, without session or history.
    #[must_use]
    pub const fn new(agent_id: AgentId, label: String, process_incarnation: Incarnation) -> Self {
        Self {
            agent_id,
            label,
            process_incarnation,
            session: None,
            history_id: None,
            revision: Revision::new(0),
        }
    }
}

impl StatusResult {
    /// A status read or report outcome.
    #[must_use]
    pub const fn new(status: StatusSnapshot, seq: EventSeq, ingest: IngestCounters) -> Self {
        Self {
            status,
            seq,
            ingest,
        }
    }
}

impl AgentResult {
    /// An agent read or registration outcome; `None` is a plain program.
    #[must_use]
    pub const fn new(agent: Option<AgentBinding>, revision: Revision) -> Self {
        Self { agent, revision }
    }
}

#[cfg(test)]
mod tests;
