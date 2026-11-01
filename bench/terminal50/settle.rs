//! Stopping a run in dependency order, every wait bounded.
//!
//! Actors stop first, then their programs and stdin forwarders; once every event sender
//! is gone the clients' forwarders end, a non-reader's unread end is hung up, and each
//! writer drains within its bound. A step that outlives `Plan::teardown` fails the run as
//! `shutdown_stalled` naming the terminal or client, instead of hanging it.

use std::future::Future;
use std::time::Duration;

use ovis_terminal_server::streams::scheduler::OutputEvent;
use tokio::sync::broadcast::error::TryRecvError;

use super::delivery::{Client, ClientOutcome};
use super::error::Terminal50Error;
use super::fleet::{self, Fleet};
use super::lanes::LaneSamples;
use super::plan::Plan;
use super::report::ViewerLag;

#[derive(Debug, Default)]
pub(super) struct Settled {
    pub(super) clients: Vec<ClientOutcome>,
    pub(super) viewers: Vec<ViewerLag>,
    pub(super) pumped: u64,
    pub(super) parsed: u64,
    pub(super) max_turn: Duration,
    pub(super) committed: u64,
    /// Actors that parsed program output.
    pub(super) live_terminals: usize,
}

#[derive(Debug)]
pub(super) struct Load {
    pub(super) samples: LaneSamples,
    pub(super) wanted: usize,
    /// Request lanes that took at least one sample.
    pub(super) lanes: usize,
    pub(super) rss_before: u64,
    pub(super) rss_peak: u64,
    pub(super) disk: u64,
    pub(super) cpu: (Duration, Duration),
    pub(super) wall: Duration,
    pub(super) settled: Settled,
}

/// `work`, or `shutdown_stalled` naming `stage` and `index` once `bound` passed.
pub(super) async fn bounded<T>(
    (stage, index): (&'static str, usize),
    bound: Duration,
    work: impl Future<Output = T>,
) -> Result<T, Terminal50Error> {
    tokio::time::timeout(bound, work)
        .await
        .map_err(|_elapsed| Terminal50Error::ShutdownStalled {
            stage,
            index,
            bound_ms: u64::try_from(bound.as_millis()).unwrap_or(u64::MAX),
        })
}

fn stalled_viewer(
    (terminal, mut output): (usize, tokio::sync::broadcast::Receiver<OutputEvent>),
) -> ViewerLag {
    let mut lag = ViewerLag {
        terminal,
        viewer: 0,
        reads: false,
        bytes: 0,
        events_lost: 0,
        events_held: 0,
    };
    loop {
        match output.try_recv() {
            Ok(event) => {
                lag.events_held = lag.events_held.saturating_add(1);
                if let OutputEvent::Chunk { bytes, .. } = event {
                    let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                    lag.bytes = lag.bytes.saturating_add(len);
                }
            }
            Err(TryRecvError::Lagged(missed)) => {
                lag.events_lost = lag.events_lost.saturating_add(missed);
            }
            Err(TryRecvError::Empty | TryRecvError::Closed) => return lag,
        }
    }
}

async fn stop_actors(
    fleet: Fleet,
    settled: &mut Settled,
    bound: Duration,
) -> Result<Fleet, Terminal50Error> {
    for handle in &fleet.handles {
        handle.stop()?;
    }
    let mut fleet = fleet;
    let mut forwarders = Vec::new();
    for (terminal, program) in fleet.programs.drain(..).enumerate() {
        let (pumped, forward) =
            bounded(("program", terminal), bound, fleet::stop(program)).await??;
        settled.pumped = settled.pumped.saturating_add(pumped);
        forwarders.extend(forward.map(|forward| (terminal, forward)));
    }
    for (terminal, task) in fleet.tasks.drain(..).enumerate() {
        let actor = bounded(("actor", terminal), bound, task.join()).await??;
        settled.parsed = settled.parsed.saturating_add(actor.parsed_bytes);
        settled.max_turn = settled.max_turn.max(actor.max_turn);
        let committed = actor
            .admitted
            .saturating_add(actor.withdrawn)
            .saturating_add(actor.resized);
        settled.committed = settled.committed.saturating_add(committed);
        settled.live_terminals = settled
            .live_terminals
            .saturating_add(usize::from(actor.parsed_bytes > 0));
    }
    for (terminal, forward) in forwarders {
        bounded(
            ("stdin forwarder", terminal),
            bound,
            tokio::task::spawn_blocking(move || forward.join()),
        )
        .await?
        .map_err(std::io::Error::other)?
        .map_err(|_panicked| std::io::Error::other("a stdin forwarder panicked"))?;
    }
    fleet.handles.clear();
    Ok(fleet)
}

/// Stops `fleet` and settles every client and viewer, each step bounded.
pub(super) async fn settle(
    plan: &Plan,
    fleet: Fleet,
    clients: Vec<Client>,
) -> Result<Settled, Terminal50Error> {
    let mut settled = Settled::default();
    let Fleet {
        mut viewers,
        stalled,
        ..
    } = stop_actors(fleet, &mut settled, plan.teardown).await?;
    for mut client in clients {
        client.hang_up_unread();
        let index = client.index();
        let outcome = bounded(
            ("client", index),
            plan.drain.saturating_add(plan.teardown),
            client.finish(plan.drain),
        )
        .await??;
        settled.clients.push(outcome);
    }
    while let Some(viewed) = bounded(("viewers", 0), plan.teardown, viewers.join_next()).await? {
        let viewed = viewed.map_err(std::io::Error::other)?;
        settled.viewers.push(ViewerLag {
            terminal: viewed.terminal,
            viewer: viewed.viewer,
            reads: true,
            bytes: viewed.bytes,
            events_lost: viewed.lost,
            events_held: 0,
        });
    }
    settled
        .viewers
        .extend(stalled.into_iter().map(stalled_viewer));
    settled
        .viewers
        .sort_by_key(|lag| (lag.terminal, lag.viewer));
    Ok(settled)
}
