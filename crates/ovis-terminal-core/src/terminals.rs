//! Terminal lifecycle and process incarnation evidence (plan todo 16).

mod error;
pub mod images;
mod inspection;
mod labels;
pub mod process_info;
mod receipts;
mod records;
#[cfg(unix)]
mod runtime;
mod snapshot;

pub use error::TerminalError;
pub use inspection::{Field, ProcessInfo, ProcessSample, Unavailable};
pub use labels::LabelRequest;
pub use receipts::{
    ControlReceipt, ControlState, KillReceipt, KillRequest, KillStage, RemovalReceipt,
};
pub use records::{Launch, LaunchState, Program, Record};
#[cfg(unix)]
pub use runtime::{Call, Host, Terminals};
pub use snapshot::{LifecycleEvent, Producer, TerminalSnapshot};
