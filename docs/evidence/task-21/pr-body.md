Plan todo 21 (wave 9). Closes GAP-004, GAP-006, GAP-007, GAP-008. Delivers IDEAL-004, IDEAL-005, IDEAL-006, IDEAL-007, IDEAL-008, IDEAL-013 as contract slices, each checked over a real socket.

## Before

RT-HD-01, GAP-004, GAP-006, GAP-007, GAP-008: `/tmp/ulw-ovis-yCJzPH/evidence/smoke/herdr-h02-connection.log:2`, durable copy `docs/evidence/baseline/smoke/herdr-h02-connection.log:2`:

```
frames received on one connection: 1 closedByServer=true ms=0
```

## After

Host: Linux x86_64 (`uname -sm`), toolchain 1.99.1. Binary under test: `target/debug/ovis-terminal` (sha256 is recorded in each evidence file).

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-21 --isolated --case happy --evidence "$ATTEMPT_DIR/task-21-ovis-terminal-v1.json"
{"case":"happy","error":null,...,"result":"passed","scenario":"part-a/task-21","totals":{"assertions":"24","cases":"1","failed":"0","passed":"1","skipped":"0"}}
exit 0
```

The actual transcript of that run (`docs/evidence/task-21/qa-happy-transcript.log`) was captured with `--manual-transcript`. Every byte went through `ovis-terminal connect`, on one process and one connection. The credential is redacted by the runner.

```
bridge >> @0    234B write: {"id":"hello",...,"credential":"<redacted-credential>","framed":false,"generation":"1"}}
bridge << @0    1121B line: {"jsonrpc":"2.0","id":"hello","result":{...}}
bridge >> @234  306B write: {"id":"held",...doc.set...}\n{"id":"set-alpha",...doc.set...}        (two requests, one write)
bridge << @1121 102B line: {"jsonrpc":"2.0","method":"$/fixture/held","params":{"op_id":"01a191f2-..."}}
bridge << @1223 75B  line: {"jsonrpc":"2.0","id":"set-alpha","result":{"name":"alpha","value":"one"}}
bridge >> @540  194B write: {"id":"get-alpha",...doc.get...}\n{"id":"inspect-held",...op.get...}
bridge << @1298 179B line: {"jsonrpc":"2.0","id":"inspect-held","result":{...,"state":"accepted",...}}   (answered before the earlier get-alpha)
bridge << @1477 75B  line: {"jsonrpc":"2.0","id":"get-alpha","result":{"name":"alpha","value":"one"}}
bridge >> @734  116B write: {"id":"cancel-held","jsonrpc":"2.0","method":"op.cancel",...}
bridge << @1552 215B line: {"jsonrpc":"2.0","id":"cancel-held","result":{...,"outcome":"cancelled_before_commit","state":"cancelled",...}}
bridge << @1767 260B line: {"jsonrpc":"2.0","id":"held","error":{...,"data":{"kind":"cancelled",...}}}  (the first request, answered last)
```

Parsed from `docs/evidence/task-21/task-21-ovis-terminal-v1.happy.json`:
- The connection sent 850 bytes in 4 writes and received 2027 bytes; 2027 bytes were decoded into 7 records. The asserted check is `connection[bridge]/bytes_received=bytes_decoded`.
- The fixture server's own serve report: `{"accepted":"1","failures":{},"panicked":"0"}`, asserted as `/accepted == "1"`. The bridge process exited 0 and the fixture process exited 0.
- The effect directory holds exactly `alpha.json` = `"one"`; the held write left no file.

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-21 --isolated --case failure --evidence "$ATTEMPT_DIR/task-21-ovis-terminal-v1.json"
{"case":"failure",...,"result":"passed","scenario":"part-a/task-21","totals":{"assertions":"42","cases":"1","failed":"0","passed":"1","skipped":"0"}}
exit 0
```

This exit-0 means the typed rejections were observed (`docs/evidence/task-21/qa-failure-transcript.log`). The run used one framed connection through the bridge:
- `{"target":{"terminal_idd":"terminal:2"}}` returned `invalid_params` with `field_path` = `params.target.terminal_idd`.
- A legacy `pane_id` was rejected at `params.pane_id`.
- A missing target and a doubled target were rejected at `params.target`.
- A numeric counter was rejected at `params.version`.
- `pane.split` returned `method_not_found` at `method`.
- An ambiguous name returned `ambiguous_target`; an unknown id returned `not_found`.
- The effect directory is `[]` both after the rejections and after the close.
- Before any byte is sent, the runner decodes the `oversized-header` wire fixture itself (whole and byte-at-a-time) and asserts the declared `{"error":"frame_too_large"}` (`wire[oversized-header]/decode/whole`, `/decode/bytewise`).
- That 9-byte header, declaring 16 MiB + 1, got `$/closing` `{"reason":"frame_too_large"}` and end of stream. The fixture's serve report counts `failures.frame_too_large = "1"`.

DEFERRED VERIFICATION (runner lands in Todo 21): this PR is that runner. `part-a/task-21` now executes and exits 0 as shown above. The earlier part-A descriptor files (for example `part-a/task-5`) carry no `"format": "ovis-conformance/1"` marker; the runner now reports them as `scenario_not_executable`, exit 1, instead of the old scaffold exit 2. Making those descriptors executable stays with their owning todos.

## What the runner is

`conformance --binary <ovis-terminal> --scenario <family/name> --isolated --case <happy|failure|all> --evidence <file>` (also `--require-no-skips` and `--manual-transcript`, for F3).

- **Isolation.** Each case gets a fresh 0700 `/tmp/ovc-<pid>-<n>` root, removed on drop. `--isolated` is mandatory; without it the run exits 2 before anything starts.
- **Fixture process.** The runner re-executes itself as `conformance fixture --root <dir>`. The fixture serves the real `ovis-terminal-server` listener, connection layer and `Dispatcher` over the real `ovis-terminal-store` SQLite coordinator and `Authority` on a private Unix socket. It announces readiness on stdout and stops when its stdin closes, then prints its `ServeReport`. No signal is ever sent; after a bound, only that child's own handle is killed.
- **Methods.** The fixture registers `doc.set`, `doc.get` and `terminal.keep` (over a real `TargetIndex`) through `Dispatcher::register`. Their effects are files the runner reads directly. `rpc.discover` and paged `terminal.output` (a deterministic 64 MiB output) are served directly by the handler.
- **Transports.** A connection goes through the binary's `ovis-terminal connect` bridge or through the runner's own socket, for exact write segmentation.
- **Scenarios.** Scenarios are closed JSON (`deny_unknown_fields`; unknown matcher operators are refused). A file is executable only with `"format": "ovis-conformance/1"`; files without it are declarations and are reported as `scenario_not_executable`.

## Descriptor rule

Every key the runner reads from a scenario or fixture file is in one of three classes. There is no fourth class, and no key is read and then ignored.

1. **Asserted.** Steps, expectations, fixture requests with `kind`/`field_path`, and wire `hex` with exactly one of `decodes` or `error`. Each one produces an assertion in the evidence. Wire `decodes`/`error` are checked at run time: `send_wire` decodes the bytes with `ovis_terminal_protocol::wire::FrameDecoder`, whole and byte-at-a-time, before writing them.
2. **Descriptors.** Free text never asserted: a scenario's `title`, a case's `summary`, and a fixture's `descriptors` map (`{"summary": ...}`). The runner copies them into the evidence (`declared`, `summary`, `descriptors[] {source, descriptors}`) and nothing compares them.
3. **Anything else is rejected.** Every schema struct is `#[serde(deny_unknown_fields)]`. An unknown key fails the load with `scenario_invalid` and its field path (for example `cases[0].steps[0].serve.extra`); a wire entry with both or neither outcome fails with `scenario_inconsistent`.

The earlier part-A descriptor files that carry keys such as `unasserted-may-follow-leader-reap` have no format marker. The runner therefore reports them as `scenario_not_executable` instead of running them with skipped keys. Their owners register them in this format.
- **Pass rule.** Every check becomes an assertion in the evidence. A case passes only with at least one assertion and no failure. Missing scenario, missing case, timeout, unexpected record, zero assertions and unwritable evidence all exit 1. There are no retries.
- **Evidence.** Each run records requests, replies, every write and decoded record with absolute offsets and sha256, byte accounting, process argv/pid/exit/stderr, the fixture report, OS/arch and the binary digest. Every counter is a decimal string.

Registered scenarios: `protocol/multiplex`, `protocol/strict-target`, `protocol/fragmentation`, `protocol/discovery`, `protocol/deadline`, `protocol/pagination`, `_runner/self-test`, `part-a/task-21`, and the suite `all/all`, which discovers every registered non-suite file.

Shared, language-neutral fixtures:
- `conformance/fixtures/wire/frames.json`: 9 frame byte sequences with their decodings or typed errors.
- `conformance/fixtures/protocol/strict-rejections.json`: 8 requests with expected kinds and field paths.

`docs/design/traceability.json` maps all 117 IDEAL, 150 GAP and 27 EXCL ids to their owning todos and scenarios. It was derived from each plan todo's `Delivers`, `Closes` and `EXCL-` text and its `--scenario` names, plus this todo's protocol scenarios.

## QA

| Command | Exit | What is asserted |
|---|---|---|
| `bash scripts/check.sh` (on the final PR head, which includes origin/integration ef1d8c1) | 0 | fmt, clippy `-D warnings` on the workspace, nextest `523 passed, 1 skipped` (the skip is pre-existing, outside this crate), doctests, hygiene (250 LOC / 3 params), manifests, JSON |
| `cargo nextest run -p ovis-terminal-conformance --all-targets` | 0 | 18 tests: 12 unit, 6 process (below) |
| `cargo clippy -p ovis-terminal-conformance --all-targets -- -D warnings` | 0 | |
| `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` | 0 | Windows compile with no warnings. On the earlier head 5bf6fd2 the same Windows-target clippy reproduced the 11 CI errors (unused `fixture::output` items, a never-constructed `RunnerError::Fixture`, 2 needless `async`, 1 needless `mut`) |

The plan's acceptance line says `-p conformance`, but the scaffold's package name is `ovis-terminal-conformance`, so that is the spec used above. The package was not renamed.

Process tests (`conformance/src/runner_tests.rs`, which run the real runner binary against the real `ovis-terminal`):
- `happy`: `part-a/task-21 --case happy` exits 0. It asserts `fixture.accepted == "1"`, exactly one connection with `via == "bridge"` and `bytes_received == bytes_decoded`, bridge exit 0, the `held` reply kind `cancelled`, and that `set-alpha` passed.
- `failure`: `--case failure` exits 0. It asserts the `params.target.terminal_idd` field path, effects `[]`, the `$/closing` reason `frame_too_large`, `fixture.failures.frame_too_large == "1"`, both runtime `wire[oversized-header]/decode/*` assertions passed with `frame_too_large`, and the evidence records the rejection fixture's descriptors.
- `a_missing_scenario_fails_and_a_declaration_is_not_executable`: exit 1 with `scenario_missing`, and exit 1 with `scenario_not_executable` for `part-a/task-5`.
- `a_run_without_isolation_is_a_usage_error_before_anything_starts`: exit 2, `isolation_required`, and no evidence file is created.
- `runner_self_test_detects_its_own_failure_modes`: `_runner/self-test` exits 0, and its evidence shows that `expect_failure` observed `assertion_failed` and `timeout`.
- `every_registered_scenario_passes_with_no_skips`: `all/all --require-no-skips` exits 0 with `skipped == "0"`, and every protocol scenario plus `_runner/self-test` ran.

Unit tests:
- `ids`: u64::MAX decimal round trip, and rejection of numeric, leading-zero, negative, overflowing and empty forms.
- Scenario-id grammar, including path traversal rejected.
- Labels: one UUIDv7 per label.
- Matcher: every operator both holds and fails; unknown and mixed operators are refused.
- Wire fixtures: every fixture decodes as declared at every single split point and bytewise through `ovis_terminal_protocol::wire::FrameDecoder`, and errors keep their kind. A wire entry with both or neither outcome is refused, and so is an entry carrying an undeclared `summary` key.
- Rejection fixture: `descriptors` is accepted and recorded, while a top-level `summary` or a misspelled case key (`feild_path`) is refused. A proptest decodes the coalesced valid fixtures identically under arbitrary segmentation.
- Pagination: the largest allowed page fits one frame, one byte more is `frame_too_large`, and a numeric offset is `invalid_params`.
- Traceability: the id sets equal the table rows of `ideal-state.md` and `gap-list.md`; every id has owners in todos 1..70 and at least one scenario, with no repeats. Every registered scenario's declared `delivers`/`closes`/`excludes` ids list that scenario and its todo.

What each scenario checks over the socket:
- **protocol/fragmentation**: the `coalesced-requests` wire fixture replayed to the server; byte-at-a-time writes and irregular chunks that cut UTF-8 sequences, in both framed and NDJSON modes, give identical values. An unknown frame kind closes only that connection; the other connection stays served.
- **protocol/discovery**: `ovis-terminal --json schema` stdout, `rpc.discover` over the bridge and `schema/ovis-terminal.openrpc.json` are equal JSON. Hello advertises the served methods and excludes `terminal.run`, `pane.split` and `server.stop`. `ovis-terminal --json call doc.get --params [1]` exits 2 with `invalid_params` before connecting.
- **protocol/deadline**: a held mutation gets `deadline_exceeded` with `op_id`, `deadline_ms` "10000" and `elapsed_ms` >= "10000", while the unrelated `doc.get` sent after it on the same connection is answered first. `op.get` reports `accepted`, and `op.cancel` returns `cancelled_before_commit` with no effect. Cancelling committed work returns `already_committed` with the retained result; an unknown op is `not_found`.
- **protocol/pagination**: through the bridge on a framed connection, 8 MiB `terminal.output` pages reassemble 67108864 bytes. Their SHA-256 equals a digest computed independently (in Python) and pinned in the scenario, every reply record is at most 16 MiB, and the run ends on an empty page at `next_offset`. Asking for the whole 64 MiB at once is `frame_too_large`.

macOS stress: `gh workflow run probe-stress-macos.yml --ref lane/t21-conformance-runner -f probe=conformance` runs 25 iterations of the package tests: https://github.com/devswha/ovis-terminal/actions/runs/41201714418 (its result is recorded in `docs/evidence/task-21/README.md` once it finishes).

## Adversarial

`docs/evidence/task-21/adversarial-probes.log` was produced with temporary scenario files that were deleted afterwards. Every probe exits 1 with its kind:

- **Flaky tests / missing assertions:** a case that only starts the fixture fails with `no_assertions`; a step with an unknown key fails with `scenario_invalid` at `cases[0].steps[0].serve.extra`; a file whose declared id differs from its path fails with `scenario_inconsistent`. Nothing is retried.
- **Misleading success:** `--binary /bin/true` fails with `closed` ("the connection ended while the hello reply was expected"). A missing binary fails with `binary_missing`. No case can pass on a synthetic result; every pass needs observed socket records, effect files or fixture counters.
- **Malformed input:** oversized header, unknown kind, truncated header and truncated payload, in the wire fixtures and over the socket. A non-object record closes the connection with `invalid_request`.
- **Cancel/resume at byte boundaries:** bytewise and chunked splits that cut frame headers and UTF-8 sequences.
- **Load:** `all/all` ran twice with exit 0 (337 assertions each) while a parallel workspace nextest run saturated the host. No fixture or bridge process outlived its case (`pgrep` listing, read-only), and no `/tmp/ovc-*` root remained.

## Contract checklist

1. **u64 as decimal strings.** Every counter in the evidence, the fixture report and `Pages` totals/offsets is a canonical decimal string (`ids::Decimal`); numeric forms are rejected, and `counters_round_trip_exactly_at_u64_max_and_reject_numbers` covers u64::MAX. Holder wire generation stays 2; this PR does not touch it.
2. **Typed errors.** `RunnerError` is one `thiserror` enum with `kind()` and `hint()`, with `#[source]` on every inner error. Display text never repeats the kind token.
3. **Newtypes.** `ScenarioId`, `CaseName`, `ConnName` and `Decimal` are distinct newtypes; `Option` is used only where absence is a real state.
4. **Tests assert behavior.** Socket records, effect files, server counters and exit codes. Proptest covers fragmentation.
5. **No suppressions.** The crate has no `#[allow]` and no `#[expect]` of any kind. Fixture prose lives in recorded `descriptors` (see Descriptor rule), and wire `decodes`/`error` drive a run-time assertion, so every deserialized field is read. No TODOs.
6. **Stated evidence.** This body states what the tests assert.

## Seams

- No landed crate's public API changed.
- `conformance/Cargo.toml` gains a lib target (`src/lib.rs`), a `[[test]] runner` at `src/runner_tests.rs` and dependencies (direct `base64 = "0.22"`, already in Cargo.lock). `src/main.rs` becomes a one-line entry.
- `.github/workflows/probe-stress-macos.yml` gains only the `conformance` option and job at the end. It was union-merged with integration's `encoding` option and job; nothing was reordered.

## Risks

- **The server is a fixture, not the assembled binary.** The binary under test (`ovis-terminal`) has no serve command before todo 31. In this PR the binary is exercised as the client and bridge (`connect`, `--json schema`, `--json call`). The server side is the real server crate, dispatcher and store hosted by the fixture process. The fixture's `doc.*` and `terminal.keep` are effect stand-ins registered through `Dispatcher::register`, not the document or terminal services. Real-binary server conformance remains todo 31's assembly gate.
- **Dispatcher race, found by this runner (todo 14 code; not fixed here).** An `op.cancel` that arrives before an admitted operation's durable `accept` returns `not_found` and never signals the in-flight task, which then runs to `deadline_exceeded`. It was reproduced over the socket before the barrier was added. The scenarios now await `$/fixture/held` (sent by the held method after accept) before inspecting or cancelling, so they are deterministic and do not mask the behavior. A follow-up is needed for the todo 14 owner.
- **Hello counter form (todo 13 code; not fixed here).** The hello result carries `"event_replay_events":10000` as a JSON number, against the decimal-string rule. It is visible in the transcripts.
- **`terminal.output` has no `total` field.** Its result has `next_offset`/`floor_offset`/`gap` but no explicit total, which IDEAL-007 asks for; the pagination scenario therefore ends on an empty page. This is a schema follow-up for the protocol owner.
- **Windows.** The runner compiles and lints clean for Windows. Its fixture listener is Unix-only, so the modules that exist only to serve it (`fixture::{documents, handler, output, serve, terminals}`) carry the same module-level `#[cfg(unix)]` as their caller. Off Unix, the fixture mode returns `RunnerError::Fixture { stage: "bind a listener", source: Unsupported }`, and a `socket` connection returns `unsupported_platform`; both are already-settled futures, not empty `async fn`s. The scenarios declare linux and macos only, and the process tests are `#![cfg(unix)]`. A named-pipe fixture transport belongs with todo 54. There are no lint attributes.
- **Process tests need the built binary.** They need `target/debug/ovis-terminal` (or `$OVIS_TERMINAL_BINARY`). A workspace test run builds it; a missing binary fails the test, it is never skipped.
