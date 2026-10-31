## Before

RT-HD-02 / GAP-009 / GAP-010: `docs/evidence/baseline/smoke/herdr-h05-process-info-trap.log:16` shows a request for `w1:p2` returning process information for focused pane `w1:p1`. The evidence is also preserved in the original smoke transcript at the same path.

## After

The named registry tests prove lifecycle identity transitions for core restart, upgrade, holder loss, relaunch and host reboot. Restart and upgrade assert that runtime, terminal and process incarnations survive while the adopted server boot cannot be reused; host reboot asserts a new runtime/boot and restored metadata; confirmed relaunch increments terminal and process generations. Removing the core/upgrade boot assignment makes the shipped `happy` test fail at its boot-ID assertion (exit 101); restoring it passes. The mutation is recorded in `docs/evidence/task-9/mutation-boot-id.log`.

Malformed UUID and overflowing resource-ID parsing preserve their underlying `uuid::Error` / `ParseIntError` through `Error::source()` while keeping `kind() == "invalid_identity"`. Focused source-retention tests also cover generation and PID integer parsing. The source tests are `malformed_machine_uuid_preserves_source_and_kind`, `malformed_resource_integer_preserves_source_and_kind`, and `malformed_generation_and_pid_integers_preserve_sources`.

`cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-9 --isolated --case happy --evidence "$ATTEMPT_DIR/task-9-ovis-terminal-v1.json"` exited 2 with:

```text
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
```

DEFERRED VERIFICATION (runner lands in Todo 21): this foundational todo has no real-socket runner or assembled binary. The exit-2 attempt is recorded verbatim in `docs/evidence/task-9/conformance-attempt.log`; no CLI/socket result is claimed.

## QA

- `cargo test -p ovis-terminal-core --test registry happy -- --exact --nocapture`: exit 0, 1 assertion-bearing test passed.
- `cargo test --locked -p ovis-terminal-core --test registry host_reboot_and_relaunch_identity_domains -- --exact --nocapture`: exit 0.
- `cargo test -p ovis-terminal-core --test registry failure -- --exact --nocapture`: exit 0, 1 assertion-bearing test passed.
- Full-width identity property test, explicit `u64::MAX` serialization test, source-preservation tests, scoped target resolver, and durable reservation reopen tests: exit 0.
- `cargo nextest run -p ovis-terminal-core`: exit 0.
- `cargo clippy -p ovis-terminal-core --all-targets -- -D warnings`: exit 0.
- `bash scripts/check.sh`: exit 0; all steps passed, 205 tests passed and 1 intentional child-process fixture skipped, workspace hygiene reported 0 violations in 246 files, 16 manifests and 23 JSON files checked.
- `cargo metadata --locked --format-version 1 --no-deps`: exit 0.
- Reconciled with `origin/integration` at `eb9827d` via merge `4e76232` before the repair commit; locked metadata validated after the merge.
- Platform: Linux 7.0.0-38-generic, x86_64; rustc 1.99.1; cargo 1.99.1.

The final full workspace nextest summary reported 205 passed and 1 skipped. The skipped case is the intentional `crash_child` process fixture invoked by its parent test.

The initial Windows CI run on head `af30f0102c93ef166d733bdcf03b7f8d9e765b` failed in `registry happy` with sharing violation error 32. The test now drops its `Snapshot` reader before closing/reopening the store; there is no product store change, cfg-skip or retry. PR #13 on the prior repair head `d8ca367` reported the Windows test and probe checks successful; this F1/F2 review-fix head will receive a fresh platform CI run.

## Adversarial

The failure registry test asserts missing targets, cross-lifetime caller bindings, and a PID reused with the prior start token; the caller receives `stale_target`, not the new process. ID unit tests assert typed rejection for zero, wrong-family IDs and invalid names. The happy test asserts that same-name terminals on two machine IDs return two ambiguity candidates and no implicit selection.

AdversarialVerify F1 was mutation-tested on the final test shape: replacing `next.identity.server_boot = server_boot_id` with a comment makes `registry happy` fail at `tests/registry.rs:77`, assertion `core restart must adopt the new server boot ID` (exit 101). Restoring the assignment makes the test pass (exit 0). F2 tests assert `Error::source().is_some()` for malformed UUID and overflowing resource IDs while `kind()` remains `invalid_identity`; generation and PID parser errors retain sources too. Evidence is in `docs/evidence/task-9/mutation-boot-id.log`.

## Contract checklist

- Machine, runtime, host-boot, server-boot, workspace, tab, pane, split, terminal, process, stream and lease identities and generations are distinct Rust types.
- IDs, generations and PIDs use canonical string JSON; maximum-width values round-trip and numeric JSON is rejected.
- Machine and family-local IDs are committed with their reservation, receipt and event before the allocator returns them.
- Every registry boundary error is a `thiserror` enum with stable `kind()` and `hint()`; inner errors remain sources.
- Process identity includes opaque process ID, generation, PID and start token. Relaunch increments generations only after spawn confirmation.
- No `#[allow]`, production `unwrap`/`expect`, numeric `as` conversions or placeholder TODOs were added.

## Risks

The real CLI/socket transcript is deferred as described above. Local platform QA covers Linux x86_64 only; macOS and Windows are not claimed. Rust and JSON language-server diagnostics were unavailable because rust-analyzer and Biome are not installed.
