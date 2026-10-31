//! The order in which a pseudoconsole terminal releases its resources.
//!
//! ConPTY keeps its output pipe open until the pseudoconsole is closed, even after the
//! program exits, and closing it while output is unread can block until a reader
//! drains it. So the pseudoconsole is closed exactly once, only after the program has
//! exited, while the output reader keeps running; the exit is published only after
//! both the program has exited and the output has ended.

use super::{OutputEnd, ProgramExit};

/// A fact one of the terminal's threads observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TeardownEvent {
    /// The waiter observed the program end.
    ProgramExited(ProgramExit),
    /// The reader observed the output end.
    OutputEnded(OutputEnd),
}

/// What the observing thread must do now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TeardownSteps {
    /// Close the pseudoconsole so its output pipe can reach end of file.
    pub close_console: bool,
    /// Publish the exit: both halves are known and this is the first time.
    pub finished: Option<(ProgramExit, OutputEnd)>,
}

/// Teardown progress of one terminal; later duplicates of an event are ignored.
#[derive(Debug, Default)]
pub struct Teardown {
    exit: Option<ProgramExit>,
    output: Option<OutputEnd>,
    console_closed: bool,
    finished: bool,
}

impl Teardown {
    /// Records `event` and returns the steps it unlocks.
    pub fn observe(&mut self, event: TeardownEvent) -> TeardownSteps {
        match event {
            TeardownEvent::ProgramExited(exit) => {
                self.exit.get_or_insert(exit);
            }
            TeardownEvent::OutputEnded(output) => {
                self.output.get_or_insert(output);
            }
        }
        let close_console = self.exit.is_some() && !self.console_closed;
        self.console_closed |= close_console;
        let finished = match (self.exit, self.output) {
            (Some(exit), Some(output)) if !self.finished => Some((exit, output)),
            _ => None,
        };
        self.finished |= finished.is_some();
        TeardownSteps {
            close_console,
            finished,
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{Teardown, TeardownEvent};
    use crate::windows::model::{OutputEnd, ProgramExit};

    fn event() -> impl Strategy<Value = TeardownEvent> {
        prop_oneof![
            any::<u32>().prop_map(|code| TeardownEvent::ProgramExited(ProgramExit::Code(code))),
            Just(TeardownEvent::ProgramExited(ProgramExit::WaitFailed {
                os_error: Some(6)
            })),
            Just(TeardownEvent::OutputEnded(OutputEnd::Eof)),
            Just(TeardownEvent::OutputEnded(OutputEnd::Failed {
                os_error: Some(109)
            })),
        ]
    }

    proptest! {
        #[test]
        fn the_console_closes_once_after_exit_and_the_exit_is_published_once_after_both(
            events in proptest::collection::vec(event(), 0..12),
        ) {
            let mut teardown = Teardown::default();
            let (mut first_exit, mut first_output) = (None, None);
            let (mut closes, mut finishes) = (0_u32, 0_u32);
            for (index, event) in events.iter().enumerate() {
                match *event {
                    TeardownEvent::ProgramExited(exit) => { first_exit.get_or_insert(exit); }
                    TeardownEvent::OutputEnded(output) => { first_output.get_or_insert(output); }
                }
                let steps = teardown.observe(*event);
                if steps.close_console {
                    closes = closes.saturating_add(1);
                    prop_assert!(first_exit.is_some(), "closed at {index} before the program exited");
                }
                if let Some(finished) = steps.finished {
                    finishes = finishes.saturating_add(1);
                    prop_assert_eq!(Some(finished), first_exit.zip(first_output), "the first halves are published");
                    prop_assert_eq!(closes, 1, "the console closed no later than the exit");
                }
            }
            prop_assert_eq!(closes, u32::from(first_exit.is_some()));
            prop_assert_eq!(finishes, u32::from(first_exit.is_some() && first_output.is_some()));
        }
    }
}
