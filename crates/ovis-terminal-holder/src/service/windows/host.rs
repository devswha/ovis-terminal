//! A holder serving its owner-only pipe: claim the name, publish the record, accept.
//!
//! The first instance is created with `FILE_FLAG_FIRST_PIPE_INSTANCE`, so a process that
//! already holds the name is never joined. The next instance exists before an accepted
//! one is handed to its connection, so the name is never released while the holder
//! runs. A failed accept ends only that instance; the holder keeps accepting.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use ovis_terminal_protocol::domain::unix_service::Observed;
use ovis_terminal_protocol::domain::windows_service::{HolderPlacement, HostRecord, PipeName};
use tokio::net::windows::named_pipe::NamedPipeServer;
use tokio::sync::watch;
use tokio::task::JoinSet;

use super::facts::{observe, own_proof};
use super::link::{self, Flow, LinkContext};
use super::pipe::{self, OwnerSecurity};
use super::terminals::{Table, lock};
use super::{HostError, HostPaths, withdraw_record, write_record};
use crate::protocol::{HolderBuildId, HolderDescriptor, HolderInstanceId};

/// Which session a holder serves and which placement the host selected for it.
#[derive(Debug, Clone)]
pub struct HostConfig {
    paths: HostPaths,
    placement: HolderPlacement,
}

impl HostConfig {
    /// A holder of `paths` in `placement`.
    #[must_use]
    pub const fn new(paths: HostPaths, placement: HolderPlacement) -> Self {
        Self { paths, placement }
    }

    /// The session's paths.
    #[must_use]
    pub const fn paths(&self) -> &HostPaths {
        &self.paths
    }
}

type Failures = Arc<Mutex<BTreeMap<&'static str, u64>>>;

struct Accept {
    server: NamedPipeServer,
    name: PipeName,
    security: Arc<OwnerSecurity>,
    context: Arc<LinkContext>,
    stop: Arc<watch::Sender<bool>>,
    failures: Failures,
}

fn count(failures: &Failures, kind: &'static str) {
    let mut failures = lock(failures);
    let entry = failures.entry(kind).or_default();
    *entry = entry.saturating_add(1);
    drop(failures);
}

async fn accept(mut accepting: Accept) -> Result<(), HostError> {
    let mut stopped = accepting.stop.subscribe();
    let mut running = JoinSet::new();
    let ended = loop {
        tokio::select! {
            biased;
            _ = stopped.wait_for(|stop| *stop) => break Ok(()),
            Some(_) = running.join_next(), if !running.is_empty() => {}
            connected = accepting.server.connect() => {
                let next = match pipe::create_instance(&accepting.name, false, &accepting.security) {
                    Ok(next) => next,
                    Err(source) => break Err(HostError::os("create the next pipe instance", source)),
                };
                let accepted = std::mem::replace(&mut accepting.server, next);
                if let Err(error) = connected {
                    count(&accepting.failures, "accept_failed");
                    drop((error, accepted));
                    continue;
                }
                let (context, stop, failures) = (Arc::clone(&accepting.context), Arc::clone(&accepting.stop), Arc::clone(&accepting.failures));
                running.spawn(async move {
                    match link::serve(accepted, context).await {
                        Ok(Flow::EndService) => { stop.send_replace(true); }
                        Ok(Flow::Ended) => {}
                        Err(error) => count(&failures, error.kind()),
                    }
                });
            }
        }
    };
    running.abort_all();
    while running.join_next().await.is_some() {}
    let table = Arc::clone(&accepting.context.table);
    let stopped = tokio::task::spawn_blocking(move || table.stop()).await;
    let stopped = stopped
        .map_err(|error| HostError::os("stop the terminals", std::io::Error::other(error)))?;
    stopped.map_err(|error| HostError::os("stop the terminals", std::io::Error::other(error)))?;
    ended
}

/// Signals a running holder to stop; any number of handles may exist.
#[derive(Debug, Clone)]
pub struct Stopper(Arc<watch::Sender<bool>>);

impl Stopper {
    /// Asks the holder to stop; it ends every terminal on the way out.
    pub fn stop(&self) {
        self.0.send_replace(true);
    }
}

/// A running holder; [`Self::stop`] ends it and withdraws its record.
#[derive(Debug)]
pub struct HolderHost {
    runtime: tokio::runtime::Runtime,
    stop: Arc<watch::Sender<bool>>,
    task: tokio::task::JoinHandle<Result<(), HostError>>,
    record: HostRecord,
    config: HostConfig,
    failures: Failures,
}

fn refuse_live(config: &HostConfig, owner: &str) -> Result<(), HostError> {
    match observe(&config.paths, owner)?.0 {
        Observed::Running { record }
        | Observed::Unreachable {
            record: Some(record),
        } => Err(HostError::AlreadyRunning {
            holder: Some(Box::new(record.boot_proof)),
        }),
        Observed::Unreachable { record: None } => Err(HostError::AlreadyRunning { holder: None }),
        _ => Ok(()),
    }
}

impl HolderHost {
    /// Claims the session's pipe, publishes the record and starts accepting.
    ///
    /// # Errors
    ///
    /// [`HostError::AlreadyRunning`] while a holder lives or another process holds the
    /// pipe name, and the security, runtime, pipe or record failures.
    pub fn start(config: HostConfig) -> Result<Self, HostError> {
        let owner = pipe::current_user_sid()
            .map_err(|source| HostError::os("read this account's SID", source))?;
        let name = config.paths.pipe(&owner)?;
        refuse_live(&config, &owner)?;
        let security = Arc::new(
            OwnerSecurity::for_owner(&owner)
                .map_err(|source| HostError::os("build the pipe security", source))?,
        );
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("ovis-holder-host")
            .enable_all()
            .build()
            .map_err(|source| HostError::os("start the holder runtime", source))?;
        let server = {
            let _entered = runtime.enter();
            pipe::create_instance(&name, true, &security)
        }
        .map_err(|source| match source.kind() {
            std::io::ErrorKind::PermissionDenied => HostError::AlreadyRunning { holder: None },
            _ => HostError::os("create the holder pipe", source),
        })?;
        let proof = own_proof()?;
        let instance = HolderInstanceId::new(format!(
            "{}:{}",
            proof.pid.get(),
            proof.start_token.as_str()
        ));
        let record = HostRecord::new(config.paths.session().label(), &name, proof)
            .with_placement(config.placement);
        write_record(&config.paths.record(), &record)?;
        let build = HolderBuildId::new(format!(
            "ovis-terminal-holder/{}",
            env!("CARGO_PKG_VERSION")
        ));
        let context = Arc::new(LinkContext {
            owner,
            descriptor: HolderDescriptor::new(build, instance),
            table: Table::new(config.placement),
        });
        let (stop, _) = watch::channel(false);
        let stop = Arc::new(stop);
        let failures = Failures::default();
        let accepting = Accept {
            server,
            name,
            security,
            context,
            stop: Arc::clone(&stop),
            failures: Arc::clone(&failures),
        };
        let task = runtime.spawn(accept(accepting));
        Ok(Self {
            runtime,
            stop,
            task,
            record,
            config,
            failures,
        })
    }

    /// The record this holder published.
    #[must_use]
    pub const fn record(&self) -> &HostRecord {
        &self.record
    }

    /// Connection failures so far, by kind.
    #[must_use]
    pub fn failures(&self) -> BTreeMap<&'static str, u64> {
        lock(&self.failures).clone()
    }

    /// A handle that asks this holder to stop.
    #[must_use]
    pub fn stopper(&self) -> Stopper {
        Stopper(Arc::clone(&self.stop))
    }

    /// Blocks until a [`Stopper`] or a core's shutdown request asks the holder to stop.
    pub fn wait(&self) {
        let mut stopped = self.stop.subscribe();
        self.runtime.block_on(async move {
            // The sender lives in `self`, so the channel cannot close while this waits.
            stopped
                .wait_for(|stop| *stop)
                .await
                .map(drop)
                .unwrap_or_default();
        });
    }

    /// Stops accepting, ends every terminal and withdraws the record it published.
    ///
    /// # Errors
    ///
    /// The accept loop's failure, a terminal that could not be ended, or the record
    /// removal failure.
    pub fn stop(self) -> Result<(), HostError> {
        self.stop.send_replace(true);
        let served = self
            .runtime
            .block_on(self.task)
            .map_err(|error| HostError::os("join the holder", std::io::Error::other(error)))
            .and_then(|served| served);
        let withdrawn = withdraw_record(
            &self.config.paths.record(),
            &self.record.endpoint.boot_proof,
        );
        served.and(withdrawn)
    }
}
