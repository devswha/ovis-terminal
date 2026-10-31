//! Negotiating `hello`: generation first, then the credential, then the identity result,
//! with a boot proof keyed by that credential when the client sent a fresh challenge.

use std::sync::Arc;

use ovis_terminal_authz::{AttestRequest, Authority, PeerAdmission, Presentation, Session};
use ovis_terminal_core::registry::RegistryIdentities;
use ovis_terminal_protocol::domain::machine_identity::{BootProof, BootStatement, ProofMac};
use ovis_terminal_protocol::rpc::RpcRequest;
use ovis_terminal_protocol::schema::methods::server::{HelloParams, HelloResult};
use ovis_terminal_protocol::schema::{PROTOCOL_GENERATION, decode_params};

use super::identity::now_ms;
use super::{HelloError, ServerIdentity};

/// What every connection of one server shares.
#[derive(Debug)]
pub(super) struct Shared {
    pub(super) authority: Arc<Authority<RegistryIdentities>>,
    pub(super) identity: ServerIdentity,
}

/// A successful hello: the reply, the authenticated session and the framing choice.
#[derive(Debug)]
pub(super) struct Negotiated {
    pub(super) result: HelloResult,
    pub(super) session: Session,
    pub(super) framed: bool,
}

/// Negotiates one `hello` over an admitted connection; nothing is exposed on failure.
pub(super) fn negotiate(
    request: &RpcRequest,
    shared: &Shared,
    admission: PeerAdmission,
) -> Result<Negotiated, HelloError> {
    let params: HelloParams = decode_params(request).map_err(HelloError::Params)?;
    if params.generation != PROTOCOL_GENERATION {
        return Err(HelloError::UnsupportedGeneration {
            requested: params.generation,
            served: PROTOCOL_GENERATION,
        });
    }
    let at = now_ms()?;
    // An absent credential is the empty bearer, which the authority reports as unauthenticated.
    let bearer = params.credential.as_deref().unwrap_or_default();
    let session = shared
        .authority
        .authenticate(&admission, Presentation::new(bearer, at))
        .map_err(HelloError::Credential)?;
    let mut result = shared.identity.hello_result(session.transport(), at);
    if let Some(challenge) = params.challenge {
        let statement = BootStatement::answering(challenge, &result);
        let mac = shared
            .authority
            .attest(&session, AttestRequest::new(&statement.signed_bytes(), at))
            .map_err(HelloError::Credential)?;
        result.boot_proof = Some(BootProof::new(
            statement,
            ProofMac::from_bytes(mac.to_bytes()),
        ));
    }
    Ok(Negotiated {
        result,
        session,
        framed: params.framed.unwrap_or(false),
    })
}
