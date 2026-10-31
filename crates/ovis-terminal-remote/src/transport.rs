//! Byte-transparent transports to remote runtimes: SSH or a custom argument vector whose
//! stdin and stdout carry protocol bytes unchanged while stderr carries only diagnostics.
//!
//! Nothing from a request is ever placed on a command line: the command is fixed when the
//! machine is added, and every protocol byte travels over stdin.

mod error;
mod handshake;
mod link;
mod spec;

pub use error::{Evidence, TransportError};
pub use handshake::Handshake;
pub use link::Link;
pub use spec::TransportSpec;
