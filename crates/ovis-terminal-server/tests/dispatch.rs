//! Strict dispatch through the real persistent connection and durable operation ledger.
#![cfg(unix)]

#[path = "../src/dispatch/qa/authority.rs"]
/// Scoped admission and commit-time revocation.
pub mod authority;
#[path = "../src/dispatch/qa/bounds.rs"]
/// Resource bounds after RPC deadlines.
pub mod bounds;
#[path = "../src/dispatch/qa/cancellation.rs"]
/// Cancellation and joined operation lifetimes.
pub mod cancellation;
#[path = "../src/dispatch/qa/lifecycle.rs"]
/// Deadline recovery over the real socket.
pub mod lifecycle;
#[path = "../src/dispatch/qa/strict.rs"]
/// Malformed targeting and operation inputs.
pub mod strict;
#[path = "../src/dispatch/qa/support.rs"]
pub mod support;
#[path = "../src/dispatch/qa/targeting.rs"]
/// Explicit and ambiguous registry targets.
pub mod targeting;

use support::TestResult;

#[tokio::test]
async fn happy() -> TestResult {
    lifecycle::deadline_keeps_work().await
}

#[tokio::test]
async fn failure() -> TestResult {
    strict::wrong_target_and_committed_cancel().await
}
