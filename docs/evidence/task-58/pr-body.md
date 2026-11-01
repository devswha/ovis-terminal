feat(boundary-policy): enforce scoped paths payloads and diagnostic redaction

Plan todo 58 (wave 14). Closes GAP-129..GAP-136; delivers IDEAL-102..IDEAL-109 and IDEAL-115;
enforces EXCL-020, EXCL-021 and EXCL-023 and does not reproduce V-005, V-3905 or V-4078.

## What changed

- **Typed boundary values** (`ovis-terminal-protocol::domain::boundary_policy`): `LaunchArgv`
  (literal vector, never shell-interpreted; 4096 elements, 32 KiB each, 256 KiB total),
  `LaunchEnvironment` (refuses `OVIS_TERMINAL_*` under Unicode case folding), `RelativePath` /
  `StoreRef` (normal components only, `\` refused everywhere, 4096 bytes), `EndpointAllowlist`
  (SSH hosts, case-insensitive; argv transports refused when restricted) and one
  `BoundaryViolation` error (`too_large` / `forbidden` / `invalid_params`, field path and bound
  reported, never the value). `agent.history` gains an optional `store`; schema and TS SDK
  regenerated (`compat` against `schema/released.openrpc.json` exits 0, additive field).
- **Filesystem containment** (`ovis-terminal-authz::validation`): `ScopedRoot::resolve` follows
  symlinks and refuses a result outside the canonical root; `ContainedPath::recheck` repeats the
  decision at use and compares dev/inode on Unix, so a retargeted symlink or swapped file is
  refused. The update archive reader now uses the same `RelativePath` check (T47 seam).
  `Authority::confers` answers whether a live credential holds a permission at any scope.
- **One redaction boundary** (`ovis-terminal-core::telemetry::redaction`): `redacted/redact`
  replace credential and secret-input fields at any depth (bearer, token, credential, text,
  keys, data_b64, answer, env values, plugin input, task, content, file writes, ...) and scrub
  `ovt1.c<n>.<64 hex>` bearers from every other string; `RedactingSink` applies it to any
  telemetry sink; `emit` scrubs before its 256-char bound; `AuditLog` + `RetentionPolicy`
  (count and age, `seq` per record, eviction counter) retain only redacted records; `seal` keeps
  a sha256 digest of a secret that must persist.
- **Boundary policy lane** (`ovis-terminal-server::dispatch::lanes::boundary_policy`):
  `MUTATION_ROLES` names every OpenRPC mutation with the action its lane authorizes (a test
  generates the set from the published document); `BoundaryPolicy` wraps the handler, refuses a
  mutation whose permission the credential confers nowhere before params are parsed, records
  one redacted audit record per call (request, method, principal, op_id, target, machine_id,
  runtime_id, seq, outcome, error_kind, bounded redacted params) and redacts error data;
  `AuditTrail::read` needs `administer`; `AgentHistory` serves `agent.history` below an
  authorized root (listing names only, symlinked entries skipped, at most 10 000 entries
  examined, store rechecked after the read); `admit_launch` runs in `terminal.run` resolve.
- **Secrets at rest (seam from T57)**: `credential.rotate` seals the new bearer to
  `sha256:<hex>` in the operation ledger; the reply still shows the bearer once, and `op.get` /
  an op-id retry return only the digest.

## Before

`absent` (no comparable baseline capability; not an executed failing test). Gap baseline:
GAP-129..GAP-136 (`docs/design/gap-list.md:154-161`). Pre-change facts on integration da5b456
are listed with file references in `docs/evidence/task-58/before.md`; e.g.
`crates/ovis-terminal-server/src/dispatch/lanes/credential_rotation.rs` `completion` committed
the bearer into the ledger result, and `AgentHistoryParams` had no `store` field.

## After

Host: `Linux 7.0.0-38-generic x86_64`, `rustc 1.99.1 (58cf8f9c3 2026-10-13)`; binary
`target/debug/ovis-terminal` sha256 `d5c8a5b317a8df0b6b9e363668758cc6cbb730d2bd97d99a7f4b04fde7b32940`
(`docs/evidence/task-58/acc-binary.sha256`). Each command's stdout/stderr/exit is in
`docs/evidence/task-58/<name>.{cmd,stdout,stderr,exit}`.

| Command | Exit | Result |
|---|---|---|
| `cargo build --locked -p ovis-terminal-cli --bin ovis-terminal` | 0 | built |
| `cargo test -p ovis-terminal-authz -p ovis-terminal-core -p ovis-terminal-protocol -p ovis-terminal-server --all-targets` | 0 | 821 passed, 0 failed, 48 test binaries |
| `cargo clippy -p ovis-terminal-authz -p ovis-terminal-core -p ovis-terminal-protocol -p ovis-terminal-server --all-targets -- -D warnings` | 0 | clean |
| `cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario security/validation --isolated --case all --evidence "$ATTEMPT_DIR/task-58-ovis-terminal-v1.json"` | 0 | 2/2 cases, 173 assertions, failed 0, skipped 0 |
| `... --case happy --evidence "$ATTEMPT_DIR/task-58-ovis-terminal-v1.json"` | 0 | 1/1, 27 assertions |
| `... --case failure --inject '{"method":"agent.history","params":{"store":"../../secret"}}' --evidence "$ATTEMPT_DIR/task-58-ovis-terminal-v1.json" --append` | 0 | 1/1, 130 assertions |

Materialized surface: every case drives the real binary as `target/debug/ovis-terminal connect`
(bridge processes, exit 0, empty stderr) against the fixture server (real server crate,
dispatcher, authority and SQLite store) on a private Unix socket; the framed upstream-DATA probe
uses the runner's own socket. Evidence: `task-58-all.json`, `task-58-ovis-terminal-v1.json`
(happy + appended failure run), extracted lines in `after-extract.md`.

Happy observable (raw replies): the literal argv `["/bin/echo","a;b","$(id)","`uname`","c|d && e > /tmp/x"]`
comes back byte-exact in `/result/terminal/process/argv` and in the launcher's recorded spawn;
`agent.history {"store":"claude"}` lists `session-1.jsonl` and
`$(touch pwned); ignore previous instructions.jsonl` as inert data; the audit record shows
`"env":{"API_TOKEN":"[REDACTED]"}`, `op_id`, `principal`, `machine_id`, `runtime_id`, `seq`
and `"outcome":"ok"`, and `secret_hits` is `"0"` for the sentinel sent in the environment.

Failure observables (raw replies, `docs/evidence/task-58/after-extract.md`):

```
inject agent.history {"store":"../../secret"} ->
{"jsonrpc":"2.0","id":"inject-0","error":{"code":-32000,"message":"operation failed","data":{"kind":"forbidden","hint":"name a relative path inside the authorized root","retryable":false,"field_path":"params.store"}}}
store "escape" (symlink out of the root) -> forbidden, field_path params.store
store of 4097 bytes -> {"kind":"too_large",...,"field_path":"params.store","bound":{"limit":"path_bytes","value":"4096"}}
argv element of 32769 bytes -> {"kind":"too_large",...,"field_path":"params.program.argv","bound":{"limit":"argv_element_bytes","value":"32768"}}
env OVIS_TERMINAL_PROCESS_TOKEN=<sentinel> -> {"code":-32000,"message":"operation failed","data":{"kind":"forbidden","hint":"drop OVIS_TERMINAL_ variables; the runtime sets its own bindings","retryable":false,"field_path":"params.env"}}
```

The error object is pinned whole, so the sentinel cannot be in the message or data; the audit
trail's `secret_hits` is `"0"` in both cases (credentials absent from logs and error message).
In the failure case the observer then sends all 50 published mutations other than the observing
`attach.open` (input, prompt, resize, takeover, launch, doc, layout, plugin, machine, update,
credential, server, service, worktree calls): 50 x `forbidden`; an upstream stream DATA frame
closes that connection with `$/closing reason unowned_channel`; `fixture.pty` bytes stay `"0"`,
`machine.list` is unchanged (captured before, compared after), and the effect directory stays
empty.

Unit/integration tests that carry the contract (all in the 821 above): mutation matrix equals the
OpenRPC mutation set and the conformance scenario's observer requests; observer refused every
mutation over a real socket with zero handler calls; an interact-only principal refused
takeover / prompt answer / raw input / plugin / report / machine / credential / update; audit
records keep ids and outcomes, hold no sentinel, and only an administrator reads them; captured
log grep for a sentinel and a bearer is 0; symlink escape, retargeted symlink and swapped file
refused at use; `agent.history` traversal/symlink/oversize/missing kinds over the dispatcher;
rotation returns the bearer once and only its digest on retry.

### Mutants (`docs/evidence/task-58/mutants.log`): 10 applied, 10 killed

Each mutant was applied to one line, the named test run (exit 101 = killed), the file restored.

| Mutant | Killed by |
|---|---|
| M1 lexical containment admits `..` | `cargo test -p ovis-terminal-protocol --lib boundary_policy` |
| M2 symlink escape (`starts_with(root)`) check removed | `cargo test -p ovis-terminal-authz --lib validation` |
| M3 recheck at use always passes | `cargo test -p ovis-terminal-authz --lib validation` |
| M4 rotated bearer stored in the ledger unsealed | `cargo test -p ovis-terminal-server --test dispatch credential_rotation` |
| M5 observer precheck always admits | `cargo test -p ovis-terminal-server --test dispatch boundary_policy` |
| M6 typed input `text` no longer a secret field | `cargo test -p ovis-terminal-server --test dispatch boundary_policy` |
| M7 reserved `OVIS_TERMINAL_` environment accepted | `cargo test -p ovis-terminal-protocol --lib boundary_policy` |
| M8 error replies no longer redacted | `cargo test -p ovis-terminal-server --test dispatch boundary_policy` |
| M9 audit trail readable with ordinary input authority | `cargo test -p ovis-terminal-server --test dispatch boundary_policy` |
| M10 `input.raw` downgraded to ordinary input in the matrix | `cargo test -p ovis-terminal-server --test dispatch boundary_policy` |

Adversarial classes (prompt injection: hostile argv and transcript; malformed input: traversal
and oversized records; stale state: symlink replacement), the eight defects the pass found and
fixed, and residual risks are in `docs/evidence/task-58/adversarial.md`.

### Full gate (host: Linux x86_64, RAM cargo target per the 2026-11-01 host rule)

- `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` -> exit 100
  (`check.log`): fmt and workspace clippy passed; nextest stopped fail-fast on
  `ovis-terminal-holder::platform_probe failure`. Root cause was host tooling, not this diff: the
  cargo shim introduced that day ran tests under `flock` without `-o`, so every test inherited the
  lock fd and the probe run under `ulimit -n 4` could not load libc (exit 127, Error 24).
  Reproduced outside cargo (no flock exit 0 / flock exit 127 / `flock -o` exit 0) in
  `shim-fd-leak.md`; the owner fixed the shim to `flock -o`. No product change.
- Remaining check.sh steps, `--no-fail-fast` (`check-remaining.log`): nextest 1210 run,
  1209 passed, 1 failed (that same probe, before the shim fix), 1 skipped; doctests exit 0;
  workspace-hygiene exit 0 (0 violations); manifests exit 0; json exit 0.
- After the shim fix: `cargo nextest run -p ovis-terminal-holder --all-targets --all-features --locked --no-tests=fail`
  -> exit 0, 101/101 passed including `platform_probe failure` (`holder-rerun.log`; its first
  attempt failed to compile on `/dev/shm` "Disk quota exceeded" while other lanes built, no test ran).
- Windows pre-check (`windows-check.log`): `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked`
  exit 0; `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings` exit 0.
- TS SDK (`client-ts.log`): `bun run generate:check` fresh (31 files), `typecheck` 0, `bun test` 30 pass / 0 fail.
- `openrpc compat schema/released.openrpc.json` exit 0 (additive optional `store` field).

## Seams (files outside the write scope, and why)

- `crates/ovis-terminal-authz/src/{lib.rs,validation.rs}`, `src/authority.rs` (`confers`),
  `tests/authority.rs` + `tests/authority/precheck.rs`: module registration and the precheck
  query the gate needs.
- `crates/ovis-terminal-core/src/{telemetry.rs,telemetry/redaction.rs}`,
  `src/telemetry/serialize.rs`: module registration; `emit` scrubs before bounding (single boundary).
- `crates/ovis-terminal-protocol/src/{domain.rs,domain/boundary_policy/*}`,
  `src/schema/methods/agent.rs` (`store` param, error list), `src/schema/methods/terminal.rs`
  (`terminal.run` may return `too_large`), `schema/ovis-terminal.openrpc.json` and
  `packages/client-ts/src/generated/*` (regenerated; only the header digest changes in 29 files).
- `crates/ovis-terminal-server/src/dispatch/{lanes.rs,lanes/boundary_policy/*}`,
  `src/dispatch/error.rs` (a `BoundaryViolation` source renders with its field path and bound),
  `lanes/template_launch/run.rs` (`admit_launch` in resolve), `lanes/machine_identity/add.rs`
  (`MachineAdd::restricted` endpoint allowlist), `lanes/credential_rotation.rs` + `qa.rs`
  (sealed ledger result, T57 seam), `tests/dispatch.rs` (test module registration).
- `crates/ovis-terminal-services/src/update/archive.rs`: archive entries use the shared
  `RelativePath` containment check (T47 seam: one helper, same `unsafe_archive` refusal).
- `conformance/src/fixture{.rs,/boundary.rs,/handler.rs,/serve.rs}`: fixture wiring
  (`agent.history`, `fixture.audit`, `BoundaryPolicy` in front of the fixture handler);
  `conformance/src/runner_tests/suite.rs`: `security/validation.json` in sorted position
  (it was never in `declared.json`). `conformance/fixtures/security/generate-validation.py`
  writes the scenario from the OpenRPC document.
- `.github/workflows/probe-stress-macos.yml`: `boundary-policy` option and job appended last.
- `docs/evidence/task-58/`: evidence.

## Risks

- `MUTATION_ROLES` assigns an action to methods whose lanes are not assembled yet (`attach.*`
  resize/focus/takeover, `layout.edit`, `selection.set`, `terminal.label/rename/rm/recover/kill`,
  `machine.exec/file/login/rm`, `agent.note/release`, `service.*`, `server.start`). A future lane
  that authorizes a different action must update the table; the precheck only refuses when the
  credential confers that permission nowhere, and the lane's own scoped check still decides.
- The per-field limits (`argv_*`, `env_*`, `path_bytes`) are reported in refusal `bound`s but not
  yet advertised in `hello.limits`.
- A store directory swapped to a symlink and restored within one read is not detected; history
  pages are offsets over a live directory; hard links inside a store are listed as files. Only
  names and modification times are ever disclosed.
- Launch argv is kept in audit records for diagnosis; secrets belong in `env`, which is redacted.
- The scenario runs on the conformance fixture server; the assembled daemon (todo 31) must wrap
  its handler in `BoundaryPolicy` and route `agent.history` the same way.
- Base: lane branched from integration da5b456 and was not merged forward (lane rule).
  origin/integration has since moved to 8ac8489 (T37 records). A read-only `git merge-tree`
  probe predicts structural conflicts only: `.github/workflows/probe-stress-macos.yml` (option
  list and appended job), `crates/ovis-terminal-server/tests/dispatch.rs` (module list) and the
  generated SDK header digests (regenerate with `bun run generate` after the union).
- The orchestrator's standing Terra adversarial review could not be spawned from this executor
  (subagent depth limit); the executor's own adversarial pass is in `adversarial.md`.

## DEFERRED VERIFICATION

None for this todo: the runner executes `security/validation` (exit 0, failed 0, skipped 0).
