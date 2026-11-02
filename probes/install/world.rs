//! One case's private world: home, XDG directories, attempt directory and the environment
//! every literal installer command runs in, plus the inventory of everything outside the
//! install directory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

use super::archive::sha256_file;
use super::error::InstallProbeError;

/// Files a shell profile edit or a service install would change; seeded so a change shows.
const HOME_SEEDS: [&str; 3] = [".profile", ".bashrc", ".zshrc"];

/// Worlds created by this process, for socket names unique within it.
static WORLDS: AtomicUsize = AtomicUsize::new(0);

/// The private directories and environment of one case.
pub(super) struct World {
    pub(super) root: PathBuf,
    pub(super) attempt: PathBuf,
    checkout: PathBuf,
    /// A private directory kept short below the temporary directory: a Unix socket path
    /// holds about 100 bytes.
    sockets: PathBuf,
    env: Vec<(String, String)>,
}

/// Creates `path` readable by its owner only, as a runtime directory must be.
fn mkdir(path: &Path) -> Result<(), InstallProbeError> {
    private_builder()
        .create(path)
        .map_err(InstallProbeError::io("create", path))
}

#[cfg(unix)]
fn private_builder() -> std::fs::DirBuilder {
    use std::os::unix::fs::DirBuilderExt as _;
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder
}

#[cfg(not(unix))]
fn private_builder() -> std::fs::DirBuilder {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    builder
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), InstallProbeError> {
    std::fs::write(path, bytes).map_err(InstallProbeError::io("write", path))
}

impl World {
    /// Creates `<scratch>/<case>` with seeded home and outside files; commands run in
    /// `checkout`, the directory holding `install.sh`.
    pub(super) fn create(
        scratch: &Path,
        case: &str,
        (checkout, urls): (&Path, &[(String, String)]),
    ) -> Result<Self, InstallProbeError> {
        let root = scratch.join(case);
        if root.exists() {
            std::fs::remove_dir_all(&root).map_err(InstallProbeError::io("clear", &root))?;
        }
        let attempt = root.join("attempt");
        let xdg = root.join("xdg");
        for dir in ["config", "data", "state", "cache", "runtime"] {
            mkdir(&xdg.join(dir))?;
        }
        mkdir(&root.join("home"))?;
        mkdir(&root.join("tmp"))?;
        mkdir(&attempt.join("outside"))?;
        for seed in HOME_SEEDS {
            write(
                &root.join("home").join(seed),
                b"# untouched by install.sh\n",
            )?;
        }
        write(
            &attempt.join("outside").join("sentinel.txt"),
            b"outside the install dir\n",
        )?;
        let text = |path: &Path| path.to_string_lossy().into_owned();
        let index = WORLDS.fetch_add(1, Ordering::Relaxed);
        let sockets = std::env::temp_dir().join(format!("oip-{}-{index}", std::process::id()));
        mkdir(&sockets)?;
        let socket = sockets.join("s.sock");
        let mut env = vec![
            ("PATH".to_owned(), std::env::var("PATH").unwrap_or_default()),
            ("HOME".to_owned(), text(&root.join("home"))),
            ("TMPDIR".to_owned(), text(&root.join("tmp"))),
            ("ATTEMPT_DIR".to_owned(), text(&attempt)),
            ("OVIS_TERMINAL_SOCKET".to_owned(), text(&socket)),
        ];
        for (name, dir) in [
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_STATE_HOME", "state"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_RUNTIME_DIR", "runtime"),
        ] {
            env.push((name.to_owned(), text(&xdg.join(dir))));
        }
        env.extend(urls.iter().cloned());
        Ok(Self {
            root,
            attempt,
            checkout: checkout.to_path_buf(),
            sockets,
            env,
        })
    }

    /// The install directory every command names: `$ATTEMPT_DIR/install`.
    pub(super) fn install(&self) -> PathBuf {
        self.attempt.join("install")
    }

    /// `sh -c <command>` in the checkout with only this world's environment, in its own
    /// process group so a cancellation reaches the whole pipeline.
    pub(super) fn command(&self, command: &str) -> Command {
        let mut built = Command::new("sh");
        built
            .arg("-c")
            .arg(command)
            .current_dir(&self.checkout)
            .env_clear()
            .envs(self.env.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        isolate(&mut built);
        built
    }

    /// Starts `command` with piped stdin, for a process the probe ends by closing it.
    pub(super) fn spawn_held(&self, command: &str) -> Result<Child, InstallProbeError> {
        self.command(command)
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|source| InstallProbeError::Spawn {
                program: command.to_owned(),
                source,
            })
    }

    /// Every path outside the install directory a run could change: the home and XDG
    /// trees, the attempt directory without `install/`, and the checkout's top level.
    pub(super) fn outside(&self) -> Result<BTreeMap<String, String>, InstallProbeError> {
        let mut seen = BTreeMap::new();
        let skip = self.install();
        for (label, dir) in [
            ("home", self.root.join("home")),
            ("xdg", self.root.join("xdg")),
            ("attempt", self.attempt.clone()),
        ] {
            walk(&dir, (label, &skip), &mut seen)?;
        }
        let entries = std::fs::read_dir(&self.checkout)
            .map_err(InstallProbeError::io("list", &self.checkout))?;
        for entry in entries {
            let entry = entry.map_err(InstallProbeError::io("list", &self.checkout))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let fact = if path.is_file() {
                sha256_file(&path)?
            } else {
                "dir".to_owned()
            };
            seen.insert(format!("checkout/{name}"), fact);
        }
        Ok(seen)
    }
}

/// Every directory, file digest and link below `dir`, keyed `<label>/<relative path>`.
pub(super) fn inventory(
    dir: &Path,
    label: &str,
) -> Result<BTreeMap<String, String>, InstallProbeError> {
    let mut seen = BTreeMap::new();
    walk(dir, (label, Path::new("")), &mut seen)?;
    Ok(seen)
}

impl Drop for World {
    fn drop(&mut self) {
        // Holds the runtime socket and its state when a case served one.
        if let Err(error) = std::fs::remove_dir_all(&self.sockets) {
            tracing::warn!(%error, path = %self.sockets.display(), "cannot remove a probe socket directory");
        }
    }
}

fn walk(
    dir: &Path,
    (label, skip): (&str, &Path),
    seen: &mut BTreeMap<String, String>,
) -> Result<(), InstallProbeError> {
    let entries = std::fs::read_dir(dir).map_err(InstallProbeError::io("list", dir))?;
    for entry in entries {
        let path = entry.map_err(InstallProbeError::io("list", dir))?.path();
        if path == skip {
            continue;
        }
        let name = format!(
            "{label}/{}",
            path.file_name().unwrap_or_default().to_string_lossy()
        );
        let meta =
            std::fs::symlink_metadata(&path).map_err(InstallProbeError::io("stat", &path))?;
        if meta.is_dir() {
            seen.insert(name.clone(), "dir".to_owned());
            walk(&path, (&name, skip), seen)?;
        } else if meta.is_file() {
            seen.insert(name, sha256_file(&path)?);
        } else {
            seen.insert(name, "link".to_owned());
        }
    }
    Ok(())
}

/// The paths two inventories disagree on.
pub(super) fn changed(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> Value {
    let keys: std::collections::BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    json!(
        keys.into_iter()
            .filter(|key| before.get(*key) != after.get(*key))
            .collect::<Vec<_>>()
    )
}

#[cfg(unix)]
fn isolate(command: &mut Command) {
    use std::os::unix::process::CommandExt as _;
    command.process_group(0);
}

#[cfg(not(unix))]
const fn isolate(_command: &mut Command) {}
