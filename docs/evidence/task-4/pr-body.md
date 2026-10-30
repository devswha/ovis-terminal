Plan todo 4, wave 3. Closes GAP-018, GAP-019, GAP-139, GAP-140; delivers IDEAL-008, IDEAL-112, IDEAL-113.

## Before

RT-HD-12, GAP-018, GAP-019, GAP-139, GAP-140. Baseline `docs/evidence/baseline/smoke/herdr-h10-errors.log:9` (original `/tmp/ulw-ovis-yCJzPH/evidence/smoke/herdr-h10-errors.log:9`; the plan's `docs/evidence/baseline/herdr-h10-errors.log` path lives under `smoke/` in the repo):

```text
<- {"id":"r29","error":{"code":"invalid_request","message":"invalid request: invalid type: string \"many\", expected u32 at line 1 column 96"}}
```

A wrong param type came back as `invalid_request` with serde prose and no field path. herdr also has no single versioned schema for events/channels/limits (GAP-139), no same-generation additive promise (GAP-140), reuses request/response names (GAP-019) and rejects unknown enum values (GAP-018).

## After

Linux 7.0.0-38-generic x86_64, rustc 1.99.1 (58cf8f9c3 2026-10-13). Logs in `docs/evidence/task-4/`.

| Command | Result | Log |
|---|---|---|
| `bash scripts/check.sh` | exit 0: fmt, workspace clippy `-D warnings`, nextest 183 passed / 1 skipped, doctests, hygiene 0 violations in 197 files, JSON 19, manifests, JSON | `check-script.log` |
| `cargo nextest run -p ovis-terminal-protocol` | exit 0, 85 passed | `acceptance-nextest.log` |
| `cargo test -p ovis-terminal-protocol --test schema_narrowing -- --nocapture` | exit 0, 8 passed (round-2 reproductions, cycle/depth/oversize) | `gate-round2-narrowing.log` |
| `cargo test -p ovis-terminal-protocol --test schema_properties -- --nocapture` | exit 0, 2 proptests × 256 cases (random narrowing reported, random widening compatible) | `gate-round2-properties.log` |
| `cargo run --locked --quiet -p ovis-terminal-protocol --bin openrpc -- compat schema/fixtures/cyclic-released.json` | exit 1, `{"kind":"schema_reference_cycle","message":"reference #/components/schemas/Cycle leads back to itself",...}` on stderr | `adversarial-cyclic-baseline.log` |
| `cargo test -p ovis-terminal-protocol --test schema_gate -- --nocapture` | exit 0, 11 passed (gate probes P1-P7, CLI fixtures) | `gate-regressions.log` |
| `cargo run --locked --quiet -p ovis-terminal-protocol --bin openrpc -- compat schema/fixtures/malformed-released.json` | exit 1, `{"kind":"malformed_baseline","message":"compared schema is malformed at methods[0].unknown_name",...}` on stderr | `adversarial-malformed-baseline.log` |
| `cargo run --locked --quiet -p ovis-terminal-protocol --bin openrpc -- compat schema/fixtures/retired-method-released.json` | exit 1, `schema_incompatible`, `[{"path":"retired.method","rule":"method_removed"}]` | `adversarial-removed-method.log` |
| `cargo clippy -p ovis-terminal-protocol --all-targets -- -D warnings` | exit 0 | `acceptance-clippy.log` |
| `cargo test -p ovis-terminal-protocol --test schema happy -- --exact --nocapture` | exit 0, 1 passed, 5 filtered | `qa-happy.log` |
| `cargo test -p ovis-terminal-protocol --test schema failure -- --exact --nocapture` | exit 0, 1 passed, 5 filtered | `qa-failure.log` |
| `cargo run --locked --quiet -p ovis-terminal-protocol --bin openrpc -- check schema/ovis-terminal.openrpc.json` | exit 0, `{"bytes":494525,"fresh":"schema/ovis-terminal.openrpc.json"}` | `openrpc-check.log` |
| `cargo run --locked --quiet -p ovis-terminal-protocol --bin openrpc -- compat schema/released.openrpc.json` | exit 0, `{"compatible_with":"schema/released.openrpc.json"}` | `openrpc-compat.log` |

### DEFERRED VERIFICATION (runner lands in Todo 21)

```text
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-4 --isolated --case happy --evidence "$ATTEMPT_DIR/task-4-ovis-terminal-v1.json"
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
exit=2
```

DEFERRED-BY-PLAN (plan line 275): no real-socket runner (todo 21) or assembled binary serving `rpc.discover` (todo 31) exists yet. This exit-2 attempt is not a conformance pass, and no `ovis-terminal` socket transcript is claimed. `conformance/scenarios/part-a/task-4.json` records the steps and adversarial cases the runner will execute.

## Gate review round 2 fixes (R2-B1, R2-B2)

- **R2-B1, narrowing through every branch:** the comparator now flattens each schema into *alternatives*: `anyOf`/`oneOf` members (nullable wrappers included), one alternative per entry of a type array, and `allOf` parts merged as an intersection (minimum of upper bounds, maximum of lower bounds, pattern union, literal intersection, required union). For inputs, every released alternative must be covered by some current alternative, recursing through `$ref`, `properties`, `additionalProperties`, `patternProperties` and `items`. Enum and const literals are compared as typed JSON values, and bound and pattern keywords apply only to the JSON types they constrain. Now reported: `[80,120] -> [80]` as `input_values_narrowed`; a pattern added to `PageToken` beneath the nullable `$ref` as `input_pattern_changed` at `terminal.list(page).after`; a lowered bound in an `allOf` part, a map value schema (`additionalProperties`/`patternProperties`) and items of a nullable array as `input_bound_narrowed`. Outputs compare the union of alternatives: closed-enum growth is checked against old literals and literal-free types, so nullable closed enums are still guarded.
- **Property tests:** `schema_properties.rs` generates random schema trees (strings, integers with finite sets, objects with required flags and open/closed extras, arrays, nullable wrappers, `$ref`s, up to four levels deep). It applies one random edit at a random preorder node: every narrowing edit must produce a break, and every widening edit must produce none (256 cases each).
- **R2-B2, bounded traversal:** reference resolution and union flattening carry a per-path `Trail` holding the set of entered `$ref`s and the depth. Re-entering a reference fails with `schema_reference_cycle`; exceeding 64 nested levels fails with `schema_depth_exceeded`. Both are checked before recursing, so a malicious baseline cannot overflow the stack. The CLI reads schema files through `take(64 MiB + 1)` and refuses larger files with `schema_file_too_large`. The reviewer's self-referencing baseline (`schema/fixtures/cyclic-released.json`) exits 1 with `schema_reference_cycle` instead of aborting with 134.
- **Mutants caught:** ignoring literals failed the numeric-enum test. Disabling the cycle check failed both cycle tests with `schema_depth_exceeded` (and still did not overflow, because the depth bound held). Sources were restored.

## Gate review round 1 fixes (B1-B5)

- **B1, narrowed accepted input:** the comparator now tracks per-node bounds and unrestricted ("free") types. It reports `input_bound_narrowed` when `maximum`, `exclusiveMaximum`, `maxLength`, `maxItems` or `maxProperties` is lowered or added, or when the matching minimum keywords are raised or added. It reports `input_pattern_changed` when a pattern is added or changed, and `input_values_narrowed` when an unrestricted string or number becomes an enum or const. Closing `additionalProperties` was already reported as `input_closed`. Widened bounds stay compatible (test).
- **B2, outputs and notifications:** `x-notifications` are compared as outputs, and a removed notification is reported as `notification_removed`. Union members are paired by property names, then required names, then position. An unpaired output member is reported as `output_variant_removed` instead of being skipped, so a required field removed inside a variant is reported as `output_no_longer_required`.
- **B3, `ErrorData` type and schema agree:** `errors::ErrorKind` is now open, with the 11 known variants plus `Other(String)` and string serde through `From<String>`/`Into<String>`. `token()` round-trips every token. A test decodes every kind token in the generated document into `ErrorData` and re-encodes it byte-identically; for example, `forbidden` becomes `ErrorKind::Other("forbidden")`.
  - **Dependency-seam edits to todo 3 files (disclosed):** `errors/kinds.rs` lost `Copy` and gained `Other`. In `errors/boundary.rs`, `kind()` now returns `&str` and `error_kind()` returns `&ErrorKind`. `ErrorData` is now boxed inside `BoundaryError`, so `Result<_, BoundaryError>` stays under clippy's large-error bound. `errors/data.rs` changed field-initialization order, and seven assertions in `tests/rpc.rs` now compare against `&ErrorKind::…`.
- **B4, exact missing-field path:** `decode_params` appends the field that serde reports missing to the tracked path. Missing `submit` gives `params.submit`, a nested missing field gives `params.escalation.term_ms`, and a sweep over every required `input.send` field checks each path. The shipped test that accepted `params` was replaced.
- **B5, malformed baselines:** a method, event, notification or param entry without a string `name`, or with a key its section does not define, now fails with the typed `malformed_baseline` error (`SchemaError::MalformedBaseline { location }`), and the CLI exits 1. Previously such entries were silently skipped.
- The weak prose-fragment check in `schema_params.rs` became an exact structured equality on the wire `ErrorData` object.
- Two mutants were killed: disabling bound narrowing failed 2 tests, and dropping the notification comparison failed P7. Both sources were restored.

## What landed

- `crates/ovis-terminal-protocol/src/schema/`: serde + schemars types for every request, reply, event, error, channel, limit and capability. Inputs are closed (`deny_unknown_fields`); outputs are additive. Partitioned by namespace (`methods/{server,terminal,terminal_read,input,attach,events,status,agent,template,layout,worktree,plugin,doc,machine,service,update,discovery}.rs`); every file is at most 250 pure LOC.
- Catalog: 99 methods, including thin layout (`layout.get/edit`, `selection.set`), input, agent, service and remote (`machine.*`) namespaces, plus 34 event types, the `event`/`heartbeat`/`missed` notifications, 123 error kinds (11 envelope + 112 domain), the framed-channel layout (read from the wire codec), limits with v1 defaults, and capabilities.
- `openrpc_document()` / `render_openrpc()` are deterministic. `verify_document` rejects duplicate method/event/notification names, dangling `$ref`s and schemars-disambiguated (digit-suffixed) definition names.
- `schema/ovis-terminal.openrpc.json` (generated) and `schema/released.openrpc.json` (the generation-1 baseline, identical at this commit). The JSON data stays outside Rust source.
- `check_compatibility` / `breaking_changes` return typed `Break { path, rule }` for: removed method, param or event; newly required param or input property; narrowed input type or values; removed input property or union variant; input closed; output type changed or lost; output property removed or no longer required; closed output enum grew.
- Output enums are `OpenEnum<T>` (`Known(T) | Unknown(String)`, marked `x-ovis-open-enum`), so a newer value decodes as `Unknown` instead of failing (GAP-018).
- `decode_params::<P>` decodes once, with the field path (`serde_path_to_error`), into `ParamsError` (kind, hint, `#[source]`), which converts to `BoundaryError::InvalidParams`.
- `advertised_methods(registered)` returns only methods with a registered handler, in catalog order, and refuses uncatalogued or duplicate handlers. `hello` therefore never advertises a specified-only method.
- `src/bin/openrpc.rs`: `write`/`check`/`compat`. Success prints JSON to stdout. Failures print typed JSON (`kind`, `hint`, `breaks`) to stderr with exit 1 (exit 2 for usage errors).

## QA

Happy test (`qa-happy.log`): renders the document twice and asserts byte equality, then equality with the checked-in `schema/ovis-terminal.openrpc.json`. It runs `verify_document` (unique names, every `$ref` resolves, no colliding definition names), asserts the presence of methods from every namespace and the stream kinds `DATA, CREDIT, CLOSE`, and runs `check_compatibility` against the released baseline. Output: `methods=99 events=34 schemas=304 bytes=494525`, `test happy ... ok`.

Failure test (`qa-failure.log`): injects `schema/fixtures/breaking-required-param.json` (a new required `pane_id` param on `terminal.get`) and asserts `kind() == "schema_incompatible"` with a `required_param_added` break at `terminal.get(pane_id)`. It then decodes `{"target":{"pane_id":"p1"}}` as `TerminalGetParams` and asserts `invalid_params`, `field_path == "params.target.pane_id"`, JSON-RPC -32602 after conversion, and a dispatch counter still at 0. Output: `injected terminal.get(pane_id): schema_incompatible -> required_param_added`, `unknown target property: invalid_params at Some("params.target.pane_id")`, `test failure ... ok`.

## Adversarial

- **Malformed input / closed schema:** proptests reject random unknown top-level and target fields at their exact paths (`schema_params.rs`). Two-key targets are rejected at `params.target`. The herdr baseline case (`"limit":"many"`) gives `invalid_params` at `params.limit` and the wire data contains no serde prose (`wrong_type_names_the_field_without_serde_prose`). Unknown input enum values are rejected (`input_enums_stay_closed`).
- **Stale state / released baseline:** `openrpc compat` against a baseline that has an extra method exits 1 with `{"kind":"schema_incompatible","breaks":[{"path":"pane.get","rule":"method_removed"}]}` (`adversarial-removed-method.log`). `schema_compat.rs` edits the real document and asserts each rule; proptest asserts that every randomly removed method is reported exactly, and that random optional params never break.
- **Gate probes (round 1):** `schema_gate.rs` turns P1-P7 into permanent tests, covering a lowered maximum, a raised minimum, lowered maxItems, a free string becoming an enum, a tightened pattern, a union variant losing a required field, a removed variant, open error kinds decoding, the `params.submit` path, an unnamed or extra-key baseline entry, and Heartbeat.seq and notification removal. It also runs the real `openrpc compat` binary against `schema/fixtures/malformed-released.json` (exit 1, `malformed_baseline`, empty stdout) and `retired-method-released.json` (exit 1, `method_removed`).
- **Misleading success:** `openrpc check` on a file differing by one byte exits 1 with `schema_stale` (`adversarial-stale.log`). The happy test fails if the checked-in document is stale. `verify_document` rejects dangling refs, duplicate names and colliding names (three tests). A mutation check (disabling the newly-required-input rule) made `newly_required_input_is_breaking` fail; the source was restored.

## Contract checklist

1. **Decimal-string counters:** `EventSeq`, `Revision`, `ByteOffset`, `ByteCount`, `LeaseEpoch`, `Incarnation`, `ProtocolGeneration`, `TimestampMs`, `RecordVersion`, `LineNumber`, `TurnNumber` and `DurationMs` wrap `DecimalU64` and are published as `{"type":"string","pattern":"^(0|[1-9][0-9]{0,19})$"}`. Tests: `u64::MAX` round trip, a numeric value rejected at `params.from_line`, and a proptest round trip of any `u64` revision.
2. **Typed boundary errors:** `SchemaError` (now with `MalformedBaseline`, kind `malformed_baseline`) and `ParamsError` (plus the bin's `CliError`) derive `thiserror::Error` and `Debug`, with `kind()`/`hint()` as static tokens and `#[source]` on `serde_json::Error`. Display strings do not repeat the kind token. Compat breaks are typed (`BreakRule` with `token()`, 19 rules), not strings. `ErrorKind` is open (`Other(String)`), so the published `ErrorData` schema and the Rust type accept the same kinds.
3. **Semantic newtypes:** 30+ distinct string identity types (terminal, lease, stream, subscription, prompt, agent, workspace/tab/pane/split node, repository/checkout, plugin/run, update, and so on). `ProcessPid(u32)`, and the counters above. `Option` is used only where absence is a real state.
4. **Behavior tests (now 85 in the crate, including 11 round-1 regressions, 8 round-2 regressions and 2 narrowing/widening proptests):** schema regeneration, compat rules on the real document, decode rejections with typed kinds and paths, advertisement filtering, stream layout checked against `Frame::encode`, and limit defaults checked against `wire::MAX_FRAME_PAYLOAD`. Proptests cover unknown-field rejection, counter round trips, removed-method reporting and optional-param additivity.
5. **No suppression or placeholders:** no `#[allow]` and no TODO. Two item-level `#[expect]`s, each with a reason: `exhaustive_enums` on `OpenEnum` (Known/Unknown is the complete contract) and `empty_structs_with_brackets` on `NoParams` (it must decode from `{}`).
6. **Evidence:** the claims above are limited to what the tests assert. Conformance is DEFERRED-BY-PLAN, with the exit-2 attempt recorded verbatim.

## Risks

- `errors::ErrorKind` now decodes every published kind; the 112 domain kinds decode as `ErrorKind::Other(token)`. Producers can emit them only as `Other` until a later lane promotes them to typed variants. `ENVELOPE_KINDS` in `error_catalog.rs` lists the 11 known variants by hand; the happy test fails if one goes missing (the reviewer's mutation showed this), but adding a variant does not force an update at compile time.
- All wire structs are `#[non_exhaustive]` (the workspace denies `exhaustive_structs`), so other crates must decode result values or use constructors added later. Server-side result construction lands with each method's owning todo.
- Field shapes follow ultrabrain-spec-full sections 1.1–1.11 plus the plan's layout/worktree/plugin/update/service additions. Later owning todos may add optional fields additively. Any breaking change must refresh `schema/released.openrpc.json` deliberately, which the gate makes visible.
- Dependency seams: `schemars 1.2` (already pinned) and `serde_path_to_error 0.1` (one new `[workspace.dependencies]` line in the root `Cargo.toml`). `Cargo.lock` gained dyn-clone, ref-cast(-impl), schemars(_derive), serde_derive_internals and serde_path_to_error, all MIT/Apache-2.0.
- Tests are split across `tests/schema.rs` (named QA targets) and `schema_catalog.rs`, `schema_compat.rs` and `schema_params.rs` to meet the 250-LOC cap (same precedent as todo 3). The compat rules independently implement the herdr-core design already covered by `THIRD_PARTY_NOTICES.md`; no upstream code is mirrored.
- The missing-field name is taken from serde's `missing field` message (serde's stable error format); every other path comes from `serde_path_to_error`.
- A legitimately recursive schema type (none exists in v1) would be reported as `schema_reference_cycle`; supporting recursive types would need pair-wise memoization instead of the per-path cycle refusal.
- Input coverage picks, for each released alternative, the current alternative with the fewest breaks; when nothing covers it, the reported rules come from that closest candidate.
