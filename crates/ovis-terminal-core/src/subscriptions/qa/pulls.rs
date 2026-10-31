//! Bounded pulls shared by the subscription scenarios.

use std::error::Error;
use std::time::Instant;

use ovis_terminal_core::events::Event;
use ovis_terminal_core::subscriptions::{LossRecord, Pulled, Recovery};

use super::fixture::{BOUND, Data, State, Sub};

/// Batches pulled before a loss, and the recovery that reported it.
pub type Missed = (Vec<Event<Data>>, Recovery<State>);
/// Delivered events and the scan cursor that covered the requested sequence.
pub type Scanned = (Vec<Event<Data>>, u64);

/// The bounded deadline of one wait.
///
/// # Errors
///
/// Returns an error if the clock cannot represent it.
pub fn deadline() -> Result<Instant, Box<dyn Error>> {
    Ok(Instant::now()
        .checked_add(BOUND)
        .ok_or("deadline overflows")?)
}

/// Pulls once with the shared bound.
///
/// # Errors
///
/// Returns the typed pull failure.
pub fn pull(feed: &Sub) -> Result<Pulled<Data, State>, Box<dyn Error>> {
    Ok(feed.next(deadline()?)?)
}

/// Pulls until the subscription reports its loss, keeping any batch delivered before it.
///
/// # Errors
///
/// Returns a pull failure or any other outcome.
pub fn until_missed(feed: &Sub) -> Result<Missed, Box<dyn Error>> {
    let deadline = deadline()?;
    let mut events = Vec::new();
    loop {
        match feed.next(deadline)? {
            Pulled::Events(batch) => events.extend(batch.events),
            Pulled::Missed(recovery) => return Ok((events, recovery)),
            other => return Err(format!("expected a loss, got {other:?}").into()),
        }
    }
}

/// Pulls batches until a committed loss marker arrives; returns its sequence and record.
///
/// # Errors
///
/// Returns a pull failure or the bound passing first.
pub fn until_loss_event(feed: &Sub) -> Result<(u64, LossRecord), Box<dyn Error>> {
    let deadline = deadline()?;
    loop {
        match feed.next(deadline)? {
            Pulled::Events(batch) => {
                let marker = batch.events.iter().find_map(|event| match &event.data {
                    Data::Lost(record) => Some((event.cursor.seq.get(), record.clone())),
                    Data::Set { .. } | Data::Gate(_) => None,
                });
                if let Some(marker) = marker {
                    return Ok(marker);
                }
            }
            Pulled::Heartbeat(_) if Instant::now() < deadline => {}
            other => return Err(format!("expected a loss marker, got {other:?}").into()),
        }
    }
}

/// Collects batches until the subscription has scanned `seq`; heartbeats only wait.
///
/// # Errors
///
/// Returns a pull failure, a loss, or the bound passing first.
pub fn scan_to(feed: &Sub, seq: u64) -> Result<Scanned, Box<dyn Error>> {
    let deadline = deadline()?;
    let mut events = Vec::new();
    loop {
        match feed.next(deadline)? {
            Pulled::Events(batch) => {
                events.extend(batch.events);
                if batch.scan_seq.get() >= seq {
                    return Ok((events, batch.scan_seq.get()));
                }
            }
            Pulled::Heartbeat(beat) if beat.scan_seq.get() >= seq => {
                return Ok((events, beat.scan_seq.get()));
            }
            Pulled::Heartbeat(_) if Instant::now() < deadline => {}
            other => return Err(format!("scan to {seq} ended with {other:?}").into()),
        }
    }
}
