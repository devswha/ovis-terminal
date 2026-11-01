//! A POSIX ustar reader that extracts only plain files and directories below one root.
//!
//! Any entry that is absolute, climbs with `..`, repeats, is a link, device or extension
//! header, or whose header checksum fails, refuses the whole archive.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::{Component, Path, PathBuf};

const BLOCK: u64 = 512;
const MAX_ENTRIES: usize = 64;

/// Why extraction stopped.
#[derive(Debug, thiserror::Error)]
pub(super) enum ArchiveError {
    /// The archive holds an entry that could land outside the root, or is malformed.
    #[error("the archive holds an unsafe or malformed entry")]
    Unsafe,
    /// The files exceed the size bound.
    #[error("the archive's files exceed their size bound")]
    TooLarge,
    /// Reading the archive or writing a file failed.
    #[error("reading the archive or writing a file failed")]
    Io(#[from] io::Error),
}

enum Entry {
    File(PathBuf, u64),
    Directory(PathBuf),
    End,
}

fn text(header: &[u8; 512], start: usize, len: usize) -> &[u8] {
    let field = header.get(start..start.saturating_add(len)).unwrap_or(&[]);
    let end = field
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(field.len());
    field.get(..end).unwrap_or(&[])
}

fn octal(field: &[u8]) -> Result<u64, ArchiveError> {
    let digits = std::str::from_utf8(field)
        .map_err(|_| ArchiveError::Unsafe)?
        .trim_matches(|c: char| c == ' ' || c == '\0');
    u64::from_str_radix(digits, 8).map_err(|_| ArchiveError::Unsafe)
}

fn checksum_holds(header: &[u8; 512]) -> Result<bool, ArchiveError> {
    let recorded = octal(text(header, 148, 8))?;
    let sum = header.iter().enumerate().fold(0_u64, |sum, (index, byte)| {
        let byte = if (148..156).contains(&index) {
            b' '
        } else {
            *byte
        };
        sum.saturating_add(u64::from(byte))
    });
    Ok(sum == recorded)
}

fn relative(header: &[u8; 512]) -> Result<PathBuf, ArchiveError> {
    let name = std::str::from_utf8(text(header, 0, 100)).map_err(|_| ArchiveError::Unsafe)?;
    let prefix = if text(header, 257, 5) == b"ustar" {
        std::str::from_utf8(text(header, 345, 155)).map_err(|_| ArchiveError::Unsafe)?
    } else {
        ""
    };
    let joined = if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}/{name}")
    };
    let path = Path::new(joined.trim_end_matches('/'));
    let normal = path
        .components()
        .all(|component| matches!(component, Component::Normal(_)));
    if !normal || path.as_os_str().is_empty() || joined.contains('\\') {
        return Err(ArchiveError::Unsafe);
    }
    Ok(path.to_path_buf())
}

fn next(reader: &mut impl Read) -> Result<Entry, ArchiveError> {
    let mut header = [0_u8; 512];
    reader
        .read_exact(&mut header)
        .map_err(|_| ArchiveError::Unsafe)?;
    if header.iter().all(|byte| *byte == 0) {
        return Ok(Entry::End);
    }
    if !checksum_holds(&header)? {
        return Err(ArchiveError::Unsafe);
    }
    let path = relative(&header)?;
    match header.get(156) {
        Some(b'0' | 0) => Ok(Entry::File(path, octal(text(&header, 124, 12))?)),
        Some(b'5') => Ok(Entry::Directory(path)),
        _ => Err(ArchiveError::Unsafe),
    }
}

fn copy_entry(reader: &mut impl Read, target: &Path, size: u64) -> Result<(), ArchiveError> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = File::create_new(target).map_err(|error| match error.kind() {
        io::ErrorKind::AlreadyExists => ArchiveError::Unsafe,
        _ => ArchiveError::Io(error),
    })?;
    let copied = io::copy(&mut reader.take(size), &mut file)?;
    if copied != size {
        return Err(ArchiveError::Unsafe);
    }
    file.sync_all()?;
    let padding = size
        .checked_next_multiple_of(BLOCK)
        .and_then(|end| end.checked_sub(size))
        .ok_or(ArchiveError::Unsafe)?;
    io::copy(&mut reader.take(padding), &mut io::sink())?;
    Ok(())
}

/// Extracts `archive` into the new directory `root`, writing at most `limit` file bytes.
pub(super) fn extract(archive: &Path, root: &Path, limit: u64) -> Result<(), ArchiveError> {
    fs::create_dir(root)?;
    let mut reader = BufReader::new(File::open(archive)?);
    let mut seen = BTreeSet::new();
    let mut written = 0_u64;
    loop {
        let entry = next(&mut reader)?;
        let path = match &entry {
            Entry::End => return Ok(()),
            Entry::File(path, _) | Entry::Directory(path) => path.clone(),
        };
        if !seen.insert(path.clone()) || seen.len() > MAX_ENTRIES {
            return Err(ArchiveError::Unsafe);
        }
        match entry {
            Entry::File(_, size) => {
                written = written.checked_add(size).ok_or(ArchiveError::TooLarge)?;
                if written > limit {
                    return Err(ArchiveError::TooLarge);
                }
                copy_entry(&mut reader, &root.join(path), size)?;
            }
            Entry::Directory(_) => fs::create_dir_all(root.join(path))?,
            Entry::End => return Ok(()),
        }
    }
}
