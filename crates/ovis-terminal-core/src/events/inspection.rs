//! Read-only inspection through the same digest-bound page path as subscriptions.

use ovis_terminal_protocol::schema::scalar::ByteOffset;
use ovis_terminal_store::Snapshot;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{
    Coordinator, EventData, EventError, SnapshotAssembler, SnapshotProducer, StateSnapshot,
};

impl<D: EventData + Clone + Serialize + DeserializeOwned, P: SnapshotProducer> Coordinator<D, P>
where
    P::Sections: DeserializeOwned,
{
    /// Reads a committed revision without installing a delivery subscription.
    ///
    /// # Errors
    ///
    /// Returns the existing pin, page, digest, decoding or release failure.
    pub fn read_snapshot(&self) -> Result<StateSnapshot<P::Sections>, EventError> {
        let lease = self.pin_snapshot()?;
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

impl<D, P> Coordinator<D, P> {
    /// Runs `read` against a snapshot of the latest committed revision.
    ///
    /// The commit lock is held only while the snapshot opens, so a slow read never blocks
    /// commits; nothing is written and no event is published.
    ///
    /// # Errors
    ///
    /// Returns `coordinator_stopped` after poisoning, a store failure opening the snapshot,
    /// or the reader's own error.
    pub fn read_view<T, E: From<EventError>>(
        &self,
        read: impl FnOnce(&Snapshot) -> Result<T, E>,
    ) -> Result<T, E> {
        let view = self.lock()?.store.snapshot().map_err(EventError::from)?;
        read(&view)
    }
}
