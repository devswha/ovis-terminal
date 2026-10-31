//! A database written by schema version 1 opens under the current build, unchanged.

use std::error::Error;
use std::path::PathBuf;

use rusqlite::Connection;

use super::super::quarantine::DB_FILE;
use super::{MIGRATIONS, Probe, SCHEMA_VERSION, probe};
use crate::{Payload, ResourceKey, ResourceLookup, Revision, Store, StoreOptions, Version};

struct Root(PathBuf);

impl Drop for Root {
    fn drop(&mut self) {
        let removed = std::fs::remove_dir_all(&self.0);
        assert!(removed.is_ok(), "cleanup: {removed:?}");
    }
}

/// Writes a version-1 database holding one resource, as a pre-v2 build left it.
fn version_one(root: &Root) -> Result<PathBuf, Box<dyn Error>> {
    std::fs::create_dir_all(&root.0)?;
    let db = root.0.join(DB_FILE);
    let mut connection = Connection::open(&db)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    let transaction = connection.transaction()?;
    transaction.execute_batch(MIGRATIONS.first().ok_or("migration 0")?)?;
    transaction.execute(
        "INSERT INTO resources (kind, id, version, value, revision) VALUES (?1, ?2, 1, ?3, 1)",
        ("operation", "rt/row", br#"{"params":[1,2,3]}"#.as_slice()),
    )?;
    transaction.execute("UPDATE head SET revision = 1 WHERE id = 0", ())?;
    transaction.pragma_update(None, "user_version", 1)?;
    transaction.commit()?;
    connection.close().map_err(|(_, error)| error)?;
    Ok(db)
}

#[test]
fn a_version_one_database_migrates_to_the_current_version_with_its_rows()
-> Result<(), Box<dyn Error>> {
    // Given: a database left by a build whose newest schema version was 1.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let unique = format!("ovis-store-v1-{}-{nanos}", std::process::id());
    let root = Root(std::env::temp_dir().join(unique));
    let db = version_one(&root)?;
    assert!(matches!(probe(&db)?, Probe::Ready { version: 1 }));

    // When: the current build opens it.
    let store = Store::open(&root.0, StoreOptions::default())?;
    let row = store
        .snapshot()?
        .resource(&ResourceKey::new("operation", "rt/row"))?;
    store.close()?;

    // Then: the row is unchanged and the file now reports the current version, which a
    // version-1 build classifies as newer and refuses.
    assert_eq!(
        row,
        ResourceLookup::Live {
            version: Version::new(1),
            value: Payload::from(r#"{"params":[1,2,3]}"#),
            revision: Revision::new(1),
        }
    );
    assert_eq!(SCHEMA_VERSION, 2);
    assert!(matches!(probe(&db)?, Probe::Ready { version: 2 }));
    Ok(())
}
