//! Operating-system process identifiers of holder-owned children.

/// PID of a process started and still owned by the holder.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProcessId(u32);

impl ProcessId {
    /// Wraps a PID reported by the operating system.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// Returns the numeric PID carried on the holder wire.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}
