use std::io;

use thiserror::Error;

use super::serialize::serialize;
use super::{FieldValue, TraceContext};

/// A sink for already-redacted structured records.
pub trait TelemetrySink {
    /// Writes one JSON object and returns an I/O error instead of panicking.
    ///
    /// # Errors
    ///
    /// Returns the underlying error when the destination cannot accept bytes.
    fn write_record(&mut self, record: &[u8]) -> io::Result<()>;
}

/// Stable telemetry boundary failures.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TelemetryError {
    /// The configured sink rejected a record.
    #[error("telemetry sink rejected a record")]
    TelemetryIo(#[source] io::Error),
    /// Encoding the structured record failed.
    #[error("telemetry record serialization failed")]
    TelemetrySerialization(#[source] serde_json::Error),
}

impl TelemetryError {
    /// Returns the machine-readable error kind.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::TelemetryIo(_) => "telemetry_io",
            Self::TelemetrySerialization(_) => "telemetry_serialize",
        }
    }

    /// Returns stable recovery guidance for the error kind.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::TelemetryIo(_) => "check sink path permissions and retry",
            Self::TelemetrySerialization(_) => "inspect telemetry fields and retry",
        }
    }
}

/// Serializes a context and optional value, then sends one record to the sink.
///
/// # Errors
///
/// Returns [`TelemetryError::TelemetrySerialization`] when JSON encoding fails or
/// [`TelemetryError::TelemetryIo`] when the injected sink rejects output.
pub fn emit(
    sink: &mut impl TelemetrySink,
    context: &TraceContext,
    value: Option<FieldValue<'_>>,
) -> Result<(), TelemetryError> {
    let record = serialize(context, value)?;
    sink.write_record(&record)
        .map_err(TelemetryError::TelemetryIo)
}

/// Logs an error at its handling boundary exactly once through the supplied sink.
///
/// Callers must propagate the returned error rather than retrying this emission.
///
/// # Errors
///
/// Returns [`TelemetryError::TelemetrySerialization`] when JSON encoding fails or
/// [`TelemetryError::TelemetryIo`] when the injected sink rejects output.
pub fn log_handled_error(
    sink: &mut impl TelemetrySink,
    context: &TraceContext,
) -> Result<(), TelemetryError> {
    emit(sink, context, None)
}
