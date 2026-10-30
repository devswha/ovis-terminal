//! Typed counters, identifiers and opaque payloads kept by the store.

use std::fmt;

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};

/// A position in the store's single commit sequence; event sequence numbers share it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Revision(u64);

impl Revision {
    /// The revision of a store that has never committed.
    pub const ZERO: Self = Self(0);

    /// Wraps a raw revision number.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// The raw revision number.
    pub const fn get(self) -> u64 {
        self.0
    }

    pub(crate) const fn checked_add(self, count: u64) -> Option<Self> {
        match self.0.checked_add(count) {
            Some(next) => Some(Self(next)),
            None => None,
        }
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The compare-and-swap version of one resource; an absent resource is at version 0.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(u64);

impl Version {
    /// The version of a resource that was never written.
    pub const ABSENT: Self = Self(0);

    /// Wraps a raw version number.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// The raw version number.
    pub const fn get(self) -> u64 {
        self.0
    }

    pub(crate) const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(next) => Some(Self(next)),
            None => None,
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The caller-chosen identity of one mutation; its receipt is stored under it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OpId(String);

impl OpId {
    /// Wraps an operation id.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The operation id text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OpId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The type name of a committed event, such as `doc.changed`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EventKind(String);

impl EventKind {
    /// Wraps an event type name.
    pub fn new(kind: impl Into<String>) -> Self {
        Self(kind.into())
    }

    /// The event type name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The address of one stored resource: its kind and its id within that kind.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResourceKey {
    kind: String,
    id: String,
}

impl ResourceKey {
    /// Addresses resource `id` of `kind`.
    pub fn new(kind: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            id: id.into(),
        }
    }

    /// The resource kind.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// The resource id within its kind.
    pub fn id(&self) -> &str {
        &self.id
    }
}

impl fmt::Display for ResourceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.kind, self.id)
    }
}

/// Opaque bytes stored for a resource value, receipt result or event body.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Payload(Vec<u8>);

impl Payload {
    /// The stored bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The number of stored bytes.
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the payload holds no bytes.
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<Vec<u8>> for Payload {
    fn from(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}

impl From<&str> for Payload {
    fn from(text: &str) -> Self {
        Self(text.as_bytes().to_vec())
    }
}

fn counter_to_sql(value: u64) -> rusqlite::Result<ToSqlOutput<'static>> {
    i64::try_from(value)
        .map(ToSqlOutput::from)
        .map_err(|overflow| rusqlite::Error::ToSqlConversionFailure(Box::new(overflow)))
}

fn counter_from_sql(value: ValueRef<'_>) -> FromSqlResult<u64> {
    let raw = i64::column_result(value)?;
    u64::try_from(raw).map_err(|_| FromSqlError::OutOfRange(raw))
}

impl ToSql for Revision {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        counter_to_sql(self.0)
    }
}

impl FromSql for Revision {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        counter_from_sql(value).map(Self)
    }
}

impl ToSql for Version {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        counter_to_sql(self.0)
    }
}

impl FromSql for Version {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        counter_from_sql(value).map(Self)
    }
}
