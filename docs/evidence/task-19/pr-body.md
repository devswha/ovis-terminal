## Before

absent. GAP-129, GAP-130, GAP-131 and GAP-136 had no comparable capability in the new runtime, and no legacy execution is claimed. For context only: the bridge trap this replaces is RT-HD-07 (`docs/evidence/baseline/smoke/herdr-h08-attach.log`), where `--takeover` displaces the current holder with no authorization (EXCL-007).

## After

`crates/ovis-terminal-authz` now holds the authority boundary: endpoint admission, then runtime-scoped credentials, then grants of one permission over one resource scope, then a decision token that is rechecked at the commit/write boundary.

- `EndpointPolicy::admit` checks local endpoint evidence, which the OS layer will supply:
  - Unix socket and directory must be owner-only (no group/other bits, owned by the runtime account), and the `SO_PEERCRED` uid must be the owner.
  - Named pipes: the DACL must be owner-only (a null DACL or any foreign allow ACE is refused), `PIPE_REJECT_REMOTE_CLIENTS` must be set, and the client SID must be the owner.
  - `PipeNamespace` enforces the first-instance lifecycle: the name is reclaimed with `FILE_FLAG_FIRST_PIPE_INSTANCE` after every full close, and additional instances serve concurrent and reconnecting accepts.
  - Admission grants nothing: an admitted local owner or remote peer without credential material gets `unauthenticated`.
- `Authority` (one per runtime lifetime) bootstraps an owner credential and issues delegated credentials.
  - A credential holds explicit grants (`observe`, `interact`, `raw_input`, `takeover`, `prompt_answer`, `report`, `admin`, `plugin`) over a machine, workspace, tab, pane, terminal, document or plugin scope.
  - No permission implies another, and `plugin` and `admin` are separate.
  - Issuers can delegate only grants, machine and expiry they hold.
  - Bearer secrets are 32 bytes from OS randomness, compared in constant time, and redacted from `Debug`.
- `authorize` returns a `Decision` bound to the authority instance, the credential epoch, the action and the resolved target, including the terminal incarnation.
  - `recheck` at the write boundary fails with `credential_revoked`, `credential_expired` or `decision_mismatch` when any of those changed.
  - The decision cannot be constructed, cloned or deserialized outside the crate.
- u64 counters (`epoch`, `expires_at_ms`) cross JSON as canonical decimal strings, and a numeric form is `invalid_params` with its field path.

Real-surface command (Before/After requirement), exit 2:

```text
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-19 --isolated --case happy --evidence "$ATTEMPT_DIR/task-19-ovis-terminal-v1.json"
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
EXIT=2
```

DEFERRED VERIFICATION (runner lands in Todo 21): there is no assembled `ovis-terminal` binary or real-socket runner yet (plan line 275), so no CLI/socket transcript is claimed. The attempt is recorded verbatim in `docs/evidence/task-19/conformance-attempt.log`. `docs/evidence/task-19/task-19-ovis-terminal-v1.json` is an executor summary of these logs, not runner output. Component evidence, run on Linux 7.0.0-38-generic x86_64 with rustc 1.99.1, cargo 1.99.1 and cargo-nextest 0.9.150:

```text
$ cargo test -p ovis-terminal-authz --test authority happy -- --exact --nocapture
observer read+subscribe t1,t2 ok; observer input -> forbidden; operator input+resize t1 ok; operator outside t1 -> forbidden
test happy ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 23 filtered out
EXIT=0
```

## QA

- `cargo test -p ovis-terminal-authz --test authority happy -- --exact --nocapture`: exit 0. It asserts that a machine-scoped observe principal is allowed `read` and `subscribe` on t1 and t2 and that each decision passes `recheck`, and that the same principal gets `forbidden` for `input`. A terminal-scoped interact principal is allowed `input` and `resize` on t1 (with `recheck`) and gets `forbidden` for `input` on t2 and for `launch` on a workspace.
- `cargo test -p ovis-terminal-authz --test authority failure -- --exact --nocapture`: exit 0. It asserts that an input decision made before the owner revoked the credential fails its write-time `recheck` with `credential_revoked`. It also asserts that an observe principal gets `forbidden` for input, resize, launch, document write, layout change, takeover and prompt answer (7 typed rejections, printed in the log).
- Every step of `conformance/scenarios/part-a/task-19.json` (13 named tests, each with its expected exit code and stdout lines) was executed: 13/13 PASS (`scenario-steps.log`).
- `cargo nextest run -p ovis-terminal-authz`: exit 0, 35 passed: 6 unit tests, 24 in `tests/authority.rs`, 5 in `tests/endpoint.rs`.
- `cargo clippy -p ovis-terminal-authz --all-targets -- -D warnings`: exit 0.
- `bash scripts/check.sh`: exit 0, all steps passed. Workspace nextest: 267 passed, 1 skipped (the intentional `crash_child` fixture). Hygiene: 0 violations in 296 files, 16 manifests, and 25 JSON files checked (the run before the evidence JSON was copied in).
- `cargo metadata --locked --format-version 1 --no-deps`: exit 0. `origin/integration` was still at `c1dd6cf` when this branch was pushed, so no merge was needed.

## Adversarial

- Stale state:
  - `revocation::revocation_reaches_a_recheck_on_another_thread`: a decision moved to a writer thread is rechecked after the owner revokes, and the result is `credential_revoked`.
  - `expiry_between_decision_and_commit_rejects_the_write`: `recheck` and re-authentication at expiry both return `credential_expired`.
  - `decision_from_another_authority_is_rejected` and `commit_must_match_the_decided_action_target_and_incarnation`: reusing a read decision for a write, committing to t2, and committing to incarnation 2 each return `decision_mismatch`.
  - `credential_is_scoped_to_its_runtime_lifetime`: a bearer from lifetime A, including one with a colliding credential id, is `invalid_credential` in lifetime B.
- Malformed input:
  - `issuance::forged_scope_documents_are_rejected_with_field_paths`: an unknown permission, unknown scope kind, smuggled scope field, extra root field or empty principal each returns `invalid_params` with its path.
  - `forged_bearer_tokens_are_rejected`: an empty bearer is `unauthenticated`; garbage, a reversed secret, an unknown id, a truncated secret or an uppercased secret is `invalid_credential`.
  - `numeric_counter_is_rejected_with_its_field_path`, the u64::MAX expiry test, and a proptest over every u64 cover counter encoding.
  - `forge-probe.log` (compile-time): building a `Decision` literal fails with E0451, an owned `Session` clone fails because `Session` is not `Clone`, and deserializing an `IssuedCredential` fails because it is not `Deserialize`.
- Prompt injection:
  - `issuance::payload_from_an_unprivileged_principal_grants_nothing`: a well-formed grant document asking for `admin`, presented by a terminal-scoped interact principal, is refused with `forbidden`, and `administer` stays `forbidden` afterward. Authority comes only from issued credentials; no request payload is an input to `authorize`.
- Mutation (`mutation.log`): making `recheck` skip the credential check fails `failure`. Making grants ignore their resource scope fails `happy`. Making `input` need only `observe` fails `failure`. Restored, all 24 tests in `tests/authority.rs` pass.

## Contract checklist

1. Every u64 on the wire is a canonical decimal string: `CredentialEpoch` wraps protocol `DecimalU64` and `expires_at_ms` is protocol `TimestampMs`. A numeric form is `invalid_params`. `u64::MAX` round-trips exactly (`issuance::expiry_preserves_u64_max_exactly`, `identity::tests::epoch_is_a_decimal_string_and_saturates_at_the_top`), and a proptest covers the whole u64 range.
2. `AuthzError` is a single `thiserror` enum with `kind()` and `hint()`. `InvalidParams` keeps the `serde_json::Error` as `#[source]`. No `Display` text repeats a kind token.
3. `PrincipalId`, `CredentialId`, `CredentialEpoch`, the private `AuthorityId`, `UnixUid` and `WindowsSid` are distinct types. Machine, terminal, workspace, tab, pane, document, plugin and lifetime identities reuse the protocol newtypes. `Option` is used only for an unplaced terminal and for `AuthzError::field_path()`.
4. Tests assert typed kinds, state transitions (issue, revoke, recheck) and round trips; proptest covers the decimal counter.
5. No `#[allow]`, `#[expect]`, `unwrap`/`expect` in production, numeric `as`, or placeholder TODOs. Every file is at most 250 pure LOC (hygiene: 0 violations), and every function takes at most 3 parameters.
6. Evidence states only what ran. The real-binary conformance run is DEFERRED to Todo 21 and its exit-2 attempt is pasted above.

## Seams

No landed crate's public API changed. The only manifest edits are the `ovis-terminal-authz` dependency seam: `serde`, `serde_json`, `serde_path_to_error`, `thiserror`, `uuid` with `v4` (the OS randomness for secrets; the workspace already locks `uuid` and `getrandom`), and `proptest` as a dev-dependency. `Cargo.lock` changes only this crate's dependency list; no new package entered the lock.

## Risks

- `credential_revoked`, `decision_mismatch`, `grant_exceeds_issuer`, `endpoint_insecure` and `peer_rejected` are stable `AuthzError::kind()` tokens but are not yet in protocol `DomainErrorKind` or the published OpenRPC error catalog. The lane that maps `AuthzError` onto the wire (todos 13/14) must add them there.
- Credentials live in memory, per `Authority` instance. A control-server restart invalidates every credential (fail-closed). Durable credentials and rotation are todo 57.
- The endpoint checks validate OS evidence they are given. Collecting it (`SO_PEERCRED`, socket/dir `stat`, pipe security descriptors, `FILE_FLAG_FIRST_PIPE_INSTANCE`) belongs to the listener todos (13, 51, 54). No Windows host ran these checks here.
- When the credential table is full, revoked and expired records are pruned, so a later presentation of one gets `invalid_credential` instead of `credential_revoked` or `credential_expired` (still denied).
- The scaffold's `ovis-terminal-store` dependency is kept for the layering record, but this crate does not persist anything yet.
