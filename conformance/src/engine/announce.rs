//! The `announced` step: the fixture's next stdout line, such as the successor boot a
//! planned restart announces, checked against the step's expectations.

use super::session::Session;
use crate::error::RunnerError;
use crate::scenario::matcher::Subject;
use crate::scenario::model::Announced;

impl Session<'_> {
    /// Awaits, bounded, the fixture's next announcement and checks it.
    pub(crate) async fn announced(&mut self, announced: &Announced) -> Result<(), RunnerError> {
        let line = self
            .fixture
            .as_mut()
            .ok_or(RunnerError::FixtureState)?
            .announcement()
            .await?;
        let expect = announced.expect.resolved(&mut self.labels);
        let checked = expect.check(
            Subject {
                scope: "announcement",
                value: &line,
            },
            self.ctx.repo.root(),
        );
        for (name, value) in expect.captures(&line) {
            self.labels.capture(name, value);
        }
        self.assert(checked)
    }
}
