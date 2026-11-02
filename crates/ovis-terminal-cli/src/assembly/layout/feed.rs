//! Publishes the layout log's committed `layout.changed` and `selection.changed` records to
//! the serving boot's dispatch log, which `events.subscribe` reads.
//!
//! The layout log stays the only record of a layout change and this is its one reader. Each
//! forwarded operation is one dispatch commit that also moves a watermark (the last layout
//! sequence forwarded) by compare-and-swap in the same transaction, so a record is never
//! published twice, and records committed while no boot served (or before a crash) are
//! forwarded when the next boot starts.

mod published;
#[cfg(test)]
mod tests;

use std::io::Write as _;

use ovis_terminal_core::events::{Commit, EventError, ResourceId};
use ovis_terminal_core::layout::LayoutEvent;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_store::{
    CasWrite, EventPage, OpId as StoreOpId, Payload, ResourceKey, ResourceLookup, Revision,
    Snapshot, StoredEvent, Version,
};
pub(in crate::assembly) use published::Published;
use serde::Deserialize;
use serde_json::Value;

use super::LayoutLog;
use crate::assembly::actor::Runtime;
use crate::assembly::lanes::{DispatchLog, ServedEvent};

/// Records read per page of the layout log.
const PAGE: usize = 256;

/// Why forwarding layout records stopped; the next forward starts again at the watermark.
#[derive(Debug, thiserror::Error)]
pub(in crate::assembly) enum FeedError {
    /// The layout log or the dispatch log could not be read or written.
    #[error("the layout feed could not read or commit")]
    Log(#[from] EventError),
    /// A committed layout record or the watermark does not decode.
    #[error("a layout record or the forwarded watermark does not decode")]
    Record(#[source] serde_json::Error),
    /// The watermark is not a decimal layout sequence.
    #[error("the forwarded watermark is not a decimal sequence")]
    Watermark(#[source] std::num::ParseIntError),
    /// Another forward already published past this operation's records.
    #[error("another forward already published up to layout sequence {forwarded}")]
    Overtaken {
        /// Watermark found under the commit lock.
        forwarded: u64,
    },
}

impl FeedError {
    /// Stable kind token.
    pub(in crate::assembly) const fn kind(&self) -> &'static str {
        match self {
            Self::Log(_) => "layout_feed_log",
            Self::Record(_) => "layout_feed_record",
            Self::Watermark(_) => "layout_feed_watermark",
            Self::Overtaken { .. } => "layout_feed_overtaken",
        }
    }

    /// What to do about it.
    pub(in crate::assembly) const fn hint(&self) -> &'static str {
        match self {
            Self::Log(_) => "the next layout change or boot forwards the records again",
            Self::Record(_) => "the layout log holds a record this build cannot read",
            Self::Watermark(_) => {
                "the dispatch store's layout_feed resource was not written by this build"
            }
            Self::Overtaken { .. } => "read the watermark again and continue after it",
        }
    }
}

/// One committed layout record: what it affected and its typed data.
#[derive(Deserialize)]
struct Stored {
    affected: Vec<ResourceId>,
    data: LayoutEvent,
}

/// The dispatch store's resource holding the last forwarded layout sequence.
fn watermark_key() -> ResourceKey {
    ResourceKey::new("layout_feed", "forwarded")
}

/// The watermark's CAS version and value at `view`; absent means nothing was forwarded.
fn watermark(view: &Snapshot) -> Result<(Version, u64), FeedError> {
    match view.resource(&watermark_key()).map_err(EventError::from)? {
        ResourceLookup::Live { version, value, .. } => {
            let seq: String =
                serde_json::from_slice(value.as_bytes()).map_err(FeedError::Record)?;
            Ok((version, seq.parse::<u64>().map_err(FeedError::Watermark)?))
        }
        ResourceLookup::Tombstone { version, .. } => Ok((version, 0)),
        _ => Ok((Version::ABSENT, 0)),
    }
}

/// One layout operation's records: its layout sequences and what subscribers receive.
struct Unit {
    last: u64,
    published: Vec<(Published, Vec<ResourceId>)>,
}

/// Groups `records` by the store operation that committed them; each unit's first record
/// is the operation transition naming the client's op id.
fn units(records: &[StoredEvent], tree: &Value) -> Result<Vec<Unit>, FeedError> {
    let mut units = Vec::new();
    for chunk in records.chunk_by(|first, second| first.op == second.op) {
        let mut op_id: Option<String> = None;
        let mut unit = Unit {
            last: 0,
            published: Vec::new(),
        };
        for record in chunk {
            let stored: Stored =
                serde_json::from_slice(record.data.as_bytes()).map_err(FeedError::Record)?;
            unit.last = record.seq.get();
            if let LayoutEvent::Operation(event) = &stored.data {
                op_id.get_or_insert_with(|| event.operation.op_id.as_str().to_owned());
            }
            let Some(op_id) = op_id.as_deref() else {
                continue;
            };
            if let Some(published) = Published::of(&stored.data, (op_id, tree))? {
                unit.published.push((published, stored.affected));
            }
        }
        units.push(unit);
    }
    Ok(units)
}

/// Commits `unit`'s records with the watermark moved to its last sequence, unless another
/// forward already moved the watermark past `after`.
fn commit(feed: &DispatchLog, unit: Unit, after: u64) -> Result<(), FeedError> {
    let mut events = unit.published.into_iter();
    let Some((first, affected)) = events.next() else {
        return Ok(());
    };
    let op = StoreOpId::new(format!("layout-feed:{}", unit.last));
    feed.commit_unit(|view| {
        let (version, forwarded) = watermark(view)?;
        if forwarded > after {
            return Err(FeedError::Overtaken { forwarded });
        }
        let mark = serde_json::to_vec(&unit.last.to_string()).map_err(FeedError::Record)?;
        let mut commit = Commit::new(op, ServedEvent::Layout(first), affected).write(
            CasWrite::new(watermark_key(), version, Some(Payload::from(mark))),
        );
        for (published, affected) in events {
            commit = commit.event(ServedEvent::Layout(published), affected);
        }
        Ok(commit)
    })?;
    Ok(())
}

/// Forwards every layout record after the watermark; `tree` is the layout at the log's head.
/// Returns how many operations were published.
pub(in crate::assembly) fn forward(
    layout: &LayoutLog,
    feed: &DispatchLog,
    tree: &Value,
) -> Result<usize, FeedError> {
    let mut after = feed.read_view(watermark)?.1;
    let mut published = 0;
    loop {
        let page = layout.read_view(|view| {
            view.events_after(Revision::new(after), PAGE)
                .map_err(EventError::from)
        })?;
        let records = match page {
            EventPage::Events(records) => records,
            EventPage::Expired { floor } => {
                after = floor.get().saturating_sub(1);
                continue;
            }
            _ => return Err(FeedError::Log(EventError::InvalidRecord)),
        };
        let full = records.len() == PAGE;
        let mut units = units(&records, tree)?;
        if full && units.len() > 1 {
            // The page may end inside the last operation; it is read whole next round.
            units.pop();
        }
        let Some(end) = units.last().map(|unit| unit.last) else {
            return Ok(published);
        };
        for unit in units {
            let last = unit.last;
            if last <= after {
                continue;
            }
            if !unit.published.is_empty() {
                match commit(feed, unit, after) {
                    Ok(()) => published = published.saturating_add(1),
                    Err(FeedError::Overtaken { forwarded }) => {
                        after = forwarded;
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            }
            after = last;
        }
        after = after.max(end);
    }
}

fn warn(error: &FeedError) {
    let line = serde_json::json!({"warning": {"stage": "publish layout events",
        "kind": error.kind(), "hint": error.hint(), "detail": error.to_string()}});
    match writeln!(std::io::stderr().lock(), "{line}") {
        Ok(()) | Err(_) => {}
    }
}

impl Runtime {
    /// The layout at the log's head, read under the control's own session.
    fn head_tree(&self) -> Result<Value, BoundaryError> {
        let layouts = self.layouts();
        let layout = self
            .control
            .with_terminals(|_, session| Ok(layouts.get(session)))
            .map_err(|error| crate::assembly::error::refusal(error.kind(), error.hint()))?
            .map_err(|error| crate::assembly::error::refusal(error.kind(), error.hint()))?;
        Ok(super::wire::tree(&layout))
    }

    /// Publishes every layout record not yet forwarded to the serving boot's subscribers.
    /// Without a serving boot nothing is published; the next boot forwards it on start.
    pub(in crate::assembly) fn publish_layout(&self) {
        let Some(feed) = self.journal.feed() else {
            return;
        };
        let tree = match self.head_tree() {
            Ok(tree) => tree,
            Err(error) => {
                let line = serde_json::json!({"warning": {"stage": "publish layout events",
                    "detail": error.data().hint.clone()}});
                match writeln!(std::io::stderr().lock(), "{line}") {
                    Ok(()) | Err(_) => {}
                }
                return;
            }
        };
        if let Err(error) = forward(&self.layout_log, &feed, &tree) {
            warn(&error);
        }
    }
}
