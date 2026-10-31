//! The declared local service profile: 16 PTYs, eight clients, 16 requests in flight and
//! 1 MiB/s aggregate output; every admitted send and committed event within 100 ms.
//!
//! Output is paced at the profile's rate (1 KiB per terminal every 16 ms) and the 16 lanes
//! fire together every 10 ms; pacing is the workload, not a synchronization: every
//! assertion waits on product replies or events.
//! The unpaced overload and release-build budgets are measured by `bench/tuning`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_vt::{Geometry, ParseBudget};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::watch;
use tokio::task::JoinSet;

use super::super::{
    ActorConfig, OutputFeed, TerminalEvent, TerminalHandle, TerminalSetup, spawn_terminal,
};
use super::{CONTROL_BOUND, WAIT, screenful, vt};

const TERMINALS: usize = 16;
const CLIENTS: usize = 8;
const IN_FLIGHT_PER_CLIENT: usize = 2;
const ROUNDS: usize = 48;
/// Four seconds of the profile's 1 MiB/s: lanes keep issuing requests until it is sent.
const OUTPUT_TARGET: u64 = 4 * 1024 * 1024;
const TICK: Duration = Duration::from_millis(16);
const PER_TICK: usize = 1024;
/// Every lane issues one request per tick, so all 16 are in flight together each tick.
const REQUEST_TICK: Duration = Duration::from_millis(10);
/// Debug builds parse roughly ten times slower than release; 4 KiB keeps one turn short.
const DEBUG_PARSE_BUDGET: usize = 4 * 1024;

#[derive(Debug, Default)]
struct Samples {
    admission: Vec<Duration>,
    control: Vec<Duration>,
}

fn max(samples: &[Duration]) -> Duration {
    samples.iter().copied().max().unwrap_or_default()
}

/// One in-flight request lane: admissions, resizes and cancels across the terminals,
/// for at least `ROUNDS` requests and until the output target was sent.
async fn lane(index: usize, (handles, sent): (Vec<TerminalHandle>, Arc<AtomicU64>)) -> Samples {
    let mut samples = Samples::default();
    let mut last = None;
    let mut round = 0;
    let mut pace = tokio::time::interval(REQUEST_TICK);
    while round < ROUNDS || sent.load(Ordering::Acquire) < OUTPUT_TARGET {
        pace.tick().await;
        round = round.wrapping_add(1);
        let slot = index.wrapping_mul(5).wrapping_add(round) % TERMINALS;
        let handle = handles.get(slot).expect("one handle per terminal");
        let started = Instant::now();
        match round % 4 {
            0 | 1 => {
                let held = handle.reserve(8).expect("fits the input budgets");
                let receipt = handle.admit(held, b"echo ok\r".to_vec()).await;
                last = Some(receipt.expect("admitted").ticket);
                samples.admission.push(started.elapsed());
            }
            2 => {
                let columns = if round % 8 == 2 { 100 } else { 80 };
                let geometry = Geometry::new(columns, 24).expect("valid geometry");
                handle.resize(geometry).await.expect("resize served");
                samples.control.push(started.elapsed());
            }
            _ => {
                let ticket = last.expect("an admission precedes every cancel");
                handle.cancel(ticket).await.expect("cancel served");
                samples.control.push(started.elapsed());
            }
        }
    }
    samples
}

/// Commit-to-delivery latency of every committed control event one client sees.
async fn watch_events(
    mut events: tokio::sync::broadcast::Receiver<TerminalEvent>,
) -> (Vec<Duration>, u64) {
    let mut latency = Vec::new();
    let mut lost = 0_u64;
    loop {
        match events.recv().await {
            Ok(
                TerminalEvent::Admitted { at, .. }
                | TerminalEvent::Withdrawn { at, .. }
                | TerminalEvent::Resized { at, .. },
            ) => latency.push(at.elapsed()),
            Ok(_) => {}
            Err(RecvError::Lagged(missed)) => lost = lost.saturating_add(missed),
            Err(RecvError::Closed) => return (latency, lost),
        }
    }
}

/// 64 KiB/s of dense output per terminal for as long as the clients run.
async fn produce(
    terminal: usize,
    feed: OutputFeed,
    (done, total): (watch::Receiver<bool>, Arc<AtomicU64>),
) -> u64 {
    let stream: Vec<u8> = (0..64)
        .flat_map(|seed| screenful(terminal.wrapping_mul(64).wrapping_add(seed)))
        .collect();
    let len = u64::try_from(PER_TICK).expect("fits");
    let mut pace = tokio::time::interval(TICK);
    let mut sent = 0_u64;
    let mut at = 0_usize;
    while !*done.borrow() {
        pace.tick().await;
        if stream.get(at..at.saturating_add(PER_TICK)).is_none() {
            at = 0;
        }
        let next = at.saturating_add(PER_TICK);
        let piece = stream.get(at..next).expect("the stream holds one tick");
        if feed.send(piece).await.is_err() {
            break;
        }
        at = next;
        sent = sent.saturating_add(len);
        total.fetch_add(len, Ordering::AcqRel);
    }
    sent
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sixteen_ptys_eight_clients_keep_every_control_within_100_ms() {
    let limits = Limits::v1();
    let budget = ParseBudget::new(DEBUG_PARSE_BUDGET.try_into().expect("nonzero"));
    let config = ActorConfig::advertised(&limits).with_parse_budget(budget);
    let setup = TerminalSetup::advertised(config, &limits);
    let (done, finished) = watch::channel(false);
    let sent = Arc::new(AtomicU64::new(0));
    let mut handles = Vec::new();
    let mut tasks = Vec::new();
    let mut producers = JoinSet::new();
    for terminal in 0..TERMINALS {
        let (sink, mut program) = tokio::io::duplex(64 * 1024);
        tokio::spawn(async move { tokio::io::copy(&mut program, &mut tokio::io::sink()).await });
        let (handle, feed, task) = spawn_terminal(vt(), sink, &setup);
        let progress = (finished.clone(), Arc::clone(&sent));
        producers.spawn(produce(terminal, feed, progress));
        handles.push(handle);
        tasks.push(task);
    }
    let mut watchers = JoinSet::new();
    for _client in 0..CLIENTS {
        for handle in &handles {
            watchers.spawn(watch_events(handle.subscribe()));
        }
    }

    let mut lanes = JoinSet::new();
    for index in 0..CLIENTS * IN_FLIGHT_PER_CLIENT {
        lanes.spawn(lane(index, (handles.clone(), Arc::clone(&sent))));
    }
    let mut samples = Samples::default();
    while let Some(lane) = tokio::time::timeout(WAIT, lanes.join_next())
        .await
        .expect("lanes end")
    {
        let lane = lane.expect("lane joined");
        samples.admission.extend(lane.admission);
        samples.control.extend(lane.control);
    }
    done.send(true).expect("producers listen");
    let mut output_bytes = 0_u64;
    while let Some(sent) = producers.join_next().await {
        output_bytes = sent.expect("producer joined").saturating_add(output_bytes);
    }
    for handle in &handles {
        handle.stop().expect("stop queued");
    }
    let mut parsed = 0_u64;
    let mut commits = 0_u64;
    let mut turns = 0_u64;
    let mut max_turn = Duration::ZERO;
    for task in tasks {
        let report = task.join().await.expect("actor joined");
        parsed = parsed.saturating_add(report.parsed_bytes);
        commits = [report.admitted, report.withdrawn, report.resized]
            .into_iter()
            .fold(commits, u64::saturating_add);
        turns = turns.saturating_add(report.parse_turns);
        max_turn = max_turn.max(report.max_turn);
    }
    drop(handles);
    let mut events = Vec::new();
    let mut lost = 0_u64;
    while let Some(watched) = watchers.join_next().await {
        let (latency, missed) = watched.expect("watcher joined");
        events.extend(latency);
        lost = lost.saturating_add(missed);
    }

    let requests = samples
        .admission
        .len()
        .saturating_add(samples.control.len());
    println!(
        "{{\"profile\":\"16-pty/8-client/16-inflight/1MiB-s\",\"requests\":{requests},\"commits\":{commits},\"admissions\":{},\"controls\":{},\"admission_max_ms\":{:.3},\"control_max_ms\":{:.3},\"events\":{},\"event_max_ms\":{:.3},\"events_lost\":{lost},\"rejected\":0,\"output_bytes\":{output_bytes},\"parsed_bytes\":{parsed},\"parse_turns\":{turns},\"max_turn_ms\":{:.3}}}",
        samples.admission.len(),
        samples.control.len(),
        max(&samples.admission).as_secs_f64() * 1e3,
        max(&samples.control).as_secs_f64() * 1e3,
        events.len(),
        max(&events).as_secs_f64() * 1e3,
        max_turn.as_secs_f64() * 1e3,
    );
    assert!(requests >= CLIENTS * IN_FLIGHT_PER_CLIENT * ROUNDS);
    assert!(
        output_bytes >= OUTPUT_TARGET,
        "the clients ran across the whole output"
    );
    assert_eq!(lost, 0, "no subscriber fell behind");
    let commits = usize::try_from(commits).expect("fits");
    assert_eq!(
        events.len(),
        commits.saturating_mul(CLIENTS),
        "every client saw every commit"
    );
    assert!(
        parsed >= OUTPUT_TARGET && parsed <= output_bytes,
        "the target was parsed; output still queued at stop is never parsed"
    );
    let measured = format!(
        "admission {:?}, control {:?}, event {:?}; longest parse turn {max_turn:?} (a turn of \
         {DEBUG_PARSE_BUDGET} bytes costs well under 10 ms of CPU, so a longer one means the \
         host descheduled the worker)",
        max(&samples.admission),
        max(&samples.control),
        max(&events),
    );
    assert!(max(&samples.admission) <= CONTROL_BOUND, "{measured}");
    assert!(max(&samples.control) <= CONTROL_BOUND, "{measured}");
    assert!(max(&events) <= CONTROL_BOUND, "{measured}");
}
