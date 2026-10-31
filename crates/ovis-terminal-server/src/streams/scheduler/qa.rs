//! Scheduler QA: the 16-PTY latency profile, typed overflow before admission with a
//! program that never reads stdin, shared fanout, and owned-thread shutdown.

mod failure;
mod fanout;
mod profile;
mod threads;

use std::time::Duration;

use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_vt::{Geometry, HistoryLimit, Vt, VtConfig};
use tokio::sync::broadcast;

use super::{ActorConfig, TerminalSetup};

/// The control contract: every admitted send and committed event within 100 ms.
const CONTROL_BOUND: Duration = Duration::from_millis(100);

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
