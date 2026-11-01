//! Rotation, withdrawal and per-request revalidation of credentials (plan todo 57).
//!
//! Every principal has an epoch that advances whenever one of its credentials is revoked
//! or rotated. A rotation names the epoch its caller last read, so two administrators
//! racing to rotate the same principal cannot both believe they hold its current
//! credential. A connection keeps no authority of its own: [`crate::Authority::validate`]
//! rechecks the credential behind a session for every request it sends.

mod requests;

pub use ovis_terminal_protocol::schema::scalar::PrincipalEpoch;
pub use requests::{Rotated, Rotation, Withdrawn};
