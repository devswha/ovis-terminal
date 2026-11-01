//! The boot stamp: a readable operating-system boot is recorded, an unreadable one is a
//! typed start-up warning and leaves no stamp behind.

use std::sync::atomic::{AtomicU32, Ordering};

use ovis_terminal_core::recovery::{RecoveryError, RecoveryPlan, read_os_boot};
use ovis_terminal_protocol::schema::ids::{RuntimeLifetimeId, ServerBootId};
use ovis_terminal_vt::{Geometry, HistoryLimit, VtConfig};

use super::stamp_boot;

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct Scratch(std::path::PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        match std::fs::remove_dir_all(&self.0) {
            Ok(()) | Err(_) => {}
        }
    }
}

fn plan() -> Result<(RecoveryPlan, Scratch), Box<dyn std::error::Error>> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let unique = NEXT.fetch_add(1, Ordering::Relaxed);
    let state = std::env::temp_dir().join(format!("ovt31r-{}-{unique}", std::process::id()));
    std::fs::create_dir_all(state.join("recovery"))?;
    let vt = VtConfig::new(Geometry::new(80, 24)?, HistoryLimit::new(100));
    let ids = (
        RuntimeLifetimeId::new("runtime:1".to_owned()),
        ServerBootId::new("boot:1".to_owned()),
    );
    Ok((RecoveryPlan::new(state.clone(), ids, vt), Scratch(state)))
}

fn stamped(scratch: &Scratch) -> bool {
    std::fs::read_dir(scratch.0.join("recovery")).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().contains("stamp"))
    })
}

#[test]
fn an_unreadable_operating_system_boot_is_a_typed_warning_and_stamps_nothing() -> TestResult {
    let (plan, scratch) = plan()?;
    let unreadable = Err(RecoveryError::BootEvidence(std::io::Error::other(
        "no boot record",
    )));
    let warning = stamp_boot(&plan, unreadable)?.ok_or("the failure was swallowed")?;
    let reported = serde_json::to_value(&warning)?;
    assert_eq!(
        reported.get("kind").and_then(serde_json::Value::as_str),
        Some(RecoveryError::BootEvidence(std::io::Error::other("x")).kind())
    );
    assert_eq!(
        reported.get("stage").and_then(serde_json::Value::as_str),
        Some("stamp the operating-system boot")
    );
    assert!(!stamped(&scratch), "an unreadable boot left a stamp");
    Ok(())
}

#[test]
fn a_readable_operating_system_boot_is_stamped_without_a_warning() -> TestResult {
    let Ok(current) = read_os_boot() else {
        return Err("this host exposes no operating-system boot record".into());
    };
    let (plan, scratch) = plan()?;
    assert_eq!(stamp_boot(&plan, Ok(current))?, None);
    assert!(stamped(&scratch), "a readable boot was not stamped");
    Ok(())
}
