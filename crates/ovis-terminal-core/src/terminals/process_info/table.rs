//! The process-table seam: the native operating-system table, or a scripted one in tests.

use std::path::PathBuf;

use ovis_terminal_holder::process::{self as native, Membership, ProbeError, ProcessId};

use crate::registry::{ProcessHandle, ProcessPid, ProcessStartToken};
use crate::terminals::{Field, Unavailable};

/// The controlling-terminal facts of one process.
///
/// A process whose terminal was closed can keep naming a device number that a newer
/// terminal reuses, so a process belongs to a terminal only when both its session and
/// its terminal device match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Session {
    /// Session of the process.
    pub session: u32,
    /// Device number of its controlling terminal.
    pub tty: u64,
    /// Process group of the process.
    pub group: u32,
    /// Foreground process group of that terminal.
    pub foreground_group: u32,
}

impl Session {
    /// The session leader's facts: session `session` on terminal `tty`, with the leader's
    /// own group in the foreground.
    #[must_use]
    pub const fn new(session: u32, tty: u64) -> Self {
        Self {
            session,
            tty,
            group: session,
            foreground_group: session,
        }
    }

    /// The same session and terminal for a member of `group` while `foreground_group`
    /// holds the terminal.
    #[must_use]
    pub const fn with_groups(self, group: u32, foreground_group: u32) -> Self {
        Self {
            group,
            foreground_group,
            ..self
        }
    }

    /// Whether this and `other` are the same session on the same terminal device.
    #[must_use]
    pub const fn same_terminal(&self, other: &Self) -> bool {
        self.session == other.session && self.tty == other.tty
    }
}

/// Whether a process belongs to a controlling terminal, as far as the OS can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Terminal {
    /// The process has this controlling terminal.
    Controlling(Session),
    /// The process has no controlling terminal.
    Detached,
    /// The operating system does not expose terminal membership.
    Unsupported,
}

/// One process-table record: PID and start token read together, run state, membership.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct NativeProcess {
    /// PID and start token.
    pub process: ProcessHandle,
    /// False once the process exited, even while its record remains.
    pub running: bool,
    /// Controlling-terminal membership.
    pub terminal: Terminal,
}

impl NativeProcess {
    /// A record of `process` with its run state and membership.
    #[must_use]
    pub const fn new(process: ProcessHandle, running: bool, terminal: Terminal) -> Self {
        Self {
            process,
            running,
            terminal,
        }
    }
}

/// Read access to a process table; every failure names why a field is unavailable.
pub trait ProcessTable {
    /// Reads one record; a PID that names no process is `process_unavailable`.
    ///
    /// # Errors
    ///
    /// Returns the reason the record could not be read.
    fn read(&self, pid: ProcessPid) -> Result<NativeProcess, Unavailable>;

    /// Lists the PIDs that may have `tty` as their controlling terminal; each one's
    /// membership is re-checked by reading its record.
    ///
    /// # Errors
    ///
    /// Returns the reason the table could not be enumerated.
    fn candidates(&self, tty: u64) -> Result<Vec<ProcessPid>, Unavailable>;

    /// Reads the current argument vector.
    fn argv(&self, pid: ProcessPid) -> Field<Vec<String>>;

    /// Reads the current working directory.
    fn cwd(&self, pid: ProcessPid) -> Field<PathBuf>;
}

/// The operating system's own process table.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct NativeTable;

const fn reason(error: &ProbeError) -> Unavailable {
    match error {
        ProbeError::Gone { .. } => Unavailable::ProcessUnavailable,
        ProbeError::Permission { .. } => Unavailable::Permission,
        ProbeError::Unsupported => Unavailable::Platform,
        ProbeError::Malformed => Unavailable::Encoding,
        _ => Unavailable::Raced,
    }
}

fn os_pid(pid: ProcessPid) -> Result<ProcessId, Unavailable> {
    u32::try_from(pid.get())
        .map(ProcessId::new)
        .map_err(|_| Unavailable::ProcessUnavailable)
}

fn field<T>(read: Result<T, ProbeError>) -> Field<T> {
    read.map_or_else(|error| Field::Unsupported(reason(&error)), Field::Known)
}

impl ProcessTable for NativeTable {
    fn read(&self, pid: ProcessPid) -> Result<NativeProcess, Unavailable> {
        let entry = native::read_entry(os_pid(pid)?).map_err(|error| reason(&error))?;
        let token = ProcessStartToken::new(entry.start_token.as_str())
            .map_err(|_| Unavailable::Encoding)?;
        let terminal = match entry.terminal {
            Membership::Controlling(member) => Terminal::Controlling(
                Session::new(member.session, member.tty)
                    .with_groups(member.group, member.foreground_group),
            ),
            Membership::Detached => Terminal::Detached,
            _ => Terminal::Unsupported,
        };
        Ok(NativeProcess::new(
            ProcessHandle::new(pid, token),
            entry.running,
            terminal,
        ))
    }

    fn candidates(&self, tty: u64) -> Result<Vec<ProcessPid>, Unavailable> {
        let listed = native::list_terminal_pids(tty).map_err(|error| reason(&error))?;
        Ok(listed
            .into_iter()
            .filter_map(|pid| ProcessPid::try_from(u64::from(pid.get())).ok())
            .collect())
    }

    fn argv(&self, pid: ProcessPid) -> Field<Vec<String>> {
        os_pid(pid).map_or_else(Field::Unsupported, |pid| field(native::read_argv(pid)))
    }

    fn cwd(&self, pid: ProcessPid) -> Field<PathBuf> {
        os_pid(pid).map_or_else(Field::Unsupported, |pid| field(native::read_cwd(pid)))
    }
}
