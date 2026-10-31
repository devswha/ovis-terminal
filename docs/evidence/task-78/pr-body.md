fix(ops): deduplicate by params digest and bound params per method (todo 78)

Closes GAP-089 (regression). Delivers IDEAL-034 (regression).

## Before

The todo-15 ledger refused any canonical params above 256 KiB (`ops/request.rs`, pinned by `ops/qa/bounds.rs::canonical_parameters_accept_262144_bytes_and_reject_262145`), while `hello` advertises `doc_max_bytes = 1 MiB`. A `doc.set` between 256 KiB and 1 MiB was refused before the document layer saw it, verbatim from `docs/evidence/task-50/qa-socket.log:15`:

```
<- {"jsonrpc":"2.0","id":"01a192fc-1897-77cc-8f6a-5c7c3231865e","error":{"code":-32000,"message":"operation failed","data":{"kind":"too_large","hint":"bound output and parameters before submitting the operation","retryable":false}}}
```

The largest `doc.set` value that round-tripped was 262092 bytes (`docs/evidence/task-50/pr-body.md`, Risks: "Open decision"). Ledger rows stored the digest as a bare JSON byte array without an algorithm name.

## After

Host: Linux x86_64 (`uname -sm`), toolchain 1.99.1. Branch base `origin/integration` 664ded7. `origin/integration` has since moved to b8866bf (T27); it changes none of the files in this PR, and per the lane rules I did not merge it.

| command | exit |
|---|---|
| `cargo test -p ovis-terminal-server --test ops_digest happy -- --exact --nocapture` | 0 |
| `cargo test -p ovis-terminal-server --test ops_digest failure -- --exact --nocapture` | 0 |
| `cargo test -p ovis-terminal-server --test documents_retention` (5 tests) | 0 |
| `cargo test -p ovis-terminal-core --lib ops::params` (6 tests) / `--test ops digest` (3 tests) | 0 |
| `cargo test -p ovis-terminal-store --lib` (incl. `store::schema::tests::a_version_one_database_migrates_to_the_current_version_with_its_rows`) | 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 |
| `bash scripts/check.sh` (nextest 758 run / 758 passed / 1 skipped) | 0 |
| `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` | 0 |
| `cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-78 --isolated --case all --evidence …/conformance.json` | 0 (2 passed, 35 assertions) |
| same with `--scenario all/all` | 0 (24 passed, 0 failed, 0 skipped, 28 declared_only) |

`happy` over the real socket (`docs/evidence/task-78/qa-happy.log`): a value whose JSON encoding is exactly 1048576 bytes is committed at version 1, `doc.get` returns it byte-exact, and the identical retry under the same op_id returns the same receipt:

```
committed version="1" value_bytes=1048576 read_back_exact=true
identical retry replayed={"seq":"4","version":"1"}
```

`failure` over the real socket (`docs/evidence/task-78/qa-failure.log`):

```
changed 1 MiB retry -> {"hint":"inspect the original operation; use a new identity only for new work","kind":"op_reused","op_id":"01a19322-3ec5-7dc6-9a25-f7fbe24139b5","retryable":false}
1 MiB + 1 byte value -> {"hint":"shrink the value below the advertised doc_max_bytes","kind":"too_large","op_id":"01a19322-40ca-76f6-afe5-e6d3509220f5","retryable":false}
params of 1052673 bytes (bound 1052672) -> {"hint":"keep doc.set params within doc_max_bytes plus 4096 bytes for name, version and machine","kind":"too_large","retryable":false}
numeric version -> {"field_path":"params.version","hint":"correct the method parameters","kind":"invalid_params","retryable":false}
```

The test then asserts the document is still version 1 with the original value.

### What changed

- `Request::new` bounds canonical params per method (`ops::ParamsBound`): `doc.set` gets `Limits::v1().doc_max_bytes` + 4096 bytes (`PARAMS_ENVELOPE_BYTES`, room for name/version/machine) = 1052672; every other method keeps the 256 KiB `DEFAULT_PARAMS_BYTES`. Above it: `OperationError::ParamsTooLarge { bound, actual }` (kind `too_large`, hint names the bound), before admission (no op_id).
- Reuse is decided by `ParamsDigest` = SHA-256 of the canonical encoding, persisted as `"params_digest": {"algorithm": "sha-256", "hex": "<64 hex>"}`. Unknown algorithm names, uppercase or short hex, or a row with no digest fail to decode; nothing matches them. `Request` equality (the in-flight join in `dispatch/tasks.rs`, todo 14 N1) compares op_id, method and digest only.
- The first 256 KiB (`PARAMS_COPY_BYTES`) of the canonical params are stored in a separate resource `operation_params/<runtime>/<key>`, written in the same commit as the acceptance and deleted in the same commit as the row's eviction. The row records `"params_copy": {"bytes": "<total>", "truncated": bool}`. The cut is moved back to a UTF-8 character boundary. The row stays under the unchanged 16 KiB receipt cap, and the snapshot producer never reads copies. New `Operations::params(&op_id) -> ParamsView { digest, copy: Option<ParamsCopy { prefix, canonical_bytes, truncated }> }`.
- Ledger schema is additive: pre-78 rows (`"params": [32 bytes]`) still decode as SHA-256, with `copy: None`. The next transition rewrites them in the new form without the bare field.
- Store `SCHEMA_VERSION` 1 → 2: a migration with no table changes. A version-1 build opening a post-78 database classifies it as `Probe::Newer` (typed `unsupported_schema`, original kept). Without that, the version-1 build would fail to decode operation rows on every request.

## QA

- happy / failure: above (real socket, real dispatcher, ledger and SQLite store).
- `core ops::qa::digest`: (1) a 1 MiB `doc.set` retried with its last byte changed is `op_reused` with the original receipt, while the identical retry replays. The stored copy equals the first 262144 bytes of the *changed* encoding, so only the digest tells the two apart. The digest hex equals an independently computed SHA-256 of the canonical bytes. (2) The copy is whole (`truncated=false`) at exactly 262144 bytes and a 262144-byte prefix with `truncated=true` at 262145. (3) A two-byte character straddling the cut is not split: the prefix is 262143 bytes and valid UTF-8.
- `core ops::qa::bounds`: terminal.run accepts 262144 and refuses 262145; doc.set accepts 1052672 and refuses 1052673. Each refusal is `ParamsTooLarge { bound, actual = bound + 1 }` with the bound's hint, and doc.set's bound equals `doc_max_bytes + PARAMS_ENVELOPE_BYTES`.
- Store: `a_version_one_database_migrates_to_the_current_version_with_its_rows` writes a version-1 database with raw SQL (migration 0 only), opens it with the current `Store`, reads the row unchanged and probes version 2.

## Adversarial

- misleading_success (digest collision claim): the copy cannot vouch for equality. Two requests identical in every stored byte but differing at byte 1048574 are refused (core digest test 1 and socket `failure`). Stored digests that drop or misname the algorithm, or carry malformed hex, are refused at decode (`params::tests::digests_without_a_known_algorithm_or_with_bad_hex_are_refused`, `a_row_without_any_digest_or_with_a_short_bare_digest_is_refused`).
- stale_state (pre-78 rows without a named digest): `params::tests::legacy::pre_78_rows_still_deduplicate_and_are_rewritten_with_a_named_digest` rewrites a row into the exact pre-78 shape (bare byte array, no copy fields, copy resource deleted) and reopens on a new boot. The identical retry replays, a changed retry is `op_reused`, `params()` reports `copy: None`, and after `op.cancel` the stored row has `params_digest.algorithm == "sha-256"` and no `params` key. `eviction_deletes_a_copy_with_its_row_and_skips_rows_without_one`: retention eviction of a pre-78 row (no copy) and a current row commits; both ids are `op_expired`, and only the current row's copy becomes a tombstone (no orphaned copies). A proptest over all 32-byte digests checks that a pre-78 field reads as SHA-256 and round-trips unchanged in the new form.
- bounds: the exact per-method boundaries above; a 1 MiB + 1 value passes the ledger and is refused by the document bound inside the admitted operation (`too_large` with op_id, document unchanged); a numeric `version` is `invalid_params`.
- No sleeps or polling: socket tests await the product's replies.

## Seams

- New public core API: `ovis_terminal_core::ops::{ParamsBound, ParamsDigest, DigestAlgorithm, DigestError, ParamsView, ParamsCopy, DEFAULT_PARAMS_BYTES, PARAMS_ENVELOPE_BYTES, PARAMS_COPY_BYTES}`, `Operations::params`, and `OperationError::ParamsTooLarge` (the enum is `#[non_exhaustive]`). `OperationError::TooLarge` now covers only receipt/evidence bounds.
- `ovis_terminal_store::SCHEMA_VERSION` is 2 (fence migration, no DDL).
- **`ops/admission.rs` is edited despite the lane note that todo 79 owns it.** There are two inserted statements and no changed lines: the copy write after the row write, and the copy delete beside the evicted row's delete. The copy has to commit atomically with acceptance and eviction, and both happen only inside `accept_once`. The hunks are at old lines 80 and 116, away from T79's hunks (`accept`/`admit` signatures and the first line of `accept_once`), so a union merge should be clean.
- Test files outside the literal write scope, changed because the behavior they pinned is exactly what this todo changes: `crates/ovis-terminal-server/tests/documents_retention.rs` and `documents_retention/large.rs` (a 1 MiB + 1 value is now refused by the document bound *with* op_id; large.rs now round-trips up to the full 1 MiB), `crates/ovis-terminal-core/src/docs/qa/bounds.rs` (the same refusal is now a retained failed operation). The `Served` harness moved verbatim into `tests/documents_retention/served.rs`, which `tests/ops_digest.rs` shares. `conformance/src/runner_tests/suite.rs` lists the new executable scenario (13 entries). `.github/workflows/probe-stress-macos.yml`: option `ops-digest` and job appended at the end.

## Contract checklist

1. Every new counter in JSON is a decimal string (`params_copy.bytes` is `ByteCount`). A numeric `version` is `invalid_params` over the socket (tested). This todo adds no new u64 wire field beyond `ByteCount`, whose u64::MAX round trip is covered by the protocol crate's existing tests.
2. `DigestError` is a thiserror enum. It is an internal decode error that surfaces through serde as `persistence_failed`/`invalid_record`, so it has no `kind()`/`hint()`. `OperationError::ParamsTooLarge` has kind `too_large` and a hint that names its bound.
3. `ParamsDigest`, `ParamsBound` and `ParamsCopy` are distinct types. `Option` is used only for "accepted before copies existed".
4. Tests assert behavior: replay vs `op_reused`, typed bounds, stored bytes vs canonical encodings, tombstones, a v1→v2 open. Proptest covers the digest round trip.
5. No `#[allow]`/`#[expect]`, no placeholders, no prose pins. The socket test compares the wire hint with the shipped `ParamsBound::Document.hint()`.
6. Holder wire generation is still 2; the checklist items are unchanged.

## Risks

- The `doc.set` → Document bound mapping keys on the method name string `"doc.set"`. A future method with an advertised payload bound must be added to `ParamsBound::of`.
- `ErrorData` has no numeric bound field, so the wire names the bound only through the hint text (typed details on the wire are todo 31). The core error carries `bound` and `actual` as typed fields.
- The store fence makes a rollback to a pre-78 build refuse the database (`unsupported_schema`, original kept) instead of misreading it. A rollback needs the v2 database moved aside.
- The runner's protocol fixture `doc.set` echoes the value into its receipt (16 KiB cap), so the executable scenario covers digest dedup and invalid_params with small values. The 1 MiB and bound cases run in `tests/ops_digest.rs` over the real socket.

## CI

- PR checks: see below once the run completes.
- macOS stress `ops-digest`: run URL recorded below.
