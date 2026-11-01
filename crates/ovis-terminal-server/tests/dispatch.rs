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
#[path = "../src/dispatch/qa/convergence.rs"]
pub mod convergence;
#[path = "../src/dispatch/qa/incarnation.rs"]
pub mod incarnation;
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

#[path = "../src/dispatch/lanes/agent_tree/qa.rs"]
/// Children listed by the parent's current program incarnation.
pub mod agent_tree;
#[path = "../src/dispatch/lanes/boundary_policy/wire_qa.rs"]
/// The boundary policy in front of a real endpoint.
pub mod boundary_policy;
#[path = "../src/dispatch/lanes/boundary_policy/history_qa.rs"]
/// `agent.history` below an authorized root.
pub mod boundary_policy_history;
#[path = "../src/dispatch/lanes/conversation_records/qa.rs"]
/// The conversation record lane: typed appends and reads over a real socket.
pub mod conversation_records;
#[path = "../src/dispatch/lanes/credential_rotation/qa.rs"]
/// Credential rotation and per-request revalidation over a real socket.
pub mod credential_rotation;
#[path = "../src/dispatch/lanes/credential_rotation/rig.rs"]
pub mod credential_rotation_rig;
#[path = "../src/dispatch/lanes/prompts/qa.rs"]
/// The prompt lane's independent answer authorization over a real socket.
pub mod prompts;
#[path = "../src/dispatch/lanes/session_binding/qa.rs"]
/// The session binding lane over a real dispatcher and socket.
pub mod session_binding;
#[path = "../src/dispatch/lanes/template_launch/qa.rs"]
/// The template launch lane: readiness gating and write-boundary rechecks on the wire.
pub mod template_launch;
#[path = "../src/dispatch/lanes/template_launch/memory.rs"]
pub mod template_launch_memory;
#[path = "../src/dispatch/lanes/worktrees/authority.rs"]
/// The worktree lane's grant for destructive removal.
pub mod worktree_authority;
#[path = "../src/dispatch/lanes/worktrees/fixture.rs"]
pub mod worktree_fixture;
#[path = "../src/dispatch/lanes/worktrees/qa.rs"]
/// The worktree lane over real Git repositories.
pub mod worktrees;

use support::TestResult;

#[tokio::test]
async fn happy() -> TestResult {
    lifecycle::deadline_keeps_work().await
}

#[tokio::test]
async fn failure() -> TestResult {
    strict::wrong_target_and_committed_cancel().await
}
