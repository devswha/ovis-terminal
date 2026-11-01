//! One continuous run: holders of the binary under test, a control boot, the observer,
//! a sample and a control ping every second, and each scheduled disruption at its time.
//! Teardown always runs, and only stops what this run started.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ovis_terminal_protocol::schema::scalar::{ByteCount, Count};

mod tick;

use super::account::Tally;
use super::control::{GroupSpec, Placement, Setup};
use super::error::SoakError;
use super::holders::{CliRun, HolderEnv, HolderProcess};
use super::launch::ControlProcess;
use super::observer::{Continuity, Observer};
use super::plan::{Kind, Plan};
use super::samples::Samples;
use super::wire::{BootMode, Event, Line, Ready};

pub(super) const SHARED: &str = "soak";
pub(super) const DEDICATED: &str = "soak-ded";
/// Bound on a new boot publishing `ready`.
const READY: Duration = Duration::from_secs(120);

/// A live run and everything it measured so far.
#[derive(Debug)]
pub(super) struct Run<'p> {
    pub(super) plan: &'p Plan,
    exe: PathBuf,
    pub(super) env: HolderEnv,
    pub(super) holders: Vec<HolderProcess>,
    pub(super) control: Option<ControlProcess>,
    pub(super) observer: Option<Observer>,
    pub(super) boot: u64,
    pub(super) tally: Tally,
    pub(super) continuity: Continuity,
    pub(super) samples: Samples,
    pub(super) done: Vec<(Kind, usize)>,
    pub(super) stalled: Vec<UnixStream>,
    pub(super) programs: Vec<u32>,
    pub(super) downtimes_ms: Vec<(Kind, u64)>,
    pub(super) planned_exits: Vec<Option<i32>>,
    pub(super) cli: Vec<CliRun>,
    /// Every holder this run started, with its identity at start.
    pub(super) started_holders: Vec<serde_json::Value>,
    ping: Option<(u64, Instant)>,
    pings: u64,
}

fn logs_dir(plan: &Plan) -> PathBuf {
    plan.evidence.as_ref().map_or_else(
        || plan.root.join("logs"),
        |evidence| evidence.with_extension("logs"),
    )
}

impl<'p> Run<'p> {
    /// Creates the private root; nothing runs yet.
    pub(super) fn new(
        plan: &'p Plan,
        (exe, binary): (PathBuf, &Path),
        evidence: &Path,
    ) -> Result<Self, SoakError> {
        let runtime = plan.root.join("r");
        let logs = logs_dir(plan);
        for dir in [&plan.root, &runtime, &logs, &plan.root.join("state")] {
            std::fs::create_dir_all(dir).map_err(SoakError::io("create the run root"))?;
        }
        let env = HolderEnv {
            binary: binary.to_path_buf(),
            runtime,
            logs,
        };
        Ok(Self {
            plan,
            exe,
            env,
            holders: Vec::new(),
            control: None,
            observer: None,
            boot: 0,
            tally: Tally::default(),
            continuity: Continuity::default(),
            samples: Samples::open(evidence)?,
            done: Kind::ALL.iter().map(|kind| (*kind, 0)).collect(),
            stalled: Vec::new(),
            programs: Vec::new(),
            downtimes_ms: Vec::new(),
            planned_exits: Vec::new(),
            cli: Vec::new(),
            started_holders: Vec::new(),
            ping: None,
            pings: 0,
        })
    }

    /// Starts the shared holder and, for two terminals or more, the dedicated one.
    pub(super) fn start_holders(&mut self) -> Result<(), SoakError> {
        let shared = self.env.start(SHARED)?;
        self.holders.push(shared);
        if self.plan.terminals > 1 {
            let dedicated = self.env.start(DEDICATED)?;
            self.holders.push(dedicated);
        }
        for holder in &self.holders {
            self.started_holders.push(serde_json::to_value(holder)?);
        }
        let status = self.env.cli(&["service", "status", "--session", SHARED])?;
        self.cli.push(status);
        self.setup().save(&self.plan.root)
    }

    fn setup(&self) -> Setup {
        let groups = self
            .holders
            .iter()
            .map(|holder| GroupSpec {
                placement: if holder.session == DEDICATED {
                    Placement::Dedicated
                } else {
                    Placement::Shared
                },
                session: holder.session.clone(),
                socket: holder.socket.clone(),
            })
            .collect();
        let rate = self.plan.bytes_per_terminal.to_string();
        Setup {
            groups,
            terminals: Count::new(u64::try_from(self.plan.terminals).unwrap_or(u64::MAX)),
            program: vec![
                self.exe.display().to_string(),
                "emit".to_owned(),
                "--bytes-per-second".to_owned(),
                rate,
            ],
            state: self.plan.root.join("state"),
            socket: self.plan.root.join("control.sock"),
            checkpoint_every_ms: Count::new(
                u64::try_from(self.plan.checkpoint_every.as_millis()).unwrap_or(u64::MAX),
            ),
            history_lines: Count::new(self.plan.history_lines),
            queue_limit_bytes: ByteCount::new(self.plan.limits.queue_bytes),
            fixture: self.plan.fixture.is_some(),
        }
    }

    /// Starts boot `boot + 1` in `mode` and waits for its `ready`; returns it with the
    /// time from the call to `ready`.
    pub(super) fn boot(&mut self, mode: BootMode) -> Result<(Duration, Box<Ready>), SoakError> {
        let asked = Instant::now();
        self.boot = self.boot.saturating_add(1);
        let logs = self.env.logs.clone();
        let control =
            ControlProcess::start(&self.exe, (&self.plan.root, &logs), (mode, self.boot))?;
        let mut observer = Observer::connect(&control.socket, "harness")?;
        self.control = Some(control);
        let deadline = asked.checked_add(READY).unwrap_or(asked);
        let ready = loop {
            let line = observer.next(deadline)?.ok_or(SoakError::Timeout {
                waiting_for: "ready control boot",
                bound_ms: u64::try_from(READY.as_millis()).unwrap_or(u64::MAX),
            })?;
            self.tally.line(self.boot, &line);
            if let Line::Event {
                event: Event::Ready(ready),
                ..
            } = line
            {
                break ready;
            }
        };
        self.programs = ready
            .terminals
            .iter()
            .map(|identity| identity.pid.get())
            .collect();
        self.observer = Some(observer);
        self.ping = None;
        Ok((asked.elapsed(), ready))
    }

    /// The scheduled interruption of the observer's connection; a ping in flight on the
    /// cut connection is forgotten, not counted late.
    pub(super) fn interrupt(&mut self) -> Result<(), SoakError> {
        let observer = self.observer.take().ok_or(SoakError::ControlGone {
            waiting_for: "an observer to interrupt",
        })?;
        self.observer = Some(observer.interrupt()?);
        self.ping = None;
        Ok(())
    }

    /// Closes the observer of the current boot, keeping its continuity counts.
    pub(super) fn close_observer(&mut self) -> Result<(), SoakError> {
        if let Some(observer) = self.observer.take() {
            let counted = observer.close()?;
            self.continuity.received = self.continuity.received.saturating_add(counted.received);
            self.continuity.gapped = self.continuity.gapped.saturating_add(counted.gapped);
            self.continuity.holes = self.continuity.holes.saturating_add(counted.holes);
            self.continuity.resumes = self.continuity.resumes.saturating_add(counted.resumes);
        }
        Ok(())
    }

    pub(super) fn count(&mut self, kind: Kind) {
        if let Some((_, count)) = self.done.iter_mut().find(|(done, _)| *done == kind) {
            *count = count.saturating_add(1);
        }
    }

    pub(super) fn observer(&mut self) -> Result<&mut Observer, SoakError> {
        self.observer.as_mut().ok_or(SoakError::ControlGone {
            waiting_for: "an observer",
        })
    }

    /// Accounts one observed line; a pong settles the outstanding ping.
    pub(super) fn observe(&mut self, line: &Line) {
        if let Line::Pong { id } = line
            && let Some((sent, at)) = self.ping
            && sent == id.get()
        {
            self.tally
                .pong(u64::try_from(at.elapsed().as_millis()).unwrap_or(u64::MAX));
            self.ping = None;
        }
        self.tally.line(self.boot, line);
    }

    /// Waits for observed lines until `until`, accounting each.
    pub(super) fn listen(&mut self, until: Instant) -> Result<(), SoakError> {
        while let Some(line) = self.observer()?.next(until)? {
            self.observe(&line);
        }
        Ok(())
    }
}
