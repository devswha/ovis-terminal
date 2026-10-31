## Before

RT-HC-01, GAP-070, GAP-071, GAP-073, GAP-074, GAP-075, GAP-078: `/tmp/ulw-ovis-yCJzPH/evidence/smoke/hc-s06-wait-output.log:6`, durable copy `docs/evidence/baseline/hc-s06-wait-output.log:6`:

```text
--- exit=0 elapsed_ms=5213
```

`hc send` blocked 5213 ms waiting for a status transition on a plain shell, and its success-shaped result did not separate admission, PTY delivery, acceptance or a turn. herdr has no atomic check-and-write against process incarnation and caller lease (GAP-070), raw bytes, keys, paste and RPC sends do not share one ordered sequence (GAP-071), partial writes and quiesced drops are silent (GAP-073), and disconnect neither revokes input-ready nor prevents replay of uncertain input (GAP-078).

## After

`crates/ovis-terminal-core/src/input/sequencer/` and `src/input/receipts/` add one `Sequencer` per terminal incarnation. It is a synchronous state machine the terminal actor owns; it never blocks on the holder or the program. The rules (also in the module docs):

- `Sequencer::admit(&View, Request) -> Result<Receipt, SequencerError>` checks, in order: stream readiness (`not_ready`/`recovering`/`finished`), runtime/machine/terminal/terminal-incarnation/pane identity (`stale_target`) and the expected process incarnation (`process_replaced`), the raw-input grant for raw bytes (decided by the sequencer itself against its own bound terminal, so a grant for another target cannot be used: `raw_not_authorized`), the encoding against the current `ModeSnapshot` (todo 32 kinds), the todo-24 lease rule via `TerminalLeases::authorize_write` (`no_lease`, `forbidden`), and the bounded queue (16 MiB / 4096 entries, `input_overflow`). It answers with a `queued` receipt: `op_id`, `queued_seq`, `kind`, `admitted_bytes`, `mode_revision`, `output_cursor`, `delivery: {"state":"queued"}`, and `agent_accepted`/`turn_started`/`turn_ended` as `unknown` (no declared producer) or `pending` (declared producer). Agent-declared readiness does not gate this queue (the first-message gate stays with todo 35).
- Text and its submission are one `Encoded` entry, so no other writer lands between them.
- `Sequencer::next(&View) -> Step` is the last write boundary. It rechecks the head entry: process incarnation (`stale_target`), `TerminalLeases::recheck` (credential revocation/expiry via the landed `Authority::recheck`, `lease_revoked`, `no_lease`), `RawGrant::recheck`, and `Encoded::ensure_current` (`mode_mismatch`). Failures are fenced with zero bytes and a typed `FenceReason`; the first passing entry becomes one holder `Input` whose `WriteGrant` carries the current process incarnation, the admitting lease epoch (`grant_epoch`, 0 when unleased) and the sequencer fence epoch. One user entry is with the holder at a time.
- `Sequencer::fence(&View)` after takeover/release/credential revocation fences failing unsent entries and, when the entry already with the holder fails its recheck, advances the fence epoch and returns an empty holder `Input` at that epoch (the holder authorization handshake: the frozen generation-2 holder revokes queued entries of an older fence epoch and acknowledges them with 0 bytes). Whatever the holder already wrote is reported as `pty_written`/`partial`; nothing claims rollback.
- `Sequencer::acknowledge(&InputWritten)` turns holder evidence into `pty_written {written_bytes, output_offset}`, `partial {written_bytes, ...}` (exact prefix) or `fenced`. `refused(WriteSequence, FenceReason)` records a holder refusal before queueing (`holder_refusal(&HolderError)` maps it on Unix).
- `link_lost()` revokes input-ready (`recovering`), fences unsent entries with `recovering` and makes the entry with the holder `outcome_unknown`; `exited()` does the same with `finished`. Unknown entries are never sent again after `stream_ready`; a late acknowledgement still resolves them to the exact bytes written.
- `stream_ready(process, acknowledged)` sets the live process (fencing entries admitted for another process as `stale_target`) and continues holder write sequences after the holder's last acknowledged one.
- `barrier(InputSeq) -> BarrierState::{Pending{pending_bytes}, Reached, Undelivered{first}}`; `seq_ahead` for a position never admitted.
- `admit_reply(Vec<u8>)` is a separate bounded lane (64 KiB / 64 entries) for terminal-query replies: dispatched ahead of user entries even while a user entry is blocked with the holder, never a receipt.
- `report(&StageReport)` applies producer evidence: `invalid_record` for a stage without a declared producer or for acceptance before the PTY write, `not_found` for an unknown op.

Holder wire generation stays 2; no holder, lease, encoding or authz code changed.

DEFERRED VERIFICATION (runner lands in Todo 21)

Executed on Linux 7.0.0-38-generic x86_64, rustc 1.99.1 (58cf8f9c3 2026-10-13), after `cargo build --locked -p ovis-terminal-cli --bin ovis-terminal` (exit 0), `ATTEMPT_DIR=/home/devswha/workspace/ovis-terminal-wt/lane-t25/.omo/evidence/ovis-terminal-v1/task-25`:

```text
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-25 --isolated --case happy --evidence "$ATTEMPT_DIR/task-25-ovis-terminal-v1.json"
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
CONFORMANCE_EXIT=2
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-25 --isolated --case failure --evidence "$ATTEMPT_DIR/task-25-ovis-terminal-v1.json"
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
CONFORMANCE_EXIT=2
```

No CLI or socket request was dispatched and no runner evidence JSON exists (`conformance.log`). The `input.*` RPC routes are not wired to the sequencer in this lane (todos 28-31 assemble them), so there is no `ovis-terminal` CLI/socket transcript yet. `conformance/scenarios/part-a/task-25.json` records the happy steps (held PTY writer, two writers' `input.send` replies `queued` within 100 ms, barrier pending then reached after release, `op.get` `pty_written`, expected output hex `616c7068612d310d626574612d310d616c7068612d320d`) and the failure steps (takeover → `lease_revoked`, credential revocation → `credential_revoked`, kill during a blocked 1 MiB write → `partial` prefix and `finished` for the queued entry, process replacement → `stale_target`, holder link drop → `outcome_unknown` with zero re-sends), each with `written_bytes` `"0"` where fenced.

The component acceptance cases ran against the real authority, the real todo-24 lease table, real VT modes and a real holder PTY (`/bin/sh` in raw mode running `cat`):

```text
$ cargo test -p ovis-terminal-core --test input happy -- --exact --nocapture
receipt {"op_id":"alpha-submission-0","queued_seq":"1","kind":"text","admitted_bytes":"19","mode_revision":"0","output_cursor":"0","delivery":{"state":"pty_written","written_bytes":"19","output_offset":"5"},"agent_accepted":{"state":"unknown"},"turn_started":{"state":"unknown"},"turn_ended":{"state":"unknown"}}
receipt {"op_id":"beta-submission-1","queued_seq":"2",...,"delivery":{"state":"pty_written","written_bytes":"18","output_offset":"5"},...}
receipt {"op_id":"alpha-submission-2","queued_seq":"3",...,"delivery":{"state":"pty_written","written_bytes":"19","output_offset":"5"},...}
receipt {"op_id":"beta-submission-3","queued_seq":"4",...,"delivery":{"state":"pty_written","written_bytes":"18","output_offset":"5"},...}
pty_bytes_hex 616c7068612d7375626d697373696f6e2d300d626574612d7375626d697373696f6e2d310d616c7068612d7375626d697373696f6e2d320d626574612d7375626d697373696f6e2d330d
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 25 filtered out
QA_HAPPY_EXIT=0

$ cargo test -p ovis-terminal-core --test input failure -- --exact --nocapture
fenced [[2,{"state":"fenced","reason":"lease_revoked"}],[3,{"state":"fenced","reason":"lease_revoked"}],[5,{"state":"fenced","reason":"credential_revoked"}]] pty_bytes onefour
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 25 filtered out
QA_FAILURE_EXIT=0
```

(`...` elides fields identical to the first line; full lines are in `qa-happy.log`.) `output_cursor` is `"0"` because the test view captures output offset 0 at admission; the screen/stream offset is wired by todos 23/31.

## QA

`crates/ovis-terminal-core/tests/input.rs` (26 tests, modules under `src/input/sequencer/qa/`):

- `happy` (real holder): two principals interleave four text+Enter submissions on an unleased terminal. Asserts each admission returns `queued` in under 100 ms (admission makes no holder call: the sequencer holds no holder handle and the test performs the PTY writes only afterwards); the barrier at seq 2 is `Pending` before any write; after dispatch the program's raw output equals the four submissions concatenated in `queued_seq` order (each `text\r` contiguous); barrier at 4 is `Reached`; every receipt is `pty_written` with `written_bytes == admitted_bytes`.
- `failure` (real holder): operator queues `one`,`two`,`three` under its lease, `one` is written; the bridge takes over at epoch 1 and `fence()` fences 2 and 3 as `lease_revoked`; the bridge queues `four`,`five`, `four` is dispatched, then the bridge credential is revoked and the next write boundary fences 5 as `credential_revoked`. Asserts the program received exactly `onefour`.
- `blocked::a_reader_that_never_reads_blocks_only_its_own_entry` (real holder, raw-mode `sleep 600`): a 1 MiB entry is with the holder and its write is stuck; 8 threads × 16 admissions all return `queued`, each slowest per thread under 100 ms (lock wait included); `Holder::resize` succeeds meanwhile; after `Holder::signal(Kill)` on the spawned program the holder's acknowledgement becomes `partial` with `0 < written_bytes < 1048576`, and `exited()` fences the 128 queued entries as `finished`.
- admission: text+Enter is one 6-byte entry and one holder write, a second `next()` sends nothing while it is unacknowledged; runtime/machine/terminal/incarnation mismatches are `stale_target`, process expectation `process_replaced`, nothing dispatched; `not_ready` → ok after `stream_ready` without any agent readiness → `recovering` → `finished`; `paste_refused`, `unknown_key`, `input_overflow` by bytes and by entries without consuming a position.
- authority: observer `forbidden`; rival with/without the holder's lease id and the holder without presenting it are `no_lease`; raw bytes without the raw grant `raw_not_authorized`, with it the 15 bytes incl. an unknown sequence and `0xff` are written unchanged (EXCL-017).
- fencing: takeover fences unsent entries, emits an empty handshake at a higher fence epoch and the same process incarnation, a 0-byte ack becomes `lease_revoked`, the new owner writes under grant epoch 2; a release while the holder already wrote reports `pty_written`; credential revocation fences only the revoked writer's entries while the other writer's entry is written in order; process replacement fences the queued entry as `stale_target`; an arrow key encoded before `ESC[?1h` is `mode_mismatch`.
- leases (todo 24 follow-ups): the same owner releasing lease 1 and reacquiring lease 2 cannot write through lease 1 (queued entry `lease_revoked`, new admission `no_lease`), lease 2 writes; releasing interact keeps the geometry lease with the operator and every write (no lease, the released interact id, the geometry id) is `no_lease`.
- recovery: holder loss makes the sent entry `outcome_unknown` and the unsent one `fenced/recovering`, three re-adoptions dispatch nothing, a late partial ack resolves the exact 1-byte prefix and a repeated ack is `stale_seq`; interruption (exit) at the queue/write/ack barriers yields `fenced/finished`, `outcome_unknown`, `pty_written` respectively and no re-send; barrier `Pending{5}` → `Pending{2}` → `Reached` → `Pending{3}` → `Undelivered{2}`, `seq_ahead` beyond; a terminal-query reply is dispatched while a user entry is blocked, produces no receipt, and the reply lane overflows at 65 entries.
- stages: acceptance before the PTY write is `invalid_record`, after it `Reported`; turn stages stay `pending`; an undeclared entry's report is `invalid_record`, an unknown op `not_found`.
- model (proptest, 256 cases × up to 60 steps of admit/next/ack 0/partial/full/link loss/ready): user writes leave in strictly increasing admission index and holder write sequence, never twice, and no receipt goes back to `queued`.
- wire: receipt JSON round trip with decimal-string counters and tagged stages; numeric `queued_seq`, `"01"` and an unknown field are rejected; `InputSeq` `u64::MAX` round trips as `"18446744073709551615"`; proptest round trip for every `u64`; fence reasons serialize to their kind token.

Gate: `bash scripts/check.sh` exit 0 on the final tree (`check.log`: 531 tests run, 531 passed, 1 skipped; hygiene 0 violations in 535 files; 47 JSON files checked). Acceptance commands: `cargo nextest run -p ovis-terminal-core` (`core-nextest.log`: 177 tests run, 177 passed, `NEXTEST_EXIT=0`) and `cargo clippy -p ovis-terminal-core --all-targets -- -D warnings` (`core-clippy.log`: `CLIPPY_EXIT=0`).

macOS: `probe-stress-macos` gains an `input` option repeating `cargo nextest run -p ovis-terminal-core --test input` 100 times on macos-latest. Run: https://github.com/devswha/ovis-terminal/actions/runs/41203080631 (dispatched with `gh workflow run probe-stress-macos.yml --ref lane/t25-ordered-input-fencing -f probe=input`; result recorded in the DoneClaim)

## Adversarial

- Stale state, all epochs: process incarnation (`stale_target` at the write, `process_replaced` at admission), lease epoch (takeover, release, same-owner reacquire), credential epoch (revocation), mode revision (`mode_mismatch`), holder fence epoch (handshake).
- Cancel/resume at queue/write/ack barriers: `interruption_at_each_barrier_leaves_one_truthful_outcome`.
- Hung command: the no-read child test keeps admission under 100 ms and resize working while the PTY write is stuck, then reports the exact prefix.
- Repeated interruptions: three link-loss/re-adoption cycles and the model test never send an entry twice.
- Misleading success: admission returns `queued` only; `pty_written` comes only from a holder acknowledgement; agent stages stay `unknown`/`pending` after delivery.
- Mutation probes (`mutations.log`), each killed: write-boundary recheck skipped (4 tests fail incl. `failure`), fence handshake never sent (2), holder loss re-queues the sent entry (3 incl. the model), process replacement keeps old entries (1), partial write reported as written (3). Sources restored after each probe; proptest regression files written by mutants were discarded.

## Contract checklist

1. Every counter in a receipt (`queued_seq`, `admitted_bytes`, `mode_revision`, `output_cursor`, `written_bytes`, `output_offset`) is a canonical decimal string; numeric and non-canonical forms are rejected; `u64::MAX` round trip tested.
2. `SequencerError` and `ReceiptError` are `thiserror` enums with `kind()`/`hint()`; `Lease` and `Encoding` carry `#[source]`; `Display` never repeats a kind token. `FenceReason` is receipt data with a `kind()` token.
3. `InputSeq` is a distinct newtype; holder write sequences use the protocol `WriteSequence`; process identity is `ProcessGeneration`, leases `LeaseId`. `Option` only for real absence (expected process, presented lease, raw grant).
4. Tests assert behavior (bytes the program received, typed kinds, state transitions); proptest for the wire and the model.
5. No `#[allow]`, no `#[expect]`, no placeholder TODO. Holder wire generation stays 2.
6. Conformance attempt recorded verbatim above.

## Seams

New public API only: `ovis_terminal_core::input::sequencer::{Sequencer, Binding, Expected, Writer, Submission, Request, View, Moment, Payload, QueueLimits, Step, BarrierState, SequencerError, holder_refusal (unix)}` and `ovis_terminal_core::input::receipts::{Receipt, Delivery, FenceReason, Stage, AgentStage, Reporter, StageReport, InputSeq, ReceiptError}`. No landed crate's public API changed. `.github/workflows/probe-stress-macos.yml`: one option (`input`) and one job appended at the end.

## Risks

- Not wired to RPC dispatch or the terminal actor yet (todos 28-31); the sequencer is driven by tests and the actor will own it.
- One user entry is with the holder at a time; throughput for many tiny entries costs one holder round trip each.
- A terminal-query reply queued in the holder is dropped by a fence handshake (the program may re-query).
- The partial-write case relies on the PTY reporting hangup to a blocked writer after the program is killed; Linux verified here, macOS by the stress run.
