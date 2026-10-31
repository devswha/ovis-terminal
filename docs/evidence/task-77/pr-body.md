fix(ops): deliver cancellations that arrive before the durable accept

Todo 77 (wave 10, follow-up to todo 14). Closes GAP-040 (regression), delivers IDEAL-050 (regression).

## Before

- `docs/evidence/task-21/pr-body.md:172`: "`op.cancel` before an admitted operation's durable accept returns `not_found` and never signals the in-flight task, which then runs to `deadline_exceeded` (dispatcher, todo 14 code). Reproduced over the socket. The scenarios await `$/fixture/held`, which is sent after accept". The workaround is `conformance/src/fixture/documents.rs:18` (`HELD_NOTIFICATION`).
- Reproduced again on the unmodified base `b280ac9`. The new `tests/ops_cancel.rs` and `src/dispatch/qa/cancel_*.rs` were copied into a detached worktree of `origin/integration` and run with `cargo test --locked -p ovis-terminal-server --test ops_cancel -- --nocapture`, which exited 101 (`docs/evidence/task-77/before-ops-cancel.log`):
  - `cancel_wins_the_race_against_the_durable_accept`: `tally cancel_pending=0 cancelled_before_commit=30 not_found=170`. In 170 of 200 pipelined pairs the cancel was answered `not_found`, and the mutation then performed its effect.
  - `every_parallel_interleaving_keeps_cancel_truthful`: the cancel was answered `cancelled_before_commit`, but the request then settled `deadline_exceeded` after 10001 ms. The ledger row was cancelled, yet the running task was never signalled. Cause: `op.cancel` collected its signals before the request reserved, then found the freshly accepted row in the ledger.
  - `happy`: 20 attempts produced no `cancel_pending`.

## After

Mechanism:

- **Reservation** (`crates/ovis-terminal-server/src/dispatch/reservation.rs`). Each dispatched request owns a phase: `Reserved`, `CancelRequested` or `Accepted`. The job holds the phase across its durable accept.
  - If an `op.cancel` from the same principal takes the phase first, the accept commits through `Operations::accept_cancelled`: `op.accepted` and `op.cancelled` in one SQLite transaction. The cancel's reply is that committed record with outcome `cancel_pending`. The handler is never invoked.
  - If the accept committed first, the cancel takes the existing ledger path.
- **Signal after the ledger answers**. A successful cancel now signals matching in-flight tasks *after* the ledger answers, so a request that reserved while the cancel was in flight observes it.
- **Foreign reservation**. A reservation held by another principal resolves as the owner for authorization, so a foreign `op.cancel` stays `forbidden`.
- **Unknown identity**. A cancel for an identity with no reservation and no ledger row stays `not_found`.

Commands, all run on Linux x86_64 (`uname -m` = x86_64):

| Command | Exit | Evidence |
|---|---|---|
| `bash scripts/check.sh` | 0 | `check.log`: fmt, clippy `-D warnings`, nextest `561 tests run: 561 passed (1 slow), 1 skipped`, doctests, hygiene |
| `cargo test -p ovis-terminal-server --test ops_cancel happy -- --exact --nocapture` | 0 | `qa-happy.log` |
| `cargo test -p ovis-terminal-server --test ops_cancel failure -- --exact --nocapture` | 0 | `qa-failure.log` |
| `cargo test -p ovis-terminal-server --test ops_cancel cancel_race -- --nocapture` | 0 | `adv-ordering.log`: `tally cancel_pending=175 cancelled_before_commit=25 not_found=0` (current-thread), `tally cancel_pending=1 cancelled_before_commit=187 not_found=12` (4 workers) |
| `cargo test -p ovis-terminal-core --test ops pending -- --nocapture` | 0 | `adv-stale-state.log` |
| `cargo run -p ovis-terminal-conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-77 --isolated --case all --evidence docs/evidence/task-77/conformance.json` | 0 | `conformance.log`, `conformance.json`: 2 cases, 35 assertions, 0 failed, 0 skipped |
| `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` | 0 | `windows-check.log` |

CI run URL: the PR checks for this head. The manual macOS stress dispatch (`probe-stress-macos.yml`, option `ops-cancel`) is linked in a comment once it has run.

## What the tests assert

- `ops_cancel happy` runs on a multi-thread runtime with exactly one blocking worker, which the test holds, so the durable accept is queued.
  1. A second `doc.set` with the same `op_id` and different params gets `op_reused`. That reply proves the first request is reserved.
  2. `op.cancel` is sent, then another `op_reused` probe; the worker is released after that probe's reply.
  3. It asserts:
     - the cancel reply has `outcome == "cancel_pending"`, `state == "cancelled"` and the same `op_id`;
     - the request reply is the error `cancelled` with that `op_id`;
     - the handler never ran (its entry set does not contain the document);
     - no effect file exists;
     - `op.get` reports `cancelled`.

  Retry: if scheduling reorders the probe, the attempt still checks every invariant except the outcome token, and is retried (at most 20 attempts). The test fails unless the last attempt is `cancel_pending`. Every recorded run needed 1 attempt (`outcomes=["cancel_pending"]`).
- `ops_cancel failure`:
  - an unknown `op_id` gives `not_found`;
  - cancelling a committed operation gives `already_committed` with `state == "committed"` and `result` equal to the original reply's result;
  - a foreign principal (credential issued for another document) gets `forbidden` for the committed op and for the owner's reserved, held op (reservation proven by `op_reused`);
  - after release, the owner's held op commits with its effect `"v"`, so the foreign cancel was not delivered.
- `cancel_race::cancel_wins_the_race_against_the_durable_accept` (current-thread runtime) pipelines `doc.set` then `op.cancel` 200 times. Per iteration it asserts:
  - `cancel_pending`: request `cancelled`, handler never entered, no effect;
  - `cancelled_before_commit`: request `cancelled`, no effect;
  - `not_found`: the test releases the handler, and the request commits `"v"` with its effect present;
  - any other outcome fails the test.

  It asserts the three counters sum to 200 and `cancel_pending >= 1`, using a counter rather than timing.
- `cancel_race::every_parallel_interleaving_keeps_cancel_truthful` runs the same 200 iterations on 4 workers and checks the same per-iteration invariants. It would have caught the unsignalled `cancelled_before_commit` shown in Before.
- `ops::pending::pre_accept_cancel_commits_acceptance_and_cancellation_together_and_survives_restart` (core):
  - `accept_cancelled` returns `cancel_pending` with state `cancelled`;
  - a subscriber receives `op.accepted` at seq+1 and `op.cancelled` at seq+2;
  - `updated_seq` is seq+2;
  - `op.get` equals the record and `start` is refused with `cancelled`.

  After reopening the store with a new boot: replaying `accept` returns the same cancelled record, `start` is refused, and `op.cancel` returns `cancelled_before_commit` with the same record.
- `ops::pending::late_delivery_to_a_retained_identity_reports_what_was_actually_cancelled` (core):
  - `accept_cancelled` on an already-accepted identity gives `cancelled_before_commit`;
  - on a committed one, `already_committed` with the original receipt;
  - with changed params, `op_reused`.
- Conformance `part-a/task-77`:
  - happy: sends the held `doc.set`, then a `doc.get` round trip, then `op.cancel`, without awaiting `$/fixture/held`. It asserts the cancel names the op in state `cancelled` with no error, the held request settles with error `cancelled`, `op.get` reports `cancelled`, and the effects directory is empty.
  - failure: unknown op is `not_found`; cancelling committed work is `already_committed` with its result; `done.json == "kept"`.

  The outcome token is deliberately not asserted, because over a socket the cancel can land on either side of the accept. In the recorded run it landed after the accept (`cancelled_before_commit`). The pre-accept path is proven by the ops_cancel tests above, not by this scenario.

## QA

- happy: `cargo test -p ovis-terminal-server --test ops_cancel happy -- --exact --nocapture` gives exit 0 with `outcomes=["cancel_pending"]`. Transcript: cancel reply `"outcome":"cancel_pending","state":"cancelled","updated_seq":"2"`, the request `"kind":"cancelled"`, and `op.get` `"state":"cancelled"`.
- failure: `cargo test -p ovis-terminal-server --test ops_cancel failure -- --exact --nocapture` gives exit 0. Replies: `not_found`; `already_committed` with `result {"name":"done","value":"kept"}`; `forbidden` twice; then the held op commits.
- conformance: the command in the table gives exit 0, `"result":"passed"`, 35 assertions.

## Adversarial

- **ordering**: both race tests above. In the 4-worker run the cancel landed before the request was reserved 12 times, and each time it was `not_found` and the request's effect really happened.
- **stale_state** (cancel after restart replay): the core restart test above. The reservation is in memory, but a `cancel_pending` reply is only sent after the cancelled accept has committed, so no restart can turn a promised cancellation into a runnable operation. A cancel for an identity that was never accepted before a restart is `not_found` (no stale reservation survives).
- **misleading_success**:
  - `cancel_pending` carries the durable record (a real `updated_seq`, and `op.get` returns the same record), never a synthesized one.
  - The handler-entry check shows that no preparation ran.
  - The parallel race now fails the build if any successful cancel leaves the request unsignalled.
  - `not_found` never hides a cancellation: in that branch the test requires the effect.

## Contract checklist

1. Counters on the wire stay decimal strings: `updated_seq` in every new reply is a string, and no new u64 field is added.
2. No new boundary error enum. Errors reuse `OperationError` / `DispatchError` kinds (`not_found`, `forbidden`, `op_reused`, `cancelled`).
3. No new identities. `OpId`, `PrincipalId` and `Request` are reused.
4. Tests assert state transitions, events, handler entry and file effects, not locally assigned literals.
5. No `#[allow]` or `#[expect]` added and no placeholders. The clippy findings (`too_many_arguments`, `significant_drop_tightening`, `large_enum_variant`) were fixed structurally.
6. Holder wire generation is unchanged (2). The protocol generation is unchanged; the only protocol change is one additive output-enum value.

## Seams

- `crates/ovis-terminal-protocol/src/schema/methods/server/operation.rs`: `CancelOutcome::CancelPending` (`"cancel_pending"`) is added to the open, `#[non_exhaustive]` output enum. `schema/ovis-terminal.openrpc.json` was regenerated with `openrpc write`, and `openrpc compat schema/released.openrpc.json` reports compatible. This file is outside the todo's write scope, but the todo requires the typed outcome.
- `ovis-terminal-core::ops`:
  - new `Operations::accept_cancelled(&Request) -> Result<OpCancelResult, OperationError>`;
  - admission moved verbatim from `context.rs` to `ops/admission.rs`, to stay under the 250-LOC rule;
  - the ledger row schema is unchanged.
- `ovis-terminal-server::dispatch`: no public API change. `Dispatcher` internals now share the task table through an `Arc` so a finished cancel can signal late reservations.
- Test registration slots:
  - `crates/ovis-terminal-core/tests/ops.rs` (`mod pending`);
  - new harness `crates/ovis-terminal-server/tests/ops_cancel.rs` (named by the QA invocation);
  - `conformance/src/runner_tests/suite.rs` (`EXECUTABLE` gains `part-a/task-77.json`, as every new executable scenario must).
- `.github/workflows/probe-stress-macos.yml`: only the `ops-cancel` option and job were appended at the end.

## Risks

- **Ledger ids.** The scenario declares no ledger ids, so part-a/task-77 does not link GAP-040 / IDEAL-050. `docs/design/traceability.json` accepts only plan todos 1-70 as owners, while also requiring every scenario's todo to be listed under each id it declares. A todo-77 scenario therefore cannot claim GAP-040/IDEAL-050 without changing that rule (`conformance/src/traceability.rs:89`/`:126`), which is outside this scope. Follow-up for the traceability owner.
- **`$/fixture/held` barrier not dropped elsewhere.** No existing scenario could drop it:
  - `protocol/multiplex` happy asserts `op.get` state `accepted`, which needs a committed accept.
  - `protocol/deadline` cancels only after 10 s, so the barrier there orders the unrelated `quick` reply.
  - `protocol/multiplex` failure needs the barrier to consume the fixture's notification. The runner's `closed` step fails on any unread record, and the fixture sends `$/fixture/held` only when the accept committed before the cancel, so without the barrier that case becomes nondeterministic.

  For the same reason the task-77 happy case ends with `hangup` + `stop` (no `closed` step).
- **Pipelined cancel can still precede the reservation.** A cancel pipelined on the same connection can still be dispatched before the request it names has reserved, because the connection driver spawns calls concurrently. It is then `not_found` (unknown identity), as specified, and the request runs. Ordering admission per connection would need a `conn::RequestHandler` hook outside this scope.
- **Panic edge.** If a job's blocking worker panicked after taking the phase and before delivering, a waiting pre-accept cancel would end at its own `deadline_exceeded` (10 s). It would not report success.
- The happy test's single held blocking worker assumes nothing else in the server path uses `spawn_blocking` before the accept. It currently holds; if that changes the test fails, it does not pass silently.

## DEFERRED VERIFICATION

None. The todo-21 runner has landed, and `part-a/task-77` is executable and passes.
