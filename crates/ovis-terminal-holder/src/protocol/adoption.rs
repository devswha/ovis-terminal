//! Ownership-neutral generation negotiation and retained adoption records.

use std::collections::BTreeMap;

use thiserror::Error;

use super::{
    HOLDER_PROTOCOL_GENERATION, HeldTerminal, Hello, HolderBuildId, HolderInstanceId, TerminalId,
    TerminalTarget, Welcome,
};

/// Negotiation or adoption rejection.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum AdoptionError {
    /// The peer's holder generation is not supported.
    #[error("holder generation is unsupported")]
    UnsupportedGeneration {
        /// Generation the peer offered.
        offered: u16,
        /// Generation this holder supports.
        supported: u16,
    },
    /// A terminal ID was reused for a different incarnation or process token.
    #[error("holder target identity is stale")]
    StaleTarget {
        /// Stable terminal identity that conflicted.
        terminal_id: TerminalId,
    },
    /// A repeated report would move output, input, or exit evidence backwards.
    #[error("holder evidence is stale")]
    StaleEvidence {
        /// Stable terminal identity with regressed evidence.
        terminal_id: TerminalId,
    },
    /// The referenced terminal has no retained exit record.
    #[error("holder exit evidence is unavailable")]
    ExitNotRetained {
        /// Stable terminal identity whose exit record is unavailable.
        terminal_id: TerminalId,
    },
    /// Exit acknowledgement did not cover the final output offset.
    #[error("holder output has not drained through the process exit")]
    ExitNotDrained {
        /// Stable terminal identity whose output remains unacknowledged.
        terminal_id: TerminalId,
        /// Required inclusive final output boundary.
        final_offset: u64,
    },
    /// Holder process, incarnation, or final-offset evidence is inconsistent.
    #[error("holder evidence is inconsistent")]
    InvalidEvidence {
        /// Stable terminal identity carrying invalid evidence.
        terminal_id: TerminalId,
    },
}

impl AdoptionError {
    /// Returns the stable machine-readable error kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::UnsupportedGeneration { .. } => "unsupported_generation",
            Self::StaleTarget { .. } => "stale_target",
            Self::StaleEvidence { .. } => "stale_evidence",
            Self::ExitNotRetained { .. } => "exit_not_retained",
            Self::ExitNotDrained { .. } => "exit_not_drained",
            Self::InvalidEvidence { .. } => "invalid_evidence",
        }
    }

    /// Returns a static recovery hint suitable for a client or operator.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::UnsupportedGeneration { .. } => {
                "Upgrade or select a peer that supports the holder protocol generation."
            }
            Self::StaleTarget { .. } => "Re-resolve the terminal and process incarnations.",
            Self::StaleEvidence { .. } => "Re-adopt the latest holder evidence before retrying.",
            Self::ExitNotRetained { .. } => {
                "Re-adopt the holder and request its current terminal record."
            }
            Self::ExitNotDrained { .. } => {
                "Read output through the final offset before acknowledging exit."
            }
            Self::InvalidEvidence { .. } => {
                "Preserve the holder identity and sequence evidence from adoption."
            }
        }
    }
}

/// Holder-owned terminal records, including exits awaiting core acknowledgement.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AdoptionTable {
    records: BTreeMap<TerminalId, HeldTerminal>,
}

/// Holder executable and process identity returned during adoption.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct HolderDescriptor {
    /// Build identifier used to decide which generations accept new spawns.
    pub build: HolderBuildId,
    /// Random identity of this holder process lifetime.
    pub instance: HolderInstanceId,
}

impl HolderDescriptor {
    /// Creates the holder identity returned during adoption.
    #[must_use]
    pub const fn new(build: HolderBuildId, instance: HolderInstanceId) -> Self {
        Self { build, instance }
    }
}

impl AdoptionTable {
    /// Inserts a newly observed terminal record or rejects identity reuse.
    ///
    /// # Errors
    ///
    /// Returns [`AdoptionError::StaleTarget`] if the ID names a different
    /// terminal or process incarnation, [`AdoptionError::StaleEvidence`] if
    /// sequence evidence regresses, or [`AdoptionError::InvalidEvidence`] if
    /// the process identity or final output offset is inconsistent.
    pub fn retain(&mut self, record: HeldTerminal) -> Result<(), AdoptionError> {
        let has_process = record.process_incarnation.is_some();
        let has_pid = record.pid.is_some();
        let has_start_token = record.start_token.is_some();
        let process_evidence_matches = has_process == has_pid && has_pid == has_start_token;
        let exit_evidence_matches = record.exit.as_ref().is_none_or(|exit| {
            exit.target.terminal_id == record.terminal_id
                && exit.target.terminal_incarnation == record.terminal_incarnation
                && exit.target.process_incarnation == record.process_incarnation
                && exit.final_offset <= record.output_offset
        });
        if !process_evidence_matches || !exit_evidence_matches {
            return Err(AdoptionError::InvalidEvidence {
                terminal_id: record.terminal_id,
            });
        }
        let retained = if let Some(existing) = self.records.get(&record.terminal_id) {
            let same_identity = existing.terminal_incarnation == record.terminal_incarnation
                && existing.process_incarnation == record.process_incarnation
                && existing.start_token == record.start_token;
            if !same_identity {
                return Err(AdoptionError::StaleTarget {
                    terminal_id: record.terminal_id.clone(),
                });
            }
            if record.output_offset < existing.output_offset
                || record.input_sequence < existing.input_sequence
                || existing
                    .exit
                    .as_ref()
                    .is_some_and(|prior| record.exit.as_ref().is_some_and(|next| next != prior))
            {
                return Err(AdoptionError::StaleEvidence {
                    terminal_id: record.terminal_id,
                });
            }
            let mut next = record;
            if next.exit.is_none() {
                next.exit.clone_from(&existing.exit);
            }
            next
        } else {
            record
        };
        self.records.insert(retained.terminal_id.clone(), retained);
        Ok(())
    }

    /// Returns the retained adoption snapshot in stable terminal-ID order.
    #[must_use]
    pub fn held_terminals(&self) -> Vec<HeldTerminal> {
        self.records.values().cloned().collect()
    }

    /// Removes an exited terminal only after its final output was acknowledged.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the record is missing, still live, or the
    /// acknowledgement ends before the exit's final output offset.
    pub fn acknowledge_exit(
        &mut self,
        target: &TerminalTarget,
        acknowledged_offset: u64,
    ) -> Result<(), AdoptionError> {
        let record = self.records.get(&target.terminal_id).ok_or_else(|| {
            AdoptionError::ExitNotRetained {
                terminal_id: target.terminal_id.clone(),
            }
        })?;
        if record.terminal_incarnation != target.terminal_incarnation
            || record.process_incarnation != target.process_incarnation
        {
            return Err(AdoptionError::StaleTarget {
                terminal_id: target.terminal_id.clone(),
            });
        }
        let exit = record
            .exit
            .as_ref()
            .ok_or_else(|| AdoptionError::ExitNotRetained {
                terminal_id: target.terminal_id.clone(),
            })?;
        if acknowledged_offset < exit.final_offset {
            return Err(AdoptionError::ExitNotDrained {
                terminal_id: target.terminal_id.clone(),
                final_offset: exit.final_offset,
            });
        }
        self.records.remove(&target.terminal_id);
        Ok(())
    }
}

/// Negotiates a holder generation before an adopter can assume ownership.
///
/// # Errors
///
/// Returns [`AdoptionError::UnsupportedGeneration`] without returning a
/// welcome record or changing any holder-owned terminal state.
pub fn negotiate(
    hello: &Hello,
    holder: &HolderDescriptor,
    records: &AdoptionTable,
) -> Result<Welcome, AdoptionError> {
    if hello.generation != HOLDER_PROTOCOL_GENERATION {
        return Err(AdoptionError::UnsupportedGeneration {
            offered: hello.generation,
            supported: HOLDER_PROTOCOL_GENERATION,
        });
    }
    Ok(Welcome {
        generation: HOLDER_PROTOCOL_GENERATION,
        build: holder.build.clone(),
        instance: holder.instance.clone(),
        terminals: records.held_terminals(),
    })
}
