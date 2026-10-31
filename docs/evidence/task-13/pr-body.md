## Fix after CI round 1 (macOS)

`test (macos-latest)` failed on 4e7ecaf: `endpoint::a_live_socket_is_never_taken_over` got `Os { code: 32, kind: BrokenPipe }` from the test client's own write.

Cause, in the product: the contested bind's `reclaim_stale` connects to the live server and drops the stream at once. When the server accepted that connection, macOS `getpeereid` on the already-closed peer failed. `UnixEndpoint::accept` returned that as an `EndpointError`, the accept loop ended the whole server, and the client queued in the backlog then hit EPIPE. One vanished peer stopped the server.

Fixes:

- **Per-connection evidence.** `UnixEndpoint::accept` returns `(stream, Result<PeerEvidence, EndpointError>)`. A peer that closed before its credential could be read is `EndpointError::PeerClosed` (kind `peer_closed`) for that connection only. The server refuses it fail-closed (`peer_rejected` from the authz seam), counts `peer_closed` in `ServeReport::failures`, and keeps serving. Only a listener-level accept failure stops the server.
- **Probe hang-up.** `Probe::observe` reports a server that hangs up mid-hello (EOF, EPIPE, ECONNRESET, ECONNABORTED, ENOTCONN) as `Unreachable`, never as an unhandled `io::Error`. New test: `endpoint::a_server_that_hangs_up_mid_hello_is_unreachable`.
- **SIGPIPE.** `SO_NOSIGPIPE` is set on Apple targets through `socket2::SockRef::set_nosigpipe` (safe API; socket2 0.6.5 was already in the lockfile, now with the `all` feature on Apple targets only). It is applied at the one accept point (`UnixEndpoint::accept`) and the one connect point (`listen::unix::connect`, used by the probe and the stale-path check). Linux has no per-socket flag and tokio writes use `write(2)`, so `MSG_NOSIGNAL` is not available there; Rust programs start with `SIGPIPE` ignored, so writes return EPIPE.
- **Deterministic test.** `a_live_socket_is_never_taken_over` now waits on a real readiness signal: the owner connection completes hello before the second bind. Afterwards the established connection still answers a request, a new client still negotiates, and `ServeReport` shows `accepted=3, panicked=0`. There is no sleep, and no `cfg` skips macOS (the test file is `#![cfg(unix)]`).
- **Local cross-checks.** `cargo clippy -p ovis-terminal-server --all-targets --target aarch64-apple-darwin -- -D warnings` passes. The local Windows cross-check is blocked by the bundled-SQLite C build under the zig wrapper; the windows-latest CI lane covers it.
- **Gate.** `bash scripts/check.sh` exits 0: 326 passed, 1 skipped (`docs/evidence/task-13/check-fix.log`).

## Before

RT-HD-01 (EXCL-014, V-RT-HD-01): `docs/evidence/baseline/smoke/herdr-h02-connection.log:2` (original `/tmp/ulw-ovis-yCJzPH/evidence/smoke/herdr-h02-connection.log:2`):

```text
frames received on one connection: 1 closedByServer=true ms=0
```

herdr answers one frame and closes the connection, so a second request on it gets no answer. GAP-001, GAP-002, GAP-003, GAP-004, GAP-005, GAP-017 and GAP-120 had no implementation in ovis-terminal (`crates/ovis-terminal-server/src/{conn,listen}.rs` were one-line scaffolds).

## After

`crates/ovis-terminal-server` now serves authenticated persistent endpoints:

- **Descriptors (IDEAL-001, GAP-001).** `listen::EndpointDescriptor::resolve` takes explicit `EndpointEnvironment` values and never touches the filesystem. It returns `{kind, address, session, source, env_override}`.
  - Unix: `OVIS_TERMINAL_SOCKET` (absolute), else `$XDG_RUNTIME_DIR` / `$XDG_STATE_HOME` / `$HOME/.local/state` + `ovis-terminal/<session>.sock`. Paths over 103 bytes are refused (macOS `sun_path`).
  - Windows: `\\.\pipe\ovis-terminal-<USERNAME>-<session>`, or an override that must be a local `\\.\pipe\` name.
- **Safe Unix binding (IDEAL-102, GAP-017).** `UnixEndpoint::bind` creates the directory 0700 and the socket 0600.
  - It refuses a live socket (`already_running`), never replaces a non-socket file (`not_a_socket`), and replaces only a socket whose connect is refused.
  - A directory reachable by other accounts is `endpoint_insecure`, and the socket it just bound is removed.
  - Every accept collects `SO_PEERCRED` plus socket/directory stat and feeds `ovis_terminal_authz::EndpointPolicy::admit`.
- **Hello (IDEAL-002, IDEAL-003, GAP-002, GAP-003).** A sans-IO `ConnectionCore` answers hello before any request reaches a handler.
  - Order of checks: closed `HelloParams` (decimal-string generation, numeric is `invalid_params`), then the generation (`unsupported_generation`), then `Authority::authenticate`.
  - The result carries live `machine_id`, `runtime_lifetime`, `server_boot_id`, generation, the handler's catalog-checked methods, capabilities and `Limits::v1()`, so features are negotiated rather than guessed per platform.
  - Any other method before a successful hello is `unauthenticated` and is never dispatched. A refused peer gets its typed refusal, then `$/closing`.
  - `framed: true` upgrades the decoder only after a successful hello result is queued. Bytes already buffered behind the hello line are parsed as frames.
- **Persistent multiplexed connections (IDEAL-004, IDEAL-005, GAP-004, GAP-005).**
  - Concurrent requests run in a per-connection `JoinSet`, bounded by `limits.max_in_flight_requests` (64). Overflow gets a typed retryable `backpressure` reply.
  - Each request id is settled exactly once through `InFlightRequests`; a duplicate in-flight id is `duplicate_request_id`.
  - Handlers queue notifications through `Notifier` on the same connection.
  - Connections are bounded per server, joined on shutdown (grace, then abort+join), and closed with `$/closing{reason, hint}`. No detached task or thread (V-220).
  - `conn::Connections::serve(stream, admission)` is transport-generic, so later listeners reuse it.
- **Status without side effects (IDEAL-014, EXCL-027, GAP-017).** `listen::Probe::observe` reports `running` (with the live hello identity), `stopped`, `unreachable` or `stale_endpoint`. It only stats, connects and sends one hello.
- **Restart proof (GAP-120).** `listen::EndpointIdentity::continuity` classifies a reconnect as `SameBoot`, `ControlRestarted` (same `runtime_lifetime`, new `server_boot_id`), `NewLifetime` or `OtherMachine`.

QA happy, Linux x86_64, rustc 1.99.1 (`docs/evidence/task-13/qa-happy.log`):

```text
$ cargo test -p ovis-terminal-server --test connection happy -- --exact --nocapture
<- {"id":"hello","jsonrpc":"2.0","result":{"capabilities":{"framed":true,"stream_modes":[],"transports":["unix"]},"endpoint_kind":"unix","generation":"1","limits":{...},...,"runtime_lifetime":"runtime:7","server_boot_id":"boot:1",...}}
<- {"id":"s1","jsonrpc":"2.0","result":{"subscription_id":"sub-1"}}
<- {"id":"b","jsonrpc":"2.0","result":{"name":"b"}}
<- {"id":"a","jsonrpc":"2.0","result":{"name":"a"}}
<- {"jsonrpc":"2.0","method":"event","params":{"seq":"1","subscription_id":"sub-1","type":"doc.changed"}}
<- {"id":"m","jsonrpc":"2.0","result":{"version":"1"}}
<- {"id":"hello",...}   (reconnect, same server)
<- {"id":"hello",...}   (reconnect after restart as boot 2)
test happy ... ok
EXIT=0
```

What `happy` asserts, all over one framed connection:

- The subscription, two concurrent `doc.get` calls and a `doc.set` all return matching ids.
- `a` blocks until `b` runs, so `b` settles first; only concurrent dispatch can satisfy this.
- The `doc.set` event arrives with the subscription's id.
- A reconnect reports `Continuity::SameBoot`, and the probe sees `Running` with the same identity.
- `ServeReport` is `accepted=3, panicked=0, aborted=0, failures={}`, and after shutdown the probe reports `stopped`.
- A server restarted as boot 2 reports the same `runtime_lifetime`, a new `server_boot_id` and `Continuity::ControlRestarted`.

QA failure (`docs/evidence/task-13/qa-failure.log`):

```text
$ cargo test -p ovis-terminal-server --test connection failure -- --exact --nocapture
<- {"error":{"code":-32000,"data":{"hint":"reconnect with a client built for this runtime's protocol generation","kind":"unsupported_generation","retryable":false},"message":"operation failed"},"id":"hello","jsonrpc":"2.0"}
<- {"error":{"code":-32000,"data":{"hint":"send hello first and wait for its result","kind":"unauthenticated","retryable":false},"message":"operation failed"},"id":"m1","jsonrpc":"2.0"}
test failure ... ok
EXIT=0
```

What `failure` asserts:

- Hello generation `"999"` returns `unsupported_generation` with no result.
- A following `doc.set` is `unauthenticated`, with the handler's call and mutation counts both 0.
- A socket path nobody listens on is `stale_endpoint`; `pgrep -P <test pid>` counts the same before and after the probe, and the stale file is left in place.
- Mutation check: with the generation check disabled, `failure` fails with exit 101 (`docs/evidence/task-13/mutation-generation.log`).

DEFERRED VERIFICATION (runner lands in Todo 21). Per plan line 275, real-binary conformance for part-A foundational todos waits for the runner and binary assembly. The required command was run and exits 2:

```text
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-13 --isolated --case happy --evidence "$ATTEMPT_DIR/task-13-ovis-terminal-v1.json"
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
EXIT=2
```

`conformance/scenarios/part-a/task-13.json` lists the component steps and the adversarial cases for that later run.

Gates on the tree merged with `origin/integration` (97c8d00):

- `bash scripts/check.sh` exits 0, with nextest at 325 passed, 1 skipped (`check-merged.log`).
- `cargo nextest run -p ovis-terminal-server`: 26 passed (after the macOS fix) (`nextest.log`).
- `cargo clippy -p ovis-terminal-server --all-targets -- -D warnings`: exit 0 (`clippy.log`).
- `openrpc check` and `openrpc compat schema/released.openrpc.json` both exit 0 (`schema-gate.log`).

## QA

- happy: `cargo test -p ovis-terminal-server --test connection happy -- --exact --nocapture`, exit 0, 1 passed.
- failure: `cargo test -p ovis-terminal-server --test connection failure -- --exact --nocapture`, exit 0, 1 passed.

## Adversarial

Logs: `docs/evidence/task-13/adversarial.log` (12 integration + 5 core unit tests, all pass).

- **Malformed input.**
  - `generation_is_an_exact_decimal_string`: `"18446744073709551615"` (u64::MAX) decodes exactly and is `unsupported_generation`. `"18446744073709551616"`, numeric `1` and `"01"` are `invalid_params`.
  - `unknown_frame_kind_after_upgrade_closes_with_its_kind`: `$/closing{reason:"unknown_kind"}` and EOF; the report counts `unknown_kind: 1`.
  - `correlatable_bad_requests_are_answered_and_the_rest_close`: a bad envelope with a string id gets `invalid_request` with that id and the connection stays open; `not json` closes with `parse_error`.
  - `a_second_hello_and_a_duplicate_in_flight_id_are_refused`.
- **Stale state.**
  - `failure` (stale socket); `a_stale_socket_is_replaced_and_a_plain_file_never_is` (a marker file is `stale_endpoint`, never replaced or modified); `a_live_socket_is_never_taken_over`.
  - `a_socket_that_never_answers_hello_is_unreachable` uses paused virtual time, not a real wait.
  - `credentials_are_checked_before_hello_exposes_identity`: absent → `unauthenticated`, forged → `invalid_credential`, expired → `credential_expired`, and no result in any case.
  - Stale boot: `happy` shows `ControlRestarted` after a restart.
- **Cancel/resume.**
  - `framed_upgrade_parses_identically_under_every_split` (proptest): hello + two frames split at arbitrary byte boundaries always yield `ok:h, upgrade, dispatch:r1, dispatch:r2`.
  - `refused_generation_does_not_upgrade_the_bytes_behind_it`; `partial_hello_establishes_nothing_until_its_line_completes`.
  - `requests_beyond_the_in_flight_bound_get_backpressure_and_cancel_on_shutdown`: all 64 held handler futures are dropped (counted on drop) when the server shuts down, then the client reads `$/closing{reason:"server_shutdown"}` and EOF.

## Contract checklist

1. **Decimal strings.** u64 values on the wire are decimal strings: `generation`, `server_time_ms`, the limits' byte counts, and the endpoint ids (`runtime:7`, `boot:1`). A numeric generation is `invalid_params`, and a u64::MAX exact round trip is tested. The event `seq` in the test fake is a string too.
2. **Error enums.** Boundary errors are `thiserror` enums with `kind()`, `hint()` and `#[source]` where an inner error exists: `EndpointError`, `ConnectionError`, `HelloError`, `ConnectionClosed`. `Display` never repeats the kind token. There are no hand-written `Error` impls.
3. **Distinct identity types.** Identities stay distinct types: registry `MachineId`/`RuntimeId`/`ServerBootId` server-side, protocol id newtypes on the wire, `SessionName`, `PrincipalId` via `Session`.
4. **Behavior tests.** Tests assert behavior: typed kinds, ids and ordering on a real socket, cancellation counts, fragmentation invariance (proptest) and descriptor rules (proptest). Nothing reads back a literal it just assigned.
5. **No suppressions.** There is no `#[allow]` or `#[expect]`, no TODO, and no new third-party dependency (`tokio` `test-util` is dev-only).
6. **Deferred conformance.** The conformance run is DEFERRED-BY-PLAN, with the exit-2 attempt recorded above.

## Seams

- **`ovis-terminal-protocol` (todo 4) gains five error kinds.** `DomainErrorKind` adds `credential_revoked`, `decision_mismatch`, `grant_exceeds_issuer`, `endpoint_insecure` and `peer_rejected` (the authz kinds from PR #16). `hello` additionally lists `credential_revoked`, `peer_rejected` and `endpoint_insecure`.
  - `schema/ovis-terminal.openrpc.json` is regenerated, and `openrpc compat schema/released.openrpc.json` passes (additive only).
  - `endpoint::every_kind_the_hello_boundary_emits_is_published` checks every emitted kind against the catalog.
- **`ovis-terminal-protocol` (todo 3) gains `BoundaryError::with_hint(&'static str)`.** It replaces only the wire `data.hint`; kind and code are unchanged. Covered by `rpc_errors::boundary_hint_replaces_only_the_wire_hint`.
- **`ovis-terminal-server` adds the `conn::RequestHandler` seam.** It carries `Call {request, session, notifier}`. Todo 14 owns real dispatch; tests use a recording fake handler for the methods `doc.get`, `doc.set` and `events.subscribe`.

## Risks

- **Windows.** This PR ships the pipe descriptor (name, override, user/session naming) but no named-pipe listener: creating a pipe with an owner-only DACL and collecting its `PipeInstanceEvidence` needs Win32 security calls. That is todo 54 (Serve named pipes with Windows service isolation), which can hand accepted pipes to `conn::Connections::serve`. On Windows the crate compiles the descriptor, continuity and connection core; the Unix listener and probe are `cfg(unix)`.
- **Credential delivery.** Nothing here writes the owner bearer for local clients; assembly (todo 31) decides where the bootstrapped credential lives. `Probe` therefore requires a bearer to report a live identity.
- **Interim handler.** Until todo 14 lands, nothing registers production handlers, so a running server advertises only what its handler serves.
- **Notification queue.** The outbound queue is 256 records per connection. A peer that never reads stalls its own reader, and the server's shutdown grace (5 s) then aborts it. The 64 MiB unwritten-bytes bound from the spec is not yet enforced separately.
- **Peer hang-up.** BrokenPipe, ConnectionReset, ConnectionAborted and NotConnected after a peer hangs up are treated as a normal close, not a server failure.
