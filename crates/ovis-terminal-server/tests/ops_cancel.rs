//! `op.cancel` against the durable accept, through the real socket and operation ledger.
#![cfg(unix)]

#[path = "../src/dispatch/qa/cancel_fixture.rs"]
pub mod cancel_fixture;
#[path = "../src/dispatch/qa/cancel_race.rs"]
/// Cancellation racing the accept.
pub mod cancel_race;
#[path = "../src/dispatch/qa/cancel_refusal.rs"]
/// Cancellations that must not be delivered.
pub mod cancel_refusal;

use cancel_fixture::TestResult;

#[test]
fn happy() -> TestResult {
    cancel_race::happy()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failure() -> TestResult {
    cancel_refusal::failure().await
}
