//! Per-connection negotiated hello and multiplexing (plan todo 13).
//!
//! A connection starts in persistent NDJSON mode, must complete `hello` before any other
//! method runs, may upgrade to framed mode at the hello boundary, and then serves
//! concurrent correlated requests until the peer closes or the server shuts down.

mod budget;
mod cause;
mod connections;
mod core;
mod driver;
mod error;
mod handler;
mod hello;
mod identity;
mod outbound;
mod writer;

#[cfg(test)]
mod half_close_order;
#[cfg(test)]
mod terminal_order;

pub use connections::Connections;
pub use error::{ConnectionError, HelloError};
pub use handler::{Call, RequestHandler};
pub use identity::ServerIdentity;
pub use outbound::{Ending, Notifier, NotifyError};
