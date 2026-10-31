//! Typed attach outcomes and isolated output streams (plan todo 23).
//!
//! `attach.open` answers with a typed `ready`, `held`, `refused`, `recovering` or
//! `ended` record bound to a fresh stream identity and the terminal and program
//! incarnations. A stream starts with an offset-fenced checkpoint (cursor, modes,
//! grid, history cursor) followed by live bytes after that offset, minus queries the
//! VT answered. Control records travel ahead of credit-limited chunks; each viewer has
//! its own credit, cursor and bounded backlog over one shared immutable output log.

mod control;
mod credit;
mod error;
mod filter;
mod handler;
mod hub;
mod journal;
mod lane;
mod limits;
mod log;
mod offset;
mod open;
mod ops;
mod pump;
mod records;
mod resync;
mod upstream;
mod viewer;

pub use control::wire_modes;
pub use credit::CreditRefusal;
pub use error::StreamError;
pub use handler::AttachHandler;
pub use hub::{StreamSetup, TerminalStreams};
pub use journal::LeaseJournal;
pub use lane::AttachLane;
pub use limits::{SessionBacklog, StreamLimits};
pub use offset::{LogOffset, StreamOffset};
pub use upstream::{StreamChannels, Upstream};
