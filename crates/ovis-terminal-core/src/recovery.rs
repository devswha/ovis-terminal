//! Crash recovery and honest reboot metadata (plan todo 49).
//!
//! After an unplanned boot the core proves, per recorded program, what survived: the
//! operating system's own boot identity is compared with the recorded one
//! ([`BootStamp`]), each holder group (the shared holder of a session and build, or the
//! internal dedicated test placement, see [`Placement`]) is adopted over its own socket,
//! and only a program the holder still owns under the same PID and start token, which the
//! operating system confirms, is reported survived. A reboot yields restored metadata;
//! inconclusive evidence yields `unknown`, and nothing here ever relaunches a program.
//!
//! The core alone parses output: a survivor's screen is rebuilt from its durable,
//! offset-fenced checkpoint ([`CheckpointStore`]) plus the holder's retained bytes, without
//! repeating answered terminal queries or committed OSC records ([`restore`]), and any
//! output nobody retains is reported as a gap. Every finding is appended to a durable
//! journal ([`Incidents`]) that reads never clear; damaged checkpoints and database
//! material are kept in quarantine, never replaced by healthy-looking defaults.

#[cfg(unix)]
mod adopt;
mod boot;
mod checkpoint;
mod durable;
mod error;
mod groups;
mod journal;
#[cfg(all(test, unix))]
mod qa;
#[cfg(unix)]
mod recover;
mod replay;
mod verdict;

#[cfg(unix)]
pub use adopt::Adopted;
pub use boot::{BootStamp, BootVerdict, OsBootId, read_os_boot};
pub use checkpoint::{CHECKPOINT_FILE_FORMAT, CheckpointStore};
pub use error::RecoveryError;
pub use groups::{GroupId, HolderGroup, Placement};
pub use journal::{Finding, Incidents};
#[cfg(unix)]
pub use recover::{Recovered, RecoveryPlan, recover};
pub use replay::{Fence, Restored, Retained, Splice, restore};
pub use verdict::{Evidence, HolderAnswer, Liveness, Recorded, verdict};
