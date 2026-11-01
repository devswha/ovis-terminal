//! A private directory, a real authenticated fixture server and a runner for the real binary.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use ovis_terminal_authz::{Authority, AuthorityConfig, OwnerSpec, PrincipalId, RuntimeBinding};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, MachineId, RuntimeId, RuntimeIdentity, ServerBootId,
};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_server::conn::{Call, RequestHandler};
use ovis_terminal_server::listen::{ServerConfig, ServerHandle, UnixEndpoint, serve_unix};
use serde_json::{Value, json};

/// Result of a test step; any failure fails the test.
pub type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// Machine identity every fixture server reports.
pub const MACHINE: &str = "machine:0191f1a2-0000-7000-8000-000000000020";
static NEXT_DIR: AtomicU32 = AtomicU32::new(0);

/// An owner-only directory removed on drop.
#[derive(Debug)]
pub struct Isolated {
    root: PathBuf,
    marker: String,
}

impl Isolated {
    /// Creates a fresh 0700 directory unique to this process and call.
    ///
    /// # Errors
    ///
    /// Fails when the directory cannot be created.
    pub fn new() -> TestResult<Self> {
        let unique = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let marker = format!("ovt20-{}-{nanos:x}-{unique}", std::process::id());
        // Not temp_dir(): on Darwin it is ~50 bytes deep, which pushes
        // `<root>/ovis-terminal/default.sock` past the 103-byte portable socket limit.
        let root = fixture_root().join(format!("ovt20-{}-{unique}", std::process::id()));
        std::os::unix::fs::DirBuilderExt::mode(&mut std::fs::DirBuilder::new(), 0o700)
            .create(&root)?;
        Ok(Self { root, marker })
    }

    /// The directory used as `XDG_RUNTIME_DIR` and `HOME`.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Every entry below the directory, sorted; an unchanged list means nothing was created.
    ///
    /// # Errors
    ///
    /// Fails when the directory cannot be read.
    pub fn entries(&self) -> TestResult<Vec<PathBuf>> {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
            for entry in std::fs::read_dir(dir)? {
                let path = entry?.path();
                if path.is_dir() {
                    walk(&path, out)?;
                }
                out.push(path);
            }
            Ok(())
        }
        let mut out = Vec::new();
        walk(&self.root, &mut out)?;
        out.sort();
        Ok(out)
    }

    /// Runs the binary with only this directory's environment plus `extra`.
    ///
    /// # Errors
    ///
    /// Fails when the binary cannot be spawned.
    pub async fn run(&self, args: &[&str], extra: &[(&str, &str)]) -> TestResult<Ran> {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_ovis-terminal"));
        command
            .args(args)
            .env_clear()
            .env("XDG_RUNTIME_DIR", &self.root)
            .env("HOME", &self.root)
            .env("OVT20_MARKER", &self.marker)
            .envs(extra.iter().copied())
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        Ok(Ran {
            argv: args.iter().map(|arg| (*arg).to_owned()).collect(),
            output: command.output().await?,
        })
    }

    /// Runs `connect` against `socket`, writing `input` to stdin and closing it.
    ///
    /// # Errors
    ///
    /// Fails when the binary cannot be spawned or fed.
    pub async fn bridge(&self, socket: &str, input: &[u8]) -> TestResult<Ran> {
        use tokio::io::AsyncWriteExt as _;
        let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_ovis-terminal"))
            .arg("connect")
            .env_clear()
            .env("HOME", &self.root)
            .env("OVIS_TERMINAL_SOCKET", socket)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let mut stdin = child.stdin.take().ok_or("stdin was not piped")?;
        stdin.write_all(input).await?;
        drop(stdin);
        Ok(Ran {
            argv: vec!["connect".to_owned()],
            output: child.wait_with_output().await?,
        })
    }

    /// Live processes carrying this directory's marker, i.e. anything the binary left behind.
    ///
    /// # Errors
    ///
    /// Fails when the process table cannot be listed.
    pub fn owned_processes(&self) -> TestResult<usize> {
        let needle = format!("OVT20_MARKER={}", self.marker);
        #[cfg(target_os = "linux")]
        {
            let mut environs = Vec::new();
            for entry in std::fs::read_dir("/proc")? {
                // A process that exits or is not ours has no readable environ; skip it.
                if let Ok(bytes) = std::fs::read(entry?.path().join("environ")) {
                    environs.push(bytes);
                }
            }
            Ok(environs
                .iter()
                .filter(|bytes| {
                    String::from_utf8_lossy(bytes)
                        .split('\0')
                        .any(|var| var == needle)
                })
                .count())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let listing = std::process::Command::new("ps")
                .args(["eww", "-ax", "-o", "command="])
                .output()?;
            let text = String::from_utf8_lossy(&listing.stdout);
            Ok(text.lines().filter(|line| line.contains(&needle)).count())
        }
    }
}

impl Drop for Isolated {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.root) {
            assert!(
                error.kind() == std::io::ErrorKind::NotFound || std::thread::panicking(),
                "cleanup failed: {error}"
            );
        }
    }
}

/// One finished invocation of the binary and the argv that produced it.
#[derive(Debug)]
#[non_exhaustive]
pub struct Ran {
    /// Arguments after the program name.
    pub argv: Vec<String>,
    /// Exit status and both captured streams.
    pub output: Output,
}

impl Ran {
    /// Checks the exit code, explaining a mismatch with argv, stdout and stderr.
    ///
    /// # Errors
    ///
    /// Fails when the process did not exit with `code`.
    pub fn exit(&self, code: i32) -> TestResult<&Output> {
        if self.output.status.code() == Some(code) {
            return Ok(&self.output);
        }
        Err(format!(
            "ovis-terminal {:?} exited {:?}, expected {code}\nstdout: {}\nstderr: {}",
            self.argv,
            self.output.status.code(),
            String::from_utf8_lossy(&self.output.stdout),
            String::from_utf8_lossy(&self.output.stderr),
        )
        .into())
    }
}

/// Answers `server.status` and lists two terminals.
#[derive(Debug)]
#[non_exhaustive]
pub struct Fixture;

impl RequestHandler for Fixture {
    fn methods(&self) -> Vec<&'static str> {
        vec!["server.status", "terminal.list"]
    }

    fn call(&self, call: Call) -> impl Future<Output = Result<Value, BoundaryError>> + Send {
        std::future::ready(match call.request.method() {
            "server.status" => Ok(json!({
                "state": "running",
                "runtime_lifetime": "lifetime-7",
                "server_boot_id": "boot-1",
                "uptime_ms": u64::MAX.to_string(),
                "holder_count": 1,
                "live_terminals": 2,
            })),
            _ => Ok(json!({
                "terminals": [
                    { "terminal_id": "term-a", "machine_id": MACHINE },
                    { "terminal_id": "term-b", "machine_id": MACHINE },
                ],
                "page": {},
                "retention_floor": "0",
            })),
        })
    }
}

/// A served fixture endpoint and the bearer that authenticates against it.
#[derive(Debug)]
#[non_exhaustive]
pub struct Served {
    /// The running server; shut it down to join its connections.
    pub handle: ServerHandle,
    /// The socket path.
    pub socket: PathBuf,
    /// Owner credential text.
    pub bearer: String,
}

/// Serves [`Fixture`] at `<dir>/run/s.sock` with a fresh authority.
///
/// # Errors
///
/// Fails when the authority, the socket or the server cannot be set up.
pub async fn serve(dir: &Isolated) -> TestResult<Served> {
    let machine: MachineId = MACHINE.parse()?;
    let runtime = RuntimeIdentity::new(
        HostIdentity::new(machine, HostBootId::try_from(1)?),
        RuntimeId::try_from(7)?,
        ServerBootId::try_from(1)?,
    );
    let binding = RuntimeBinding::new(runtime.machine_id(), runtime.runtime_id());
    let owner = OwnerSpec::new(PrincipalId::parse("owner")?, TimestampMs::new(u64::MAX));
    let (authority, issued) = Authority::bootstrap(AuthorityConfig::new(binding, owner))?;
    let socket = dir.root().join("run").join("s.sock");
    let endpoint = UnixEndpoint::bind(&socket).await?;
    let config = ServerConfig::new(Arc::new(authority), runtime);
    let handle = serve_unix(endpoint, config, Arc::new(Fixture))?;
    Ok(Served {
        handle,
        socket,
        bearer: issued.bearer().to_owned(),
    })
}

/// Where test state lives: `OVIS_FIXTURE_ROOT` when the host sets it, else `/tmp`, which
/// stays inside every platform's socket path limit.
fn fixture_root() -> PathBuf {
    std::env::var_os("OVIS_FIXTURE_ROOT").map_or_else(|| PathBuf::from("/tmp"), PathBuf::from)
}

/// The value at a JSON pointer such as `/error/kind`, or null when absent.
pub fn field<'a>(value: &'a Value, pointer: &str) -> &'a Value {
    value.pointer(pointer).unwrap_or(&Value::Null)
}

/// Parses one JSON document from a stream.
///
/// # Errors
///
/// Fails when the bytes are not one JSON document.
pub fn json(bytes: &[u8]) -> TestResult<Value> {
    Ok(serde_json::from_slice(bytes)?)
}
