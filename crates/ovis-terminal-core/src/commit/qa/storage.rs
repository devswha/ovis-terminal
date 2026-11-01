//! A full store acknowledges and publishes nothing, and consumes no sequence.

use std::sync::mpsc;
use std::time::Duration;

use ovis_terminal_core::commit::{Boundary, CommitSource};
use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_store::{ReceiptLookup, StoreLimits};

use super::*;

const RECORD: usize = 16 << 10;

#[test]
fn a_full_store_acknowledges_and_publishes_nothing() -> TestResult {
    // Given a store capped at 64 pages and an observer.
    let fixture = Fixture::new();
    let capped = StoreOptions::default().with_limits(StoreLimits::default().with_max_pages(64));
    let core = fixture.core((capped, Producer::default()))?;
    let observer = core.subscribe(None)?;
    let boundary = Boundary::new(Arc::clone(&core), 1)?;
    // When records carrying 16 KiB of state commit until the store is full.
    let mut acknowledged = 0_u64;
    let refused = loop {
        let op = OpId::new(Uuid::now_v7().to_string());
        let record = with_state(
            full_record(&op, acknowledged)?,
            (&acknowledged.to_string(), RECORD),
        );
        match boundary.commit(CommitSource::Registry, |_| Ok(record)) {
            Ok(committed) => {
                acknowledged += 1;
                assert_eq!(committed.local_seq(), EventSeq::new(acknowledged));
                assert!(
                    acknowledged < 64,
                    "a 64-page store cannot hold 64 such records"
                );
            }
            Err(error) => break (error, op),
        }
    };
    // Then the refusal is typed, and its operation left no receipt or event.
    assert!(acknowledged > 0);
    assert_eq!(
        (refused.0.kind(), refused.0.hint().is_empty()),
        ("storage_full", false)
    );
    let view = core.read_snapshot()?;
    assert_eq!(view.cursor.seq.get(), acknowledged);
    assert_eq!(view.sections.seqs.len(), usize::try_from(acknowledged)?);
    for expected in 1..=acknowledged {
        let event = observer.delivery.recv_timeout(Duration::from_secs(5))?;
        assert_eq!(event.cursor.seq.get(), expected);
    }
    assert!(matches!(
        observer.delivery.try_recv(),
        Err(EventError::TryReceive(mpsc::TryRecvError::Empty))
    ));
    drop((observer, boundary));
    close(core)?;
    // And with space again, the next commit takes the very next sequence.
    let store = fixture.open(StoreOptions::default())?;
    assert!(matches!(
        store.snapshot()?.receipt(&refused.1)?,
        ReceiptLookup::Absent
    ));
    let core = Arc::new(Coordinator::new(store, identity()?, Producer::default()));
    let boundary = Boundary::new(Arc::clone(&core), 1)?;
    let record = wrote("terminal", acknowledged)?;
    let next = boundary.commit(CommitSource::Registry, |_| Ok(record))?;
    assert_eq!(next.local_seq(), EventSeq::new(acknowledged + 1));
    drop(boundary);
    close(core)
}

fn full_record(op: &OpId, n: u64) -> Result<Commit<Probe>, Box<dyn Error>> {
    Ok(Commit::new(
        op.clone(),
        Probe::Wrote {
            source: "terminal".to_owned(),
            n,
        },
        vec![ResourceId::Terminal(TerminalId::try_from(1)?)],
    ))
}
