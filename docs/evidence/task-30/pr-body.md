test(attach-input): prove attach input and aggregate capacity conformance (plan todo 30)

Closes GAP-039, GAP-042, GAP-043, GAP-046, GAP-052, GAP-056, GAP-060, GAP-061, GAP-063, GAP-070, GAP-071, GAP-073, GAP-074, GAP-075, GAP-076, GAP-077, GAP-078, GAP-084, GAP-148, except the product bounds listed under "Product bounds not met".
Delivers IDEAL-028..032, 035, 037..041, 044, 048..055, 061, 063. Does not reproduce EXCL-007/017/019/024/025/026 or V-RT-HC-01/02/03.

## Before

RT-HC-02, `docs/evidence/baseline/smoke/hc-s06-wait-output.log:11` (the durable copy of `/tmp/ulw-ovis-yCJzPH/evidence/smoke/hc-s06-wait-output.log:11`; the block names `docs/evidence/baseline/hc-s06-wait-output.log`, and the file lives under `smoke/`). After a send, the marker wait timed out:

```
{"children":[],"ended":false,"exit_code":null,"reason":"timeout","signal":null,"status":"idle","terminal":"t1","turn":1}
```

## After

Host: Linux 7.0.0-38-generic x86_64 (`{"arch":"x86_64","family":"unix","os":"linux"}` in the evidence). `target/debug/ovis-terminal` is this lane's RAM target `/dev/shm/ovt-target-lane-t30/debug/ovis-terminal`, because host rules forbid writing `<worktree>/target`. Each case writes its own evidence file, so the failure run does not overwrite the happy file. Logs: `docs/evidence/task-30/qa-happy.log` and `qa-failure.log`.

```
$ cargo run -q --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary /dev/shm/ovt-target-lane-t30/debug/ovis-terminal --scenario part-a/task-30 --isolated --case happy --evidence docs/evidence/task-30/task-30-happy.json
{"case":"happy",...,"result":"passed","scenario":"part-a/task-30","totals":{"assertions":"175","cases":"1","failed":"0","passed":"1","skipped":"0"}}
EXIT=0
$ ... --case failure --evidence docs/evidence/task-30/task-30-failure.json
{"case":"failure",...,"result":"passed","scenario":"part-a/task-30","totals":{"assertions":"252","cases":"1","failed":"0","passed":"1","skipped":"0"}}
EXIT=0
```

The happy count fell from 412 (round 3) to 175 because the 16-PTY profile moved to the declared `capacity/sixteen` (see "Product bounds not met").

Socket transcript lines, verbatim from the evidence. The full excerpt (71 lines, both cases) is in `docs/evidence/task-30/after-transcript.md`.

```
[control received 51ms] {"jsonrpc":"2.0","id":"mark-wait","result":{"cursor":{"offset":"29"},"matched_at":"22","outcome":"matched",...}}
[control received 52ms] {"jsonrpc":"2.0","id":"mark-barrier","result":{"output_offset":"33","pending_bytes":"0","reached":true}}
[control received 1743ms] {"jsonrpc":"2.0","id":"fx-screen","result":{"cursor":{"col":0,"row":3,"visible":true},"lines":["utf8:€ csi:RED","REPLY-ONCE","NEXT[x]",...],...,"output_offset":"52",...}}
[observer received 1686ms] {"jsonrpc":"2.0","id":"late","result":{...,"first_paint":{...,"inexact":[],...}}}
[control received 4756ms] {"jsonrpc":"2.0","id":"fill6","error":{...,"data":{"kind":"input_overflow",...}}}
[control received 4757ms] {"jsonrpc":"2.0","id":"after-cap","result":{"output_offset":"7","pending_bytes":"15859713","reached":false}}
[control received 4845ms] {"jsonrpc":"2.0","id":"delivered","result":{"data_b64":"b25lLWENCnR3by1hDQpvbmUtYg0KdHdvLWINCg==",...}}   (= "one-a\r\ntwo-a\r\none-b\r\ntwo-b\r\n")
[observer received 4900ms] {"jsonrpc":"2.0","id":"mid","result":{...,"first_paint":{...,"inexact":["control_inside_open_sequence"],...}}}
```

### What `part-a/task-30 --case happy` (= `attach-input/all` happy) asserts

The run uses the real `ovis-terminal serve` with a 64 KiB QA output ring.
- **Advertised budgets in hello:** output ring 64 MiB, credit 256 KiB initial and 1 MiB max, input queue 16 MiB, session retained 4 GiB, session input and backlog 64 MiB each, VT 128 MiB, plus a stream backlog.
- **First-paint parity:** the prompt is matched, and the raw bytes at its `matched_at` read back as `$ `. An interact writer and an observe-only viewer receive the *identical* `first_paint` (the whole Checkpoint object) at the prompt's end.
- **Admission before delivery:** the `input.send` receipt has `admitted_bytes` "19", `queued_seq` "1", and an `output_cursor` at the prompt's end, which is before the echo. The marker `mark-42` matches exactly 20 bytes (the echoed line) after that cursor, and the raw bytes at `matched_at` read back as `mark-42`. `input.barrier` on seq 1 is then `reached` with 0 pending bytes.
- **Operator lease:**
  - The operator is `held` behind the writer, with retry boundary `{"on":"lease_released","lease":{"holder":"writer"}}`.
  - It takes over by the epoch it observed.
  - The writer's stream reports, in order: checkpoint, input_ready, input_revoked, then close `taken_over` by `operator`.
- **Bounded shared storage and overflow:**
  - 300 000 bytes pass the 64 KiB ring while the observer never credits. The operator's marker still matches (credit isolation).
  - The observer's repaint answers at its own `stream_offset`, which is ≤ the 262 144-byte initial credit. The `gap` record and the following checkpoint repeat that offset, and the checkpoint equals `terminal.screen`'s offset and revision.
  - `terminal.output` from 0 resumes at floor = head − 65 536 with `gap: true`. A read from the floor ends at the head.
  - `server.metrics` reports exactly one resynced byte loss for the observer (the gap's skipped range through head−1) and none for the operator.
- **Checkpoint/tail reconstruction (B2):** a second program writes a UTF-8 `€` and a CSI SGR split across separate writes. It then sends a DSR query (`ESC[6n`), which the server answers.
  - The program saw exactly one reply (`REPLY-ONCE`).
  - A late observer's first paint and its repaint are exact (`inexact: []`).
  - The next byte the program read is the operator's `x`, not a second reply.
  - The Rust test feeds the raw output from offset 0, in one piece, to a fresh `ovis-terminal-vt` and requires its 24 lines and cursor to equal `terminal.screen`.
- Offsets are relative to the run's own output. On macOS, bash-as-`/bin/sh` first prints 8 invisible bytes (`ESC[?1034h`, round 2).

### What `part-a/task-30 --case failure` (= `attach-input/adversarial` failure) asserts

- **Observer safety:** an observer naming the writer's lease is `forbidden`.
- **Credit:** an acknowledgement of u64::MAX is `invalid_credit`, a numeric acknowledgement is `invalid_params`, and an unknown stream is `stream_not_found`.
- **Fake status/prompt text** leaves `status_at_send.reported` false.
- **Takeover and leases:** a stale-epoch takeover is `lease_epoch_mismatch`. The revoked writer's queued write is `no_lease`. An unenabled bracketed paste is `paste_refused`.
- **Overflow:**
  - A wait armed before a 300 000-byte overflow matches the marker printed after it.
  - The same cursor afterwards is `cursor_expired`.
  - `terminal.output` from it resumes at the floor with `gap: true`.
  - A start past the head, and a numeric offset at `params.from_offset`, are each `invalid_params`.
- **Stalled child:**
  - Two 64 KiB writes go in at seq 1-2, and the barrier is not reached.
  - Escape is admitted at seq 3. The exit wait times out and the same pid is unfinished.
  - Five 3 MiB writes go in at seq 4-8. The sixth is `input_overflow`, and the 15 859 713 pending bytes stay ≤ hello's `input_queue_bytes` 16 777 216 (per-object cap, typed rejection before the budget).
- **Launch cap:** a 70 000-byte argv element is `too_large` with `bound` `argv_element_bytes`/32768, and nothing spawns.
- **Atomic input vs another writer (B4):** owner and writer connections hold no lease and alternate four submissions into a child that does not read (`stty -echo -icanon noflsh`). They are admitted at seq 1-4, and nothing is delivered (`held-back` read is empty). `ctrl+c` at seq 5 releases them, and the program reads exactly `one-a\r\ntwo-a\r\none-b\r\ntwo-b\r\n`: every submission whole and in admission order.
- **`checkpoint_inexact` (B2):** a program stops right after a CR inside an open CSI. A late observer's first paint reports `inexact: ["control_inside_open_sequence"]`, and once the sequence completes the repaint is exact again.

## Product bounds not met

Per the orchestrator's policy, these scenarios are written exactly against the contract, with no loosened bound, and are listed in `conformance/scenarios/declared.json` with the missing capability and measured numbers. Each becomes an acceptance test when its declaration is flipped. Evidence of each failing run is committed (`docs/evidence/task-30/declared-*.json`, `measure-*.json`).

| Scenario | Contract | Measured (Linux x86_64, debug) |
|---|---|---|
| `capacity/sixteen` | Every 16-PTY admission and committed-event sample within 100 ms (ideal-state.md:259, IDEAL-027/051) | Admissions 3-19 ms (pass). **Echo deliveries t1-t16: 22, 81, 111, 193, 220, 291, 304, 388, 444, 499, 559, 627, 691, 709, 811, 840 ms. t16 is 840 ms > 100 ms**: output ingestion is serialized across terminals (~50 ms each; the arrival order does not follow the request order). Exit events 15-64 ms. The declared run fails at step 38: `correlated replies did not happen within 100 ms`. |
| `attach/geometry` | `geometry.changed` → `geometry.checkpoint` → `geometry.input_ready` in order, before output at the new grid (IDEAL-037/038) | `attach.resize` answers `applied` (revision advances) and the program sees SIGWINCH and prints `30 100`. **No geometry event ever reaches an `events.subscribe` subscriber** (they are committed only to the daemon's geometry log), and the attach stream gets no new checkpoint or input_ready record. Fails at step 19 (20 000 ms bound). |
| `capacity/session-budget` | Session-wide input refused with typed `backpressure` before hello's `session_input_bytes` 67 108 864 is exceeded (IDEAL-052) | **25 writes of 3 MiB (75 MiB, 11 MiB past the budget) to five stalled terminals were all admitted.** There was no backpressure or limit_exceeded; only the per-terminal 16 MiB queue refuses. The declared run fails at step 66 (`w22` admitted). |

Also not enforced, so not stressable: `session_retained_bytes` (4 GiB), `session_vt_bytes` (128 MiB) and `session_rss_bytes` are advertised, but no daemon code reads them (grep of `crates/`). The session backlog budget (64 MiB, `SessionBacklog`) resynchronizes over-budget viewers with gap + checkpoint instead of rejecting.

## Gate r3 rulings → round 4

- **B1 (100 ms event samples).** The retained-byte `echoN` re-reads are gone. Admissions, committed echo deliveries and exit events are each bounded `within_ms: 100`, now in `capacity/sixteen`. The product fails the delivery bound, so the scenario is declared with the numbers above.
  - The exit outcome is asserted present rather than pinned: a one-line program can exit before the wait arms (`already_satisfied`) or after (`exited`), and `events.wait` serves no cursor.
- **B2 (`checkpoint_inexact`, VT reconstruction).** This is a contract change; see the next section. The split UTF-8/CSI and replayed-DSR fixtures, the single-reply check and the independent VT comparison are in the happy case, and the inexact paint is in the failure case.
- **B3 (session-wide budgets).** `capacity/session-budget` expects typed `backpressure` before the budget is exceeded. It is declared because the daemon admits past it. The per-object input queue cap is asserted in the failure case.
- **B4 (geometry ordering; atomic input vs a second writer).** The two-writer drain is asserted in the failure case. Geometry ordering is `attach/geometry`, declared because no events are delivered.
  - Not exercised: IDEAL-050's "partial-write failure reported explicitly". It needs a holder write-fault injection, which neither the runner nor a QA seam has.
- **B5 (repaint race).** The repaint offset comes from the repaint reply, bounded by hello's credit (round-4 prepared fix).
- **N1:** plan amendment, below.
- **N3:** `assembly::live::tests::production_screens_keep_the_default_eight_mib_ring` pins `screen_bounds(None) == ScreenBounds::DEFAULT` (8 MiB). `a_qa_ring_scales_checkpoints_and_slices_inside_it` pins the QA scaling.
- **N4:** `task-30-happy.json` and `task-30-failure.json` were re-recorded on this tree.

### Contract change: `Checkpoint.inexact` (`checkpoint_inexact`)

The VT engine (`ovis-terminal-vt`) already computes checkpoint `Fidelity` and `InexactReason` at the parser boundary, and core's paint carries it. Nothing published it. The smallest change publishes it:
- `crates/ovis-terminal-protocol/src/schema/records/lease.rs` adds `Checkpoint.inexact: Vec<OpenEnum<CheckpointInexact>>`, an open enum mirroring the four VT reasons. It has `#[serde(default)]`, so it is not in `required`: the daemon always sends it, older servers' checkpoints still validate, and the released-baseline compatibility test passes.
- `records/attach.rs` adds `PaintedScreen.inexact`, and `records.rs` re-exports the enum.
- `crates/ovis-terminal-server/src/streams/records.rs` maps the paint's fidelity into `inexact` for every first paint, repaint and resync checkpoint.
- `schema/ovis-terminal.openrpc.json` is regenerated (`ovis-terminal schema openrpc`). `cargo nextest run -p ovis-terminal-protocol --test schema`: 6/6.
- `quality-matrix --mode contract` exits 0 and updated `conformance/ideal-map.json`.
- The SDK was regenerated with `bun run generate` (`src/generated`, never hand-edited). `generate:check` is fresh, `typecheck` passes, and `bun test` passes 47/47 with `OVIS_CONFORMANCE_BIN`/`OVIS_TERMINAL_BIN` set to the RAM target. A stale ignored `dist/`/`pack/` from an earlier build was removed first.

### Tests and gates (literal commands, exit codes)

- `cargo nextest run -p ovis-terminal-conformance`: exit 0, 34 tests passed (including the `$repeat:` labels test), among them `attach_input::happy`, `attach_input::failure` and the suite inventory with the three declarations (`nextest.log`). The literal `-p conformance` exits 101 (`nextest-literal-p-conformance.log`); see the amendment.
- `cargo clippy -p ovis-terminal-conformance -p ovis-terminal-cli -p ovis-terminal-protocol -p ovis-terminal-server --all-targets -- -D warnings`: exit 0 (`clippy.log`).
- `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh`: exit 0, `check.sh: all steps passed` (workspace fmt/clippy/nextest/doc tests, hygiene, source-check, evidence-audit with 0 findings) (`check.log`).
- Windows: `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` and `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings`: both exit 0 (WINCHECK_EXIT=0, WINCLIPPY_EXIT=0) (`windows.log`).
- **Mutants (`mutants.log`).** Each was applied, the binary rebuilt, the shipped `attach_input::*` tests run, and the mutant reverted. All seven were killed:
  - M1: credit beyond sent accepted → `failure` fails.
  - M2: takeover epoch check removed → `failure` fails.
  - M3: pending barrier reported reached → `failure` fails.
  - M4: no floor resume → both cases fail.
  - M5: inexact reasons dropped → `failure` fails at `reply[mid]/result/first_paint/inexact`.
  - M6: queue cap raised to 64 MiB → `failure` fails at `fill6` (admitted at seq 9).
  - M7: a query reply admitted twice → `happy` fails at `next-seen` (the program read the duplicate reply instead of `x`).

### QA seam (disclosed, T31 precedent)

`serve --qa-output-ring-bytes <N>` (N ≥ 4096) sets the raw output each terminal retains; the default is 8 MiB. The checkpoint interval (N/2) and parse slice (min(N/4, 64 KiB)) shrink with it. It is documented in `serve --help`, and it is refused (`accept the QA seams`) unless `--root` holds a `qa-fixture` file (`pending::a_qa_seam_is_refused_on_a_root_that_is_not_a_qa_fixture_root`). The runner's `serve` step takes `"qa_output_ring_bytes"`. Integration's T83 added the sibling `--qa-graphics-sequence-max-bytes`, and both were union-merged.

### Earlier rounds

- Round 2: macOS `/bin/sh` prints 8 invisible bytes before its prompt, so offsets became relative to the run's prompt, with byte checks at each match.
- Round 3: a missing output fence on the previous command, so the failure case now fences each command's output before capturing a cursor.

### Plan-literal amendment

The block's acceptance commands name `-p conformance`, but the package is `ovis-terminal-conformance` (`conformance/Cargo.toml`). The literal command exits 101. The commands run and recorded are `cargo nextest run -p ovis-terminal-conformance` and `cargo clippy -p ovis-terminal-conformance --all-targets -- -D warnings`. The QA invocations are literal except that `--binary` points at the lane's RAM target.

## Reported findings (not declared scenarios)

1. **Debug output throughput is about 0.3 MB/s.** 4 MB took about 13 s, and 70 MB did not finish within 240 s. That is why the QA ring seam exists.
2. **hello advertises `output_ring_bytes` 64 MiB, but each screen retains 8 MiB** (`ScreenBounds::DEFAULT`, now pinned by N3).
3. **Fixed in this lane:** `terminal.output` from an evicted offset answered the unpublished `history_gap`. It now resumes at the floor with `gap: true` (M4).
4. **The output-wait `gap` outcome is unreachable from an armed wait.** The wait scans every ingested byte. A cursor already behind retention is `cursor_expired`.
5. **Stopping 50 live `/bin/cat` terminals takes about 34 s**, which exceeds the runner's 30 s stop bound. `capacity/fifty` therefore uses one-line programs.
6. **Unrelated flake (F2):** `remote/isolation` `child-on-bravo` `internal`, seen locally and in CI run 41349272942 (`flake-remote-isolation.log`). F2a fixed it on integration (60076f7, merged here).

## Seams (files outside the write scope, and why)

- `conformance/scenarios/attach-input/{all,adversarial}.json`: the scenario id grammar is `<family>/<name>`, so `attach-input/all` and `attach-input/adversarial` must be these files. `attach/geometry.json`, `capacity/{fifty,sixteen,session-budget}.json` and `part-a/task-30.json` are in scope.
- `conformance/scenarios/declared.json`: three entries for the product-limited scenarios.
- `conformance/src/runner_tests.rs`, `runner_tests/attach_input.rs` and `runner_tests/attach_input/oracle.rs` (new): the named tests and the VT oracle. `runner_tests/suite.rs`: the lane's own `ATTACH_INPUT` registration.
- `conformance/src/scenario/serve.rs`, `engine/steps.rs`, `engine/steps/serve.rs`: `qa_output_ring_bytes` on the `serve` step.
- `conformance/src/scenario/labels.rs`: the `$repeat:<count>:<text>` placeholder. It expands to `text` repeated `count` times, bounded at 128 MiB; a malformed or oversized spec stays literal. It has the unit test `repeat_placeholders_expand_and_malformed_ones_stay_as_written`.
- `crates/ovis-terminal-cli/src/invocation/{args,plan}.rs`, `src/assembly.rs`, `src/assembly/{actor,serve,live}.rs`: the QA ring seam; `live.rs` also holds the N3 unit tests.
- `crates/ovis-terminal-cli/src/assembly/reads.rs`: the floor resume (finding 3). `crates/ovis-terminal-cli/tests/assembly/pending.rs`: seam refusal.
- `crates/ovis-terminal-protocol/src/schema/records{.rs,/lease.rs,/attach.rs}`, `crates/ovis-terminal-server/src/streams/records.rs`, `schema/ovis-terminal.openrpc.json`, `packages/client-ts/src/generated/`, `conformance/ideal-map.json`: the `checkpoint_inexact` contract change.
- `probes/platform-qa/stress/attach-input.{json,sh}`: this lane's macOS stress lane, which keeps evidence as `*.evidence.log`.
- `docs/evidence/task-30/`.
- Merges of `origin/integration`: 1abcced (T70) and 60076f7 (F2a), authorized.

## Landing

Land with `git merge --squash` onto integration; the orchestrator deletes the lane branch at teardown.

The lane branch history (commits bee84e2..dc685a1) carries two oversized scenario blobs that this head no longer has: `conformance/scenarios/attach-input/adversarial.json` (19 MB) and `capacity/session-budget.json` (69 MB). They held multi-megabyte input payloads as literal text. History is not rewritten (no force-push). The squash keeps the blobs out of integration's history.

The head stores payloads as `$repeat:<count>:<text>` placeholders: the two files are now 64 KB and 32 KB, and no scenario file exceeds 200 KB.

## Risks

- The VT oracle uses the same `ovis-terminal-vt` emulator as the daemon. It is independent of the daemon's chunked ingestion and checkpoint path (it is fed the whole stream at once), not of the emulator itself.
- The three declared scenarios are acceptance tests for the product-fix lanes. They assert the exact contract and will fail until those lanes land.
- `Checkpoint.inexact` is additive, so older clients ignore it. The schema change regenerates 31 SDK files (mostly the embedded document).

DEFERRED VERIFICATION: none for `part-a/task-30`: happy and failure both execute on the assembled binary with exit 0. The declared scenarios run and fail on product capabilities, as recorded above.

macOS stress dispatch:
- Round 1: https://github.com/devswha/ovis-terminal/actions/runs/41347211460 (fb2e4cd) failed on the Darwin prompt offset.
- Round 2: https://github.com/devswha/ovis-terminal/actions/runs/41349240120 (b6fecf8) failed with no evidence kept.
- Round 3: https://github.com/devswha/ovis-terminal/actions/runs/41352274612 (96ffc78) failed on the command fence.
- Round 3b: https://github.com/devswha/ovis-terminal/actions/runs/41354657097 (edecd73) passed 13 iterations, then failed at happy iteration 14 on the repaint race.
- Round 4: MACOS_RUN5.
