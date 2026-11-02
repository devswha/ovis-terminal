## F2b: CI hotfix, ubuntu runner disk full (debuginfo footprint)

CI-only change. No product code, no `Cargo.toml` profile, no `scripts/check.sh` change.

## Before

- `docs/evidence/task-70/ci-disk-root-cause.md:31-33`: CI builds `[profile.dev] debug = true`
  (`Cargo.toml:148`), and `--all-targets` builds every bench/probe binary with full DWARF
  (~210 MB each). The job runs at the ~14 GB runner edge.
- Failing runs. Each annotation (`gh api repos/devswha/ovis-terminal/check-runs/<job>/annotations`)
  reads `System.IO.IOException: No space left on device : '/home/runner/actions-runner/cached/2.338.2/_diag/Worker_<ts>.log'`:
  - run 41342888203, job 124956466669, `test (ubuntu-latest)`, PR #76 e54c967
  - run 41363408351, job 125018646412, `test (ubuntu-latest)`, PR #78 9160c8e
  - run 41363408351, job 125018646453, `scripts/check.sh (ubuntu)`, PR #78 9160c8e
- Local full-debug measurement (`docs/evidence/task-F2b/guard-mutants.log`, case M1): the
  `ovis-terminal` binary is 341,496,168 B and the bench test harness 207,796,032 B. The
  bench+cli target is 2,740,826,183 B.

## Change

1. `.github/workflows/ci.yml` workflow env: `CARGO_PROFILE_DEV_DEBUG=line-tables-only`,
   `CARGO_PROFILE_TEST_DEBUG=line-tables-only`. These are the same values the dev host's
   toolchain env exports, so local check.sh and CI now build identical artifacts. rust-cache
   puts `CARGO_*` in its key, so the old full-debug caches are not restored.
2. `Swatinem/rust-cache@v2` `cache-targets`: `false` on ubuntu `test` and `check-script`
   (registry and git only); `true` on macOS and Windows (unchanged behaviour).
3. New `scripts/ci/target-footprint.sh`, run as the last step of ubuntu `test` and
   `check-script`. It writes `du -sb` of target, deps, build and incremental plus the free
   disk to the step summary, and fails (`rule=debuginfo-ceiling`) if any debug executable is
   above 200,000,000 B. That is between 150.8 MB (largest with line-tables-only) and
   341.5 MB (CLI with full debug).

## After

| command | exit | result |
|---|---|---|
| `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` | 0 | nextest `1490 tests run: 1490 passed (3 slow), 2 skipped`; subscriptions 22 passed; `check.sh: all steps passed` (`check-sh.log`) |
| `bash scripts/ci/target-footprint.sh /dev/shm/ovt-target-lane-tF2b` (target of that check.sh run, line-tables-only) | 0 | whole-workspace target 6,208,584,571 B; deps 6,000,527,138 B; largest executable `conformance` 150,776,920 B (`after-workspace-footprint.log`) |
| `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` | 0 | `windows-precheck.log` |
| `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings` | 0 | `windows-precheck.log` |
| `actionlint -shellcheck= .github/workflows/ci.yml` (1.7.7) | 0 | no findings |

Bench crate subset, `cargo nextest run -p ovis-terminal-bench --all-targets --all-features --no-run`,
fresh target per case (`bench-profile-measure.log`):

| case | target B | largest test harness B |
|---|---|---|
| profile default (`debug = true`) | 1,495,632,179 | 207,796,032 |
| DEV + TEST line-tables-only | 985,256,213 | 66,120,952 |

Backtraces (`backtrace-file-line.log`, a scratch crate with a failing test, `RUST_BACKTRACE=1`):
line-tables-only prints `panicked at src/lib.rs:2:5` plus 3 `at ./src/lib.rs:N` frames
(`bt::inner at ./src/lib.rs:2:5`). `debug = none` prints 0 such frames.

CI after: CI_AFTER_PLACEHOLDER

## Mutants (each must fail a shipped check)

`guard-mutants.log` (the guard script on a real bench + cli target):

- M1, both env vars removed (profile default `debug = true`): guard exit 1. Three executables
  are above the ceiling: `ovis-terminal` 341,496,168, its deps copy, and the bench harness
  207,796,032.
- M2, only `CARGO_PROFILE_DEV_DEBUG` removed: guard exit 1 (`ovis-terminal` 341,496,040).
  `cargo build`/`cargo run`/clippy use the dev profile, so the DEV var is load-bearing.
- M3, debuginfo lowered to `none`: the backtrace check finds 0 file:line frames (3 with
  line-tables-only). line-tables-only is therefore the smallest setting that keeps file:line.
- Survivor, recorded honestly: removing only `CARGO_PROFILE_TEST_DEBUG` changes nothing
  today (985,256,684 vs 985,256,213 B), because `test` inherits `dev`. It is kept so a future
  `[profile.test] debug` key in `Cargo.toml` cannot quietly bring full debuginfo back.

## Adversarial

- Cache poisoning: an old full-debug target cache would restore 2.5x artifacts. rust-cache's
  key includes `CARGO_*` env, and ubuntu test/check no longer cache targets at all.
- Windows/macOS regression: line-tables-only is valid for MSVC (PDB line tables) and Apple
  ld. Their target cache stays on. The guard step is gated `runner.os == 'Linux'` because it
  uses GNU `stat -c`/`du -b`. The PR's macOS/Windows `test` jobs are the regression check.
- Build time: ubuntu test/check now rebuild dependencies every run (no target cache), which
  trades minutes for disk. Recorded under Risks.

## Seams

All files are in scope for this CI lane: `.github/workflows/ci.yml`,
`scripts/ci/target-footprint.sh` (new, CI-only), and `docs/evidence/task-F2b/`. Nothing else
changed.

## Risks

- Ubuntu `test`/`check-script` wall time goes up by the dependency compile (no target cache).
  If that matters more than disk, re-enable `cache-targets` once the line-tables-only footprint
  is confirmed green.
- The 200 MB guard ceiling is a size heuristic. CI links with gcc-14 and local builds with
  zig, so sizes differ slightly. A binary that legitimately grows past 200 MB with
  line-tables-only would trip it, and the failure message names the file.
- Other workflows (platform-qa, quality, soak, probe-stress) still build with `debug = true`.
  They were not reported as disk-full and are out of this hotfix's scope.

## macOS stress dispatch

N/A. This lane adds no probe, and `probe-stress-macos.yml` does not read `ci.yml`'s env, so
a dispatch would exercise nothing this PR changes. macOS coverage is the PR's
`test (macos-latest)` job.

## DEFERRED VERIFICATION

None: no conformance scenario applies to a CI-workflow change.

## DoneClaim

DONECLAIM_PLACEHOLDER
