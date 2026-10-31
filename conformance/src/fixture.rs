//! The reusable fixture process and the runner's handle on it.

#[cfg(unix)]
pub mod documents;
#[cfg(unix)]
pub mod handler;
pub mod launch;
pub mod output;
#[cfg(unix)]
pub mod serve;
#[cfg(unix)]
pub mod terminals;
pub mod wire;

#[cfg(unix)]
pub(crate) use serve::serve;

/// Without a Unix listener there is nothing for the fixture to serve.
#[cfg(not(unix))]
pub(crate) async fn serve(_: &std::path::Path) -> Result<(), crate::error::RunnerError> {
    Err(crate::error::RunnerError::UnsupportedPlatform)
}
