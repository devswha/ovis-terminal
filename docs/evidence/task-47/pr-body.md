# feat(update): stage verified updates with inspectable rollback outcomes

Plan todo 47. Closes GAP-127. Delivers IDEAL-100, IDEAL-107 and IDEAL-117. It enforces EXCL-013, EXCL-020 and EXCL-021, and does not reproduce V-005, V-3905 or V-RT-HC-03.

## Before

`absent`. There was no comparable baseline capability, and no failing test was run as a baseline. Gap baseline: GAP-127 ("Updates lack staged verification and rollback exposed by the runtime"). `update.*` was declared in the todo-4 catalog but had no route.

## After

Host: `linux x86_64` (`uname -sm`: `Linux x86_64`). The binary under test is `target/debug/ovis-terminal`, sha256 `aada0e17acbc295cab04fef08030a6e31676dfdb36d8c7c949bbe4d34f353ff1`, as recorded in the evidence `binary` block. `ATTEMPT_DIR=.omo/evidence/ovis-terminal-v1/task-47`.

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario services/update --isolated --case happy --evidence "$ATTEMPT_DIR/task-47-ovis-terminal-v1.json"
{"case":"happy","error":null,...,"result":"passed","scenario":"services/update","totals":{"assertions":"81","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
exit 0

$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario services/update --isolated --case failure --inject '{"method":"update.stage","params":{"sha256":"0000000000000000000000000000000000000000000000000000000000000000"}}' --evidence "$ATTEMPT_DIR/task-47-ovis-terminal-v1.json" --append
{"case":"failure","error":null,...,"result":"passed","scenario":"services/update","totals":{"assertions":"48","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
exit 0

$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario services/update --isolated --case all --evidence "$ATTEMPT_DIR/task-47-case-all.json"
{"case":"all",...,"result":"passed","scenario":"services/update","totals":{"assertions":"137","cases":"2","declared_only":"0","failed":"0","passed":"2","skipped":"0"}}
exit 0
```

Materialized commands: the runner starts `target/debug/conformance fixture --root <isolated root>`, which is the real server crate (listener, dispatcher, SQLite ledger, authority). It also runs a real in-process holder with one live program under the todo-48 restartable `Control`. Every request goes through `target/debug/ovis-terminal connect`, which exits 0 with empty stderr. Wire lines of the failure run (bridge transcript):

```
-> {"id":"inject-0","jsonrpc":"2.0","method":"update.stage","params":{"op_id":"01a19439-5cb4-77a0-8b4f-3d1f654db7f7","sha256":"0000000000000000000000000000000000000000000000000000000000000000","version":"2.0.0"}}
<- {"jsonrpc":"2.0","id":"inject-0","error":{"code":-32000,"message":"operation failed","data":{"kind":"integrity_failed","hint":"check the release checksum and stage it again","retryable":false,"op_id":"01a19439-5cb4-77a0-8b4f-3d1f654db7f7"}}}
-> {"id":"untouched","jsonrpc":"2.0","method":"update.query","params":{}}
<- {"jsonrpc":"2.0","id":"untouched","result":{"active":{"binary_sha256":"f43133a9ecab78e2fd9a9d632a028413b257756b8e8edef2dbd902fbb52f1165","target":"linux-x86_64","version":"1.0.0"},"active_version":"1.0.0","live_incarnations":["1"],"pending":null,"previous":null,"updates":[]}}
```

The scenario asserts that `active.binary_sha256` and `live_incarnations` after the refusal equal the values captured before it, and that no update record exists.

Happy observable (same transcript format):

```
<- {"id":"activate-2","result":{"state":"activated","active_version":"2.0.0","activation":{"previous":{"binary_sha256":"f43133a9…","version":"1.0.0",…},"readiness":{"exit_code":0,"outcome":"ready",…},"restart":{"server_boot_id_old":"boot:1","server_boot_id_new":"boot:2","preserved_incarnations":["1"],"ended_incarnations":[],…}},…}}
<- {"id":"activate-3","error":{"code":-32000,"message":"operation failed","data":{"kind":"activation_failed","hint":"update.query shows the rollback receipt and evidence",…}}}
<- {"id":"rolled-back","result":{"active":{"binary_sha256":"7b86b759…","version":"2.0.0",…},"active_version":"2.0.0","live_incarnations":["1"],"pending":null,…,"updates":[…,{"state":"rolled_back","rollback":{"reason":"readiness_failed","from":{"version":"3.0.0",…},"restored":{"version":"2.0.0","binary_sha256":"7b86b759…"},"readiness":{"outcome":"exited","exit_code":1,…},…}}]}}
```

`op.get` for the failed activation returns `state: failed` with `error.kind: activation_failed`, not `outcome_unknown`. Raw evidence, including the stdout, stderr and wire transcripts per connection, is in `docs/evidence/task-47/`.

## What changed

- **Protocol (`domain/update`, `schema/methods/update.rs`).**
  - Validated values: `Sha256Digest` (64 lowercase hex), `BuildTarget` (`<os>-<arch>`), `ReleaseVersion` (never a path), and `HolderGeneration` (decimal string).
  - The archive manifest `BuildManifest` (`build.json`).
  - Receipts: `ActiveBuild`, `UpdateRejection`/`RejectReason`, `ReadinessEvidence`/`ProbeOutcome`, `RollbackReceipt`/`RollbackReason`, `ActivationReceipt` and `PendingActivation`/`ActivationPhase`.
  - New error kinds `invalid_update_state` and `activation_failed`.
- **Services (`ovis-terminal-services::update`).**
  - `update.stage` looks up `<releases>/<version>/ovis-terminal-<version>-<target>.tar`, then hashes it while copying into `<install>/staging/<id>/` with a size bound and cancellation. On a mismatch or cancel it removes the partial files and the active build is untouched. A target other than the runtime's own is refused as `incompatible`.
  - `update.verify` re-hashes the staged archive, then extracts it with a ustar reader. The reader refuses absolute paths, `..`, links, devices, extension headers, duplicate entries and bad header checksums. It then checks `build.json` against the request and the runtime's protocol and holder generations. A rejection is recorded on the update with its reason.
  - `update.activate` works through a durable journal in `<install>/activation.json` (`intent` → `selected` → `ready`):
    1. It swaps `active.json` atomically (write, fsync, rename, directory fsync).
    2. It probes the selected executable with `--json --version`, using a fixed argv, an empty environment and a deadline.
    3. It delegates the restart to the typed `RestartControl` port. The server adapter calls todo 48's `Control::restart`, which keeps every program incarnation.

    If readiness or the restart fails, the previous build is selected again and a `RollbackReceipt` with the probe evidence is recorded. Opening the install root finishes a journal at `ready` and reverts any earlier phase. The revert is idempotent, so an interruption during recovery is also covered.
  - `update.rollback` re-verifies the previous executable's digest. It then swaps back, restarts, and records a `requested` receipt. On failure the selection is unchanged.
  - Nothing in the update path signals, kills or replaces a program. Holders keep their programs and drain by build.
- **Server (`dispatch/lanes/update.rs`, `lanes/update/control.rs`).**
  - Four `Action::Administer` mutations and the unrecorded `update.query` read.
  - `ControlRestart` adapts `Control` to the port (unix).
  - New `Execution::commit_proven` (`dispatch/execution/proven.rs`). With it, a refusal with a known outcome settles the ledger as `failed` with its typed kind; plain `commit` would record `outcome_unknown`. Only install-root I/O and control failures stay uncertain.
- **Conformance.**
  - Executable scenario `services/update.json` (happy and failure) and the fixture lane `fixture/update.rs`.
  - The fixture uses a real holder, a real `Control` with one live program, and releases written by the system `tar --format=ustar`. It adds an unadvertised `fixture.update` probe that reports the digests and whether any file escaped.
  - The runner gains a `$capture` matcher and `$value:<name>` placeholders. These are needed to chain the service-minted `update_id` and the per-host digests.

## QA

- `bash scripts/check.sh`: exit 0. That covers fmt; clippy `-D warnings` on the workspace; nextest `987 tests run: 987 passed (1 slow), 1 skipped`; doc tests; hygiene `0 violation(s) in 978 file(s)`; manifests; and json (`check-script.log`).
- Windows pre-check: `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` exits 0 with no warnings (`windows-precheck.log`).
- Acceptance build: `cargo build --locked -p ovis-terminal-cli --bin ovis-terminal` exits 0.
- What the lane tests assert:
  - `ovis-terminal-protocol` `domain::update` (10, 3 of them proptest):
    - `u64::MAX` holder generations round-trip as decimal strings, and numeric ones are refused.
    - Manifests with unknown keys are refused.
    - Path-like versions (`../x`, `a/b`, `.hidden`, 65 chars) and bare targets are refused.
    - Stage params refuse uppercase digests and the legacy `digest` field.
    - A rolled-back record round-trips with its receipt.
    - Any 32-byte digest round-trips, and any other length is refused.
  - `ovis-terminal-services` `update::tests` (14):
    - lifecycle: activation only after readiness and restart, with the receipt naming the preserved incarnations; readiness failure rolled back with exit code 1, the stdout excerpt and zero restarts; a refused restart rolled back; an explicit rollback through a restart, after which a second rollback is `invalid_update_state`.
    - refusal: wrong digest, wrong target, missing archive, cancellation, four unsafe archive entry kinds, three manifest misfits, a tampered staged archive, out-of-order steps and foreign ids.
    - recovery: reopen after stage; three interrupted-activation journals, each reopened twice; a `ready` journal finished on open.
  - `ovis-terminal-server` `lanes::update::qa::wire` (2). These run over a real socket, dispatcher, SQLite ledger, holder and todo-48 `Control` with a live `read gate` child.
    - Activation restarts control (`boot:1` → `boot:2`) with `preserved_incarnations` equal to the child's generation, and the child keeps its PID and start token.
    - `integrity_failed` and `activation_failed` are settled as `failed` in `op.get`. A numeric digest is `invalid_params`. After the refusals, the active build and live incarnations equal the values from before.
  - Conformance runner (2): `$capture` requires and yields the pointed value; `$value:` placeholders resolve, and unknown names stay literal.
- Conformance `services/update`:
  - happy: 81 assertions.
  - failure with the plan's `--inject`: 48 assertions.
  - `--case all`: 137 assertions, 2/2 passed.
  - Every run: failed 0, skipped 0, binary hash recorded.
- macOS stress: `gh workflow run probe-stress-macos.yml --ref lane/t47-stage-verified-updates-with -f probe=update` ran 25 iterations of the services `update::` tests, the server `lanes::update` tests and `conformance --scenario services/update --case all` on `macos-latest`. Conclusion: success. https://github.com/devswha/ovis-terminal/actions/runs/41252025533

## Adversarial

- Malformed input:
  - Archive traversal: `../escape`, `/escape`, `dir/../../escape` and a symlink entry are each rejected as `unsafe_archive` with no file outside the root (services test). The conformance failure case stages a `tar -P` archive holding `../escape`; verify returns `integrity_failed`, the record shows `unsafe_archive`, and `fixture.update` reports `escaped: false`.
  - Wrong target is `incompatible`. A numeric digest and a `../2.0.0` version are `invalid_params` (planned inject entries, run under `--case all`).
  - Foreign or path-shaped `update_id`s (`../active`, `update-not-a-uuid`) are `no_staged_update`.
  - A manifest naming another target, holder generation `1`, or a missing executable each give the matching rejection reason.
- Cancel/resume:
  - A cancelled stage removes `archive.tar.partial` and records nothing.
  - A staged update survives a reopen: `update.query` lists the identical record, and `update.verify` then succeeds.
- Repeated interruptions: journal recovery runs three cases, and each one reopens twice with the same outcome:
  - `intent` with the old selection;
  - `selected` with the new selection;
  - `selected` after the selection was already restored, i.e. an interrupted recovery.

  A `ready` journal is finished as `activated` on open.
- Tampering: a staged archive rewritten after `stage` fails `verify` with `digest_mismatch`.

## Contract checklist

1. Every u64 on the wire is a canonical decimal string: generations, `recorded_ms`, incarnations. A numeric `holder_generations` entry is refused, and `u64::MAX` round-trips exactly (`holder_generation_round_trips_u64_max_as_a_decimal_string`; proptest `protocol_generations_round_trip`).
2. Boundary errors are `thiserror` enums with `kind()` and `hint()`: `UpdateError`, `RestartFailure`, `UpdateValueError`. `#[source]` is used where an inner error exists, and Display never repeats the kind token.
3. Distinct newtypes: `UpdateId`, `Sha256Digest`, `BuildTarget`, `ReleaseVersion`, `HolderGeneration`, `ProtocolGeneration`, `Incarnation`. `Option` is used only for real absence (`build` before verification, `previous` before the first activation, `pending`).
4. Tests assert behavior: state transitions, typed kinds, on-disk effects, the ledger outcome through `op.get`, and the live child's start token. Proptest covers digest and generation round trips.
5. No `#[allow]` and no placeholder TODOs. Platform differences use symmetric `cfg` (directory fsync, executable mode bit).
6. Evidence states exactly what the scenario asserts. The conformance scenario is executable and is not deferred.

Holder wire generation stays 2.

## Seams

- **`schema/released.openrpc.json` is amended in place, for `update.*` only.**
  - Changed: `update.stage` params now carry `sha256` and an optional `target` in place of `digest`, and `version` is pattern-constrained. `UpdateRecord` changed (`sha256` instead of `digest`, a `build` object instead of `generation`, and the receipts). `UpdateQueryResult` gains `active`, `previous`, `pending` and `live_incarnations`. Error lists gained entries.
  - Against the pre-amendment baseline, `openrpc compat` reported 27 breaks, all under `update.*`/`update.progress` (`schema-compat-pre-amendment.log`). After the amendment both `openrpc compat` and `openrpc check` exit 0 (`schema-compat-post-amendment.log`).
  - Generation 1 is unreleased, and todo 22 set the precedent. If the baseline must stay immutable, this part must be reverted along with the stage-param shape.
  - **Union-merge action:** `schema/BASELINE-AMENDMENTS.md` (todo 4, PR #58) landed on integration after this lane's base, so this branch cannot add it without an add/add conflict. Append the entry in `docs/evidence/task-47/baseline-amendment-entry.md` to that ledger when merging. It chains from the ledger's current head (`previous: sha256:362980ac…`, which equals this lane's base baseline) to `current: sha256:53782aab…`. That value is the compact-JSON SHA-256 of the amended `released.openrpc.json`, computed the way `baseline_digest` does (verified by reproducing the base digest `362980ac…`). Without it, `openrpc compat` refuses the merged tree with `baseline_amendment_missing`.
- **`ovis-terminal-protocol`**: new `domain::update`; `DomainErrorKind::{InvalidUpdateState, ActivationFailed}`; `UpdateStageParams.target: Option<BuildTarget>`; new `VerifiedBuild`. `UpdateRecord::staged` and `UpdateQueryResult::new` are new constructors.
- **`ovis-terminal-server`**: `Execution::commit_proven` and `dispatch::Unsettled` (new child module `execution/proven.rs`, plus one `mod` line in `execution.rs`); `lanes::update::{UpdateLane, UpdateStage, UpdateVerify, UpdateActivate, UpdateRollback, UpdateQuery}`; `lanes::update::control::ControlRestart` (unix). New dev-dependency `sha2`. `dispatch/runtime.rs` is untouched.
- **`ovis-terminal-services`**: new `update` module; new dependency `sha2` (workspace).
- **Conformance runner** (todo 21's crate):
  - `Matcher::Capture`, `Expect::captures` and `Labels::capture` with `$value:` resolution (unit-tested). The `replies` step records captures.
  - Fixture changes: `fixture/update.rs`, one probe route in `handler.rs`, two lines in `serve.rs`, and a `suite.rs` entry (`services/update.json`, `N = 18`).
  - New dependencies `ovis-terminal-holder` and `ovis-terminal-services`.
- **Scenario path**: `conformance/scenarios/services/update.json`, not the plan's `services/update/` directory, because the runner's id grammar is `<family>/<name>.json`.
- **`.github/workflows/probe-stress-macos.yml`**: the option `update` and its job are appended at the end of the lists.
- **Integration drift:** `origin/integration` moved to `804e508` (todos 4, 56 and 35) after this lane's base `759c85a`, and this lane does not merge. `git merge-tree` shows three structural conflicts:
  - `.github/workflows/probe-stress-macos.yml`: both sides appended a job; keep both.
  - `crates/ovis-terminal-services/src/lib.rs`: union the `pub mod` lines.
  - `schema/ovis-terminal.openrpc.json`: regenerate with `openrpc write` after the union.

  Also append the baseline ledger entry described above.

## Risks

- The readiness probe runs the selected executable's `--json --version`. A real build must keep that output (`{"binary","version","generation"}`), which todo 20's CLI already prints.
- Archives are uncompressed POSIX ustar: there is no gzip support because no compression crate is in the lockfile. Release packaging (todo 58) must produce `.tar`.
- `update.progress` events are not published. Progress is durable in the install-root records and the ledger, and is read through `update.query` and `op.get`.
- The served binary does not register this lane yet; that is todo 31's assembly. The conformance fixture registers it.
- The service holds its state lock for a whole step, so `update.query` waits while a stage copy or a readiness probe (10 s deadline) is running.
