//! Remote endpoints a machine may be reached at.

use std::collections::BTreeSet;

use super::violation::BoundaryViolation;
use crate::schema::methods::machine::MachineTransport;

const TRANSPORT: &str = "params.transport";

/// Which remote endpoints `machine.add` may connect to.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum EndpointAllowlist {
    /// Any destination the transport itself accepts; the runtime owner's default.
    #[default]
    Unrestricted,
    /// Only SSH destinations whose host is listed; a custom argv transport, which can
    /// reach anything, is refused.
    Hosts(BTreeSet<String>),
}

impl EndpointAllowlist {
    /// Allows only SSH destinations whose host part is one of `hosts`.
    #[must_use]
    pub fn hosts<I: IntoIterator<Item = String>>(hosts: I) -> Self {
        Self::Hosts(hosts.into_iter().collect())
    }

    /// Admits `transport` or refuses it as forbidden.
    ///
    /// The host of an SSH destination is the text after its last `@`, compared without
    /// ASCII case as DNS names are; the port, if any, belongs to SSH configuration and is not
    /// part of the destination.
    ///
    /// # Errors
    ///
    /// [`BoundaryViolation::EndpointNotAllowed`] for an unlisted host or an argv transport
    /// under [`EndpointAllowlist::Hosts`].
    pub fn admit(&self, transport: &MachineTransport) -> Result<(), BoundaryViolation> {
        let refused = BoundaryViolation::EndpointNotAllowed { field: TRANSPORT };
        match (self, transport) {
            (Self::Unrestricted, _) => Ok(()),
            (Self::Hosts(hosts), MachineTransport::Ssh(destination)) => {
                let host = destination
                    .rsplit_once('@')
                    .map_or(destination.as_str(), |(_, host)| host);
                if hosts
                    .iter()
                    .any(|allowed| allowed.eq_ignore_ascii_case(host))
                {
                    Ok(())
                } else {
                    Err(refused)
                }
            }
            (Self::Hosts(_), _) => Err(refused),
        }
    }
}
