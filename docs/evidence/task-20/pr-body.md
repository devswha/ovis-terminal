feat(cli): ship explicit json cli and side effect free discovery (plan todo 20)

Closes GAP-016, GAP-137, GAP-138, GAP-141. Delivers IDEAL-014, IDEAL-110, IDEAL-111, IDEAL-114.

## Before

RT-HC-04: after `hc server stop`, a read-only `hc server status` started a new core.
`docs/evidence/baseline/smoke/hc-s18-stop-cleanup.log:11` (original `/tmp/ulw-ovis-yCJzPH/evidence/smoke/hc-s18-stop-cleanup.log:11`):

```
{"core":{"id":"dcb5ded90ae69879514cb21f0b30c7fe","name":"<user>-thinkstation-p620"},"finished":1,"holder":null,"holders":[],"installed_by":null,"live":0,"pid":2678504,"uptime_ms":9,"version":"0.1.14"}
```

A new pid with `uptime_ms":9` appears after a status read. GAP-137: hc chose JSON vs text by isatty. GAP-138/141: herdr could fall back to the focused pane, and it hides flags.

## After

The real binary, Linux 7.0.0-38-generic x86_64, rustc 1.99.1, run in a fresh isolated `XDG_RUNTIME_DIR` with `env -i` (full transcript: `docs/evidence/task-20/qa-transcript.log`):

```
$ ovis-terminal --json server status          (run twice)
stdout: {"holder_count":0,"live_terminals":0,"runtime_lifetime":null,"server_boot_id":null,"state":"stopped","uptime_ms":null}
stderr: (empty)   exit: 0
files under isolated root after status: 0

$ ovis-terminal --json schema | sha256sum      exit: 0
6fab81ee2026cf83eeba1f830aa82863feb1edce206e19a8136ce0cbc3ad3e96  (== sha256 of schema/ovis-terminal.openrpc.json)

$ ovis-terminal --json --skill                 exit: 0
{"generation":"1","manual":"# ovis-terminal agent manual ...","version":"0.1.0"}

$ ovis-terminal --json terminal get            exit: 7   (no server)
stderr: {"error":{"candidates":[],"candidates_listed":false,"exit_code":7,"kind":"needs_target",...,"origin":"cli"}}

$ ovis-terminal --json --unknown-flag server status   exit: 2
stderr: {"error":{"exit_code":2,"kind":"usage","message":"the command line does not parse: unexpected argument '--unknown-flag' found",...}}
```

The transcript also has a `pgrep -f ovis-terminal` before/after line (7 -> 2). It is **not** evidence: `pgrep -f` matched unrelated command lines, including the probe shell itself. The owned-process proof is the marker-scoped count in the `happy` test (below).

DEFERRED VERIFICATION (runner lands in Todo 21):

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-20 --isolated --case happy --evidence "$ATTEMPT_DIR/task-20-ovis-terminal-v1.json"
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
EXIT=2
```

`conformance/scenarios/part-a/task-20.json` records the expected fields and bytes for that later run (`verification.status = deferred_by_plan`).

## What ships

- `ovis-terminal` grammar (clap derive): `--json` (global; `OVIS_JSON=1` too, never isatty), `--skill`, `--version`, `schema [openrpc|cli]`, `endpoint`, `server status`, `terminal list`, `terminal get [--terminal-id|--address|--name]`, `op get|cancel --op-id`, `call METHOD --params JSON`, `connect` (stdio <-> endpoint byte relay).
- Parse-then-plan before any I/O: clap parse, then typed params decoded with the protocol's own `decode_params::<P>` (`TerminalGetParams`, `TerminalListParams`, `OpRefParams`, `NoParams`). `call` refuses `hello`, names outside `all_methods()` and non-object params.
- Success is one JSON document on stdout with empty stderr. Failure leaves stdout empty and writes `{"error":{origin,kind,hint,message,exit_code,...}}` to stderr. Server errors pass their `data` through with `origin:"server"`.
- Published exit codes (`ExitStatus`, also in `schema cli` and the manual): 0 success, 1 failure, 2 usage, 3 not_found, 4 unavailable, 7 needs_target, 9 blocked, 10 cancelled, 11 conflict, 12 forbidden, 13 outcome_unknown.
- `server status` classifies the endpoint from the same connect path every command uses. A missing endpoint is `stopped`, a refused one `stale_endpoint`, and one that accepts but never completes hello within 10 s, or closes, is `unreachable`. These are reported locally as a `ServerStatusResult`; only a server that completes hello is asked `server.status`. Nothing is ever created or started.
- One platform-neutral transport: a generic duplex NDJSON session over the Unix socket, or on Windows the named pipe `EndpointDescriptor` names (`tokio::net::windows::named_pipe::ClientOptions`). The only `cfg` is on `connect` itself.
- `terminal get` without a target lists `terminal.list` and refuses with `needs_target` + `TargetCandidate`s (runtime_lifetime from hello). It never picks one. With no server it returns `candidates: []`, `candidates_listed: false`.
- The bearer credential comes from `OVIS_TERMINAL_CREDENTIAL`, with a redacted Debug.
- `.github/workflows/probe-stress-macos.yml`: new `cli` choice that repeats `cargo nextest run -p ovis-terminal-cli` 100 times on macos-latest.

## QA

```
$ cargo test -p ovis-terminal-cli --test cli happy -- --exact --nocapture
test happy ... ok
test result: ok. 1 passed; 0 failed; ... EXIT=0
$ cargo test -p ovis-terminal-cli --test cli failure -- --exact --nocapture
test failure ... ok
test result: ok. 1 passed; 0 failed; ... EXIT=0
$ bash scripts/check.sh   -> "check.sh: all steps passed", exit 0 (447 workspace tests; log docs/evidence/task-20/check.log)
$ cargo nextest run -p ovis-terminal-cli -> 16 tests run: 16 passed
```

What the tests assert (and only this):
- `happy` (real binary, `env_clear`, isolated 0700 dir):
  - `--json server status` runs 5 times. Each run asserts exit 0, empty stderr, `state == "stopped"` and `holder_count == 0`, that the isolated dir's recursive entry list is unchanged, and that 0 live processes carry the child-only env marker (`/proc/*/environ` on Linux, `ps eww` elsewhere).
  - `--json schema` parses equal to `schema/ovis-terminal.openrpc.json`.
  - `--json --skill` returns `version == CARGO_PKG_VERSION` and a non-empty manual.
  - Against a real authenticated `serve_unix` fixture, status reports `running` with `uptime_ms == "18446744073709551615"`.
  - `connect` relays a hello line and the reply carries the fixture `machine_id`.
- `failure`:
  - Against the fixture with two terminals, `--json terminal get` exits 7 with empty stdout and `kind == needs_target`, `exit_code == 7`, `candidates_listed == true`. The candidate ids are `[term-a, term-b]` with the fixture `machine_id`.
  - Three malformed invocations (`--unknown-flag`, two targets plus an unknown flag, `call terminal.list --params [1]`, plus `call hello`) each exit 2 with empty stdout and a usage, invalid_params or unknown_method kind. A non-blocking `UnixListener` set as the endpoint then returns `WouldBlock` on `accept()`, so no connection was attempted.
  - With no server, a targetless get exits 7 with `candidates == []` and `candidates_listed == false`.
- Mutation checks (run locally, then reverted):
  - Making `server status` create the endpoint directory fails `happy` at the unchanged-entries assertion.
  - Making a missing socket not count as "not running" fails `failure` (exit 1 instead of 7).

## Adversarial

- Malformed input: unknown flag, two targets, non-object `--params`, `call hello`, and `OVIS_JSON=maybe`. All exit 2 and none dials the socket. While writing the tests, an `op get --op-id not-a-uuid` case **did** dial and time out (exit 13): `OpId` is an opaque string in the protocol, so it is not a parse-time refusal. It was replaced rather than claimed.
- Misleading success: every failure has empty stdout, every success has empty stderr, and `exit::tests` proves codes are distinct and each kind maps to exactly one status. An unknown future server kind exits 1, never 0.
- Repeated interruptions: 5 consecutive stopped-status reads leave no file, socket or process (no auto-start or cleanup loop).
- Precision: `uptime_ms` is relayed as an exact decimal string (proptest over all u64, plus an explicit u64::MAX case). A numeric uptime is `invalid_response`.

## Contract checklist

1. u64 on the wire stays a decimal string. The status result is decoded and encoded through `ServerStatusResult` (`DurationMs`); there is a u64::MAX round trip, and a numeric form is refused. The CLI accepts no u64 JSON input of its own; `exit_code` is a small published status code, not a counter.
2. `CliError` is a `thiserror` enum with static `kind()`/`hint()` and `#[source]` on inner errors (clap, serde_json, io, `ParamsError`, `EndpointError`, `BoundaryError`, `SchemaError`). Display never repeats the kind token. Server errors are relayed as the server's `RpcErrorObject`, not re-stringified.
3. Identities reuse the protocol newtypes (`TerminalId`, `MachineId`, `RuntimeLifetimeId`, `TargetCandidate`, `OpRefParams`). `Credential` is its own newtype.
4. Tests assert behavior through the real binary and real sockets, with a proptest for the u64 relay. The fixture server is a real `serve_unix` + authz handler; the behavior under test (the CLI) is not mocked.
5. No `#[allow]` or `#[expect]` added, and no TODOs. Text rendering is not a contract and its one test pins structure only.
6. Conformance is deferred-by-plan and recorded verbatim above. Holder wire generation is unchanged (2); this PR does not touch the holder.

## Seams

- Root `Cargo.toml`: new workspace dependency `clap = 4.5` (default-features off; `std, derive, help, usage, error-context, suggestions`; MIT/Apache-2.0). `crates/ovis-terminal-cli/Cargo.toml` gains clap, serde, serde_json, tokio, and dev-deps proptest, ovis-terminal-authz, ovis-terminal-core.
- `ovis-terminal-cli` public API changed. Removed: the scaffold `Invocation`, `UsageError`, `USAGE_EXIT_CODE`, `version_line`. Added: `run`, `Context`, `Credential`, `Report`, `ExitStatus`, `JSON_ENV`, `CREDENTIAL_ENV`. No other landed crate's API changed.
- `.github/workflows/probe-stress-macos.yml`: new `cli` input and job; the existing `stress` job now gates on `probe == 'platform'`.

## Risks

- `serve` and `hold` are **not** in this PR's grammar. They need the runtime assembly that todo 31 owns (`src/assembly/`, binding core/store/holder/authz). Shipping them as typed "not assembled" stubs would advertise commands this build cannot perform. `connect` is implemented for real.
- No credential file convention exists yet. The bearer comes only from `OVIS_TERMINAL_CREDENTIAL`, and a running server without one answers `unauthenticated` (exit 12) to `server status`.
- Windows connects over the named pipe, but only compile- and clippy-checked: locally with `--target x86_64-pc-windows-msvc` (zig cc with MinGW headers for bundled SQLite, check only; no link) and in CI. The process tests are `cfg(unix)`, so the Windows pipe path has no runtime test in this PR. `ERROR_PIPE_BUSY` is reported as `transport_failed`, without retry.
- `server status` no longer reuses `listen::Probe` (it is Unix-only). The connect-path classification replaces it on both platforms.
- `call` checks the method name and the object shape only; per-method typing of generic `call` params is the server's job (named verbs are typed locally).
- Worktree: the brief named `lane-t76`, which is occupied by a live lane (`lane/t24-input-geometry-leases`). This work used `/home/devswha/workspace/ovis-terminal-wt/lane-t20` on the brief's branch `lane/t20-json-cli-discovery`.

## Follow-up commit: keep the credential and transport seam live on every target

CI run 41194283701 job `test (windows-latest)` failed to compile `ovis-terminal-cli` under `-D warnings`: seven dead-code errors and one `unused_async`. The transport, credential and `Failure::Server` paths were platform-neutral, but their only users sat behind `cfg(unix)`. The fix makes the transport itself platform-neutral (named-pipe client on Windows) instead of silencing anything:

- No `#[allow]`/`#[expect]` added.
- `cargo clippy -p ovis-terminal-cli --all-targets --all-features --locked --target x86_64-pc-windows-msvc -- -D warnings` -> Finished (local, check-only).
- Linux clippy -> Finished; `cargo nextest run -p ovis-terminal-cli` -> 16 passed; `bash scripts/check.sh` -> exit 0 (log `docs/evidence/task-20/check.log`).

## Follow-up commit: Darwin socket-path limit in the CLI tests

CI on 8e2e122 failed `test (macos-latest)`: `happy` got exit 2 instead of 0 (cli.rs:22) and `failure` got 2 instead of 7 (cli.rs:133), with no explanation in the log.

Cause: the tests' isolated root sat under `std::env::temp_dir()`. On Darwin that is `/var/folders/<..>/T/`, and the resolved default endpoint `<root>/ovis-terminal/default.sock` reaches about 106 bytes, past the 103-byte `MAX_UNIX_SOCKET_PATH`. `EndpointDescriptor::resolve` then correctly refused with `socket_path_too_long`, which the CLI maps to exit 2. That is the product behaving as specified on a test environment that was wrong. Reproduced locally with a 112-byte path: `{"error":{"exit_code":2,"kind":"socket_path_too_long",...}}`, exit 2.

Fix (tests only, no product change):
- Isolated roots now live at `/tmp/ovt20-<pid>-<n>`.
- Every exit-code check goes through `Ran::exit(code)`. On a mismatch it fails with the argv, stdout and stderr; this was verified by forcing a wrong expectation locally, and the error printed the typed stderr.
- `failure` now also asserts the real long-path case: `XDG_RUNTIME_DIR=<root>/d*100` gives exit 2, empty stdout and `kind == socket_path_too_long`.
- `bash scripts/check.sh` -> exit 0; `cargo nextest run -p ovis-terminal-cli` -> 16 passed.
