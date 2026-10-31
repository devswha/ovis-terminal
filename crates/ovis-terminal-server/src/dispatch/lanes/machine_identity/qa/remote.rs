//! The remote runtime: its own authority and identity behind the real connection layer,
//! reached through a child relay whose stdio is joined to two FIFOs.

use std::fs::{File, OpenOptions};
use std::os::unix::fs::DirBuilderExt as _;
use std::path::PathBuf;
use std::sync::Arc;

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
use tokio::net::unix::pipe;
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

pub(super) fn private_root(label: &str) -> TestResult<PathBuf> {
    let root = std::env::temp_dir().join(format!("ovt43-{label}-{}", uuid::Uuid::now_v7()));
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
            root: private_root("remote")?,
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
            let stream = tokio::io::join(
                pipe::Receiver::from_file(from_local)?,
                pipe::Sender::from_file(to_local)?,
            );
            let admission = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(0)))
                .admit(&PeerEvidence::Remote(RemoteTransport::Ssh));
            connections.serve(stream, admission).await?;
            Ok(())
        }));
        Ok(argv)
    }

    /// Waits, bounded, for every relayed connection to end once its link is gone.
    pub(super) async fn finish(self) -> TestResult {
        for serving in self.serving {
            tokio::time::timeout(std::time::Duration::from_secs(15), serving)
                .await??
                .map_err(|error| error.to_string())?;
        }
        std::fs::remove_dir_all(&self.root)?;
        Ok(())
    }
}
