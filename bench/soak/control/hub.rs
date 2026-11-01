//! Viewers of the control process: every published event enters a bounded replay ring
//! and every viewer's bounded queue. A viewer whose queue would pass the byte bound is
//! disconnected with an explicit `viewer_cut` record; nobody else waits for it.

mod accept;
mod viewer;

use std::collections::{BTreeMap, VecDeque};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::SyncSender;

use accept::Accept;
pub(super) use accept::ViewerMsg;
use ovis_terminal_protocol::schema::scalar::{ByteCount, Count, EventSeq};
use viewer::Viewer;

use super::super::error::SoakError;
use super::super::wire::{Event, Health, Hello, Line, Request};
use super::Inbound;

/// Events kept for a viewer that resumes after an interruption.
const RING: usize = 512;

#[derive(Debug)]
pub(super) struct Hub {
    accept: Accept,
    limit: u64,
    ring: VecDeque<(u64, Arc<str>)>,
    next: u64,
    viewers: BTreeMap<u64, Viewer>,
}

fn encode(line: &Line) -> Result<Arc<str>, SoakError> {
    let mut text = serde_json::to_string(line)?;
    text.push('\n');
    Ok(Arc::from(text))
}

impl Hub {
    pub(super) fn bind(
        socket: &Path,
        (inbound, limit): (SyncSender<Inbound>, u64),
    ) -> Result<Self, SoakError> {
        Ok(Self {
            accept: Accept::bind(socket, inbound)?,
            limit,
            ring: VecDeque::with_capacity(RING),
            next: 0,
            viewers: BTreeMap::new(),
        })
    }

    pub(super) fn publish(&mut self, event: Event) -> Result<(), SoakError> {
        let mut pending = vec![event];
        while let Some(event) = pending.pop() {
            let seq = self.next;
            self.next = self.next.saturating_add(1);
            let line = encode(&Line::Event {
                seq: EventSeq::new(seq),
                event,
            })?;
            if self.ring.len() >= RING {
                self.ring.pop_front();
            }
            self.ring.push_back((seq, Arc::clone(&line)));
            let limit = self.limit;
            for viewer in self.viewers.values_mut() {
                if !viewer.cut() && !viewer.offer(&line, limit) {
                    pending.push(Self::cut_event(viewer));
                }
            }
        }
        Ok(())
    }

    fn cut_event(viewer: &Viewer) -> Event {
        Event::ViewerCut {
            viewer: viewer.hello.viewer.clone(),
            queued_bytes: ByteCount::new(viewer.queued()),
            dropped: Count::new(viewer.dropped),
        }
    }

    /// Offers `line` to viewer `id`; the cut record when that cut it.
    fn offer(&mut self, id: u64, line: &Arc<str>) -> Option<Event> {
        let limit = self.limit;
        let viewer = self.viewers.get_mut(&id)?;
        if viewer.cut() || viewer.offer(line, limit) {
            return None;
        }
        Some(Self::cut_event(viewer))
    }

    /// Sends `line` to viewer `id` alone; a cut is published to everyone else.
    fn reply(&mut self, id: u64, line: &Line) -> Result<(), SoakError> {
        let encoded = encode(line)?;
        self.offer(id, &encoded)
            .map_or(Ok(()), |event| self.publish(event))
    }

    fn join(&mut self, id: u64, (hello, stream): (Hello, UnixStream)) -> Result<(), SoakError> {
        let floor = self.ring.front().map_or(self.next, |(seq, _)| *seq);
        let from = hello.from_seq.map_or(floor, EventSeq::get);
        self.viewers.insert(id, Viewer::start(hello, stream)?);
        if from < floor {
            self.reply(
                id,
                &Line::Gap {
                    from: EventSeq::new(from),
                    to: EventSeq::new(floor),
                },
            )?;
        }
        let replay: Vec<Arc<str>> = self
            .ring
            .iter()
            .filter(|(seq, _)| *seq >= from)
            .map(|(_, line)| Arc::clone(line))
            .collect();
        for line in replay {
            if let Some(event) = self.offer(id, &line) {
                return self.publish(event);
            }
        }
        Ok(())
    }

    /// Applies one viewer message; `lag` feeds status replies.
    pub(super) fn handle(&mut self, message: ViewerMsg, lag: &[u64]) -> Result<(), SoakError> {
        match message {
            ViewerMsg::Joined { id, hello, stream } => self.join(id, (hello, stream)),
            ViewerMsg::Asked { id, request } => {
                let line = match request {
                    Request::Ping { id: ping } => Line::Pong { id: ping },
                    Request::Status { id: asked } => Line::Status {
                        id: asked,
                        health: self.health(lag),
                    },
                };
                self.reply(id, &line)
            }
            ViewerMsg::Left { id } => {
                if let Some(viewer) = self.viewers.remove(&id) {
                    viewer.close();
                }
                Ok(())
            }
        }
    }

    pub(super) fn health(&self, lag: &[u64]) -> Health {
        Health {
            head: EventSeq::new(self.next),
            queue_limit_bytes: ByteCount::new(self.limit),
            viewers: self.viewers.values().map(Viewer::health).collect(),
            lag: lag.iter().map(|bytes| ByteCount::new(*bytes)).collect(),
            accept_failures: Count::new(self.accept.failures()),
        }
    }

    /// Ends every viewer and joins every thread.
    pub(super) fn close(mut self) -> Result<(), SoakError> {
        for (_, viewer) in std::mem::take(&mut self.viewers) {
            viewer.close();
        }
        self.accept.close()
    }
}
