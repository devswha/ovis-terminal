Plan todo 62 (wave 14): `ovis-terminal terminal attach`, a plain interactive attach client without multiplexer chrome.

**Stacked on #57 (todo 31, `lane/t31-assembly`).** This branch starts at b276c50 on that lane, which builds the binary and the first attach client. The todo-62 commits are 9e009f4 and 33bc03a plus the evidence commit; the earlier commits in this diff belong to #57.

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
- **Held attaches.** On the wire, `attach.open` against a live interact lease answers `outcome: held` with `holder`, a `retry` boundary (`on: lease_released`, `event: lease.revoked`, the holder's lease) and an observe-only lease. It never evicts. The CLI reports this competing acquisition as `lease_conflict` (exit 11) with `holder` and `retry`.
- **Takeover.** Takeover requires an explicit `--takeover <epoch>`. A stale epoch is `lease_epoch_mismatch` (exit 11), naming `expected_epoch` and the `current` lease (epoch and holder). A current epoch displaces the holder, whose client exits 0 with `closed.close.reason: taken_over` naming the new holder.
- **Keys.** There is no prefix key and no client mode. Ctrl-\ (0x1c) detaches without touching the child. Every other byte, including unknown escape sequences and non-UTF-8, goes through `input.raw` unchanged, and its refusal is the server's typed error.
- **Stream credit.** Credit is download-only: `attach.credit` acknowledges every half window.
- **Terminal restore on every exit path.** Modes the program switched on (mouse, bracketed paste, sync output, keypad, keyboard protocol, cursor, alternate screen last) are switched off. Then the saved termios is put back, on detach, refusal, connection loss, signal and hangup.
- **Signals.** Handlers are installed before raw mode. An interruption before the stream is attached ends as typed `interrupted` at its stage. A second interruption while detaching stops waiting for CLOSE.
- **Hangup.** A hung-up outer terminal detaches with `reason: hangup`, exit 0, every lease released, and the program untouched.
  - On the raw keyboard terminal (VMIN 1), a zero-byte read or EIO is classified as hangup; any other read error is typed `transport_failed`.
  - Screen draws write the descriptor unbuffered on the blocking pool, so a failed draw leaves nothing queued for the exit report's flush to fail on. This was the cause of a 1-in-5 exit-1 race found by the 5x loop.

## After

Host: `Linux x86_64` (`docs/evidence/task-62/host.txt`). Binary `target/debug/ovis-terminal` sha256 `f59df2629c171071da54b5874b6a0da33dd6c2aef843679645b6660cebf2c475` (243285368 bytes; recorded in the evidence JSON).

Acceptance (`docs/evidence/task-62/acceptance.log`):

```
$ cargo build --locked -p ovis-terminal-cli --bin ovis-terminal                 exit=0
$ cargo test -p ovis-terminal-cli --all-targets                                exit=0
  lib 25 passed; assembly 8; attach 5; cli 2; interactive_attach 2; interactive_restore 4
$ cargo clippy -p ovis-terminal-cli --all-targets -- -D warnings               exit=0
```

QA happy (`qa-happy.log`):

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario cli/interactive-attach --isolated --case happy --evidence "$ATTEMPT_DIR/task-62-ovis-terminal-v1.json"
{"case":"happy","error":null,...,"result":"passed","scenario":"cli/interactive-attach","totals":{"assertions":"40","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
exit=0
```

The happy case uses a real `serve` running `/bin/sh` as t1, with a socket observer (`phone`) attached. It runs `target/debug/ovis-terminal terminal attach --target t1` on a real outer PTY (100x30) and asserts:
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
$ target/debug/ovis-terminal --json terminal attach --target t1 --client-id rival            -> exit 11
{"error":{"exit_code":11,"holder":{"epoch":"1","holder":"cli","lease_id":"lease:1","role":"interact"},"kind":"lease_conflict","retry":{"event":"lease.revoked","lease":{"epoch":"1","holder":"cli",...},"on":"lease_released"},...}}
$ target/debug/ovis-terminal --json terminal attach --target t1 --client-id bridge --takeover 7 -> exit 11
{"error":{"current":{"epoch":"1","holder":"cli","lease_id":"lease:1","role":"interact"},"exit_code":11,"expected_epoch":"7","kind":"lease_epoch_mismatch",...}}
$ target/debug/ovis-terminal --json terminal attach --target t1   (the holder, detaching with Ctrl-\) -> exit 0
{"detached":{"displaced":null,"leases_held":[],"leases_released":true,"reason":"detach_key","stream_closed":true,...}}
```

Socket requests in the same case:
- `attach.open` with `takeover.lease_epoch: "7"` gets `lease_epoch_mismatch` (wire reply: `{"error":{"code":-32000,"data":{"kind":"lease_epoch_mismatch","hint":"read the current lease and retry the takeover at its epoch",...}}}`).
- `lease_epoch: "0"` gets `invalid_params`.
- An ordinary `attach.open` while held returns `outcome: held`, `holder.holder: cli`, `holder.epoch: "1"`, `retry.on: lease_released`, `retry.event: lease.revoked`, `lease.role: observe` and `displaced: null`.
- A following `attach.leases` shows `cli` still holding the interact lease (no eviction).

**The plan's literal inject.** There is no `attach.takeover` wire method. EXCL-007 makes takeover an explicit, epoch-bound field of `attach.open`, not a separate verb, so the literal request is `method_not_found`. The failure case plans the literal request alongside the canonical ones, the same way #50 and #37 handled their literal injects.
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
  - A competing live-lease acquisition gives `lease_conflict` with holder and retry.
  - A current epoch displaces the holder, which reports `taken_over` (`tests/interactive_attach.rs`).
- **Malformed input:** unknown key sequences and non-UTF-8 bytes are forwarded unchanged, and bytes after Ctrl-\ are never forwarded (unit tests plus a proptest in `attach/input.rs`; PTY test `unknown_sequences_reach_the_program_and_its_modes_are_switched_off_on_detach`).
- **Misleading success:** every PTY test checks the outer termios against the saved copy, `attach.leases`, and the program incarnation, not only the exit code.

### Stability

- `docs/evidence/task-62/attach-loop-5x.log`: 5/5 iterations of `cargo nextest run -p ovis-terminal-cli --test attach --test interactive_attach --test interactive_restore` pinned to 4 CPUs (`taskset -c 0-3`), 11/11 tests each.
- Before the unbuffered-draw fix, the hangup test failed 1 in 5 (exit 1, no record). After it, the isolated hangup test passed 30/30 under the same pinning.

### Mutants (`docs/evidence/task-62/mutants.log`)

4 of 5 killed; 1 is equivalent:

| Mutant | Killed by |
|---|---|
| `DETACH_KEY` 0x1c -> 0x1d | `attach::input::tests::the_detach_key_ends_forwarding_mid_chunk` |
| stale-epoch guard never matches | `interactive_attach::a_stale_takeover_names_the_current_epoch_and_a_current_one_displaces_the_holder` |
| `held` mapped to `Refused` | `attach::a_second_interact_attach_is_held_with_the_owner_and_never_touches_its_terminal` and the stale-takeover test |
| alternate screen 1049 not reset | `attach::modes::tests::modes_left_on_are_reset_with_the_alternate_screen_last` and `interactive_restore::unknown_sequences_reach_the_program_and_its_modes_are_switched_off_on_detach` |
| observer reads the geometry lease | equivalent: `geometry_lease` returns only a lease whose holder is this client, and observers are never granted geometry; the observable is asserted by `an_observer_on_a_terminal_never_resizes_and_stays_live_after_the_operator_detaches` |

### Gates

- `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` (foreground): exit 0. fmt, workspace clippy, `cargo nextest run --workspace` (949 tests passed, 1 skipped), doc tests, workspace-hygiene (0 violations in 957 files), manifests and json all pass (`docs/evidence/task-62/check-summary.log`).
- `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked`: exit 0.
- `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings`: exit 0 (`windows-precheck.log`).

### macOS stress

STRESS_PLACEHOLDER

## Seams

Files outside the write scope (`crates/ovis-terminal-cli/src/attach/`, `conformance/scenarios/cli/interactive-attach/`), and why:

- `conformance/scenarios/cli/interactive-attach.json`: the scenario. The runner resolves `--scenario cli/interactive-attach` to this file, so it is a file rather than a directory.
- `conformance/src/runner_tests/suite.rs`: registers the scenario in the executable inventory. This is a separate `INTERACTIVE_ATTACH` array chained into the sorted list, the same pattern #57 used for `ASSEMBLED`, to keep the shared `EXECUTABLE` array conflict-free.
- `conformance/scenarios/part-a/task-31.json` and `crates/ovis-terminal-cli/tests/attach.rs`: the todo-31 refusal kind `lease_held` is renamed to `lease_conflict`. The plan reserves `lease_conflict` for competing acquisition against a live lease.
- `crates/ovis-terminal-cli/src/attach.rs`: the module root (submodule declarations and contract doc).
- `crates/ovis-terminal-cli/src/invocation/{args.rs,plan.rs}`: the `--takeover <EPOCH>` flag (conflicts with `--observe`), decoded into the `LeaseEpoch` newtype as a decimal string.
- `crates/ovis-terminal-cli/src/invocation/exit.rs`:
  - `interrupted` maps to exit 10 (Cancelled);
  - `lease_epoch_mismatch` maps to exit 11 (Conflict);
  - `lease_held` is removed.
- `crates/ovis-terminal-cli/src/invocation/report.rs`: attach errors contribute their typed detail fields (`holder`, `retry`, `expected_epoch`, `current`, `stage`, `signal`) to the JSON error record.
- `crates/ovis-terminal-cli/src/invocation/skill.md`: the shipped agent skill documents the attach roles, takeover, detach key and exit kinds.
- `crates/ovis-terminal-cli/tests/{interactive_attach.rs,interactive_attach/helpers.rs,interactive_restore.rs}`: real-PTY behaviour tests (unix only, matching the attach client's unix-only transport).
- `docs/evidence/task-62/`: evidence.

No schema change, so the SDK was not regenerated.

## Risks

- This PR is stacked on #57, so its diff against `integration` includes #57's commits until #57 lands.
- The wire `lease_epoch_mismatch` error data names no current epoch; the server's error is outside this write scope. The CLI reads `attach.leases` after the mismatch and reports `current` (epoch and holder). That read is a separate request, so a lease change between the two calls would show the newer lease.
- Hangup classification relies on raw mode (VMIN 1): only a hung-up terminal makes a read return zero bytes. The interact role requires a TTY on stdin and stdout, so the keyboard is always that raw terminal.
- The attach client is unix-only (Unix socket transport). On Windows, `UnsupportedTransport` remains, as in #57.

DEFERRED VERIFICATION: none. The runner executes `cli/interactive-attach`; all commands above ran with the exit codes shown.
