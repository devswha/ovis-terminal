## Before

Task 73 closes V-220 and delivers the IDEAL-108 regression follow-up to task 12.
Baseline: absent from the original herdr/hc transcripts; this is a regression
in the native ovis-terminal probe, not a legacy product feature.

PR #12, head `1e397d64736c551d4234e6f9ef993f0f3a09f5da`,
macOS job 124348370980 failed at `unix_happy.rs:45`:

```text
left: Grid { columns: 80, rows: 24 }
right: Grid { columns: 132, rows: 40 }
```

Original source:
`/tmp/ulw-ovis-yCJzPH/evidence/g6/task-73-macos-happy-failure-job-124348370980.log:1095-1098`.
The excerpt is preserved in `docs/evidence/task-73/investigation.md`.

Pre-fix stress reproduced the same failure at iteration 2:
https://github.com/devswha/ovis-terminal/actions/runs/41140283121
(macos-latest/all, job 124352609444; native macOS 26 aarch64).
The macOS 15 whole-binary job 124352609130 also failed with those sizes.
`docs/evidence/task-73/pre-fix-latest-all.log` is the uploaded stress log.

Command in each loop:

```sh
cargo nextest run -p ovis-terminal-holder --test platform_probe happy --locked
# Whole-binary variant, with normal nextest concurrency:
cargo nextest run -p ovis-terminal-holder --test platform_probe --locked
```

The failing nextest invocation exited 100; the job stopped at its first failure.
Task 72 was explicitly merged as the reproduction scheduling seam
(`32f12df`); its `unix_failure.rs` and evidence changes are inherited, not
task 73 implementation edits.

The diagnostic job
https://github.com/devswha/ovis-terminal/actions/runs/41140598357/job/124353637950
failed at the master-grid assertion immediately after resize, before a second
slave query. This distinguishes an overwritten resize from only a stale slave
read or buffered first-report tail. Apple's Bash readline reads then writes
the winsize while preparing its next input. The old `GRID` output was not a
barrier for that preparation.

## After

The shell now emits a recognizable prompt. The probe consumes it after the
complete slave-side `stty size` report, so the next-input terminal preparation
has finished before resize. It still requires the master and a fresh slave
report to equal exactly 132x40. No test serialization, sleep, weakened
assertion, holder change, macOS skip, or session API change.

Linux x86_64, Rust 1.99.1, local-only isolated Zig C/linker wrapper:

```text
$ bash scripts/check.sh
Summary [   5.020s] 194 tests run: 194 passed, 1 skipped
check.sh: all steps passed
exit 0
```

Full output: `docs/evidence/task-73/check.log`.
The one pre-existing skip is unchanged by this Unix-only probe edit.

Three consecutive manual stress dispatches target fixed source
`89bb89b41c48cf221249a4be0cf9b40112840210`:

- https://github.com/devswha/ovis-terminal/actions/runs/41141232863
- https://github.com/devswha/ovis-terminal/actions/runs/41141274758
- https://github.com/devswha/ovis-terminal/actions/runs/41141298345

These runs are currently pending, not claimed green. Each requires 100/100 in
both single-happy and whole-binary variants on macos-latest and macOS 15.
The temporary lane push trigger registered the workflow; the retained workflow
is manual-only. macos-latest uses Xcode 26.6 and macOS 15 uses Xcode 16.4.

DEFERRED VERIFICATION (runner lands in Todo 21):

```sh
cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario probes/platform --isolated --case happy --evidence .omo/evidence/ovis-terminal-v1/task-73/conformance.json
```

```text
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
exit 2
```

Verbatim attempt: `docs/evidence/task-73/conformance-attempt.log`.
This is not a passing product CLI/socket conformance claim.

## QA

- Pre-fix: native runner failure above, not a mocked test.
- Local happy/failure: the workspace gate runs both platform probe cases.
  Happy asserts actual master/slave sizes, live process identity, final bytes
  and exit ordering, then the existing mode, endpoint, detach and harness checks.
- Stress: normal nextest concurrency is retained, including the task 72 tests.
  Acceptance results will be recorded before handoff.

## Adversarial

```sh
cargo nextest run -p ovis-terminal-holder --test platform_probe shell_readiness_deadline --locked --nocapture
```

Exit 0; one test passed. A real shell is held in builtin `read` and cannot emit
the next prompt. The readiness helper is invoked with a zero deadline after an
actual resize, producing the intentionally caught assertion:

```text
shell readiness failed: slave Grid { columns: 80, rows: 24 }, master Ok(Grid { columns: 132, rows: 40 }): Err(Timeout { waited: 0ns })
```

The test catches only the helper call, requires failure, then releases and joins
the shell. Setup errors cannot satisfy it. It does not pin diagnostic prose.
Full output: `docs/evidence/task-73/deadline.log`.

The `flaky_tests` class is exercised by the native 100-iteration jobs; its final
counts are pending. Timing is exercised through the deterministic expired
deadline, with both observed sizes shown in the captured output.

## Contract checklist

1. Canonical decimal-string u64 JSON and u64::MAX: not exercised by this todo;
   no wire/JSON field or codec changed.
2. Typed boundary errors with kind/hint/source: not exercised by this todo;
   no boundary error/API changed. Existing probe timeout is reported, not hidden.
3. Semantic identity newtypes: not exercised by this todo; no identity changed.
4. Behavior tests: real pre-fix PTY failure and real fixed-code PTY checks;
   the added deadline test exercises the real held-shell/read path. No protocol,
   wire or store round-trip code changed, so no new property-test dependency.
5. No `#[allow]`, blanket `#[expect]`, placeholder TODO, speculative abstraction,
   prose-pinning test, sleep, weakened assertion or platform skip.
6. Evidence describes only actual assertions; conformance exited 2 and is
   explicitly deferred to Todo 21.

## Risks

The readiness marker is controlled by this test's `PS1`, not a heuristic for
arbitrary application readiness. This does not change runtime input readiness.
The ordering relies on the tested shell printing its prompt after preparing
terminal input; Apple readline's source ordering is cited in the investigation.
Native stress and PR CI must finish before the final DoneClaim.
Rust and YAML language servers were unavailable locally; compiler/Clippy and
GitHub's executed workflow provide the relevant diagnostics.
