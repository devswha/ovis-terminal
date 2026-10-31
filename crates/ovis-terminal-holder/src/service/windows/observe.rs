//! Judging one observation of a Windows holder from its record, process and pipe.
//!
//! A record is only a claim: the recorded process must still exist with its start
//! token, and its pipe must have a listening instance. A record whose process is gone is
//! stale even when nothing else is left, so a leftover marker is never reported as a
//! live endpoint. Judging changes nothing.

use ovis_terminal_protocol::domain::unix_service::{Observed, StaleReason};
use ovis_terminal_protocol::domain::windows_service::{HolderPlacement, HostRecord, PipeName};
use ovis_terminal_protocol::schema::ids::ProcessPid;

use super::HostError;

/// What the operating system says about a recorded process.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProcessCheck {
    /// The process exists with this start token.
    Alive {
        /// Its start token.
        start_token: String,
    },
    /// No process has the PID.
    Gone,
}

/// Whether a pipe name has a listening or busy instance; nothing is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PipePresence {
    /// At least one instance of the name exists.
    Listening,
    /// No instance of the name exists.
    Absent,
}

/// The read-only operating-system facts one observation needs.
pub trait Facts {
    /// The process the record names.
    ///
    /// # Errors
    ///
    /// A process table that cannot be read.
    fn process(&self, pid: ProcessPid) -> Result<ProcessCheck, HostError>;

    /// Whether `pipe` has an instance.
    ///
    /// # Errors
    ///
    /// A presence check that failed for another reason than absence.
    fn pipe(&self, pipe: &PipeName) -> Result<PipePresence, HostError>;
}

/// Judges the record (if any), falling back to the session's own pipe name.
///
/// # Errors
///
/// The failures of `facts` and a record whose address is not a pipe.
pub fn judge(
    record: Option<HostRecord>,
    expected: &PipeName,
    facts: &impl Facts,
) -> Result<(Observed, Option<HolderPlacement>), HostError> {
    let Some(record) = record else {
        return Ok(match facts.pipe(expected)? {
            PipePresence::Absent => (Observed::Stopped, None),
            PipePresence::Listening => (Observed::Unreachable { record: None }, None),
        });
    };
    let placement = Some(record.placement);
    let stale = match facts.process(record.endpoint.boot_proof.pid)? {
        ProcessCheck::Gone => Some(StaleReason::ProcessGone),
        ProcessCheck::Alive { start_token }
            if start_token != record.endpoint.boot_proof.start_token.as_str() =>
        {
            Some(StaleReason::ProcessReplaced)
        }
        ProcessCheck::Alive { .. } => None,
    };
    if let Some(reason) = stale {
        let record = Some(record.endpoint);
        return Ok((Observed::StaleEndpoint { record, reason }, placement));
    }
    let pipe = record.pipe().map_err(HostError::Name)?;
    let observed = match facts.pipe(&pipe)? {
        PipePresence::Listening => Observed::Running {
            record: record.endpoint,
        },
        PipePresence::Absent => Observed::Unreachable {
            record: Some(record.endpoint),
        },
    };
    Ok((observed, placement))
}

#[cfg(test)]
mod tests {
    use ovis_terminal_protocol::domain::unix_service::{BootProof, Observed, StaleReason};
    use ovis_terminal_protocol::domain::windows_service::{HolderPlacement, HostRecord, PipeName};
    use ovis_terminal_protocol::schema::ids::{ProcessPid, ServiceLabel, StartToken};

    use super::{Facts, PipePresence, ProcessCheck, judge};
    use crate::service::windows::HostError;

    struct Host {
        process: ProcessCheck,
        pipe: PipePresence,
    }

    impl Facts for Host {
        fn process(&self, _: ProcessPid) -> Result<ProcessCheck, HostError> {
            Ok(self.process.clone())
        }

        fn pipe(&self, _: &PipeName) -> Result<PipePresence, HostError> {
            Ok(self.pipe)
        }
    }

    fn pipe() -> PipeName {
        PipeName::parse(r"\\.\pipe\ovis-terminal-holder-S-1-5-21-1-s").unwrap()
    }

    fn record() -> HostRecord {
        let proof = BootProof::new(ProcessPid::new(7), StartToken::new("t1".to_owned()));
        HostRecord::new(ServiceLabel::new("l".to_owned()), &pipe(), proof)
            .with_placement(HolderPlacement::Dedicated)
    }

    fn alive(token: &str) -> ProcessCheck {
        ProcessCheck::Alive {
            start_token: token.to_owned(),
        }
    }

    fn state(record: Option<HostRecord>, host: &Host) -> (&'static str, Option<StaleReason>) {
        let (observed, _) = judge(record, &pipe(), host).unwrap();
        let reason = match &observed {
            Observed::StaleEndpoint { reason, .. } => Some(*reason),
            _ => None,
        };
        (
            match observed.state() {
                ovis_terminal_protocol::schema::methods::server::ServerState::Running => "running",
                ovis_terminal_protocol::schema::methods::server::ServerState::Stopped => "stopped",
                ovis_terminal_protocol::schema::methods::server::ServerState::Unreachable => {
                    "unreachable"
                }
                _ => "stale_endpoint",
            },
            reason,
        )
    }

    #[test]
    fn a_marker_without_its_process_is_stale_even_with_no_pipe_left() {
        for pipe in [PipePresence::Absent, PipePresence::Listening] {
            let gone = Host {
                process: ProcessCheck::Gone,
                pipe,
            };
            assert_eq!(
                state(Some(record()), &gone),
                ("stale_endpoint", Some(StaleReason::ProcessGone))
            );
            let replaced = Host {
                process: alive("t2"),
                pipe,
            };
            assert_eq!(
                state(Some(record()), &replaced),
                ("stale_endpoint", Some(StaleReason::ProcessReplaced))
            );
        }
    }

    #[test]
    fn a_live_holder_is_running_only_while_its_pipe_has_an_instance() {
        let serving = Host {
            process: alive("t1"),
            pipe: PipePresence::Listening,
        };
        assert_eq!(state(Some(record()), &serving), ("running", None));
        let (_, placement) = judge(Some(record()), &pipe(), &serving).unwrap();
        assert_eq!(placement, Some(HolderPlacement::Dedicated));
        let silent = Host {
            process: alive("t1"),
            pipe: PipePresence::Absent,
        };
        assert_eq!(state(Some(record()), &silent), ("unreachable", None));
    }

    #[test]
    fn without_a_record_only_the_pipe_name_decides_stopped_or_unreachable() {
        let empty = Host {
            process: ProcessCheck::Gone,
            pipe: PipePresence::Absent,
        };
        assert_eq!(state(None, &empty), ("stopped", None));
        let squatted = Host {
            process: ProcessCheck::Gone,
            pipe: PipePresence::Listening,
        };
        assert_eq!(state(None, &squatted), ("unreachable", None));
    }
}
