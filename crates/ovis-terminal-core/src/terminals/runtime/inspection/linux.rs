//! Bounded Linux process-table observations tied to one controlling PTY session.

use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use ovis_terminal_holder::process::{ProcessId, read_start_token};

use crate::registry::{ProcessHandle, ProcessIdentity, ProcessPid, ProcessStartToken, TerminalId};
use crate::terminals::{Field, ProcessInfo, ProcessSample, Unavailable};

struct Native {
    alive: bool,
    group: i64,
    session: i64,
    tty: i64,
    foreground: i64,
}

fn native(pid: u32) -> Result<Native, Unavailable> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|error| unavailable(error.kind()))?;
    let (_, fields) = stat.rsplit_once(')').ok_or(Unavailable::Encoding)?;
    let fields: Vec<_> = fields.split_whitespace().take(6).collect();
    let read = |index: usize| {
        fields
            .get(index)
            .ok_or(Unavailable::Encoding)?
            .parse::<i64>()
            .map_err(|_| Unavailable::Encoding)
    };
    Ok(Native {
        alive: match fields.first().copied() {
            Some("Z" | "X" | "x") => false,
            Some("R" | "S" | "D" | "T" | "t" | "K" | "W" | "P" | "I") => true,
            _ => return Err(Unavailable::Encoding),
        },
        group: read(2)?,
        session: read(3)?,
        tty: read(4)?,
        foreground: read(5)?,
    })
}

const fn unavailable(kind: ErrorKind) -> Unavailable {
    match kind {
        ErrorKind::PermissionDenied => Unavailable::Permission,
        ErrorKind::NotFound => Unavailable::ProcessUnavailable,
        _ => Unavailable::Raced,
    }
}

pub(super) fn alive(pid: u32) -> Result<bool, Unavailable> {
    Ok(native(pid)?.alive)
}

fn sample(pid: u32) -> Result<(Native, ProcessSample), Unavailable> {
    let first =
        read_start_token(ProcessId::new(pid)).map_err(|_| Unavailable::ProcessUnavailable)?;
    let membership = native(pid)?;
    let argv = match fs::read(format!("/proc/{pid}/cmdline")) {
        Ok(bytes) if bytes.is_empty() => Field::Known(Vec::new()),
        Ok(bytes) => bytes
            .strip_suffix(&[0])
            .ok_or(Unavailable::Encoding)?
            .split(|byte| *byte == 0)
            .map(|arg| String::from_utf8(arg.to_vec()))
            .collect::<Result<Vec<_>, _>>()
            .map_or(Field::Unsupported(Unavailable::Encoding), Field::Known),
        Err(error) => Field::Unsupported(unavailable(error.kind())),
    };
    let cwd = match fs::read_link(format!("/proc/{pid}/cwd")) {
        Ok(path) if path.to_str().is_some() => Field::Known(path),
        Ok(_) => Field::Unsupported(Unavailable::Encoding),
        Err(error) => Field::Unsupported(unavailable(error.kind())),
    };
    let second = read_start_token(ProcessId::new(pid)).map_err(|_| Unavailable::Raced)?;
    if first != second {
        return Err(Unavailable::Raced);
    }
    Ok((
        membership,
        ProcessSample {
            process: ProcessHandle::new(
                ProcessPid::try_from(u64::from(pid)).map_err(|_| Unavailable::Encoding)?,
                ProcessStartToken::new(first.as_str()).map_err(|_| Unavailable::Encoding)?,
            ),
            argv,
            cwd,
        },
    ))
}

fn groups(root: &Native) -> Result<(Vec<ProcessSample>, Vec<ProcessSample>), Unavailable> {
    let mut foreground = Vec::new();
    let mut background = Vec::new();
    for (index, entry) in fs::read_dir(PathBuf::from("/proc"))
        .map_err(|error| unavailable(error.kind()))?
        .enumerate()
    {
        if index > 16384 {
            return Err(Unavailable::Limit);
        }
        let entry = entry.map_err(|error| unavailable(error.kind()))?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let observed = match native(pid) {
            Ok(native) => native,
            Err(Unavailable::ProcessUnavailable) => continue,
            Err(error) => return Err(error),
        };
        if observed.session != root.session || observed.tty != root.tty {
            continue;
        }
        let (observed, process) = sample(pid)?;
        if observed.session != root.session || observed.tty != root.tty {
            return Err(Unavailable::Raced);
        }
        if observed.group == root.foreground {
            foreground.push(process);
        } else {
            background.push(process);
        }
    }
    foreground.sort_by_key(|sample| sample.process.pid().get());
    background.sort_by_key(|sample| sample.process.pid().get());
    Ok((foreground, background))
}

pub(super) fn inspect(terminal: TerminalId, expected: &ProcessIdentity) -> ProcessInfo {
    let observed = u32::try_from(expected.pid().get())
        .map_err(|_| Unavailable::Encoding)
        .and_then(|pid| {
            let (root, launcher) = sample(pid)?;
            if launcher.process.start_token() != expected.start_token() {
                return Err(Unavailable::Raced);
            }
            let groups = groups(&root);
            let after = native(pid)?;
            let token = read_start_token(ProcessId::new(pid)).map_err(|_| Unavailable::Raced)?;
            if token.as_str() != expected.start_token().as_str() {
                return Err(Unavailable::Raced);
            }
            if (root.session, root.tty, root.foreground)
                != (after.session, after.tty, after.foreground)
            {
                return Err(Unavailable::Raced);
            }
            Ok((launcher, groups))
        });
    let (launcher, foreground, background) = match observed {
        Ok((launcher, Ok((foreground, background)))) => (
            Field::Known(launcher),
            Field::Known(foreground),
            Field::Known(background),
        ),
        Ok((launcher, Err(reason))) => (
            Field::Known(launcher),
            Field::Unsupported(reason),
            Field::Unsupported(reason),
        ),
        Err(reason) => (
            Field::Unsupported(reason),
            Field::Unsupported(reason),
            Field::Unsupported(reason),
        ),
    };
    ProcessInfo {
        terminal,
        shell: Field::Unsupported(Unavailable::UndeclaredRole),
        launcher,
        foreground,
        background,
    }
}
