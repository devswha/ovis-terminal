feat(ws): expose authenticated websocket transport with parity — plan todo 28.
Closes GAP-023, GAP-129, GAP-130. Delivers IDEAL-001, IDEAL-004, IDEAL-020, IDEAL-034, IDEAL-102, IDEAL-103.

`ovis_terminal_server::ws` serves any `conn::RequestHandler` over `ws://`. A WebSocket is one more byte stream for the unchanged `conn::Connections` core: the payloads of the client's binary messages are concatenated in order and decoded by the same `ConnectionDecoder`, so hello, the credential check (`hello.credential`), limits, NDJSON/framed modes, dispatch, the 16 KiB receipt cap, attach streams and `$/closing` are the local socket's. Every record the server writes (one NDJSON line, or one frame after a framed hello) leaves as exactly one binary message. No dispatch path is forked and no browser-specific semantics exist.

- `ws/upgrade.rs`: the RFC 6455 opening handshake in-tree (`httparse` + `sha1` 0.10), bounded to an 8 KiB head and a configurable deadline (default 10 s). Refusals are HTTP 400/403/426/431 with a typed JSON body `{kind, hint}` that echoes nothing from the request (no origin, no `Authorization`).
- `ws/origin.rs`: `AllowedOrigins` — a request with no `Origin` header is a non-browser client and authenticates with `hello.credential` like any socket peer; a browser `Origin` must be configured exactly (default: none). Repeated or non-UTF-8 `Origin` is refused.
- `ws/frame.rs`: RFC 6455 framing — masked client frames, fragmentation, ping→pong on the transport, close echo; text frames, unmasked frames, reserved bits, unknown opcodes, malformed control frames and frames declaring more than one maximal protocol frame (16 MiB + 9) are refused before their payload is buffered.
- `ws/pump.rs` + `ws/transport.rs`: one bounded message in each direction between the connection and the socket, read and write concurrent; a violation closes with code 1002/1003/1009 and the typed kind as the reason; a peer that leaves is a clean close (as on a socket).
- `ws/binding.rs`: `WsBinding::default()` is `127.0.0.1:0`; any other address needs `WsBinding::exposed`.
- `ws/server.rs` + `ws/handle.rs`: bounded accepts (64 default), every connection task in a `JoinSet`, shutdown closes with `$/closing`, joins, and aborts past the grace; `WsReport` counts failures by kind. No logging, no detached tasks (V-220, V-235).

## Before

absent (GAP-023, GAP-129, GAP-130): `crates/ovis-terminal-server/src/ws.rs` was the one-line scaffold `//! Authenticated WebSocket transport with protocol parity (plan todo 28).`; no comparable new-runtime capability is claimed as a legacy execution.

## After

No served `ovis-terminal` binary binds a WebSocket endpoint until todo 31 assembles the server, so the real-surface transcript is the integration test driving the real server crate (authority, SQLite coordinator, lease table, screen, attach lane) over a real TCP WebSocket and a real private Unix socket at once (`docs/evidence/task-28/ws-socket.log`, Linux 7.0.0-38-generic x86_64):

```
$ cargo nextest run -p ovis-terminal-server --locked --test ws --no-capture
<- {"jsonrpc":"2.0","id":"hello","result":{...,"endpoint_kind":"websocket","generation":"1","limits":{"connection_unwritten_bytes":"67108864",...,"heartbeat_ms":"15000",...
<- {"jsonrpc":"2.0","id":"hello","result":{...,"endpoint_kind":"unix","generation":"1","limits":{"connection_unwritten_bytes":"67108864",...
<- {"jsonrpc":"2.0","id":"hello","error":{"code":-32000,"message":"operation failed","data":{"kind":"credential_expired","hint":"request a new credential from an administrator","retryable":false}}}
        PASS (1/3) ovis-terminal-server::ws failure
        PASS (2/3) ovis-terminal-server::ws happy
        PASS (3/3) ovis-terminal-server::ws transport_violations_close_with_typed_reasons
     Summary 3 tests run: 3 passed, 0 skipped
exit=0
```

What the tests assert (and nothing more):
- `happy`: hello over WebSocket equals hello over the Unix socket except `endpoint_kind` (`websocket` vs `unix`) and `server_time_ms`; `heartbeat_ms`, `event_replay_events`, `connection_unwritten_bytes` are decimal strings. A WebSocket ping is answered with a pong of the same payload and the next record is the reply to the next request. Two WebSocket viewers (owner/interact, phone/observe) and one Unix viewer (bridge/observe) get `ready` with identical `first_paint`; after output (including a cursor query split across reads) each viewer receives contiguous offsets, `attach.repaint` at the same stream position answers an identical checkpoint on both transports, a dropped-and-reconnected WebSocket viewer attaches with a fresh `stream_id` at `first_paint.output_offset == raw end`; on exit every viewer closes `exited` with `final_output_offset == raw end` and `final_offset == bytes received`, its VT equals a VT fed every raw byte, and the WebSocket interact viewer's received bytes equal the Unix viewer's byte for byte. `WsReport.failures` is empty.
- `failure`: an expired credential gets `credential_expired` at hello and a following `attach.open` gets `unauthenticated`, identically on both transports, and neither reply contains the bearer; an observe-only credential asking for `interact` is `refused`/`forbidden` with `stream_id: null` and `lease: null` (identical on both); a client DATA frame on stream channel 1 after a framed hello yields exactly one control-channel `$/closing {reason: "unowned_channel"}` record, identical on both transports, and over WebSocket that record is exactly one binary message followed by close 1000 and the end of TCP.
- `transport_violations_close_with_typed_reasons`: a request sent as a text message is never answered and gets close 1003 `invalid_ws_message`; an unmasked frame gets 1002 `ws_protocol`; a header declaring 16 MiB + 10 bytes gets 1009 `message_too_large`; a request split over a fragmented binary message with a ping between its fragments is answered once and the ping gets its pong; `Origin: https://evil.example` gets HTTP 403 with body `kind: origin_rejected` echoing nothing; the configured origin upgrades and authenticates; a TCP peer that never sends the upgrade gets no bytes and is counted `handshake_timeout`; `WsReport` counts each kind exactly once.

Interop with an independent client (Bun 1.4.3's built-in `WebSocket`, scratch test not committed, `docs/evidence/task-28/bun-interop.log`): no Origin → hello, `attach.open`, checkpoint, `attach.data`, client close 1000; configured Origin → same; text message → close `{code: 1003, reason: "invalid_ws_message"}`; foreign Origin → `Expected 101 status code`; server report `{"invalid_ws_message": 1, "origin_rejected": 1}`; no output contained the bearer.

DEFERRED VERIFICATION (runner lands in Todo 21; it landed, but it cannot reach this surface): the todo-21 runner speaks only the Unix endpoint through the `ovis-terminal` bridge, its fixture opens no WebSocket listener and registers no attach routes, and no served binary binds a WebSocket endpoint until todo 31. `conformance/scenarios/part-a/task-28.json` is therefore a declaration, allowlisted in `declared.json` (owner 28, blocked on 31). Attempts, verbatim:

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-28 --isolated --case happy --evidence "$ATTEMPT_DIR/task-28-ovis-terminal-v1.json"
{"case":"happy","error":{"detail":"part-a/task-28 is a declaration, not a registered executable scenario","hint":"register the scenario with \"format\": \"ovis-conformance/1\" and typed steps","kind":"scenario_not_executable","step":null},...}
exit=1
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-28 --isolated --case failure --evidence "$ATTEMPT_DIR/task-28-ovis-terminal-v1.json"
{"case":"failure","error":{"detail":"part-a/task-28 is a declaration, ...","kind":"scenario_not_executable","step":null},...}
exit=1
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario all/all --isolated --case all --evidence ...
{"case":"all","error":null,...,"result":"passed","scenario":"all/all","totals":{"assertions":"685","cases":"24","declared_only":"32","failed":"0","passed":"24","skipped":"0"}}
exit=0
```

## QA

- `bash scripts/check.sh` → `check.sh: all steps passed` (fmt, clippy `-D warnings`, nextest 833 passed / 1 skipped, doctests, hygiene, manifests, json) — `docs/evidence/task-28/check.log`.
- `cargo clippy -p ovis-terminal-server --all-targets -- -D warnings` → exit 0; `cargo nextest run -p ovis-terminal-server` covered by the workspace run.
- `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` → exit 0 (`windows-check.log`); the ws module has no platform cfg, `tests/ws.rs` is `#![cfg(unix)]` like `tests/streams.rs` (the parity fixture needs a Unix socket).
- Unit: 17 `ws::` tests (`ws-unit.log`) including proptests — frames decode identically under every split, the upgrade head is found under every split with pipelined frame bytes handed on, inbound payloads read back as one stream under every read size, an origin round-trips through its text; RFC 6455 vectors (sample key → `s3pPLMBiTxaQ9kYGzzhZRbK+xOo=`, masked "Hello" sample, 16/64-bit length encodings, close payload bytes).
- Stress: the 3 integration tests 60/60 sequential runs and 30/30 pinned to 2 CPUs (`taskset -c 0-1`). One earlier run exposed a test race (repaint checkpoint `stream_offset` depends on delivery progress); the test now waits until every viewer received the last chunk before replaying.
- macOS: workflow option `websocket` added at the end of `probe-stress-macos.yml` (25 iterations of `--test ws` + `ws::` unit tests); run link below.

## Adversarial

- malformed input — invalid WS message: text frame (1003), unmasked frame (1002), reserved bits / unknown opcode / fragmented or >125-byte control / continuation without a message / interleaved data / one-byte close (unit, `ws_protocol`), oversized declared length (1009, refused before payload), malformed/TLS-garbage/POST/missing-key/wrong-version/oversized upgrade heads (400/426/431).
- stale state — expired credential: `credential_expired` at hello, then `unauthenticated` for `attach.open` on the same connection, identical to the Unix socket.
- cancel/resume — reconnect cursor: a dropped WebSocket viewer reconnects and attaches with a fresh stream id at the current output head; a peer that vanishes mid-stream ends as a clean close (no `connection_io` failure), which the first stress run caught and the pump now guarantees.
- prompt injection — no diagnostic parsing: refusal text in output stays data (VT equality); close reasons and HTTP bodies are machine tokens only; nothing parses or echoes request text; refusals echo neither origin nor `Authorization`.

## Contract checklist

1. Counters on the wire are unchanged protocol records (decimal strings asserted for `heartbeat_ms`, `event_replay_events`, `connection_unwritten_bytes`); the ws layer adds no JSON counters. Frame lengths are binary RFC 6455 fields.
2. `WsError` is one `thiserror` enum with `kind()`/`hint()`, `#[source]` on inner errors, `FrameDefect`/`UpgradeDefect` typed; a test asserts Display never repeats the kind token.
3. `Origin`, `WsBinding`, `WsEndpoint` are distinct types; no new identities were introduced (sessions, principals, streams reuse authz/protocol types).
4. Tests assert behavior on real sockets; proptest covers frame, upgrade and stream fragmentation.
5. No `#[allow]`, no blanket `#[expect]`, no TODOs; every file ≤ 250 pure LOC, ≤ 3 parameters.
6. Holder wire generation stays 2; checklist items unchanged.

## Seams

- New public API: `ovis_terminal_server::ws::{serve_websocket, WsConfig, WsBinding, WsEndpoint, WsServerHandle, WsReport, AllowedOrigins, Origin, WsError, FrameDefect, UpgradeDefect}`. No landed crate's public API changed.
- Dependencies (workspace `Cargo.toml` + server manifest): `httparse 1.10` (no dependencies) and `sha1 0.10` (shares `digest 0.10` / `cpufeatures 0.2` with `sha2`). tokio-tungstenite was tried and rejected: 0.30 pulls `sha1 0.11`/`digest 0.11`, and even without its handshake feature `rand 0.10`→`chacha20`→`cpufeatures 0.3`; 0.29 duplicates `rand`/`getrandom` — each fails `clippy::multiple_crate_versions`.
- `conformance/scenarios/declared.json`: one entry for `part-a/task-28.json` in sorted position.
- `.github/workflows/probe-stress-macos.yml`: option `websocket` appended at the end of the list and one job appended at the end.

## Risks

- `hello.capabilities.transports` still advertises `["unix"]` even over WebSocket (`endpoint_kind` is correct): it is built in `conn/identity.rs` (todo 13 scope) and should list the listeners the assembled server runs (todo 31).
- `direction_violation` for upstream DATA is not produced on any transport: `conn` routes no stream channels (`attach.open` with `stream_mode: channel` answers `needs_framed`), so a DATA frame on channel 1 closes the connection with `unowned_channel` on both transports and nothing reaches a program. Routing `StreamChannels::upstream` is todo 31's.
- There is no `input.*` or `attach.resize` route yet (todos 25/31), so "observer input/resize forbidden" is proven only as `attach.open{interact}` → `refused/forbidden` with no lease.
- TLS is not terminated here; non-loopback exposure requires `WsBinding::exposed` and a TLS-terminating proxy.
- Pre-existing flake seen once during `check.sh`, outside this diff: `ovis-terminal-core::subscriptions stall::a_stalled_fanout_worker_loses_every_subscription_and_each_recovers` (todo 22) failed with `Missed(Recovery {cause: FanoutOverflow})`; 20/20 isolated reruns passed (`check-run2-core-flake.log`).
- origin/integration moved to `436506e` (todos 78, 79) after this lane was cut; not merged here per the lane rule.
