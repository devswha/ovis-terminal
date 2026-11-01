Plan todo 62 (wave 14): `ovis-terminal terminal attach`, a plain interactive attach client without multiplexer chrome.

**Stacked on #57 (todo 31, `lane/t31-assembly`).** This branch starts at b276c50 on that lane, which builds the binary and the first attach client. The todo-62 commits are 9e009f4, 33bc03a, bd0f933, the round-2 commits 2926824 and fe95709, the evidence commits and the authorized integration merge fe8f4de; the earlier commits in this diff belong to #57.

Closes GAP-039, GAP-040, GAP-041, GAP-042, GAP-043, GAP-044, GAP-045, GAP-056, GAP-057, GAP-058, GAP-059, GAP-145, GAP-148, GAP-149.
Delivers IDEAL-028, IDEAL-029, IDEAL-030, IDEAL-031, IDEAL-033, IDEAL-037, IDEAL-038, IDEAL-039, IDEAL-046, IDEAL-055, IDEAL-103.
Excludes (enforced, not copied): EXCL-001, EXCL-002, EXCL-004, EXCL-007, EXCL-016, EXCL-017, EXCL-019; violations V-RT-HD-07, V-250, V-198.

## Before

`docs/evidence/baseline/smoke/herdr-h08-attach.log:4` (RT-HD-07; copy of `/tmp/ulw-ovis-yCJzPH/evidence/smoke/herdr-h08-attach.log`):

> `attach#2 (no takeover) exited=true exitCode=1 output="\u001b[?1016$p\u001b[0 qherdr: server shut down: terminal attach failed: terminal term_65f10d20ab0831 already has an attached client; retry with --takeover\r\n"`

A held attach is refused as terminal text prefixed `server shut down`. It exits 1 and reports no owner and no retry boundary. Line 6 shows `--takeover` silently evicting the holder with the same text and no epoch.

Gap baseline: GAP-039, GAP-040, GAP-041, GAP-042, GAP-043, GAP-044, GAP-045, GAP-056, GAP-057, GAP-058, GAP-059, GAP-145, GAP-148, GAP-149 (open before this PR).

## What changed

- **Roles.** `--observe` needs no TTY. It never reads stdin, never resizes, never takes focus and never takes a controlling lease. Interact mode holds the interact lease and the geometry lease, and resizes only through `attach.resize` under that geometry lease.
- **Explicit lease acquire and release.** Detach closes the stream, waits (bounded) for CLOSE, then reads `attach.leases` back. The record states `leases_released` and lists `leases_held`.
- **Owners: a lease belongs to its connection (B1; rounds 1 and 3).** The plan rule is "lease_conflict is reserved for competing acquisition against a live lease; held attachment reports owner and a named lease-change retry boundary".
  - **Round 1:** the CLI's default label became `cli-<pid>`.
  - **Round 3, the root fix** (option (b) of the gate: separate connection identity from the client label):
    - `authz::Session` carries a `ConnectionId` drawn fresh by every `Authority::authenticate`.
    - `core::leases::Owner` is now principal + client label + connection, filled by `Caller::owner()`, the server's stream owner and geometry focus.
    - A second live connection presenting the same principal and `client_id` is therefore another owner. Its `attach.open{role:interact}` is answered `held`, naming the holder and the `lease.revoked` retry boundary, and the CLI exits 11 `lease_conflict`. Its `input.raw` under the holder's `lease_id`, which `attach.leases` publishes, is `no_lease`, so nothing it types reaches the program.
    - A connection's disconnect revokes only its own leases. The same connection still re-acquires its own lease idempotently, `self_lease` still refuses the holder's own takeover, and only an explicit takeover at the current epoch moves a lease between connections. This matches spec 4.2: a disconnect revokes, and a resumed connection keeps its leases.
  - **Why (b) rather than (a)** (refusing re-acquisition by the same principal and label): `input.raw` carries no client id, so the write check derives the owner from the presented `lease_id`. With (a) alone, a second connection could still read `lease:1` from `attach.leases` and type with it. With the connection inside `Owner`, the write, resize, focus and disconnect checks all use the same identity.
  - The wire protocol is unchanged: `LeaseRecord` still shows only the label. There is no schema or SDK change.
- **No chrome, byte for byte (gate round 1, B2).** `tests/interactive_transparency.rs` runs a probe that writes C0 (BS, HT, BEL, CR, LF), 8-bit C1 (CSI, IND, NEL), SGR, DECSET/DECRST (alternate screen, bracketed paste, cursor) and OSC (title, hyperlink with ST). It asserts three things: the outer terminal received exactly the bytes the server sent a protocol-level reference viewer of the same screen, followed only by the documented `ESC[0m` reset on detach; the server sent the program's raw bytes unchanged; and the probe really wrote every class.
- **Held attaches.** On the wire, `attach.open` against a live interact lease answers `outcome: held` with `holder`, a `retry` boundary (`on: lease_released`, `event: lease.revoked`, the holder's lease) and an observe-only lease. It never evicts. The CLI reports this competing acquisition as `lease_conflict` (exit 11) with `holder` and `retry`.
- **Takeover.** Takeover requires an explicit `--takeover <epoch>`. A stale epoch is `lease_epoch_mismatch` (exit 11), naming `expected_epoch` and the `current` lease (epoch and holder). A current epoch displaces the holder, whose client exits 0 with `closed.close.reason: taken_over` naming the new holder.
- **Keys.** There is no prefix key and no client mode. Ctrl-\ (0x1c) detaches without touching the child. Every other byte, including unknown escape sequences and non-UTF-8, goes through `input.raw` unchanged, and its refusal is the server's typed error.
- **Stream credit.** Credit is download-only: `attach.credit` acknowledges every half window.
- **Terminal restore on every exit path.** Modes the program switched on (mouse, bracketed paste, sync output, keypad, keyboard protocol, cursor, alternate screen last) are switched off. Then the saved termios is put back, on detach, refusal, connection loss, signal and hangup.
- **Signals.** Handlers are installed before raw mode. An interruption before the stream is attached ends as typed `interrupted` at its stage. Interruptions delivered while a detach is in progress join that detach. They do not cut its wait for CLOSE short; only `CLOSE_BOUND` (5 s) ends that wait early. The server sends CLOSE only after it commits the lease revocation, so the record's `leases_released` is read after the release, never racing it.
- **Hangup.** A hung-up outer terminal detaches with `reason: hangup`, exit 0, every lease released, and the program untouched.
  - On the raw keyboard terminal (VMIN 1), a zero-byte read or EIO is classified as hangup; any other read error is typed `transport_failed`.
  - Screen draws write the descriptor unbuffered on the blocking pool, so a failed draw leaves nothing queued for the exit report's flush to fail on. This was the cause of a 1-in-5 exit-1 race found by the 5x loop.

## After

Host: `Linux x86_64` (`docs/evidence/task-62/host.txt`). Binary `target/debug/ovis-terminal` sha256 `08127fb8b4feeedadd88cf014537813b95a7f6091b08f7396a2e9c715c85e0ad` (258142856 bytes, round-2 head fe95709; recorded in the evidence JSON).

Acceptance (`docs/evidence/task-62/acceptance.log`):

```
$ cargo build --locked -p ovis-terminal-cli --bin ovis-terminal                 exit=0
$ cargo test -p ovis-terminal-cli --all-targets                                exit=0
  lib 25 passed; assembly 8; attach 5; cli 2; interactive_attach 2; interactive_owners 2; interactive_restore 4; interactive_transparency 1 (49)
$ cargo clippy -p ovis-terminal-cli --all-targets -- -D warnings               exit=0
```

QA happy (`qa-happy.log`):

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario cli/interactive-attach --isolated --case happy --evidence "$ATTEMPT_DIR/task-62-ovis-terminal-v1.json"
{"case":"happy","error":null,...,"result":"passed","scenario":"cli/interactive-attach","totals":{"assertions":"40","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
exit=0
```

The happy case uses a real `serve` running `/bin/sh` as t1, with a socket observer (`phone`) attached. It runs `target/debug/ovis-terminal terminal attach --target t1 --client-id cli` on a real outer PTY (100x30) and asserts:
- a typed marker is evaluated by the shell;
- a resize is reported by the inner PTY;
- an unknown escape sequence passes through unchanged;
- Ctrl-\ detaches with exit 0 and `{"detached":{"leases_held":[],"leases_released":true,"reason":"detach_key","stream_closed":true,...}}`;
- the outer termios equals the original and the program's modes are switched off;
- the observer is still attached;
- the program's pid/start_token incarnation is unchanged.

QA failure, with the plan's literal inject (`qa-failure.log`):

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario cli/interactive-attach --isolated --case failure --inject '{"method":"attach.takeover","params":{"lease_epoch":"0"}}' --evidence "$ATTEMPT_DIR/task-62-ovis-terminal-v1.json" --append
{"case":"failure","error":null,...,"result":"passed","scenario":"cli/interactive-attach","totals":{"assertions":"49","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
exit=0
```

Materialized CLI commands in the failure case, with their actual stderr records:

```
$ target/debug/ovis-terminal --json terminal attach --target t1   (the rival, default label cli-<pid>) -> exit 11
{"error":{"exit_code":11,"holder":{"epoch":"1","holder":"cli","lease_id":"lease:1","role":"interact"},"kind":"lease_conflict","retry":{"event":"lease.revoked","lease":{"epoch":"1","holder":"cli",...},"on":"lease_released"},...}}
$ target/debug/ovis-terminal --json terminal attach --target t1 --client-id bridge --takeover 7 -> exit 11
{"error":{"current":{"epoch":"1","holder":"cli","lease_id":"lease:1","role":"interact"},"exit_code":11,"expected_epoch":"7","kind":"lease_epoch_mismatch",...}}
$ target/debug/ovis-terminal --json terminal attach --target t1 --client-id cli   (the holder, detaching with Ctrl-\) -> exit 0
{"detached":{"displaced":null,"leases_held":[],"leases_released":true,"reason":"detach_key","stream_closed":true,...}}
```

Socket requests in the same case:
- `attach.open` with `takeover.lease_epoch: "7"` gets `lease_epoch_mismatch` (wire reply: `{"error":{"code":-32000,"data":{"kind":"lease_epoch_mismatch","hint":"read the current lease and retry the takeover at its epoch",...}}}`).
- `lease_epoch: "0"` gets `invalid_params`.
- An ordinary `attach.open` while held returns `outcome: held`, `holder.holder: cli`, `holder.epoch: "1"`, `retry.on: lease_released`, `retry.event: lease.revoked`, `lease.role: observe` and `displaced: null`.
- A following `attach.leases` shows `cli` still holding the interact lease (no eviction).

**The plan's literal inject.** `attach.takeover` *is* a released method (`schema/released.openrpc.json`; `AttachTakeoverParams` = `op_id`, `stream_id`, `target`, `takeover{lease_epoch, reason}`; its errors include `lease_epoch_mismatch`). The assembled dispatcher does not serve it yet and answers `method_not_found`; serving it is a todo-31 follow-up, and core `TerminalLeases::take_over` already returns `EpochMismatch{current}`. The plan's literal inject could not produce `lease_epoch_mismatch` even if it were routed: the runner merges it over the case's base params, which gives `{op_id, target, role, client_id, geometry, stream_mode, takeover:null, lease_epoch:"0"}`. That lacks the required `stream_id` and `takeover` object and violates `additionalProperties:false`, so a correct server answers `invalid_params`. The failure case therefore plans the literal request and asserts the binary's actual reply (`method_not_found`), the same way #50 and #37 handled their literal injects. The plan's intent, `lease_epoch_mismatch` with the current epoch, is enforced in the same case by the CLI `--takeover 7` step and the canonical `attach.open` inject below.
- The canonical form, `--inject '{"method":"attach.open","params":{"takeover":{"lease_epoch":"7","reason":"stale takeover epoch"}}}'`, passes: exit 0, 49 assertions (`qa-failure-canonical.log`). It asserts `lease_epoch_mismatch`, and the CLI step of the same case asserts the current epoch `"1"`.
- An inject the case does not plan fails the run: `--inject '{"method":"attach.takeover","params":{"lease_epoch":"9"}}'` exits 1 on `inject/planned` (`qa-failure-unplanned-probe.log`).

All cases: `--case all`, exit 0, 94 assertions, failed 0, skipped 0 (`qa-all.log`, `task-62-ovis-terminal-v1-all.json`).

Raw evidence (request/response transcripts, process argv/stdout/stderr, binary hash): `docs/evidence/task-62/task-62-ovis-terminal-v1.json` (happy plus two appended failure runs) and `task-62-ovis-terminal-v1-all.json`.

### Adversarial classes

- **Repeated interruptions** (`tests/interactive_restore.rs`):
  - SIGINT/SIGTERM/SIGHUP/SIGQUIT before the stream is attached end as typed `interrupted` at their stage, with termios restored.
  - Repeated signals while attached detach once, release every lease and restore the terminal.
  - Closing the outer PTY master (hangup/EOF) while attached detaches with `reason: hangup`, exit 0, `leases_released: true`, empty `attach.leases`, and the program incarnation unchanged.
- **Stale state:**
  - A stale `--takeover` epoch gives `lease_epoch_mismatch` naming the current epoch.
  - A competing live-lease acquisition gives `lease_conflict` with holder and retry, including two processes with the default label (`interactive_owners.rs`). The refused process's typing never reaches the program: it exits before the holder's fence line, and the program reads lines in order.
  - A current epoch displaces the holder, which reports `taken_over` (`tests/interactive_attach.rs`).
- **Malformed input:** unknown key sequences and non-UTF-8 bytes are forwarded unchanged, and bytes after Ctrl-\ are never forwarded (unit tests plus a proptest in `attach/input.rs`; PTY test `unknown_sequences_reach_the_program_and_its_modes_are_switched_off_on_detach`).
- **Misleading success:**
  - Every PTY test checks the outer termios against the saved copy, `attach.leases` and the program incarnation, not only the exit code.
  - The transparency test checks the drawn bytes exactly, so a client that adds chrome cannot pass by still showing the program's text.

### Stability

- `docs/evidence/task-62/attach-loop-5x.log`: 5/5 iterations of `cargo nextest run -p ovis-terminal-cli --test attach --test interactive_attach --test interactive_restore` pinned to 4 CPUs (`taskset -c 0-3`), 11/11 tests each.
- Before the unbuffered-draw fix, the hangup test failed 1 in 5 (exit 1, no record). After it, the isolated hangup test passed 30/30 under the same pinning.

**CI round 2 (fe8f4de, ubuntu `scripts/check.sh`).**

`repeated_signals_while_attached_detach_once_and_restore_the_terminal` failed with `leases_released: false, stream_closed: false`.

Root cause, in the product:
- `attach.close` replies before the server commits the lease revocation; CLOSE follows the commit.
- The client let a second interruption cut its wait for CLOSE short, then read `attach.leases` before the revocation landed.
- A notice that could not be drawn (a hung-up terminal) also ended the wait as if it were CLOSE. That is the same early read on the hangup path.

Fix (`attach/session.rs`, `attach/signals.rs`):
- Interruptions delivered while detaching join the detach in progress.
- Only CLOSE (or `CLOSE_BOUND`, 5 s) ends the wait.
- The lease read therefore always follows the committed release.

Proof, under 8 busy loops pinned to CPUs 0-1 with the tests pinned to the same CPUs, through the heavy gate, 25 runs of `cargo nextest run -p ovis-terminal-cli --test interactive_restore`:
- Before the fix: 13/25 (`docs/evidence/task-62/repeated-signals-load25-before-fix.log`). The repeated-signals test failed 10 times and the hangup test twice, all with `leases_released: false`.
- After the fix: 25/25 (`repeated-signals-load25.log`).
- `cargo nextest run -p ovis-terminal-cli`: 46/46.
- `cli/interactive-attach --case all`: 94 assertions, exit 0.

**Round 2 (gate round 1 fixes), same load recipe.** 25 runs through the heavy gate of `cargo nextest run -p ovis-terminal-cli --test interactive_restore --test interactive_owners --test interactive_transparency`, with 8 busy loops pinned to CPUs 0-1 and the tests pinned there too: **25/25**, 7/7 tests each (`docs/evidence/task-62/load25-r2.log`).

### Mutants (`docs/evidence/task-62/mutants.log`)

Round 2, at 2926824 through the heavy gate: **8 of 8 killed**, tree clean after every restore.

| Mutant | Killed by |
|---|---|
| status line appended to every drawn chunk (`attach/screen.rs`) | `interactive_transparency::the_attach_client_draws_exactly_the_bytes_the_server_sent_and_nothing_else` |
| `ESC[31m` before every drawn chunk | same transparency test |
| default label back to the shared `cli` (`invocation/plan.rs`) | `interactive_owners::two_attach_processes_with_the_default_label_compete_and_the_second_is_refused` and `interactive_attach::an_observer_on_a_terminal_never_resizes_and_stays_live_after_the_operator_detaches` |
| observer reads the geometry lease (`plan.observe && false`) | `interactive_owners::an_observer_sharing_its_operators_label_never_claims_the_owners_geometry_or_input` |
| `DETACH_KEY` 0x1c -> 0x1d | `attach::input::tests::the_detach_key_ends_forwarding_mid_chunk` and three `attach` PTY tests |
| stale-epoch guard never matches | `interactive_attach::a_stale_takeover_names_the_current_epoch_and_a_current_one_displaces_the_holder` |
| `held` mapped to `Refused` | `attach::a_second_interact_attach_is_held_with_the_owner_and_never_touches_its_terminal`, the owners competition test and the stale-takeover test |
| alternate screen 1049 not reset | `attach::modes::tests::modes_left_on_are_reset_with_the_alternate_screen_last` and `interactive_restore::unknown_sequences_reach_the_program_and_its_modes_are_switched_off_on_detach` |

**Correction.** Round 1 called the observer-geometry mutant equivalent. It was not: an observer that shares its operator's client label (round 1: the shared default `cli`) reads the owner's geometry lease, and its grid reached the program (gate probe). It has the same root cause as B1, and it is now killed (see `mutants.log`).

### Gates

- `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` (foreground): exit 0. fmt, workspace clippy, `cargo nextest run --workspace` (1174 tests passed, 1 skipped), doc tests, workspace-hygiene (0 violations in 1169 files), manifests and json all pass (`docs/evidence/task-62/check-summary.log`).
All gate, acceptance and QA results above are from round-2 head fe95709, which sits on the authorized merge of `origin/integration` 7f6d268 (fe8f4de).
- `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked`: exit 0.
- `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings`: exit 0 (`windows-precheck.log`).

### macOS stress

The macOS stress job runs on the stress-only branch `lane/t62-stress`, so the shared workflow file stays at base in this PR. Each run does 25 iterations of the three attach PTY test binaries plus `cli/interactive-attach --case all` on `macos-latest`.
- Previous lane head (9e009f4): https://github.com/devswha/ovis-terminal/actions/runs/41256920783 (success).
- This head (9e07178, merged into stress branch 50c522c): https://github.com/devswha/ovis-terminal/actions/runs/41273039178: success (25/25 iterations on `macos-latest`). The attach client is unchanged by the later integration merge, which only touched the runner and the assembled fixture's ready line.
- Round 2 did not re-dispatch the stress job (not requested for this round). CI's `test (macos-latest)` job runs the new owners and transparency tests on every push.

## Merge addendum: origin/integration 26278c1 (authorized; merge-only round)

Round 2 (8f1e5d0) was CI-green and awaits the gate. Integration then moved to 26278c1 (T59 health/metrics, PR #66, with a regenerated schema and SDK), and PR #67 became CONFLICTING. This merge commit keeps the PR reviewable. The lane is still stacked on #57: merged, not rebased.

- **Textual conflict**, `conformance/src/runner_tests/suite.rs`: both sides added an entry to the sorted `EXECUTABLE` slice. Both are kept (`cli/interactive-attach.json`, `observability/health.json`); the slice stays sorted and unique.
- **Semantic conflict, the assembled `server.status`.** T59 changed `ServerStatusResult`:
  - `holder_count` and `live_terminals` are now `Count` (a canonical decimal string) or null;
  - a new `census` flag says whether those counts were reported;
  - `recovery_state` is optional.

  #57's assembled `serve` (`crates/ovis-terminal-cli/src/assembly/terminal.rs::status()`) still emitted JSON numbers, so the CLI's typed decoder rejected `server status` as `invalid_response`, and `assembly::terminal_run_spawns_a_real_program_that_list_get_and_status_report` failed. The fix:
  - `status()` now emits `census: "reported"` with decimal-string counts, the same shape as integration's own fixture (`tests/cli/support.rs`);
  - `tests/assembly.rs` asserts `census`, `holder_count: "1"` and `live_terminals: "1"`.

  The assembled serve does not report the optional `recovery_state` yet; that is a todo-31 follow-up. #57 needs the same two hunks in its own integration merge.
- **Generated outputs, regenerated rather than trusted from the textual merge.** Both came out byte-identical to integration's, so this PR carries no schema or SDK change of its own:
  - OpenRPC: `cargo run --locked -p ovis-terminal-protocol --bin openrpc -- write schema/ovis-terminal.openrpc.json` (639597 bytes), then `check` (fresh) and `compat schema/released.openrpc.json` (compatible).
  - SDK, in `packages/client-ts`: `bun install --frozen-lockfile && bun run generate` (31 files, 0 changed) `&& bun run generate:check` (fresh) `&& bun run typecheck && bun test` (30 pass, 0 fail). The tests ran against `OVIS_TERMINAL_BIN` and `OVIS_CONFORMANCE_BIN` freshly built into the RAM target, with `TMPDIR=$OVIS_FIXTURE_ROOT`.
- **`Cargo.lock`:** `git show origin/integration:Cargo.lock > Cargo.lock && cargo generate-lockfile --offline && cargo metadata --locked`. The result is identical to the textually merged lock: integration's plus 8 lines for #57/lane packages (`base64` for the unix attach client).
- **Stress workflow:** merged without conflict. This lane does not change the shared workflow; the t62 stress job stays on `lane/t62-stress`.
- **Verification on the merged tree**, all with `CARGO_TARGET_DIR=/dev/shm/ovt-target-lane-t62`:
  - `cargo clippy -p ovis-terminal-cli --all-targets -- -D warnings`: exit 0.
  - `cargo nextest run -p ovis-terminal-cli`: 49/49.
  - Conformance `--case all`: `cli/interactive-attach` 94, `part-a/task-31` 49 and `observability/health` 96 assertions, each exit 0.
  - `check.sh` through the heavy gate: exit 0 (`1256 tests run: 1256 passed (3 slow), 1 skipped`; hygiene, manifests and json pass; `docs/evidence/task-62/check-summary.log`). The first attempt stopped in the test build with rustc `Disk quota exceeded (os error 122)`: `/dev/shm` is a tmpfs with a per-user quota that every lane shares. After freeing this lane's 7.3 GB incremental cache, the rerun used `CARGO_INCREMENTAL=0`; this is a cache setting only and matches CI's rust-cache, with no change to code or gate steps.

## Merge addendum 2: origin/integration 167c0a7 (authorized; merge-only)

Integration moved to 167c0a7, which brings T86's graphics caps (#68) and T58's boundary policy, and PR #67 became CONFLICTING on round-3 head c34080d. This is one merge commit on top of c34080d. The lane is still stacked on #57, merged and not rebased.

- **Textual conflict:** only `crates/ovis-terminal-authz/src/authority.rs`, in the import block. Both imports are kept: `ConnectionId` (round 3) and T58's `Permission`. The auto-merged rest of the file keeps both sides: `Session.connection`, its accessor and `authenticate` drawing a fresh `ConnectionId`, plus T58's code. T58's `BoundaryPolicy` does not touch lease ownership. The merged file differs from integration's only by the round-3 connection hunks.
- **Generated outputs:** not regenerated, because the merge changes nothing of this lane's in them. The schema and the generated SDK in the merge are integration's own (T86's regeneration) and byte-identical to 167c0a7. `openrpc check schema/ovis-terminal.openrpc.json` reports fresh (645575 bytes), and `openrpc compat schema/released.openrpc.json` reports compatible.
- **`Cargo.lock`:** integration's plus the same 8 lane-dependency lines as before.
- **Verified locally** on a cold RAM target (`CARGO_INCREMENTAL=0`, line-tables-only, lane-scoped):
  - `cargo +nightly fmt --all -- --check`: exit 0.
  - `cargo clippy --locked -p ovis-terminal-authz -p ovis-terminal-core -p ovis-terminal-server -p ovis-terminal-cli --all-targets -- -D warnings`: exit 0, including the round-3 CLI test files.
  - `cargo nextest run --locked -p ovis-terminal-authz -p ovis-terminal-core -p ovis-terminal-server -p ovis-terminal-cli` through the heavy gate: **664/664**. This includes the round-3 tests: the core `leases::connections` tests, `interactive_connections`, `interactive_owners` (default label, shared id, test 2) and `interactive_transparency`. This is their first local run; round 3 itself was gated by CI.
  - Conformance `--case all`: `cli/interactive-attach` 106 assertions (94 before the twin-connection steps) and `part-a/task-31` 49 assertions, each exit 0 (`docs/evidence/task-62/merge2-lane-tests.log`).
- **CI is the gate for the rest:** `check.sh` (the full workspace), the Windows pre-check, client-ts and the other workspace crates. The round-3 mutants are being run by the Opus gate and were not started here.

## Round 3: B1 at the root (gate round 2)

**Finding.** Round 2 fixed only the CLI default label. The gate's socket probe showed two live connections presenting the same `client_id` both answered `ready` on `lease:1`, with `holder:null` and `retry:null`. The second connection's `input.raw` was admitted and its marker reached the program, and the first stream was never told. The cause was the core rule `held.owner == claim.owner => granted(held)` with an owner of principal + label only.

**Fix: option (b), lease owner = principal + client label + connection.** See *Owners* above for the mechanism and why (b) rather than (a).

**Tests added:**
- `ovis-terminal-core` `leases::connections`:
  - a second connection under the holder's label gets `lease_conflict` naming the holder; its write under the holder's lease is `no_lease`; the holder keeps writing and re-acquires idempotently;
  - one connection's disconnect never revokes another's leases;
  - only an explicit takeover at the current epoch moves a lease between connections, and a repeat is `self_lease`.
- `ovis-terminal-authz` `identity`: connection ids are fresh per draw and round-trip only as canonical hyphenated text.
- `ovis-terminal-cli` `interactive_connections` (socket level, real `serve`): two connections with `client_id: "sdk"`.
  - The second `attach.open{role:interact}` is `held`, naming the holder `sdk` and `lease:1`, with retry `lease_released`/`lease.revoked` and an observe-only lease.
  - Its `input.raw` under the holder's `lease_id` is `no_lease`.
  - After the holder's fence line, the program's raw output (`terminal.wait_output`, then `terminal.output`) never contains the intruder's marker.
  - The holder's lease and the program incarnation are unchanged.
- `ovis-terminal-cli` `interactive_owners`:
  - one shared helper is now also run with a shared `--client-id shared`: the second process exits 11 `lease_conflict` naming `shared`, and its typing never reaches the program;
  - test 2's premise is fixed: an observer sharing its operator's label is another owner, its grid never reaches the program, and its detach reports `leases_released:true` with `leases_held:[]` while the operator's leases stay.
- Conformance `cli/interactive-attach` failure case: a twin connection presenting the holder's own `client_id: cli` is `held` naming `cli`/`lease:1`, and its `input.raw` under `lease:1` is refused `no_lease`.

**Verified locally**, with `CARGO_INCREMENTAL=0` and the RAM target:
- `cargo clippy --locked -p ovis-terminal-authz -p ovis-terminal-core -p ovis-terminal-server -p ovis-terminal-cli --all-targets -- -D warnings`: exit 0. This ran before the new and reworked CLI test files existed; a later CLI clippy flagged `needless_pass_by_value` in the new socket helper, which is fixed.
- `cargo nextest run --locked -p ovis-terminal-authz -p ovis-terminal-core -p ovis-terminal-server` through the heavy gate: 583/583, including the four new authz and core tests (`docs/evidence/task-62/r3-authz-core-server-tests.log`).
- `cargo +nightly fmt --all -- --check`: exit 0.

**CI is the gate for the rest.** The owner's budget directive applies: `/dev/shm` is a per-user tmpfs quota shared by every lane. This lane's 23 GB mixed-profile target pushed the T86 landing gate into EDQUOT, and no new RAM build may start until that landing has run. The target was wiped, and the in-flight CLI build was stopped and its partial target removed. Therefore none of these ran locally this round:
- the CLI crate tests, including the new socket, shared-id and observer tests;
- the conformance scenarios with the twin steps;
- `check.sh`;
- the Windows pre-check;
- the round-3 mutants.

PR CI runs all of the above except the mutants.

**Round-3 mutants (deferred under the same directive).** Each is the old cross-connection rule restored at one site:
- `acquire-by-label`: `leases/table.rs` `acquire` re-grants when principal and label match, ignoring the connection. This is the old same-owner re-grant. It must fail `leases::connections` test 1, `interactive_connections` and `interactive_owners` (shared id).
- `write-by-label`: `leases/admission.rs` `check_write` matches principal and label only. It must fail `leases::connections` test 1.
- `disconnect-by-label`: `leases/table.rs` `take_owned` matches principal and label only. It must fail `leases::connections` test 2.
- `observer-reads-geometry`: `attach/session.rs` `plan.observe && false`. It must fail `interactive_owners` test 2: the observer's resize under the operator's geometry lease is refused, so the observer exits non-zero.

## Seams

Files outside the write scope (`crates/ovis-terminal-cli/src/attach/`, `conformance/scenarios/cli/interactive-attach/`), and why:

- `conformance/scenarios/cli/interactive-attach.json`: the scenario. The runner resolves `--scenario cli/interactive-attach` to this file, so it is a file rather than a directory.
- `conformance/src/runner_tests/suite.rs`: `cli/interactive-attach.json` is added in sorted position to the shared `EXECUTABLE` slice. #57's separate `ASSEMBLED` chain is kept as is. Nothing is removed from `declared.json` because the scenario was never declared there.
- `conformance/scenarios/part-a/task-31.json` and `crates/ovis-terminal-cli/tests/attach.rs`: the todo-31 refusal kind `lease_held` is renamed to `lease_conflict`. The plan reserves `lease_conflict` for competing acquisition against a live lease.
- `crates/ovis-terminal-cli/src/attach.rs`: the module root (submodule declarations and contract doc).
- `crates/ovis-terminal-cli/src/invocation/{args.rs,plan.rs}`:
  - the `--takeover <EPOCH>` flag (conflicts with `--observe`), decoded into the `LeaseEpoch` newtype as a decimal string;
  - round 2: `--client-id` defaults to `cli-<pid>` (B1).
- `crates/ovis-terminal-cli/src/invocation/exit.rs`:
  - `interrupted` maps to exit 10 (Cancelled);
  - `lease_epoch_mismatch` maps to exit 11 (Conflict);
  - `lease_held` is removed.
- `crates/ovis-terminal-cli/src/invocation/report.rs`: attach errors contribute their typed detail fields (`holder`, `retry`, `expected_epoch`, `current`, `stage`, `signal`) to the JSON error record.
- `crates/ovis-terminal-cli/src/invocation/skill.md`: the shipped agent skill documents the attach roles, takeover, detach key, exit kinds and (round 2) the per-process default owner label.
- `crates/ovis-terminal-cli/tests/{interactive_attach.rs,interactive_attach/helpers.rs,interactive_restore.rs}`: real-PTY behaviour tests (unix only, matching the attach client's unix-only transport).
- Round 2 tests:
  - `crates/ovis-terminal-cli/tests/interactive_owners.rs`: default-label competition, and an observer sharing its operator's label.
  - `crates/ovis-terminal-cli/tests/interactive_transparency.rs` and `interactive_transparency/viewer.rs`: the byte-exact test and its protocol-level reference viewer.
- `crates/ovis-terminal-cli/tests/attach/{support.rs,outer.rs}` (#57's shared PTY test support):
  - `Outer` moved verbatim into the child module `outer.rs` and re-exported as `support::Outer`, keeping `support.rs` under the 250 pure-LOC cap (hygiene 0 violations).
  - `Served::endpoint()` and `Outer::drawn_until_closed()` added for the transparency test.
  - #57 has not changed `support.rs` since b276c50, so the move cannot conflict with it.
- `crates/ovis-terminal-cli/tests/attach.rs`, `conformance/scenarios/part-a/task-31.json`, `conformance/scenarios/cli/interactive-attach.json`: assertions that name the holder `cli` now pin it with an explicit `--client-id cli` on the holder. Default-label holders are asserted as `cli-<pid>`, and the interactive-attach rival now uses the default label, so the conformance failure case also covers B1.
- **Authorized merge of `origin/integration` 7f6d268.** PR #67 was CONFLICTING after the first push. The conflicts were all additive between #57's runner seams and integration's, and were resolved keeping both sides:
  - `conformance/src/engine.rs`: `pty` and `redact` modules;
  - `conformance/src/engine/session.rs`: `ptys` and `kept` fields;
  - `suite.rs`: as above.
- `crates/ovis-terminal-cli/src/assembly/{boot.rs,serve.rs}`: integration's fixture launcher now requires a `writer` bearer in the ready line (`deny_unknown_fields`). Without it, both assembled-binary scenarios (`part-a/task-31`, `cli/interactive-attach`) failed with `fixture_report`. `boot.rs` is taken verbatim from #57's head (`delegate` issuing observer and writer bearers), and `serve.rs` adds the same `"writer"` ready field #57 announces, so the union with #57 stays conflict-free.
- `crates/ovis-terminal-cli/src/assembly/terminal.rs` and `crates/ovis-terminal-cli/tests/assembly.rs` (both #57's): the `server.status` census and decimal-string resolution of the 26278c1 merge (see the merge addendum).
- **Round 3, the B1 root fix (lease owner = principal + label + connection).** Every file below is outside todo-62's write scope; the gate required the fix at the lease owner identity.
  - `crates/ovis-terminal-authz/src/{identity.rs,authority.rs,lib.rs}`: the `ConnectionId` newtype (a random v4 id; its canonical hyphenated text is the only accepted form) and `Session::connection()`, drawn per `authenticate`, plus the re-export.
  - `crates/ovis-terminal-core/src/leases/{lease.rs,requests.rs}`: `Owner.connection` and `Caller::owner()`.
  - `crates/ovis-terminal-core/src/geometry/view.rs`: geometry focus derives the owner with its connection.
  - `crates/ovis-terminal-server/src/streams/handler.rs`: the stream owner carries the connection, so detach and disconnect release only that connection's leases.
  - `crates/ovis-terminal-core/src/leases/qa/{connections.rs,fixture.rs,wire.rs}` and `crates/ovis-terminal-core/tests/leases.rs`:
    - three connection-scoped lease tests;
    - a `twins` fixture (one credential, two connections);
    - the wire fixture's owner gains `connection`.
  - `crates/ovis-terminal-cli/tests/interactive_connections.rs` and `interactive_connections/socket.rs`: the socket-level test (two connections, one `client_id`), with a raw connection helper.
  - `crates/ovis-terminal-cli/tests/interactive_owners.rs`:
    - a shared helper, now also run with a shared `--client-id`;
    - test 2's premise fixed: an observer sharing its operator's label is another owner and releases only its own observe lease.
  - `conformance/scenarios/cli/interactive-attach.json`: the failure case gains a twin connection presenting the holder's own `client_id: cli`. It must be answered `held` naming the holder, and its `input.raw` under `lease:1` must be refused `no_lease`.
- `docs/evidence/task-62/`: evidence.

No schema change, so the SDK was not regenerated.

## Risks

- This PR is stacked on #57, so its diff against `integration` includes #57's commits until #57 lands. The `boot.rs`/`serve.rs` writer-bearer hunks are #57's own code, carried here only so the merged runner can start the assembled fixture.
- The wire `lease_epoch_mismatch` error data names no current epoch; the server's error is outside this write scope. The CLI reads `attach.leases` after the mismatch and reports `current` (epoch and holder). That read is a separate request, so a lease change between the two calls would show the newer lease.
- Hangup classification relies on raw mode (VMIN 1): only a hung-up terminal makes a read return zero bytes. The interact role requires a TTY on stdin and stdout, so the keyboard is always that raw terminal.
- The attach client is unix-only (Unix socket transport). On Windows, `UnsupportedTransport` remains, as in #57.
- `attach.takeover` is a released method that the assembled dispatcher does not serve yet (`method_not_found`). Serving it is a todo-31 follow-up outside this write scope; the CLI takes over through the `attach.open` takeover field.
- `hello.resume{connection_id}` is not implemented yet. When it lands, the resumed connection's `Session` must keep its `ConnectionId`, so that its leases survive the reconnect as spec 4.2 requires; today a reconnect is a new owner.
- Persisted `LeaseEvent` records now carry `owner.connection`, and the core wire QA fixture shows the field. No production code reads lease events back, and lease liveness ends with the connection anyway.
- Gate NOTE, not changed: `interactive_attach.rs` and `attach.rs` assert that observer typing is absent with a time-bounded negative check. Observe mode never reads stdin, which `observing_needs_no_tty_and_never_forwards_input` also covers. The round-2 refused-attach check orders causally: the refused process exits before the holder types its fence.

DEFERRED VERIFICATION: none. The runner executes `cli/interactive-attach`; all commands above ran with the exit codes shown.
