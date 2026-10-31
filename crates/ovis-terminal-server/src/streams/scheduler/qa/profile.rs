//! The declared local service profile: 16 PTYs, eight clients, 16 requests in flight and
//! 1 MiB/s aggregate output; every admitted send and committed event within 100 ms.
//!
//! Output is paced at the profile's rate (1 KiB per terminal every 16 ms) and the 16 lanes
//! fire together every 10 ms; pacing is the workload, not a synchronization: every
//! assertion waits on product replies or events.
//!
//! The test asserts what does not depend on host speed: every request answered, every
//! commit seen by every client, nothing lost, every accepted byte parsed. The 100 ms
//! contract is reported (`within_contract`), not asserted, since wall time depends on the
//! host; control-first ordering is asserted by counts in `ordering`, and release-build
//! latency is measured by `bench/tuning`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_vt::{Geometry, ParseBudget};
use tokio::sync::watch;
use tokio::task::JoinSet;

use super::super::{ActorConfig, Engine, TerminalHandle, TerminalSetup, spawn_terminal};
use super::workload::{drain, produce, track_parsed, watch_events};
use super::{WAIT, vt};

const TERMINALS: usize = 16;
const CLIENTS: usize = 8;
const IN_FLIGHT_PER_CLIENT: usize = 2;
const ROUNDS: usize = 48;
/// Four seconds of the profile's 1 MiB/s: lanes keep issuing requests until it is sent.
const OUTPUT_TARGET: u64 = 4 * 1024 * 1024;
/// Every lane issues one request per tick, so all 16 are in flight together each tick.
const REQUEST_TICK: Duration = Duration::from_millis(10);
/// The plan's bound on every admitted send and committed event.
const CONTRACT: Duration = Duration::from_millis(100);
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sixteen_ptys_eight_clients_get_every_request_and_commit_with_latency_reported() {
    let limits = Limits::v1();
    let budget = ParseBudget::new(DEBUG_PARSE_BUDGET.try_into().expect("nonzero"));
    let config = ActorConfig::advertised(&limits).with_parse_budget(budget);
    let setup = TerminalSetup::advertised(config, &limits);
    let (done, finished) = watch::channel(false);
    let sent = Arc::new(AtomicU64::new(0));
    let mut handles = Vec::new();
    let mut tasks = Vec::new();
    let mut trackers = Vec::new();
    let mut producers = JoinSet::new();
    for terminal in 0..TERMINALS {
        let (sink, mut program) = tokio::io::duplex(64 * 1024);
        tokio::spawn(async move { tokio::io::copy(&mut program, &mut tokio::io::sink()).await });
        let (handle, feed, task) = spawn_terminal(Engine::new(vt()), sink, &setup).expect("spawns");
        trackers.push(track_parsed(handle.subscribe_output()));
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
    let output_bytes = drain(producers, trackers).await;
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
    assert_eq!(
        parsed, output_bytes,
        "every accepted byte was parsed before stop"
    );
    report_bound(&samples, &events, max_turn);
}

/// Reports the worst sample against the plan's 100 ms contract. It is evidence, not an
/// assertion: wall time depends on the host (a contended CI runner delays every sample
/// alike), while control-first ordering is asserted by counts in `ordering`.
fn report_bound(samples: &Samples, events: &[Duration], max_turn: Duration) {
    let worst = max(&samples.admission)
        .max(max(&samples.control))
        .max(max(events));
    let within_contract = worst <= CONTRACT;
    println!(
        "{{\"within_contract\":{within_contract},\"contract_ms\":100,\"worst_ms\":{:.3},\"max_turn_ms\":{:.3}}}",
        worst.as_secs_f64() * 1e3,
        max_turn.as_secs_f64() * 1e3,
    );
}
