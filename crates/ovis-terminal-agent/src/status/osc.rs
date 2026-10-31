//! OSC 7501 records, as bounded by the VT engine, read into status reports.
//!
//! Format: `ESC ] 7501 ; state=<s>:id=<id>:kind=<k>:progress=<0-100>:app=<a>:title=<b64>:msg=<b64> ST`.
//! A base64 field that fails to decode keeps its verbatim text and a typed error; no
//! decoded text is invented. Anything else that is not a well-formed record is malformed.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ovis_terminal_protocol::domain::agent_status::{
    DecodeFailure, OscDecodeError, OscFields, OscProvenance, OscTextField,
};
use ovis_terminal_protocol::schema::records::StatusReport;
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use ovis_terminal_vt::{OscContent, OscRecord};

use super::machine::Lifecycle;

const STATUS_OSC: u32 = 7501;

/// What one OSC record means for status.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum OscOutcome {
    /// A well-formed status record.
    Report(Box<OscReport>),
    /// `7501;?`, a request for current states; it changes nothing.
    Query,
    /// A 7501 payload above the engine's 4 KiB record bound.
    Oversize,
    /// A 7501 payload that is not a status record.
    Malformed,
    /// Another OSC number.
    Foreign,
}

/// A parsed OSC 7501 status record with its provenance.
#[derive(Debug, Clone)]
pub struct OscReport {
    pub(super) lifecycle: Lifecycle,
    /// Reported fields, with decoded text only where decoding succeeded.
    pub report: StatusReport,
    /// Raw payload, verbatim fields and decoding outcome.
    pub provenance: OscProvenance,
}

impl OscReport {
    /// Names of the text fields that failed to decode.
    pub(super) fn failed_fields(&self) -> Vec<String> {
        self.provenance
            .decode_errors
            .iter()
            .filter_map(|error| match error.field {
                OpenEnum::Known(OscTextField::Title) => Some("title".to_owned()),
                OpenEnum::Known(OscTextField::Msg) => Some("msg".to_owned()),
                OpenEnum::Known(_) | OpenEnum::Unknown(_) => None,
            })
            .collect()
    }
}

/// Classifies one OSC record the VT engine located in a program's output.
#[must_use]
pub fn classify(record: &OscRecord) -> OscOutcome {
    match &record.content {
        OscContent::Fields {
            number: STATUS_OSC,
            fields,
        } => parse(fields),
        OscContent::Oversize {
            number: Some(STATUS_OSC),
            ..
        } => OscOutcome::Oversize,
        _ => OscOutcome::Foreign,
    }
}

fn parse(fields: &[Vec<u8>]) -> OscOutcome {
    let [field] = fields else {
        return OscOutcome::Malformed;
    };
    let Ok(raw) = std::str::from_utf8(field) else {
        return OscOutcome::Malformed;
    };
    if raw == "?" {
        return OscOutcome::Query;
    }
    fields_of(raw)
        .and_then(|fields| report(raw, fields))
        .map_or(OscOutcome::Malformed, |report| {
            OscOutcome::Report(Box::new(report))
        })
}

fn fields_of(raw: &str) -> Option<OscFields> {
    let mut fields = OscFields::default();
    let mut state = None;
    for pair in raw.split(':') {
        let (key, value) = pair.split_once('=')?;
        let slot = match key {
            "state" => &mut state,
            "id" => &mut fields.id,
            "kind" => &mut fields.kind,
            "progress" => &mut fields.progress,
            "app" => &mut fields.app,
            "title" => &mut fields.title_b64,
            "msg" => &mut fields.msg_b64,
            _ => return None,
        };
        if slot.replace(value.to_owned()).is_some() {
            return None;
        }
    }
    fields.state = state?;
    Some(fields)
}

fn decode(value: &str) -> Result<String, DecodeFailure> {
    let bytes = STANDARD
        .decode(value)
        .map_err(|_| DecodeFailure::InvalidBase64)?;
    String::from_utf8(bytes).map_err(|_| DecodeFailure::InvalidUtf8)
}

fn report(raw: &str, fields: OscFields) -> Option<OscReport> {
    let lifecycle = Lifecycle::parse(&fields.state)?;
    let progress = match fields.progress.as_deref() {
        Some(text) => Some(text.parse::<u8>().ok().filter(|value| *value <= 100)?),
        None => None,
    };
    if fields.id.as_deref().is_some_and(str::is_empty) {
        return None;
    }
    let mut report = StatusReport::new(lifecycle.state());
    report.id.clone_from(&fields.id);
    report.kind.clone_from(&fields.kind);
    report.app.clone_from(&fields.app);
    report.progress = progress;
    let texts = [
        (OscTextField::Title, fields.title_b64.clone()),
        (OscTextField::Msg, fields.msg_b64.clone()),
    ];
    let mut provenance = OscProvenance::new(raw.to_owned(), fields);
    for (field, value) in texts {
        let Some(value) = value else { continue };
        let slot = match field {
            OscTextField::Title => &mut provenance.decoded.title,
            OscTextField::Msg => &mut provenance.decoded.msg,
            _ => continue,
        };
        match decode(&value) {
            Ok(text) => *slot = Some(text),
            Err(error) => provenance
                .decode_errors
                .push(OscDecodeError::new(field, error)),
        }
    }
    report.title.clone_from(&provenance.decoded.title);
    report.msg.clone_from(&provenance.decoded.msg);
    Some(OscReport {
        lifecycle,
        report,
        provenance,
    })
}
