//! A mirrored record keeps its upstream position apart from its local sequence, and a
//! replayed or reordered upstream position is refused before any local sequence exists.

use std::sync::mpsc;

use ovis_terminal_core::commit::{Boundary, CommitError, CommitSource, Committed};
use ovis_terminal_core::events::{RemoteOrigin, RemoteSeq};
use ovis_terminal_protocol::schema::scalar::EventSeq;

use super::*;

fn remote(runtime: u64, upstream: u64) -> Result<CommitSource, Box<dyn Error>> {
    Ok(CommitSource::Remote {
        origin: RemoteOrigin::new(
            MachineId::new(Uuid::from_u128(2))?,
            RuntimeId::try_from(runtime)?,
            RemoteSeq::new(EventSeq::new(upstream)),
        ),
    })
}

fn mirror(
    boundary: &Boundary<Probe, Producer>,
    (runtime, upstream): (u64, u64),
) -> Result<Result<Committed, CommitError>, Box<dyn Error>> {
    let record = wrote("remote", upstream)?;
    Ok(boundary.commit(remote(runtime, upstream)?, |_| Ok(record)))
}

#[test]
fn replayed_or_reordered_upstream_positions_are_refused() -> TestResult {
    // Given an observer and one mirrored record at upstream sequence 5.
    let fixture = Fixture::new();
    let core = fixture.core((StoreOptions::default(), Producer::default()))?;
    let observer = core.subscribe(None)?;
    let boundary = Boundary::new(Arc::clone(&core), 1)?;
    let first = mirror(&boundary, (7, 5))??;
    assert_eq!(first.local_seq(), EventSeq::new(1));
    assert_eq!(first.remote().map(|origin| origin.seq.get()), Some(5));
    // When the same position is replayed, and an older one arrives late.
    for upstream in [5, 4] {
        let refused = mirror(&boundary, (7, upstream))?.expect_err("not after the mark");
        assert_eq!(refused.kind(), "remote_replayed");
        assert!(matches!(
            refused,
            CommitError::RemoteReplayed { upstream: offered, last }
                if offered.get() == upstream && last.get() == 5
        ));
    }
    // Then nothing was committed or published for them.
    assert_eq!(core.read_snapshot()?.cursor.seq, EventSeq::new(1));
    let published = observer
        .delivery
        .recv_timeout(std::time::Duration::from_secs(5))?;
    assert_eq!(published.cursor.seq, EventSeq::new(1));
    assert!(matches!(
        observer.delivery.try_recv(),
        Err(EventError::TryReceive(mpsc::TryRecvError::Empty))
    ));
    // And a later position (a gap the mirror reports itself) and another lifetime commit.
    let later = mirror(&boundary, (7, 9))??;
    assert_eq!(
        (later.local_seq().get(), later.remote().map(|o| o.seq.get())),
        (2, Some(9))
    );
    let other = mirror(&boundary, (8, 1))??;
    assert_eq!(other.local_seq(), EventSeq::new(3));
    drop((observer, boundary));
    close(core)
}

#[test]
fn upstream_positions_round_trip_as_decimal_strings() -> TestResult {
    // Given a mirrored record at the largest representable upstream sequence.
    let fixture = Fixture::new();
    let core = fixture.core((StoreOptions::default(), Producer::default()))?;
    let boundary = Boundary::new(Arc::clone(&core), 1)?;
    let committed = mirror(&boundary, (7, u64::MAX))??;
    // When it crosses the JSON boundary.
    let wire = serde_json::to_value(&committed)?;
    // Then both sequences are canonical decimal strings in distinct fields.
    let field = |path: &str| wire.pointer(path).and_then(serde_json::Value::as_str);
    assert_eq!(field("/source/origin/seq"), Some("18446744073709551615"));
    assert_eq!(field("/cursor/seq"), Some("1"));
    assert_eq!(
        serde_json::from_value::<Committed>(wire.clone())?,
        committed
    );
    let mut numeric = wire;
    *numeric.pointer_mut("/source/origin/seq").ok_or("seq")? = serde_json::json!(5);
    assert!(serde_json::from_value::<Committed>(numeric).is_err());
    // And nothing can follow the maximum on that lifetime.
    let refused = mirror(&boundary, (7, u64::MAX))?.expect_err("already at the maximum");
    assert_eq!(refused.kind(), "remote_replayed");
    drop(boundary);
    close(core)
}

#[test]
fn an_unreadable_upstream_mark_is_never_reset_to_a_default() -> TestResult {
    // Given a stored high-water mark that is not a decimal sequence.
    let fixture = Fixture::new();
    let core = fixture.core((StoreOptions::default(), Producer::default()))?;
    let boundary = Boundary::new(Arc::clone(&core), 1)?;
    let key = format!(
        "{}/{}",
        MachineId::new(Uuid::from_u128(2))?,
        RuntimeId::try_from(7)?
    );
    let corrupt = wrote("registry", 1)?.write(CasWrite::new(
        ResourceKey::new("commit.remote", key),
        Version::ABSENT,
        Some(Payload::from("not-a-sequence")),
    ));
    boundary.commit(CommitSource::Registry, |_| Ok(corrupt))?;
    // When a mirrored record arrives for that lifetime.
    let refused = mirror(&boundary, (7, 1))?.expect_err("unreadable mark");
    // Then it is refused as an invalid record and the sequence does not advance.
    assert_eq!(refused.kind(), "invalid_event");
    assert_eq!(core.read_snapshot()?.cursor.seq, EventSeq::new(1));
    drop(boundary);
    close(core)
}
