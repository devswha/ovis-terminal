//! Parent/child links bound to parent program incarnations (plan todo 41).
//!
//! A child names its parent by terminal and program incarnation. A finished or replaced
//! parent incarnation is refused before anything is linked, so a reused address never
//! adopts or ends another program's children. When a parent incarnation finishes, only
//! its live children that declared `die_with_parent` are ordered to end; a child counts
//! as ended only once its own exit is observed. Child news reaches a parent that asked
//! for it as a factual notice the parent acknowledges. Machines holding children of a
//! finished parent get its death as a durable notice, kept until they acknowledge it.

mod error;
mod exit;
mod remote;
mod service;
mod store;

pub use error::TreeError;
pub use exit::Exit;
pub use service::{Link, Linked, TreeService};

#[cfg(test)]
mod qa;
