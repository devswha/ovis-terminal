//! The runtime's trust boundary policy (plan todo 58).
//!
//! [`BoundaryPolicy`] wraps the request handler: it refuses a mutation the session's
//! credential confers no permission for, before parameters are even parsed, keeps one
//! redacted audit record per call with its request, operation, principal, target and
//! outcome, and redacts the data of every error reply. [`AgentHistory`] serves
//! `agent.history` from an authorized root without following paths out of it, and
//! [`admit_launch`] is the one check of a launch's literal argv and environment.

mod audit;
mod gate;
mod history;
mod launch;
mod matrix;

#[cfg(test)]
mod qa;

pub use audit::AuditTrail;
pub use gate::BoundaryPolicy;
pub use history::AgentHistory;
pub use launch::{admit_launch, violation};
pub use matrix::{CONTROL_METHODS, MUTATION_ROLES, is_control, required_action};
