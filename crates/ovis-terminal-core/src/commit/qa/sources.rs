//! Terminal, registry and remote sources committing at once share one gapless order.

use std::collections::BTreeSet;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ovis_terminal_core::commit::{Boundary, CommitSource, Committed, PageTtl, Pager};
use ovis_terminal_core::events::{RemoteOrigin, RemoteSeq, StateSnapshot};
use ovis_terminal_core::subscriptions::{
    Pulled, QueueLimits, Request, Selector, Start, Subscriptions,
};
use ovis_terminal_protocol::schema::scalar::{ByteOffset, EventSeq, TimestampMs};

use super::*;

const PER_SOURCE: u64 = 60;
const BOUND: Duration = Duration::from_secs(20);

fn source(index: u64, n: u64) -> Result<CommitSource, Box<dyn Error>> {
    Ok(match index {
        0 => CommitSource::Terminal {
            terminal_id: TerminalId::try_from(1)?,
        },
        1 => CommitSource::Registry,
        _ => CommitSource::Remote {
            origin: RemoteOrigin::new(
                MachineId::new(Uuid::from_u128(2))?,
                RuntimeId::try_from(7)?,
                RemoteSeq::new(EventSeq::new(n.saturating_add(1000))),
            ),
        },
    })
}

/// Commits `PER_SOURCE` records from one source, signalling after the tenth.
fn producer(
    boundary: Boundary<Probe, Producer>,
    index: u64,
    started: mpsc::Sender<()>,
) -> std::thread::JoinHandle<Result<Vec<Committed>, String>> {
    std::thread::spawn(move || {
        let mut committed = Vec::new();
        for n in 1..=PER_SOURCE {
            let from = source(index, n).map_err(|error| error.to_string())?;
            let label = format!("source-{index}");
            let record = boundary
                .commit(from, |_| {
                    wrote(&label, n).map_err(|_| EventError::InvalidRecord.into())
                })
                .map_err(|error| error.kind().to_owned())?;
            committed.push(record);
            if n == 10 {
                started.send(()).map_err(|error| error.to_string())?;
            }
        }
        Ok(committed)
    })
}

fn read_window(pager: &Pager<Probe, Producer>) -> Result<StateSnapshot<Captured>, Box<dyn Error>> {
    let window = pager.open(TimestampMs::new(0))?;
    let mut bytes = Vec::new();
    let mut offset = Some(ByteOffset::new(0));
    while let Some(at) = offset {
        let page = pager.page(&window.token, (at, 1 << 20, TimestampMs::new(1)))?;
        assert_eq!(
            page.cursor, window.cursor,
            "every page is pinned to one revision"
        );
        bytes.extend_from_slice(&page.data);
        offset = page.continuation;
    }
    pager.release(&window.token)?;
    assert_eq!(u64::try_from(bytes.len())?, window.total.get());
    Ok(serde_json::from_slice(&bytes)?)
}

#[test]
fn simultaneous_sources_share_one_gapless_order() -> TestResult {
    // Given three sources committing through one boundary.
    let fixture = Fixture::new();
    let core = fixture.core((StoreOptions::default(), Producer::default()))?;
    let boundary = Boundary::new(Arc::clone(&core), 4)?;
    let hub = Subscriptions::start(Arc::clone(&core), identity()?, QueueLimits::v1()?)?;
    let (started, ready) = mpsc::channel();
    let threads = (0..3)
        .map(|index| producer(boundary.clone(), index, started.clone()))
        .collect::<Vec<_>>();
    for _ in 0..3 {
        ready.recv_timeout(BOUND)?;
    }
    // When a snapshot is paged and a subscription installs while they keep committing.
    let pager = Pager::new(Arc::clone(&core), PageTtl::new(60_000)?);
    let snapshot = read_window(&pager)?;
    let installed = hub.subscribe(Request::new(Start::Snapshot, Selector::all()))?;
    let mut sources = Vec::new();
    for thread in threads {
        sources.push(thread.join().map_err(|_| "producer panicked")??);
    }
    let total = 3 * PER_SOURCE;
    // Then every committed sequence appears exactly once, in each source's order.
    let all: BTreeSet<u64> = sources
        .iter()
        .flatten()
        .map(|c| c.local_seq().get())
        .collect();
    assert_eq!(all, (1..=total).collect::<BTreeSet<_>>());
    for records in &sources {
        assert!(
            records
                .windows(2)
                .all(|w| w.first().map(Committed::local_seq) < w.last().map(Committed::local_seq))
        );
    }
    let [terminal, registry, remote] = sources.as_slice() else {
        return Err("three sources".into());
    };
    for (n, record) in (1_u64..).zip(remote) {
        assert_eq!(
            record.remote().map(|origin| origin.seq.get()),
            Some(n.saturating_add(1000))
        );
    }
    assert!(
        terminal
            .iter()
            .chain(registry)
            .all(|c| c.remote().is_none())
    );
    // And the paged snapshot holds exactly the records up to its revision.
    let at = snapshot.cursor.seq.get();
    assert_eq!(snapshot.sections.seqs, (1..=at).collect::<Vec<_>>());
    // And the subscription's snapshot plus its deliveries partition the whole sequence.
    let boundary_seq = installed.snapshot_seq.get();
    let held = installed.snapshot.as_ref().ok_or("snapshot requested")?;
    assert_eq!(held.sections.seqs, (1..=boundary_seq).collect::<Vec<_>>());
    let mut delivered = Vec::new();
    let deadline = Instant::now().checked_add(BOUND).ok_or("deadline")?;
    while delivered.last().copied().unwrap_or(boundary_seq) < total {
        match installed.feed.next(deadline)? {
            Pulled::Events(batch) => {
                delivered.extend(batch.events.iter().map(|event| event.cursor.seq.get()));
            }
            Pulled::Heartbeat(_) if Instant::now() < deadline => {}
            other => return Err(format!("delivery stopped: {other:?}").into()),
        }
    }
    assert_eq!(delivered, ((boundary_seq + 1)..=total).collect::<Vec<_>>());
    drop((installed, pager, boundary));
    hub.shutdown()?;
    close(core)
}

#[test]
fn async_submissions_are_bounded_and_hold_no_lock_across_awaits() -> TestResult {
    // Given a boundary with one submission slot and a writer held before COMMIT.
    let fixture = Fixture::new();
    let (applied, reached) = mpsc::sync_channel(1);
    let (release, held) = mpsc::sync_channel::<()>(2);
    let held = std::sync::Mutex::new(held);
    let options = StoreOptions::default().with_barrier(move |point| {
        if matches!(point, ovis_terminal_store::CommitBarrier::Applied) {
            applied.try_send(()).ok();
            if let Ok(held) = held.lock() {
                held.recv_timeout(BOUND).ok();
            }
        }
    });
    let core = fixture.core((options, Producer::default()))?;
    let boundary = Boundary::new(Arc::clone(&core), 1)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    // When one submission is in flight and a second arrives.
    let first = {
        let boundary = boundary.clone();
        runtime.spawn(async move {
            boundary
                .submit(CommitSource::Registry, |_| {
                    wrote("first", 1).map_err(|_| EventError::InvalidRecord.into())
                })
                .await
        })
    };
    reached.recv_timeout(BOUND)?;
    let second = runtime.block_on(boundary.submit(CommitSource::Registry, |_| {
        wrote("second", 1).map_err(|_| EventError::InvalidRecord.into())
    }));
    // Then the second is refused at admission while the async runtime stays responsive.
    assert_eq!(
        second.map(|_| ()).map_err(|e| e.kind()),
        Err("backpressure")
    );
    assert_eq!(runtime.block_on(async { 7 }), 7);
    release.send(())?;
    let committed = runtime.block_on(first)??;
    assert_eq!(committed.local_seq(), EventSeq::new(1));
    release.send(())?;
    let third = runtime.block_on(boundary.submit(CommitSource::Registry, |_| {
        wrote("third", 1).map_err(|_| EventError::InvalidRecord.into())
    }))?;
    assert_eq!(third.local_seq(), EventSeq::new(2));
    runtime.shutdown_timeout(BOUND);
    drop(boundary);
    close(core)
}
