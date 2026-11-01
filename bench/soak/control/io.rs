//! The control process's own channels: harness commands on stdin, the socket
//! announcement on stdout, and one forwarding thread per adopted holder link.

use std::io::{BufRead as _, Write as _};
use std::path::Path;
use std::sync::mpsc::{Sender, SyncSender};
use std::thread::JoinHandle;

use super::super::error::SoakError;
use super::Inbound;
use super::links::{Link, Outbound};

/// A harness command, one per stdin line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in super::super) enum Command {
    /// Save every screen, say goodbye to every holder and exit for a successor.
    Handover,
    /// Exit without a goodbye; also what the end of stdin means.
    Stop,
    /// Fixture: send input whose receipt this boot never commits.
    LoseReceipt,
}

impl Command {
    pub(super) fn parse(line: &str) -> Option<Self> {
        match line.trim() {
            "handover" => Some(Self::Handover),
            "stop" => Some(Self::Stop),
            "lose-receipt" => Some(Self::LoseReceipt),
            _ => None,
        }
    }
}

/// Forwards stdin commands; the end of stdin is a stop. `false` once the loop is gone.
pub(super) fn read_commands(inbound: &SyncSender<Inbound>) -> bool {
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Some(command) = Command::parse(&line) else {
            continue;
        };
        if inbound.send(Inbound::Command(command)).is_err() {
            return false;
        }
    }
    inbound.send(Inbound::Command(Command::Stop)).is_ok()
}

pub(super) fn announce(socket: &Path) -> Result<(), SoakError> {
    let line = format!("{}\n", serde_json::json!({ "listening": socket }));
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(line.as_bytes())
        .and_then(|()| stdout.flush())
        .map_err(SoakError::io("announce the control socket"))
}

/// Request senders per group (`None` for a group without a link) and link threads.
pub(super) type Started = (Vec<Option<Sender<Outbound>>>, Vec<JoinHandle<()>>);

pub(super) fn start_links(
    begun: Vec<Option<Link>>,
    inbound: &SyncSender<Inbound>,
) -> Result<Started, SoakError> {
    let mut senders = Vec::with_capacity(begun.len());
    let mut threads = Vec::new();
    for (group, link) in begun.into_iter().enumerate() {
        let Some(link) = link else {
            senders.push(None);
            continue;
        };
        let (sender, thread) = link.start(group, inbound.clone())?;
        senders.push(Some(sender));
        threads.push(thread);
    }
    Ok((senders, threads))
}
