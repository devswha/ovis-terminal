//! The `serve` step: the runner's own fixture or the assembled binary, in the isolated root.

use super::super::session::Session;
use crate::error::RunnerError;
use crate::fixture::launch;
use crate::scenario::model::FixtureKind;

impl Session<'_> {
    pub(crate) async fn serve(&mut self, kind: FixtureKind) -> Result<(), RunnerError> {
        if self.fixture.is_some() {
            return Err(RunnerError::FixtureState);
        }
        if !cfg!(unix) {
            return Err(RunnerError::UnsupportedPlatform);
        }
        let root = self.root.path();
        // Each server starts in its isolated root, so a relative runner or binary path
        // (as a caller in another directory passes it) is made absolute first.
        let absolute = |path: &std::path::Path| {
            std::path::absolute(path).map_err(|source| RunnerError::Spawn {
                program: path.to_path_buf(),
                source,
            })
        };
        let (runner, binary) = (absolute(&self.ctx.runner)?, absolute(&self.ctx.binary)?);
        let started = match kind {
            FixtureKind::Protocol => {
                let args = [
                    "fixture".into(),
                    "--root".into(),
                    root.into(),
                    "--binary".into(),
                    binary.as_os_str().to_owned(),
                ];
                launch::start(&runner, &args, root).await?
            }
            FixtureKind::Assembled => {
                let args = [
                    "serve".into(),
                    "--stdin-lifetime".into(),
                    "--root".into(),
                    root.join("srv").into(),
                ];
                launch::start(&binary, &args, root).await?
            }
        };
        self.fixture = Some(started);
        Ok(())
    }
}
