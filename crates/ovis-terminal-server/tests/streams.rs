//! Typed attach outcomes and isolated output streams through a real socket (plan todo 23).
#![cfg(unix)]

use std::error::Error;

/// Result of a scenario step; any failure fails the test.
pub type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

#[path = "../src/dispatch/qa/socket.rs"]
pub mod socket;

#[path = "../src/streams/qa/fixture.rs"]
pub mod fixture;

#[path = "../src/streams/qa/viewer.rs"]
pub mod viewer;

#[path = "../src/streams/qa/happy.rs"]
pub mod happy;

#[path = "../src/streams/qa/failure.rs"]
pub mod failure;

#[path = "../src/streams/qa/fence.rs"]
pub mod fence;

#[path = "../src/streams/qa/stall.rs"]
pub mod stall;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn happy() -> TestResult {
    happy::happy().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failure() -> TestResult {
    failure::failure().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stalled_viewers_close_without_pausing_others() -> TestResult {
    stall::stalled_viewers_close_without_pausing_others().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn paint_is_fenced_ahead_of_live_bytes_under_credit_pressure() -> TestResult {
    fence::paint_is_fenced_ahead_of_live_bytes_under_credit_pressure().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_exited_terminal_answers_ended() -> TestResult {
    fence::an_exited_terminal_answers_ended().await
}
