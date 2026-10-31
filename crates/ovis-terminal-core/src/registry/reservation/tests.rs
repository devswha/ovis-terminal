use std::error::Error;
use std::sync::atomic::{AtomicU64, Ordering};

use ovis_terminal_store::{Store, StoreOptions};

use crate::registry::{IdentityReservation, TerminalId};

type TestResult<T> = Result<T, Box<dyn Error>>;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

#[tokio::test]
async fn reservation_is_durable_before_return_and_survives_store_reopen() -> TestResult<()> {
    let root = std::env::temp_dir().join(format!(
        "ovis-reservation-{}-{}",
        std::process::id(),
        NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
    ));
    let store = Store::open(&root, StoreOptions::default())?;
    let reservation = IdentityReservation::new(&store);
    let machine_id = reservation
        .load_or_reserve_machine(ovis_terminal_store::OpId::new("reserve-machine"))
        .await?;
    let first_id = reservation
        .reserve::<TerminalId>(ovis_terminal_store::OpId::new("reserve-first"))
        .await?;
    let snapshot = store.snapshot()?;
    assert!(reservation.is_reserved(&snapshot, first_id)?);
    drop(snapshot);
    store.close()?;

    let reopened = Store::open(&root, StoreOptions::default())?;
    let restored_machine = IdentityReservation::new(&reopened)
        .load_or_reserve_machine(ovis_terminal_store::OpId::new("reload-machine"))
        .await?;
    let second_id = IdentityReservation::new(&reopened)
        .reserve::<TerminalId>(ovis_terminal_store::OpId::new("reserve-second"))
        .await?;
    assert_eq!(restored_machine, machine_id);
    assert_ne!(first_id, second_id);
    reopened.close()?;
    std::fs::remove_dir_all(root)?;
    Ok(())
}
