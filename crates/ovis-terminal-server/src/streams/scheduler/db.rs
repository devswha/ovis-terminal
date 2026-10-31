//! The dedicated database writer: one owned thread draining a bounded job queue.
//!
//! Synchronous database work never occupies an async worker or an unbounded number of
//! blocking-pool threads. A full queue refuses the job before it is accepted; shutdown
//! closes the queue, lets accepted jobs finish and joins the thread.

use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};

use tokio::sync::oneshot;

use super::error::SchedulerError;

type Job<S> = Box<dyn FnOnce(&mut S) + Send>;

/// A bounded queue of jobs run in order against `S` on one dedicated thread.
#[derive(Debug)]
pub struct DbWriter<S> {
    jobs: SyncSender<Job<S>>,
    thread: JoinHandle<S>,
    capacity: usize,
}

impl<S: Send + 'static> DbWriter<S> {
    /// Starts the writer thread named `name` over `state`, queuing at most `capacity` jobs.
    ///
    /// # Errors
    ///
    /// Returns `internal` when the thread cannot start.
    pub fn spawn(name: &str, state: S, capacity: usize) -> Result<Self, SchedulerError> {
        let (jobs, queue) = mpsc::sync_channel::<Job<S>>(capacity);
        let thread = thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                let mut state = state;
                for job in queue {
                    job(&mut state);
                }
                state
            })
            .map_err(SchedulerError::Spawn)?;
        Ok(Self {
            jobs,
            thread,
            capacity,
        })
    }

    /// Queues `job`; the receiver yields its result once the writer ran it.
    ///
    /// # Errors
    ///
    /// Returns `backpressure` when the queue is full and `unavailable` when the writer
    /// stopped; the job is not queued then.
    pub fn try_submit<R, F>(&self, job: F) -> Result<oneshot::Receiver<R>, SchedulerError>
    where
        F: FnOnce(&mut S) -> R + Send + 'static,
        R: Send + 'static,
    {
        let (reply, result) = oneshot::channel();
        let job: Job<S> = Box::new(move |state| {
            reply.send(job(state)).ok();
        });
        self.jobs.try_send(job).map_err(|error| match error {
            TrySendError::Full(_) => SchedulerError::WriterBusy {
                capacity: self.capacity,
            },
            TrySendError::Disconnected(_) => SchedulerError::WriterStopped,
        })?;
        Ok(result)
    }

    /// Closes the queue, waits for every accepted job and returns the state.
    ///
    /// Blocks the calling thread; call it from a blocking context.
    ///
    /// # Errors
    ///
    /// Returns `internal` when a job panicked on the writer thread.
    pub fn shutdown(self) -> Result<S, SchedulerError> {
        drop(self.jobs);
        self.thread.join().map_err(|_| SchedulerError::Panicked)
    }
}
