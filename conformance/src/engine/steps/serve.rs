//! The `serve` step: the runner's own fixture or the assembled binary, in the isolated root.

use super::super::session::Session;
use crate::error::RunnerError;
use crate::fixture::launch;
use crate::scenario::model::{FixtureKind, Serve};

/// `serve` arguments for the assembled binary in `srv`; a QA output ring marks `srv` a QA
/// fixture root first, since the binary refuses QA seams anywhere else.
fn assembled_args(
    srv: &std::path::Path,
    ring: Option<crate::ids::Decimal>,
) -> Result<Vec<std::ffi::OsString>, RunnerError> {
    let mut args = vec!["serve".into(), "--stdin-lifetime".into(), "--root".into()];
    args.push(srv.into());
    if let Some(ring) = ring {
        std::fs::create_dir_all(srv)
            .and_then(|()| std::fs::write(srv.join("qa-fixture"), b""))
            .map_err(|source| RunnerError::Io {
                operation: "mark the server root a QA fixture root",
                source,
            })?;
        args.push("--qa-output-ring-bytes".into());
        args.push(ring.0.to_string().into());
    }
    Ok(args)
}

impl Session<'_> {
    pub(super) async fn serve(&mut self, serve: &Serve) -> Result<(), RunnerError> {
        let kind = serve.fixture;
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
                let args = assembled_args(&root.join("srv"), serve.qa_output_ring_bytes)?;
                launch::start(&binary, &args, root).await?
            }
        };
        self.fixture = Some(started);
        Ok(())
    }
}
