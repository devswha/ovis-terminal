//! Native proof is distinct from declared agent metadata and unknown platform fields.

use ovis_terminal_holder::process::StartTokenError;

use crate::registry::ProcessHandle;
use crate::terminals::process_info::{self, Liveness, NativeTable};
use crate::terminals::{Field, ProcessInfo, Record, TerminalError, Unavailable};

pub(super) fn current(owner: &ProcessHandle) -> Result<(), TerminalError> {
    match process_info::liveness(&NativeTable, owner) {
        Liveness::Current => Ok(()),
        Liveness::Replaced | Liveness::Exited => Err(TerminalError::ProcessReplaced),
        Liveness::Unknown(_) => Err(TerminalError::UnsupportedField),
    }
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
        return Ok(process_info::unavailable(
            record.id(),
            Unavailable::ProcessUnavailable,
        ));
    };
    process_info::observe(&NativeTable, record.id(), expected.handle()).map_err(|ended| match ended
    {
        Liveness::Exited => TerminalError::Process(StartTokenError::ProcessGone {
            pid: u32::try_from(expected.pid().get()).unwrap_or(u32::MAX),
        }),
        Liveness::Current | Liveness::Replaced | Liveness::Unknown(_) => {
            TerminalError::ProcessReplaced
        }
    })
}
