//! PTY steps: each check is an assertion; the client's drawn screen is process evidence.
//! Every step's plan is recorded on every platform; only Unix has outer PTYs to run it on.

#[cfg(unix)]
pub mod outer;

use crate::engine::session::Session;
use crate::evidence::DescriptorRecord;
use crate::scenario::model::Step;

/// Records `step`'s plan; `false` when `step` is not a PTY step.
fn record(session: &mut Session<'_>, step: &Step) -> bool {
    let (source, descriptors) = match step {
        Step::PtyOpen(open) => ("pty_open", open.planned()),
        Step::PtyType(typed) => ("pty_type", typed.planned()),
        Step::PtyExpect(expect) => ("pty_expect", expect.planned()),
        Step::PtyAbsent(absent) => ("pty_absent", absent.planned()),
        Step::PtyResize(resize) => ("pty_resize", resize.planned()),
        Step::PtyExit(exit) => ("pty_exit", exit.planned()),
        _ => return false,
    };
    session.descriptors.push(DescriptorRecord {
        source: source.to_owned(),
        descriptors,
    });
    true
}

/// Off Unix a PTY step is recorded, then refused.
#[cfg(not(unix))]
impl Session<'_> {
    pub(crate) fn pty(
        &mut self,
        step: &Step,
    ) -> std::future::Ready<Result<(), crate::error::RunnerError>> {
        let refused = if record(self, step) {
            crate::error::RunnerError::UnsupportedPlatform
        } else {
            crate::error::RunnerError::FixtureState
        };
        std::future::ready(Err(refused))
    }
}

#[cfg(unix)]
mod steps {
    use std::time::Duration;

    use serde_json::{Value, json};

    use super::outer::OuterPty;
    use super::record;
    use crate::engine::process::document;
    use crate::engine::session::Session;
    use crate::error::RunnerError;
    use crate::scenario::matcher::{Assertion, Subject};
    use crate::scenario::model::Step;
    use crate::scenario::pty::{PtyAbsent, PtyExit, PtyExpect, PtyOpen, PtyResize, PtyType};

    /// Bound on one expected output appearing.
    const SHOW_BOUND: Duration = Duration::from_secs(15);

    impl Session<'_> {
        /// Records `step`'s plan, then runs it on its outer PTY.
        pub(crate) async fn pty(&mut self, step: &Step) -> Result<(), RunnerError> {
            record(self, step);
            match step {
                Step::PtyOpen(open) => self.pty_open(open),
                Step::PtyType(typed) => self.pty_type(typed),
                Step::PtyExpect(expect) => self.pty_expect(expect).await,
                Step::PtyAbsent(absent) => self.pty_absent(absent),
                Step::PtyResize(resize) => self.pty_resize(resize),
                Step::PtyExit(exit) => self.pty_exit(exit).await,
                _ => Err(RunnerError::FixtureState),
            }
        }

        fn outer(&mut self, name: &str) -> Result<&mut OuterPty, RunnerError> {
            self.ptys
                .get_mut(name)
                .ok_or_else(|| RunnerError::UnknownConnection {
                    conn: name.to_owned(),
                })
        }

        fn pty_open(&mut self, open: &PtyOpen) -> Result<(), RunnerError> {
            let bearer = self.bearer().map(str::to_owned);
            let outer = OuterPty::open(
                &self.endpoint(),
                (&open.args, bearer.as_deref()),
                (open.cols, open.rows),
            )?;
            self.ptys.insert(open.pty.clone(), outer);
            Ok(())
        }

        fn pty_type(&mut self, typed: &PtyType) -> Result<(), RunnerError> {
            self.outer(&typed.pty)?.type_bytes(typed.bytes.as_bytes())
        }

        fn pty_resize(&mut self, resize: &PtyResize) -> Result<(), RunnerError> {
            self.outer(&resize.pty)?.resize((resize.cols, resize.rows))
        }

        async fn pty_expect(&mut self, expect: &PtyExpect) -> Result<(), RunnerError> {
            let outer = self.outer(&expect.pty)?;
            let shown = outer.shows(&expect.output, SHOW_BOUND).await;
            let check = format!("pty[{}]/shows", expect.pty);
            self.assert(vec![Assertion::equal(
                check,
                json!(expect.output),
                json!(shown.then_some(&expect.output)),
            )])
        }

        fn pty_absent(&mut self, absent: &PtyAbsent) -> Result<(), RunnerError> {
            let shown = self.outer(&absent.pty)?.delivered_contains(&absent.output);
            let check = format!("pty[{}]/absent", absent.pty);
            self.assert(vec![Assertion::equal(check, json!(false), json!(shown))])
        }

        async fn pty_exit(&mut self, exit: &PtyExit) -> Result<(), RunnerError> {
            let outer =
                self.ptys
                    .remove(&exit.pty)
                    .ok_or_else(|| RunnerError::UnknownConnection {
                        conn: exit.pty.clone(),
                    })?;
            let (code, stderr, restored, report) = outer.exit().await?;
            self.processes.push(report);
            let last = stderr
                .rsplit(|byte| *byte == b'\n')
                .find(|line| !line.is_empty())
                .unwrap_or_default();
            let record: Value = document(last);
            let mut checked = vec![
                Assertion::equal(
                    format!("pty[{}]/exit", exit.pty),
                    json!(exit.exit),
                    json!(code),
                ),
                Assertion::equal(
                    format!("pty[{}]/terminal_settings_restored", exit.pty),
                    json!(true),
                    json!(restored),
                ),
            ];
            let repo = self.ctx.repo.root().to_path_buf();
            checked.extend(exit.stderr.check(
                Subject {
                    scope: "pty/stderr",
                    value: &record,
                },
                &repo,
            ));
            self.assert(checked)
        }
    }
}
