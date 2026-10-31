//! Typed terminal service boundary failures.

/// A rejected lifecycle action or an unavailable piece of process evidence.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TerminalError {
    /// Authority denied admission or the actual commit boundary.
    #[error(transparent)]
    Authority(#[from] ovis_terminal_authz::AuthzError),
    /// Operation admission, deduplication or completion failed.
    #[error(transparent)]
    Operation(#[from] crate::ops::OperationError),
    /// Durable event or snapshot coordination failed.
    #[error(transparent)]
    Event(#[from] crate::events::EventError),
    /// Registry identity validation failed.
    #[error(transparent)]
    Identity(#[from] crate::registry::IdentityError),
    /// Registry lifecycle evidence did not permit a transition.
    #[error(transparent)]
    Lifecycle(#[from] crate::registry::LifecycleTransitionError),
    /// A typed record could not be serialized.
    #[error("terminal records could not be encoded or decoded")]
    Serialization(#[from] serde_json::Error),
    /// Parameters failed the launch or metadata boundary.
    #[error("the terminal parameters are invalid")]
    InvalidParams,
    /// A JSON boundary rejected malformed fields or numeric counter forms.
    #[error("the terminal request does not match its closed schema")]
    InvalidJson(#[source] serde_json::Error),
    /// No terminal has this identity.
    #[error("no terminal matches the requested identity")]
    NotFound,
    /// A live name is already reserved.
    #[error("another live terminal already owns the name")]
    DuplicateName,
    /// The requested process no longer matches the owned PID and start token.
    #[error("the requested process is no longer the owned process")]
    ProcessReplaced,
    /// Native evidence cannot prove the requested process role.
    #[error("the requested process role cannot be observed on this platform")]
    UnsupportedField,
    /// A request exceeds its advertised bounded receipt or registry capacity.
    #[error("the terminal request exceeds its configured resource budget")]
    LimitExceeded,
    /// A metadata removal must not implicitly terminate a live program.
    #[error("the terminal still has a live or unproven process")]
    StillLive,
    /// Only a kept, ended process may be relaunched.
    #[error("the terminal is not eligible for recovery")]
    NotLost,
    /// A process already ended.
    #[error("the terminal process already ended")]
    AlreadyFinished,
    /// Control ownership has been explicitly stopped or handed over.
    #[error("the control actor no longer admits mutations")]
    ControlStopped,
    /// Output evidence lost continuity.
    #[error("holder output is not contiguous with the retained offset")]
    OutputGap,
    /// A bounded worker notification was unavailable.
    #[error("no lifecycle notification arrived before the deadline")]
    Notification(#[source] std::sync::mpsc::RecvTimeoutError),
    /// Background launch worker could not be created.
    #[error("the launch worker could not start")]
    Worker(#[source] std::io::Error),
    /// A launch worker panicked.
    #[error("a launch worker stopped unexpectedly")]
    WorkerPanicked,
    /// Holder rejected an ownership operation.
    #[cfg(unix)]
    #[error(transparent)]
    Holder(#[from] ovis_terminal_holder::unix::HolderError),
    /// No holder message arrived or ownership disconnected.
    #[cfg(unix)]
    #[error(transparent)]
    Receive(#[from] ovis_terminal_holder::unix::RecvError),
    /// Native process identity could not be inspected.
    #[cfg(unix)]
    #[error(transparent)]
    Process(#[from] ovis_terminal_holder::process::StartTokenError),
}

impl TerminalError {
    /// Stable machine token.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Authority(error) => error.kind(),
            Self::Operation(error) => error.kind(),
            Self::Event(error) => error.kind(),
            Self::Identity(error) => error.kind(),
            Self::Lifecycle(error) => error.kind(),
            Self::Serialization(_) => "persistence_failed",
            Self::InvalidParams | Self::InvalidJson(_) => "invalid_params",
            Self::NotFound => "not_found",
            Self::DuplicateName => "duplicate_name",
            Self::ProcessReplaced => "process_replaced",
            Self::UnsupportedField => "unsupported_field",
            Self::LimitExceeded => "limit_exceeded",
            Self::StillLive => "still_live",
            Self::NotLost => "not_lost",
            Self::AlreadyFinished => "already_finished",
            Self::ControlStopped => "control_stopped",
            Self::OutputGap => "output_gap",
            Self::Notification(_) => "recv_timeout",
            Self::Worker(_) | Self::WorkerPanicked => "worker_unavailable",
            #[cfg(unix)]
            Self::Holder(error) => error.kind(),
            #[cfg(unix)]
            Self::Receive(error) => error.kind(),
            #[cfg(unix)]
            Self::Process(error) => error.kind(),
        }
    }

    /// Recovery guidance, independent of Display prose.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Authority(error) => error.hint(),
            Self::Operation(error) => error.hint(),
            Self::Event(error) => error.hint(),
            Self::Identity(error) => error.hint(),
            Self::Lifecycle(error) => error.hint(),
            Self::Serialization(_) => "inspect the retained terminal and operation records",
            Self::InvalidParams | Self::InvalidJson(_) => {
                "supply explicit valid argv, cwd, environment, grid and metadata"
            }
            Self::NotFound | Self::DuplicateName => "read the current terminal registry",
            Self::ProcessReplaced => "refresh PID and start token; never redirect a stale action",
            Self::UnsupportedField => {
                "inspect process_info field availability before requesting agent-only actions"
            }
            Self::LimitExceeded => {
                "reduce the launch metadata or operate on fewer terminals per request"
            }
            Self::StillLive => "stop the exact process explicitly before removing its metadata",
            Self::NotLost | Self::AlreadyFinished => "inspect the terminal's lifecycle evidence",
            Self::ControlStopped => {
                "connect to the replacement control server without replaying pending work"
            }
            Self::OutputGap => "read retained holder output before accepting process exit",
            Self::Notification(_) => "inspect the pending operation or wait for holder evidence",
            Self::Worker(_) | Self::WorkerPanicked => {
                "inspect the operation before requesting another launch"
            }
            #[cfg(unix)]
            Self::Holder(error) => error.hint(),
            #[cfg(unix)]
            Self::Receive(error) => error.hint(),
            #[cfg(unix)]
            Self::Process(error) => error.hint(),
        }
    }
}
