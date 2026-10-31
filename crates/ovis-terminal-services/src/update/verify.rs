//! `update.verify`: re-hash, extract safely, and check the manifest against the request
//! and this runtime before a build can be selected.

use std::path::Path;

use ovis_terminal_protocol::domain::update::{BuildManifest, RejectReason, UpdateRejection};
use ovis_terminal_protocol::schema::methods::update::{
    UpdateRecord, UpdateState, UpdateStepParams, VerifiedBuild,
};
use ovis_terminal_protocol::schema::scalar::OpenEnum;

use super::archive::{ArchiveError, extract};
use super::digest::hash_file;
use super::layout::{binary_name, io, remove_tree};
use super::{UpdateError, Updates};

enum Check {
    Reject(RejectReason),
    Failed(UpdateError),
}

impl From<UpdateError> for Check {
    fn from(error: UpdateError) -> Self {
        Self::Failed(error)
    }
}

fn unpacking(limit: u64) -> impl FnOnce(ArchiveError) -> Check {
    move |error| match error {
        ArchiveError::Unsafe => Check::Reject(RejectReason::UnsafeArchive),
        ArchiveError::TooLarge => Check::Failed(UpdateError::TooLarge { limit }),
        ArchiveError::Io(source) => Check::Failed(io("extract a release")(source)),
    }
}

fn manifest(dir: &Path) -> Result<BuildManifest, Check> {
    let bytes = match std::fs::read(dir.join("build.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(Check::Reject(RejectReason::MissingEntry));
        }
        Err(error) => return Err(Check::Failed(io("read build.json")(error))),
    };
    serde_json::from_slice(&bytes).map_err(|_| Check::Reject(RejectReason::ManifestMismatch))
}

#[cfg(unix)]
fn mark_executable(binary: &Path) -> Result<(), UpdateError> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(binary, std::fs::Permissions::from_mode(0o755))
        .map_err(io("mark the executable"))
}

/// Windows runs any `.exe` file; there is no mode bit to set.
#[cfg(not(unix))]
fn mark_executable(binary: &Path) -> Result<(), UpdateError> {
    let _: &Path = binary;
    Ok(())
}

impl Updates {
    fn check(&self, record: &UpdateRecord) -> Result<VerifiedBuild, Check> {
        let limit = self.config.limits.archive_bytes;
        let archive = self.layout.archive(&record.update_id);
        if hash_file(&archive, limit)? != record.sha256 {
            return Err(Check::Reject(RejectReason::DigestMismatch));
        }
        let dir = self.layout.build(&record.update_id);
        extract(&archive, &dir, limit).map_err(unpacking(limit))?;
        let manifest = manifest(&dir)?;
        let requirement = &self.config.requirement;
        if manifest.version != record.version || manifest.target != record.target {
            return Err(Check::Reject(RejectReason::ManifestMismatch));
        }
        if !manifest
            .protocol_generations
            .contains(&requirement.protocol)
        {
            return Err(Check::Reject(RejectReason::ProtocolIncompatible));
        }
        if !manifest.holder_generations.contains(&requirement.holder) {
            return Err(Check::Reject(RejectReason::HolderIncompatible));
        }
        let binary = dir.join(binary_name(&record.target));
        if !binary.is_file() {
            return Err(Check::Reject(RejectReason::MissingEntry));
        }
        mark_executable(&binary)?;
        Ok(VerifiedBuild::new(manifest, hash_file(&binary, limit)?))
    }

    /// Verifies a staged update; a rejection is recorded and leaves the active build alone.
    ///
    /// # Errors
    ///
    /// `no_staged_update`, `invalid_update_state` unless it is staged, `integrity_failed`
    /// or `incompatible` with the rejection recorded, or install-root I/O.
    pub fn verify(&self, params: &UpdateStepParams) -> Result<UpdateRecord, UpdateError> {
        let mut state = self.lock()?;
        let mut record = state.record(&params.update_id)?;
        if record.state != OpenEnum::Known(UpdateState::Staged) {
            return Err(UpdateError::InvalidState { step: "verify" });
        }
        remove_tree(&self.layout.build(&record.update_id))?;
        match self.check(&record) {
            Ok(build) => {
                record.state = OpenEnum::Known(UpdateState::Verified);
                record.build = Some(build);
                self.save(&mut state, &record)?;
                drop(state);
                Ok(record)
            }
            Err(Check::Reject(reason)) => {
                remove_tree(&self.layout.build(&record.update_id))?;
                record.state = OpenEnum::Known(UpdateState::Rejected);
                record.rejection = Some(UpdateRejection::new(reason, params.op_id.clone()));
                self.save(&mut state, &record)?;
                Err(UpdateError::Rejected(reason))
            }
            Err(Check::Failed(error)) => {
                remove_tree(&self.layout.build(&record.update_id))?;
                Err(error)
            }
        }
    }
}
