//! An owner-only named-pipe endpoint: first-instance claim, overlapped accepts, peer
//! evidence from the applied descriptor and the client's own token.

use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use ovis_terminal_authz::{
    AceKind, AuthzError, EndpointPolicy, LocalAccount, PeerAdmission, PeerEvidence, PipeInstance,
    PipeNamespace, WindowsSid,
};
use ovis_terminal_holder::service::windows::pipe::{self, OwnerSecurity};
use ovis_terminal_protocol::domain::windows_service::PipeName;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::windows::named_pipe::NamedPipeServer;

use super::evidence::{AppliedSecurity, pipe_evidence};
use crate::listen::EndpointError;

/// Bound on a client sending its first bytes, after which its account can be read.
const FIRST_BYTES_BOUND: Duration = Duration::from_secs(10);

fn pipe_error(operation: &'static str, name: &PipeName, source: io::Error) -> EndpointError {
    EndpointError::Io {
        operation,
        path: name.as_str().into(),
        source,
    }
}

fn applied(pipe: &NamedPipeServer) -> io::Result<AppliedSecurity> {
    let security = pipe::instance_security(pipe)?;
    let dacl = security.dacl.map(|entries| {
        entries
            .into_iter()
            .map(|entry| {
                (
                    if entry.allow {
                        AceKind::Allow
                    } else {
                        AceKind::Deny
                    },
                    entry.trustee,
                )
            })
            .collect()
    });
    Ok(AppliedSecurity::new(security.owner, dacl))
}

/// A bound pipe name: the instance waiting for the next client, and what serves it.
#[derive(Debug)]
pub struct PipeEndpoint {
    name: PipeName,
    security: OwnerSecurity,
    policy: EndpointPolicy,
    namespace: Arc<Mutex<PipeNamespace>>,
    next: NamedPipeServer,
    next_kind: PipeInstance,
}

fn insecure(name: &PipeName) -> impl FnOnce(AuthzError) -> EndpointError + '_ {
    move |source| EndpointError::Insecure {
        path: name.as_str().into(),
        source,
    }
}

impl PipeEndpoint {
    /// Claims `name` with a first instance owned by this account.
    ///
    /// # Errors
    ///
    /// [`EndpointError::AlreadyServing`] when any process holds an instance of the name,
    /// [`EndpointError::Insecure`] when Windows applied weaker security than requested,
    /// and [`EndpointError::Io`] for other failures. Must run inside a Tokio runtime.
    pub fn bind(name: &PipeName) -> Result<Self, EndpointError> {
        let owner = pipe::current_user_sid()
            .map_err(|source| pipe_error("read the account SID", name, source))?;
        let sid = WindowsSid::parse(&owner).map_err(insecure(name))?;
        let security = OwnerSecurity::for_owner(&owner)
            .map_err(|source| pipe_error("build the pipe security", name, source))?;
        let next =
            pipe::create_instance(name, true, &security).map_err(|source| match source.kind() {
                io::ErrorKind::PermissionDenied => EndpointError::AlreadyServing {
                    path: name.as_str().into(),
                },
                _ => pipe_error("create the pipe", name, source),
            })?;
        let endpoint = Self {
            name: name.clone(),
            security,
            policy: EndpointPolicy::new(LocalAccount::Windows(sid.clone())),
            namespace: Arc::new(Mutex::new(PipeNamespace::new(sid))),
            next,
            next_kind: PipeInstance::First,
        };
        endpoint.track(&endpoint.next, PipeInstance::First)?;
        Ok(endpoint)
    }

    /// The bound pipe name.
    #[must_use]
    pub const fn name(&self) -> &PipeName {
        &self.name
    }

    /// Checks the security Windows applied to a new instance and counts it as open.
    fn track(&self, instance: &NamedPipeServer, kind: PipeInstance) -> Result<(), EndpointError> {
        let security = applied(instance)
            .map_err(|source| pipe_error("read the pipe security", &self.name, source))?;
        let evidence =
            pipe_evidence(kind, &security, self.security.owner()).map_err(insecure(&self.name))?;
        let PeerEvidence::NamedPipe { instance, .. } = evidence else {
            return Err(insecure(&self.name)(AuthzError::PeerRejected));
        };
        self.namespace
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .open(&instance)
            .map_err(insecure(&self.name))
    }

    /// Waits for the next client; the following instance exists before this returns.
    ///
    /// # Cancel Safety
    ///
    /// Cancel safe: a dropped call cancels the pending overlapped connect and keeps the
    /// waiting instance, so the next call accepts on it.
    ///
    /// # Errors
    ///
    /// [`EndpointError::Io`] or [`EndpointError::Insecure`] when the following instance
    /// cannot be created securely. A connect that failed for one client is that
    /// connection's evidence failure, so it refuses only that client.
    pub async fn accept(&mut self) -> Result<PipeConnection, EndpointError> {
        let connected = self.next.connect().await;
        let following = pipe::create_instance(&self.name, false, &self.security)
            .map_err(|source| pipe_error("create the next pipe instance", &self.name, source))?;
        self.track(&following, PipeInstance::Additional)?;
        let accepted = std::mem::replace(&mut self.next, following);
        let kind = std::mem::replace(&mut self.next_kind, PipeInstance::Additional);
        let evidence = connected
            .and_then(|()| applied(&accepted))
            .map(|security| (kind, security))
            .map_err(|source| pipe_error("accept a client", &self.name, source));
        Ok(PipeConnection {
            pipe: accepted,
            evidence,
            namespace: Arc::clone(&self.namespace),
            policy: self.policy.clone(),
        })
    }
}

/// One accepted client; dropping it closes its instance.
#[derive(Debug)]
pub struct PipeConnection {
    pipe: NamedPipeServer,
    evidence: Result<(PipeInstance, AppliedSecurity), EndpointError>,
    namespace: Arc<Mutex<PipeNamespace>>,
    policy: EndpointPolicy,
}

impl PipeConnection {
    /// Admits the client once it has sent its first bytes: the instance's applied
    /// descriptor must be owner-only and the client's token must be the owner's.
    ///
    /// # Errors
    ///
    /// [`AuthzError::EndpointInsecure`], [`AuthzError::PeerRejected`] for another account,
    /// a client that sent nothing in time, or an account that could not be read.
    pub async fn admit(&self) -> Result<PeerAdmission, AuthzError> {
        let Ok((instance, security)) = &self.evidence else {
            return Err(AuthzError::PeerRejected);
        };
        let ready = tokio::time::timeout(FIRST_BYTES_BOUND, self.pipe.readable()).await;
        if !matches!(ready, Ok(Ok(()))) {
            return Err(AuthzError::PeerRejected);
        }
        let client = pipe::client_sid(&self.pipe).map_err(|_| AuthzError::PeerRejected)?;
        let evidence = pipe_evidence(*instance, security, &client)?;
        self.policy.admit(&evidence)
    }
}

impl Drop for PipeConnection {
    fn drop(&mut self) {
        // Every accepted instance was counted open once, so closing it cannot underflow.
        self.namespace
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .close()
            .unwrap_or_default();
    }
}

impl AsyncRead for PipeConnection {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.pipe).poll_read(cx, buf)
    }
}

impl AsyncWrite for PipeConnection {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.pipe).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.pipe).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.pipe).poll_shutdown(cx)
    }
}
