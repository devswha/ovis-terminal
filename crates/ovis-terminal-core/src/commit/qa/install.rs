//! A subscription installed while a commit is held before COMMIT sees it exactly once.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use ovis_terminal_core::commit::{Boundary, CommitSource};
use ovis_terminal_core::subscriptions::{
    Pulled, QueueLimits, Request, Selector, Start, Subscriptions,
};
use ovis_terminal_store::CommitBarrier;

use super::*;

const BOUND: Duration = Duration::from_secs(20);

#[test]
fn a_subscription_installs_at_the_commit_barrier() -> TestResult {
    // Given an early subscriber and a writer that stops the first commit before COMMIT.
    let fixture = Fixture::new();
    let (applied, reached) = mpsc::sync_channel(4);
    let (release, held) = mpsc::sync_channel::<()>(4);
    let held = std::sync::Mutex::new(held);
    let options = StoreOptions::default().with_barrier(move |point| {
        if matches!(point, CommitBarrier::Applied) {
            applied.try_send(()).ok();
            if let Ok(held) = held.lock() {
                held.recv_timeout(BOUND).ok();
            }
        }
    });
    let core = fixture.core((options, Producer::default()))?;
    let hub = Subscriptions::start(Arc::clone(&core), identity()?, QueueLimits::v1()?)?;
    let early = hub.subscribe(Request::new(Start::Head, Selector::all()))?;
    let boundary = Boundary::new(Arc::clone(&core), 1)?;
    let committer = {
        let boundary = boundary.clone();
        std::thread::spawn(move || {
            let record = wrote("terminal", 1).map_err(|error| error.to_string())?;
            let source = CommitSource::Terminal {
                terminal_id: TerminalId::try_from(1).map_err(|error| error.to_string())?,
            };
            boundary
                .commit(source, |_| Ok(record))
                .map_err(|error| error.kind().to_owned())
        })
    };
    reached.recv_timeout(BOUND)?;
    // When nothing is published before COMMIT, and a subscription installs meanwhile.
    assert!(matches!(
        early.feed.next(Instant::now())?,
        Pulled::Heartbeat(_)
    ));
    let installing = {
        let hub_core = Arc::clone(&core);
        std::thread::spawn(move || {
            hub_core
                .read_snapshot()
                .map(|snapshot| snapshot.cursor.seq.get())
                .map_err(|error| error.kind().to_owned())
        })
    };
    release.send(())?;
    let first = committer.join().map_err(|_| "committer panicked")??;
    let late = hub.subscribe(Request::new(Start::Snapshot, Selector::all()))?;
    let seen = installing.join().map_err(|_| "reader panicked")??;
    // Then the reader saw the barrier before or after the whole commit, never inside it.
    assert_eq!(first.local_seq().get(), 1);
    assert_eq!(seen, 1, "the snapshot waits for the held commit");
    assert_eq!(late.snapshot_seq.get(), 1);
    assert_eq!(
        late.snapshot.as_ref().ok_or("snapshot")?.sections.seqs,
        vec![1]
    );
    // And the early subscriber gets seq 1 once, the late one only what follows.
    release.send(())?;
    let record = wrote("registry", 2)?;
    boundary.commit(CommitSource::Registry, |_| Ok(record))?;
    assert_eq!(drain(&early, 2)?, vec![1, 2]);
    assert_eq!(drain(&late, 2)?, vec![2]);
    drop((early, late, boundary));
    hub.shutdown()?;
    close(core)
}

fn drain(
    started: &ovis_terminal_core::subscriptions::Started<Probe, Producer>,
    until: u64,
) -> Result<Vec<u64>, Box<dyn Error>> {
    let deadline = Instant::now().checked_add(BOUND).ok_or("deadline")?;
    let mut seqs: Vec<u64> = Vec::new();
    while seqs.last().copied().unwrap_or(0) < until {
        match started.feed.next(deadline)? {
            Pulled::Events(batch) => seqs.extend(batch.events.iter().map(|e| e.cursor.seq.get())),
            Pulled::Heartbeat(_) if Instant::now() < deadline => {}
            other => return Err(format!("delivery stopped: {other:?}").into()),
        }
    }
    Ok(seqs)
}
