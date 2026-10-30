use std::fmt::{Display, Formatter};

use super::FrameError;

/// Absolute byte position in a wire connection.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WireOffset(u64);

impl WireOffset {
    /// The first byte position in a wire connection.
    pub const ZERO: Self = Self(0);

    /// Returns the numeric representation for a protocol boundary.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Advances by a byte count without wrapping.
    ///
    /// # Errors
    /// Returns [`FrameError::OffsetOverflow`] if the new position is not representable.
    pub fn checked_add(self, amount: usize) -> Result<Self, FrameError> {
        let amount_u64 = u64::try_from(amount).map_err(|_| FrameError::OffsetOverflow {
            offset: self,
            amount,
        })?;
        self.0
            .checked_add(amount_u64)
            .map(Self)
            .ok_or(FrameError::OffsetOverflow {
                offset: self,
                amount,
            })
    }
}

impl Display for WireOffset {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Absolute byte position within one logical result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LogicalOffset(u64);

impl LogicalOffset {
    /// Returns the numeric representation for a protocol boundary.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Constructs an offset from a host byte position with checked conversion.
    ///
    /// # Errors
    /// Returns [`FrameError::LogicalLengthOverflow`] when the host position is too large.
    pub fn from_usize(value: usize) -> Result<Self, FrameError> {
        u64::try_from(value)
            .map(Self)
            .map_err(|_| FrameError::LogicalLengthOverflow { length: value })
    }
}

/// Complete byte length of one logical result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LogicalTotal(u64);

impl LogicalTotal {
    /// Returns the numeric representation for a protocol boundary.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Constructs a total from a host byte length with checked conversion.
    ///
    /// # Errors
    /// Returns [`FrameError::LogicalLengthOverflow`] when the host length is too large.
    pub fn from_usize(value: usize) -> Result<Self, FrameError> {
        u64::try_from(value)
            .map(Self)
            .map_err(|_| FrameError::LogicalLengthOverflow { length: value })
    }
}

/// A continuation cursor in a logical result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Continuation(LogicalOffset);

impl Continuation {
    /// Creates a continuation from its logical-result offset.
    #[must_use]
    pub const fn new(offset: LogicalOffset) -> Self {
        Self(offset)
    }

    /// Returns the logical offset to request next.
    #[must_use]
    pub const fn offset(self) -> LogicalOffset {
        self.0
    }
}
