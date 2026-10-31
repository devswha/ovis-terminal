//! The WebSocket transport through real sockets, against the local socket (plan todo 28).
#![cfg(unix)]

use std::error::Error;

/// Result of a scenario step; any failure fails the test.
pub type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

#[path = "../src/ws/qa/client.rs"]
pub mod client;

#[path = "../src/ws/qa/socket.rs"]
pub mod socket;

#[path = "../src/ws/qa/seams.rs"]
pub mod seams;

#[path = "../src/ws/qa/fixture.rs"]
pub mod fixture;

#[path = "../src/streams/qa/viewer.rs"]
pub mod viewer;

#[path = "../src/ws/qa/happy.rs"]
pub mod happy;

#[path = "../src/ws/qa/failure.rs"]
pub mod failure;

#[path = "../src/ws/qa/violations.rs"]
pub mod violations;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn happy() -> TestResult {
    happy::happy().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failure() -> TestResult {
    failure::failure().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transport_violations_close_with_typed_reasons() -> TestResult {
    violations::violations().await
}
