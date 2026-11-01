//! The CLI client against real named pipes. The busy-wait test first polls the open
//! once and requires it to be pending, which happens only after the open was refused
//! with `ERROR_PIPE_BUSY` and is waiting in `WaitNamedPipeW`; only then is an instance
//! freed. A pipe that stays busy is `pipe_busy_timeout`; production sessions are in
//! `sessions`.

use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::task::Poll;
use std::time::Duration;

use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};

use super::open;
use crate::invocation::error::CliError;

#[path = "pipe_tests/served.rs"]
mod served;
#[path = "pipe_tests/sessions.rs"]
mod sessions;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

static NEXT: AtomicU32 = AtomicU32::new(0);

fn unique(prefix: &str) -> String {
    let unique = NEXT.fetch_add(1, Ordering::Relaxed);
    format!(r"\\.\pipe\ovt60-{prefix}-{}-{unique}", std::process::id())
}

#[tokio::test]
async fn a_busy_pipe_is_waited_out_and_opened_once_an_instance_is_free() -> TestResult {
    let name = unique("busy");
    let first = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&name)?;
    let holder = ClientOptions::new().open(&name)?;
    let mut opening = pin!(open(&name, Duration::from_secs(20)));
    let polled = std::future::poll_fn(|cx| Poll::Ready(opening.as_mut().poll(cx))).await;
    assert!(
        polled.is_pending(),
        "the open must be refused with ERROR_PIPE_BUSY and wait for an instance: {polled:?}"
    );
    let second = ServerOptions::new().create(&name)?;
    let opened = opening.await;
    assert!(opened.is_ok(), "{opened:?}");
    drop((holder, first, second));
    Ok(())
}

#[tokio::test]
async fn a_pipe_that_stays_busy_is_pipe_busy_timeout_after_the_bound() -> TestResult {
    let name = unique("taken");
    let only = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&name)?;
    let holder = ClientOptions::new().open(&name)?;
    let refused = open(&name, Duration::from_millis(300)).await;
    let Err(error) = refused else {
        return Err("a pipe whose only instance is taken was opened".into());
    };
    assert_eq!(error.kind(), "pipe_busy_timeout", "{error:?}");
    assert!(
        matches!(error, CliError::PipeBusy { waited_ms: 300, .. }),
        "{error:?}"
    );
    let missing = open(&unique("absent"), Duration::from_millis(300)).await;
    assert!(
        matches!(
            missing,
            Err(CliError::NotRunning {
                state: "stopped",
                ..
            })
        ),
        "{missing:?}"
    );
    drop((holder, only));
    Ok(())
}
