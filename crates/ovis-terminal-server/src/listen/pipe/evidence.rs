//! Turning what Windows reports about a pipe instance and its client into peer evidence.
//!
//! The owner and DACL come from the instance's applied security descriptor, not from the
//! descriptor the listener asked for, so an instance Windows created with weaker security
//! is refused at admission instead of trusted.

use ovis_terminal_authz::{
    Ace, AceKind, AuthzError, Dacl, PeerEvidence, PipeInstance, PipeInstanceEvidence, PipeSecurity,
    RemoteClients, WindowsSid,
};

/// The owner and DACL entries Windows applied to one pipe instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedSecurity {
    owner: String,
    dacl: Option<Vec<(AceKind, String)>>,
}

impl AppliedSecurity {
    /// `owner` and the DACL entries as SID strings; `None` is a null DACL.
    #[must_use]
    pub const fn new(owner: String, dacl: Option<Vec<(AceKind, String)>>) -> Self {
        Self { owner, dacl }
    }
}

/// Evidence of a connection accepted on an instance created as `instance` with remote
/// clients refused, whose client account is `client`.
///
/// # Errors
///
/// [`AuthzError::InvalidValue`] when Windows reported a SID that is not one.
pub fn pipe_evidence(
    instance: PipeInstance,
    applied: &AppliedSecurity,
    client: &str,
) -> Result<PeerEvidence, AuthzError> {
    let dacl = match &applied.dacl {
        None => Dacl::Null,
        Some(entries) => Dacl::Entries(
            entries
                .iter()
                .map(|(kind, trustee)| WindowsSid::parse(trustee).map(|sid| Ace::new(*kind, sid)))
                .collect::<Result<_, _>>()?,
        ),
    };
    let security = PipeSecurity::new(WindowsSid::parse(&applied.owner)?, dacl);
    Ok(PeerEvidence::NamedPipe {
        instance: PipeInstanceEvidence::new(instance, RemoteClients::Rejected, security),
        client: WindowsSid::parse(client)?,
    })
}

#[cfg(test)]
mod tests {
    use ovis_terminal_authz::{AceKind, EndpointPolicy, LocalAccount, PipeInstance, WindowsSid};

    use super::{AppliedSecurity, pipe_evidence};

    const OWNER: &str = "S-1-5-21-10-20-30-1001";
    const OTHER: &str = "S-1-5-21-10-20-30-1002";

    fn admit(applied: &AppliedSecurity, client: &str) -> Result<(), &'static str> {
        let policy = EndpointPolicy::new(LocalAccount::Windows(WindowsSid::parse(OWNER).unwrap()));
        let evidence = pipe_evidence(PipeInstance::First, applied, client).map_err(|e| e.kind())?;
        policy.admit(&evidence).map(drop).map_err(|e| e.kind())
    }

    fn only(entries: &[(AceKind, &str)]) -> AppliedSecurity {
        let dacl = entries
            .iter()
            .map(|(kind, sid)| (*kind, (*sid).to_owned()))
            .collect();
        AppliedSecurity::new(OWNER.to_owned(), Some(dacl))
    }

    #[test]
    fn the_owner_connecting_to_an_owner_only_pipe_is_admitted() {
        assert_eq!(admit(&only(&[(AceKind::Allow, OWNER)]), OWNER), Ok(()));
        let denying = only(&[(AceKind::Allow, OWNER), (AceKind::Deny, OTHER)]);
        assert_eq!(admit(&denying, OWNER), Ok(()));
    }

    #[test]
    fn another_account_is_refused_even_on_an_owner_only_pipe() {
        assert_eq!(
            admit(&only(&[(AceKind::Allow, OWNER)]), OTHER),
            Err("peer_rejected")
        );
    }

    #[test]
    fn a_null_global_or_foreign_owned_descriptor_is_never_trusted() {
        let null = AppliedSecurity::new(OWNER.to_owned(), None);
        assert_eq!(admit(&null, OWNER), Err("endpoint_insecure"));
        let everyone = only(&[(AceKind::Allow, OWNER), (AceKind::Allow, "S-1-1-0")]);
        assert_eq!(admit(&everyone, OWNER), Err("endpoint_insecure"));
        let foreign = AppliedSecurity::new(
            OTHER.to_owned(),
            Some(vec![(AceKind::Allow, OWNER.to_owned())]),
        );
        assert_eq!(admit(&foreign, OWNER), Err("endpoint_insecure"));
    }

    #[test]
    fn text_that_is_not_a_sid_is_refused_rather_than_ignored() {
        let garbled = only(&[(AceKind::Allow, "Everyone")]);
        assert_eq!(admit(&garbled, OWNER), Err("invalid_params"));
        assert_eq!(
            admit(&only(&[(AceKind::Allow, OWNER)]), "alice"),
            Err("invalid_params")
        );
    }
}
