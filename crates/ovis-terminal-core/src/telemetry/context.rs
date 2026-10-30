use std::fmt;

macro_rules! identity_type {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Debug, Eq, Hash, PartialEq)]
        pub struct $name(String);

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

identity_type!(
    RequestId,
    "A request identity used for telemetry correlation."
);
identity_type!(
    OperationId,
    "An operation identity used for telemetry correlation."
);
identity_type!(
    PrincipalId,
    "An authenticated principal identity used for telemetry correlation."
);
identity_type!(
    MachineId,
    "A machine identity used for telemetry correlation."
);
identity_type!(
    RuntimeId,
    "A runtime lifetime identity used for telemetry correlation."
);
identity_type!(
    ResourceId,
    "A resource identity used for telemetry correlation."
);

/// Correlation values for one handled request or operation.
#[non_exhaustive]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TraceContext {
    /// Request identifier, when the record belongs to an RPC.
    pub(super) request: Option<RequestId>,
    /// Operation identifier, when effects may have begun.
    pub(super) operation: Option<OperationId>,
    /// Authenticated principal identifier.
    pub(super) principal: Option<PrincipalId>,
    /// Machine identifier.
    pub(super) machine: Option<MachineId>,
    /// Runtime lifetime identifier.
    pub(super) runtime: Option<RuntimeId>,
    /// Resource identifier.
    pub(super) resource: Option<ResourceId>,
    /// Monotonic runtime sequence.
    pub(super) sequence: Option<u64>,
    /// Machine-readable outcome token.
    pub(super) outcome: Option<Outcome>,
}

impl TraceContext {
    /// Sets the correlated request identifier.
    #[must_use]
    pub fn with_request(mut self, request: RequestId) -> Self {
        self.request = Some(request);
        self
    }

    /// Sets the correlated operation identifier.
    #[must_use]
    pub fn with_operation(mut self, operation: OperationId) -> Self {
        self.operation = Some(operation);
        self
    }

    /// Sets the authenticated principal identifier.
    #[must_use]
    pub fn with_principal(mut self, principal: PrincipalId) -> Self {
        self.principal = Some(principal);
        self
    }

    /// Sets the machine identifier.
    #[must_use]
    pub fn with_machine(mut self, machine: MachineId) -> Self {
        self.machine = Some(machine);
        self
    }

    /// Sets the runtime lifetime identifier.
    #[must_use]
    pub fn with_runtime(mut self, runtime: RuntimeId) -> Self {
        self.runtime = Some(runtime);
        self
    }

    /// Sets the resource identifier.
    #[must_use]
    pub fn with_resource(mut self, resource: ResourceId) -> Self {
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
