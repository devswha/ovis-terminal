//! The `serve` step: the runner's own fixture or the assembled binary, in the isolated root.

use super::super::session::Session;
use crate::error::RunnerError;
use crate::fixture::launch;
use crate::scenario::model::FixtureKind;

impl Session<'_> {
    pub(super) async fn serve(&mut self, kind: FixtureKind) -> Result<(), RunnerError> {
        if self.fixture.is_some() {
            return Err(RunnerError::FixtureState);
        }
        if !cfg!(unix) {
            return Err(RunnerError::UnsupportedPlatform);
        }
        let root = self.root.path();
        let started = match kind {
            FixtureKind::Protocol => {
                let args = [
                    "fixture".into(),
                    "--root".into(),
                    root.into(),
                    "--binary".into(),
                    self.ctx.binary.as_os_str().to_owned(),
                ];
                launch::start(&self.ctx.runner, &args, root).await?
            }
            FixtureKind::Assembled => {
                let args = [
                    "serve".into(),
                    "--stdin-lifetime".into(),
                    "--root".into(),
                    root.join("srv").into(),
                ];
                launch::start(&self.ctx.binary, &args, root).await?
            }
        };
        self.fixture = Some(started);
        Ok(())
    }
}
