//! Scheduler QA: count-based control-first ordering, typed overflow before admission with
//! a program that never reads stdin, session caps, disk history, shared fanout and
//! slow-subscriber loss counts, owned-thread shutdown, and the 16-PTY latency profile.
//!
//! Every assertion waits on product replies or events. Wall-clock latency is reported,
//! never asserted.

mod caps;
mod failure;
mod fanout;
mod history;
mod ordering;
mod profile;
mod threads;
mod workload;

use std::time::Duration;

use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_vt::{Geometry, HistoryLimit, OutputOffset, Vt, VtConfig};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

use super::{
    ActorConfig, ActorTask, Engine, OutputEvent, OutputFeed, TerminalHandle, TerminalSetup,
    spawn_terminal,
};

/// Upper bound on any single wait for a product signal; reaching it fails the test.
const WAIT: Duration = Duration::from_secs(20);

fn vt() -> Vt {
    let geometry = Geometry::new(80, 24).expect("valid geometry");
    Vt::new(VtConfig::new(geometry, HistoryLimit::new(1_000)))
}

fn setup() -> TerminalSetup {
    let limits = Limits::v1();
    TerminalSetup::advertised(ActorConfig::advertised(&limits), &limits)
}

/// A terminal whose program discards its input.
fn spawn(setup: &TerminalSetup) -> (TerminalHandle, OutputFeed, ActorTask) {
    spawn_terminal(Engine::new(vt()), tokio::io::sink(), setup).expect("spawns")
}

/// Dense, colored terminal output: SGR runs, cursor moves and line breaks.
fn screenful(seed: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for line in 0..24 {
        let colour = (seed.wrapping_add(line) % 7).wrapping_add(31);
        let text = format!(
            "\x1b[{colour};1m{seed:06}:{line:02}\x1b[0m build step {line} ok \x1b[K\x1b[2;{}H\r\n",
            (line % 70).wrapping_add(1)
        );
        out.extend_from_slice(text.as_bytes());
    }
    out
}

/// Waits for the first event `pick` accepts, failing on loss, close or the bound.
async fn await_event<E: Clone, T>(
    events: &mut broadcast::Receiver<E>,
    mut pick: impl FnMut(&E) -> Option<T>,
) -> T {
    let found = tokio::time::timeout(WAIT, async {
        loop {
            match events.recv().await {
                Ok(event) => {
                    if let Some(found) = pick(&event) {
                        return found;
                    }
                }
                Err(error) => panic!("event stream ended: {error}"),
            }
        }
    })
    .await;
    found.expect("the event arrives within the bound")
}

/// Reads output events until the screen reflects `end`; returns the events lost to lag.
///
/// The final `Parsed` event is the newest one the actor publishes, so a receiver that
/// lags still receives it: the broadcast keeps the newest events.
async fn parsed_through(mut output: broadcast::Receiver<OutputEvent>, end: OutputOffset) -> u64 {
    let mut lost = 0_u64;
    let reached = tokio::time::timeout(WAIT, async {
        loop {
            match output.recv().await {
                Ok(OutputEvent::Parsed { offset }) if offset == end => return,
                Ok(_) => {}
                Err(RecvError::Lagged(missed)) => lost = lost.saturating_add(missed),
                Err(RecvError::Closed) => panic!("output closed before offset {end}"),
            }
        }
    })
    .await;
    reached.expect("the output is parsed within the bound");
    lost
}
