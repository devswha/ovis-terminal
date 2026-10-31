//! The only way to resize: a geometry-lease authorization issued by the lease table.

use super::GeometryError;
use crate::leases::{Authorization, Caller, LeaseError, LeaseSlot, TerminalLeases};
use crate::registry::LeaseId;

/// Permission to resize, produced only by [`GeometryAuthorization::admit`], which asks the
/// lease table's `authorize_resize`; a write or any other authorization cannot become one.
#[derive(Debug)]
pub struct GeometryAuthorization {
    pub(super) admitted: Authorization,
    pub(super) owner: LeaseId,
}

impl GeometryAuthorization {
    /// Admits `caller` to resize under the geometry lease `lease`.
    ///
    /// # Errors
    ///
    /// Returns `forbidden`, `observe_forbidden_op`, `no_lease` or `not_geometry_owner`.
    pub fn admit(
        leases: &TerminalLeases,
        caller: Caller<'_>,
        lease: LeaseId,
    ) -> Result<Self, GeometryError> {
        let admitted = leases
            .authorize_resize(caller, lease)
            .map_err(GeometryError::Lease)?;
        let owner = match admitted.lease() {
            Some(held) if held.slot == LeaseSlot::Geometry => held.lease_id,
            Some(_) | None => {
                let refusal = leases.view().geometry.map_or(LeaseError::NoLease, |owner| {
                    LeaseError::NotGeometryOwner {
                        owner: Box::new(owner),
                    }
                });
                return Err(GeometryError::Lease(refusal));
            }
        };
        Ok(Self { admitted, owner })
    }
}
