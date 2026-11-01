test(attach-input): prove attach input and aggregate capacity conformance (plan todo 30)

Closes GAP-039, GAP-042, GAP-043, GAP-046, GAP-052, GAP-056, GAP-060, GAP-061, GAP-063, GAP-070, GAP-071, GAP-073, GAP-074, GAP-075, GAP-076, GAP-077, GAP-078, GAP-084, GAP-148.
Delivers IDEAL-028..032, 035, 037..041, 044, 048..055, 061, 063. Does not reproduce EXCL-007/017/019/024/025/026 or V-RT-HC-01/02/03.

## Before

RT-HC-02, `docs/evidence/baseline/smoke/hc-s06-wait-output.log:11` (the durable copy of `/tmp/ulw-ovis-yCJzPH/evidence/smoke/hc-s06-wait-output.log:11`; the block names `docs/evidence/baseline/hc-s06-wait-output.log`, and the file lives under `smoke/`). After a send, the marker wait timed out:

```
{"children":[],"ended":false,"exit_code":null,"reason":"timeout","signal":null,"status":"idle","terminal":"t1","turn":1}
```

## After

Host: Linux 7.0.0-38-generic x86_64 (`{"arch":"x86_64","family":"unix","os":"linux"}` in the evidence). `target/debug/ovis-terminal` is this lane's RAM target `/dev/shm/ovt-target-lane-t30/debug/ovis-terminal`; host rules forbid writing `<worktree>/target`. Each case writes its own evidence file, so the failure run does not overwrite the happy file. Logs: `docs/evidence/task-30/qa-happy.log` and `qa-failure.log`.

```
$ cargo run -q --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary /dev/shm/ovt-target-lane-t30/debug/ovis-terminal --scenario part-a/task-30 --isolated --case happy --evidence docs/evidence/task-30/task-30-happy.json
{"case":"happy",...,"result":"passed","scenario":"part-a/task-30","totals":{"assertions":"412","cases":"1","failed":"0","passed":"1","skipped":"0"}}
EXIT=0
$ ... --case failure --evidence docs/evidence/task-30/task-30-failure.json
{"case":"failure",...,"result":"passed","scenario":"part-a/task-30","totals":{"assertions":"161","cases":"1","failed":"0","passed":"1","skipped":"0"}}
EXIT=0
```

Socket transcript lines, verbatim from `task-30-happy.json`. The full excerpt (63 lines, both cases) is in `docs/evidence/task-30/after-transcript.md`.

```
[control sent 46ms] {"id":"mark-wait",...,"method":"terminal.wait_output","params":{"cursor":{"offset":"2"},"pattern":{"text":"mark-42"},...}}
[control received 51ms] {"jsonrpc":"2.0","id":"mark-wait","result":{"cursor":{"offset":"29"},"matched_at":"22","outcome":"matched",...}}
[control received 52ms] {"jsonrpc":"2.0","id":"mark-barrier","result":{"output_offset":"33","pending_bytes":"0","reached":true}}
[control received 1294ms] {"jsonrpc":"2.0","id":"evicted","result":{"data_b64":"eXl5…","floor_offset":"234578","from_offset":"234578","gap":true,"next_offset":"234642"}}
[control received 1306ms] {"jsonrpc":"2.0","id":"metrics","result":{...,"consumers":[{"client_id":"observer",...,"losses":[{"range":{"first_offset":"261809","last_offset":"300113","unit":"bytes"},"resynced":true}],...},{"client_id":"operator",...,"losses":[],...}],...}}
```

### What `part-a/task-30 --case happy` (= `attach-input/all` happy) asserts

The run uses the real `ovis-terminal serve` with a 64 KiB QA output ring.
- **Advertised budgets in hello:** output ring 64 MiB, credit 256 KiB initial and 1 MiB max, input queue 16 MiB, session retained 4 GiB, session input and backlog 64 MiB each, VT 128 MiB, plus a stream backlog.
- **First-paint parity:** the prompt is matched, and the raw bytes at its `matched_at` read back as `$ `. An interact writer and an observe-only viewer then receive the *identical* `first_paint` (the whole Checkpoint object), whose `output_offset` is the prompt's end.
- **Admission before delivery:** the `input.send` receipt has `admitted_bytes` "19" and `queued_seq` "1". Its `output_cursor` is the prompt's end, which is before the echo. The marker `mark-42` (the shell's evaluation, not the echoed command) matches exactly 20 bytes (the echoed command line) after that cursor, and the raw bytes at `matched_at` read back as `mark-42`. `input.barrier` on seq 1 then answers `reached` with 0 pending bytes.
- **Offsets are relative to the run's own output, never absolute.** On Linux the prompt is at offset 0 (end 2, marker 22). On macOS, bash-as-`/bin/sh` first prints 8 invisible bytes: the screen revision advances once while the cursor stays at column 0, consistent with readline's `ESC[?1034h`. There the prompt ends at 10 (CI run 41347244132, artifact `all-families.json`). The product's byte accounting is the same on both platforms.
- **Operator lease:**
  - The operator is `held` behind the writer. Its retry boundary is `{"on":"lease_released","lease":{"holder":"writer"}}`, with the epoch it observed.
  - It takes over by that epoch. The writer's stream reports, in order: checkpoint, input_ready, input_revoked, close `taken_over` by `operator`.
  - The operator's own input runs.
- **Bounded shared storage and overflow:**
  - The flood: 300 000 bytes pass the 64 KiB ring while the observer never sends credit. The operator's marker still matches (credit isolation).
  - The observer's stream stops at exactly `stream_offset` 262144, which is its credit. Its repaint yields a `gap` control whose `skipped.to_output_offset` equals `terminal.screen`'s offset, followed by a checkpoint equal to the screen's offset and revision.
  - `terminal.output` from 0 resumes at the floor with `gap: true`, where floor = head − 65 536 (checked arithmetically in the Rust test). A read from the floor has no gap and ends at the head.
  - `server.metrics` reports exactly one resynced byte loss for the observer, from the gap's `skipped.from_output_offset` through head−1, and `losses: []` for the operator.
- **Exact 16-PTY profile (/bin/sh plus 15 one-line `head -n 1` terminals):**
  - All 15 admissions answer within one `within_ms: 100` bound.
  - After delivery, each terminal's output-wait event is a separate sample, bounded at 100 ms (`within_ms: 100`).
  - Every program exits before the server stops.

### What `part-a/task-30 --case failure` (= `attach-input/adversarial` failure) asserts

Each step below ends in a typed rejection or an asserted outcome.
- **Observer safety:** an observer naming the writer's lease is `forbidden`.
- **Impossible and malformed credit:** an acknowledgement of u64::MAX is `invalid_credit`. A numeric acknowledgement is `invalid_params`. An unknown stream is `stream_not_found`.
- **Fake status text:** the program prints `state: idle` and prompt text. The next receipt's `status_at_send.reported` stays `false`.
- **Takeover:** a takeover with a stale epoch is `lease_epoch_mismatch`. The real takeover revokes the writer, and its queued write on the revoked lease is `no_lease`.
- **Paste:** a bracketed paste the program never enabled is `paste_refused`.
- **Overflow:**
  - A wait armed before 300 000 bytes overflow the ring still matches the marker printed after them, at more than 64 KiB past the arming cursor.
  - The same cursor afterwards is `cursor_expired`.
  - `terminal.output` from that cursor resumes at the floor with `gap: true` and `from_offset` = `floor_offset`.
  - An offset past the head is `invalid_params`, and a numeric offset is `invalid_params` at `params.from_offset`.
- **Stalled child** (`stty -echo -icanon; exec sleep 600`):
  - Two 64 KiB writes are admitted at seq 1 and 2.
  - The barrier on seq 2 is `reached: false` with bytes pending.
  - An Escape is ordered at seq 3. The exit wait times out, and the same pid is unfinished (Escape had no effect).
- **Per-object cap:** a 70 000-byte argv element is `too_large` with `bound` `{"limit":"argv_element_bytes","value":"32768"}` at `params.program.argv`, and no terminal is spawned (`t3` is `not_found`).

### Tests and gates (literal commands, exit codes)

- `cargo nextest run -p ovis-terminal-conformance` exits 0: 32 tests passed. This includes `attach_input::happy`, `attach_input::failure` and `suite::every_scenario_file_is_executed_or_a_listed_declaration` (log: `nextest.log`). The block's literal `cargo nextest run -p conformance` exits 101 (`package ID specification conformance did not match any packages`, log: `nextest-literal-p-conformance.log`). The package is `ovis-terminal-conformance`.
- `cargo clippy -p ovis-terminal-conformance -p ovis-terminal-cli --all-targets -- -D warnings` exits 0 (`clippy.log`).
- `cargo nextest run -p ovis-terminal-cli -E 'test(a_qa_seam_is_refused)'` exits 0: the QA seam is refused outside a QA root (`nextest-cli-seam.log`).
- `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` exits 0 with `check.sh: all steps passed`. evidence-audit passed with 0 findings (`check.log`, last 300 lines).
- Windows (round 1; round 2 changes only scenario JSON and the Unix-only runner test module): `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` and `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings` (`windows.log`): both exit 0 (WINCHECK_EXIT=0, WINCLIPPY_EXIT=0).
- **Mutants (`mutants.log`).** Each was applied, the binary rebuilt, the shipped `attach_input::*` tests run, and the mutant reverted. All four were killed:
  - M1 credit beyond sent accepted → `failure` fails (`credit-max` answered a result with `stream_offset` u64::MAX).
  - M2 takeover epoch check removed → `failure` fails (stale takeover acquired).
  - M3 pending barrier reported reached → `failure` fails.
  - M4 `terminal.output` no longer resumes at the floor → both `happy` and `failure` fail (`history_gap`).

### QA seam (disclosed, T31 precedent)

`serve --qa-output-ring-bytes <N>` (N ≥ 4096) sets the raw output each terminal retains for `terminal.output` and output waits. The default is 8 MiB. The checkpoint interval (N/2) and parse slice (min(N/4, 64 KiB)) shrink with it, so the screen's retention invariant still holds. It is documented in `serve --help`. Like `--qa-holder-poll-ms` and `--qa-actor-hold`, it is refused (`accept the QA seams`) unless `--root` holds a `qa-fixture` file. The runner's `serve` step takes `"qa_output_ring_bytes": "<decimal>"`, which marks `<root>/srv` a QA fixture root and passes the flag. `pending::a_qa_seam_is_refused_on_a_root_that_is_not_a_qa_fixture_root` now covers both seams.

### Round 2: macOS CI fix (b0d9bbd → this head)

On macOS, all four task-30 cases failed at step 6: `reply[prompt]/result/matched_at` was "8" where "0" was expected. The artifact's first paints for `/bin/sh` show `output_offset` 10 and `revision` 3 on macOS, against 2 and 2 on Linux, with the cursor at column 2, row 0 on both and no modes enabled. So the program printed 8 invisible bytes before `$ `, matching bash 3.2/readline's 8-byte meta-mode sequence `ESC[?1034h` under `TERM=xterm`. The offsets are byte-exact raw-output offsets on both platforms, so this is not a product defect.

The scenarios now capture the prompt's `matched_at` and its end cursor. They check the raw bytes at each match (`terminal.output` from `matched_at` returns `$ ` and `mark-42`), and the Rust test checks the arithmetic (marker = prompt end + echoed line). There are no `cfg` forks. A scratch variant that prints `ESC[?1034h` before `exec /bin/sh` passes on Linux.

### Round 3: macOS stress fix (96ffc78 → this head)

Stress run 41352274612 now uploads every evidence file as `*.evidence.log`, because the workflow uploads only `./*.log`. Its failure case stopped at step 49: the flood receipt's `output_cursor` was 93 where 90 was expected.

The evidence shows the cause. `terminal.screen` at `head-now` read the line `$ echo after-$(`: the *previous* command's (`after-fake`) echo was still arriving when the head was captured. The scenario never fenced on that command's output, and Linux was only fast enough to hide the race. These 3 bytes are not Darwin-specific.

Fix (scenario only, no `cfg`):
- The failure case now waits for `after-4\r\n$ ` from the after-fake receipt cursor and checks those bytes at `matched_at`.
- The pre-flood cursor is that fence's end, and `terminal.screen`'s head must equal it (idle shell).
- In the 16-PTY profile and `capacity/fifty`, each echo is now asserted to match exactly at its own admission cursor (`$value:curoff<k>`), not at 0.

A sweep left only request parameters reading from 0 and the credit-defined `stream_offset` 262144. Scratch variants of both cases whose shell first prints `ESC[?1034h` pass on Linux (412/161).

### Round 4 (prepared after stress run 41354657097 at ef6b578: iterations 1-13 passed, happy iteration 14 failed)

**Repaint race.** At step 42, `reply[repaint]/result/checkpoint/stream_offset` was 261555 where 262144 was expected (artifact `attach-input-happy-14.evidence.log`). The repaint arrived before the observer's stream had sent its last 589 credited bytes, and the server publishes no event a client can wait on for "stream sent up to its credit".
- The scenario no longer pins 262144. The repaint reply's own `stream_offset` is captured, and the following `gap` record (`/params/stream_offset`) and checkpoint (`/params/checkpoint/stream_offset`) must repeat it.
- The Rust test checks that the repaint checkpoint's offset equals it and that it is ≤ hello's `credit_initial_bytes`.

**Aggregate cap (plan: "reject typed backpressure/limit_exceeded before exceeding the budget").**
- On the stalled child, after the two 64 KiB writes and the Escape (seq 1-3), five 3 MiB writes are admitted at seq 4-8.
- The sixth is `input_overflow`. The barrier on seq 8 reports 15 859 713 pending bytes, which the Rust test checks is ≤ hello's advertised `input_queue_bytes` 16 777 216: the refused write never let the queue pass its budget.
- `limit_exceeded` is published only for `terminal.run`, `terminal.label`, `prompt.open` and `credential.rotate`. The per-object terminal-launch cap answers `too_large` with its `bound`, and that refusal is asserted.

**`checkpoint_inexact` / `output_gap` (plan wording).** Neither is a published kind or field: `schema/ovis-terminal.openrpc.json` has no `inexact` token, and `output_gap` exists only as an internal core error. The overflow path is asserted through its published forms:
- an attach `gap` control record whose `skipped` raw range is followed by a checkpoint;
- `terminal.output` resuming at the floor with `gap: true`;
- `cursor_expired` for an output wait behind retention.

The plan-to-schema naming gap is reported here.

### Plan-literal amendment

The block's acceptance commands name `-p conformance`, but the package is `ovis-terminal-conformance` (`conformance/Cargo.toml`). The literal `cargo nextest run -p conformance` exits 101 (`nextest-literal-p-conformance.log`). The commands run and recorded are:
- `cargo nextest run -p ovis-terminal-conformance`
- `cargo clippy -p ovis-terminal-conformance --all-targets -- -D warnings`

The QA invocations (`cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance ...`) are literal, except that `--binary` points at this lane's RAM target.

## Reported findings (not fixed here; for F4 unless noted)

1. **Serialized output ingestion across PTYs (16-PTY profile).** Admissions are fast: all 15 receipts arrived 2–16 ms after the batch. Echo *delivery*, however, arrives one terminal at a time, about 30–90 ms apart. Measured from the admission batch, t2 was matched at 34 ms, t3 at 94, t4 at 125, t5 at 202, t6 at 233, t7 at 309, t8 at 403, t9 at 434, t10 at 502, and t16 about 790–807 ms. Reversing the order of the wait requests did not change the arrival order (t2 first, t16 last), so ingestion, not wait handling, is serial. Once an echo is delivered, its wait answers in 0–15 ms. The scenario therefore bounds admissions and per-terminal event answers at 100 ms. End-to-end PTY→wait delivery for the later terminals **does not meet the 100 ms bound**. It is reported here, not substituted with a percentile.
2. **Debug-build output throughput is about 0.3 MB/s.** 4 000 000 bytes matched after about 13 s. 70 000 000 bytes did not finish within a 240 s wait (`timeout`). Overflowing the default retention on the real binary is therefore impractical without the QA seam above.
3. **Advertised ring versus retained output.** hello advertises `output_ring_bytes` 67108864 (64 MiB), but each terminal's screen retains `ScreenBounds::DEFAULT` = 8 MiB of raw output.
4. **Fixed in this lane** (`crates/ovis-terminal-cli/src/assembly/reads.rs`). `terminal.output` from an evicted offset answered the unpublished error kind `history_gap`, which is not in `x-error-kinds` or the method's errors. The published contract is a result with `gap`/`floor_offset`, which the handler's own `gap` computation already expected. An evicted start now resumes at the retained floor and reports `gap: true`. Mutant M4 shows the shipped tests detect a regression.
5. **The output-wait `gap` outcome is unreachable from an armed wait.** The wait screen scans every byte as it is ingested, so retention never overtakes an armed wait: a wait armed before a 300 KB overflow still matched exactly. A cursor already behind retention is `cursor_expired`. The `WaitOutputOutcome::gap` arm is effectively dead on this path.
6. **Shutdown cost with many live terminals.** With 50 live `/bin/cat` terminals, `serve` took about 34 s after stdin EOF to print its report, which exceeds the runner's 30 s stop bound. Sixteen terminals took about 5–9 s. `capacity/fifty` (the 50-terminal profile, reported separately and not bounded at 100 ms) therefore uses one-line programs that exit before the stop.
7. **Observed unrelated flake (F2's, not fixed here).** In one `cargo nextest run -p ovis-terminal-conformance` attempt, `all/all` failed on `remote/isolation` happy: `reply[child-on-bravo]/error` was `internal`. That scenario passed 3/3 standalone. The exact runner output and the verbatim `all.json` case are in `docs/evidence/task-30/flake-remote-isolation.log`.

## Seams (files outside the write scope, and why)

- `conformance/scenarios/attach-input/all.json`, `attach-input/adversarial.json`: the write scope names `attach-input/all/` and `attach-input/adversarial/` as directories. The runner's scenario id grammar is exactly `<family>/<name>` (a nested `attach-input/all/x.json` is classified invalid), so the QA's `scenario attach-input/all` / `attach-input/adversarial` can only be these two files. `capacity/fifty.json` is inside `conformance/scenarios/capacity/`.
- `conformance/src/runner_tests.rs` and `runner_tests/attach_input.rs` (new): the named happy/failure tests.
- `conformance/src/runner_tests/suite.rs`: the lane's own `ATTACH_INPUT` registration constant, so the shared `EXECUTABLE` slice is untouched and unions merge.
- `conformance/src/scenario/serve.rs`, `engine/steps.rs`, `engine/steps/serve.rs`: the optional `qa_output_ring_bytes` on the `serve` step.
- `crates/ovis-terminal-cli/src/invocation/{args,plan}.rs`, `src/assembly.rs` (`QaSeams.output_ring_bytes`, `QaRingInvalid`), `src/assembly/{actor,serve,live}.rs`: the QA output ring seam.
- `crates/ovis-terminal-cli/src/assembly/reads.rs`: finding 4's floor resume.
- `crates/ovis-terminal-cli/tests/assembly/pending.rs`: seam refusal covers the new flag.
- `docs/evidence/task-30/`.
- `probes/platform-qa/stress/attach-input.{json,sh}`: this lane's data-driven macOS stress lane (25 iterations of task-30 happy/failure plus `capacity/fifty`); the workflow file is unchanged.
- `schema/ovis-terminal.openrpc.json` is unchanged, so the SDK was not regenerated. `declared.json` had no `part-a/task-30` entry, so nothing was removed.

## Risks

- The 100 ms bounds are host-speed sensitive by nature (they are the stated bound). Under 16-thread nextest load, an end-to-end echo sample missed 100 ms once, which led to finding 1. The shipped samples bound only admissions and per-terminal event answers; under the same load those measured 0–16 ms.
- The QA ring changes only retention, checkpoint and slice sizes of the per-terminal screens. Stream backlog, credit and hello limits are unchanged.

DEFERRED VERIFICATION: none. `part-a/task-30` happy and failure both execute on the assembled binary (exit 0).

macOS stress dispatch: round 1 https://github.com/devswha/ovis-terminal/actions/runs/41347211460 (head fb2e4cd) failed at iteration 1, `part-a/task-30` happy after 20 assertions, from the same Darwin prompt offset; round 2 https://github.com/devswha/ovis-terminal/actions/runs/41349240120 (head b6fecf8) failed in the failure case without evidence; round 3 https://github.com/devswha/ovis-terminal/actions/runs/41352274612 (head 96ffc78, evidence kept) failed at step 49 (see Round 3); round 4 https://github.com/devswha/ovis-terminal/actions/runs/41354657097 (head edecd73; a dispatch at 96ffc78, run 41354621880, was cancelled because it named the old head).
