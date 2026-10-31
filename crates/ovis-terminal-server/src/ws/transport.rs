//! The byte stream a connection reads and writes, carried in binary WebSocket messages.
//!
//! Each `poll_write` the connection makes becomes exactly one outbound message, and the
//! connection writes each record (an NDJSON line or one frame) with a single
//! `write_all`, so a record is never split across messages or merged with the next.
//! A write completes only once the pump has room for it, which keeps the connection's
//! unwritten-byte budget meaningful. Inbound payloads are read back to back.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc::error::SendError;
use tokio::sync::mpsc::{self, OwnedPermit};

/// Messages queued between the connection and the pump in each direction. One keeps a
/// peer that stops reading from holding more than one record outside the budget.
pub(super) const QUEUED_MESSAGES: usize = 1;

type Reservation =
    Pin<Box<dyn Future<Output = Result<OwnedPermit<Vec<u8>>, SendError<()>>> + Send>>;

/// The connection's side of a WebSocket.
pub(super) struct MessageStream {
    inbound: mpsc::Receiver<Vec<u8>>,
    unread: Vec<u8>,
    read_at: usize,
    outbound: Option<mpsc::Sender<Vec<u8>>>,
    reserving: Option<Reservation>,
}

impl MessageStream {
    pub(super) const fn new(
        inbound: mpsc::Receiver<Vec<u8>>,
        outbound: mpsc::Sender<Vec<u8>>,
    ) -> Self {
        Self {
            inbound,
            unread: Vec::new(),
            read_at: 0,
            outbound: Some(outbound),
            reserving: None,
        }
    }
}

impl AsyncRead for MessageStream {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            if let Some(unread) = this
                .unread
                .get(this.read_at..)
                .filter(|rest| !rest.is_empty())
            {
                let count = buf.remaining().min(unread.len());
                buf.put_slice(unread.get(..count).unwrap_or_default());
                this.read_at = this.read_at.saturating_add(count);
                return Poll::Ready(Ok(()));
            }
            match ready!(this.inbound.poll_recv(context)) {
                Some(payload) => (this.unread, this.read_at) = (payload, 0),
                // The pump stopped reading: the peer closed, so this is end of stream.
                None => return Poll::Ready(Ok(())),
            }
        }
    }
}

impl AsyncWrite for MessageStream {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let Some(outbound) = &this.outbound else {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        };
        let reservation = this
            .reserving
            .get_or_insert_with(|| Box::pin(outbound.clone().reserve_owned()));
        let reserved = ready!(reservation.as_mut().poll(context));
        this.reserving = None;
        let Ok(permit) = reserved else {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        };
        permit.send(buf.to_vec());
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.reserving = None;
        this.outbound = None;
        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::sync::mpsc;

    use super::{MessageStream, QUEUED_MESSAGES};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[tokio::test]
    async fn every_write_is_one_message_and_waits_for_room() -> TestResult {
        let (_inbound, receiver) = mpsc::channel(QUEUED_MESSAGES);
        let (sender, mut outbound) = mpsc::channel(QUEUED_MESSAGES);
        let mut stream = MessageStream::new(receiver, sender);
        stream.write_all(b"{\"id\":\"1\"}\n").await?;
        let blocked = tokio::time::timeout(std::time::Duration::ZERO, stream.write_all(b"x"));
        assert!(blocked.await.is_err(), "a full queue holds the next write");
        assert_eq!(
            outbound.recv().await.as_deref(),
            Some(&b"{\"id\":\"1\"}\n"[..])
        );
        stream.write_all(b"{\"id\":\"2\"}\n").await?;
        assert_eq!(
            outbound.recv().await.as_deref(),
            Some(&b"{\"id\":\"2\"}\n"[..])
        );
        stream.shutdown().await?;
        assert_eq!(
            outbound.recv().await,
            None,
            "shutdown ends the outbound side"
        );
        assert!(stream.write_all(b"late").await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn the_outbound_side_reports_a_gone_pump_as_a_broken_pipe() -> TestResult {
        let (_inbound, receiver) = mpsc::channel(QUEUED_MESSAGES);
        let (sender, outbound) = mpsc::channel(QUEUED_MESSAGES);
        let mut stream = MessageStream::new(receiver, sender);
        drop(outbound);
        let refused = stream
            .write_all(b"reply")
            .await
            .map_err(|error| error.kind());
        assert_eq!(refused, Err(std::io::ErrorKind::BrokenPipe));
        Ok(())
    }

    proptest! {
        #[test]
        fn inbound_payloads_read_back_as_one_stream_under_every_split(
            payloads in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..64), 0..12),
            chunk in 1_usize..17,
        ) {
            let runtime = tokio::runtime::Builder::new_current_thread().build()
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            let read = runtime.block_on(async {
                let (inbound, receiver) = mpsc::channel(payloads.len().max(1));
                let (sender, _outbound) = mpsc::channel(QUEUED_MESSAGES);
                for payload in &payloads {
                    inbound.send(payload.clone()).await.map_err(|_| "closed")?;
                }
                drop(inbound);
                let mut stream = MessageStream::new(receiver, sender);
                let mut read = Vec::new();
                let mut buffer = vec![0_u8; chunk];
                loop {
                    let count = stream.read(&mut buffer).await.map_err(|_| "read")?;
                    if count == 0 {
                        return Ok::<_, &str>(read);
                    }
                    read.extend_from_slice(buffer.get(..count).ok_or("count")?);
                }
            }).map_err(TestCaseError::fail)?;
            prop_assert_eq!(read, payloads.concat());
        }
    }
}
