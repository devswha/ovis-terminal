//! A sink layer that redacts every record before the destination sees it.

use std::io;

use serde_json::Value;

use super::rules::redact;
use crate::telemetry::TelemetrySink;

/// Wraps a destination so that only redacted JSON records reach it.
#[derive(Debug)]
pub struct RedactingSink<S> {
    inner: S,
}

impl<S: TelemetrySink> RedactingSink<S> {
    /// Redacts every record written to `inner`.
    pub const fn new(inner: S) -> Self {
        Self { inner }
    }

    /// The wrapped destination.
    pub const fn inner(&self) -> &S {
        &self.inner
    }
}

impl<S: TelemetrySink> TelemetrySink for RedactingSink<S> {
    /// Refuses a record that is not one JSON value rather than passing unredacted bytes on.
    fn write_record(&mut self, record: &[u8]) -> io::Result<()> {
        let mut value: Value = serde_json::from_slice(record)
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
        redact(&mut value);
        let redacted = serde_json::to_vec(&value)
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
        self.inner.write_record(&redacted)
    }
}
