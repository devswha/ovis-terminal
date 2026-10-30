//! The dedicated blocking writer thread behind a bounded queue.

use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::{self, JoinHandle};

use rusqlite::{Connection, TransactionBehavior};
use tokio::sync::oneshot;

use super::apply::apply;
use crate::error::StoreError;
use crate::mutation::{CommitReceipt, Mutation};
use crate::options::{CommitBarrier, StoreOptions};

type Outcome = Result<CommitReceipt, StoreError>;

#[derive(Debug)]
struct Job {
    mutation: Mutation,
    reply: oneshot::Sender<Outcome>,
}

/// The submitting side of the writer; dropping it lets the thread drain and exit.
#[derive(Debug)]
pub(super) struct Writer {
    queue: SyncSender<Job>,
    capacity: usize,
    thread: JoinHandle<()>,
}

impl Writer {
    pub(super) fn spawn(connection: Connection, options: StoreOptions) -> Result<Self, StoreError> {
        let capacity = options.limits.writer_queue;
        let (queue, jobs) = sync_channel(capacity);
        let thread = thread::Builder::new()
            .name("ovis-store-writer".to_owned())
            .spawn(move || run(connection, &jobs, &options))
            .map_err(StoreError::io("spawn the writer thread for", "state.db"))?;
        Ok(Self {
            queue,
            capacity,
            thread,
        })
    }

    pub(super) fn submit(&self, mutation: Mutation) -> Result<Ticket, StoreError> {
        let (reply, outcome) = oneshot::channel();
        match self.queue.try_send(Job { mutation, reply }) {
            Ok(()) => Ok(Ticket { outcome }),
            Err(TrySendError::Full(_)) => Err(StoreError::Backpressure {
                capacity: self.capacity,
            }),
            Err(TrySendError::Disconnected(_)) => Err(StoreError::WriterStopped),
        }
    }

    pub(super) fn shut_down(self) -> Result<(), StoreError> {
        drop(self.queue);
        self.thread.join().map_err(|_| StoreError::WriterPanicked)
    }
}

fn run(mut connection: Connection, jobs: &Receiver<Job>, options: &StoreOptions) {
    for job in jobs {
        let outcome = commit(&mut connection, &job.mutation, options);
        if let Err(unclaimed) = job.reply.send(outcome) {
            // The submitter stopped waiting; a committed outcome stays readable through its receipt.
            drop(unclaimed);
        }
    }
}

fn commit(connection: &mut Connection, mutation: &Mutation, options: &StoreOptions) -> Outcome {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let receipt = apply(&tx, mutation, &options.limits)?;
    options.reach(CommitBarrier::Applied);
    tx.commit()?;
    options.reach(CommitBarrier::Committed);
    Ok(receipt)
}

/// The pending outcome of one submitted mutation.
#[derive(Debug)]
pub struct Ticket {
    outcome: oneshot::Receiver<Outcome>,
}

impl Ticket {
    /// Blocks the calling thread until the writer answers.
    ///
    /// # Errors
    ///
    /// Returns the commit's own error, or `writer_stopped` when the writer exited first.
    ///
    /// # Panics
    ///
    /// Panics when called from inside a Tokio runtime; await [`Ticket::outcome`] there.
    pub fn wait(self) -> Result<CommitReceipt, StoreError> {
        self.outcome
            .blocking_recv()
            .map_err(|_| StoreError::WriterStopped)?
    }

    /// Waits without blocking an async worker thread.
    ///
    /// # Errors
    ///
    /// Returns the commit's own error, or `writer_stopped` when the writer exited first.
    pub async fn outcome(self) -> Result<CommitReceipt, StoreError> {
        self.outcome.await.map_err(|_| StoreError::WriterStopped)?
    }
}
