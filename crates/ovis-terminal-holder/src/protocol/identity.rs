//! Identities carried by every holder lifecycle and data message.

/// Build identity for a holder binary.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HolderBuildId(String);

impl HolderBuildId {
    /// Creates a holder build identity.
    #[must_use]
    pub const fn new(value: String) -> Self {
        Self(value)
    }

    /// Borrows the stable build identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Build identity for the adopting core binary.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CoreBuildId(String);

impl CoreBuildId {
    /// Creates a core build identity.
    #[must_use]
    pub const fn new(value: String) -> Self {
        Self(value)
    }

    /// Borrows the stable build identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Random identity for one holder process lifetime.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HolderInstanceId(String);

impl HolderInstanceId {
    /// Creates a holder process-lifetime identity.
    #[must_use]
    pub const fn new(value: String) -> Self {
        Self(value)
    }

    /// Borrows the opaque holder instance identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable terminal identity, never reused within a runtime lifetime.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TerminalId(String);

impl TerminalId {
    /// Creates a stable terminal identity.
    #[must_use]
    pub const fn new(value: String) -> Self {
        Self(value)
    }

    /// Borrows the opaque terminal identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Terminal incarnation, incremented whenever the terminal is recovered.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TerminalIncarnation(u64);

impl TerminalIncarnation {
    /// Creates an incarnation number.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the numeric incarnation.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Process incarnation, incremented for every process spawned in a terminal.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProcessIncarnation(u64);

impl ProcessIncarnation {
    /// Creates a process incarnation number.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the numeric process incarnation.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// OS-provided token distinguishing a process from a reused PID.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProcessStartToken(String);

impl ProcessStartToken {
    /// Creates an opaque operating-system process start token.
    #[must_use]
    pub const fn new(value: String) -> Self {
        Self(value)
    }

    /// Borrows the opaque process start token.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
