//! Bounded, cancellable SHA-256 hashing while copying or reading a file.

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use ovis_terminal_protocol::domain::update::Sha256Digest;
use sha2::{Digest as _, Sha256};
use tokio::sync::watch;

use super::UpdateError;
use super::layout::io;

const CHUNK: usize = 64 * 1024;

/// The owning operation's cancellation request; a closed channel also counts.
#[derive(Debug, Clone)]
pub struct Interrupt {
    requested: Option<watch::Receiver<bool>>,
}

impl Interrupt {
    /// Follows a flag that becomes `true` when the operation is cancelled.
    #[must_use]
    pub const fn new(requested: watch::Receiver<bool>) -> Self {
        Self {
            requested: Some(requested),
        }
    }

    /// Never requested.
    #[must_use]
    pub const fn never() -> Self {
        Self { requested: None }
    }

    fn requested(&self) -> bool {
        self.requested
            .as_ref()
            .is_some_and(|flag| flag.has_changed().is_err() || *flag.borrow())
    }
}

fn digest_of(hasher: Sha256) -> Result<Sha256Digest, UpdateError> {
    let hex: String = hasher
        .finalize()
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
        .collect();
    Ok(hex.parse()?)
}

/// Hashes `source` into `sink`, refusing more than `limit` bytes.
fn pump(
    source: &mut impl Read,
    sink: &mut impl Write,
    (limit, interrupt): (u64, &Interrupt),
) -> Result<Sha256Digest, UpdateError> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; CHUNK];
    let mut total = 0_u64;
    loop {
        if interrupt.requested() {
            return Err(UpdateError::Cancelled);
        }
        let read = source.read(&mut buffer).map_err(io("read a release"))?;
        let Some(chunk) = buffer.get(..read).filter(|chunk| !chunk.is_empty()) else {
            return digest_of(hasher);
        };
        total = total.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        if total > limit {
            return Err(UpdateError::TooLarge { limit });
        }
        hasher.update(chunk);
        sink.write_all(chunk)
            .map_err(io("write a staged release"))?;
    }
}

/// Copies `from` to the new file `to` and returns the digest of the bytes copied.
pub(super) fn copy_hashed(
    from: &mut File,
    to: &Path,
    (limit, interrupt): (u64, &Interrupt),
) -> Result<Sha256Digest, UpdateError> {
    let mut file = File::create_new(to).map_err(io("create a staged release"))?;
    let digest = pump(from, &mut file, (limit, interrupt))?;
    file.sync_all().map_err(io("sync a staged release"))?;
    Ok(digest)
}

/// The digest of the file at `path`, read up to `limit` bytes.
pub(super) fn hash_file(path: &Path, limit: u64) -> Result<Sha256Digest, UpdateError> {
    let mut file = File::open(path).map_err(io("read a build file"))?;
    pump(
        &mut file,
        &mut std::io::sink(),
        (limit, &Interrupt::never()),
    )
}
