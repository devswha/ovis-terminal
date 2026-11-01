//! A line to a machine: newline-terminated protocol records in both directions.

use std::future::Future;

use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader};

use crate::transport::{Link, TransportError};

/// Largest record accepted, the protocol's control-record bound.
const MAX_LINE_BYTES: usize = ovis_terminal_protocol::wire::MAX_FRAME_PAYLOAD;

/// A byte-transparent line carrying one record per newline.
pub trait Line: Send + 'static {
    /// Writes `bytes` unchanged.
    ///
    /// # Errors
    ///
    /// The transport failure that ended the line.
    fn send(&mut self, bytes: &[u8]) -> impl Future<Output = Result<(), TransportError>> + Send;

    /// Reads one record without its newline. Must be cancel safe: a record that has not
    /// ended stays buffered for the next call.
    ///
    /// # Errors
    ///
    /// The transport failure or end of stream that ended the line.
    fn receive(&mut self) -> impl Future<Output = Result<Vec<u8>, TransportError>> + Send;
}

impl Line for Link {
    async fn send(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        Self::send(self, bytes).await
    }

    async fn receive(&mut self) -> Result<Vec<u8>, TransportError> {
        Self::receive(self).await
    }
}

/// A line over any byte stream pair, such as a socket or an in-process pipe.
#[derive(Debug)]
pub struct Lines<R, W> {
    reader: BufReader<R>,
    writer: W,
    partial: Vec<u8>,
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> Lines<R, W> {
    /// Records read from `reader` and written to `writer`.
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader: BufReader::new(reader),
            writer,
            partial: Vec::new(),
        }
    }
}

impl<R, W> Line for Lines<R, W>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    async fn send(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        self.writer.write_all(bytes).await?;
        self.writer.flush().await?;
        Ok(())
    }

    async fn receive(&mut self) -> Result<Vec<u8>, TransportError> {
        loop {
            let available = self.reader.fill_buf().await?;
            if available.is_empty() {
                return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into());
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let taken = newline.map_or(available.len(), |at| at.saturating_add(1));
            self.partial.extend(available.iter().take(taken));
            self.reader.consume(taken);
            if self.partial.len() > MAX_LINE_BYTES {
                return Err(std::io::Error::from(std::io::ErrorKind::InvalidData).into());
            }
            if newline.is_some() {
                let mut line = std::mem::take(&mut self.partial);
                line.pop();
                return Ok(line);
            }
        }
    }
}
