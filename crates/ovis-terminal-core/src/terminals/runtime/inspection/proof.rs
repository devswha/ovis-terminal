//! Native proof is distinct from declared agent metadata and unknown platform fields.

use ovis_terminal_holder::process::{ProcessId, StartTokenError, read_start_token};

use crate::registry::ProcessHandle;
use crate::terminals::{Field, ProcessInfo, Record, TerminalError, Unavailable};

pub(super) fn current(owner: &ProcessHandle) -> Result<(), TerminalError> {
    let pid = ProcessId::new(
        u32::try_from(owner.pid().get()).map_err(|_| TerminalError::ProcessReplaced)?,
    );
    match read_start_token(pid) {
        Ok(token) if token.as_str() == owner.start_token().as_str() => {}
        Ok(_) | Err(StartTokenError::ProcessGone { .. }) => {
            return Err(TerminalError::ProcessReplaced);
        }
        Err(error) => return Err(error.into()),
    }
    #[cfg(target_os = "linux")]
    match super::linux::alive(pid.get()) {
        Ok(true) => {}
        Ok(false) | Err(Unavailable::ProcessUnavailable) => {
            return Err(TerminalError::ProcessReplaced);
        }
        Err(_) => return Err(TerminalError::UnsupportedField),
    }
    Ok(())
}

pub(super) fn includes(info: &ProcessInfo, owner: &ProcessHandle) -> Result<bool, TerminalError> {
    match current(owner) {
        Ok(()) => {}
        Err(TerminalError::ProcessReplaced) => return Ok(false),
        Err(error) => return Err(error),
    }
    let mut unknown = false;
    match &info.launcher {
        Field::Known(sample) if &sample.process == owner => return Ok(true),
        Field::Known(_) => {}
        Field::Unsupported(_) => unknown = true,
    }
    for group in [&info.foreground, &info.background] {
        match group {
            Field::Known(samples) if samples.iter().any(|sample| &sample.process == owner) => {
                return Ok(true);
            }
            Field::Known(_) => {}
            Field::Unsupported(_) => unknown = true,
        }
    }
    if unknown {
        Err(TerminalError::UnsupportedField)
    } else {
        Ok(false)
    }
}

pub(super) fn native_info(record: &Record) -> Result<ProcessInfo, TerminalError> {
    let Some(expected) = record.lifecycle.process() else {
        return Ok(ProcessInfo {
            terminal: record.id(),
            shell: Field::Unsupported(Unavailable::UndeclaredRole),
            launcher: Field::Unsupported(Unavailable::ProcessUnavailable),
            foreground: Field::Unsupported(Unavailable::ProcessUnavailable),
            background: Field::Unsupported(Unavailable::ProcessUnavailable),
        });
    };
    let pid = ProcessId::new(
        u32::try_from(expected.pid().get()).map_err(|_| TerminalError::ProcessReplaced)?,
    );
    let token = read_start_token(pid)?;
    if token.as_str() != expected.start_token().as_str() {
        return Err(TerminalError::ProcessReplaced);
    }
    #[cfg(target_os = "linux")]
    {
        Ok(super::linux::inspect(record.id(), expected))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(ProcessInfo {
            terminal: record.id(),
            shell: Field::Unsupported(Unavailable::UndeclaredRole),
            launcher: Field::Known(crate::terminals::ProcessSample {
                process: expected.handle().clone(),
                argv: Field::Unsupported(Unavailable::Platform),
                cwd: Field::Unsupported(Unavailable::Platform),
            }),
            foreground: Field::Unsupported(Unavailable::Platform),
            background: Field::Unsupported(Unavailable::Platform),
        })
    }
}
