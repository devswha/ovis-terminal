//! Admitted entries, their bounded queue and the write-boundary recheck.

use std::collections::VecDeque;

use ovis_terminal_authz::{Authority, AuthzError};

use super::{SequencerError, View};
use crate::input::encoding::{Encoded, EncodingError, RawGrant};
use crate::input::receipts::{FenceReason, InputSeq};
use crate::leases::{Authorization, LeaseError};
use crate::registry::{ProcessGeneration, RegistryIdentities};

/// Bounds of the admitted input of one terminal that is not yet with the holder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct QueueLimits {
    /// Bytes that may wait.
    pub bytes: usize,
    /// Entries that may wait.
    pub entries: usize,
}

impl QueueLimits {
    /// The published per-terminal input bound: 16 MiB in at most 4096 entries.
    pub const DEFAULT: Self = Self::new(16 << 20, 4096);

    /// Explicit bounds.
    pub const fn new(bytes: usize, entries: usize) -> Self {
        Self { bytes, entries }
    }
}

/// One admitted entry and the authority it must still hold at the write.
#[derive(Debug)]
pub(super) struct Entry {
    pub(super) seq: InputSeq,
    pub(super) process: ProcessGeneration,
    pub(super) authorization: Authorization,
    pub(super) raw: Option<RawGrant>,
    pub(super) encoded: Encoded,
}

/// The state an entry is rechecked against at the write boundary.
pub(super) struct Check<'a> {
    pub(super) view: &'a View<'a>,
    pub(super) authority: &'a Authority<RegistryIdentities>,
    pub(super) process: ProcessGeneration,
}

const fn authz_reason(error: &AuthzError, otherwise: FenceReason) -> FenceReason {
    match error {
        AuthzError::CredentialRevoked { .. } => FenceReason::CredentialRevoked,
        AuthzError::CredentialExpired { .. } => FenceReason::CredentialExpired,
        _ => otherwise,
    }
}

const fn lease_reason(error: &LeaseError) -> FenceReason {
    match error {
        LeaseError::Authority(source) => authz_reason(source, FenceReason::Forbidden),
        LeaseError::NoLease => FenceReason::NoLease,
        _ => FenceReason::LeaseRevoked,
    }
}

const fn raw_reason(error: &EncodingError) -> FenceReason {
    match error {
        EncodingError::RawNotAuthorized(source) => {
            authz_reason(source, FenceReason::RawNotAuthorized)
        }
        _ => FenceReason::RawNotAuthorized,
    }
}

impl Entry {
    pub(super) fn len(&self) -> usize {
        self.encoded.bytes().len()
    }

    /// Epoch of the interact lease the entry was admitted under; 0 on an unleased terminal.
    pub(super) fn grant_epoch(&self) -> u64 {
        self.authorization
            .lease()
            .map_or(0, |lease| lease.epoch.get())
    }

    /// Process, credential, lease, raw grant and modes, in that order.
    pub(super) fn recheck(&self, check: &Check<'_>) -> Result<(), FenceReason> {
        if self.process != check.process {
            return Err(FenceReason::StaleTarget);
        }
        let at = check.view.moment.at;
        check
            .view
            .leases
            .recheck(&self.authorization, at)
            .map_err(|error| lease_reason(&error))?;
        if let Some(raw) = &self.raw {
            raw.recheck(check.authority, at)
                .map_err(|error| raw_reason(&error))?;
        }
        self.encoded
            .ensure_current(check.view.modes.revision())
            .map_err(|_| FenceReason::ModeMismatch)
    }
}

/// Admitted entries not yet with the holder, in admission order.
#[derive(Debug, Default)]
pub(super) struct Queue {
    entries: VecDeque<Entry>,
    bytes: usize,
}

impl Queue {
    pub(super) fn room(&self, requested: usize, limits: QueueLimits) -> Result<(), SequencerError> {
        let fits = self
            .bytes
            .checked_add(requested)
            .is_some_and(|total| total <= limits.bytes)
            && self.entries.len() < limits.entries;
        if fits {
            Ok(())
        } else {
            Err(SequencerError::Overflow {
                queued_bytes: self.bytes,
                queued_entries: self.entries.len(),
                requested,
                limits,
            })
        }
    }

    pub(super) fn push(&mut self, entry: Entry) {
        self.bytes = self.bytes.saturating_add(entry.len());
        self.entries.push_back(entry);
    }

    pub(super) fn pop(&mut self) -> Option<Entry> {
        let entry = self.entries.pop_front()?;
        self.bytes = self.bytes.saturating_sub(entry.len());
        Some(entry)
    }

    pub(super) fn drain(&mut self) -> VecDeque<Entry> {
        self.bytes = 0;
        std::mem::take(&mut self.entries)
    }

    /// Bytes of entries at or before `seq` still waiting here.
    pub(super) fn pending_through(&self, seq: InputSeq) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.seq <= seq)
            .map(Entry::len)
            .fold(0, usize::saturating_add)
    }
}
