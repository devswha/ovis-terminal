//! The holder service process: bind, publish the record, serve until stopped, withdraw.

use std::io::ErrorKind;
use std::os::unix::net::UnixListener;

use ovis_terminal_protocol::domain::unix_service::{BootProof, DiscoveryRecord, Observed};
use ovis_terminal_protocol::schema::ids::{ProcessPid, StartToken};
use ovis_terminal_protocol::schema::methods::service::ServiceManager;

use super::{ServiceError, ServicePaths, observe, record};
use crate::process::{ProcessId, read_start_token};
use crate::protocol::{HolderBuildId, HolderDescriptor, HolderInstanceId};
use crate::unix::{Holder, HolderConfig, HolderLimits, HolderService};

/// The holder instance identity a boot proof names; `Welcome` carries it back.
pub(super) fn instance(proof: &BootProof) -> String {
    format!("{}:{}", proof.pid.get(), proof.start_token.as_str())
}

fn own_proof() -> Result<BootProof, ServiceError> {
    let pid = std::process::id();
    let token = read_start_token(ProcessId::new(pid)).map_err(ServiceError::StartToken)?;
    Ok(BootProof::new(
        ProcessPid::new(pid),
        StartToken::new(token.as_str().to_owned()),
    ))
}

/// Clears a proven-dead endpoint; a live or unanswering holder is never replaced.
fn claim(paths: &ServicePaths) -> Result<(), ServiceError> {
    record::create_directory(paths.directory())?;
    match observe(paths)? {
        Observed::Stopped => Ok(()),
        Observed::StaleEndpoint { .. } => {
            record::remove(&paths.socket())?;
            record::remove(&paths.record())
        }
        _ => Err(ServiceError::AlreadyRunning),
    }
}

fn bind(paths: &ServicePaths) -> Result<UnixListener, ServiceError> {
    UnixListener::bind(paths.socket()).map_err(|source| match source.kind() {
        ErrorKind::AddrInUse => ServiceError::AlreadyRunning,
        _ => ServiceError::Io {
            operation: "bind the holder socket",
            source,
        },
    })
}

/// Runs the holder of `paths` in this process until a core stops it.
///
/// `ready` runs once the socket accepts and the record is published; the manager or
/// launcher uses it to learn the holder is up. The record and socket are withdrawn on
/// the way out, but only while the record still names this process.
///
/// # Errors
///
/// [`ServiceError::AlreadyRunning`] when a holder lives at the endpoint, or the bind,
/// record, readiness or serve failure.
pub fn run(
    paths: &ServicePaths,
    manager: Option<ServiceManager>,
    ready: impl FnOnce(&DiscoveryRecord) -> Result<(), ServiceError>,
) -> Result<(), ServiceError> {
    claim(paths)?;
    let listener = bind(paths)?;
    let proof = own_proof()?;
    let socket = paths.socket();
    let address = socket
        .to_str()
        .ok_or_else(|| ServiceError::UnsupportedPath {
            path: socket.clone(),
        })?
        .to_owned();
    let descriptor = HolderDescriptor::new(
        HolderBuildId::new(format!(
            "ovis-terminal-holder/{}",
            env!("CARGO_PKG_VERSION")
        )),
        HolderInstanceId::new(instance(&proof)),
    );
    let holder = Holder::new(HolderConfig::new(descriptor, HolderLimits::DEFAULT));
    let service = HolderService::start(listener, holder).map_err(ServiceError::Serve)?;
    let published =
        DiscoveryRecord::new(paths.session().label(), address, proof.clone()).with_manager(manager);
    let announced = record::write(&paths.record(), &published).and_then(|()| ready(&published));
    let served = match announced {
        Ok(()) => service.wait().map_err(ServiceError::Serve),
        Err(error) => service.stop().map_err(ServiceError::Serve).and(Err(error)),
    };
    let withdrawn = record::withdraw(&paths.record(), &socket, &proof);
    served.and(withdrawn)
}
