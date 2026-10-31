//! Fair FIFO operation admission under the pinned-snapshot bound, over the real socket.
#![cfg(unix)]

#[path = "../src/dispatch/qa/admission.rs"]
/// Writers beyond the budget, the queue bound and queued deadlines.
pub mod admission;
#[path = "../src/dispatch/qa/admission_fixture.rs"]
pub mod admission_fixture;

use admission_fixture::TestResult;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn happy() -> TestResult {
    admission::happy().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failure() -> TestResult {
    admission::failure().await
}
