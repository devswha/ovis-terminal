//! Hand-built release archives for the adversarial fixtures, and digests.
//!
//! `tar` refuses to write a `..` member name, so the traversal fixture writes its ustar
//! headers itself; `gzip -n` compresses them like the release stage's archives.

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use sha2::{Digest as _, Sha256};

use super::error::InstallProbeError;

const BLOCK: usize = 512;

/// One regular file of an archive.
pub(super) struct Member<'a> {
    pub(super) name: &'a str,
    pub(super) mode: u32,
    pub(super) bytes: &'a [u8],
}

/// Lowercase hex SHA-256 of `bytes`.
pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            // Writing to a String cannot fail.
            if write!(hex, "{byte:02x}").is_err() {
                tracing::debug!("formatting a digest byte failed");
            }
            hex
        })
}

/// Lowercase hex SHA-256 of the file at `path`.
pub(super) fn sha256_file(path: &Path) -> Result<String, InstallProbeError> {
    let bytes = std::fs::read(path).map_err(InstallProbeError::io("read", path))?;
    Ok(sha256_hex(&bytes))
}

fn octal(field: &mut [u8], value: u64) {
    let width = field.len().saturating_sub(1);
    let text = format!("{value:0width$o}");
    let digits = text.as_bytes();
    let start = digits.len().saturating_sub(width);
    if let (Some(target), Some(source)) = (field.get_mut(..width), digits.get(start..)) {
        target.copy_from_slice(source);
    }
}

fn put(header: &mut [u8; BLOCK], at: std::ops::Range<usize>, bytes: &[u8]) {
    if let Some(field) = header.get_mut(at.start..at.start.saturating_add(bytes.len()).min(at.end))
    {
        let len = field.len();
        field.copy_from_slice(bytes.get(..len).unwrap_or_default());
    }
}

fn header(member: &Member<'_>) -> Result<[u8; BLOCK], InstallProbeError> {
    let size = u64::try_from(member.bytes.len()).map_err(|_| InstallProbeError::Usage {
        detail: format!("{} is too large for a fixture archive", member.name),
    })?;
    if member.name.len() > 99 {
        return Err(InstallProbeError::Usage {
            detail: format!("{} is longer than a ustar name", member.name),
        });
    }
    let mut header = [0_u8; BLOCK];
    put(&mut header, 0..100, member.name.as_bytes());
    if let Some(field) = header.get_mut(100..108) {
        octal(field, u64::from(member.mode));
    }
    for range in [108..116, 116..124] {
        if let Some(field) = header.get_mut(range) {
            octal(field, 0);
        }
    }
    if let Some(field) = header.get_mut(124..136) {
        octal(field, size);
    }
    if let Some(field) = header.get_mut(136..148) {
        octal(field, 0);
    }
    put(&mut header, 148..156, b"        ");
    put(&mut header, 156..157, b"0");
    put(&mut header, 257..265, b"ustar\x0000");
    let sum: u64 = header.iter().map(|byte| u64::from(*byte)).sum();
    if let Some(field) = header.get_mut(148..155) {
        octal(field, sum);
    }
    Ok(header)
}

/// An uncompressed ustar stream of `members`.
pub(super) fn ustar(members: &[Member<'_>]) -> Result<Vec<u8>, InstallProbeError> {
    let mut out = Vec::new();
    for member in members {
        out.extend_from_slice(&header(member)?);
        out.extend_from_slice(member.bytes);
        let tail = member.bytes.len().checked_rem(BLOCK).unwrap_or(0);
        let pad = if tail == 0 {
            0
        } else {
            BLOCK.saturating_sub(tail)
        };
        out.resize(out.len().saturating_add(pad), 0);
    }
    out.resize(out.len().saturating_add(BLOCK.saturating_mul(2)), 0);
    Ok(out)
}

/// `gzip -n` of `bytes`, written to `path`.
pub(super) fn gzip_to(bytes: &[u8], path: &Path) -> Result<(), InstallProbeError> {
    let file = std::fs::File::create(path).map_err(InstallProbeError::io("create", path))?;
    let spawn = |source| InstallProbeError::Spawn {
        program: "gzip -n -c".to_owned(),
        source,
    };
    let mut child = Command::new("gzip")
        .args(["-n", "-c"])
        .stdin(Stdio::piped())
        .stdout(file)
        .spawn()
        .map_err(spawn)?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(bytes).map_err(spawn)?;
    }
    let status = child.wait().map_err(spawn)?;
    if status.success() {
        Ok(())
    } else {
        Err(spawn(std::io::Error::other(format!(
            "gzip exited {status}"
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::{Member, sha256_hex, ustar};

    #[test]
    fn a_ustar_header_carries_a_valid_checksum_and_a_dotdot_name_verbatim() {
        let bytes = ustar(&[Member {
            name: "a/../../escaped",
            mode: 0o644,
            bytes: b"x",
        }]);
        let bytes = bytes.unwrap_or_default();
        assert_eq!(bytes.len(), 512 * 4);
        let header = bytes.get(..512).unwrap_or_default();
        assert!(header.starts_with(b"a/../../escaped\0"));
        let stored = std::str::from_utf8(header.get(148..154).unwrap_or_default()).unwrap_or("");
        let mut blanked = header.to_vec();
        if let Some(field) = blanked.get_mut(148..156) {
            field.copy_from_slice(b"        ");
        }
        let sum: u64 = blanked.iter().map(|byte| u64::from(*byte)).sum();
        assert_eq!(u64::from_str_radix(stored, 8).ok(), Some(sum));
    }

    #[test]
    fn sha256_hex_matches_the_published_empty_digest() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
