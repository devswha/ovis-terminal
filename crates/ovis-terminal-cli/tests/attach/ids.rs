//! Client-minted operation identities for the attach tests.

use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{NEXT, TestResult};

/// A version-7 UUID operation id, unique within this test process.
pub(super) fn op_id() -> TestResult<String> {
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let unique = u128::from(NEXT.fetch_add(1, Ordering::Relaxed));
    let id = (millis << 80)
        | (0x7 << 76)
        | (0b10 << 62)
        | (u128::from(std::process::id()) << 16)
        | unique;
    let hex = format!("{id:032x}");
    let part = |from: usize, to: usize| hex.get(from..to).unwrap_or_default().to_owned();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        part(0, 8),
        part(8, 12),
        part(12, 16),
        part(16, 20),
        part(20, 32)
    ))
}
