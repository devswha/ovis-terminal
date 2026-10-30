use std::{fmt, io};

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
#[non_exhaustive]
#[derive(Debug)]
pub enum TelemetryError {
    /// The configured sink rejected a record.
    TelemetryIo(io::Error),
}

impl TelemetryError {
    /// Returns the machine-readable error kind.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::TelemetryIo(_) => "telemetry_io",
        }
    }
}

impl fmt::Display for TelemetryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TelemetryIo(_) => formatter.write_str("telemetry sink rejected a record"),
        }
    }
}

impl std::error::Error for TelemetryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::TelemetryIo(error) => Some(error),
        }
    }
}

/// Serializes a context and optional value, then sends one record to the sink.
///
/// # Errors
///
/// Returns [`TelemetryError::TelemetryIo`] when the injected sink rejects output.
pub fn emit(
    sink: &mut impl TelemetrySink,
    context: &TraceContext,
    value: Option<FieldValue<'_>>,
) -> Result<(), TelemetryError> {
    let record = serialize(context, value);
    sink.write_record(&record)
        .map_err(TelemetryError::TelemetryIo)
}

/// Logs an error at its handling boundary exactly once through the supplied sink.
///
/// Callers must propagate the returned error rather than retrying this emission.
///
/// # Errors
///
/// Returns [`TelemetryError::TelemetryIo`] when the injected sink rejects output.
pub fn log_handled_error(
    sink: &mut impl TelemetrySink,
    context: &TraceContext,
) -> Result<(), TelemetryError> {
    emit(sink, context, None)
}
