Plan todo 1 (`.omo/plans/ovis-terminal-v1.md`): bootstrap the public repository and strict workspace. Closes GAP-145; delivers the scaffold for IDEAL-110 and IDEAL-112.

## Before

**absent: no repository, no CI.** The plan's Before line for todo 1 is `absent (GAP-145; no comparable new-runtime capability is claimed as a legacy execution)`. This PR replaces no herdr/hc behavior, so it cites no legacy transcript as a failing test.

This PR adds the herdr/hc baseline that later PRs cite. The redacted copies of the smoke transcripts named in the plan are in `docs/evidence/baseline/smoke/`. `docs/evidence/baseline/README.md` lists each copy's source path, line count, sha256 of the source and the copy, and the RT observation it supports. Examples: `herdr-h02-connection.log` (RT-HD-01, one request per connection), `herdr-h05-process-info-trap.log` (RT-HD-02, wrong-target answer) and `hc-s18-stop-cleanup.log` (RT-HC-04, status starts a daemon).

## After

Run on Linux x86_64 with the toolchain pinned in `rust-toolchain.toml` (rustc 1.99.1) and nightly rustfmt:

| Command | Exit |
|---|---|
| `git init -b main` | 0 |
| `gh repo create devswha/ovis-terminal --public --source . --remote origin --push --description "..."` | 0 |
| `cargo build --workspace --locked` | 0 |
| `bash scripts/check.sh` (fmt, clippy `-D warnings` over all targets, nextest `--no-tests=fail`, doctests, hygiene) | 0 |
| `cargo run -p ovis-terminal-cli -- --version` -> `ovis-terminal 0.1.0` | 0 |

nextest: `11 tests run: 11 passed, 0 skipped`. The tests cover the hygiene checker (8) and the CLI argument and version path (3). The product crates compile but carry no runtime behavior, so this is not a tested runtime.

Full log: [`docs/evidence/baseline/task-1-check.log`](docs/evidence/baseline/task-1-check.log).

CI: https://github.com/devswha/ovis-terminal/actions/runs/41103014618

Per-job conclusions:
- cargo deny = pass (49s)
- msrv (1.89) = pass (1m39s)
- rustdoc -D warnings = pass (1m3s)
- scripts/check.sh (ubuntu) = pass (38s)
- test (macos-latest) = pass (1m37s)
- test (ubuntu-latest) = pass (43s)
- test (windows-latest) = pass (1m29s)
- unused dependencies (cargo-machete) = pass (17s)

First PR: OPEN, 8/8 workflow jobs SUCCESS (no branch protection configured, so gh pr checks --required has no required set; recorded as such).

What landed:
- An edition-2024 workspace with the strict lint set from cargo-strict.md, licensed MIT. Every package uses `[lints] workspace = true`. Configs: `rust-toolchain.toml` (1.99.1), `rustfmt.toml`, `clippy.toml` (3-argument threshold), `deny.toml`.
- Compiling stubs for all 12 `crates/ovis-terminal-*` crates. Each declares its inter-crate dependencies (recorded in `DEPENDS_ON`) and the module slots that later todos fill (for example `protocol::wire`, `core::input::encoding`, `server::dispatch`).
- The `ovis-terminal --version` binary, the `conformance` binary (exits 2 because no scenarios are registered yet), `bench`, `probes`, `packages/client-ts/package.json`, and the root `package.json` workspaces.
- `scripts/check.sh` and `scripts/hygiene/`. The `workspace-hygiene` checker is syntax-aware: it counts pure lines of code from tokens using proc-macro2 spans and counts parameters through syn signatures. It also checks manifest lint inheritance and JSON validity.
- CI matrix on ubuntu, macos and windows: fmt, clippy, nextest, doctests, hygiene, a run of the failure fixture, and `--version`. Ubuntu also runs `check.sh`, rustdoc `-D warnings`, MSRV 1.89, `cargo deny` and `cargo machete`.
- `README.md`, `AGENTS.md` (bridge traps), and `conformance/scenarios/part-a/task-1.json`.

## QA

**Happy path.** `cargo run -p workspace-hygiene -- check --fixture boundary-250-loc-three-parameters` reports 0 violations and exits 0 (exactly 250 pure LOC and a 3-parameter function). `cargo run -p ovis-terminal-cli -- --version` prints `ovis-terminal 0.1.0` and exits 0.

**Failure: oversize fixture.** `cargo run -p workspace-hygiene -- check --fixture excessive-loc-and-parameters` exits 1:
```
scripts/hygiene/fixtures/excessive-loc-and-parameters.rs:1: rule=max-pure-loc count=251 limit=250
scripts/hygiene/fixtures/excessive-loc-and-parameters.rs:8: rule=max-parameters fn=too_many count=4 limit=3
workspace-hygiene: 2 violation(s) in 1 file(s) checked
```

**Failure: injected mis-formatted file (misleading success).** I appended `pub const   MISFORMATTED :u8=1;` to `crates/ovis-terminal-vt/src/lib.rs`, and `bash scripts/check.sh` stopped at the fmt step with exit 1:
```
==> cargo +nightly fmt --all -- --check
Diff in <home>/workspace/ovis-terminal-wt/ovis-terminal-v1-w1/crates/ovis-terminal-vt/src/lib.rs:9:
 /// Workspace crates this crate builds on; every entry is a declared Cargo dependency.
 pub const DEPENDS_ON: &[&str] = &[ovis_terminal_protocol::CRATE_NAME];
 
-pub const   MISFORMATTED :u8=1;
+pub const MISFORMATTED: u8 = 1;
 
check.sh exit=1
```
I then restored the file (sha256 matches the original), and `check.sh` exited 0 again.

**Failure: malformed input.** I put a copy of the malformed fixture at `crates/ovis-terminal-vt/src/malformed_probe.rs`. The workspace check rejected it, then I removed it:
```
crates/ovis-terminal-vt/src/malformed_probe.rs:3: rule=syntax message=cannot parse string into token stream
workspace-hygiene: 1 violation(s) in 53 file(s) checked
hygiene exit=1
```

**Adversarial classes:**
- dirty_worktree: `git status --porcelain` was empty after the commit. The staged set was audited against the todo 1 write scope.
- stale_state: no `target/` or `node_modules/` path is tracked, and `.omo/evidence/` is ignored.
- misleading_success: covered by the fmt injection above. I judge CI by the per-job conclusions, not by the exit of a `gh` command.
- flaky tests: one CI run with no retries.

Not merged automatically.
