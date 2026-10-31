//! The one thread that owns the terminal lifecycle, the holder handle and every live
//! terminal: jobs from request handlers and holder messages are applied in one order.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ovis_terminal_authz::Authority;
use ovis_terminal_core::registry::{RegistryIdentities, RuntimeIdentity, TerminalId};
use ovis_terminal_core::terminals::{TerminalError, Terminals};
use ovis_terminal_holder::unix::Holder;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::common::Geometry;
use ovis_terminal_protocol::schema::scalar::{Revision, TimestampMs};
use ovis_terminal_server::streams::{AttachLane, StreamSetup};
use tokio::sync::oneshot;

use super::boot::GeometryLog;
use super::directory::Directory;
use super::error::worker_gone;
use super::live::Live;
use super::{AssemblyError, StageSource};

/// How long one wait for a holder message lasts before queued jobs run again.
const POLL: Duration = Duration::from_millis(5);

/// A unit of work run on the actor thread.
type Job = Box<dyn FnOnce(&mut Runtime) + Send>;

/// Everything the actor thread owns.
#[derive(Debug)]
pub(super) struct Runtime {
    pub(super) terminals: Terminals,
    pub(super) holder: Holder,
    pub(super) identity: RuntimeIdentity,
    pub(super) authority: Arc<Authority<RegistryIdentities>>,
    pub(super) lane: Arc<AttachLane<Arc<Directory>>>,
    pub(super) directory: Arc<Directory>,
    pub(super) setup: StreamSetup,
    pub(super) geometry_log: GeometryLog,
    pub(super) live: BTreeMap<TerminalId, Live>,
    pub(super) launches: BTreeMap<TerminalId, (Geometry, Option<String>)>,
    pub(super) created: BTreeMap<TerminalId, Revision>,
    pub(super) started: Instant,
}

/// The current wall-clock time in milliseconds.
pub(super) fn now() -> Result<TimestampMs, StageSource> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH)?;
    Ok(TimestampMs::new(u64::try_from(elapsed.as_millis())?))
}

/// One structured line on stderr for a holder fact the runtime could not apply.
fn report(stage: &str, error: &dyn std::error::Error) {
    let line = serde_json::json!({"warning": {"stage": stage, "detail": error.to_string()}});
    match writeln!(std::io::stderr().lock(), "{line}") {
        Ok(()) | Err(_) => {}
    }
}

impl Runtime {
    /// Applies at most one holder message; `false` once the holder link is gone.
    pub(super) fn receive(&mut self, wait: Duration) -> bool {
        match self.terminals.receive_message(wait) {
            Ok(Some(message)) => {
                if let Err(error) = self.follow(&message) {
                    report("follow holder message", error.as_ref());
                }
                true
            }
            Ok(None) | Err(TerminalError::Notification(RecvTimeoutError::Timeout)) => true,
            Err(TerminalError::Notification(RecvTimeoutError::Disconnected)) => false,
            Err(error) => {
                report("apply holder message", &error);
                true
            }
        }
    }

    fn close(self) -> Result<(), AssemblyError> {
        drop(self.live);
        self.terminals
            .close()
            .map_err(AssemblyError::stop("release the terminal lifecycle"))?;
        self.holder
            .shutdown()
            .map_err(AssemblyError::stop("shut the holder down"))?;
        self.geometry_log
            .close()
            .map_err(AssemblyError::stop("close the geometry log"))
    }
}

fn run(mut runtime: Runtime, jobs: &mpsc::Receiver<Job>) -> Result<(), AssemblyError> {
    let mut linked = true;
    loop {
        loop {
            match jobs.try_recv() {
                Ok(job) => job(&mut runtime),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return runtime.close(),
            }
        }
        if linked {
            linked = runtime.receive(POLL);
        } else {
            match jobs.recv() {
                Ok(job) => job(&mut runtime),
                Err(_) => return runtime.close(),
            }
        }
    }
}

/// The handle request handlers use to run work on the actor thread.
#[derive(Debug)]
pub(super) struct Actor {
    jobs: mpsc::Sender<Job>,
}

/// The joined actor thread, closed when every job sender is gone.
#[derive(Debug)]
pub(super) struct ActorThread(JoinHandle<Result<(), AssemblyError>>);

impl Actor {
    /// Starts the actor thread over `runtime`.
    pub(super) fn spawn(runtime: Runtime) -> Result<(Self, ActorThread), AssemblyError> {
        let (jobs, receiver) = mpsc::channel::<Job>();
        let thread = std::thread::Builder::new()
            .name("ovis-terminal-runtime".into())
            .spawn(move || run(runtime, &receiver))
            .map_err(AssemblyError::start("start the runtime thread"))?;
        Ok((Self { jobs }, ActorThread(thread)))
    }

    /// Runs `work` on the actor thread and returns what it returned.
    pub(super) async fn call<R: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Runtime) -> Result<R, BoundaryError> + Send + 'static,
    ) -> Result<R, BoundaryError> {
        let (reply, answer) = oneshot::channel();
        let job: Job = Box::new(move |runtime| match reply.send(work(runtime)) {
            Ok(()) | Err(_) => {}
        });
        self.jobs.send(job).map_err(|_| worker_gone())?;
        answer.await.map_err(|_| worker_gone())?
    }
}

impl ActorThread {
    /// Waits for the actor to close the lifecycle, the holder and the geometry log.
    pub(super) fn join(self) -> Result<(), AssemblyError> {
        self.0.join().map_err(|_| AssemblyError::Stop {
            stage: "join the runtime thread",
            source: "the runtime thread panicked".into(),
        })?
    }
}
