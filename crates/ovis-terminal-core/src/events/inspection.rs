//! Read-only inspection through the same digest-bound page path as subscriptions.

use ovis_terminal_protocol::schema::scalar::ByteOffset;
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
