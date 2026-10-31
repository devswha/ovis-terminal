//! Commit barriers and exact receipt counters through the shared snapshot path.

use std::sync::{Arc, Barrier, mpsc};
use std::time::Duration;

use ovis_terminal_store::CommitBarrier;
use proptest::prelude::*;

use super::{
    Coordinator, Fixture, OpId, OperationProducer, Operations, Store, StoreOptions, TestResult,
    context, identity, json, request,
};

#[test]
fn inspection_enforces_the_snapshot_lease_byte_budget() -> TestResult {
    // Given a coordinator whose pinned-snapshot budget cannot hold its typed representation.
    let fixture = Fixture::new();
    let identity = identity(1)?;
    let mut limits = ovis_terminal_core::events::Limits::default();
    limits.snapshot_bytes = 1;
    let core: super::Core = Coordinator::new(
        Store::open(&fixture.0, StoreOptions::default())?,
        identity,
        OperationProducer::new(identity.runtime_id()),
    )
    .with_limits(limits)?;

    // When read-only operation inspection uses its production accessor.
    let error = core
        .read_snapshot()
        .expect_err("snapshot lease budget must apply");

    // Then the lease/paging path rejects before exposing a direct store capture.
    assert_eq!(error.kind(), "limit_exceeded");
    core.close()?;
    Ok(())
}

#[test]
fn inspection_requires_an_available_snapshot_lease() -> TestResult {
    // Given a coordinator whose only pinned-snapshot slot is occupied.
    let fixture = Fixture::new();
    let identity = identity(1)?;
    let mut limits = ovis_terminal_core::events::Limits::default();
    limits.pinned_snapshots = 1;
    let core: super::Core = Coordinator::new(
        Store::open(&fixture.0, StoreOptions::default())?,
        identity,
        OperationProducer::new(identity.runtime_id()),
    )
    .with_limits(limits)?;
    let lease = core.pin_snapshot()?;

    // When inspection runs before and after releasing the sole lease.
    let blocked = core.read_snapshot();
    core.release_snapshot(&lease.token)?;
    let released = core.read_snapshot();
    core.close()?;

    // Then the accessor consumes real lease capacity and succeeds once capacity returns.
    assert_eq!(
        blocked
            .expect_err("inspection must reserve a real snapshot lease")
            .kind(),
        "limit_exceeded"
    );
    assert!(released.is_ok());
    Ok(())
}

#[test]
fn inspection_exposes_only_a_committed_revision() -> TestResult {
    // Given a writer suspended after its rows change but before COMMIT.
    let fixture = Fixture::new();
    let release = Arc::new(Barrier::new(2));
    let writer_release = Arc::clone(&release);
    let (applied, reached) = mpsc::sync_channel(1);
    let options = StoreOptions::default().with_barrier(move |point| {
        if matches!(point, CommitBarrier::Applied) {
            applied.send(()).expect("barrier observer");
            writer_release.wait();
        }
    });
    let core: super::Core = Coordinator::new(
        Store::open(&fixture.0, options)?,
        identity(1)?,
        OperationProducer::new(identity(1)?.runtime_id()),
    );
    let req = request("barrier")?;
    let subscription = core.subscribe(None)?;
    assert_eq!(core.read_snapshot()?.cursor.seq.get(), 0);

    // When inspection races the uncommitted acceptance.
    std::thread::scope(|scope| -> TestResult {
        let writer =
            scope.spawn(|| Operations::new(&core, context(1).expect("context")).accept(&req));
        reached.recv_timeout(Duration::from_secs(30))?;
        let (started, begin) = mpsc::sync_channel(1);
        let (seen, observed) = mpsc::sync_channel(1);
        let reader_core = &core;
        let reader = scope.spawn(move || {
            started.send(()).expect("reader started");
            let snapshot = reader_core.read_snapshot();
            seen.send(snapshot).expect("reader result");
        });
        begin.recv_timeout(Duration::from_secs(30))?;
        assert!(matches!(
            observed.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        release.wait();

        // Then every visible row and event belongs to the completed transaction.
        let receipt = writer.join().expect("writer joined")?;
        let snapshot = observed.recv_timeout(Duration::from_secs(30))??;
        reader.join().expect("reader joined");
        assert_eq!(snapshot.cursor.seq, receipt.updated_seq);
        let event = subscription
            .delivery
            .recv_timeout(Duration::from_secs(30))?;
        assert_eq!(event.cursor.seq, event.data.operation.updated_seq);
        assert_eq!(event.data.operation, receipt);
        Ok(())
    })?;
    drop(subscription);
    core.close()?;
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]
    #[test]
    fn receipt_round_trip_preserves_arbitrary_decimal_sequences(sequence in any::<u64>()) {
        // Given a retained record encoded with an arbitrary 64-bit revision.
        let mut record = ovis_terminal_protocol::schema::methods::server::OpRecord::accepted(
            OpId::new("000005f5-e100-7000-8000-000000000001".into()),
            ovis_terminal_protocol::schema::ids::MethodName::new("terminal.run".into()),
        );
        record.updated_seq = ovis_terminal_protocol::schema::scalar::EventSeq::new(sequence);
        // When it crosses JSON encoding.
        let wire = serde_json::to_value(&record).expect("encode");
        let round_trip = serde_json::from_value::<ovis_terminal_protocol::schema::methods::server::OpRecord>(wire.clone()).expect("decode");
        // Then no precision or identity is lost.
        prop_assert_eq!(wire.get("updated_seq"), Some(&json!(sequence.to_string())));
        prop_assert_eq!(round_trip, record);
    }

    #[test]
    fn store_round_trip_preserves_canonical_retry(argv in "[a-z]{1,16}") {
        // Given an actual durable receipt for generated parameters.
        let fixture = Fixture::new();
        let core = fixture.open(1).expect("store");
        let operations = Operations::new(&core, context(1).expect("context"));
        let request = request(&argv).expect("request");
        operations.accept(&request).expect("accept");
        let permit = operations.start(request.op_id()).expect("start");
        let receipt = operations.commit(permit, json!({"argv":argv})).expect("commit");
        core.close().expect("close");
        // When a later boot retries the same operation.
        let core = fixture.open(2).expect("reopen");
        let recovered = Operations::new(&core, context(2).expect("context")).accept(&request).expect("retry");
        // Then the actual store round trip preserves the entire retained receipt.
        prop_assert_eq!(recovered, receipt);
        core.close().expect("close");
    }
}

#[test]
fn maximum_sequence_round_trips_and_numeric_form_is_invalid_params() -> TestResult {
    use ovis_terminal_protocol::rpc::parse_request;
    use ovis_terminal_protocol::schema::decode_params;
    use ovis_terminal_protocol::schema::methods::server::OpRecord;

    // Given the maximum exact wire counter.
    let wire = json!({
        "op_id":"000005f5-e100-7000-8000-000000000001","method":"terminal.run",
        "state":"committed","result":null,"error":null,
        "updated_seq":u64::MAX.to_string()
    });
    let decoded: OpRecord = serde_json::from_value(wire.clone())?;
    assert_eq!(decoded.updated_seq.get(), u64::MAX);
    assert_eq!(serde_json::to_value(decoded)?, wire);

    // When the same counter arrives as a JSON number.
    let mut malformed = wire;
    *malformed.get_mut("updated_seq").ok_or("missing counter")? = json!(u64::MAX);
    let envelope = json!({"jsonrpc":"2.0","id":"r","method":"op.get","params":malformed});
    let request = parse_request(&envelope.to_string())?;
    let rejection =
        decode_params::<OpRecord>(&request).expect_err("number is not a decimal string");

    // Then the accepted-input boundary returns its stable typed classification.
    assert_eq!(rejection.kind(), "invalid_params");
    Ok(())
}
