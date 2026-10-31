//! Read-only inspection through the same digest-bound page path as subscriptions.

use ovis_terminal_protocol::schema::scalar::ByteOffset;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::admission::SnapshotSlot;
use super::{
    Coordinator, EventData, EventError, SnapshotAssembler, SnapshotProducer, StateSnapshot,
};

impl<D: EventData + Clone + Serialize + DeserializeOwned, P: SnapshotProducer> Coordinator<D, P>
where
    P::Sections: DeserializeOwned,
{
    /// Reads a committed revision without installing a delivery subscription.
    ///
    /// Waits in FIFO order for a pinned-snapshot slot instead of failing at the budget;
    /// call from a blocking worker.
    ///
    /// # Errors
    ///
    /// Returns the existing pin, page, digest, decoding or release failure.
    pub fn read_snapshot(&self) -> Result<StateSnapshot<P::Sections>, EventError> {
        let slot = self.pins.enqueue().wait()?;
        self.read_snapshot_in(slot)
    }

    /// Reads a committed revision under a slot already granted in FIFO order.
    pub(crate) fn read_snapshot_in(
        &self,
        slot: SnapshotSlot,
    ) -> Result<StateSnapshot<P::Sections>, EventError> {
        let lease = self.pin_in(slot)?;
        let token = lease.token.clone();
        let mut assembler = SnapshotAssembler::new(lease);
        let result = (|| {
            let mut offset = ByteOffset::new(0);
            loop {
                let page = self.page(&token, offset)?;
                assembler.push(&page)?;
                match page.next_offset {
                    Some(next) => offset = next,
                    None => return assembler.finish(),
                }
            }
        })();
        // Release even when reconstruction fails; never turn a failure into an empty view.
        self.release_snapshot(&token)?;
        result
    }
}
