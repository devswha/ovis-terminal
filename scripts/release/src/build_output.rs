//! A binary unpacked from a `build.sh` artifact carries its own build receipt and packed SDK
//! beside it; the native fixture stages from those instead of fixture stand-ins.

use std::path::{Path, PathBuf};

use crate::docs::Receipt;
use crate::error::ReleaseError;
use crate::files::read_json;

/// The real inputs found next to a built binary.
#[derive(Debug, Clone)]
pub(crate) struct BuildOutput {
    /// The build's `receipt.json`.
    pub(crate) receipt: PathBuf,
    /// The build's packed `ovis-terminal-client-<version>.tgz`.
    pub(crate) sdk: PathBuf,
    /// The triple the receipt names.
    pub(crate) target: String,
}

/// The build output beside `binary`, or `None` when `binary` is not an unpacked artifact.
///
/// # Errors
///
/// A malformed receipt, or an artifact directory without its packed SDK.
pub(crate) fn beside(binary: &Path) -> Result<Option<BuildOutput>, ReleaseError> {
    let Some(dir) = binary.parent() else {
        return Ok(None);
    };
    let receipt = dir.join("receipt.json");
    if !receipt.is_file() {
        return Ok(None);
    }
    let target = read_json::<Receipt>(&receipt, "build receipt")?.target;
    let listed: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(ReleaseError::io("list", dir))?
        .map(|item| item.map(|item| item.path()))
        .collect::<Result<_, _>>()
        .map_err(ReleaseError::io("list", dir))?;
    let sdk = listed
        .into_iter()
        .find(|path| {
            let packed = path.extension().is_some_and(|extension| extension == "tgz");
            let named = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("ovis-terminal-client-"));
            packed && named
        })
        .ok_or_else(|| ReleaseError::Tool {
            program: "build.sh artifact".to_owned(),
            detail: format!("{} has a receipt but no packed SDK", dir.display()),
        })?;
    Ok(Some(BuildOutput {
        receipt,
        sdk,
        target,
    }))
}
