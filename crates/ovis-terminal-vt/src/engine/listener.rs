//! Adapters `alacritty_terminal` needs: an event sink, a synchronized-update
//! timer and a handler that ignores every action.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::vte::ansi::{Handler, Timeout};

/// Collects the events the emulator raises while it parses one segment.
#[derive(Debug, Clone, Default)]
pub(super) struct Listener {
    events: Arc<Mutex<Vec<Event>>>,
}

impl Listener {
    pub(super) fn drain(&self) -> Vec<Event> {
        // A panic while pushing cannot leave the Vec half-written, so a
        // poisoned lock still guards a valid queue.
        std::mem::take(&mut *self.events.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}

/// Synchronized-update (mode 2026) timer that never arms.
///
/// The engine applies every byte as it arrives: a headless runtime has no
/// frame to hold back, and a program that never sends the end marker must not
/// freeze the authoritative screen. Attach streams still carry the raw mode.
#[derive(Debug, Default)]
pub(super) struct ImmediateUpdates;

impl Timeout for ImmediateUpdates {
    fn set_timeout(&mut self, _: Duration) {}

    fn clear_timeout(&mut self) {}

    fn pending_timeout(&self) -> bool {
        false
    }
}

/// Handler that discards every action; used to prime parser state on restore.
#[derive(Debug)]
pub(super) struct Inert;

impl Handler for Inert {}
