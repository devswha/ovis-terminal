# CI `test (ubuntu-latest)` disk exhaustion on e54c967: root cause

Failure: run 41342888203, job 124956466669. Annotation `System.IO.IOException: No space left
on device : '/home/runner/actions-runner/cached/2.338.2/_diag/Worker_20261101-194122-utc.log'`.
Every build/test step had succeeded: rustfmt, clippy, nextest, doctests, hygiene and the
hygiene failure fixture. The runner's worker died writing its own log, so no job log was
uploaded (`gh api .../jobs/124956466669/logs` returns nothing).

## What the soak harness and its tests write (measured)

Command: `OVIS_FIXTURE_ROOT=<fresh>/fixtures TMPDIR=<fresh>/tmp cargo nextest run -p
ovis-terminal-bench --all-targets --no-tests=fail`, with `du -sb` before and after, and
`git status --porcelain --ignored` before and after.

| | before the fix | after the fix |
|---|---|---|
| tests run | 30 passed | 30 passed |
| bytes under `OVIS_FIXTURE_ROOT` and `TMPDIR` after the run | 0 | 0 |
| entries left under them | 2 empty directories (`soak-hub-stall-<pid>`, `soak-hub-gap-<pid>`), also on panic | 0 |
| repo tree (`git status --ignored`) | unchanged | unchanged |

- Only the two hub tests touch the file system. Each creates one directory holding a Unix
  socket; `Hub::close` removes the socket. The directories themselves were never removed.
  They are now owned by a `Scratch` guard that removes them on drop, including on panic.
- No CI job runs the `soak` binary. `.github/workflows/platform-probes.yml`'s "soak" step
  is todo 12's `pty-harness soak`. `performance/soak` is declared-only, so the conformance
  suite does not run it. Runtime artifacts of the harness (`samples.ndjson`, checkpoints,
  holder sockets, logs) exist only in `soak.yml` and the macOS stress lane. They are
  bounded there: one 1 s sample line and one replaced checkpoint, and the private root is
  removed at teardown, which runs on every exit path.

## What the PR adds to the CI build footprint (measured)

CI builds `[profile.dev] debug = true` (root `Cargo.toml`; `ci.yml` does not override it),
and `--all-targets` builds every `[[bin]]` plus a test harness for each.

| artifact | line-tables-only (local) | full debuginfo (CI profile, measured in a scratch target) |
|---|---|---|
| `target/debug/soak` (hardlinked with `deps/soak-<hash>`) | 65,379,176 B | 210,816,152 B |
| `target/debug/terminal50` (existing, for comparison) | | 207,287,424 B |
| `deps/soak-<hash>` test harness of `soak/main.rs` (no tests) | 7,354,696 B | removed by `test = false` |

The same job passed on d7255d8 (run 41341635800) with the identical soak footprint, and
e54c967 changed nothing that job builds. e54c967's edits are `bench/soak/cli.rs` (the default
root of `soak` binary runs, never used by tests), `probes/platform-qa/stress/soak.sh` and
`pr-body.md`. The job therefore runs close to the runner's limit (about 14 GB free), and
this PR adds one ~211 MB full-debug binary to it. Fixes in this lane: the scratch directory
guard, and `test = false` on the `soak` bin (no empty test harness). Reducing the
workspace-wide debug footprint (`[profile.dev] debug = true`, every bench binary linking
the whole bench lib) is a shared-profile change and is left as a follow-up for the
orchestrator.

## soak.yml validity

`actionlint` 1.7.7 (`-shellcheck=`): no findings for `.github/workflows/soak.yml` or any
other workflow. `gh workflow view soak.yml --yaml --ref <lane>` and `gh workflow run
soak.yml --ref <lane>` both answer HTTP 404, because the file is not on the default branch.
