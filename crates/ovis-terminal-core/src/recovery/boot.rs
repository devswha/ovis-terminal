//! Independent host-boot evidence: the operating system's own boot identity, compared
//! with the one the runtime recorded when it last started.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::RecoveryError;
use super::durable::replace;

/// The operating system's identity of the current boot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OsBootId(String);

impl OsBootId {
    /// Wraps a boot identity read elsewhere.
    #[must_use]
    pub const fn new(id: String) -> Self {
        Self(id)
    }

    /// The identity text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Reads the current boot identity from the operating system.
///
/// Linux reads the kernel's random boot id; macOS reads `kern.boottime`. Elsewhere no
/// independent evidence is available and the read fails, so survival stays unknown.
///
/// # Errors
///
/// [`RecoveryError::BootEvidence`] when the operating system record is unreadable.
pub fn read_os_boot() -> Result<OsBootId, RecoveryError> {
    os_boot().map(OsBootId).map_err(RecoveryError::BootEvidence)
}

#[cfg(target_os = "linux")]
fn os_boot() -> std::io::Result<String> {
    let id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    Ok(format!("linux:{}", id.trim()))
}

#[cfg(target_os = "macos")]
fn os_boot() -> std::io::Result<String> {
    let output = std::process::Command::new("/usr/sbin/sysctl")
        .args(["-n", "kern.boottime"])
        .env("LC_ALL", "C")
        .output()?;
    let text = String::from_utf8(output.stdout)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let stamp = text.split('}').next().unwrap_or_default().trim();
    if !output.status.success() || !stamp.contains("sec") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "kern.boottime has an unexpected layout",
        ));
    }
    Ok(format!("darwin:{stamp}}}"))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn os_boot() -> std::io::Result<String> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no independent boot identity is read on this platform",
    ))
}

/// What the boot evidence says about the host since the runtime last recorded it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BootVerdict {
    /// The recorded and current boot identities are equal.
    Same,
    /// They differ: every process of the recorded boot is gone.
    Rebooted,
    /// Either identity is missing or unreadable; nothing is concluded.
    Inconclusive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum StampFormat {
    #[serde(rename = "ovis-terminal-boot-stamp/1")]
    V1,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stamp {
    format: StampFormat,
    os_boot: OsBootId,
}

/// The boot identity recorded by the runtime's last start, in its state directory.
#[derive(Debug, Clone)]
pub struct BootStamp {
    path: PathBuf,
}

impl BootStamp {
    /// The stamp kept under `dir`.
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        Self {
            path: dir.join("boot-stamp.json"),
        }
    }

    /// Compares the recorded identity with `current`; a missing stamp or an unreadable
    /// current identity is inconclusive, never `Same`.
    ///
    /// # Errors
    ///
    /// A stamp that cannot be read or is not of the published shape; it is kept.
    pub fn compare(
        &self,
        current: Result<&OsBootId, &RecoveryError>,
    ) -> Result<BootVerdict, RecoveryError> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(BootVerdict::Inconclusive);
            }
            Err(source) => {
                return Err(RecoveryError::BootStampIo {
                    operation: "read",
                    source,
                });
            }
        };
        let stamp: Stamp =
            serde_json::from_slice(&bytes).map_err(RecoveryError::BootStampMalformed)?;
        Ok(match current {
            Ok(current) if *current == stamp.os_boot => BootVerdict::Same,
            Ok(_) => BootVerdict::Rebooted,
            Err(_) => BootVerdict::Inconclusive,
        })
    }

    /// Records `current` as the boot this runtime serves, atomically.
    ///
    /// # Errors
    ///
    /// [`RecoveryError::BootStampIo`] when the stamp cannot be made durable.
    pub fn record(&self, current: &OsBootId) -> Result<(), RecoveryError> {
        let stamp = Stamp {
            format: StampFormat::V1,
            os_boot: current.clone(),
        };
        let bytes = serde_json::to_vec(&stamp).map_err(RecoveryError::BootStampMalformed)?;
        replace(&self.path, &bytes)
            .map_err(|(operation, source)| RecoveryError::BootStampIo { operation, source })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("ovis-t49-boot-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&root).unwrap_or_default();
        root
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn the_os_boot_identity_is_stable_within_one_boot() -> Result<(), RecoveryError> {
        assert_eq!(read_os_boot()?, read_os_boot()?);
        Ok(())
    }

    #[test]
    fn a_missing_stamp_or_unreadable_boot_is_inconclusive_and_a_change_is_a_reboot()
    -> Result<(), RecoveryError> {
        let root = root();
        let stamp = BootStamp::new(&root);
        let before = OsBootId::new("linux:aaaa".to_owned());
        let after = OsBootId::new("linux:bbbb".to_owned());
        assert_eq!(stamp.compare(Ok(&before))?, BootVerdict::Inconclusive);
        stamp.record(&before)?;
        assert_eq!(stamp.compare(Ok(&before))?, BootVerdict::Same);
        assert_eq!(stamp.compare(Ok(&after))?, BootVerdict::Rebooted);
        let unreadable = RecoveryError::BootEvidence(std::io::ErrorKind::Unsupported.into());
        assert_eq!(stamp.compare(Err(&unreadable))?, BootVerdict::Inconclusive);
        std::fs::write(
            root.join("boot-stamp.json"),
            br#"{"format":"ovis-terminal-boot-stamp/2","os_boot":"x"}"#,
        )
        .unwrap_or_default();
        let refused = stamp.compare(Ok(&before)).map_err(|error| error.kind());
        assert_eq!(refused, Err("boot_stamp_malformed"));
        assert!(
            root.join("boot-stamp.json").exists(),
            "a refused stamp is kept"
        );
        std::fs::remove_dir_all(root).unwrap_or_default();
        Ok(())
    }
}
