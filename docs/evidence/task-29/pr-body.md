feat(client): generate typescript sdk and replay neutral fixtures (todo 29)

Closes GAP-005, GAP-018, GAP-019, GAP-041, GAP-060, GAP-078. Delivers IDEAL-004, 005, 006, 019, 028, 040, 052, 055, 112, 113 at the client/SDK level (see Risks for what is scripted-peer only until todo 31 routes attach/events/input).

## Before

RT-HD-01 / V-RT-HD-01, GAP-005: `docs/evidence/baseline/smoke/herdr-h02-connection.log:2` (original `/tmp/ulw-ovis-yCJzPH/evidence/smoke/herdr-h02-connection.log:2`):

```
frames received on one connection: 1 closedByServer=true ms=0
```

Ovis opened one socket per RPC (GAP-005), widened enums by hand (GAP-018), renamed colliding schema sections (GAP-019), inferred readiness from bytes (GAP-041), validated acknowledgements itself (GAP-060) and kept drafts without a contract (GAP-078).

## After

Host: `Linux x86_64` (`uname -sm`), bun 1.4.3, toolchain 1.99.1. Base: origin/integration 804e508 (integration has since moved to b96f583; not merged here, see Seams).

Real binary, real socket (the `ovis-terminal connect` bridge to the real server-crate fixture):

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal \
    --scenario part-a/task-29 --isolated --case happy --manual-transcript --evidence "$ATTEMPT_DIR/task-29-ovis-terminal-v1.json"
{"case":"happy","result":"passed","scenario":"part-a/task-29","totals":{"assertions":"33","cases":"1","failed":"0","passed":"1","skipped":"0"}}
exit 0
```

Transcript excerpt (stderr of `--manual-transcript`; full file `docs/evidence/task-29/happy.transcript.log`), one framed connection, shared fixture bytes written verbatim, every reply correlated by id:

```
bridge >> @0 233B write: {"id":"hello",...,"framed":true,"generation":"1"}
bridge << @0 1443B line: {"jsonrpc":"2.0","id":"hello","result":{...,"capabilities":{"framed":true,...},"endpoint_kind":"unix",...
bridge >> @233 81B write: <frames.json#control-request bytes>{"jsonrpc":"2.0","id":"w1","method":"doc.get","params":{"name":"alpha"}}
bridge << @1443 75B frame:0:data: {"jsonrpc":"2.0","id":"w1","result":{"name":"alpha","value":null}}
bridge >> @314 161B write: <frames.json#coalesced-requests bytes> w2 + w3
bridge << @1518 75B frame:0:data: {"jsonrpc":"2.0","id":"w2","result":{"name":"alpha","value":null}}
bridge << @1593 74B frame:0:data: {"jsonrpc":"2.0","id":"w3","result":{"name":"beta","value":null}}
bridge << @1667 151B frame:0:data: {"jsonrpc":"2.0","id":"page-at-max","result":{"data_b64":"","floor_offset":"0","from_offset":"67108864","gap":false,"next_offset":"67108864"}}
bridge << @1818 176B frame:0:data: {"jsonrpc":"2.0","id":"set-wide","result":{"name":"wide","value":{"above_2_53":"9007199254740993","kind":"kind_from_a_newer_runtime","offset":"18446744073709551615"}}}
bridge << @1994 176B frame:0:data: {"jsonrpc":"2.0","id":"get-wide","result":{"name":"wide","value":{"above_2_53":"9007199254740993","kind":"kind_from_a_newer_runtime","offset":"18446744073709551615"}}}
```

The request batch carrying `set-wide` and `page-at-max` (from_offset "18446744073709551615") was written one byte per write. The serve report asserts `accepted: "1"`: one connection carried every request.

```
$ ... --case failure --manual-transcript --append --evidence "$ATTEMPT_DIR/task-29-ovis-terminal-v1.json"
{"case":"failure","result":"passed","scenario":"part-a/task-29","totals":{"assertions":"48","cases":"1","failed":"0","passed":"1","skipped":"0"}}
exit 0

credit >> @233 17B write: <frames.json#credit-offset: CREDIT on channel 7, no stream>
credit << @1443 144B frame:0:data: {"jsonrpc":"2.0","method":"$/closing","params":{"reason":"unowned_channel","hint":"send frames only on channel 0 or an opened stream"}}
bridge >> @1520 165B write: {"id":"numeric-offset",...,"params":{"from_offset":9007199254740993,...}}
bridge << @3572 244B line: {"jsonrpc":"2.0","id":"numeric-offset","error":{"code":-32602,...,"data":{"kind":"invalid_params",...,"field_path":"params.from_offset"}}}
bridge << @3816 77B line: {"jsonrpc":"2.0","id":"still-served","result":{"name":"alpha","value":null}}
```

Failure case also replays all 8 `strict-rejections.json` cases (kind + field path, empty effect directory) and asserts `/failures/unowned_channel: "1"`, `/accepted: "2"`, `/panicked: "0"`.

Acceptance (one run, no retries):

```
$ cd packages/client-ts && bun run generate:check && bun run typecheck && bun test
{"generated":"fresh","files":30}
$ tsc --noEmit -p tsconfig.json
 26 pass
 0 fail
 3349 expect() calls
Ran 26 tests across 5 files.
exit 0
```

## QA

- happy: `cd packages/client-ts && bun test test/client.test.ts --test-name-pattern happy` -> 7 pass, 0 fail, exit 0 (`docs/evidence/task-29/qa-happy.log`).
- failure: `cd packages/client-ts && bun test test/client.test.ts --test-name-pattern failure` -> 7 pass, 0 fail, exit 0 (`docs/evidence/task-29/qa-failure.log`).

What the tests assert:

| Test | Against | Asserts |
|---|---|---|
| happy: shared wire fixtures ... every split | `conformance/fixtures/wire/frames.json`, unchanged | each of the 9 entries decodes to its declared frames/error kind at every single split and byte by byte; valid entries re-encode to the identical hex |
| happy: the Rust runner replayed the same fixture bytes | real `conformance` + `ovis-terminal` binaries | spawns the runner on `part-a/task-29 --case happy`; result passed; the sha256 and length the Rust runner recorded for each sent fixture write equal the sha256 Bun computes from the same fixture file; each decoded request id has exactly one reply in the Rust transcript |
| happy: one persistent real-socket connection | real server fixture (Unix socket) | framed hello; doc.set + terminal.output(from_offset u64::MAX) + rpc.discover pipelined on one connection; doc value with u64::MAX and 2^53+1 strings round-trips exactly; `rpc.discover` equals the generated `openrpc.json`; op.get reports committed; serve report `{accepted:"1",failures:{},panicked:"0"}` |
| happy: aborting a held mutation | real server fixture | AbortSignal -> client sends op.cancel; original request rejects with server kind `cancelled`; op.get state `cancelled`; no effect |
| happy: framed stream ... unknown output enums | scripted peer (schema-checked) | hello `endpoint_kind` unknown value and an unknown added field are kept; DATA on channel 5 offset from 2^53+1 exactly; unknown `attach.control` kind kept in order; `input_ready` sets readiness; ack sends one CREDIT frame on channel 5 whose u64 LE payload is exactly 2^53+1+5 |
| happy: input admission is separate | scripted peer | `input.send` carries the stream's lease_id/target; the returned Admission holds only the receipt (queued_seq "9007199254740993", output_cursor u64::MAX); the later `op.committed` event is a separate update; plain `attach.credit` acknowledged "9007199254740996" exactly |
| happy: reconnect resumes after the exact retained cursor | scripted peer | after transport loss the subscription is `detached`; `resume` sends the exact cursor `{lifetime, "9007199254740994"}`; a replayed duplicate is dropped (staleDropped 1), the next new event is delivered |
| failure: shared strict rejections | real server fixture | for each of the 8 `strict-rejections.json` cases the server's typed kind and field_path equal the fixture; for schema-level cases the local check yields the same kind and field path without sending; a numeric counter is local `invalid_params` at `params.version` |
| failure: reply lost after durable accept | real server fixture | after `$/fixture/held` the transport is severed; the call rejects `outcome_unknown` with a pending Draft; after reconnect the draft is listed, `inspect` returns the op, `discard` cancels it, a second decision throws `draft_settled`, the document holds no value unless the cancel reported already_committed |
| failure: client deadline | real server fixture | deadlineMs 50 -> `deadline_exceeded` with a Draft; op stays inspectable (accepted/running); discard -> `cancelled_before_commit`; no effect |
| failure: lost reply after input admission | scripted peer | peer received input.send and vanished; call rejects `outcome_unknown` with the exact params as a Draft; stream `inputReady` false, closed `disconnected`; a new send is refused locally `input_not_ready`; after reconnect the first request the peer sees is `op.get`, not input.send; only the explicit `retry` resends, with the same op_id and params; a second decision throws |
| failure: old stream's ack | scripted peer | stream 5 closes (CLOSE `taken_over`); ack on it -> `stream_closed`; ack beyond delivered or below acknowledged -> `impossible_ack`; the peer receives exactly one CREDIT frame, on channel 6 |
| failure: unreplayable cursor | scripted peer | resume answered `missed: cursor_expired` -> state `missed`, reason kept, cursor cleared |
| failure: numeric counter in a reply | scripted peer | `queued_seq` as a JSON number -> `invalid_response` at `result.receipt.queued_seq` |

Other files: `decimal.test.ts` (u64::MAX and 2^53+1 round trips; numbers, non-canonical and out-of-range refused), `frames.test.ts` (2000 seeded segmentations of all coalesced valid fixtures; every invalid fixture after a valid frame under 200 splits; 300 random frames round trip; CREDIT u64::MAX), `schema.test.ts` (outputs tolerate added fields and open-enum values but refuse missing required fields, numeric counters and unknown closed values; inputs refuse added fields and unknown values), `generate.test.ts` (every method in exactly one namespace module; deterministic; committed output equals generation; nominal Decimal/Id/Open types; `unsupported_schema` and `name_collision` typed failures).

## Adversarial

- Mutants (each reverted): auto-resend on reconnect -> lost-input test fails; ack on a closed stream allowed -> stale-ack test fails; stale-cursor drop removed -> resume test fails; open-enum output sites treated as closed -> unknown-enum test fails; abort without op.cancel -> cancellation test fails. 5/5 killed.
- Stale state (stream/cursor): stale ack, impossible ack, replayed duplicate event, missed cursor.
- Cancel/resume: AbortSignal cancel, client deadline, reconnect with drafts and cursor resume.
- Misleading success (admission only): Admission exposes only the receipt; delivery comes from op.get / op.* events.
- Flaky tests: full `bun test` STRESS_PLACEHOLDER under `taskset -c 0-1` with 8 busy loops. Waits are event-driven (`once`/`within` hang guards); the only time-based assertion is the 50 ms client deadline on an operation the fixture holds until cancelled.
- macOS: `probe-stress-macos.yml` job `client-ts` (generate:check, typecheck, 25x bun test): MACOS_PLACEHOLDER

## Contract checklist

1. u64 on the wire: decimal strings only; `Decimal<Kind>` brand; `parseDecimal`/`decimal` refuse numbers; u64::MAX and 2^53+1 round trips in unit tests and over the real socket; numeric counters refused both in requests (`invalid_params`) and replies (`invalid_response`).
2. Errors: TypeScript has no thiserror; the boundary equivalent is one `ClientError` class with a closed `kind` union and a `hint` table (`clientErrorHints`), `cause` kept; `RpcError` keeps the server's `data` (kind kept even when unknown). The generator has `GenerateError` with kind + hint.
3. Identities: every named string component is `Id<"Name">`, every decimal is `Decimal<"Name">`; StreamId, LeaseId, OpId, SubscriptionId etc. are not interchangeable.
4. Behaviour tests (state transitions, round trips under every split, typed kinds); seeded property tests replace proptest on the Bun side; Rust side unchanged.
5. No lint suppression, no placeholder TODOs.
6. Holder wire generation unchanged (2); protocol generation "1" read from the schema.

## Seams

- `bun.lock` (repo root, new): the workspace lockfile for the package's dev dependencies (`typescript` 5.9.3, `@types/bun` 1.4.4). Bun writes it at the workspace root; no runtime dependency.
- `conformance/src/runner_tests/suite.rs`: registers `part-a/task-29.json` in `EXECUTABLE`, written on integration's slice form (b96f583) so the PR merges cleanly.
- `.github/workflows/probe-stress-macos.yml`: the `client-ts` job was committed only to dispatch the macOS stress run and then restored to base so the PR stays MERGEABLE; the block for the orchestrator's union is `docs/evidence/task-29/stress-job.yml`.
- No Rust public API changes.

## Risks

- Routes not served by the runtime yet: `attach.*`, `events.subscribe` and real `input.send` receipts are not routed (todo 31). Those client paths are tested against a scripted peer whose every reply/notification is checked against the published schema first; they are not real-binary proof.
- Fixture divergence found: the conformance fixture's `doc.set`/`doc.get` stand-ins answer `{name, value}`, while the catalog publishes `DocSetResult{version, seq}` and `DocRecord{name, value, version}`. The typed client refuses those replies as `invalid_response`, so tests use `client.unchecked` for doc.* against the fixture. Owner: the fixture/documents lanes (todo 21/50), not changed here.
- No per-PR CI job runs `bun test` (ci.yml is outside this write scope); CI evidence is the macOS stress job plus local runs. Todo 63 packages the client.
- `test/client.test.ts` needs prebuilt `target/debug/conformance` and `target/debug/ovis-terminal` (overridable with `OVIS_CONFORMANCE_BIN` / `OVIS_TERMINAL_BIN`); it fails with the build command when they are missing.
