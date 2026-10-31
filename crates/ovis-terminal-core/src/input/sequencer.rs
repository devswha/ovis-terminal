//! Per-terminal ordered input admission and write fencing (plan todo 25).
//!
//! One [`Sequencer`] per terminal incarnation orders raw bytes, keys, paste, composer and
//! agent sends in a single bounded queue:
//!
//! - [`Sequencer::admit`] checks machine, runtime, pane, terminal and process identity, the
//!   runtime stream readiness, the interact lease (or the write grant on an unleased
//!   terminal), the raw-input grant and the encoding against the current modes, then answers
//!   with a `queued` receipt. It never waits for the PTY or the program.
//! - Text and its submission are one entry, so no other writer lands between them.
//! - [`Sequencer::next`] is the last write boundary: it rechecks process identity, the
//!   credential, the lease, the raw grant and the mode revision of the head entry and only
//!   then hands one entry to the holder, bound to the current process incarnation, lease
//!   epoch and fence epoch. One user entry is with the holder at a time.
//! - Revocation, takeover, process replacement, holder loss and exit fence unsent entries
//!   with a typed reason and zero bytes; an entry already with the holder is fenced by an
//!   empty holder write at a newer fence epoch, and whatever the holder already wrote is
//!   reported as written. Entries whose evidence ended become `outcome_unknown` and are
//!   never sent again.
//! - Terminal-query replies use their own bounded lane that a blocked user writer never
//!   holds back in the core, and never become receipts.
//! - Agent-declared readiness does not gate this queue; only the first message of a launch
//!   waits for it, and that gate belongs to the launch.

mod ack;
mod admit;
mod binding;
mod entry;
mod error;
mod lifecycle;
mod replies;
mod request;
mod state;
mod write;

#[cfg(unix)]
pub use ack::holder_refusal;
pub use binding::{Binding, Expected};
pub use entry::QueueLimits;
pub use error::SequencerError;
pub use request::{Moment, Payload, Request, Submission, View, Writer};
pub use state::{BarrierState, Sequencer};
pub use write::Step;
