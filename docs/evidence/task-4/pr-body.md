feat(schema): generate openrpc and enforce released schema compatibility

Round-4 repair of todo 4 (`[~]`). It closes the round-3 gate blocker R3-B1 (IDEAL-113) and adds the baseline-amendment ledger. Everything else from PR #8 is unchanged. `schema/released.openrpc.json` is untouched.

## Before

- Round-3 gate (`.omo/evidence/ovis-terminal-v1-task-4-round-3-gate-review.md`, R3-B1): `check_compatibility` returned `Ok(())` for two shapes that narrow accepted input.
- `docs/evidence/task-4/round4-before-composition.log` (base `759c85a`, new tests, unmodified code): `cargo test -p ovis-terminal-protocol --test schema_composition` exits **101**, and both tests find no breaks (`[]`):
  - `pattern_beside_a_nullable_union_is_narrowed` adds `pattern: "^p[0-9]+$"` beside the nullable `anyOf` of `PageRequest.after`. The released input `{"after":"old-token","limit":1}` decodes under the released Rust type.
  - `bound_inside_a_nested_intersection_is_narrowed` adds `allOf: [{"allOf": [{"maximum": 80}]}]` to `Geometry.cols`. Integer 120 was accepted before.
- `docs/evidence/task-4/round4-before-amendments.log`: an edited copy of the released baseline passes `target/debug/openrpc compat` (**exit 0**), and `schema/BASELINE-AMENDMENTS.md` does not exist.
- Historical baseline (unchanged): RT-HD-12 `docs/evidence/baseline/herdr-h10-errors.log:9`.

## After

All commands below were run on Linux x86_64 (`uname -sm`).

- `compat/facts.rs` now flattens a schema as a conjunction. A value matches the node's own keywords, every `allOf` member, one `anyOf` member and one `oneOf` member. Each alternative is one combination of those parts, built recursively, so keywords beside a union and nested intersections are kept. A type array still splits into one alternative per type.
- Products are bounded at 256 alternatives. Beyond that the typed error `SchemaError::TooManyAlternatives` (`schema_alternatives_exceeded`) is returned.
- A combined alternative keeps only the references both sides entered plus the larger depth. Cycle detection and the 64-level depth bound still apply.
- `schema/BASELINE-AMENDMENTS.md` starts from the digest of the baseline as first committed (PR #8). It records the PR #36 / todo 22 amendment: on 2026-10-31 the hello limits `event_replay_events`, `history_lines` and `max_in_flight_requests` became `$ref Count` decimal strings, `Count` was added, and the `x-limits.defaults` values became strings. The entry gives date, PR, fields, reason and the previous/current digests.
- `check_amendments` (new public API) checks the ledger against the baseline. Digests are SHA-256 over compact JSON. It refuses:
  - a baseline whose digest is not the chain head, with `baseline_amendment_missing`, naming the current digest and the `previous` digest the missing entry must continue from;
  - an entry with a missing key or a broken chain, with `baseline_amendments_malformed` at `<entry>.<key>`.
- `openrpc compat <released>` runs that check against `BASELINE-AMENDMENTS.md` in the same directory, after the compatibility comparison. The `schema` test `happy` runs it on the shipped files.

```
$ target/debug/openrpc compat schema/released.openrpc.json
{"compatible_with":"schema/released.openrpc.json"}
EXIT=0
$ target/debug/openrpc check schema/ovis-terminal.openrpc.json
{"bytes":550415,"fresh":"schema/ovis-terminal.openrpc.json"}
EXIT=0
$ # copy of released with Count.description edited, shipped ledger beside it
$ target/debug/openrpc compat /tmp/tmp.ldDQb15yKD/released.openrpc.json
{"breaks":[],"hint":"add an entry to schema/BASELINE-AMENDMENTS.md with date, pr, fields, reason, previous and current digests","kind":"baseline_amendment_missing","message":"released baseline sha256:2bd3d2d55a66f1079c3a64fb6ab0710cce271330ea26d85de75bf020f95e7ae7 has no amendment entry with previous: sha256:362980ac3f118f9866ae95f33defb0a5542b4d8a20825432a0a580ec809ae971"}
EXIT=1
```
(`docs/evidence/task-4/round4-after-cli.log`)

- `cargo nextest run -p ovis-terminal-protocol`: `168 tests run: 168 passed, 0 skipped`, **exit 0**.
- `cargo clippy -p ovis-terminal-protocol --all-targets -- -D warnings`: **exit 0** (`round4-acceptance.log`).
- `bash scripts/check.sh`: **exit 0**, `check.sh: all steps passed`, hygiene `0 violation(s) in 953 file(s)` (`round4-check-script.log`).
- `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked`: **exit 0** (`round4-windows-check.log`).

### DEFERRED VERIFICATION (runner lands in Todo 21)

Todo 21's runner has landed. `part-a/task-4` is still a declaration in `conformance/scenarios/declared.json`: owner 4, waiting on RPC routes from todos 23, 25 and 28-31. The runner therefore refuses it. Verbatim:

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-4 --isolated --case happy --evidence $ATTEMPT_DIR/task-4-ovis-terminal-v1.json
{"case":"happy","error":{"detail":"part-a/task-4 is a declaration, not a registered executable scenario","hint":"register the scenario with \"format\": \"ovis-conformance/1\" and typed steps","kind":"scenario_not_executable","step":null},...,"result":"failed","scenario":"part-a/task-4",...}
EXIT=1
```
(`round4-conformance-attempt.log`). This round's brief limits the scope to the compat rules and the ledger, so the scenario was not converted.

## QA

- happy: `cargo test -p ovis-terminal-protocol --test schema happy -- --exact --nocapture`, **exit 0**. It prints `methods=99 events=39 schemas=353 bytes=550415` and `released baseline sha256:362980ac…`. It asserts:
  - regeneration is byte-identical twice and equals the checked-in document;
  - `verify_document` passes (unique names, references resolve);
  - `check_compatibility(released, current)` is `Ok`;
  - `check_amendments(released, ledger)` is `Ok`.
- failure: `cargo test -p ovis-terminal-protocol --test schema failure -- --exact --nocapture`, **exit 0**. It asserts:
  - the injected required param gives `schema_incompatible` / `required_param_added` at `terminal.get(pane_id)`;
  - the unknown target property gives `invalid_params` at `params.target.pane_id` with zero dispatches and JSON-RPC -32602.
- New tests (`round4-after-tests.log`):
  - `schema_composition`, 3 tests. They assert the exact rule and path: `input_pattern_changed` at `terminal.list(page).after` with no other rule; `input_bound_narrowed` at `terminal.run(geometry).cols`. A looser nested maximum (70000) and removing the pattern both yield no breaks. 2^9 combinations fail with `schema_alternatives_exceeded`; 2^8 compare with no breaks.
  - `schema_amendments`, 5 tests:
    - the shipped ledger accepts the shipped baseline;
    - the digest ignores formatting and changes with content;
    - an unrecorded edit gives `AmendmentMissing` with the exact digest and previous head, and appending a chained entry makes it pass;
    - a missing `reason`, an unchained `previous` and a missing `released` line fail as malformed at `test amendment.reason`, `test amendment.previous` and `released`;
    - the real CLI process gives `io` with no ledger, then `baseline_amendment_missing` naming the digest, then exit 0 once the entry exists.
  - `schema_properties`, 2 proptests × 256 cases. The generator gains `Within` (a child inside `allOf:[{allOf:[child]}]`) and `Guarded` (a nullable `anyOf` with an optional `pattern` beside it). Narrowing a `Guarded` string adds a sibling pattern; widening removes it.
- Mutation proofs:
  - `round4-mutation-rule-removed.log`: `facts.rs` restored to `origin/integration`. All 3 composition tests fail, and the narrowing property fails with minimal input `Within(Str{..})`, both **exit 101**. That seed is committed in `schema_properties.proptest-regressions`.
  - `round4-mutation-ledger-check-removed.log`: the ledger check removed from the CLI makes `cli_compat_requires_a_ledger_entry_for_a_changed_baseline` fail (**exit 101**).

## Adversarial

- Malformed input:
  - the closed request schema still rejects unknown target properties (the `failure` test);
  - a malformed ledger gives `baseline_amendments_malformed` with a location;
  - a combinatorial composition gives `schema_alternatives_exceeded` instead of exponential work;
  - the cyclic, deep and oversized baseline tests from round 3 still pass.
- Stale state: a released baseline edited without a ledger entry gives `baseline_amendment_missing` (library and real CLI). The released baseline is checked on every `happy` run.
- Misleading success:
  - the two R3-B1 shapes now report typed breaks instead of `Ok(())`;
  - the mutation runs show the new tests fail without the production change;
  - regeneration stays byte-stable.

## Contract checklist

1. No wire u64 counters are added. Holder wire generation is still 2. The `Count` amendment is recorded, and its decimal-string form is the checklist rule.
2. The new boundary failures are variants of the existing `thiserror` enum `SchemaError`, each with its own `kind()`/`hint()` (`schema_alternatives_exceeded`, `baseline_amendment_missing`, `baseline_amendments_malformed`). Display does not repeat the kind token.
3. There are no new identities. Digests are hex strings used only for ledger matching.
4. Tests assert behavior: typed rules and paths, digests computed from content, real CLI exit codes. Proptest covers the new composition shapes.
5. There is no `#[allow]` or `#[expect]`, no placeholder, and no prose-pinning test. The ledger test edits ledger text only to make malformed inputs.
6. The evidence above states exactly what each test asserts. The conformance attempt is recorded verbatim.

## Seams

- New public API in `ovis_terminal_protocol::schema`: `check_amendments`, `baseline_digest`, `ENTRY_KEYS`.
- New `SchemaError` variants (`#[non_exhaustive]` enum): `TooManyAlternatives`, `AmendmentMissing`, `AmendmentsMalformed`.
- `ovis-terminal-protocol` depends on workspace `sha2`, which is already in the lockfile.
- `openrpc compat <released>` now also needs `BASELINE-AMENDMENTS.md` beside `<released>`. Fixture runs that fail during comparison (malformed or cyclic) report the same errors as before.
- No other crate is touched.

## Risks

- Output comparison uses the same flattening. Keywords beside a union now apply to each output alternative too. That is more accurate; the shipped baseline still compares clean.
- Any future change to `released.openrpc.json`, including the release in todo 67, must append a ledger entry. Its digest comes from the error message or from `baseline_digest`.
- Properties stated in two conjunct parts still use the first part's schema. This predates the round and is not one of the gate's shapes.
