//! Reading a document at one snapshot and planning its compare-and-swap unit.

use ovis_terminal_protocol::domain::documents_retention::{DocValue, validate_name};
use ovis_terminal_protocol::schema::ids::DocName;
use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_protocol::schema::methods::doc::{
    DocEntry, DocRecord, DocSetParams, DocSetResult,
};
use ovis_terminal_protocol::schema::scalar::{ByteCount, EventSeq, RecordVersion};
use ovis_terminal_store::{CasWrite, Payload, ResourceKey, ResourceLookup, Snapshot, Version};
use serde_json::Value;

use super::{DocChanged, DocError};
use crate::events::ResourceId;
use crate::ops::Completion;

/// Largest compact JSON encoding of a document value, as advertised in `hello`.
pub const DOC_MAX_BYTES: ByteCount = Limits::v1().doc_max_bytes;

fn key(name: &DocName) -> ResourceKey {
    ResourceKey::new("document", name.as_str())
}

fn stored(snapshot: &Snapshot, name: &DocName) -> Result<(Value, Version), DocError> {
    validate_name(name)?;
    Ok(match snapshot.resource(&key(name))? {
        ResourceLookup::Live { version, value, .. } => (
            DocValue::decode(value.as_bytes().to_vec(), DOC_MAX_BYTES)?.into_value(),
            version,
        ),
        ResourceLookup::Tombstone { version, .. } => (Value::Null, version),
        ResourceLookup::Absent => (Value::Null, Version::ABSENT),
        _ => return Err(DocError::UnsupportedRecord),
    })
}

/// Reads `name` at the snapshot's revision; absence is a null value at version zero, and a
/// deleted document is a null value at the version its deletion committed.
///
/// # Errors
///
/// Returns `invalid_name`, `invalid_record` or a typed store failure.
pub fn read(snapshot: &Snapshot, name: &DocName) -> Result<DocRecord, DocError> {
    let (value, version) = stored(snapshot, name)?;
    Ok(DocRecord::new(
        name.clone(),
        value,
        RecordVersion::new(version.get()),
    ))
}

/// A validated `doc.set`: the name, the version the caller read and a bounded value.
#[derive(Debug, Clone)]
pub struct DocWrite {
    name: DocName,
    expected: RecordVersion,
    value: DocValue,
}

impl DocWrite {
    /// Validates the name and bounds the value by [`DOC_MAX_BYTES`].
    ///
    /// # Errors
    ///
    /// Returns `invalid_name` or `too_large`.
    pub fn new(params: DocSetParams) -> Result<Self, DocError> {
        validate_name(&params.name)?;
        Ok(Self {
            value: DocValue::bounded(params.value, DOC_MAX_BYTES)?,
            name: params.name,
            expected: params.version,
        })
    }

    /// Decides the compare-and-swap at `snapshot` and builds its unit: the stored value
    /// (a tombstone for null), the next version and one `doc.changed` event.
    ///
    /// The change event follows the operation event in the unit, so its sequence is the
    /// snapshot revision plus two.
    ///
    /// # Errors
    ///
    /// Returns `conflict` with the current version, or a counter or store failure.
    pub fn plan<D: From<DocChanged>>(self, snapshot: &Snapshot) -> Result<Completion<D>, DocError> {
        let (_, current) = stored(snapshot, &self.name)?;
        if current.get() != self.expected.get() {
            return Err(DocError::Conflict {
                name: self.name,
                expected: self.expected,
                current: RecordVersion::new(current.get()),
            });
        }
        let next = self
            .expected
            .get()
            .checked_add(1)
            .ok_or(DocError::Exhausted {
                counter: "document version",
            })?;
        let seq = snapshot
            .revision()
            .get()
            .checked_add(2)
            .ok_or(DocError::Exhausted {
                counter: "event sequence",
            })?;
        let version = RecordVersion::new(next);
        let result = serde_json::to_value(DocSetResult::new(version, EventSeq::new(seq)))?;
        let stored = (!self.value.deletes()).then(|| Payload::from(self.value.encoded().to_vec()));
        let changed = DocChanged::new(DocEntry::new(self.name.clone(), version));
        Ok(Completion::new(result)
            .write(CasWrite::new(key(&self.name), current, stored))
            .event(D::from(changed), vec![ResourceId::Document(self.name)]))
    }
}

/// The `doc.set` unit for `params` at `snapshot`; a rejected write becomes the operation's
/// typed failure and leaves the document unchanged.
pub fn set_unit<D: From<DocChanged>>(params: DocSetParams, snapshot: &Snapshot) -> Completion<D> {
    DocWrite::new(params)
        .and_then(|write| write.plan(snapshot))
        .unwrap_or_else(|error| Completion::failed(error.error_data()))
}
