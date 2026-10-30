//! Native platform probes over the pinned portable-pty backend.
//!
//! Every probe reports what it measured on the running host or a typed
//! [`ProbeError`]; a capability that could not be observed is never reported as
//! present.

mod decimal;
#[cfg(unix)]
mod detach;
mod endpoint;
mod error;
mod host;
mod measure;
mod modes;
mod output;
mod process;
mod pump;
mod session;
mod stall;

#[cfg(unix)]
pub use detach::{DetachReport, Interruption, SurvivingProcess, probe_detach};
#[cfg(unix)]
pub use endpoint::{
    BrokenEndpoint, EndpointReport, SocketIdentity, probe_broken_unix_endpoint, probe_unix_endpoint,
};
#[cfg(windows)]
pub use endpoint::{PipeReport, probe_broken_named_pipe, probe_named_pipe};
pub use error::{Capability, EndpointOperation, ProbeError, PtyOperation};
pub use host::{ByteOrder, HostFacts};
pub use measure::{ByteCount, Micros, SampleCount};
pub use modes::{ModeRoundTrip, ModeSnapshot, probe_mode_round_trip};
pub use output::StreamEnd;
#[cfg(unix)]
pub use process::force_kill;
pub use process::{
    Pid, ProcessObservation, ProcessState, StartTokenSource, observe_process, resident_memory,
};
pub use session::{ExitRecord, Grid, PtySession};
pub use stall::{StallReport, WriterAfterKill, probe_stalled_writer};
