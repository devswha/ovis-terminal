//! Parent deaths delivered to the machines holding children, until each acknowledges.
//!
//! The notices themselves stay in a durable [`Outbox`] until acknowledged. A machine's
//! notices are replayed oldest first and one at a time, each waiting for its answer, so a
//! later incarnation's death never overtakes an earlier one. The receiving side applies a
//! notice at most once per notification id and answers a replay as a duplicate, so an
//! acknowledgement lost with the line costs one more round trip and never ends or starts
//! remote work twice.

mod ack;
mod error;
mod inbox;
mod outbox;
mod replay;

pub use ack::Acknowledgements;
pub use error::{OrphanError, OutboxError};
pub use inbox::Inbox;
pub use outbox::Outbox;
pub use replay::{Replayed, replay};

#[cfg(test)]
mod qa;
