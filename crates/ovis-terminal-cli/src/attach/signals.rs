//! Signals the client handles from before it connects until it exits.
//!
//! Every handler is installed before the outer terminal is touched, so no interruption
//! can take the default action (terminate, or dump core on SIGQUIT) while raw mode is on.
//! Before a stream is attached an interruption ends the attach as `interrupted` at its
//! stage; once attached it detaches, and interruptions delivered while detaching join that
//! detach (its wait for CLOSE is bounded). Window changes only ever resize.

use std::future::Future;

use tokio::signal::unix::{Signal, SignalKind, signal};

use super::AttachError;

/// One delivered signal, in the terms the client acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Interrupt {
    /// SIGWINCH: the outer terminal's grid changed.
    Resize,
    /// SIGINT.
    Interrupted,
    /// SIGTERM.
    Terminated,
    /// SIGHUP: the outer terminal went away.
    Hangup,
    /// SIGQUIT: Ctrl-\ on an outer terminal that is not raw, as when observing.
    Quit,
}

impl Interrupt {
    /// The detach reason this signal records.
    pub(super) const fn reason(self) -> &'static str {
        match self {
            Self::Resize => "resize",
            Self::Interrupted => "interrupted",
            Self::Terminated => "terminated",
            Self::Hangup => "hangup",
            Self::Quit => "quit_signal",
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Resize => "SIGWINCH",
            Self::Interrupted => "SIGINT",
            Self::Terminated => "SIGTERM",
            Self::Hangup => "SIGHUP",
            Self::Quit => "SIGQUIT",
        }
    }
}

/// The installed handlers.
#[derive(Debug)]
pub(super) struct Signals {
    winch: Signal,
    interrupt: Signal,
    terminate: Signal,
    hangup: Signal,
    quit: Signal,
}

fn listen(kind: SignalKind) -> Result<Signal, AttachError> {
    signal(kind).map_err(|source| AttachError::Io {
        operation: "watch signals",
        source,
    })
}

impl Signals {
    /// Installs every handler; call it before connecting.
    pub(super) fn listen() -> Result<Self, AttachError> {
        Ok(Self {
            winch: listen(SignalKind::window_change())?,
            interrupt: listen(SignalKind::interrupt())?,
            terminate: listen(SignalKind::terminate())?,
            hangup: listen(SignalKind::hangup())?,
            quit: listen(SignalKind::quit())?,
        })
    }

    /// The next delivered signal, a resize included.
    pub(super) async fn next(&mut self) -> Interrupt {
        tokio::select! {
            _ = self.winch.recv() => Interrupt::Resize,
            _ = self.interrupt.recv() => Interrupt::Interrupted,
            _ = self.terminate.recv() => Interrupt::Terminated,
            _ = self.hangup.recv() => Interrupt::Hangup,
            _ = self.quit.recv() => Interrupt::Quit,
        }
    }

    /// The next signal that is not a resize.
    pub(super) async fn stop(&mut self) -> Interrupt {
        loop {
            let delivered = self.next().await;
            if delivered != Interrupt::Resize {
                return delivered;
            }
        }
    }

    /// Runs `work` unless a stopping signal arrives first, which ends the attach as
    /// `interrupted` at `stage`; dropping `work` closes whatever it had opened.
    pub(super) async fn during<T>(
        &mut self,
        stage: &'static str,
        work: impl Future<Output = Result<T, AttachError>>,
    ) -> Result<T, AttachError> {
        tokio::select! {
            done = work => done,
            delivered = self.stop() => Err(AttachError::Interrupted {
                stage,
                signal: delivered.name(),
            }),
        }
    }
}
