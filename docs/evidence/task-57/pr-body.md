## Summary

Plan todo 57: runtime-scoped credential rotation and revocation with principal epochs, a
per-request credential recheck so no socket keeps authority, and a recheck of queued input
writes that reports each one as cancelled before commit, already committed (with the
holder's delivery facts) or of unknown outcome.

Closes GAP-070, GAP-071, GAP-073, GAP-074, GAP-075, GAP-078, GAP-084, GAP-131, GAP-133.
Delivers IDEAL-048, IDEAL-049, IDEAL-050, IDEAL-051, IDEAL-052, IDEAL-055, IDEAL-061, IDEAL-103, IDEAL-104, IDEAL-106.

- `ovis-terminal-authz`: `PrincipalEpoch` per principal (starts at 1, advances on every
  revocation or rotation, never resets); `Authority::rotate` issues a replacement with the
  same principal, grants and expiry and withdraws the old credential under one write lock,
  compare-and-swapped on the principal epoch the caller read (`version_conflict` naming the
  current epoch otherwise; CAT-3229 pattern); `Authority::withdraw` (revoke + standing);
  `Authority::validate` rechecks the credential behind a session per request.
  `credential_revoked` now carries the principal's current epoch. Bearer secrets are still
  compared byte-for-byte without early exit (`CredentialSecret::matches`).
- `ovis-terminal-protocol`: `credential.rotate` / `credential.revoke` catalog methods and
  records (`domain::credential_rotation`), `CommitRace` (`cancelled_before_commit` /
  `already_committed{written_bytes, output_offset}` / `outcome_unknown`), `BearerText`
  (redacted `Debug`), scalars `PrincipalEpoch` / `CredentialEpoch` (decimal strings), ids
  `CredentialId` / `PrincipalName`, `ErrorData.principal_epoch`. OpenRPC regenerated;
  `openrpc compat schema/released.openrpc.json` exits 0.
- `ovis-terminal-server`: `authn::Revalidated<H>` wraps any `RequestHandler` and refuses a
  revoked or expired session before the request is decoded; `dispatch::lanes::credential_rotation`
  (admin-only, one commit unit, refusals kept as the op's typed failure, `QueuedWrites`
  seam rechecked after every withdrawal); dispatch errors from the authority carry
  `principal_epoch`; `Execution::session()`.
- `ovis-terminal-core`: `input::authority::{recheck, race, raced}` fence a terminal's
  sequencer at once after a credential change and classify every affected write;
  `Sequencer::with_holder()`.
- Conformance: executable `security/rotation` scenario; the fixture's `rotation` terminal
  (terminal:6) admits `input.send` through the real `Sequencer` (unleased lease table, real
  VT modes) and its holder stand-in writes only on `fixture.deliver`, which is the
  pre-commit barrier the scenario holds.

## Before

`absent` (no comparable baseline capability; not an executed failing test). Gap baseline:
GAP-070, GAP-071, GAP-073, GAP-074, GAP-075, GAP-078, GAP-084, GAP-131, GAP-133.

## After

Host: `Linux 7.0.0-38-generic x86_64`; binary `target/debug/ovis-terminal`
sha256 `f2f430a36a0868639acdbc0004de0476939d4e38ddd9e87f2adc03b270f7c119` (also recorded under `/binary` in the evidence).

```
$ cargo build --locked -p ovis-terminal-cli --bin ovis-terminal                         # exit 0
$ cargo test -p ovis-terminal-authz -p ovis-terminal-core -p ovis-terminal-server -p ovis-terminal-protocol --all-targets   # exit 0
$ cargo clippy -p ovis-terminal-authz -p ovis-terminal-core -p ovis-terminal-server -p ovis-terminal-protocol --all-targets -- -D warnings   # exit 0
$ bash scripts/check.sh                                                                   # exit 0 (915 tests run: 915 passed, 1 skipped)
$ RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked   # exit 0
```

Happy (exit 0):

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario security/rotation --isolated --case happy --evidence "$ATTEMPT_DIR/task-57-ovis-terminal-v1.json"
{"case":"happy","error":null,"evidence":"/home/devswha/workspace/ovis-terminal-wt/lane-t57/.omo/evidence/ovis-terminal-v1/task-57/task-57-ovis-terminal-v1.json","evidence_written":null,"result":"passed","scenario":"security/rotation","totals":{"assertions":"87","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
# exit=0
```

Failure (exit 0):

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario security/rotation --isolated --case failure --inject '{"method":"prompt.answer","params":{"prompt_id":"p1","version":1},"principal_fixture":"revoked-token"}' --evidence "$ATTEMPT_DIR/task-57-ovis-terminal-v1.json" --append
{"case":"failure","error":null,"evidence":"/home/devswha/workspace/ovis-terminal-wt/lane-t57/.omo/evidence/ovis-terminal-v1/task-57/task-57-ovis-terminal-v1.json","evidence_written":null,"result":"passed","scenario":"security/rotation","totals":{"assertions":"50","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
# exit=0
```

`--case all` (exit 0): `{"case":"all","error":null,"evidence":"/home/devswha/workspace/ovis-terminal-wt/lane-t57/.omo/evidence/ovis-terminal-v1/task-57/task-57-all.json","evidence_written":null,"result":"passed","scenario":"security/rotation","totals":{"assertions":"140","cases":"2","declared_only":"0","failed":"0","passed":"2","skipped":"0"}}`

Materialized commands: every connection runs through `target/debug/ovis-terminal connect`
(the bridge) against the fixture socket; the evidence records each bridge argv, exit code
and the byte-exact transcript. Credentials are redacted in every transcript (the runner now
also scrubs any `ovt1.c<n>.<64 hex>` bearer a reply hands out); `grep` finds no bearer in
`docs/evidence/task-57/task-57-ovis-terminal-v1.json`.

Observed on the wire (from that evidence):

- writer `input.send "ok"` on `rotation` → receipt `queued_seq "1"`, `delivery {"state":"queued"}`
  within the 1000 ms bound while `fixture.pty` reads `"0"`; `fixture.deliver` then settles it
  `{"outcome":"already_committed","written_bytes":"2","output_offset":"0"}`.
- `"echo a\nrm -rf b"` with `submit:none` → `paste_refused`; `"\u001b[2J"` → `invalid_encoding`; no bytes.
- writer `input.send "late"` queued, owner `credential.rotate {credential:"c3", principal_epoch:"1"}`
  → `issued.credential "c4"`, `replaced.withdrawn {credential:"c3", epoch:"2"}`,
  `standing {principal:"writer", principal_epoch:"2"}`, `writes[0] {op_id: late, queued_seq "2",
  race {"outcome":"cancelled_before_commit","reason":"credential_revoked"}}`; releasing the
  boundary writes nothing (`fixture.pty` stays `"2"`).
- the writer's still-open socket: `op.get` → `credential_revoked`, `principal_epoch "2"`;
  reconnecting with the old bearer: hello `credential_revoked`; the replacement bearer
  reconnects, reads its principal's earlier op, sends again and is delivered (pty `"5"`); it
  may not rotate itself (`forbidden`); a rotation naming epoch `"1"` → `version_conflict`,
  `principal_epoch "2"`.
- failure: after `credential.revoke c3`, `prompt.answer {prompt_id:"p1", version:1}` (plan's
  numeric literal) and `version:"1"` over the revoked socket → `credential_revoked`,
  `principal_epoch "2"`, no result; `prompt.get p1` → `state "open"`, `version "1"`,
  `answer null`, `closed_seq null`; the queued write was reported `cancelled_before_commit`
  and `fixture.pty` is `"0"`.

## QA

Tests added (all pass in the runs above):

- authz `tests/authority/rotation.rs`: rotation swaps credentials atomically — old bearer,
  old session and a decided-but-uncommitted write all fail `credential_revoked` naming
  principal epoch 2, replacement authenticates as the same principal; rotation keeps
  `prompt_answer` / `interact` / `takeover` separate; stale epoch → `version_conflict`,
  rotating a replaced or revoked credential → `credential_revoked`, expiry beyond the issuer
  → `grant_exceeds_issuer`; only an administrator rotates or withdraws, expiry refuses
  without moving the epoch, repeated withdrawal advances the epoch; a live id with a wrong
  secret → `invalid_credential`. `authority::epochs` unit test (start, advance, saturate).
- core `input/authority/qa.rs`: withdrawal reports queued writes cancelled and the write with
  the holder `outcome_unknown`, returns a newer-epoch empty fence write, then the holder's
  acknowledgement resolves it `already_committed {written_bytes 3}` and nothing is resent;
  withdrawing one writer leaves another's queue and order intact.
- server `lanes/credential_rotation/qa.rs` (real socket, dispatcher, SQLite ledger):
  revoked connection refused before decoding with `principal_epoch`, old bearer's hello
  refused, the lane rechecks queued writes exactly once; rotation hands out a working
  replacement, self-rotation forbidden, stale epoch conflicts, retry with the same op_id
  returns the same result; a write parked before commit is refused after revocation with
  `principal_epoch "2"` and its effect never ran.
- protocol `domain/credential_rotation/tests.rs`: `u64::MAX` principal epoch round trip,
  numeric epoch and unknown fields rejected, race outcomes tagged, proptest round trip of
  revoke results.
- conformance `engine/redact.rs`: bearer-shaped tokens scrubbed, look-alikes kept.

## Adversarial

- cancel/resume and stale state — revoke between check and write: the parked write
  (server test) and the queued sequencer write (scenario, core test) write zero bytes.
- repeated interruptions — reconnect with the old token: hello `credential_revoked` (scenario
  both cases, server test).
- misleading success — `already_committed` carries `written_bytes` / `output_offset`
  (scenario `fixture.deliver`, core test); an in-flight write is `outcome_unknown`, never
  reported cancelled or delivered without holder evidence.
- mutation probe: with `Revalidated::call`'s `validate` line removed, the failure command
  exits 1 (`inject[0]/error/data/kind`: expected `credential_revoked`, actual
  `invalid_params`); logs `docs/evidence/task-57/mutation-no-revalidate.stdout` and `.json`.

## Contract checklist

1. Every new u64 on the wire (`principal_epoch`, `epoch`, `queued_seq`, `written_bytes`,
   `output_offset`, `admitted_bytes`) is a decimal string; numeric → `invalid_params`;
   `u64::MAX` round trip tested.
2. New refusals are `AuthzError` variants (`PrincipalEpochConflict`, `CredentialRevoked.principal_epoch`)
   with `kind()` / `hint()`; Display does not repeat the kind.
3. `PrincipalEpoch`, `CredentialEpoch`, `CredentialId`, `PrincipalName`, `BearerText`,
   `FenceToken` are distinct newtypes.
4. Tests assert state transitions and typed kinds over real sockets/ledgers; proptest round trip.
5. No `#[allow]`, no placeholder TODO; `workspace-hygiene` 0 violations.
6. Holder wire generation stays 2.

## Seams (public API changes in landed crates)

- authz: `CredentialRevoked` gains `principal_epoch`; new `AuthzError::PrincipalEpochConflict`
  (kind `version_conflict`), `AuthzError::principal_epoch()`; `Authority::{rotate, withdraw, validate}`;
  `IssuedCredential::expires_at()`; `Rotation`, `Rotated`, `Withdrawn`, `PrincipalEpoch` exports.
  Revoking twice still advances the credential epoch (unchanged) and now also the principal epoch.
- protocol: `ErrorData.principal_epoch` (additive, optional) + `BoundaryError::with_principal_epoch`;
  scalars `PrincipalEpoch`, `CredentialEpoch`; ids `CredentialId`, `PrincipalName`;
  methods `credential.rotate`, `credential.revoke` (regenerated `schema/ovis-terminal.openrpc.json`).
- server: `pub mod authn`; `Execution::session()`; `lanes::credential_rotation`.
- core: `input::authority`; `Sequencer::with_holder()`.
- conformance runner: `Principal::Writer` (fixture issues a `writer` credential: observe +
  interact + prompt_answer), `connect.bearer` + `replies.keep_bearer` to reconnect with a
  rotated bearer, bearer scrubbing in transcripts, `fixture.deliver`, fixture terminal
  `rotation` (terminal:6), the fixture handler is wrapped in `Revalidated`.

**Assembly note for todo 31:** the binary's handler must be wrapped in `authn::Revalidated`
and register the credential lane with a `QueuedWrites` over the runtime's sequencers.

## Risks

- The rotate op's committed result (and so `op.get` by the same principal) contains the
  replacement bearer in the SQLite ledger, so an idempotent retry can return it; the store
  is owner-only. Authority state is in memory, so the bearer dies with the runtime lifetime.
- If the commit unit's SQLite commit fails after the in-memory rotation, the replacement is
  not returned; the caller sees the principal epoch moved and rotates again.
- Principal epochs are kept for every principal issued to during the runtime lifetime.
- No `authority_lapsed` kind was added: lapses surface as the published `credential_revoked`,
  `credential_expired` and `forbidden` kinds.
- `origin/integration` moved to a74fba7 after this lane's base (0d3022c); per the lane rule
  it was not merged here. Only `Cargo.lock` overlaps (separate hunks).
- The macOS stress job is not in this PR's workflow file (integration's a74fba7 rewrote it);
  the job block is in `docs/evidence/task-57/stress-job.yml` for the orchestrator union and
  was dispatched from a stress-only branch: run https://github.com/devswha/ovis-terminal/actions/runs/41247294624 (branch `lane/t57-stress` = this head + integration a74fba7's workflow + the job; result recorded after it finishes).
