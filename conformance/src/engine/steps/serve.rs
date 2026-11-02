//! The `serve` step: the runner's own fixture or the assembled binary, in the isolated root.

use super::super::session::Session;
use crate::error::RunnerError;
use crate::fixture::launch;
use crate::scenario::model::{FixtureKind, Serve};

impl Session<'_> {
    pub(crate) async fn serve(&mut self, serve: &Serve) -> Result<(), RunnerError> {
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
        let started = match serve.fixture {
            FixtureKind::Protocol if serve.qa_graphics_sequence_max_bytes.is_some() => {
                return Err(RunnerError::ScenarioInconsistent {
                    scenario: "serve".to_owned(),
                    detail: "only the assembled binary takes a QA graphics cap",
                });
            }
            FixtureKind::Protocol if serve.qa_output_ring_bytes.is_some() => {
                return Err(RunnerError::ScenarioInconsistent {
                    scenario: "serve".to_owned(),
                    detail: "only the assembled binary takes a QA output ring",
                });
            }
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
                let srv = root.join("srv");
                let mut args: Vec<std::ffi::OsString> = vec![
                    "serve".into(),
                    "--stdin-lifetime".into(),
                    "--root".into(),
                    srv.clone().into(),
                ];
                if let Some(cap) = serve.qa_graphics_sequence_max_bytes {
                    mark_qa_root(&srv)?;
                    args.push("--qa-graphics-sequence-max-bytes".into());
                    args.push(cap.0.to_string().into());
                }
                if let Some(ring) = serve.qa_output_ring_bytes {
                    mark_qa_root(&srv)?;
                    args.push("--qa-output-ring-bytes".into());
                    args.push(ring.0.to_string().into());
                }
                launch::start(&binary, &args, root).await?
            }
        };
        self.fixture = Some(started);
        Ok(())
    }
}

/// Marks `srv` a QA fixture root, the only root on which the binary accepts QA seams.
fn mark_qa_root(srv: &std::path::Path) -> Result<(), RunnerError> {
    let io = |source| RunnerError::Io {
        operation: "mark the server root a QA fixture root",
        source,
    };
    std::fs::create_dir_all(srv).map_err(io)?;
    std::fs::write(srv.join("qa-fixture"), b"").map_err(io)
}
