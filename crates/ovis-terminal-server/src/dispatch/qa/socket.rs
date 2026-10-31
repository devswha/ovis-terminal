//! Private Unix endpoint driven with literal NDJSON.

use std::io::Write as _;
use std::os::unix::fs::DirBuilderExt as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ovis_terminal_authz::{Authority, AuthorityConfig, OwnerSpec, PrincipalId, RuntimeBinding};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, MachineId, RegistryIdentities, RuntimeId, RuntimeIdentity,
    ServerBootId,
};
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_server::conn::RequestHandler;
use ovis_terminal_server::listen::{ServerConfig, ServerHandle, UnixEndpoint, serve_unix};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;

use super::TestResult;

#[derive(Debug)]
pub(crate) struct Fixture {
    root: PathBuf,
    pub(crate) authority: Arc<Authority<RegistryIdentities>>,
    pub(crate) bearer: String,
}

impl Fixture {
    pub(crate) fn new() -> TestResult<Self> {
        let root = std::env::temp_dir().join(format!("ovt14-{}", uuid::Uuid::now_v7()));
        std::fs::DirBuilder::new().mode(0o700).create(&root)?;
        let runtime = runtime(1)?;
        let binding = RuntimeBinding::new(runtime.machine_id(), runtime.runtime_id());
        let owner = OwnerSpec::new(PrincipalId::parse("owner")?, TimestampMs::new(u64::MAX));
        let (authority, issued) = Authority::bootstrap(AuthorityConfig::new(binding, owner))?;
        Ok(Self {
            root,
            authority: Arc::new(authority),
            bearer: issued.bearer().to_owned(),
        })
    }

    pub(crate) fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    pub(crate) async fn serve<H: RequestHandler>(
        &self,
        boot: u64,
        handler: Arc<H>,
    ) -> TestResult<ServerHandle> {
        let endpoint = UnixEndpoint::bind(&self.path("s.sock")).await?;
        let config = ServerConfig::new(Arc::clone(&self.authority), runtime(boot)?);
        Ok(serve_unix(endpoint, config, handler)?)
    }

    pub(crate) async fn connect(&self) -> TestResult<Client> {
        Ok(Client(BufReader::new(
            UnixStream::connect(self.path("s.sock")).await?,
        )))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let removed = std::fs::remove_dir_all(&self.root);
        assert!(removed.is_ok(), "cleanup: {removed:?}");
    }
}

pub(crate) fn runtime(boot: u64) -> TestResult<RuntimeIdentity> {
    let machine: MachineId = "machine:0191f1a2-0000-7000-8000-000000000014".parse()?;
    Ok(RuntimeIdentity::new(
        HostIdentity::new(machine, HostBootId::try_from(1)?),
        RuntimeId::try_from(7)?,
        ServerBootId::try_from(boot)?,
    ))
}

pub(crate) fn request(id: &str, method: &str, params: &Value) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params})
}

pub(crate) fn at<'a>(value: &'a Value, pointer: &str) -> &'a Value {
    value.pointer(pointer).unwrap_or(&Value::Null)
}

pub(crate) fn error_kind(reply: &Value) -> &str {
    at(reply, "/error/data/kind").as_str().unwrap_or_default()
}

#[derive(Debug)]
pub(crate) struct Client(BufReader<UnixStream>);

impl Client {
    pub(crate) async fn send(&mut self, message: &Value) -> TestResult {
        let mut bytes = serde_json::to_vec(message)?;
        bytes.push(b'\n');
        self.0.get_mut().write_all(&bytes).await?;
        if message.get("method") != Some(&json!("hello")) {
            writeln!(std::io::stdout(), "-> {message}")?;
        }
        Ok(())
    }

    pub(crate) async fn hello(&mut self, generation: &str, bearer: &str) -> TestResult<Value> {
        self.send(&request(
            "hello",
            "hello",
            &json!({
                "generation": generation, "client":{"name":"dispatch-test","version":"1"},
                "credential":bearer,
            }),
        ))
        .await?;
        self.expect_next().await
    }

    pub(crate) async fn expect_next(&mut self) -> TestResult<Value> {
        let mut line = String::new();
        let bytes =
            tokio::time::timeout(Duration::from_secs(15), self.0.read_line(&mut line)).await??;
        if bytes == 0 {
            return Err("connection ended before reply".into());
        }
        let value = serde_json::from_str(&line)?;
        writeln!(std::io::stdout(), "<- {line}")?;
        Ok(value)
    }
}
