fix(ops): admit concurrent operations fairly under the pinned-snapshot bound (todo 79)

Replaces the fail-fast `limit_exceeded` at `pinned_snapshots = 8` with FIFO admission. Each dispatched mutation takes a ticket in the coordinator's snapshot queue at dispatch time. A released slot goes to the oldest waiter: waiting blocks on a `Condvar`, so nothing spins or polls. A fail-fast `pin_snapshot` (paged subscriptions, direct pins) can never take a slot that a waiter is owed. `pinned_snapshots` stays 8 and the snapshot barrier path is unchanged: pin → page → assemble → release under the same WAL read transaction. Each connection may hold at most `queued_operations` (advertised in hello, `"32"`) waiting operations. One more is refused with `limit_exceeded{bound:{limit:"queued_operations",value:"32"}}`. A queued operation is inspectable through `op.get` as `state:"queued"` with a decimal `queued_position`. A deadline that expires while the operation is queued carries `queued_position`.

## Before
Todo-50 gate review N1 (`.omo/evidence/ovis-terminal-v1-task-50-gate-review.md:78-82`): "With 16 concurrent writers, doc.get and doc.set fail with `limit_exceeded` ... n=12 commits 10 of 12, and 2 writers starve; n=16 made no completion within 40 s. Over the socket, 16 writers made zero progress after about 2000 `limit_exceeded` replies."

Reproduced on base 846a210 with this PR's socket test (`docs/evidence/task-79/red-happy-base.log`): `cargo test -p ovis-terminal-server --test ops_admission happy -- --exact --nocapture` → exit 101, `assertion failed: no writer may be refused`; 24 of 32 writers got `limit_exceeded` ("release a pinned snapshot or disconnect an unused subscription").

## After
All commands ran on linux x86_64 in the lane-t79 worktree.
- `cargo test -p ovis-terminal-server --test ops_admission happy -- --exact --nocapture` → exit 0 (`qa-happy.log`). 32 connections, one `doc.set` each on the same document, all sent before any reply was read. Results: 32/32 replies without error, versions exactly {1..32}, the reply at version "32" reports `writers:"32"`, zero `limit_exceeded`, hello `queued_operations:"32"`, and afterwards exactly 8 slots are pinnable again.
- `cargo test -p ovis-terminal-server --test ops_admission failure -- --exact --nocapture` → exit 0 (`qa-failure.log`). All 8 slots are held; 33 = bound + 1 `doc.set` requests are sent on one connection. Exactly one reply is `limit_exceeded`, `retryable:true`, `bound:{limit:"queued_operations",value:"32"}`. `op.get` on each of the other 32 returns `state:"queued"`, `method:"doc.set"` and positions exactly {1..32}. Each of their 10 s deadlines returns `deadline_exceeded` with its `op_id` and `queued_position` (set {1..32}). After the slots are released, all 32 commit with versions {1..32} and no `queued_position`. The refused op is `not_found`.
- `bash scripts/check.sh` → exit 0, 704 tests run, 704 passed, 1 skipped (`check.log`).
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` → exit 0 (`clippy.log`).
- `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` → exit 0 (`windows-check.log`).
- `cargo run -p ovis-terminal-protocol --bin openrpc -- compat schema/released.openrpc.json` → exit 0, `{"compatible_with":"schema/released.openrpc.json"}` (`schema-compat.log`).
- CI run URL: see PR checks.

## Conformance
`cargo run -p ovis-terminal-conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-79 --isolated --case all --evidence docs/evidence/task-79/conformance.json` → exit 0: 2 cases passed, 120 assertions, 0 skipped (`conformance.log`, `conformance.json`). Executable scenario (format marker present):
- happy: hello advertises `queued_operations:"32"`; 32 pipelined `doc.set` on one connection all reply with their value and no error; 32 effect files exist.
- failure: an `op.get` for an identity never dispatched is `not_found` with no `queued_position`; a committed op reports `committed` with no `queued_position`.

The runner fixture holds no snapshot leases, so the bound-exceeded and queued-deadline paths are covered by the socket test `ops_admission failure`, not by the scenario.

## QA
| Scenario | Command | Result |
|---|---|---|
| happy | `cargo test -p ovis-terminal-server --test ops_admission happy -- --exact --nocapture` | exit 0, 32/32 commit, final version "32", no limit_exceeded |
| failure | `cargo test -p ovis-terminal-server --test ops_admission failure -- --exact --nocapture` | exit 0, bound+1 → limit_exceeded{bound}, queued deadline carries queued_position |
| conformance | as above | exit 0 |

## Adversarial
- starvation: core `fairness::thirty_two_concurrent_writers_all_commit_under_eight_pinned_snapshots` sends 32 barrier-synchronized threads through accept → start → commit, and all 32 commit. The socket happy test runs 32 writers. `admission::a_fail_fast_pin_cannot_take_the_slot_a_waiter_is_owed` checks that a released slot cannot be stolen by `pin_snapshot` while a ticket waits.
- ordering (FIFO): `admission::released_slots_reach_waiters_in_arrival_order`. Five tickets get positions 1..5, their waiter threads start in reverse order, a single slot is released, and the grants are observed in order 0,1,2,3,4. The socket failure test checks that positions are exactly {1..32}.
- misleading_success: a queued op is reported as `queued` and never as `accepted` or `committed`. Its deadline is an error, not a success. The refused op stays `not_found` after the queue drains. The conformance failure case checks that no `queued_position` is invented for unknown or committed ops. `withdrawn_and_dropped_tickets_leave_the_line` covers shutdown: a withdrawn ticket fails `unavailable`, and a dropped ticket frees its place.

## Seams (public API changes, all additive)
- protocol: `OpState::Queued`; `OpRecord.queued_position: Option<QueuePosition>` (skip-if-none) and `OpRecord::queued`; scalar `QueuePosition` (decimal string); `Limits.queued_operations: Count` = 32; `ErrorData.queued_position` and `ErrorData.bound: Option<ExceededBound>`; `ExceededBound`/`LimitName`; `BoundaryError::with_queued_position`/`with_bound`. `schema/ovis-terminal.openrpc.json` regenerated; the compat gate against `released.openrpc.json` passes.
- core events: `SnapshotQueue`, `QueueTicket`, `SnapshotSlot`, `TicketId`, `Coordinator::snapshot_queue`; `EventError::AdmissionWithdrawn` (kind `unavailable`). `read_snapshot` now waits in FIFO order instead of failing at the budget. `pin_snapshot` also refuses while a waiter is queued.
- core ops: `Operations::accept_granted`, `Operations::accept_cancelled_granted`. The ledger schema and the params/receipt files are unchanged (todo 78 scope).
- `ops/completion.rs`: `Outcome::Failed` is now boxed and `Completion::failed` is no longer `const fn`. This was required because `ErrorData` grew and tripped `clippy::large_enum_variant`. No caller used it in a const context.
- server dispatch: `DispatchError::QueueFull{bound}` (kind `limit_exceeded`, retryable).
- Updated landed test `ops/qa/inspection.rs`: `inspection_requires_an_available_snapshot_lease` became `inspection_waits_for_a_real_snapshot_lease`. The fail-fast behavior it asserted is what this todo replaces.
- Shared files: `.github/workflows/probe-stress-macos.yml` (option `ops-admission` and job appended at the end) and `conformance/src/runner_tests/suite.rs` (EXECUTABLE += `part-a/task-79.json`).

## Contract checklist
1. `queued_position`, `bound.value` and `queued_operations` are decimal strings. `tests/queued_admission.rs` round-trips `u64::MAX` (proptest over every position) and checks that the numeric form fails to decode. `limits_counts.rs` covers `queued_operations`.
2. The new boundary errors (`EventError::AdmissionWithdrawn`, `DispatchError::QueueFull`) are thiserror variants with `kind()`/`hint()`; Display does not repeat the kind.
3. `QueuePosition`, `TicketId` and `LimitName` are distinct newtypes.
4. Tests assert behavior: grant order, typed kinds, versions and counts on the real socket and SQLite.
5. No `#[allow]`/`#[expect]` was added and no placeholder TODOs exist.
6. Holder wire generation is unchanged (2). The contract checklist items are unchanged.

## Risks
- The plan block tags this todo "Closes: GAP-041 (regression)". In `docs/design/gap-list.md`, GAP-041 is attach readiness, and the traceability test (`every_registered_scenario_is_listed_under_each_id_it_declares`) rejects the claim. The scenario therefore declares `closes: []`. The orchestrator should fix the plan's GAP id.
- origin/integration moved from 846a210 to b9be58f after dispatch. Per the lane rule I did not merge. `git merge-tree` shows conflicts only in the two shared union files (`probe-stress-macos.yml`, `suite.rs` EXECUTABLE list), so the PR may show as CONFLICTING until the orchestrator union-merges them.
- A domain whose effect itself reads `read_snapshot` while all 8 slots are pinned by long-lived leases now waits instead of failing fast. Today only paged subscriptions hold long leases, and they are bounded by `subscriptions`.
- `capacity_accepts_1024_and_rejects_1025_without_eviction` runs about 115 s. It already ran about 126 s before this change (task-15/77 evidence), so this is not a regression.
