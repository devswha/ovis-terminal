//! `stream_data`: the bytes one attach stream delivered from an offset, as its client saw them.

use base64::Engine as _;
use serde_json::{Value, json};

use super::conn::Wait;
use super::exchange::RECORD_BOUND_MS;
use super::session::Session;
use crate::error::RunnerError;
use crate::ids::Decimal;
use crate::scenario::matcher::Assertion;
use crate::scenario::stream::StreamData;

/// Bytes of one stream from a start offset, in stream order.
struct Collected {
    from: u64,
    bytes: Vec<u8>,
    contiguous: bool,
}

impl Collected {
    /// Appends what `offset`..`offset + data.len()` adds past the collected end.
    fn take(&mut self, offset: u64, data: &[u8]) {
        let end = self
            .from
            .saturating_add(u64::try_from(self.bytes.len()).unwrap_or(u64::MAX));
        let reach = offset.saturating_add(u64::try_from(data.len()).unwrap_or(u64::MAX));
        if reach <= end {
            return;
        }
        if offset > end {
            self.contiguous = false;
        }
        let skip = usize::try_from(end.saturating_sub(offset)).unwrap_or(usize::MAX);
        self.bytes
            .extend_from_slice(data.get(skip..).unwrap_or_default());
    }
}

/// The stream id, offset and decoded bytes of an `attach.data` record of `stream`.
fn data_of(record: &Value, stream: &str) -> Result<Option<(u64, Vec<u8>)>, RunnerError> {
    let malformed = || RunnerError::Unexpected {
        waiting_for: "a well-formed attach.data record",
    };
    if record.get("method").and_then(Value::as_str) != Some("attach.data") {
        return Ok(None);
    }
    let params = record.get("params").ok_or_else(malformed)?;
    if params.get("stream_id").and_then(Value::as_str) != Some(stream) {
        return Ok(None);
    }
    let offset = params
        .get("offset")
        .and_then(Value::as_str)
        .ok_or_else(malformed)
        .and_then(Decimal::parse)?;
    let data = params
        .get("data_b64")
        .and_then(Value::as_str)
        .and_then(|text| base64::engine::general_purpose::STANDARD.decode(text).ok())
        .ok_or_else(malformed)?;
    Ok(Some((offset.0, data)))
}

impl Session<'_> {
    /// Reads `step.conn` until the stream's bytes from `step.from` contain `step.until`;
    /// other records are kept for later steps, earlier bytes of the stream are dropped.
    pub(crate) async fn stream_data(&mut self, step: &StreamData) -> Result<(), RunnerError> {
        let stream = self.labels.resolve(&json!(step.stream));
        let stream = stream.as_str().unwrap_or_default().to_owned();
        let from = self.labels.resolve(&json!(step.from));
        let from = Decimal::parse(from.as_str().unwrap_or_default())?.0;
        let until = step.until.as_bytes();
        let wait = Wait::within(RECORD_BOUND_MS, "stream data up to its marker");
        let mut collected = Collected {
            from,
            bytes: Vec::new(),
            contiguous: true,
        };
        let mut others = Vec::new();
        while !collected
            .bytes
            .windows(until.len())
            .any(|seen| seen == until)
        {
            let record = self
                .conn(&step.conn)?
                .next(wait)
                .await?
                .ok_or(RunnerError::Closed {
                    waiting_for: "stream data up to its marker",
                })?;
            match data_of(&record, &stream)? {
                Some((offset, data)) => collected.take(offset, &data),
                None => others.push(record),
            }
        }
        self.conn(&step.conn)?.unread(others);
        let encoded = base64::engine::general_purpose::STANDARD.encode(&collected.bytes);
        self.assert(vec![
            Assertion::equal(
                format!("stream[{}]/contiguous", step.id),
                Value::Bool(true),
                Value::Bool(collected.contiguous),
            ),
            Assertion {
                check: format!("stream[{}]/data_b64", step.id),
                expected: json!({ "$present": true }),
                actual: Value::String(encoded),
                passed: true,
            },
        ])
    }
}
