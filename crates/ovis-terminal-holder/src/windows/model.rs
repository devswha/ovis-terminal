//! Pure ownership rules of a pseudoconsole terminal, free of OS calls.
//!
//! These types decide what a terminal may publish and release, and in which order; the
//! Windows backend only feeds them events. They build on every host so the rules are
//! unit- and property-tested everywhere, and the `ffi-model` feature exposes them for
//! Miri. Miri never executes ConPTY itself.

mod input;
mod output;
mod teardown;

pub use input::{InputLimits, InputQueue, InputRejected, QueueUsage};
pub use output::{OutputError, OutputLog, OutputRead};
pub use teardown::{Teardown, TeardownEvent, TeardownSteps};

use crate::protocol::{
    Exit, ExitUnknown, ExitUnknownReason, PtyIoFailure, PtyOperation, TerminalTarget,
};

/// Absolute offset in a terminal's raw output stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OutputOffset(u64);

impl OutputOffset {
    /// The offset of the first output byte.
    pub const ZERO: Self = Self(0);

    /// Wraps a raw offset.
    #[must_use]
    pub const fn new(offset: u64) -> Self {
        Self(offset)
    }

    /// The raw offset.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Position of an admitted input request in its terminal's write order, from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InputSequence(u64);

impl InputSequence {
    /// The raw sequence number.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// How a pseudoconsole program ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProgramExit {
    /// The process exit code as the OS reports it.
    Code(u32),
    /// Waiting for the process failed; no status was observed.
    WaitFailed {
        /// Operating-system error number, when one was reported.
        os_error: Option<i32>,
    },
}

/// How the output stream of a pseudoconsole ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutputEnd {
    /// The pseudoconsole closed its output pipe after its last byte.
    Eof,
    /// Reading failed; bytes after the final offset may be missing.
    Failed {
        /// Operating-system error number, when one was reported.
        os_error: Option<i32>,
    },
}

/// Exit evidence, published only once every byte before `final_offset` is readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Ended {
    /// How the program ended.
    pub exit: ProgramExit,
    /// How its output ended.
    pub output: OutputEnd,
    /// Offset just past the last output byte.
    pub final_offset: OutputOffset,
}

impl Ended {
    /// The holder `Exit` message for `target`.
    ///
    /// A Windows exit code is a `u32` (for example `0xC000013A`); it is carried
    /// bit-for-bit in the signed wire field, so no code is clamped or lost.
    #[must_use]
    pub const fn exit_message(&self, target: TerminalTarget) -> Exit {
        let (code, exit_unknown) = match self.exit {
            ProgramExit::Code(code) => (Some(i32::from_ne_bytes(code.to_ne_bytes())), None),
            ProgramExit::WaitFailed { os_error } => (
                None,
                Some(ExitUnknown {
                    reason: ExitUnknownReason::WaitFailed,
                    os_error,
                }),
            ),
        };
        let io_failure = match self.output {
            OutputEnd::Eof => None,
            OutputEnd::Failed { os_error } => Some(PtyIoFailure {
                operation: PtyOperation::Read,
                os_error,
                poll_revents: None,
            }),
        };
        Exit {
            target,
            code,
            signal: None,
            exit_unknown,
            io_failure,
            final_offset: self.final_offset.get(),
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{Ended, OutputEnd, OutputOffset, ProgramExit};
    use crate::protocol::{
        Message, TerminalId, TerminalIncarnation, TerminalTarget, decode_message, encode_message,
    };

    fn target() -> TerminalTarget {
        TerminalTarget {
            terminal_id: TerminalId::new("conpty-1".to_owned()),
            terminal_incarnation: TerminalIncarnation::new(1),
            process_incarnation: None,
        }
    }

    proptest! {
        #[test]
        fn every_windows_exit_code_and_offset_survives_the_holder_frame(code in any::<u32>(), offset in any::<u64>()) {
            let ended = Ended { exit: ProgramExit::Code(code), output: OutputEnd::Eof, final_offset: OutputOffset::new(offset) };
            let message = Message::Exit(ended.exit_message(target()));
            let frame = encode_message(&message).expect("encode");
            let Message::Exit(exit) = decode_message(&frame).expect("decode") else {
                panic!("not an exit");
            };
            let wire = exit.code.expect("an exit code");
            prop_assert_eq!(u32::from_ne_bytes(wire.to_ne_bytes()), code);
            prop_assert_eq!(exit.final_offset, offset);
            prop_assert!(exit.exit_unknown.is_none() && exit.io_failure.is_none());
        }
    }

    #[test]
    fn an_unobserved_status_and_a_failed_read_are_reported_not_defaulted() {
        let ended = Ended {
            exit: ProgramExit::WaitFailed { os_error: Some(6) },
            output: OutputEnd::Failed {
                os_error: Some(109),
            },
            final_offset: OutputOffset::new(u64::MAX),
        };
        let json = serde_json::to_value(Message::Exit(ended.exit_message(target()))).expect("json");
        let field = |pointer: &str| json.pointer(pointer).cloned();
        assert_eq!(field("/data/code"), Some(serde_json::Value::Null));
        assert_eq!(
            field("/data/exit_unknown/reason"),
            Some("wait_failed".into())
        );
        assert_eq!(field("/data/exit_unknown/os_error"), Some(6.into()));
        assert_eq!(field("/data/io_failure/operation"), Some("read".into()));
        assert_eq!(field("/data/io_failure/os_error"), Some(109.into()));
        assert_eq!(
            field("/data/final_offset"),
            Some("18446744073709551615".into())
        );
    }
}
