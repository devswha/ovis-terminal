//! The one thread that owns the terminal lifecycle, the holder handle and every live
//! terminal: jobs from request handlers and holder messages are applied in one order.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ovis_terminal_authz::Authority;
use ovis_terminal_core::registry::{RegistryIdentities, RuntimeIdentity, TerminalId};
use ovis_terminal_core::restart::{Control, RestartError};
use ovis_terminal_core::terminals::{TerminalError, Terminals};
use ovis_terminal_holder::unix::Holder;
use ovis_terminal_protocol::errors::{BoundaryError, OpId};
use ovis_terminal_protocol::schema::common::Geometry;
use ovis_terminal_protocol::schema::methods::server::StopScope;
use ovis_terminal_protocol::schema::scalar::{Revision, TimestampMs};
use ovis_terminal_server::streams::{AttachLane, StreamSetup};
use tokio::sync::oneshot;

use super::boot::GeometryLog;
use super::directory::Directory;
use super::error::worker_gone;
use super::live::Live;
use super::{AssemblyError, StageSource};

/// The holder link as one receive found it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Link {
    /// A message from the current lifecycle was applied; more may be waiting.
    Linked,
    /// No message is waiting; the link is intact.
    Idle,
    /// A planned restart owns the lifecycle; the successor's messages follow it.
    Restarting,
    /// Control stopped or the holder went away; no message will follow.
    Gone,
}

/// A unit of work run on the actor thread.
type Job = Box<dyn FnOnce(&mut Runtime) + Send>;

/// Everything the actor thread owns.
#[derive(Debug)]
pub(super) struct Runtime {
    pub(super) control: Arc<Control>,
    /// How long the actor waits for a job, with the control released, before it looks
    /// for holder messages again.
    pub(super) poll: Duration,
    /// QA seam: a FIFO opened, holding the control, before the next `terminal.run`.
    pub(super) qa_hold: Option<std::path::PathBuf>,
    /// Output retention and checkpoint bounds of every terminal's screen.
    pub(super) screen_bounds: ovis_terminal_core::screen::ScreenBounds,
    pub(super) journal: Arc<super::journal::BootJournal>,
    /// The current boot's status events, drained between jobs.
    pub(super) status_feed: Option<ovis_terminal_core::events::Delivery<super::lanes::ServedEvent>>,
    pub(super) holder: Holder,
    pub(super) identity: RuntimeIdentity,
    pub(super) authority: Arc<Authority<RegistryIdentities>>,
    pub(super) lane: Arc<AttachLane<Arc<Directory>>>,
    pub(super) directory: Arc<Directory>,
    pub(super) setup: StreamSetup,
    pub(super) geometry_log: GeometryLog,
    pub(super) layout_log: super::layout::LayoutLog,
    pub(super) live: BTreeMap<TerminalId, Live>,
    pub(super) launches: BTreeMap<TerminalId, (Geometry, Option<String>)>,
    pub(super) created: BTreeMap<TerminalId, Revision>,
    /// Process tokens handed to spawned programs, shared with the agent lanes.
    pub(super) status: Arc<ovis_terminal_agent::status::StatusService>,
    /// The boot's health, which the runtime publishes its census into.
    pub(super) health: Arc<ovis_terminal_core::health::Health>,
    /// The census last published.
    pub(super) census: Option<ovis_terminal_core::health::Census>,
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
    /// Runs `work` on the lifecycle the restart control currently owns.
    pub(super) fn lifecycle<R>(
        &self,
        work: impl FnOnce(&mut Terminals) -> Result<R, TerminalError>,
    ) -> Result<R, BoundaryError> {
        self.control
            .with_terminals(|terminals, _| work(terminals))
            .map_err(|error| match error {
                RestartError::Terminal(error) => super::terminal::terminal_refusal(&error),
                other => super::error::refusal(other.kind(), other.hint()),
            })
    }

    /// Applies at most one holder message that already arrived and says whether the holder
    /// link is still there. It cannot wait while it holds the control (`try_receive_message`
    /// takes no duration): a caller blocked on the lifecycle gets it between receives.
    pub(super) fn receive(&mut self) -> Link {
        let received = self
            .control
            .with_terminals(|terminals, _| terminals.try_receive_message());
        match received {
            Ok(Some(message)) => {
                if let Err(error) = self.follow(&message) {
                    report("follow holder message", error.as_ref());
                }
                Link::Linked
            }
            Ok(None) => Link::Linked,
            Err(RestartError::Terminal(TerminalError::Notification(RecvTimeoutError::Timeout))) => {
                Link::Idle
            }
            Err(RestartError::Restarting { .. }) => Link::Restarting,
            Err(
                RestartError::Terminal(TerminalError::Notification(RecvTimeoutError::Disconnected))
                | RestartError::ControlStopped,
            ) => Link::Gone,
            Err(error) => {
                report("apply holder message", &error);
                Link::Linked
            }
        }
    }

    fn close(self) -> Result<(), AssemblyError> {
        drop(self.live);
        stop_control(&self.control)?;
        drop(self.control);
        self.holder
            .shutdown()
            .map_err(AssemblyError::stop("shut the holder down"))?;
        self.geometry_log
            .close()
            .map_err(AssemblyError::stop("close the geometry log"))?;
        self.layout_log
            .close()
            .map_err(AssemblyError::stop("close the layout log"))
    }
}

/// Ends the programs this holder owns and releases control, without requiring the last
/// handle: dispatched lanes may still hold the control while their tasks wind down.
/// Programs a previous boot recorded and this holder never owned cannot be proven
/// ended; control is then stopped without signalling anything further.
fn stop_control(control: &Control) -> Result<(), AssemblyError> {
    let op = || OpId::new(uuid::Uuid::now_v7().to_string());
    let stopped = match control.stop(op(), StopScope::ControlAndPrograms) {
        Err(RestartError::StopUnproven { .. }) => control.stop(op(), StopScope::ControlOnly),
        other => other,
    };
    match stopped {
        Ok(_) | Err(RestartError::ControlStopped) => Ok(()),
        Err(error) => Err(AssemblyError::stop("stop the terminal lifecycle")(error)),
    }
}

fn run(mut runtime: Runtime, jobs: &mpsc::Receiver<Job>) -> Result<(), AssemblyError> {
    let mut linked = true;
    loop {
        loop {
            match jobs.try_recv() {
                Ok(job) => applied(&mut runtime, job),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return runtime.close(),
            }
        }
        // Events delivered while no job arrives still settle the waits they satisfy.
        runtime.drain_status();
        if linked {
            match runtime.receive() {
                Link::Linked => {}
                // Waiting happens here, on the job queue, with the control released.
                Link::Idle | Link::Restarting => match jobs.recv_timeout(runtime.poll) {
                    Ok(job) => applied(&mut runtime, job),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return runtime.close(),
                },
                Link::Gone => linked = false,
            }
            runtime.publish_census();
        } else {
            match jobs.recv() {
                Ok(job) => applied(&mut runtime, job),
                Err(_) => return runtime.close(),
            }
        }
    }
}

/// The handle request handlers use to run work on the actor thread.
#[derive(Debug, Clone)]
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
        let job: Job = Box::new(move |runtime| {
            let result = work(runtime);
            runtime.publish_census();
            match reply.send(result) {
                Ok(()) | Err(_) => {}
            }
        });
        self.jobs.send(job).map_err(|_| worker_gone())?;
        answer.await.map_err(|_| worker_gone())?
    }
}

impl Actor {
    /// Runs `work` on the actor thread from synchronous code; `None` once it stopped.
    pub(super) fn call_blocking<R: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Runtime) -> R + Send + 'static,
    ) -> Option<R> {
        let (reply, answer) = mpsc::channel();
        let job: Job = Box::new(move |runtime| {
            let result = work(runtime);
            runtime.publish_census();
            match reply.send(result) {
                Ok(()) | Err(_) => {}
            }
        });
        self.jobs.send(job).ok()?;
        answer.recv().ok()
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

/// Runs `job` after applying every status event delivered so far, so a job queued behind a
/// `status.report` observes it whatever batch the two jobs land in.
fn applied(runtime: &mut Runtime, job: Job) {
    runtime.drain_status();
    job(runtime);
}
