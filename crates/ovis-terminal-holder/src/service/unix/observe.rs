//! Side-effect-free observation of one holder service.
//!
//! An observation reads the record, reads the recorded process's start token and makes
//! one bounded, non-blocking connection attempt that sends nothing. It never creates,
//! binds, removes, adopts or starts anything, so observing a stopped service leaves it
//! stopped and observing a stale one leaves the evidence in place.

use std::io::ErrorKind;
use std::os::unix::fs::FileTypeExt as _;
use std::path::Path;
use std::time::Duration;

use ovis_terminal_protocol::domain::unix_service::{DiscoveryRecord, Observed, StaleReason};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::io::Errno;
use rustix::net::{AddressFamily, SocketAddrUnix, SocketType};

use super::{ServiceError, ServicePaths, record};
use crate::process::{ProcessId, StartTokenError, read_start_token};

/// Bound on one connection attempt.
const CONNECT_BOUND: Duration = Duration::from_millis(500);

enum Occupant {
    Vacant,
    Socket,
    Other,
}

fn occupant(path: &Path) -> Result<Occupant, ServiceError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_socket() => Ok(Occupant::Socket),
        Ok(_) => Ok(Occupant::Other),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Occupant::Vacant),
        Err(source) => Err(ServiceError::Io {
            operation: "inspect the holder socket",
            source,
        }),
    }
}

/// Whether something accepts on `path` within [`CONNECT_BOUND`]; nothing is sent.
fn accepts(path: &Path) -> Result<bool, ServiceError> {
    let failed = |source: Errno| ServiceError::Io {
        operation: "probe the holder socket",
        source: source.into(),
    };
    let address = SocketAddrUnix::new(path).map_err(failed)?;
    let socket =
        rustix::net::socket(AddressFamily::UNIX, SocketType::STREAM, None).map_err(failed)?;
    let flags = rustix::fs::fcntl_getfl(&socket).map_err(failed)?;
    rustix::fs::fcntl_setfl(&socket, flags | rustix::fs::OFlags::NONBLOCK).map_err(failed)?;
    match rustix::net::connect(&socket, &address) {
        Ok(()) => return Ok(true),
        Err(Errno::INPROGRESS | Errno::INTR) => {}
        Err(Errno::CONNREFUSED | Errno::NOENT | Errno::AGAIN) => return Ok(false),
        Err(error) => return Err(failed(error)),
    }
    let mut fds = [PollFd::new(&socket, PollFlags::OUT)];
    let bound = Timespec::try_from(CONNECT_BOUND).map_err(|_| failed(Errno::INVAL))?;
    if poll(&mut fds, Some(&bound)).map_err(failed)? == 0 {
        return Ok(false);
    }
    Ok(rustix::net::sockopt::socket_error(&socket)
        .map_err(failed)?
        .is_ok())
}

fn unrecorded(socket: &Path) -> Result<Observed, ServiceError> {
    Ok(match occupant(socket)? {
        Occupant::Vacant => Observed::Stopped,
        Occupant::Other => Observed::StaleEndpoint {
            record: None,
            reason: StaleReason::NotASocket,
        },
        Occupant::Socket if accepts(socket)? => Observed::Unreachable { record: None },
        Occupant::Socket => Observed::StaleEndpoint {
            record: None,
            reason: StaleReason::SocketRefused,
        },
    })
}

fn recorded(record: DiscoveryRecord) -> Result<Observed, ServiceError> {
    let pid = ProcessId::new(record.boot_proof.pid.get());
    let reason = match read_start_token(pid) {
        Err(StartTokenError::ProcessGone { .. }) => Some(StaleReason::ProcessGone),
        Err(other) => return Err(ServiceError::StartToken(other)),
        Ok(token) if token.as_str() != record.boot_proof.start_token.as_str() => {
            Some(StaleReason::ProcessReplaced)
        }
        Ok(_) => None,
    };
    if let Some(reason) = reason {
        return Ok(Observed::StaleEndpoint {
            record: Some(record),
            reason,
        });
    }
    let address = Path::new(&record.address);
    let live = matches!(occupant(address)?, Occupant::Socket) && accepts(address)?;
    Ok(if live {
        Observed::Running { record }
    } else {
        Observed::Unreachable {
            record: Some(record),
        }
    })
}

/// Observes the service at `paths` without changing anything.
///
/// # Errors
///
/// A record that cannot be parsed, a process table that cannot be read or a path that
/// cannot be inspected; none of them is turned into a state.
pub fn observe(paths: &ServicePaths) -> Result<Observed, ServiceError> {
    record::read(&paths.record())?.map_or_else(|| unrecorded(&paths.socket()), recorded)
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixListener;

    use ovis_terminal_protocol::domain::unix_service::{
        BootProof, DiscoveryRecord, Observed, StaleReason,
    };
    use ovis_terminal_protocol::schema::ids::{ProcessPid, StartToken};

    use super::observe;
    use crate::process::{ProcessId, read_start_token};
    use crate::service::unix::record;
    use crate::service::unix::test_root::TestRoot;

    fn own_proof() -> BootProof {
        let token = read_start_token(ProcessId::new(std::process::id())).unwrap();
        BootProof::new(
            ProcessPid::new(std::process::id()),
            StartToken::new(token.as_str().to_owned()),
        )
    }

    fn reaped_pid() -> u32 {
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .spawn()
            .unwrap();
        let pid = child.id();
        child.wait().unwrap();
        pid
    }

    fn entries(root: &TestRoot) -> Vec<std::path::PathBuf> {
        let mut found: Vec<_> = walk(root.path());
        found.sort();
        found
    }

    fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        std::fs::read_dir(dir)
            .unwrap()
            .flat_map(|entry| {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    let mut below = walk(&path);
                    below.push(path);
                    below
                } else {
                    vec![path]
                }
            })
            .collect()
    }

    #[test]
    fn an_empty_root_is_stopped_and_observing_creates_nothing() {
        let root = TestRoot::new("stopped");
        let paths = root.paths("s");
        for _ in 0..3 {
            assert_eq!(observe(&paths).unwrap(), Observed::Stopped);
        }
        assert_eq!(entries(&root), Vec::<std::path::PathBuf>::new());
    }

    #[test]
    fn a_record_naming_a_reaped_or_replaced_process_is_stale_and_is_left_in_place() {
        let root = TestRoot::new("stale");
        let paths = root.paths("s");
        record::create_directory(paths.directory()).unwrap();
        let gone = BootProof::new(
            ProcessPid::new(reaped_pid()),
            StartToken::new("x".to_owned()),
        );
        let written = DiscoveryRecord::new(
            paths.session().label(),
            paths.socket().display().to_string(),
            gone,
        );
        record::write(&paths.record(), &written).unwrap();
        let before = entries(&root);
        let reason = |observed| match observed {
            Observed::StaleEndpoint {
                reason,
                record: Some(_),
            } => Some(reason),
            _ => None,
        };
        assert_eq!(
            reason(observe(&paths).unwrap()),
            Some(StaleReason::ProcessGone)
        );
        let mut replaced = own_proof();
        replaced.start_token = StartToken::new("another-start".to_owned());
        let reused = DiscoveryRecord::new(
            paths.session().label(),
            paths.socket().display().to_string(),
            replaced,
        );
        record::write(&paths.record(), &reused).unwrap();
        assert_eq!(
            reason(observe(&paths).unwrap()),
            Some(StaleReason::ProcessReplaced)
        );
        assert_eq!(entries(&root), before);
    }

    #[test]
    fn a_live_recorded_process_is_running_only_while_its_socket_accepts() {
        let root = TestRoot::new("live");
        let paths = root.paths("s");
        record::create_directory(paths.directory()).unwrap();
        let written = DiscoveryRecord::new(
            paths.session().label(),
            paths.socket().display().to_string(),
            own_proof(),
        );
        record::write(&paths.record(), &written).unwrap();
        assert!(matches!(
            observe(&paths).unwrap(),
            Observed::Unreachable { record: Some(_) }
        ));
        let listener = UnixListener::bind(paths.socket()).unwrap();
        assert_eq!(
            observe(&paths).unwrap(),
            Observed::Running { record: written }
        );
        drop(listener);
        assert!(matches!(
            observe(&paths).unwrap(),
            Observed::Unreachable { record: Some(_) }
        ));
    }

    #[test]
    fn an_unrecorded_socket_nobody_accepts_on_is_stale_and_a_plain_file_is_not_a_socket() {
        let root = TestRoot::new("orphan");
        let paths = root.paths("s");
        record::create_directory(paths.directory()).unwrap();
        drop(UnixListener::bind(paths.socket()).unwrap());
        let expected = Observed::StaleEndpoint {
            record: None,
            reason: StaleReason::SocketRefused,
        };
        assert_eq!(observe(&paths).unwrap(), expected);
        assert!(
            paths.socket().exists(),
            "observation removed the stale socket"
        );
        std::fs::remove_file(paths.socket()).unwrap();
        std::fs::write(paths.socket(), b"").unwrap();
        let expected = Observed::StaleEndpoint {
            record: None,
            reason: StaleReason::NotASocket,
        };
        assert_eq!(observe(&paths).unwrap(), expected);
    }
}
