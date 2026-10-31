//! The control server's named-pipe endpoint (plan todo 54).
//!
//! A pipe instance is owned by the serving account and its protected DACL allows only
//! that account; it refuses remote clients, and its first instance claims the name so a
//! squatter is never joined. Admission reads the descriptor Windows applied and the
//! client's own token, then every connection authenticates with a credential at hello,
//! exactly as on a Unix socket. Turning those operating-system facts into peer evidence
//! is platform-neutral; binding and accepting are Windows-only.

mod evidence;

#[cfg(windows)]
mod endpoint;
#[cfg(windows)]
mod serve;
#[cfg(test)]
#[cfg(windows)]
mod tests;

#[cfg(windows)]
pub use endpoint::{PipeConnection, PipeEndpoint};
pub use evidence::{AppliedSecurity, pipe_evidence};
#[cfg(windows)]
pub use serve::{PipeServeReport, PipeServerHandle, serve_pipe};
