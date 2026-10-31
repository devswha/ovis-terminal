//! The connection's read classification: a transport error is never end of stream.
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, ReadBuf};

use super::{Connection, Wait};
use crate::engine::transport::Opened;
use crate::evidence::ConnectionReport;

/// A transport whose every read fails, as a reset peer does.
struct Broken;

impl AsyncRead for Broken {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Poll::Ready(Err(std::io::Error::from(
            std::io::ErrorKind::ConnectionReset,
        )))
    }
}

fn connection(reader: crate::engine::transport::Reader) -> Connection {
    let opened = Opened {
        reader,
        writer: Box::new(tokio::io::sink()),
        bridge: None,
    };
    Connection::new(opened, ConnectionReport::default(), "secret".to_owned())
}

#[tokio::test]
async fn a_read_error_is_a_typed_io_failure_and_only_a_clean_end_is_end_of_stream() {
    let failed = connection(Box::new(Broken))
        .next(Wait::within(1000, "a record"))
        .await;
    assert_eq!(
        failed.as_ref().err().map(crate::error::RunnerError::kind),
        Some("io"),
        "{failed:?}"
    );
    let ended = connection(Box::new(tokio::io::empty()))
        .next(Wait::within(1000, "a record"))
        .await;
    assert!(matches!(ended, Ok(None)), "{ended:?}");
}
