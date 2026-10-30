//! Starting one program on a fresh PTY with explicit argv, cwd, environment and grid.

use std::ffi::OsString;
use std::fs::File;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::Path;
use std::process::Child as StdChild;

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, SlavePty, native_pty_system};

use super::config::ProcessEnvironment;
use super::launch_error::LaunchError;
use super::pty_io::set_nonblocking;
use crate::process::{ProcessId, read_start_token};
use crate::protocol::{ProcessStartToken, TerminalId, TerminalIncarnation};

/// Terminal geometry with nonzero columns and rows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Grid {
    cols: u16,
    rows: u16,
}

impl Grid {
    /// Creates a grid.
    ///
    /// # Errors
    ///
    /// Returns [`LaunchError::InvalidGrid`] when either dimension is zero.
    pub const fn new(cols: u16, rows: u16) -> Result<Self, LaunchError> {
        if cols == 0 || rows == 0 {
            return Err(LaunchError::InvalidGrid { cols, rows });
        }
        Ok(Self { cols, rows })
    }

    /// Returns the column count.
    #[must_use]
    pub const fn cols(self) -> u16 {
        self.cols
    }

    /// Returns the row count.
    #[must_use]
    pub const fn rows(self) -> u16 {
        self.rows
    }

    pub(super) const fn pty_size(self) -> PtySize {
        PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

/// Everything a program starts with; nothing is inherited from the holder.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct LaunchSpec {
    /// Program and arguments; the first element names the program.
    pub argv: Vec<OsString>,
    /// Working directory and complete environment.
    pub environment: ProcessEnvironment,
    /// Initial terminal geometry.
    pub grid: Grid,
}

impl LaunchSpec {
    /// Creates a launch specification.
    #[must_use]
    pub const fn new(argv: Vec<OsString>, environment: ProcessEnvironment, grid: Grid) -> Self {
        Self {
            argv,
            environment,
            grid,
        }
    }
}

/// A launch bound to the terminal identity that will own the process.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct LaunchRequest {
    /// Stable terminal identity, unused by any record the holder retains.
    pub terminal_id: TerminalId,
    /// Terminal incarnation receiving the process.
    pub terminal_incarnation: TerminalIncarnation,
    /// Program specification.
    pub spec: LaunchSpec,
}

impl LaunchRequest {
    /// Creates a launch request.
    #[must_use]
    pub const fn new(
        terminal_id: TerminalId,
        terminal_incarnation: TerminalIncarnation,
        spec: LaunchSpec,
    ) -> Self {
        Self {
            terminal_id,
            terminal_incarnation,
            spec,
        }
    }
}

/// A started program and the PTY descriptors its worker threads own.
pub(super) struct PtyProcess {
    pub(super) child: StdChild,
    pub(super) pid: ProcessId,
    pub(super) start_token: ProcessStartToken,
    pub(super) master: Box<dyn MasterPty + Send>,
    pub(super) slave: Box<dyn SlavePty + Send>,
    pub(super) reader: File,
    pub(super) writer: File,
    pub(super) probe: OwnedFd,
}

/// Starts `spec` on a new PTY.
///
/// Reader and writer are plain duplicates of the non-blocking master descriptor:
/// the PTY library's own writer sends a newline and EOF when dropped, which
/// would inject input.
pub(super) fn launch(spec: &LaunchSpec) -> Result<PtyProcess, LaunchError> {
    let program = spec.argv.first().ok_or(LaunchError::EmptyArgv)?;
    let cwd = &spec.environment.cwd;
    let metadata = std::fs::metadata(cwd).map_err(|source| LaunchError::CwdUnavailable {
        cwd: cwd.clone(),
        source,
    })?;
    if !metadata.is_dir() {
        return Err(LaunchError::CwdNotDirectory { cwd: cwd.clone() });
    }
    // A program given as a path is checked here so its OS error is kept; bare
    // names are resolved by the PTY library through the explicit PATH.
    let program = Path::new(program);
    if program
        .parent()
        .is_some_and(|parent| !parent.as_os_str().is_empty())
    {
        let path = cwd.join(program);
        std::fs::metadata(&path)
            .map_err(|source| LaunchError::ExecutableUnavailable { path, source })?;
    }
    let pair = native_pty_system()
        .openpty(spec.grid.pty_size())
        .map_err(|error| LaunchError::PtyUnavailable {
            source: error.into(),
        })?;
    let reader = File::from(duplicate_master(pair.master.as_ref())?);
    let writer = File::from(duplicate_master(pair.master.as_ref())?);
    let probe = duplicate_master(pair.master.as_ref())?;
    set_nonblocking(probe.as_fd()).map_err(|error| LaunchError::PtyUnavailable {
        source: error.into(),
    })?;
    let mut command = CommandBuilder::from_argv(spec.argv.clone());
    command.env_clear();
    for (name, value) in &spec.environment.vars {
        command.env(name, value);
    }
    command.cwd(cwd);
    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| LaunchError::from_spawn(error.into()))?;
    let mut child = into_std_child(child)?;
    let pid = ProcessId::new(child.id());
    let start_token = match read_start_token(pid) {
        Ok(token) => token,
        Err(source) => {
            let cleanup = child.kill().and_then(|()| child.wait().map(drop)).err();
            return Err(LaunchError::StartToken { source, cleanup });
        }
    };
    Ok(PtyProcess {
        child,
        pid,
        start_token,
        master: pair.master,
        slave: pair.slave,
        reader,
        writer,
        probe,
    })
}

fn duplicate_master(master: &dyn MasterPty) -> Result<OwnedFd, LaunchError> {
    let raw = master
        .as_raw_fd()
        .ok_or(LaunchError::PtyDescriptorMissing)?;
    // SAFETY: `raw` is the open master descriptor owned by `master`, which is
    // borrowed for the whole lifetime of `borrowed`.
    let borrowed = unsafe { BorrowedFd::borrow_raw(raw) };
    borrowed
        .try_clone_to_owned()
        .map_err(|error| LaunchError::PtyUnavailable {
            source: error.into(),
        })
}

fn into_std_child(child: Box<dyn Child + Send + Sync>) -> Result<StdChild, LaunchError> {
    let child: Box<dyn Child> = child;
    match child.downcast::<StdChild>() {
        Ok(child) => Ok(*child),
        Err(mut unsupported) => {
            let cleanup = unsupported
                .kill()
                .and_then(|()| unsupported.wait().map(drop))
                .err();
            Err(LaunchError::UnsupportedChild { cleanup })
        }
    }
}
