feat(client-package): package generated typescript types and persistent bun client

Todo 63 (wave 17). Closes GAP-004, GAP-005, GAP-018, GAP-019, GAP-023, GAP-026, GAP-060, GAP-139, GAP-140. Delivers IDEAL-004, IDEAL-006, IDEAL-007, IDEAL-013, IDEAL-019, IDEAL-028, IDEAL-040, IDEAL-055, IDEAL-112, IDEAL-113. Enforces EXCL-014, EXCL-015, EXCL-016, EXCL-025 (scenario `client/package` declares all four).

## What changed

- `@ovis-terminal/client` is now a packable ESM package: `exports` for `.` (client), `./protocol` (generated types and catalogs only), `./openrpc.json` (the versioned OpenRPC document) and `./package.json`. It also declares `types`, `files` (`dist`, `LICENSE`, `README.md`), `engines.bun >=1.4.3`, `license: MIT` and `sideEffects: false`. `private: true` stays as the publication guard: packing does not publish.
- `bun run build` (`scripts/build.ts`) regenerates every protocol module from `schema/ovis-terminal.openrpc.json` and refuses with the typed `stale_generated` error (exit 1, nothing written) when `src/generated` differs. It then writes `dist/index.js` and `dist/protocol.js` (Bun ESM bundles), `dist/types/**.d.ts` (tsc) and `dist/openrpc.json`.
- `bun run pack` (`scripts/pack.ts`) writes `pack/ovis-terminal-client-0.1.0.tgz` through `bun pm pack` and prints its sha256. Rebuilding and repacking reproduces the tarball byte for byte (sha256 `ed9f8c9b…5963`). Packaging failures are a typed `PackageError` (`kind` + `hint`).
- Client: `client.failure` exposes the typed error that ended the last connection (for example `unknown_kind` from the frame decoder), and `onDisconnect` listeners receive it once per ended connection. `endpointTransport(platform)` refuses Windows named pipes with the typed `unsupported_transport` instead of opening a Unix path (see Seams).
- `packages/client-ts/tests/` installs the produced tarball into a project outside the repository and runs every case there. They import `@ovis-terminal/client` only, never `../src`.
- `conformance/scenarios/client/package.json`: the real-binary wire contract the packed client relies on.

## Before

`docs/evidence/baseline/smoke/herdr-h02-connection.log:2` (RT-HD-01; source `<run-root>/evidence/smoke/herdr-h02-connection.log`, sha256 `696b0506…c0378b`, `docs/evidence/baseline/README.md:26`):

```
### two requests on one connection (ping then ping)
frames received on one connection: 1 closedByServer=true ms=0
```

Gap baseline: GAP-004, GAP-005, GAP-018, GAP-019, GAP-023, GAP-026, GAP-060, GAP-139, GAP-140 (`docs/design/gap-list.md`). Copy: `docs/evidence/task-63/before.log`.

## After

Host: Linux 7.0.0-38-generic x86_64, rustc 1.99.1, bun 1.4.3. Binary `target/debug/ovis-terminal` sha256 `74a2c2d7a7d59509e74865c9a4c5705cb3ac467abdb24f515307d79916bbce73` (also recorded in the evidence `binary` field). Full transcript: `docs/evidence/task-63/qa-conformance.log`. Evidence: `docs/evidence/task-63/task-63-ovis-terminal-v1.json` (happy + appended failure) and `task-63-all.json`.

```
$ cargo build --locked -p ovis-terminal-cli --bin ovis-terminal
exit=0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario client/package --isolated --case happy --evidence "$ATTEMPT_DIR/task-63-ovis-terminal-v1.json"
{"case":"happy",...,"result":"passed","scenario":"client/package","totals":{"assertions":"28","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
exit=0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario client/package --isolated --case failure --inject '{"method":"terminal.run","params":{"op_id":"expired-operation"}}' --evidence "$ATTEMPT_DIR/task-63-ovis-terminal-v1.json" --append
{"case":"failure",...,"result":"passed","scenario":"client/package","totals":{"assertions":"27","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
exit=0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario client/package --isolated --case all --evidence "$ATTEMPT_DIR/task-63-all.json"
{"case":"all",...,"result":"passed","scenario":"client/package","totals":{"assertions":"59","cases":"2","declared_only":"0","failed":"0","passed":"2","skipped":"0"}}
exit=0
```

What the scenario asserts, through the binary's `ovis-terminal connect` bridge (the materialized CLI/socket path):
- **happy**: one framed connection carries `doc.set`, `terminal.run` and `terminal.output`, written in 1/7/64/3-byte fragments. Each reply is correlated by id. The launched terminal is `live`, with its spawn effect pinned. A document holding `"status":"future_status"` and `"offset":"18446744073709551615"` round-trips byte-exactly through `doc.get`, and `op.get` reports `committed`. The fixture serve report is `accepted:"1"`, meaning one persistent connection served everything (EXCL-014).
- **failure**: the injected literal `terminal.run` op_id `"expired-operation"` is not a UUIDv7, so the binary refuses it as `invalid_params` at `params.op_id` (`retryable:false`, no result). An op_id older than the 24 h retention window (`00000000-0000-7000-8000-000000000063`, and `01890000-…0001` in the planned injections) is the typed `op_expired` refusal carrying that `op_id` with `retryable:false`. A later `op.get` on it is still `op_expired` (never admitted). A numeric u64 is `invalid_params` at `params.from_offset`. The effects directory is empty and the same connection keeps serving afterwards.

The packed client: `bun run --cwd packages/client-ts build && bun run --cwd packages/client-ts pack && bun test packages/client-ts/tests` gives **exit 0, 17 pass, 0 fail, 51 expect() calls**, in one run (`docs/evidence/task-63/acceptance-bun.log`). The cases run inside the isolated consumer:
- every entry point resolves to `<consumer>/node_modules/@ovis-terminal/client/dist/…`, and the installed package has no `src/`. The consumer typechecks (`tsc`, exit 0) against the packed declarations, and five `@ts-expect-error` misuses stay rejected (numeric u64, crossed identity kinds, an unbranded additive enum value, an unpublished method).
- **real Rust fixture**: one framed connection pipelines `doc.set`/`terminal.output`/`rpc.discover`/`terminal.run`. `rpc.discover` equals the exported `openrpc.json`, `future_status` and u64::MAX stay exact, and the serve report is `accepted:"1"`. **`op_expired` surfaces as a typed `RpcError`** (`serverKind:"op_expired"`, `op_id` echoed, `retryable:false`, not `OutcomeUnknown`, no draft). A byte-transparent relay counts **exactly 1** `terminal.run` reaching the server across a reconnect, so there is **no automatic resend**. A held mutation whose reply is lost is `OutcomeUnknown` with a retained draft. After reconnect the relay still counts 1 `doc.set` until the explicit `discard()`, and `retry()` after it is `draft_settled`.
- **scripted peer** (records schema-checked; the fixture serves no `attach.*`/`events.*` yet): an additive `op.get` state `future_status` is kept with every known field equal. Attach outcomes are typed (`ready`, `ready`, `refused` with reason and no stream, additive `future_outcome`), and stream bytes are isolated per channel. A CREDIT frame carries channel 3 and offset 5, and an impossible ack is `impossible_ack`. A closed stream's ack is `stream_closed`, and the next CREDIT on the wire belongs to the live stream (channel 5). Resubscribing after reconnect sends the retained cursor `{runtime_lifetime, seq:"6"}`, and the replayed duplicate is dropped (`staleDropped:1`). **A malformed server frame (kind 0x7f) fails the decoder once**: one disconnect carrying `unknown_kind`, `client.failure.kind === "unknown_kind"` while the connection is down, the pending read is `disconnected` with cause `unknown_kind`, and the pending mutation becomes a draft. After reconnect the first request is `op.get`, not a resend. Frames decode identically at every split, and a truncated tail is `truncated_frame`.
- the package itself: the tarball holds only `package/dist/**`, `package.json`, `LICENSE` and `README.md`. Every `exports`/`main`/`types` target exists in it, `dist/openrpc.json` equals `schema/ovis-terminal.openrpc.json` byte for byte, and `LICENSE` equals the repository LICENSE. Rebuild plus repack gives the same sha256, and the build refuses a drifted schema with `stale_generated` and writes no `dist/`.

Gates:
- `cd packages/client-ts && bun run generate:check && bun run typecheck && bun test`: exit 0, 47 pass, 0 fail, 3416 expect() calls (`client-ts-gate.log`).
- `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh`: exit 0, `check.sh: all steps passed`, nextest `1349 tests run: 1349 passed (3 slow), 2 skipped`, evidence-audit `verdict: pass` (`check.log`).
- Windows pre-check: `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` exit 0, and `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings` exit 0 (`windows-precheck.log`).

Adversarial (`docs/evidence/task-63/mutants.log`): five mutants applied to `src/`, each rebuilt into the tarball and run through `bun test packages/client-ts/tests`. All five were **killed**: auto-resend of drafts on reconnect (2 fails), the Windows pipe not refused (1), output open enums treated as closed (3), a closed stream's ack granting credit (2), and the decoder failure not surfaced (it survived round 1, so I added the `failure_while_down` assertion, and it was then killed with 1 fail). The adversarial classes are covered as follows: stale state (replay cursor, stream ack), cancel/resume (lost mutation reply), malformed input (fragmented frames, malformed server frame), and misleading success (tarball-only imports, resolution and no `src/` asserted).

## Seams

- `conformance/scenarios/client/package.json`: the runner maps the scenario id `client/package` to this file path, which sits one level above the `conformance/scenarios/client/package/` write scope.
- `conformance/src/runner_tests/suite.rs`: `client/package.json` is added to `EXECUTABLE` in sorted position. `declared.json` had no entry for it.
- `docs/evidence/task-63/*`: evidence.
- Not changed, recorded in `docs/evidence/task-63/seams.md`: the **Windows named-pipe transport** (todo 60's seam). The missing piece is a `\\.\pipe\…` connect path that waits in `WaitNamedPipeW` (bounded) on `ERROR_PIPE_BUSY` (231), reopens, and reports `pipe_busy_timeout` after the bound, as `crates/ovis-terminal-cli/src/invocation/transport/pipe.rs` does. It isn't built here because the client-ts CI matrix is ubuntu/macOS only, so it would ship unexercised. Until then `Client.connect` on win32 refuses with the typed `unsupported_transport`, and the CLI `connect` bridge is the Windows route.

## Risks

- Real-server attach/events for the packed client are exercised only against the schema-checked scripted peer, because the protocol fixture answers `method_not_found` for `attach.*`/`events.*` (probed). The real-binary journey arrives with assembly (todo 31) and todo 64's consumer fixtures.
- QA deviation: the plan's literal injection `op_id:"expired-operation"` cannot yield `op_expired`. It is not a UUIDv7, so the binary answers `invalid_params` at `params.op_id`, before any ledger lookup (`crates/ovis-terminal-core/src/ops/request.rs:93-104`). The scenario plans that literal with its true typed reply, and proves `op_expired` with UUIDv7 ids older than the retention window. The `--inject` run exits 0 with both asserted.
- Observation outside scope (`crates/ovis-terminal-core/src/ops`): two concurrent requests reusing one expired op_id under different methods can answer `op_reused` for the second, while sequential reuse always answers `op_expired`. The scenario uses distinct ids.
- Tarball file modes come from the checkout (umask), so byte-reproducibility holds per checkout, not across hosts with different umasks.

DEFERRED VERIFICATION: none. The runner executes `client/package` today (happy, failure with `--inject`, and all; exit 0 each).
