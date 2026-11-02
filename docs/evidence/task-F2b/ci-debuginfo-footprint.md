# F2b: CI debuginfo footprint (ubuntu runner disk full)

## Before (CI)

Root cause, measured in `docs/evidence/task-70/ci-disk-root-cause.md:31-33`: root `Cargo.toml`
`[profile.dev] debug = true` (`Cargo.toml:148`), `ci.yml` did not override it, and
`--all-targets` builds every bench/probe binary with full DWARF.

| run / job | head | job | annotation |
|---|---|---|---|
| 41342888203 / 124956466669 (PR #76) | e54c967 | test (ubuntu-latest) | `System.IO.IOException: No space left on device : '/home/runner/actions-runner/cached/2.338.2/_diag/Worker_20261101-194122-utc.log'` |
| 41363408351 / 125018646412 (PR #78) | 9160c8e | test (ubuntu-latest) | `No space left on device : '.../_diag/Worker_20261101-234328-utc.log'` |
| 41363408351 / 125018646453 (PR #78) | 9160c8e | scripts/check.sh (ubuntu) | `No space left on device : '.../_diag/Worker_20261101-234328-utc.log'` |

Command: `gh api repos/devswha/ovis-terminal/check-runs/<job>/annotations --jq '.[].message'`.

## Fix (CI only; Cargo.toml, check.sh and product code unchanged)

- `ci.yml` workflow env: `CARGO_PROFILE_DEV_DEBUG=line-tables-only` and
  `CARGO_PROFILE_TEST_DEBUG=line-tables-only`. These are the values the development host
  already exports in its toolchain env, so local `scripts/check.sh` and CI now build the same
  artifacts. rust-cache hashes `CARGO_*` variables into its key, so no full-debug cache is
  restored.
- `Swatinem/rust-cache@v2` `cache-targets: false` on the ubuntu `test` job and on `check-script`
  (registry and git only). macOS and Windows keep their target cache.
- `scripts/ci/target-footprint.sh` (CI-only step after ubuntu `test` and `check-script`): it
  writes `du -sb` of target, deps, build and incremental plus the free disk to the step
  summary, and fails if a debug executable is above 200,000,000 bytes.

## Measured locally (same toolchain, zig linker; CI links with gcc-14)

`bench-profile-measure.log`: `cargo nextest run -p ovis-terminal-bench --all-targets
--all-features --no-run`, fresh target per case.

| case | target bytes | largest test harness |
|---|---|---|
| profile default (`debug = true`) | 1,495,632,179 | 207,796,032 |
| DEV + TEST line-tables-only | 985,256,213 | 66,120,952 |
| DEV only | 985,256,684 | 66,120,952 |
| TEST only | 985,257,243 | 66,120,952 |

`guard-mutants.log`: the same build plus `cargo build -p ovis-terminal-cli`, then the guard.

| case | ovis-terminal bytes | target bytes | guard exit |
|---|---|---|---|
| M1 profile default (both vars removed) | 341,496,168 | 2,740,826,183 | 1 (3 executables above ceiling) |
| After (both vars) | 145,558,200 | 1,606,468,550 | 0 |
| M2 DEV var removed (TEST kept) | 341,496,040 | 2,617,038,210 | 1 |

That is a 41% smaller target and a 57% smaller CLI binary for this subset. The full-workspace
after size comes from the check.sh run below and from the CI step summary.

## Backtraces keep file:line

`backtrace-file-line.log`: a scratch crate with a failing test (out-of-bounds index in
`inner`) run with `RUST_BACKTRACE=1`.

- line-tables-only: `panicked at src/lib.rs:2:5`, with frame `bt::inner at ./src/lib.rs:2:5`.
  3 `at .../src/lib.rs:N` frames.
- M3, `debug = none`: the panic location is still printed, but there are 0 file:line frames.
  line-tables-only is therefore the smallest setting that keeps file:line backtraces.
