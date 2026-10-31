//! Stable machine identities: validated names, aliases resolved once, explicit trust and
//! fresh boot proofs.
//!
//! A machine is keyed by the identity its runtime proves, never by its mutable name. Work is
//! bound to a [`Binding`] resolved once from a name or alias; renaming later keeps the old
//! name as an alias of the same machine, so no name can be redirected to another one.

mod credential;
mod error;
mod name;
mod proof;
mod registry;

pub use credential::Credential;
pub use error::IdentityError;
pub use name::Name;
pub use proof::{Trust, Trusted, fresh_challenge};
pub use registry::{Binding, Connected, Machines, NameHold};
