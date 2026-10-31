//! The identity seam: authority scopes are generic over the identity types of the crate
//! that resolves targets, so the control core plugs in its registry identities without a
//! string conversion and without this crate depending on the core.

use std::fmt::Debug;
use std::hash::Hash;

use serde::Serialize;
use serde::de::DeserializeOwned;

/// A resolved identity usable inside scopes, targets and issuance documents.
pub trait Identity:
    Debug + Clone + Eq + Hash + Send + Sync + Serialize + DeserializeOwned + 'static
{
}

impl<T> Identity for T where
    T: Debug + Clone + Eq + Hash + Send + Sync + Serialize + DeserializeOwned + 'static
{
}

/// The identity types one resolver issues; each resource family is its own type, so a scope
/// cannot be built from an identity of another family.
pub trait IdentityFamily: Debug + Clone + Copy + Eq + Hash + Send + Sync + 'static {
    /// Stable machine identity.
    type Machine: Identity;
    /// Runtime lifetime credentials are scoped to.
    type Lifetime: Identity;
    /// Workspace identity.
    type Workspace: Identity;
    /// Tab identity.
    type Tab: Identity;
    /// Pane identity.
    type Pane: Identity;
    /// Terminal identity.
    type Terminal: Identity;
    /// Terminal incarnation (generation) a request resolved to.
    type Incarnation: Identity;
    /// Shared document name.
    type Document: Identity;
    /// Plugin identity.
    type Plugin: Identity;
}
