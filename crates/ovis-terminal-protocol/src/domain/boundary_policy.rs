//! Typed construction of values that cross the runtime's trust boundary (plan todo 58).
//!
//! Launch argument vectors and environments, relative store paths and remote endpoints are
//! parsed once, at the boundary, into types that cannot hold an unsafe value: an argv is a
//! literal vector that no shell ever sees, a relative path has only normal components, an
//! endpoint either names an allowlisted host or is refused. Every refusal is a
//! [`BoundaryViolation`] whose kind is the domain's own (`too_large`, `forbidden`,
//! `invalid_params`) and whose field path names the parameter, never its value, so a
//! refusal cannot echo a secret.

mod endpoint;
mod history;
mod launch;
mod path;
mod violation;

#[cfg(test)]
mod qa;

pub use endpoint::EndpointAllowlist;
pub use launch::{LaunchArgv, LaunchEnvironment, RESERVED_ENV_PREFIX};
pub use path::{RelativePath, StoreRef};
pub use violation::{BoundaryViolation, Malformation, PayloadLimit};
