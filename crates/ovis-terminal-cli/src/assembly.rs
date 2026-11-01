//! `ovis-terminal serve`: the real server assembled from the landed crates.
//!
//! One process owns the endpoint listener, the authority, the SQLite stores, a shared
//! in-process holder and the terminal lifecycle, and serves exactly the methods routed
//! here; hello advertises that list and nothing else. Holder facts flow through one actor
//! thread in order: lifecycle commit, then screen and attach streams, then the input
//! sequencer and geometry transitions of the same terminal.

mod actor;
mod barrier;
mod boot;
mod boundary;
mod credentials;
mod directory;
mod discovery;
mod error;
mod generation;
mod handler;
mod health;
mod identity;
mod input;
mod inspect;
mod journal;
mod lanes;
mod layout;
mod lease_ops;
mod lifetime;
mod live;
mod mutations;
mod reads;
mod records;
mod recovery;
mod resize;
mod restart;
mod routes;
mod serve;
mod terminal;
mod websocket;

use std::io;
use std::path::PathBuf;

use ovis_terminal_server::listen::{EndpointEnvironment, EndpointError};
use thiserror::Error;

/// A boxed failure of one start or stop stage.
pub(crate) type StageSource = Box<dyn std::error::Error + Send + Sync>;

/// Why `serve` could not start, keep serving or stop cleanly.
#[derive(Debug, Error)]
pub(crate) enum AssemblyError {
    /// A start-up stage failed; nothing is served.
    #[error("starting the server failed at: {stage}")]
    Start {
        /// What was being prepared.
        stage: &'static str,
        /// The failure.
        #[source]
        source: StageSource,
    },
    /// The endpoint could not be resolved or bound.
    #[error("the endpoint cannot be served")]
    Endpoint(#[source] EndpointError),
    /// The ready or report line could not be written.
    #[error("writing to stdout failed")]
    Announce(#[source] io::Error),
    /// A stop stage failed after serving ended.
    #[error("stopping the server failed at: {stage}")]
    Stop {
        /// What was being released.
        stage: &'static str,
        /// The failure.
        #[source]
        source: StageSource,
    },
}

impl AssemblyError {
    /// Maps a failure at start-up `stage`, keeping it as the source.
    pub(crate) fn start<E: Into<StageSource>>(stage: &'static str) -> impl FnOnce(E) -> Self {
        move |source| Self::Start {
            stage,
            source: source.into(),
        }
    }

    /// Maps a failure at stop `stage`, keeping it as the source.
    pub(crate) fn stop<E: Into<StageSource>>(stage: &'static str) -> impl FnOnce(E) -> Self {
        move |source| Self::Stop {
            stage,
            source: source.into(),
        }
    }

    /// The stable machine-readable kind.
    pub(crate) const fn kind(&self) -> &'static str {
        match self {
            Self::Start { .. } => "serve_failed",
            Self::Endpoint(error) => error.kind(),
            Self::Announce(_) => "transport_failed",
            Self::Stop { .. } => "stop_failed",
        }
    }

    /// Recovery guidance for the kind.
    pub(crate) const fn hint(&self) -> &'static str {
        match self {
            Self::Start { .. } => "check the state directory and that no other server owns it",
            Self::Endpoint(error) => error.hint(),
            Self::Announce(_) => "keep stdout open while the server runs",
            Self::Stop { .. } => "inspect the state directory; programs ended with the server",
        }
    }
}

/// Where one server lifetime keeps its socket and state, and what ends it.
#[derive(Debug, Clone)]
pub(crate) struct ServePlan {
    /// `<root>/run/s.sock` and `<root>/state` instead of the resolved endpoint.
    pub(crate) root: Option<PathBuf>,
    /// Stop when standard input reaches end of file, as well as on SIGINT or SIGTERM.
    pub(crate) stdin_lifetime: bool,
    /// Also serve over WebSocket on this loopback port.
    pub(crate) websocket_port: Option<u16>,
    /// How long the runtime waits for a request before it looks for holder messages.
    pub(crate) holder_poll: std::time::Duration,
}

/// Serves until SIGINT, SIGTERM or (with `stdin_lifetime`) end of stdin; prints one
/// ready line before serving and one report line after stopping.
pub(crate) async fn serve(
    plan: &ServePlan,
    env: &EndpointEnvironment,
) -> Result<(), AssemblyError> {
    serve::run(plan, env).await
}
