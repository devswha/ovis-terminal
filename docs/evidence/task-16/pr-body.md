## Before

RT-HC-03, GAP-095, GAP-096 and GAP-126: `/tmp/ulw-ovis-yCJzPH/evidence/smoke/hc-s15-label-templates.log:4`, with durable copy `docs/evidence/baseline/smoke/hc-s15-label-templates.log:4`. The earlier interrupt reply claimed `outcome="stopped"` while foreground argv remained `["sleep","30"]`.

## After

Terminal operations now use the existing registry identities, holder ownership, authenticated authority decisions and operation ledger. Launch admission, pending holder work, confirmed spawn and failed spawn are separate facts. The Coordinator commits operation outcomes, identity reservations, terminal state and ordered lifecycle events in one SQLite transaction before publication.

DEFERRED VERIFICATION (runner lands in Todo 21)

Executed on Linux x86_64, rustc 1.99.1 (58cf8f9c3 2026-10-13):

```sh
ATTEMPT_DIR=/home/devswha/workspace/ovis-terminal-wt/lane-t16/.omo/evidence/ovis-terminal-v1/task-16
cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-16 --isolated --case happy --evidence "$ATTEMPT_DIR/task-16-ovis-terminal-v1.json"
```

```text
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
CONFORMANCE_EXIT=2
Linux x86_64
```

No CLI/socket request was dispatched, and no runner evidence JSON was produced. `conformance-resumed.md` preserves the complete resumed attempt. The JSON at the requested evidence path is explicitly an executor-attempt artifact, not a conformance pass.

The real holder/SQLite/authority acceptance cases were executed separately:

```sh
cargo test -p ovis-terminal-core --test terminals happy -- --exact --nocapture
cargo test -p ovis-terminal-core --test terminals failure -- --exact --nocapture
```

Both exit 0 and each runs one assertion-bearing case. `lifecycle-qa.log` contains the literal component output, not a CLI transcript. Parsed facts from that output:

```json
{
  "cwd": "/tmp/ovis-t16-01a19081-a18a-73da-93bd-a9c4def8f4b5",
  "env": {
    "OVIS_FIXTURE": "explicit-environment",
    "PATH": "/usr/bin:/bin"
  },
  "geometry": {
    "cols": 103,
    "rows": 37
  },
  "output_bytes": [
    99,
    119,
    100,
    61,
    47,
    116,
    109,
    112,
    47,
    111,
    118,
    105,
    115,
    45,
    116,
    49,
    54,
    45,
    48,
    49,
    97,
    49,
    57,
    48,
    56,
    49,
    45,
    97,
    49,
    56,
    97,
    45,
    55,
    51,
    100,
    97,
    45,
    57,
    51,
    98,
    100,
    45,
    97,
    57,
    99,
    52,
    100,
    101,
    102,
    56,
    102,
    52,
    98,
    53,
    13,
    10,
    101,
    110,
    118,
    61,
    101,
    120,
    112,
    108,
    105,
    99,
    105,
    116,
    45,
    101,
    110,
    118,
    105,
    114,
    111,
    110,
    109,
    101,
    110,
    116,
    13,
    10,
    97,
    114,
    103,
    61,
    108,
    105,
    116,
    101,
    114,
    97,
    108,
    32,
    59,
    32,
    97,
    114,
    103,
    117,
    109,
    101,
    110,
    116,
    13,
    10,
    51,
    55,
    32,
    49,
    48,
    51,
    13,
    10,
    195,
    169
  ],
  "exit": {
    "target": {
      "terminal_id": "terminal:1",
      "terminal_incarnation": "1",
      "process_incarnation": "1"
    },
    "code": 7,
    "signal": null,
    "exit_unknown": null,
    "io_failure": null,
    "final_offset": "116"
  },
  "kill_receipt": {
    "terminal": "terminal:2",
    "terminal_incarnation": "1",
    "process": {
      "id": "process:2",
      "generation": "1",
      "process": {
        "pid": "1682359",
        "start_token": "linux:44213f41-39db-4eed-a62b-f5e5992ca358:28603798"
      }
    },
    "delivered": [
      "hangup"
    ],
    "exit": {
      "target": {
        "terminal_id": "terminal:2",
        "terminal_incarnation": "1",
        "process_incarnation": "2"
      },
      "code": null,
      "signal": 1,
      "exit_unknown": null,
      "io_failure": null,
      "final_offset": "5"
    }
  }
}
```

### Reconciliation with todo 76

The integration merge at dfa7de2 composed todo 16's "child unreaped at EOF" barrier ahead of todo 76's reap-under-lock. A same-group descendant can hold the PTY slave indefinitely after the leader exits, so the waiter blocked forever. Three todo-76 shutdown tests timed out at 30 s.

Resolution (full reasoning in `docs/evidence/task-16/reconciliation.md`): the barrier was a means, not the contract.

- The waiter is todo 76's again: `waitid(EXITED|NOWAIT)` (retried on EINTR), then the reap under the holder-state mutex (`WaiterState::Reaped`), then the pending-byte drain and the slave drop. `DrainSignal::wait_until_finished` and `HolderState::begin_reap` are removed.
- `record_hangup` publishes Hangup from the PID and start token retained in `HeldTerminal` at spawn. These survive the reap, so a post-reap EOF is still correctly attributed. The pump still publishes Hangup before it consumes the waiter report and records Exit, and Exit stays last.
- `Holder::hangup` now uses todo 76's reap-serialized, incarnation-checked delivery (`signalling::signal_with` + `process::send_hangup`). A reaped leader's group is still addressed, a replaced PID never is, and the PID 0/1 refusal covers SIGHUP. No wire record changed.

Follow-up after CI on 10888e3. `each_launch_gets_its_own_cwd_and_environment_over_the_socket` received Exit with no output on ubuntu. `pty_io::read_chunk` returned `Hangup` as soon as `poll` reported POLLHUP, even with POLLIN set. With the reap no longer delayed, the slave can close while the child's last bytes are still in the tty flip buffer (FIONREAD reads 0), so those bytes were dropped. Hang-up readiness now only marks the PTY hung up. The pump keeps reading until a read reports EOF/EIO or finds nothing, and PumpStop still ends output at once. The deterministic regression `pty_io::tests::hangup_readiness_with_queued_final_bytes_returns_them_before_hangup` runs a real pipe with POLLIN|POLLHUP behind a first read that finds nothing. It fails with the old decision line and passes with the fix.

Tests guarding the invariants:

- I-76: `shutdown_tests::{shutdown_reports_already_exited_when_waiter_reaped_before_output_completion, shutdown_kills_same_group_descendant_after_leader_is_reaped, shutdown_stops_pump_when_reaped_leader_group_is_gone, shutdown_reports_zombie_without_signalling_before_waiter_reaps}`.
- I-16: `reap_tests::hangup_after_reap_carries_spawned_incarnation_and_precedes_exit` replaces `child_is_unreaped_at_the_real_pty_hangup_barrier`. With the leader reaped and nothing queued yet, it asserts exactly `[Hangup, Exit]`: the Hangup carries the spawned pid and start token at `final_offset`, the Exit has code 3, and nothing follows Exit.
- I-16 + I-76: `reap_tests::shutdown_with_descendant_holding_slave_is_bounded_and_keeps_hangup_before_exit`. After the reap, `hangup()` reaches the HUP-ignoring descendant's group. Shutdown returns in under 2 s with all workers joined and `already_exited == 1`. Delivered lifecycle events are `[]`, `[Hangup]` or `[Hangup, Exit]`, and the exit is recorded at the tail head.

Reconciliation verification (Linux x86_64):

| Command | Exit | Result |
| --- | --- | --- |
| `cargo nextest run -p ovis-terminal-holder -p ovis-terminal-core --all-targets --locked` | 0 | 149 passed (run before the final lint-only edits: `send_hangup` visibility, `matches!` assertion form) |
| `bash scripts/check.sh` (final code, after the read_chunk fix) | 0 | fmt, workspace clippy `-D warnings`, workspace nextest 434 passed / 1 skipped, doctests, 0 hygiene violations in 429 files, manifest + JSON gates; `reconcile-check.log` |
| `cargo nextest run -p ovis-terminal-holder --locked -E 'test(hangup_readiness_with_queued_final_bytes)'` with the old `poll`-hangup decision line restored | 101 | the regression fails ("queued final bytes were dropped as a hang-up"); restored fix: 57/57 holder tests pass |
| `cargo nextest run -p ovis-terminal-holder --locked` x50 under 8 busy-loop CPU hogs (final code) | 0 | `STRESS_DONE pass=50 fail=0`, 57/57 each (rerun after the read_chunk fix); `reconcile-stress.log` |

## QA

Recorded in docs/evidence/task-16/checks.log and lifecycle-qa.log:

| Command | Exit | Assertion-bearing result |
| --- | --- | --- |
| cargo test -p ovis-terminal-core --test terminals happy -- --exact --nocapture | 0 | 1 passed; exact cwd/env/argument/grid/output bytes, exit 7, named kill token, metadata-removal isolation |
| cargo test -p ovis-terminal-core --test terminals failure -- --exact --nocapture | 0 | 1 passed; stale start token rejected without signalling, executable-not-found settles operation as spawn_failed |
| cargo nextest run -p ovis-terminal-core | 0 | 91 passed, 0 skipped |
| cargo clippy -p ovis-terminal-core --all-targets -- -D warnings | 0 | No diagnostics |
| cargo nextest run -p ovis-terminal-holder --no-tests=fail --status-level fail --final-status-level fail | 0 | 46 passed, 0 skipped |
| bash scripts/check.sh | 0 | Workspace Clippy, 392 nextest tests passed, doctest command passed, 0 hygiene violations in 395 Rust files, manifest and JSON gates passed |
| cargo metadata --locked --no-deps --format-version 1 | 0 | Locked workspace metadata resolves |

The workspace's one ignored store test is the existing child-process fixture at crates/ovis-terminal-store/tests/transactions.rs:43; its parent scenarios execute it. No test was removed, disabled or weakened. The operation-capacity test is the one slow case (approximately 120 seconds).

## Adversarial

- Stale state / PID reuse: the failure case supplies the current PID with a different start token and asserts process_replaced. The live native token remains unchanged. Full identity is checked at every escalation stage. Every holder signal, SIGHUP included, is serialized with the waiter's reap and refuses a PID whose start token no longer matches.
- Hung command: repeated_kill_and_recover_never_retarget_the_replacement waits for the real fixture's ARMED marker after it installs HUP/TERM traps, then asserts all three delivered stages and actual signal-9 exit.
- Repeated interruption: the same kill op returns its historical receipt after relaunch, and the same recover op creates only one new incarnation. A pending launch is adopted without replay; its interrupted operation stays outcome_unknown.
- Misleading success: Hangup, delivered signals and Exit are separate evidence. The holder test proves that, even when the leader is reaped before PTY EOF, Hangup carries the spawned pid and start token at the final offset and precedes Exit. No status text is used as exit evidence.
- Metadata removal: a live sibling crosses an explicit input/output barrier after removal; no signal handler ran. Removal names only the ended process metadata.
- Authority: a real SQLite COMMIT barrier lets the test revoke the credential after admission but before spawn. No process is created, and the terminal plus operation retain the typed credential_revoked failure.
- Child replacement: after the shell reaps the declared child agent, agent authority is cleared while the independent shell identity remains live.
- Atomicity: a failed terminal CAS rolls back operation outcome, identity allocator and both events. A child aborts at the real post-COMMIT/pre-ack barrier; reopening recovers the operation, terminal resource, reservation and ordered replay together.
- Reboot metadata: the registry transition is exercised with independent host/runtime/control identities; it clears current process and agent authority. This is not a native reboot simulation.

## Contract checklist

1. Runtime u64 sequences, revisions, generations, offsets and durations use existing canonical decimal-string newtypes or holder decimal serializers. Maximum u64 values round-trip exactly, and numeric request generations produce typed invalid_params. Holder extension PID is also canonical decimal text.
2. TerminalError derives thiserror::Error, preserves typed sources and supplies stable kind()/hint(). Existing operation/reservation/frame error enums are reused and extended rather than duplicated; Display does not repeat a machine token.
3. Terminal, process, runtime, control boot, PID, start token and incarnation identities reuse the registry/holder newtypes. Historical process evidence is distinct from current ownership.
4. Existing wire fragmentation properties include the Hangup fixture; every two-fragment boundary is also exercised. Lifecycle persistence and kill-request round trips use proptest, with exact u64::MAX cases. The SQLite tests use the real writer and crash barrier.
5. No new lint suppression, placeholder TODO, focus fallback, screen-status inference, or mocked holder was added. Existing assertions remain intact. Source sizes and parameter counts pass the repository checker.
6. Conformance exited 2. DEFERRED VERIFICATION (runner lands in Todo 21); no component result is presented as a real CLI/socket scenario pass.

## Seams

The two orchestrator decisions are applied without exposing the raw store or adding a writer path:

- crates/ovis-terminal-core/src/events/unit.rs:24: additive Coordinator::commit_unit builds writes from a read-only snapshot and commits all ordered events under the original durable-before-publish lock/barrier. events/record.rs:159 adds Commit::event; existing Coordinator::commit forwards to the unit.
- crates/ovis-terminal-core/src/ops/completion.rs:18, :96, :121: typed Completion, Operations::commit_unit and record_unit bind operation state, evidence, domain writes and events. Operations::commit is now a thin caller. The existing OpRecord wire representation remains unchanged, including when wrapped by LifecycleEvent.
- crates/ovis-terminal-core/src/registry/reservation/staged.rs:36: IdentityReservation::stage returns the existing family's identity with mandatory reservation writes for that same transaction. The original reserve API reuses the same allocator logic.
- crates/ovis-terminal-core/src/registry/lifecycle.rs:88 and registry/lifecycle/terminal.rs:14: add ProcessEnded/Finished, validated serde persistence and historical-process accessors. Registry property tests reject contradictory Running metadata and preserve live/ended round trips.
- crates/ovis-terminal-holder/src/protocol/messages/io.rs:25, :35 and protocol/payload.rs:10: Output gains optional typed Hangup evidence carried outside the unchanged nested payload. Message tags and HOLDER_PROTOCOL_GENERATION remain 2. Legacy adjacent-tagged generation-two readers ignore the optional envelope field while retaining identical target/offset/bytes; today's fixtures replay in the compatibility test.
- crates/ovis-terminal-holder/src/protocol/process_pid.rs:7: move the existing PID newtype and add canonical string serde; src/process.rs preserves its old public re-export.
- crates/ovis-terminal-holder/src/unix/holder/threads.rs:51, :130 and holder/state.rs:83: the waiter reaps under the state mutex as soon as the leader exits (todo 76), then drains pending bytes before dropping the slave. The pump publishes Hangup at EOF from the retained `HeldTerminal` pid and start token, then consumes the waiter report and records Exit. Existing input-ack-before-Exit and final-output invariants remain covered.
- crates/ovis-terminal-holder/src/unix/pty_io.rs (`read_chunk`): POLLHUP readiness only marks the master hung up, and output ends at a read that reports EOF/EIO or finds nothing, so bytes queued behind the slave close still precede Exit. PumpStop ends output immediately. Tests moved to unix/pty_io/tests.rs for the 250-LOC rule.
- crates/ovis-terminal-holder/src/unix/holder/hangup.rs:20, holder/signalling.rs:34 and process/signal.rs:94: the local holder SIGHUP method requires the exact process incarnation and goes through the reap-serialized, start-token-checked `signal_with` and the reserved-PID-refusing `send_hangup`, without changing the frozen Signal wire enum. The Unix-only rustix process dependency is recorded in Cargo.lock.

Todo-5/6 scenarios, property examples, counter tests, and the task-16 scenario are updated. No todo-10/15 test was weakened; additive rollback and crash-replay tests run beside them.

## Risks

- The native lifecycle service is currently a core component, not a claim that the CLI/socket assembly exists. Todo 21 must execute part-a/task-16 on the assembled binary and supply the actual runtime request/reply transcript.
- Local proof is Linux x86_64 only. Native macOS behavior (including waitid and PTY draining) and Windows release behavior are not claimed from this run. Non-Linux native process roles that cannot be observed are explicitly marked unsupported rather than guessed.
- The shell role is explicitly undeclared for arbitrary executable launches; no process-name or agent-brand heuristic invents that role.
- Rust LSP diagnostics could not run because rust-analyzer is missing; compiler, Clippy and source hygiene checks passed. No new unsafe block was introduced.
- Holder loss remains uncertain until native old-process absence or replacement is proven; recovery refuses to duplicate an unproven surviving process.

## Cleanup

All real-process fixtures join their owned holder/control workers and remove their isolated temporary roots. No merge, force-push, unrelated worktree edit or external person-message is part of this change. Resume decisions are recorded per file in resume-assessment.md.

## CI refresh

Head `c501322b804193b9feaf12a23c3f199bbc50debb` records canonical cwd evidence
before operation admission and passed the local `bash scripts/check.sh` gate:
workspace Clippy, 392 nextest tests, doctests, hygiene, manifest and JSON checks
all exited 0. In PR run `41178324576`, the macOS terminal `happy` case also
passed; the sole required-check failure was
`program_stop_names_every_reaped_incarnation_and_leaves_control_available`,
where the landed holder returned `EPERM` after a process-group leader was
reaped.

Integration `a9a4ed4d75c33147ab1503c53e7cf2ee3828e3f2` lands Todo 76's holder fix for
that macOS `killpg` race. This evidence-only commit does not merge integration
or change terminal/holder behavior; its push refreshes PR #23's merge ref so
the unchanged task-16 implementation is verified against the landed fix.
