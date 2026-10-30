## Before

RT-HD-02 / GAP-009 / GAP-010: `docs/evidence/baseline/smoke/herdr-h05-process-info-trap.log:16` shows a request for `w1:p2` returning process information for focused pane `w1:p1`. The evidence is also preserved in the original smoke transcript at the same path.

## After

The named registry tests prove scoped explicit-ID resolution, stable opaque identity after rename, structured ambiguity candidates for equal names on different machines, durable reservations across store reopen, and lifecycle identity changes for restart, upgrade, holder loss, relaunch and host reboot.

`cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-9 --isolated --case happy --evidence "$ATTEMPT_DIR/task-9-ovis-terminal-v1.json"` exited 2 with:

```text
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
```

DEFERRED VERIFICATION (runner lands in Todo 21): this foundational todo has no real-socket runner or assembled binary. The exit-2 attempt is recorded verbatim in `docs/evidence/task-9/conformance-attempt.log`; no CLI/socket result is claimed.

## QA

- `cargo test -p ovis-terminal-core --test registry happy -- --exact --nocapture`: exit 0, 1 assertion-bearing test passed.
- `cargo test -p ovis-terminal-core --test registry failure -- --exact --nocapture`: exit 0, 1 assertion-bearing test passed.
- Full-width identity property test and explicit `u64::MAX` serialization test: exit 0 each.
- `cargo nextest run -p ovis-terminal-core`: exit 0.
- `cargo clippy -p ovis-terminal-core --all-targets -- -D warnings`: exit 0.
- `bash scripts/check.sh`: exit 0; all steps passed, workspace hygiene reported 0 violations, 16 manifests and 22 JSON files checked.
- `cargo metadata --locked --format-version 1 --no-deps`: exit 0.
- Platform: Linux 7.0.0-38-generic, x86_64; rustc 1.99.1; cargo 1.99.1.

The full workspace nextest summary reported 195 passed and 1 skipped. The skip reason was not present in the captured summary.

The initial Windows CI run on head `af30f0102c93ef166d733bdcf03b7f8d9e765b` failed in `registry happy` with sharing violation error 32. Inspection found the test retained a `Store::snapshot()` SQLite reader across store close/reopen. This repair explicitly drops the snapshot before closing the store; it changes no product store path and adds no retry or skip. The updated Windows CI result is available on this PR's current head.

## Adversarial

The failure test asserts typed rejection for zero and wrong-family IDs, invalid names, missing targets, cross-lifetime caller bindings, and a PID reused with the prior start token. A caller bound to the old process receives `stale_target`, not the new process. The happy test asserts that same-name terminals on two machine IDs return two ambiguity candidates and no implicit selection.

## Contract checklist

- Machine, runtime, host-boot, server-boot, workspace, tab, pane, split, terminal, process, stream and lease identities and generations are distinct Rust types.
- IDs, generations and PIDs use canonical string JSON; maximum-width values round-trip and numeric JSON is rejected.
- Machine and family-local IDs are committed with their reservation, receipt and event before the allocator returns them.
- Every registry boundary error is a `thiserror` enum with stable `kind()` and `hint()`; inner errors remain sources.
- Process identity includes opaque process ID, generation, PID and start token. Relaunch increments generations only after spawn confirmation.
- No `#[allow]`, production `unwrap`/`expect`, numeric `as` conversions or placeholder TODOs were added.

## Risks

The real CLI/socket transcript is deferred as described above. Local platform QA covers Linux x86_64 only; macOS and Windows are not claimed. Rust and JSON language-server diagnostics were unavailable because rust-analyzer and Biome are not installed.
