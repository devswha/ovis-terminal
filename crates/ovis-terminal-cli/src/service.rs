//! OS service verbs of the binary.

#[cfg(unix)]
pub mod unix;
#[cfg(windows)]
pub mod windows;

#[cfg(unix)]
pub use ovis_terminal_holder::service::unix::ServiceError;
#[cfg(windows)]
pub use ovis_terminal_holder::service::windows::HostError as ServiceError;
#[cfg(unix)]
pub use unix::{ServiceCommand, execute};
#[cfg(windows)]
pub use windows::{ServiceCommand, execute};
