//! The authority epoch of every principal this runtime lifetime issued to.

use std::collections::HashMap;

use crate::identity::PrincipalId;
use crate::rotation::PrincipalEpoch;

/// Epochs never go back: a principal's entry is kept for the whole runtime lifetime, so a
/// reclaimed credential cannot reset the epoch a client compares against.
#[derive(Debug, Default)]
pub(super) struct PrincipalEpochs(HashMap<PrincipalId, PrincipalEpoch>);

const FIRST: PrincipalEpoch = PrincipalEpoch::new(1);

impl PrincipalEpochs {
    /// Starts `principal` at epoch 1 unless it already holds one.
    pub(super) fn enroll(&mut self, principal: &PrincipalId) {
        self.0.entry(principal.clone()).or_insert(FIRST);
    }

    /// The current epoch of `principal`; 1 for a principal never issued to.
    pub(super) fn current(&self, principal: &PrincipalId) -> PrincipalEpoch {
        self.0.get(principal).copied().unwrap_or(FIRST)
    }

    /// Advances `principal` by one, saturating at the top, and returns the new epoch.
    pub(super) fn advance(&mut self, principal: &PrincipalId) -> PrincipalEpoch {
        let epoch = self.0.entry(principal.clone()).or_insert(FIRST);
        *epoch = PrincipalEpoch::new(epoch.get().saturating_add(1));
        *epoch
    }
}

#[cfg(test)]
mod tests {
    use super::{PrincipalEpoch, PrincipalEpochs};
    use crate::identity::PrincipalId;

    #[test]
    fn epochs_start_at_one_advance_per_principal_and_saturate() -> Result<(), crate::AuthzError> {
        let (writer, observer) = (
            PrincipalId::parse("writer")?,
            PrincipalId::parse("observer")?,
        );
        let mut epochs = PrincipalEpochs::default();
        epochs.enroll(&writer);

        let advanced = epochs.advance(&writer);
        epochs.enroll(&writer);
        epochs
            .0
            .insert(observer.clone(), PrincipalEpoch::new(u64::MAX));
        let top = epochs.advance(&observer);

        assert_eq!(advanced.get(), 2);
        assert_eq!(epochs.current(&writer).get(), 2);
        assert_eq!(top.get(), u64::MAX);
        Ok(())
    }
}
