//! The single writer of a connection, and what it does when the byte budget overflows.
//!
//! Every write races the budget's overflow signal. On overflow the writer abandons a
//! write that is blocked on a peer that is not reading, which frees that record, closes
//! its queue and frees every queued record, offers `$/closing` once without waiting, and
//! shuts its side down. Nothing waits for the peer or for server shutdown.

use std::future::poll_fn;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;

use ovis_terminal_protocol::wire::{Channel, Frame, FrameKind};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, watch};

use super::ConnectionError;
use super::budget::{Budget, Overflow};
use super::outbound::{Closing, Outbound, encoded_size, record};

/// Why the writer stopped.
enum Stop {
    /// The queue ended or `$/closing` was written.
    Drained,
    /// The budget overflowed; `mid_record` says a record was partly written.
    Overflowed { hit: Overflow, mid_record: bool },
}

/// Writes queued records in order until the queue ends, `$/closing` is written or the
/// budget overflows.
pub(super) async fn write_loop<W: AsyncWrite + Unpin>(
    mut writer: W,
    mut queue: mpsc::Receiver<Outbound>,
    budget: Arc<Budget>,
) -> Result<(), ConnectionError> {
    let mut overflow = budget.subscribe();
    let mut framed = false;
    let stop = loop {
        let item = tokio::select! {
            biased;
            hit = overflowed(&mut overflow) => break Stop::Overflowed { hit, mid_record: false },
            item = queue.recv() => item,
        };
        let (json, last) = match item {
            None | Some(Outbound::End) => break Stop::Drained,
            Some(Outbound::Upgrade) => {
                framed = true;
                continue;
            }
            Some(Outbound::Record(json)) => (json, false),
            Some(Outbound::Closing(closing)) => (record("$/closing", &closing)?, true),
        };
        let reserved = if last { 0 } else { encoded_size(&json) };
        let bytes = match encode(json, framed) {
            Ok(bytes) => bytes,
            Err(error) => {
                budget.record_connection_end();
                return Err(error);
            }
        };
        let written = tokio::select! {
            biased;
            hit = overflowed(&mut overflow) => {
                budget.release(reserved);
                break Stop::Overflowed { hit, mid_record: true };
            }
            written = writer.write_all(&bytes) => written,
        };
        budget.release(reserved);
        if let Err(error) = written {
            budget.record_connection_end();
            return Err(error.into());
        }
        if last {
            break Stop::Drained;
        }
    };
    match stop {
        Stop::Drained => match writer.shutdown().await {
            Ok(()) => Ok(()),
            Err(error) => {
                budget.record_connection_end();
                Err(error.into())
            }
        },
        Stop::Overflowed { hit, mid_record } => {
            abandon(&mut queue, &budget);
            // After a partly written record the stream is mid-frame, so `$/closing`
            // would be garbage to the peer; it is offered only at a record boundary.
            if !mid_record {
                let closing = Closing::of(&ConnectionError::Backpressure {
                    limit: hit.limit,
                    queued: hit.queued,
                });
                write_once(
                    &mut writer,
                    &encode(record("$/closing", &closing)?, framed)?,
                )
                .await;
            }
            shutdown_once(&mut writer).await;
            Ok(())
        }
    }
}

/// Resolves with the first refused reservation; never resolves before one.
async fn overflowed(signal: &mut watch::Receiver<Option<Overflow>>) -> Overflow {
    let hit = signal
        .wait_for(Option::is_some)
        .await
        .ok()
        .and_then(|hit| *hit);
    match hit {
        Some(hit) => hit,
        None => std::future::pending().await,
    }
}

/// Closes the queue and frees every record still in it, releasing their budget.
fn abandon(queue: &mut mpsc::Receiver<Outbound>, budget: &Budget) {
    queue.close();
    while let Ok(item) = queue.try_recv() {
        if let Outbound::Record(json) = item {
            budget.release(encoded_size(&json));
        }
    }
}

/// Offers `bytes` to the transport once; whatever cannot be written now is dropped.
async fn write_once<W: AsyncWrite + Unpin>(writer: &mut W, bytes: &[u8]) {
    poll_fn(
        |context| match Pin::new(&mut *writer).poll_write(context, bytes) {
            // Written, refused or blocked: the connection closes next either way.
            Poll::Ready(Ok(_) | Err(_)) | Poll::Pending => Poll::Ready(()),
        },
    )
    .await;
}

/// Shuts the write side down if the transport allows it without waiting.
async fn shutdown_once<W: AsyncWrite + Unpin>(writer: &mut W) {
    poll_fn(
        |context| match Pin::new(&mut *writer).poll_shutdown(context) {
            // The read half is dropped right after, which closes the socket regardless.
            Poll::Ready(Ok(()) | Err(_)) | Poll::Pending => Poll::Ready(()),
        },
    )
    .await;
}

fn encode(json: Vec<u8>, framed: bool) -> Result<Vec<u8>, ConnectionError> {
    if framed {
        return Ok(Frame::new(Channel::CONTROL, FrameKind::Data, json)?.encode()?);
    }
    let mut line = json;
    line.push(b'\n');
    Ok(line)
}
