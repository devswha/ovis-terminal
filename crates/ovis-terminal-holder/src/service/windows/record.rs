//! Reading, atomically replacing and withdrawing the host record.

use std::io::{ErrorKind, Write as _};
use std::path::Path;

use ovis_terminal_protocol::domain::unix_service::BootProof;
use ovis_terminal_protocol::domain::windows_service::HostRecord;

use super::HostError;

fn io(operation: &'static str) -> impl FnOnce(std::io::Error) -> HostError {
    move |source| HostError::Io { operation, source }
}

/// The record at `path`; a missing file is `None`, an unusable one is an error.
///
/// # Errors
///
/// [`HostError::Record`] for a record of another shape and [`HostError::Io`] for a read
/// failure.
pub fn read(path: &Path) -> Result<Option<HostRecord>, HostError> {
    match std::fs::read(path) {
        Ok(bytes) => HostRecord::parse(&bytes)
            .map(Some)
            .map_err(HostError::Record),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(source) => Err(HostError::Io {
            operation: "read the host record",
            source,
        }),
    }
}

/// Writes `record` to a fresh file next to `path` and renames it over `path`.
///
/// # Errors
///
/// [`HostError::Io`] when the directory, file or rename fails.
pub fn write(path: &Path, record: &HostRecord) -> Result<(), HostError> {
    let bytes = serde_json::to_vec(record).map_err(|source| HostError::Io {
        operation: "encode the host record",
        source: source.into(),
    })?;
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory).map_err(io("create the state directory"))?;
    }
    let staging = path.with_extension(format!("json.{}", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging)
        .map_err(io("create the host record"))?;
    file.write_all(&bytes)
        .map_err(io("write the host record"))?;
    file.sync_all().map_err(io("sync the host record"))?;
    drop(file);
    std::fs::rename(&staging, path).map_err(io("publish the host record"))
}

/// Removes the record only while it still names `owner`; a newer holder's stays.
///
/// # Errors
///
/// The read or removal failure.
pub fn withdraw(path: &Path, owner: &BootProof) -> Result<(), HostError> {
    match read(path)? {
        Some(record) if record.endpoint.boot_proof == *owner => {}
        Some(_) | None => return Ok(()),
    }
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(source) => Err(HostError::Io {
            operation: "withdraw the host record",
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use ovis_terminal_protocol::domain::unix_service::BootProof;
    use ovis_terminal_protocol::domain::windows_service::{HostRecord, PipeName};
    use ovis_terminal_protocol::schema::ids::{ProcessPid, ServiceLabel, StartToken};

    use super::{read, withdraw, write};

    fn record(pid: u32) -> HostRecord {
        let pipe = PipeName::parse(r"\\.\pipe\p").unwrap();
        let proof = BootProof::new(ProcessPid::new(pid), StartToken::new(format!("t{pid}")));
        HostRecord::new(ServiceLabel::new("l".to_owned()), &pipe, proof)
    }

    #[test]
    fn a_record_is_replaced_whole_and_withdrawn_only_by_the_holder_it_names() {
        let root = std::env::temp_dir().join(format!("ovs-win-record-{}", std::process::id()));
        let path = root.join("state").join("holder-host.json");
        assert!(read(&path).unwrap().is_none());
        write(&path, &record(1)).unwrap();
        write(&path, &record(2)).unwrap();
        assert_eq!(read(&path).unwrap(), Some(record(2)));
        withdraw(&path, &record(1).endpoint.boot_proof).unwrap();
        assert!(path.exists(), "a former holder removed a newer record");
        withdraw(&path, &record(2).endpoint.boot_proof).unwrap();
        assert!(!path.exists());
        let left = std::fs::read_dir(root.join("state")).unwrap().count();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(left, 0, "staging files remain");
    }

    #[test]
    fn a_corrupt_record_is_an_error_and_never_absent() {
        let root = std::env::temp_dir().join(format!("ovs-win-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("holder-host.json");
        std::fs::write(&path, b"{\"format\":\"ovis-terminal-windows-host/1\"}").unwrap();
        let kind = read(&path).map(drop).map_err(|e| e.kind());
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(kind, Err("descriptor_malformed"));
    }
}
