//! Bounded pulls shared by the subscription scenarios.

use std::error::Error;
use std::time::Instant;

use ovis_terminal_core::events::Event;
use ovis_terminal_core::subscriptions::{LossRecord, Pulled, Recovery, Selector, Start};

use super::fixture::{BOUND, Data, State, Sub, TestResult, World};

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

fn loss_marker(events: &[Event<Data>]) -> Option<(u64, LossRecord)> {
    events.iter().find_map(|event| match &event.data {
        Data::Lost(record) => Some((event.cursor.seq.get(), record.clone())),
        Data::Set { .. } | Data::Gate(_) => None,
    })
}

/// The first committed loss marker among `pulled`, the events this feed already
/// delivered, or else in the batches pulled next; returns its sequence and record.
///
/// A marker may share a batch with the events before it, so a caller that already
/// pulled past them passes them here instead of assuming the marker comes later.
///
/// # Errors
///
/// Returns a pull failure or the bound passing first.
pub fn until_loss_event(
    feed: &Sub,
    pulled: &[Event<Data>],
) -> Result<(u64, LossRecord), Box<dyn Error>> {
    if let Some(marker) = loss_marker(pulled) {
        return Ok(marker);
    }
    let deadline = deadline()?;
    loop {
        match feed.next(deadline)? {
            Pulled::Events(batch) => {
                if let Some(marker) = loss_marker(&batch.events) {
                    return Ok(marker);
                }
            }
            Pulled::Heartbeat(_) if Instant::now() < deadline => {}
            other => return Err(format!("expected a loss marker, got {other:?}").into()),
        }
    }
}

/// Waits until the fan-out worker has fanned out every committed event, observed by a
/// probe subscription that starts at the worker's head and scans up to the durable head.
///
/// # Errors
///
/// Returns a subscription or pull failure, or the bound passing first.
pub fn until_fanned_out(world: &World) -> TestResult {
    let (durable, _) = world.fresh()?;
    let probe = world.subscribe(Start::Head, Selector::all())?;
    if probe.snapshot_seq.get() < durable {
        scan_to(&probe.feed, durable)?;
    }
    Ok(())
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
