//! Reading and atomically replacing one discovery record, owner-only.

use std::fs::{DirBuilder, OpenOptions};
use std::io::{ErrorKind, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::path::Path;

use ovis_terminal_protocol::domain::unix_service::{BootProof, DiscoveryRecord};

use super::ServiceError;

fn io(operation: &'static str) -> impl FnOnce(std::io::Error) -> ServiceError {
    move |source| ServiceError::Io { operation, source }
}

/// Creates the owner-only service directory and its parents.
pub(super) fn create_directory(directory: &Path) -> Result<(), ServiceError> {
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)
        .map_err(io("create the service directory"))
}

/// The record at `path`; a missing file is `None`, an unusable one is an error.
pub(super) fn read(path: &Path) -> Result<Option<DiscoveryRecord>, ServiceError> {
    match std::fs::read(path) {
        Ok(bytes) => DiscoveryRecord::parse(&bytes)
            .map(Some)
            .map_err(ServiceError::Record),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ServiceError::Io {
            operation: "read the discovery record",
            source,
        }),
    }
}

/// Writes `record` to a fresh owner-only file and renames it over `path`.
pub(super) fn write(path: &Path, record: &DiscoveryRecord) -> Result<(), ServiceError> {
    let bytes = serde_json::to_vec(record).map_err(|source| ServiceError::Io {
        operation: "encode the discovery record",
        source: source.into(),
    })?;
    let staging = path.with_extension(format!("json.{}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&staging)
        .map_err(io("create the discovery record"))?;
    file.write_all(&bytes)
        .map_err(io("write the discovery record"))?;
    file.sync_all().map_err(io("sync the discovery record"))?;
    std::fs::rename(&staging, path).map_err(io("publish the discovery record"))
}

/// Removes the record and socket only while the record still names `owner`.
pub(super) fn withdraw(path: &Path, socket: &Path, owner: &BootProof) -> Result<(), ServiceError> {
    match read(path)? {
        Some(record) if record.boot_proof == *owner => {}
        Some(_) | None => return Ok(()),
    }
    remove(socket)?;
    remove(path)
}

/// Removes `path`; an already missing file is not an error.
pub(super) fn remove(path: &Path) -> Result<(), ServiceError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(source) => Err(ServiceError::Io {
            operation: "remove a service file",
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use ovis_terminal_protocol::domain::unix_service::{BootProof, DiscoveryRecord};
    use ovis_terminal_protocol::schema::ids::{ProcessPid, ServiceLabel, StartToken};

    use super::{read, withdraw, write};
    use crate::service::unix::test_root::TestRoot;

    fn proof(pid: u32) -> BootProof {
        BootProof::new(
            ProcessPid::new(pid),
            StartToken::new(format!("token-{pid}")),
        )
    }

    #[test]
    fn a_record_is_owner_only_replaced_whole_and_withdrawn_only_by_its_owner() {
        let root = TestRoot::new("record");
        let path = root.path().join("r.json");
        let socket = root.path().join("s.sock");
        assert!(read(&path).unwrap().is_none());
        let first = DiscoveryRecord::new(
            ServiceLabel::new("l".to_owned()),
            "/s".to_owned(),
            proof(1000),
        );
        write(&path, &first).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let second = DiscoveryRecord::new(
            ServiceLabel::new("l".to_owned()),
            "/s".to_owned(),
            proof(2000),
        );
        write(&path, &second).unwrap();
        assert_eq!(read(&path).unwrap(), Some(second));
        std::fs::write(&socket, b"").unwrap();
        withdraw(&path, &socket, &proof(1000)).unwrap();
        assert!(
            path.exists() && socket.exists(),
            "a former owner removed a newer record"
        );
        withdraw(&path, &socket, &proof(2000)).unwrap();
        assert!(!path.exists() && !socket.exists());
        assert_eq!(
            std::fs::read_dir(root.path()).unwrap().count(),
            0,
            "staging files remain"
        );
    }

    #[test]
    fn a_corrupt_record_is_an_error_and_is_never_read_as_absent() {
        let root = TestRoot::new("corrupt");
        let path = root.path().join("r.json");
        std::fs::write(&path, b"{\"format\":\"ovis-terminal-holder-endpoint/1\"}").unwrap();
        assert_eq!(
            read(&path).map(drop).map_err(|e| e.kind()),
            Err("descriptor_malformed")
        );
    }
}
