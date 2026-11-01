//! Damaged, unsupported and stale material is kept and reported, never replaced by a
//! healthy-looking default; incident evidence survives every read.

use std::path::{Path, PathBuf};

use ovis_terminal_holder::protocol::{TerminalId, TerminalIncarnation, TerminalTarget};
use ovis_terminal_protocol::domain::recovery::IncidentKind;
use ovis_terminal_protocol::schema::ids::{ServerBootId, TerminalId as WireTerminalId};
use ovis_terminal_store::{Store, StoreOptions};
use ovis_terminal_vt::{Checkpoint, Geometry, HistoryLimit, Vt, VtConfig};

use crate::recovery::{CheckpointStore, Finding, Incidents, RecoveryError};

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct Dir(PathBuf);

impl Dir {
    fn new() -> Result<Self, std::io::Error> {
        let root = std::env::temp_dir().join(format!("ovis-t49-durable-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&root)?;
        Ok(Self(root))
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let removed = std::fs::remove_dir_all(&self.0);
        assert!(removed.is_ok(), "remove owned directory: {removed:?}");
    }
}

fn target(incarnation: u64) -> TerminalTarget {
    TerminalTarget {
        terminal_id: TerminalId::new("terminal:7".to_owned()),
        terminal_incarnation: TerminalIncarnation::new(incarnation),
        process_incarnation: None,
    }
}

fn checkpoint(bytes: &[u8]) -> Result<Checkpoint, Box<dyn std::error::Error>> {
    let mut vt = Vt::new(VtConfig::new(Geometry::new(20, 4)?, HistoryLimit::new(10)));
    vt.feed(bytes);
    Ok(vt.export_checkpoint())
}

fn only_file(dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut files = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file());
    let file = files.next().ok_or("no checkpoint file")?;
    assert!(files.next().is_none(), "one checkpoint file expected");
    Ok(file)
}

#[test]
fn a_flipped_byte_is_quarantined_and_reported_never_imported() -> TestResult {
    let dir = Dir::new()?;
    let store = CheckpointStore::new(&dir.0);
    store.save(&target(1), &checkpoint(b"precious screen")?)?;
    let file = only_file(&dir.0)?;
    let mut bytes = std::fs::read(&file)?;
    let last = bytes.last_mut().ok_or("empty checkpoint")?;
    *last ^= 0x01;
    std::fs::write(&file, &bytes)?;

    let loaded = store.load(&target(1));

    let Err(RecoveryError::CheckpointCorrupted { quarantine }) = loaded else {
        return Err(format!("expected checkpoint_corrupted, got {loaded:?}").into());
    };
    assert_eq!(
        std::fs::read(&quarantine)?,
        bytes,
        "damaged bytes are kept verbatim"
    );
    assert!(
        !file.exists(),
        "the damaged file no longer poses as the checkpoint"
    );
    assert!(store.load(&target(1))?.is_none());
    Ok(())
}

#[test]
fn another_format_is_unsupported_and_kept_and_another_incarnation_is_refused() -> TestResult {
    let dir = Dir::new()?;
    let store = CheckpointStore::new(&dir.0);
    store.save(&target(1), &checkpoint(b"x")?)?;
    let file = only_file(&dir.0)?;
    let text = std::fs::read(&file)?;
    let newer = String::from_utf8_lossy(&text).replacen("checkpoint/1", "checkpoint/2", 1);
    std::fs::write(&file, newer.as_bytes())?;

    let unsupported = store.load(&target(1)).map(drop).map_err(|e| e.kind());
    assert_eq!(unsupported, Err("checkpoint_unsupported"));

    store.save(&target(2), &checkpoint(b"y")?)?;
    let moved = dir.0.join("terminal_7.2.ckpt");
    std::fs::rename(&moved, dir.0.join("terminal_7.3.ckpt"))?;
    let foreign = store.load(&target(3)).map(drop).map_err(|e| e.kind());
    assert_eq!(foreign, Err("checkpoint_target_mismatch"));
    Ok(())
}

#[test]
fn an_older_checkpoint_never_replaces_a_newer_one() -> TestResult {
    let dir = Dir::new()?;
    let store = CheckpointStore::new(&dir.0);
    let newer = checkpoint(b"0123456789")?;
    store.save(&target(1), &newer)?;
    let refused = store.save(&target(1), &checkpoint(b"0123")?);
    assert!(
        matches!(
            refused,
            Err(RecoveryError::CheckpointStale {
                stored: 10,
                offered: 4
            })
        ),
        "{refused:?}"
    );
    assert_eq!(store.load(&target(1))?, Some(newer));
    Ok(())
}

#[test]
fn a_staged_file_left_by_a_crash_mid_write_is_never_read_as_the_checkpoint() -> TestResult {
    let dir = Dir::new()?;
    let store = CheckpointStore::new(&dir.0);
    let kept = checkpoint(b"committed")?;
    store.save(&target(1), &kept)?;
    std::fs::write(dir.0.join("terminal_7.1.staged"), b"torn half write")?;
    assert_eq!(store.load(&target(1))?, Some(kept));
    Ok(())
}

#[test]
fn incidents_survive_every_read_and_a_damaged_journal_is_refused_unchanged() -> TestResult {
    let dir = Dir::new()?;
    let journal = Incidents::new(&dir.0);
    let boot = ServerBootId::new("server-boot:2".to_owned());
    let affected = vec![WireTerminalId::new("terminal:7".to_owned())];
    let first = journal.record(&boot, Finding::new(IncidentKind::CoreCrash, affected, None))?;
    let second = journal.record(
        &boot,
        Finding::new(
            IncidentKind::CheckpointCorrupted,
            Vec::new(),
            Some(dir.0.clone()),
        ),
    )?;
    assert!(first.incident_id < second.incident_id);

    let read = journal.list()?;
    assert_eq!(read, vec![first, second]);
    assert_eq!(
        Incidents::new(&dir.0).list()?,
        read,
        "a read clears nothing"
    );

    let path = dir.0.join("incidents.jsonl");
    let mut bytes = std::fs::read(&path)?;
    bytes.extend_from_slice(b"{\"not\":\"an incident\"}\n");
    std::fs::write(&path, &bytes)?;
    let refused = journal.list();
    assert!(
        matches!(refused, Err(RecoveryError::JournalCorrupt { line: 3, .. })),
        "{refused:?}"
    );
    assert_eq!(std::fs::read(&path)?, bytes, "the journal is kept exactly");
    Ok(())
}

#[test]
fn a_corrupt_database_is_quarantined_with_its_wal_and_recorded_as_an_incident() -> TestResult {
    let dir = Dir::new()?;
    let root = dir.0.join("terminals");
    std::fs::create_dir_all(&root)?;
    std::fs::write(
        root.join("state.db"),
        b"this is not a sqlite database at all",
    )?;
    std::fs::write(root.join("state.db-wal"), b"wal bytes")?;

    let opened = Store::open(&root, StoreOptions::default());

    let Err(error) = opened else {
        return Err("a corrupt database must not open as an empty store".into());
    };
    assert_eq!(error.kind(), "store_corrupted");
    let finding = Finding::of_store(&error).ok_or("store evidence")?;
    assert_eq!(finding.kind, IncidentKind::StoreCorrupted);
    let location = finding.quarantine.clone().ok_or("quarantine location")?;
    let kept: Vec<String> = std::fs::read_dir(&location)?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(kept.iter().any(|name| name == "state.db"), "{kept:?}");
    assert!(kept.iter().any(|name| name == "state.db-wal"), "{kept:?}");
    let journal = Incidents::new(&dir.0);
    let incident = journal.record(&ServerBootId::new("server-boot:3".to_owned()), finding)?;
    assert_eq!(journal.list()?, vec![incident]);
    Ok(())
}
