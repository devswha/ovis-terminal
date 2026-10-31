//! Per-request authentication (plan todo 57).
//!
//! `hello` proves a credential once, but a connection keeps no authority of its own:
//! [`Revalidated`] rechecks the credential behind the connection's session before every
//! request reaches its handler, ahead of parameter decoding, so a revoked or expired
//! credential is refused with its typed kind and the principal's current epoch whatever
//! it sends. Grants are still decided per request and rechecked at every commit.

mod revalidate;

pub use revalidate::{Revalidated, refusal};
