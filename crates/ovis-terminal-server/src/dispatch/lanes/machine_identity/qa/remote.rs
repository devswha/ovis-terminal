//! The remote runtime: its own authority and identity behind the real connection layer,
//! reached through a child relay whose stdio is joined to two FIFOs.

use std::fs::{File, OpenOptions};
use std::os::unix::fs::DirBuilderExt as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ovis_terminal_authz::{
    Authority, AuthorityConfig, EndpointPolicy, LocalAccount, OwnerSpec, PeerEvidence, PrincipalId,
    RemoteTransport, RuntimeBinding, UnixUid,
};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, MachineId, RuntimeId, RuntimeIdentity, ServerBootId,
};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use serde_json::Value;
use tokio::task::JoinHandle;

use super::TestResult;
use crate::conn::{Call, Connections, RequestHandler};

pub(super) const MACHINE: &str = "machine:0191f1a2-0000-7000-8000-00000000004b";
pub(super) const BOOT: &str = "boot:5";

/// A remote that serves no methods: the handshake is all these tests need from it.
#[derive(Debug)]
struct NoMethods;

impl RequestHandler for NoMethods {
    fn methods(&self) -> Vec<&'static str> {
        Vec::new()
    }

    fn call(&self, _: Call) -> impl Future<Output = Result<Value, BoundaryError>> + Send {
        std::future::ready(Err(BoundaryError::method_not_found()))
    }
}

pub(super) fn identity(machine: &str, boot: u64) -> TestResult<RuntimeIdentity> {
    let machine: MachineId = machine.parse()?;
    Ok(RuntimeIdentity::new(
        HostIdentity::new(machine, HostBootId::try_from(1)?),
        RuntimeId::try_from(43)?,
        ServerBootId::try_from(boot)?,
    ))
}

pub(super) fn bootstrap(
    runtime: RuntimeIdentity,
) -> TestResult<(
    Authority<ovis_terminal_core::registry::RegistryIdentities>,
    String,
)> {
    let binding = RuntimeBinding::new(runtime.machine_id(), runtime.runtime_id());
    let owner = OwnerSpec::new(PrincipalId::parse("owner")?, TimestampMs::new(u64::MAX));
    let (authority, issued) = Authority::bootstrap(AuthorityConfig::new(binding, owner))?;
    Ok((authority, issued.bearer().to_owned()))
}

/// A private root short enough for a socket path on every platform (Darwin allows 104
/// bytes): `/tmp/ovt43-<pid>-<n>`, unique per process and call.
pub(super) fn private_root() -> TestResult<PathBuf> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let index = NEXT.fetch_add(1, Ordering::Relaxed);
    let root = PathBuf::from(format!("/tmp/ovt43-{}-{index}", std::process::id()));
    std::fs::DirBuilder::new().mode(0o700).create(&root)?;
    Ok(root)
}

#[derive(Debug)]
pub(super) struct Remote {
    root: PathBuf,
    connections: Arc<Connections<NoMethods>>,
    serving: Vec<JoinHandle<TestResult>>,
    pub(super) bearer: String,
}

impl Remote {
    pub(super) fn start() -> TestResult<Self> {
        let runtime = identity(MACHINE, 5)?;
        let (authority, bearer) = bootstrap(runtime)?;
        let connections = Connections::new(Arc::new(authority), runtime, Arc::new(NoMethods))?;
        Ok(Self {
            root: private_root()?,
            connections: Arc::new(connections),
            serving: Vec::new(),
            bearer,
        })
    }

    /// A transport argv whose child carries stdin to the remote and the remote to stdout.
    pub(super) fn relay(&mut self) -> TestResult<Vec<String>> {
        let index = self.serving.len();
        let up = self.root.join(format!("up{index}"));
        let down = self.root.join(format!("down{index}"));
        for fifo in [&up, &down] {
            if !std::process::Command::new("mkfifo")
                .arg(fifo)
                .status()?
                .success()
            {
                return Err("mkfifo failed".into());
            }
        }
        let argv = vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "cat < \"$2\" & exec cat > \"$1\"".to_owned(),
            "relay".to_owned(),
            up.display().to_string(),
            down.display().to_string(),
        ];
        let connections = Arc::clone(&self.connections);
        self.serving.push(tokio::spawn(async move {
            // Each open blocks until the relay child opens the other end of that FIFO.
            let (from_local, to_local) = tokio::task::spawn_blocking(move || {
                Ok::<_, std::io::Error>((
                    File::open(&up)?,
                    OpenOptions::new().write(true).open(&down)?,
                ))
            })
            .await??;
            // Blocking reads see the FIFO's end of file on every platform; kqueue readiness
            // on a Darwin FIFO did not report it once the relay exited.
            let stream = tokio::io::join(
                tokio::fs::File::from_std(from_local),
                tokio::fs::File::from_std(to_local),
            );
            let admission = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(0)))
                .admit(&PeerEvidence::Remote(RemoteTransport::Ssh));
            connections.serve(stream, admission).await?;
            Ok(())
        }));
        Ok(argv)
    }

    /// Waits, bounded, for every relayed connection to end once its link is gone; on
    /// expiry it names the relay and the processes still running under the fixture root.
    pub(super) async fn finish(self) -> TestResult {
        const BOUND: std::time::Duration = std::time::Duration::from_secs(15);
        for (index, serving) in self.serving.into_iter().enumerate() {
            let Ok(joined) = tokio::time::timeout(BOUND, serving).await else {
                let table = std::process::Command::new("ps")
                    .args(["-axo", "pid,ppid,stat,command"])
                    .output()?;
                let root = self.root.display().to_string();
                let live: Vec<String> = String::from_utf8_lossy(&table.stdout)
                    .lines()
                    .filter(|line| line.contains(&root))
                    .map(str::to_owned)
                    .collect();
                return Err(
                    format!("relay {index} still served after {BOUND:?}; live: {live:?}").into(),
                );
            };
            joined?.map_err(|error| error.to_string())?;
        }
        std::fs::remove_dir_all(&self.root)?;
        Ok(())
    }
}
