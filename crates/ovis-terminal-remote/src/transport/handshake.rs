//! The `hello` exchange that opens a transport, answered with a fresh boot proof.

use std::time::Duration;

use ovis_terminal_protocol::domain::machine_identity::BootChallenge;
use ovis_terminal_protocol::rpc::{RpcReply, parse_reply};
use ovis_terminal_protocol::schema::PROTOCOL_GENERATION;
use ovis_terminal_protocol::schema::methods::server::HelloResult;
use serde_json::json;

use super::error::Evidence;
use super::{Link, TransportError};
use crate::identity::Credential;

const HELLO_ID: &str = "hello";

/// One handshake: the challenge to send and how long the whole exchange may take.
#[derive(Debug, Clone, Copy)]
pub struct Handshake {
    challenge: BootChallenge,
    bound: Duration,
}

impl Handshake {
    /// A handshake sending `challenge`, bounded by `bound`.
    #[must_use]
    pub const fn new(challenge: BootChallenge, bound: Duration) -> Self {
        Self { challenge, bound }
    }
}

impl Link {
    /// Sends `hello` with `credential` and the challenge, then reads its reply, which must
    /// be the first bytes on stdout. The result is not trusted until its proof is verified.
    ///
    /// # Errors
    ///
    /// [`TransportError::Timeout`] past the bound, [`TransportError::NotProtocol`] for a
    /// banner or prompt, [`TransportError::Closed`] when the transport ends,
    /// [`TransportError::Refused`] for a typed remote refusal and
    /// [`TransportError::Incompatible`] for a result of another protocol.
    pub async fn handshake(
        &mut self,
        credential: &Credential,
        handshake: Handshake,
    ) -> Result<HelloResult, TransportError> {
        let request = json!({
            "jsonrpc": "2.0", "id": HELLO_ID, "method": "hello",
            "params": {
                "generation": PROTOCOL_GENERATION,
                "client": {"name": env!("CARGO_PKG_NAME"), "version": env!("CARGO_PKG_VERSION")},
                "credential": credential.expose(),
                "challenge": handshake.challenge,
            },
        });
        let mut bytes = serde_json::to_vec(&request).map_err(TransportError::Incompatible)?;
        bytes.push(b'\n');
        let exchange = async {
            self.send(&bytes).await?;
            self.receive().await
        };
        let line = match tokio::time::timeout(handshake.bound, exchange).await {
            Ok(line) => line?,
            Err(_) => {
                return Err(TransportError::Timeout {
                    bound: handshake.bound,
                    received: self.received(),
                });
            }
        };
        let not_protocol = || TransportError::NotProtocol {
            first_bytes: Evidence::of(line.get(..256).unwrap_or(&line)),
        };
        let text = std::str::from_utf8(&line).map_err(|_| not_protocol())?;
        match parse_reply(text).map_err(|_| not_protocol())? {
            RpcReply::Success(reply) if reply.id.as_str() == HELLO_ID => {
                serde_json::from_value(reply.result).map_err(TransportError::Incompatible)
            }
            RpcReply::Failure(reply) if reply.id.as_str() == HELLO_ID => {
                Err(TransportError::Refused {
                    remote_kind: reply.error.data.kind.token().to_owned(),
                })
            }
            _ => Err(not_protocol()),
        }
    }
}

#[cfg(all(test, unix))]
#[path = "handshake/tests.rs"]
mod tests;
