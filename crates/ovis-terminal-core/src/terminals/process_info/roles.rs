//! Launcher, foreground and background roles from one bounded process-table pass.
//!
//! A role is known only when its PID and start token were read together and the
//! launcher's terminal facts did not change during the pass; otherwise it is an
//! explicit unsupported value, never a guess and never evidence of exit.

use super::liveness::{Liveness, liveness};
use super::table::{NativeProcess, ProcessTable, Session, Terminal};
use crate::registry::{ProcessHandle, ProcessPid, TerminalId};
use crate::terminals::{Field, ProcessInfo, ProcessSample, Unavailable};

/// Most process-table entries one observation reads.
const TABLE_LIMIT: usize = 16384;

type Groups = (Vec<ProcessSample>, Vec<ProcessSample>);

fn sample(table: &dyn ProcessTable, native: &NativeProcess) -> Result<ProcessSample, Unavailable> {
    let pid = native.process.pid();
    let argv = table.argv(pid);
    let cwd = table.cwd(pid);
    let again = table.read(pid).map_err(|reason| match reason {
        Unavailable::ProcessUnavailable => reason,
        _ => Unavailable::Raced,
    })?;
    if again.process != native.process {
        return Err(Unavailable::Raced);
    }
    Ok(ProcessSample {
        process: native.process.clone(),
        argv,
        cwd,
    })
}

fn member(
    table: &dyn ProcessTable,
    pid: ProcessPid,
    root: &Session,
) -> Result<Option<(NativeProcess, Session)>, Unavailable> {
    match table.read(pid) {
        Ok(native) => match native.terminal {
            Terminal::Controlling(session) if session.same_terminal(root) && native.running => {
                Ok(Some((native, session)))
            }
            Terminal::Controlling(_) | Terminal::Detached | Terminal::Unsupported => Ok(None),
        },
        Err(Unavailable::ProcessUnavailable) => Ok(None),
        Err(reason) => Err(reason),
    }
}

fn groups(table: &dyn ProcessTable, root: Session) -> Result<Groups, Unavailable> {
    let pids = table.candidates(root.tty)?;
    if pids.len() > TABLE_LIMIT {
        return Err(Unavailable::Limit);
    }
    let (mut foreground, mut background) = (Vec::new(), Vec::new());
    for pid in pids {
        let Some((native, session)) = member(table, pid, &root)? else {
            continue;
        };
        let observed = match sample(table, &native) {
            Ok(observed) => observed,
            Err(Unavailable::ProcessUnavailable) => continue,
            Err(reason) => return Err(reason),
        };
        if session.group == root.foreground_group {
            foreground.push(observed);
        } else {
            background.push(observed);
        }
    }
    foreground.sort_by_key(|sample| sample.process.pid().get());
    background.sort_by_key(|sample| sample.process.pid().get());
    Ok((foreground, background))
}

fn session_groups(
    table: &dyn ProcessTable,
    launcher: &NativeProcess,
) -> Result<Groups, Unavailable> {
    let Terminal::Controlling(root) = launcher.terminal else {
        return Err(match launcher.terminal {
            Terminal::Unsupported => Unavailable::Platform,
            Terminal::Controlling(_) | Terminal::Detached => Unavailable::ProcessUnavailable,
        });
    };
    let observed = groups(table, root)?;
    match table.read(launcher.process.pid()) {
        Ok(after) if after.process == launcher.process && after.terminal == launcher.terminal => {
            Ok(observed)
        }
        Ok(_) | Err(_) => Err(Unavailable::Raced),
    }
}

/// Every role unavailable for one `reason`; the shell role is never inferred.
#[must_use]
pub const fn unavailable(terminal: TerminalId, reason: Unavailable) -> ProcessInfo {
    ProcessInfo {
        terminal,
        shell: Field::Unsupported(Unavailable::UndeclaredRole),
        launcher: Field::Unsupported(reason),
        foreground: Field::Unsupported(reason),
        background: Field::Unsupported(reason),
    }
}

/// Observes the roles of `terminal`, whose holder launched `owned`.
///
/// # Errors
///
/// Returns [`Liveness::Replaced`] or [`Liveness::Exited`] only when the table proves
/// the owned process no longer runs; unprovable facts are unsupported fields instead.
pub fn observe(
    table: &dyn ProcessTable,
    terminal: TerminalId,
    owned: &ProcessHandle,
) -> Result<ProcessInfo, Liveness> {
    match liveness(table, owned) {
        Liveness::Current => {}
        Liveness::Unknown(reason) => return Ok(unavailable(terminal, reason)),
        ended @ (Liveness::Replaced | Liveness::Exited) => return Err(ended),
    }
    let launcher = match table.read(owned.pid()) {
        Ok(native) if &native.process == owned => native,
        Ok(_) | Err(_) => return Ok(unavailable(terminal, Unavailable::Raced)),
    };
    let (foreground, background) = match session_groups(table, &launcher) {
        Ok((foreground, background)) => (Field::Known(foreground), Field::Known(background)),
        Err(reason) => (Field::Unsupported(reason), Field::Unsupported(reason)),
    };
    let launcher = sample(table, &launcher).map_or_else(Field::Unsupported, Field::Known);
    Ok(ProcessInfo {
        terminal,
        shell: Field::Unsupported(Unavailable::UndeclaredRole),
        launcher,
        foreground,
        background,
    })
}
