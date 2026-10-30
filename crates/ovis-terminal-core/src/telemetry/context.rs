use std::fmt;

/// Correlation values for one handled request or operation.
#[non_exhaustive]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TraceContext {
    /// Request identifier, when the record belongs to an RPC.
    pub(super) request: Option<String>,
    /// Operation identifier, when effects may have begun.
    pub(super) operation: Option<String>,
    /// Authenticated principal identifier.
    pub(super) principal: Option<String>,
    /// Machine identifier.
    pub(super) machine: Option<String>,
    /// Runtime lifetime identifier.
    pub(super) runtime: Option<String>,
    /// Resource identifier.
    pub(super) resource: Option<String>,
    /// Monotonic runtime sequence.
    pub(super) sequence: Option<u64>,
    /// Machine-readable outcome token.
    pub(super) outcome: Option<Outcome>,
}

impl TraceContext {
    /// Sets the correlated request identifier.
    #[must_use]
    pub fn with_request(mut self, request: String) -> Self {
        self.request = Some(request);
        self
    }

    /// Sets the correlated operation identifier.
    #[must_use]
    pub fn with_operation(mut self, operation: String) -> Self {
        self.operation = Some(operation);
        self
    }

    /// Sets the authenticated principal identifier.
    #[must_use]
    pub fn with_principal(mut self, principal: String) -> Self {
        self.principal = Some(principal);
        self
    }

    /// Sets the machine identifier.
    #[must_use]
    pub fn with_machine(mut self, machine: String) -> Self {
        self.machine = Some(machine);
        self
    }

    /// Sets the runtime lifetime identifier.
    #[must_use]
    pub fn with_runtime(mut self, runtime: String) -> Self {
        self.runtime = Some(runtime);
        self
    }

    /// Sets the resource identifier.
    #[must_use]
    pub fn with_resource(mut self, resource: String) -> Self {
        self.resource = Some(resource);
        self
    }

    /// Sets the monotonic runtime sequence.
    #[must_use]
    pub const fn with_sequence(mut self, sequence: u64) -> Self {
        self.sequence = Some(sequence);
        self
    }

    /// Sets the machine-readable operation outcome.
    #[must_use]
    pub const fn with_outcome(mut self, outcome: Outcome) -> Self {
        self.outcome = Some(outcome);
        self
    }
}

/// Outcome classification suitable for machine queries.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    /// The operation completed successfully.
    Success,
    /// The operation was rejected or failed.
    Failure,
    /// The operation has not reached a terminal state.
    Pending,
}

impl Outcome {
    pub(super) const fn field(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Pending => "pending",
        }
    }
}

/// Values that can cross the telemetry serialization boundary.
#[non_exhaustive]
pub enum FieldValue<'a> {
    /// Non-secret structured text, escaped and bounded before serialization.
    Text(&'a str),
    /// Credential material, always replaced at serialization.
    Credential(&'a str),
    /// Secret terminal or agent input, always replaced at serialization.
    SecretInput(&'a str),
}

impl fmt::Debug for FieldValue<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(text) => formatter.debug_tuple("Text").field(text).finish(),
            Self::Credential(_) => formatter.write_str("Credential([REDACTED])"),
            Self::SecretInput(_) => formatter.write_str("SecretInput([REDACTED])"),
        }
    }
}
